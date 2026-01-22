//! Garbage collection for unreferenced objects.
//!
//! Implements mark-and-sweep garbage collection to remove objects that are no longer
//! reachable from any references (HEAD, STAGE, or refs/*).

use crate::error::Result;
use crate::object_id::ObjectId;
use crate::object_store::ObjectStore;
use crate::refs::Refs;
use crate::types::Commit;
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, SystemTime};

/// Progress callback for GC operations.
/// Called with (current, total, phase) where phase is "scan", "mark", or "sweep".
pub type GcProgressCallback = dyn Fn(usize, usize, &str);

/// Configuration for garbage collection.
#[derive(Debug, Clone)]
pub struct GcConfig {
    /// Don't actually delete objects, just report what would be deleted.
    pub dry_run: bool,

    /// Grace period in days - keep objects newer than this even if unreachable.
    /// This prevents accidental deletion of recently created objects.
    pub grace_period_days: u32,

    /// Skip grace period and delete unreachable objects immediately.
    /// Use with caution!
    pub aggressive: bool,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            dry_run: false,
            grace_period_days: 7,
            aggressive: false,
        }
    }
}

/// Report from garbage collection operation.
#[derive(Debug)]
#[derive(Default)]
pub struct GcReport {
    /// Total number of objects scanned.
    pub objects_scanned: usize,

    /// Number of objects that are reachable (kept).
    pub objects_reachable: usize,

    /// Number of objects deleted.
    pub objects_deleted: usize,

    /// Bytes freed from deletion.
    pub bytes_freed: u64,

    /// Errors encountered during GC (non-fatal).
    pub errors: Vec<String>,
}


/// Run garbage collection on the repository.
///
/// This implements mark-and-sweep garbage collection:
/// 1. **Mark phase**: Find all reachable objects starting from roots (HEAD, STAGE, refs/*)
/// 2. **Sweep phase**: Delete unreachable objects older than grace period
///
/// # Safety
///
/// GC is safe to run on a properly-managed repository. The grace period
/// (default: 24 hours) ensures that recently-created objects are never deleted,
/// protecting against race conditions with concurrent operations.
///
/// **Do not run GC if:**
/// - Another process has an active session (check for LOCK file)
/// - You've manually created objects without updating refs
/// - The repository verification failed (run `verify` first)
///
/// # Examples
///
/// ```no_run
/// use ctx_core::{CtxRepo, GcConfig};
///
/// let mut repo = CtxRepo::open(".").unwrap();
/// let config = GcConfig::default();
/// let report = repo.gc(config).unwrap();
///
/// println!("Freed {} bytes", report.bytes_freed);
/// ```
///
/// Note: This is a low-level function. Most users should call `CtxRepo::gc()` instead,
/// which handles the borrowing internally.
pub fn gc(
    refs: &Refs,
    object_store: &mut ObjectStore,
    config: GcConfig,
    progress: Option<&GcProgressCallback>,
) -> Result<GcReport> {
    let mut report = GcReport::default();

    // Phase 1: Collect roots
    if let Some(cb) = progress {
        cb(0, 3, "roots");
    }
    let roots = collect_roots(refs)?;

    // Phase 2: Mark reachable objects
    if let Some(cb) = progress {
        cb(1, 3, "mark");
    }
    let reachable = mark_reachable(object_store, &roots, &mut report)?;

    // Phase 3: Sweep unreachable objects
    if let Some(cb) = progress {
        cb(2, 3, "sweep");
    }
    let (deleted, bytes_freed) = sweep_unreachable(object_store, &reachable, &config, &mut report, progress)?;

    report.objects_deleted = deleted;
    report.bytes_freed = bytes_freed;

    if let Some(cb) = progress {
        cb(3, 3, "done");
    }

    Ok(report)
}

/// Collect all GC roots (HEAD, STAGE, refs/*).
fn collect_roots(refs: &Refs) -> Result<Vec<ObjectId>> {
    let mut roots = Vec::new();

    // Add HEAD if it exists
    if let Ok(head) = refs.read_head() {
        roots.push(head);
    }

    // Add STAGE if it exists
    if let Ok(Some(stage)) = refs.read_stage() {
        roots.push(stage);
    }

    // Add all refs/* entries
    for (_name, id) in refs.list_refs()? {
        roots.push(id);
    }

    Ok(roots)
}

/// Mark all reachable objects starting from roots.
///
/// Uses BFS to traverse the object graph and mark all reachable objects.
fn mark_reachable(
    store: &ObjectStore,
    roots: &[ObjectId],
    report: &mut GcReport,
) -> Result<HashSet<ObjectId>> {
    let mut reachable = HashSet::new();
    let mut queue = VecDeque::from_iter(roots.iter().copied());

    while let Some(id) = queue.pop_front() {
        // Skip if already marked
        if !reachable.insert(id) {
            continue;
        }

        // Try to load as commit and traverse its references
        if let Ok(commit) = store.get_typed::<Commit>(id) {
            // Add parents
            for parent in &commit.parents {
                queue.push_back(*parent);
            }

            // Add root tree
            queue.push_back(commit.root_tree);

            // Add edge batches
            for batch_id in &commit.edge_batches {
                queue.push_back(*batch_id);
            }

            // Add narrative refs
            for narrative_ref in &commit.narrative_refs {
                queue.push_back(narrative_ref.blob_id);
            }

            // Add optional snapshots
            if let Some(cargo_snapshot) = commit.cargo_snapshot {
                queue.push_back(cargo_snapshot);
            }
            if let Some(rust_snapshot) = commit.rust_snapshot {
                queue.push_back(rust_snapshot);
            }
            if let Some(diagnostics_snapshot) = commit.diagnostics_snapshot {
                queue.push_back(diagnostics_snapshot);
            }
        }

        // Try to load as tree and traverse its entries
        if let Ok(tree) = store.get_typed::<crate::types::Tree>(id) {
            for entry in &tree.entries {
                queue.push_back(entry.id);
            }
        }
    }

    report.objects_reachable = reachable.len();
    Ok(reachable)
}

/// Sweep unreachable objects that are older than grace period.
fn sweep_unreachable(
    store: &mut ObjectStore,
    reachable: &HashSet<ObjectId>,
    config: &GcConfig,
    report: &mut GcReport,
    progress: Option<&GcProgressCallback>,
) -> Result<(usize, u64)> {
    let mut deleted = 0;
    let mut bytes_freed = 0u64;

    // Get grace period cutoff time
    let grace_period = if config.aggressive {
        Duration::from_secs(0)
    } else {
        Duration::from_secs(config.grace_period_days as u64 * 24 * 60 * 60)
    };

    let cutoff_time = SystemTime::now() - grace_period;

    // List all objects
    let all_objects = store.list_all_objects()?;
    report.objects_scanned = all_objects.len();
    let total = all_objects.len();

    // Sweep unreachable objects
    for (idx, (id, size, mtime)) in all_objects.into_iter().enumerate() {
        if let Some(cb) = progress {
            if idx % 100 == 0 || idx == total - 1 {
                cb(idx + 1, total, "sweep");
            }
        }
        // Skip reachable objects
        if reachable.contains(&id) {
            continue;
        }

        // Check grace period (skip recent objects unless aggressive)
        if mtime > cutoff_time {
            continue;
        }

        // Delete object
        if config.dry_run {
            // Dry run: just count what would be deleted
            deleted += 1;
            bytes_freed += size;
        } else {
            // Actually delete
            match store.delete(id) {
                Ok(()) => {
                    deleted += 1;
                    bytes_freed += size;
                }
                Err(e) => {
                    report
                        .errors
                        .push(format!("Failed to delete {}: {}", id.as_hex(), e));
                }
            }
        }
    }

    Ok((deleted, bytes_freed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Tree, TreeEntry};
    use tempfile::TempDir;

    #[test]
    fn test_gc_config_default() {
        let config = GcConfig::default();
        assert!(!config.dry_run);
        assert_eq!(config.grace_period_days, 7);
        assert!(!config.aggressive);
    }

    #[test]
    fn test_mark_reachable() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // Create a simple commit
        let empty_tree = Tree { entries: vec![] };
        let tree_id = store.put_typed(&empty_tree).unwrap();

        let commit = Commit {
            parents: vec![],
            timestamp_unix: 0,
            message: "Test commit".into(),
            root_tree: tree_id,
            edge_batches: vec![],
            narrative_refs: vec![],
            cargo_snapshot: None,
            rust_snapshot: None,
            diagnostics_snapshot: None,
            commit_type: None,
        };
        let commit_id = store.put_typed(&commit).unwrap();

        // Mark reachable
        let mut report = GcReport::default();
        let reachable = mark_reachable(&store, &[commit_id], &mut report).unwrap();

        // Both commit and tree should be reachable
        assert!(reachable.contains(&commit_id));
        assert!(reachable.contains(&tree_id));
        assert_eq!(reachable.len(), 2);
    }

    #[test]
    fn test_gc_dry_run() {
        let tmp = TempDir::new().unwrap();
        let ctx_root = tmp.path().join(".ctx");
        std::fs::create_dir_all(&ctx_root).unwrap();

        let mut store = ObjectStore::new(ctx_root.join("objects"));
        let refs = Refs::new(&ctx_root);

        // Create some objects
        let blob1 = store.put_blob(b"keep me").unwrap();
        let blob2 = store.put_blob(b"delete me").unwrap();

        // Create a commit referencing only blob1
        let tree = Tree {
            entries: vec![TreeEntry {
                name: "file1.txt".into(),
                kind: crate::types::TreeEntryKind::Blob,
                id: blob1,
            }],
        };
        let tree_id = store.put_typed(&tree).unwrap();

        let commit = Commit {
            parents: vec![],
            timestamp_unix: 0,
            message: "Test".into(),
            root_tree: tree_id,
            edge_batches: vec![],
            narrative_refs: vec![],
            cargo_snapshot: None,
            rust_snapshot: None,
            diagnostics_snapshot: None,
            commit_type: None,
        };
        let commit_id = store.put_typed(&commit).unwrap();
        refs.write_head(commit_id).unwrap();

        // Run GC in dry-run mode
        let config = GcConfig {
            dry_run: true,
            grace_period_days: 0,
            aggressive: true,
        };

        let report = gc(&refs, &mut store, config, None).unwrap();

        // blob2 should be marked for deletion but not actually deleted
        assert_eq!(report.objects_deleted, 1);
        assert!(store.exists(blob2)); // Still exists because dry-run
    }

    #[test]
    fn test_gc_with_grace_period() {
        let tmp = TempDir::new().unwrap();
        let ctx_root = tmp.path().join(".ctx");
        std::fs::create_dir_all(&ctx_root).unwrap();

        let mut store = ObjectStore::new(ctx_root.join("objects"));
        let refs = Refs::new(&ctx_root);

        // Create an unreachable object
        let blob = store.put_blob(b"recent").unwrap();

        // Run GC with grace period (should not delete recent objects)
        let config = GcConfig {
            dry_run: false,
            grace_period_days: 7,
            aggressive: false,
        };

        let report = gc(&refs, &mut store, config, None).unwrap();

        // Recent object should not be deleted
        assert_eq!(report.objects_deleted, 0);
        assert!(store.exists(blob));
    }
}
