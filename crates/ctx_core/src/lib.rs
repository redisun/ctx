//! CTX Core Library
//!
//! A context management system for coding agents, providing:
//! - Content-addressed object storage
//! - Semantic relationship graphs
//! - Human-readable narrative
//! - Intelligent retrieval
//!
//! # Quick Start
//!
//! ```
//! use ctx_core::{ObjectStore, ObjectId};
//! use tempfile::TempDir;
//!
//! let tmp = TempDir::new().unwrap();
//! let store = ObjectStore::new(tmp.path().join("objects"));
//!
//! // Store a blob
//! let id = store.put_blob(b"hello world").unwrap();
//!
//! // Retrieve it
//! let data = store.get_blob(id).unwrap();
//! assert_eq!(data, b"hello world");
//! ```
//!
//! # Features
//!
//! ## Content-Addressed Storage
//!
//! Objects are stored using BLAKE3 content hashing with zstd compression:
//!
//! ```
//! use ctx_core::ObjectStore;
//! use tempfile::TempDir;
//!
//! let tmp = TempDir::new().unwrap();
//! let store = ObjectStore::new(tmp.path().join("objects"));
//!
//! // Same content = same ID (deduplication)
//! let id1 = store.put_blob(b"content").unwrap();
//! let id2 = store.put_blob(b"content").unwrap();
//! assert_eq!(id1, id2);
//! ```
//!
//! ## Typed Objects
//!
//! Store and retrieve structured data with deterministic serialization:
//!
//! ```
//! use ctx_core::ObjectStore;
//! use serde::{Serialize, Deserialize};
//! use tempfile::TempDir;
//!
//! #[derive(Serialize, Deserialize, PartialEq, Debug)]
//! struct Config {
//!     name: String,
//!     value: i32,
//! }
//!
//! let tmp = TempDir::new().unwrap();
//! let store = ObjectStore::new(tmp.path().join("objects"));
//!
//! let config = Config { name: "test".into(), value: 42 };
//! let id = store.put_typed(&config).unwrap();
//!
//! let loaded: Config = store.get_typed(id).unwrap();
//! assert_eq!(loaded, config);
//! ```

mod cargo;
mod classification;
mod config;
mod error;
mod gc;
mod graph;
mod index;
mod lsp;
mod narrative;
mod object_id;
mod object_store;
mod pack;
mod refs;
mod repo;
mod session;
mod session_handler;
mod staging;
mod types;
mod verify;

pub use cargo::{
    CargoAnalysisReport, CargoMetadataSnapshot, DepKind, DepKindInfo, Package, PackageDep, Resolve,
    ResolveNode, ResolvedDep, Target, TargetKind,
};
pub use config::{
    CleanupReport, Config, GcConfig as ConfigGcConfig, SearchConfig, SessionConfig,
    StaleSessionConfig, StaleSessionStatus, StorageConfig,
};
pub use error::{CtxError, Result};
pub use gc::{gc, GcConfig, GcReport};
pub use graph::{
    adjacency_to_dot, compute_scc, expand_from_seeds, expansion_to_dot, AdjacencyList,
    ExpansionConfig, ExpansionResult, SccId, SccView,
};
pub use index::{CommitInfo, EdgeDirection, Index, NameNamespace, INDEX_SCHEMA_VERSION};
pub use lsp::{AnalyzedItem, CallInfo, FileAnalysis, ItemKind, RustAnalyzer};
pub use narrative::{NarrativeSpace, TaskInfo};
pub use object_id::ObjectId;
pub use object_store::ObjectStore;
pub use pack::{
    build_pack, estimate_tokens, parse_query_for_seeds, ChunkKind, GraphContext, PromptPack,
    RetrievalConfig, RetrievedChunk, TokenBudget,
};
pub use refs::Refs;
pub use classification::{classify_message, ClassificationContext, MessageClassification};
pub use repo::{AnalysisReport, CtxRepo, FileAnalysisReport, RecoverySummary};
pub use session::Session;
pub use session_handler::{PendingAction, SessionHandler, SessionResponse, UserChoice};
pub use types::*;
pub use verify::{recover_staging, verify, VerifyConfig, VerifyReport};

/// Time provider trait for testing.
///
/// Allows injecting controlled time into sessions for testing stale session behavior.
/// This is always available but only used when explicitly set via `with_time_provider()`.
pub trait TimeProvider: Send + Sync {
    /// Returns the current Unix timestamp in seconds.
    fn now(&self) -> i64;
}

impl<F> TimeProvider for F
where
    F: Fn() -> i64 + Send + Sync,
{
    fn now(&self) -> i64 {
        self()
    }
}
