//! Staging area management for work-in-progress commits.

use crate::error::{CtxError, Result};
use crate::types::{Commit, CommitType, NarrativeRef, Observation, Tree, WorkCommit};
use crate::{ObjectId, ObjectStore};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Walks the staging chain from STAGE back to base commit.
///
/// Returns WorkCommits in order from oldest to newest (reversed chronological).
pub fn walk_staging_chain(
    staging_head: ObjectId,
    base_commit: ObjectId,
    object_store: &ObjectStore,
) -> Result<Vec<(ObjectId, WorkCommit)>> {
    let mut chain = Vec::new();
    let mut current = staging_head;

    loop {
        if current == base_commit {
            break;
        }

        let work: WorkCommit =
            object_store
                .get_typed(current)
                .map_err(|_| CtxError::StagingCorrupted {
                    reason: format!("Missing WorkCommit: {}", current.as_hex()),
                })?;

        chain.push((current, work.clone()));

        if let Some(&parent) = work.parents.first() {
            current = parent;
        } else {
            // Reached end of chain without finding base
            if current != base_commit {
                return Err(CtxError::StagingCorrupted {
                    reason: format!(
                        "Chain ended at {} without reaching base {}",
                        current.as_hex(),
                        base_commit.as_hex()
                    ),
                });
            }
            break;
        }
    }

    // Reverse to get oldest-first order
    chain.reverse();

    Ok(chain)
}

/// Compacts a staging chain into a canonical commit.
///
/// Walks the staging chain, aggregates all work, creates edges,
/// and produces a single canonical Commit.
pub fn compact_staging(
    staging_head: ObjectId,
    base_commit: ObjectId,
    message: &str,
    commit_type: CommitType,
    object_store: &ObjectStore,
) -> Result<Commit> {
    let chain = walk_staging_chain(staging_head, base_commit, object_store)?;
    let base: Commit = object_store.get_typed(base_commit)?;
    let narrative_refs = collect_narrative_refs_from_chain(&chain);
    let observations = collect_observations_from_chain(&chain, object_store)?;
    let root_tree = build_tree_from_observations(&observations, base.root_tree, object_store)?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before Unix epoch")
        .as_secs();

    let edge_batch_ids = extract_edges_from_observations(
        &observations,
        base_commit, // Will be updated after commit is created
        now,
        object_store,
    )?;

    let commit = Commit {
        parents: vec![base_commit],
        timestamp_unix: now,
        message: message.to_string(),
        root_tree,
        edge_batches: edge_batch_ids,
        narrative_refs,
        cargo_snapshot: base.cargo_snapshot,
        rust_snapshot: base.rust_snapshot,
        diagnostics_snapshot: base.diagnostics_snapshot,
        commit_type: Some(commit_type),
    };

    Ok(commit)
}

/// Collects all observations from a staging chain.
///
/// This is an internal utility used by tests and debugging.
/// Not part of the public API.
#[allow(dead_code)]
pub(crate) fn collect_observations(
    staging_head: ObjectId,
    base_commit: ObjectId,
    object_store: &ObjectStore,
) -> Result<Vec<Observation>> {
    let chain = walk_staging_chain(staging_head, base_commit, object_store)?;
    collect_observations_from_chain(&chain, object_store)
}

/// Collects all narrative refs from a staging chain.
///
/// This is an internal utility for debugging and future features.
/// Not part of the public API.
#[allow(dead_code)]
pub(crate) fn collect_narrative_refs(
    staging_head: ObjectId,
    base_commit: ObjectId,
    object_store: &ObjectStore,
) -> Result<Vec<NarrativeRef>> {
    let chain = walk_staging_chain(staging_head, base_commit, object_store)?;
    Ok(collect_narrative_refs_from_chain(&chain))
}

/// Builds a tree from file write observations.
///
/// Takes the latest version of each file path and creates a Tree snapshot.
pub fn build_tree_from_observations(
    observations: &[Observation],
    base_tree_id: ObjectId,
    object_store: &ObjectStore,
) -> Result<ObjectId> {
    // Collect file writes (latest wins)
    let mut file_map: HashMap<String, ObjectId> = HashMap::new();
    for obs in observations {
        if let Observation::FileWrite { path, content_id } = obs {
            file_map.insert(path.clone(), *content_id);
        }
    }

    if file_map.is_empty() {
        return Ok(base_tree_id);
    }

    build_tree_from_paths(&file_map, object_store)
}

/// Builds a tree structure from a map of file paths to content IDs.
fn build_tree_from_paths(
    file_map: &HashMap<String, ObjectId>,
    object_store: &ObjectStore,
) -> Result<ObjectId> {
    use crate::types::{TreeEntry, TreeEntryKind};
    use std::collections::BTreeMap;

    // Map: directory path -> (filename -> content_id or subtree_id)
    let mut dir_structure: BTreeMap<String, BTreeMap<String, (TreeEntryKind, ObjectId)>> =
        BTreeMap::new();

    for (path, content_id) in file_map {
        let parts: Vec<&str> = path.split('/').collect();

        if parts.is_empty() {
            continue;
        }

        if parts.len() == 1 {
            // File in root directory
            dir_structure
                .entry(String::new())
                .or_default()
                .insert(parts[0].to_string(), (TreeEntryKind::Blob, *content_id));
        } else {
            // File in subdirectory
            let filename = parts.last().unwrap();
            let dir_path = parts[..parts.len() - 1].join("/");

            dir_structure
                .entry(dir_path)
                .or_default()
                .insert(filename.to_string(), (TreeEntryKind::Blob, *content_id));

            // Ensure all parent directories exist in the structure
            let mut current_path = String::new();
            for (i, part) in parts[..parts.len() - 1].iter().enumerate() {
                let parent = if i == 0 {
                    String::new()
                } else {
                    current_path.clone()
                };

                if i > 0 {
                    current_path.push('/');
                }
                current_path.push_str(part);

                // Mark this directory in parent (will be updated with tree ID later)
                dir_structure.entry(parent).or_default().insert(
                    part.to_string(),
                    (TreeEntryKind::Tree, ObjectId::from_bytes([0; 32])), // Placeholder
                );
            }
        }
    }

    // Build trees from deepest level up
    let mut tree_cache: BTreeMap<String, ObjectId> = BTreeMap::new();

    // Sort directory paths by depth (deepest first)
    let mut sorted_dirs: Vec<String> = dir_structure.keys().cloned().collect();
    sorted_dirs.sort_by(|a, b| {
        let depth_a = if a.is_empty() {
            0
        } else {
            a.matches('/').count() + 1
        };
        let depth_b = if b.is_empty() {
            0
        } else {
            b.matches('/').count() + 1
        };
        depth_b.cmp(&depth_a) // Reverse order (deepest first)
    });

    // Build trees from leaves up
    for dir_path in sorted_dirs {
        let entries_map = dir_structure.get(&dir_path).unwrap();
        let mut tree_entries = Vec::new();

        for (name, (kind, id)) in entries_map {
            let actual_id = if *kind == TreeEntryKind::Tree {
                // Look up the actual tree ID for subdirectories
                let subdir_path = if dir_path.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", dir_path, name)
                };

                tree_cache.get(&subdir_path).copied().unwrap_or(*id)
            } else {
                *id
            };

            tree_entries.push(TreeEntry {
                name: name.clone(),
                kind: *kind,
                id: actual_id,
            });
        }

        let tree = Tree::new(tree_entries);
        let tree_id = object_store.put_typed(&tree)?;
        tree_cache.insert(dir_path, tree_id);
    }

    tree_cache
        .get("")
        .copied()
        .ok_or_else(|| CtxError::StagingCorrupted {
            reason: "Failed to build root tree".to_string(),
        })
}

// === Internal Helpers ===

fn collect_observations_from_chain(
    chain: &[(ObjectId, WorkCommit)],
    _object_store: &ObjectStore,
) -> Result<Vec<Observation>> {
    let mut all_observations = Vec::new();

    for (_id, work) in chain {
        if let Ok(observations) = decode_observations(&work.payload) {
            all_observations.extend(observations);
        }
    }

    Ok(all_observations)
}

fn collect_narrative_refs_from_chain(chain: &[(ObjectId, WorkCommit)]) -> Vec<NarrativeRef> {
    let mut all_refs = Vec::new();

    for (_id, work) in chain {
        all_refs.extend(work.narrative_refs.clone());
    }

    all_refs
}

fn decode_observations(payload: &[u8]) -> Result<Vec<Observation>> {
    postcard::from_bytes(payload)
        .map_err(|e| CtxError::Deserialization(format!("Failed to decode observations: {}", e)))
}

/// Extracts edges from observations and creates EdgeBatch objects.
///
/// Currently creates basic UpdatedIn edges for file modifications. These edges
/// help track which files were modified during the session.
fn extract_edges_from_observations(
    observations: &[Observation],
    commit_id: ObjectId,
    created_at: u64,
    object_store: &ObjectStore,
) -> Result<Vec<ObjectId>> {
    use crate::types::{
        Confidence, Edge, EdgeBatch, EdgeLabel, Evidence, EvidenceTool, NodeId, NodeKind,
    };
    use std::collections::BTreeSet;

    // Collect unique file paths that were written (using BTreeSet for determinism)
    let mut written_files: BTreeSet<String> = BTreeSet::new();
    for obs in observations {
        if let Observation::FileWrite { path, .. } = obs {
            written_files.insert(path.clone());
        }
    }

    // If no files were written, return empty list
    if written_files.is_empty() {
        return Ok(vec![]);
    }

    // Create edges: For each modified file, create a self-referencing UpdatedIn edge
    // that tracks the modification. The evidence points to the commit.
    //
    // Note: In a full implementation, these would point to Commit nodes, but
    // NodeKind::Commit doesn't exist yet. For Phase 5, we use self-references
    // with the commit tracked in the evidence.
    let mut edges = Vec::new();
    for file_path in written_files {
        let file_node = NodeId {
            kind: NodeKind::File,
            id: file_path.clone(),
        };

        edges.push(Edge {
            from: file_node.clone(),
            to: file_node, // Self-reference until Commit nodes are added
            label: EdgeLabel::UpdatedIn,
            weight: None,
            evidence: Evidence {
                commit_id,
                tool: EvidenceTool::Human, // Session observations are human-driven
                confidence: Confidence::High,
                span: None,
                blob_id: None,
            },
        });
    }

    // Edges are already sorted (BTreeSet iteration is sorted)

    // Create EdgeBatch
    // Note: We don't store which commit introduces this batch - that can be
    // derived by querying which commit references this EdgeBatch's ObjectId.
    // This avoids self-reference issues in content-addressed storage.
    let edge_batch = EdgeBatch { edges, created_at };

    // Store EdgeBatch
    let edge_batch_id = object_store.put_typed(&edge_batch)?;

    Ok(vec![edge_batch_id])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SessionState, StepKind};
    use tempfile::TempDir;

    fn create_work_commit(
        store: &ObjectStore,
        parent: ObjectId,
        base: ObjectId,
        observations: Vec<Observation>,
    ) -> ObjectId {
        let payload = postcard::to_allocvec(&observations).unwrap();

        let work = WorkCommit {
            parents: vec![parent],
            base,
            session_id: "test-session".to_string(),
            created_at: 1234567890,
            step_kind: StepKind::Note,
            payload,
            narrative_refs: vec![],
            session_state: SessionState::Running,
            task_description: "Test task".to_string(),
        };

        store.put_typed(&work).unwrap()
    }

    #[test]
    fn test_walk_staging_chain() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let base = ObjectId::from_bytes([0; 32]);

        // Create chain: base <- work1 <- work2 <- work3
        let work1 = create_work_commit(
            &store,
            base,
            base,
            vec![Observation::Note {
                content: "step 1".to_string(),
            }],
        );
        let work2 = create_work_commit(
            &store,
            work1,
            base,
            vec![Observation::Note {
                content: "step 2".to_string(),
            }],
        );
        let work3 = create_work_commit(
            &store,
            work2,
            base,
            vec![Observation::Note {
                content: "step 3".to_string(),
            }],
        );

        let chain = walk_staging_chain(work3, base, &store).unwrap();

        assert_eq!(chain.len(), 3);
        // Oldest first
        assert_eq!(chain[0].0, work1);
        assert_eq!(chain[1].0, work2);
        assert_eq!(chain[2].0, work3);
    }

    #[test]
    fn test_collect_observations() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let base = ObjectId::from_bytes([0; 32]);

        let work1 = create_work_commit(
            &store,
            base,
            base,
            vec![Observation::FileRead {
                path: "test1.rs".to_string(),
                content_id: None,
            }],
        );
        let work2 = create_work_commit(
            &store,
            work1,
            base,
            vec![Observation::Note {
                content: "note".to_string(),
            }],
        );

        let observations = collect_observations(work2, base, &store).unwrap();

        assert_eq!(observations.len(), 2);
        assert!(matches!(observations[0], Observation::FileRead { .. }));
        assert!(matches!(observations[1], Observation::Note { .. }));
    }

    #[test]
    fn test_compact_staging() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // Create base commit
        let empty_tree = Tree::new(vec![]);
        let tree_id = store.put_typed(&empty_tree).unwrap();

        let base_commit = Commit {
            parents: vec![],
            timestamp_unix: 1000,
            message: "Base".to_string(),
            root_tree: tree_id,
            edge_batches: vec![],
            narrative_refs: vec![],
            cargo_snapshot: None,
            rust_snapshot: None,
            diagnostics_snapshot: None,
            commit_type: None,
        };
        let base_id = store.put_typed(&base_commit).unwrap();

        // Create staging chain
        let work1 = create_work_commit(&store, base_id, base_id, vec![]);
        let work2 = create_work_commit(&store, work1, base_id, vec![]);

        let commit =
            compact_staging(work2, base_id, "Completed task", CommitType::Normal, &store).unwrap();

        assert_eq!(commit.parents, vec![base_id]);
        assert_eq!(commit.message, "Completed task");
        assert_eq!(commit.commit_type, Some(CommitType::Normal));
    }

    #[test]
    fn test_build_tree_from_observations() {
        use crate::types::TreeEntryKind;

        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // Create base tree (empty)
        let base_tree = Tree::new(vec![]);
        let base_tree_id = store.put_typed(&base_tree).unwrap();

        // Create file observations with nested paths
        let file1_content = store.put_blob(b"content1").unwrap();
        let file2_content = store.put_blob(b"content2").unwrap();
        let file3_content = store.put_blob(b"content3").unwrap();

        let observations = vec![
            Observation::FileWrite {
                path: "src/main.rs".to_string(),
                content_id: file1_content,
            },
            Observation::FileWrite {
                path: "src/lib.rs".to_string(),
                content_id: file2_content,
            },
            Observation::FileWrite {
                path: "tests/test.rs".to_string(),
                content_id: file3_content,
            },
        ];

        // Build tree
        let root_tree_id =
            build_tree_from_observations(&observations, base_tree_id, &store).unwrap();

        // Verify root tree structure
        let root_tree: Tree = store.get_typed(root_tree_id).unwrap();
        assert_eq!(root_tree.entries.len(), 2); // src/ and tests/

        // Should be sorted: src, tests
        assert_eq!(root_tree.entries[0].name, "src");
        assert_eq!(root_tree.entries[0].kind, TreeEntryKind::Tree);
        assert_eq!(root_tree.entries[1].name, "tests");
        assert_eq!(root_tree.entries[1].kind, TreeEntryKind::Tree);

        // Verify src/ subtree
        let src_tree: Tree = store.get_typed(root_tree.entries[0].id).unwrap();
        assert_eq!(src_tree.entries.len(), 2); // main.rs, lib.rs

        // Should be sorted: lib.rs, main.rs
        assert_eq!(src_tree.entries[0].name, "lib.rs");
        assert_eq!(src_tree.entries[0].kind, TreeEntryKind::Blob);
        assert_eq!(src_tree.entries[0].id, file2_content);
        assert_eq!(src_tree.entries[1].name, "main.rs");
        assert_eq!(src_tree.entries[1].kind, TreeEntryKind::Blob);
        assert_eq!(src_tree.entries[1].id, file1_content);

        // Verify tests/ subtree
        let tests_tree: Tree = store.get_typed(root_tree.entries[1].id).unwrap();
        assert_eq!(tests_tree.entries.len(), 1); // test.rs

        assert_eq!(tests_tree.entries[0].name, "test.rs");
        assert_eq!(tests_tree.entries[0].kind, TreeEntryKind::Blob);
        assert_eq!(tests_tree.entries[0].id, file3_content);
    }

    #[test]
    fn test_build_tree_latest_version_wins() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // Create base tree (empty)
        let base_tree = Tree::new(vec![]);
        let base_tree_id = store.put_typed(&base_tree).unwrap();

        // Create multiple versions of the same file
        let file1_v1 = store.put_blob(b"version 1").unwrap();
        let file1_v2 = store.put_blob(b"version 2").unwrap();

        let observations = vec![
            Observation::FileWrite {
                path: "test.rs".to_string(),
                content_id: file1_v1,
            },
            Observation::FileWrite {
                path: "test.rs".to_string(),
                content_id: file1_v2, // This should win
            },
        ];

        // Build tree
        let root_tree_id =
            build_tree_from_observations(&observations, base_tree_id, &store).unwrap();

        // Verify latest version is used
        let root_tree: Tree = store.get_typed(root_tree_id).unwrap();
        assert_eq!(root_tree.entries.len(), 1);
        assert_eq!(root_tree.entries[0].id, file1_v2); // Latest version
    }

    #[test]
    fn test_edge_extraction_from_file_writes() {
        use crate::types::{EdgeLabel, NodeKind};

        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // Create observations with file writes
        let observations = vec![
            Observation::FileWrite {
                path: "src/main.rs".to_string(),
                content_id: ObjectId::from_bytes([1; 32]),
            },
            Observation::FileWrite {
                path: "src/lib.rs".to_string(),
                content_id: ObjectId::from_bytes([2; 32]),
            },
            Observation::FileRead {
                path: "README.md".to_string(),
                content_id: None,
            },
            Observation::Note {
                content: "Updated source files".to_string(),
            },
        ];

        let commit_id = ObjectId::from_bytes([99; 32]);
        let created_at = 1234567890;

        // Extract edges
        let edge_batch_ids =
            extract_edges_from_observations(&observations, commit_id, created_at, &store).unwrap();

        // Should create one EdgeBatch
        assert_eq!(edge_batch_ids.len(), 1);

        // Load the EdgeBatch
        let edge_batch: crate::types::EdgeBatch = store.get_typed(edge_batch_ids[0]).unwrap();

        // Should have 2 edges (one per file write, not for read or note)
        assert_eq!(edge_batch.edges.len(), 2);

        // Check first edge (src/lib.rs comes before src/main.rs alphabetically)
        assert_eq!(edge_batch.edges[0].from.kind, NodeKind::File);
        assert_eq!(edge_batch.edges[0].from.id, "src/lib.rs");
        assert_eq!(edge_batch.edges[0].label, EdgeLabel::UpdatedIn);
        assert_eq!(edge_batch.edges[0].evidence.commit_id, commit_id);

        // Check second edge
        assert_eq!(edge_batch.edges[1].from.kind, NodeKind::File);
        assert_eq!(edge_batch.edges[1].from.id, "src/main.rs");
        assert_eq!(edge_batch.edges[1].label, EdgeLabel::UpdatedIn);

        // Verify batch metadata
        assert_eq!(edge_batch.created_at, created_at);

        // Note: To find which commit introduced this batch, we would query
        // commits to see which one references edge_batch_ids[0].
    }

    #[test]
    fn test_edge_extraction_no_file_writes() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // Create observations with no file writes
        let observations = vec![
            Observation::FileRead {
                path: "test.rs".to_string(),
                content_id: None,
            },
            Observation::Note {
                content: "Just reading".to_string(),
            },
        ];

        let commit_id = ObjectId::from_bytes([99; 32]);
        let created_at = 1234567890;

        // Extract edges
        let edge_batch_ids =
            extract_edges_from_observations(&observations, commit_id, created_at, &store).unwrap();

        // Should create no EdgeBatches (no file writes)
        assert_eq!(edge_batch_ids.len(), 0);
    }

    #[test]
    fn test_build_tree_no_observations_returns_base() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // Create base tree
        let base_tree = Tree::new(vec![]);
        let base_tree_id = store.put_typed(&base_tree).unwrap();

        // No file write observations
        let observations = vec![
            Observation::FileRead {
                path: "test.rs".to_string(),
                content_id: None,
            },
            Observation::Note {
                content: "Just a note".to_string(),
            },
        ];

        // Build tree
        let root_tree_id =
            build_tree_from_observations(&observations, base_tree_id, &store).unwrap();

        // Should return base tree unchanged
        assert_eq!(root_tree_id, base_tree_id);
    }
}
