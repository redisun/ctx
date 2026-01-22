//! Core data types for CTX.

use crate::ObjectId;
use serde::{Deserialize, Serialize};

/// Session state machine states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    /// Agent is actively working on the task.
    Running,

    /// Agent has asked a question and is waiting for user response.
    AwaitingUser {
        /// The question or clarification request.
        question: String,
        /// Unix timestamp when question was asked.
        asked_at: i64,
    },

    /// User sent a message while agent was working (intervention).
    Interrupted {
        /// The user's message that caused the interruption.
        user_message: String,
    },

    /// Agent believes task is complete, awaiting user confirmation.
    PendingComplete {
        /// Summary of what was accomplished.
        summary: String,
    },

    /// User has confirmed completion, ready for compaction.
    Complete,

    /// Session was abandoned or cancelled.
    Aborted {
        /// Reason for abortion.
        reason: String,
    },
}

/// How a commit was created (for distinguishing normal vs auto-compacted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitType {
    /// Normal task completion.
    Normal,

    /// User explicitly abandoned task.
    Abandoned,

    /// Session auto-compacted due to staleness.
    StaleAutoCompact {
        /// How long the session was idle (seconds).
        idle_duration_secs: u64,
    },

    /// Session compacted when user started new task.
    InterruptedByNewTask {
        /// Brief description of the new task.
        new_task_summary: String,
    },
}

/// Single observation during a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Observation {
    /// File was read by the agent.
    FileRead {
        /// Path to the file.
        path: String,
        /// Optional content blob ID.
        content_id: Option<ObjectId>,
    },

    /// File was written by the agent.
    FileWrite {
        /// Path to the file.
        path: String,
        /// Content blob ID.
        content_id: ObjectId,
    },

    /// Command was executed.
    Command {
        /// Command string.
        command: String,
        /// Exit code (if available).
        exit_code: Option<i32>,
        /// Output blob ID (if captured).
        output_id: Option<ObjectId>,
    },

    /// Agent made a note.
    Note {
        /// Note content.
        content: String,
    },

    /// Agent created a plan.
    Plan {
        /// Plan content.
        content: String,
    },
}

/// Source location within a file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// Unique identifier for the file.
    pub file_id: ObjectId,
    /// Version of the file this span refers to.
    pub file_version_id: ObjectId,
    /// Byte offset where span starts.
    pub start_byte: u32,
    /// Byte offset where span ends (exclusive).
    pub end_byte: u32,
    /// Line number where span starts (0-indexed).
    pub start_line: u32,
    /// Column number where span starts (0-indexed).
    pub start_col: u32,
    /// Line number where span ends (0-indexed).
    pub end_line: u32,
    /// Column number where span ends (0-indexed).
    pub end_col: u32,
}

/// Reference to narrative documentation.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NarrativeRef {
    /// Path to narrative file (e.g., "log/2024-01-15.md").
    pub path: String,
    /// Stream within the file (e.g., "##" heading).
    pub stream: Option<String>,
    /// Role that created this narrative (e.g., "agent", "user").
    pub role: String,
    /// ObjectId of the narrative content blob.
    pub blob_id: ObjectId,
}

/// File hierarchy tree.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    /// Sorted list of entries (MUST be sorted by name for determinism).
    pub entries: Vec<TreeEntry>,
}

impl Tree {
    /// Creates a new tree, automatically sorting entries by name.
    pub fn new(mut entries: Vec<TreeEntry>) -> Self {
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Self { entries }
    }
}

/// Entry in a tree.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// Name of the entry (filename or directory name).
    pub name: String,
    /// Type of entry.
    pub kind: TreeEntryKind,
    /// ObjectId pointing to the content (blob or tree).
    pub id: ObjectId,
}

/// Type of tree entry.
#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeEntryKind {
    /// Regular file (blob).
    Blob = 1,
    /// Directory (subtree).
    Tree = 2,
}

/// Identifier for a node in the knowledge graph.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    /// Type of node.
    pub kind: NodeKind,
    /// Unique identifier within the kind.
    pub id: String,
}

/// Type of node in the knowledge graph.
#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeKind {
    /// Source file.
    File = 1,
    /// Rust module.
    Module = 2,
    /// Code item (function, struct, etc.).
    Item = 3,
    /// Cargo package.
    Package = 4,
    /// Cargo target.
    Target = 5,
    /// Rust crate.
    Crate = 6,
    /// Task or work item.
    Task = 7,
    /// Note or comment.
    Note = 8,
    /// Design decision.
    Decision = 9,
    /// Diagnostic message.
    Diagnostic = 10,
}

/// Type of edge relationship.
#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeLabel {
    // Structural (1-9)
    /// Parent-child containment.
    Contains = 1,
    /// Definition site.
    Defines = 2,
    /// Version relationship.
    HasVersion = 3,

    // Dependencies (10-19)
    /// Package/crate dependency.
    DependsOn = 10,
    /// Target membership.
    TargetOf = 11,
    /// Crate derived from target.
    CrateFromTarget = 12,

    // Code relationships (20-29)
    /// Import/use statement.
    Imports = 20,
    /// Reference to symbol.
    References = 21,
    /// Function/method call.
    Calls = 22,
    /// Trait implementation.
    Implements = 23,
    /// Type usage.
    UsesType = 24,

    // Documentation (30-39)
    /// Mentioned in narrative.
    Mentions = 30,
    /// Updated in commit/session.
    UpdatedIn = 31,
    /// Derived from source.
    DerivedFrom = 32,
}

/// Indicates which tool or system extracted the edge relationship.
///
/// Different tools provide different levels of precision and coverage. For example,
/// Cargo provides definitive dependency information, while LLM inference is more
/// speculative but can identify semantic relationships.
#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceTool {
    /// Cargo metadata.
    Cargo = 1,
    /// Syntax parser.
    Parser = 2,
    /// rust-analyzer.
    RustAnalyzer = 3,
    /// Human annotation.
    Human = 4,
    /// LLM inference.
    Llm = 5,
}

/// Indicates the reliability of edge evidence based on the tool and method used to extract it.
///
/// High confidence means the relationship is definitively established (e.g., parsed from source).
/// Medium confidence indicates strong heuristic evidence. Low confidence is used for
/// speculative or inferred relationships.
#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// High confidence.
    High = 1,
    /// Medium confidence.
    Medium = 2,
    /// Low confidence.
    Low = 3,
}

/// Evidence supporting an edge.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// Commit where evidence was recorded.
    pub commit_id: ObjectId,
    /// Tool that provided the evidence.
    pub tool: EvidenceTool,
    /// Confidence level.
    pub confidence: Confidence,
    /// Source location (if applicable).
    pub span: Option<Span>,
    /// Related blob (if applicable).
    pub blob_id: Option<ObjectId>,
}

/// Edge in the knowledge graph.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// Source node.
    pub from: NodeId,
    /// Target node.
    pub to: NodeId,
    /// Edge label.
    pub label: EdgeLabel,
    /// Optional weight (fixed-point: 1500 = 1.5).
    pub weight: Option<u32>,
    /// Evidence supporting this edge.
    pub evidence: Evidence,
}

/// Batch of edges introduced together.
///
/// Note: To find which commit introduced this EdgeBatch, query commits
/// to see which one references this EdgeBatch's ObjectId. This avoids
/// self-reference issues in content-addressed storage.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EdgeBatch {
    /// Edges in this batch.
    pub edges: Vec<Edge>,
    /// Timestamp when batch was created (Unix seconds).
    pub created_at: u64,
}

/// Canonical commit representing a stable checkpoint.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Parent commit IDs.
    pub parents: Vec<ObjectId>,
    /// Timestamp (Unix seconds).
    pub timestamp_unix: u64,
    /// Commit message.
    pub message: String,
    /// Root tree snapshot.
    pub root_tree: ObjectId,
    /// Edge batches added in this commit.
    ///
    /// Note: These are ObjectIds that reference EdgeBatch objects in the object store,
    /// not the EdgeBatch structs themselves. This avoids data duplication and allows
    /// edge batches to be shared between commits. Use `CtxRepo::load_edge_batches()`
    /// to load the actual EdgeBatch objects.
    pub edge_batches: Vec<ObjectId>,
    /// References to narrative documentation.
    pub narrative_refs: Vec<NarrativeRef>,
    /// Cargo.toml snapshot (if applicable).
    pub cargo_snapshot: Option<ObjectId>,
    /// Rust file snapshots (if applicable).
    pub rust_snapshot: Option<ObjectId>,
    /// Diagnostic snapshot (if applicable).
    pub diagnostics_snapshot: Option<ObjectId>,
    /// How this commit was created (None for legacy commits).
    pub commit_type: Option<CommitType>,
}

/// Type of work step.
#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// Session start.
    SessionStart = 1,
    /// File read operation.
    FileRead = 2,
    /// File write operation.
    FileWrite = 3,
    /// Command execution.
    CommandRun = 4,
    /// Note or annotation.
    Note = 5,
    /// Plan or design.
    Plan = 6,
    /// Compaction to canonical.
    Compact = 7,
}

/// Work commit in staging area.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct WorkCommit {
    /// Parent work commits.
    pub parents: Vec<ObjectId>,
    /// Base canonical commit.
    pub base: ObjectId,
    /// Session identifier.
    pub session_id: String,
    /// Timestamp (Unix seconds).
    pub created_at: u64,
    /// Type of step.
    pub step_kind: StepKind,
    /// Step payload (serialized data).
    pub payload: Vec<u8>,
    /// Narrative references for this step.
    pub narrative_refs: Vec<NarrativeRef>,
    /// Current session state at this step.
    pub session_state: SessionState,
    /// Task description for the session.
    pub task_description: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ObjectStore;
    use tempfile::TempDir;

    #[test]
    fn test_tree_sorts_entries() {
        let entry1 = TreeEntry {
            name: "z.txt".to_string(),
            kind: TreeEntryKind::Blob,
            id: ObjectId::from_bytes([0; 32]),
        };
        let entry2 = TreeEntry {
            name: "a.txt".to_string(),
            kind: TreeEntryKind::Blob,
            id: ObjectId::from_bytes([1; 32]),
        };
        let entry3 = TreeEntry {
            name: "m.txt".to_string(),
            kind: TreeEntryKind::Blob,
            id: ObjectId::from_bytes([2; 32]),
        };

        let tree = Tree::new(vec![entry1.clone(), entry2.clone(), entry3.clone()]);

        // Should be sorted: a, m, z
        assert_eq!(tree.entries[0].name, "a.txt");
        assert_eq!(tree.entries[1].name, "m.txt");
        assert_eq!(tree.entries[2].name, "z.txt");
    }

    #[test]
    fn test_tree_deterministic() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let entry1 = TreeEntry {
            name: "b.txt".to_string(),
            kind: TreeEntryKind::Blob,
            id: ObjectId::from_bytes([0; 32]),
        };
        let entry2 = TreeEntry {
            name: "a.txt".to_string(),
            kind: TreeEntryKind::Blob,
            id: ObjectId::from_bytes([1; 32]),
        };

        // Different insertion order should produce same ID
        let tree1 = Tree::new(vec![entry1.clone(), entry2.clone()]);
        let tree2 = Tree::new(vec![entry2.clone(), entry1.clone()]);

        let id1 = store.put_typed(&tree1).unwrap();
        let id2 = store.put_typed(&tree2).unwrap();

        assert_eq!(id1, id2);
    }

    #[test]
    fn test_commit_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let commit = Commit {
            parents: vec![],
            timestamp_unix: 1234567890,
            message: "Test commit".to_string(),
            root_tree: ObjectId::from_bytes([0; 32]),
            edge_batches: vec![],
            narrative_refs: vec![],
            cargo_snapshot: None,
            rust_snapshot: None,
            diagnostics_snapshot: None,
            commit_type: None,
        };

        let id = store.put_typed(&commit).unwrap();
        let retrieved: Commit = store.get_typed(id).unwrap();

        assert_eq!(commit, retrieved);
    }

    #[test]
    fn test_edge_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let edge_batch = EdgeBatch {
            edges: vec![Edge {
                from: NodeId {
                    kind: NodeKind::File,
                    id: "main.rs".to_string(),
                },
                to: NodeId {
                    kind: NodeKind::Item,
                    id: "main".to_string(),
                },
                label: EdgeLabel::Defines,
                weight: Some(1000),
                evidence: Evidence {
                    commit_id: ObjectId::from_bytes([0; 32]),
                    tool: EvidenceTool::Parser,
                    confidence: Confidence::High,
                    span: None,
                    blob_id: None,
                },
            }],
            created_at: 1234567890,
        };

        let id = store.put_typed(&edge_batch).unwrap();
        let retrieved: EdgeBatch = store.get_typed(id).unwrap();

        assert_eq!(edge_batch, retrieved);
    }

    #[test]
    fn test_work_commit_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let work_commit = WorkCommit {
            parents: vec![],
            base: ObjectId::from_bytes([0; 32]),
            session_id: "session-123".to_string(),
            created_at: 1234567890,
            step_kind: StepKind::FileRead,
            payload: b"test payload".to_vec(),
            narrative_refs: vec![],
            session_state: SessionState::Running,
            task_description: "Test task".to_string(),
        };

        let id = store.put_typed(&work_commit).unwrap();
        let retrieved: WorkCommit = store.get_typed(id).unwrap();

        assert_eq!(work_commit, retrieved);
    }

    #[test]
    fn test_all_node_kinds_serialize() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let kinds = vec![
            NodeKind::File,
            NodeKind::Module,
            NodeKind::Item,
            NodeKind::Package,
            NodeKind::Target,
            NodeKind::Crate,
            NodeKind::Task,
            NodeKind::Note,
            NodeKind::Decision,
            NodeKind::Diagnostic,
        ];

        for kind in kinds {
            let node_id = NodeId {
                kind,
                id: "test".to_string(),
            };
            let id = store.put_typed(&node_id).unwrap();
            let retrieved: NodeId = store.get_typed(id).unwrap();
            assert_eq!(node_id, retrieved);
        }
    }
}
