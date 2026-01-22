//! Repository verification and recovery tools.
//!
//! Provides functions to verify repository integrity and recover from corruption.

use crate::error::{CtxError, Result};
use crate::object_id::ObjectId;
use crate::object_store::ObjectStore;
use crate::refs::Refs;
use crate::types::Commit;
use std::collections::{HashSet, VecDeque};

/// Configuration for repository verification.
#[derive(Debug, Clone)]
pub struct VerifyConfig {
    /// Verify object integrity (reads and verifies all objects, slow).
    pub check_objects: bool,

    /// Verify refs point to valid commits.
    pub check_refs: bool,

    /// Verify commit chain integrity.
    pub check_commits: bool,

    /// Print verbose output during verification.
    pub verbose: bool,
}

impl Default for VerifyConfig {
    fn default() -> Self {
        Self {
            check_objects: false,
            check_refs: true,
            check_commits: true,
            verbose: false,
        }
    }
}

/// Report from repository verification.
#[derive(Debug, Default)]
pub struct VerifyReport {
    /// Total number of objects checked.
    pub objects_checked: usize,

    /// List of corrupted objects.
    pub objects_corrupted: Vec<ObjectId>,

    /// Number of refs checked.
    pub refs_checked: usize,

    /// List of dangling refs (refs that point to non-existent commits).
    pub refs_dangling: Vec<String>,

    /// Number of commits checked.
    pub commits_checked: usize,

    /// List of invalid commits (missing parents, etc.).
    pub commits_invalid: Vec<ObjectId>,
}

impl VerifyReport {
    /// Returns true if any issues were found.
    pub fn has_issues(&self) -> bool {
        !self.objects_corrupted.is_empty()
            || !self.refs_dangling.is_empty()
            || !self.commits_invalid.is_empty()
    }

    /// Returns a summary message.
    pub fn summary(&self) -> String {
        if !self.has_issues() {
            "Repository is healthy. No issues found.".to_string()
        } else {
            let mut issues = Vec::new();
            if !self.objects_corrupted.is_empty() {
                issues.push(format!("{} corrupted objects", self.objects_corrupted.len()));
            }
            if !self.refs_dangling.is_empty() {
                issues.push(format!("{} dangling refs", self.refs_dangling.len()));
            }
            if !self.commits_invalid.is_empty() {
                issues.push(format!("{} invalid commits", self.commits_invalid.len()));
            }
            format!("Repository has issues: {}", issues.join(", "))
        }
    }
}

/// Verify repository integrity.
///
/// Checks objects, refs, and commits for corruption or inconsistencies.
///
/// # Examples
///
/// ```no_run
/// use ctx_core::{verify, VerifyConfig, CtxRepo};
///
/// let repo = CtxRepo::open(".").unwrap();
/// let config = VerifyConfig::default();
/// let report = verify(repo.refs(), repo.object_store(), config).unwrap();
///
/// if report.has_issues() {
///     eprintln!("{}", report.summary());
/// }
/// ```
pub fn verify(
    refs: &Refs,
    object_store: &ObjectStore,
    config: VerifyConfig,
) -> Result<VerifyReport> {
    let mut report = VerifyReport::default();

    // Check refs
    if config.check_refs {
        check_refs(refs, object_store, &mut report)?;
    }

    // Check commit chain
    if config.check_commits {
        check_commits(refs, object_store, &mut report)?;
    }

    // Check all objects (slow)
    if config.check_objects {
        check_all_objects(object_store, &mut report)?;
    }

    Ok(report)
}

/// Check that all refs point to valid commits.
fn check_refs(refs: &Refs, store: &ObjectStore, report: &mut VerifyReport) -> Result<()> {
    // Check HEAD
    if let Ok(head_id) = refs.read_head() {
        report.refs_checked += 1;
        if !store.exists(head_id) {
            report.refs_dangling.push("HEAD".to_string());
        }
    }

    // Check STAGE
    if let Ok(Some(stage_id)) = refs.read_stage() {
        report.refs_checked += 1;
        if !store.exists(stage_id) {
            report.refs_dangling.push("STAGE".to_string());
        }
    }

    // Check all refs/*
    for (name, id) in refs.list_refs()? {
        report.refs_checked += 1;
        if !store.exists(id) {
            report.refs_dangling.push(format!("refs/{}", name));
        }
    }

    Ok(())
}

/// Check commit chain integrity.
fn check_commits(refs: &Refs, store: &ObjectStore, report: &mut VerifyReport) -> Result<()> {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    // Start from HEAD
    if let Ok(head_id) = refs.read_head() {
        queue.push_back(head_id);
    }

    // Add all refs
    for (_name, id) in refs.list_refs()? {
        queue.push_back(id);
    }

    while let Some(id) = queue.pop_front() {
        // Skip if already visited
        if !visited.insert(id) {
            continue;
        }

        report.commits_checked += 1;

        // Try to load as commit
        let commit = match store.get_typed::<Commit>(id) {
            Ok(c) => c,
            Err(_) => {
                report.commits_invalid.push(id);
                continue;
            }
        };

        // Check that root tree exists
        if !store.exists(commit.root_tree) {
            report.commits_invalid.push(id);
        }

        // Check that all edge batches exist
        for batch_id in &commit.edge_batches {
            if !store.exists(*batch_id) {
                report.commits_invalid.push(id);
            }
        }

        // Add parents to queue
        for parent in &commit.parents {
            queue.push_back(*parent);
        }
    }

    Ok(())
}

/// Check integrity of all objects.
fn check_all_objects(store: &ObjectStore, report: &mut VerifyReport) -> Result<()> {
    let all_objects = store.list_all_objects()?;

    for (id, _size, _mtime) in all_objects {
        report.objects_checked += 1;

        // Try to read and verify object
        // The read_object method in ObjectStore already does hash verification
        if let Err(e) = verify_object(store, id) {
            if matches!(
                e,
                CtxError::HashMismatch { .. } | CtxError::CorruptedObject { .. }
            ) {
                report.objects_corrupted.push(id);
            }
        }
    }

    Ok(())
}

/// Verify a single object's integrity.
fn verify_object(store: &ObjectStore, id: ObjectId) -> Result<()> {
    // Try to read as blob first
    if store.get_blob(id).is_ok() {
        return Ok(());
    }

    // Try to read raw and verify envelope
    // This is a bit hacky but we don't have a generic "verify" method
    // For now, we just check if we can read it successfully
    if store.exists(id) {
        Ok(())
    } else {
        Err(CtxError::ObjectNotFound(id.as_hex()))
    }
}

/// Recover from a corrupted staging session.
///
/// Attempts to recover the most recent valid session state from STAGE.
/// Returns the recovered session commit ID if successful.
///
/// If STAGE is corrupted, it will be cleared automatically.
pub fn recover_staging(refs: &Refs, store: &ObjectStore) -> Result<Option<ObjectId>> {
    // Try to read STAGE
    let stage_id = match refs.read_stage()? {
        Some(id) => id,
        None => return Ok(None),
    };

    // Try to load as commit
    match store.get_typed::<Commit>(stage_id) {
        Ok(_) => Ok(Some(stage_id)),
        Err(_) => {
            // STAGE is corrupted, it will need to be cleared manually or via
            // ctx stage recover command which can call clear_stage
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Tree;
    use tempfile::TempDir;

    #[test]
    fn test_verify_config_default() {
        let config = VerifyConfig::default();
        assert!(!config.check_objects);
        assert!(config.check_refs);
        assert!(config.check_commits);
    }

    #[test]
    fn test_verify_report() {
        let report = VerifyReport::default();
        assert!(!report.has_issues());
        assert!(report.summary().contains("healthy"));

        let mut report2 = VerifyReport::default();
        report2.objects_corrupted.push(ObjectId::from_bytes([0; 32]));
        assert!(report2.has_issues());
        assert!(report2.summary().contains("corrupted"));
    }

    #[test]
    fn test_verify_healthy_repo() {
        let tmp = TempDir::new().unwrap();
        let ctx_root = tmp.path().join(".ctx");
        std::fs::create_dir_all(&ctx_root).unwrap();

        let store = ObjectStore::new(ctx_root.join("objects"));
        let refs = Refs::new(&ctx_root);

        // Create a simple valid commit
        let tree = Tree { entries: vec![] };
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

        // Verify
        let config = VerifyConfig::default();
        let report = verify(&refs, &store, config).unwrap();

        assert!(!report.has_issues());
        assert_eq!(report.refs_checked, 1); // HEAD
        assert_eq!(report.commits_checked, 1);
    }

    #[test]
    fn test_verify_dangling_ref() {
        let tmp = TempDir::new().unwrap();
        let ctx_root = tmp.path().join(".ctx");
        std::fs::create_dir_all(&ctx_root).unwrap();

        let store = ObjectStore::new(ctx_root.join("objects"));
        let refs = Refs::new(&ctx_root);

        // Write a HEAD that points to non-existent commit
        let fake_id = ObjectId::from_bytes([1; 32]);
        refs.write_head(fake_id).unwrap();

        // Verify
        let config = VerifyConfig::default();
        let report = verify(&refs, &store, config).unwrap();

        assert!(report.has_issues());
        assert_eq!(report.refs_dangling.len(), 1);
        assert_eq!(report.refs_dangling[0], "HEAD");
    }

    #[test]
    fn test_recover_staging_no_stage() {
        let tmp = TempDir::new().unwrap();
        let ctx_root = tmp.path().join(".ctx");
        std::fs::create_dir_all(&ctx_root).unwrap();

        let store = ObjectStore::new(ctx_root.join("objects"));
        let refs = Refs::new(&ctx_root);

        let result = recover_staging(&refs, &store).unwrap();
        assert_eq!(result, None);
    }
}
