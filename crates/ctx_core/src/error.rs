//! Error types for ctx_core operations.

use std::path::PathBuf;
use thiserror::Error;

/// Core error type for ctx_core operations.
#[derive(Error, Debug)]
pub enum CtxError {
    /// Object with the given ID was not found in the store.
    #[error("object not found: {0}")]
    ObjectNotFound(String),

    /// Hash verification failed during object read.
    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch {
        /// The expected object ID
        expected: String,
        /// The actual computed hash
        actual: String,
    },

    /// The object file is corrupted or has invalid format.
    #[error("corrupted object at {}: {}", path.display(), reason)]
    CorruptedObject {
        /// Path to the corrupted object
        path: PathBuf,
        /// Description of the corruption
        reason: String,
    },

    /// Invalid hex string for ObjectId parsing.
    #[error("invalid hex string: {0}")]
    InvalidHex(String),

    /// Serialization error during typed object operations.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Deserialization error during typed object operations.
    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// Compression or decompression failed.
    #[error("compression error: {0}")]
    Compression(String),

    /// Blob exceeds maximum allowed size.
    #[error("blob too large: {size} bytes exceeds limit of {limit} bytes")]
    BlobTooLarge {
        /// Actual size of the blob
        size: usize,
        /// Maximum allowed size
        limit: usize,
    },

    /// Reference not found.
    #[error("ref not found: {0}")]
    RefNotFound(String),

    /// Invalid ref file content or format.
    #[error("invalid ref at {}: {}", path.display(), reason)]
    InvalidRef {
        /// Path to the invalid ref file
        path: PathBuf,
        /// Description of what's invalid
        reason: String,
    },

    /// I/O error during file operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A session is already active.
    #[error("session already active: {0}")]
    SessionAlreadyActive(String),

    /// No active session exists.
    #[error("no active session")]
    NoActiveSession,

    /// Invalid session state transition.
    #[error("invalid state transition from {from} to {to}")]
    InvalidStateTransition {
        /// Source state
        from: String,
        /// Target state
        to: String,
    },

    /// Repository is locked by another process.
    #[error("repository locked by another process")]
    RepositoryLocked,

    /// Staging chain is corrupted or inconsistent.
    #[error("staging chain corrupted: {reason}")]
    StagingCorrupted {
        /// Description of the corruption
        reason: String,
    },

    /// rust-analyzer is not installed or not found on PATH.
    #[error("rust-analyzer not found. Install with: rustup component add rust-analyzer")]
    RustAnalyzerNotFound,

    /// rust-analyzer process failed to start.
    #[error("failed to start rust-analyzer: {0}")]
    RustAnalyzerStartFailed(String),

    /// LSP request timed out.
    #[error("LSP request timed out after {timeout_ms}ms: {method}")]
    LspTimeout {
        /// Method name that timed out
        method: String,
        /// Timeout in milliseconds
        timeout_ms: u64,
    },

    /// LSP protocol error.
    #[error("LSP protocol error: {0}")]
    LspProtocolError(String),

    /// LSP returned an error response.
    #[error("LSP error {code}: {message}")]
    LspError {
        /// Error code
        code: i32,
        /// Error message
        message: String,
    },

    /// rust-analyzer crashed or exited unexpectedly.
    #[error("rust-analyzer exited unexpectedly: {0}")]
    RustAnalyzerCrashed(String),

    /// cargo is not installed or not found on PATH.
    #[error("cargo not found. Ensure Rust toolchain is installed.")]
    CargoNotFound,

    /// cargo metadata command failed.
    #[error("cargo metadata failed: {0}")]
    CargoMetadataFailed(String),

    /// Cargo.toml not found in project directory.
    #[error("no Cargo.toml found in {0}")]
    NoCargoManifest(String),

    /// Failed to parse cargo metadata JSON.
    #[error("failed to parse cargo metadata: {0}")]
    CargoMetadataParseFailed(String),

    /// Configuration error (loading, parsing, invalid values).
    #[error("configuration error: {0}")]
    ConfigError(String),

    /// Index is corrupted and needs rebuilding.
    #[error("index corrupted: {message}. Run 'ctx rebuild' to repair.")]
    IndexCorrupted {
        /// Description of the corruption
        message: String,
    },

    /// Error in narrative file operations.
    #[error("narrative file error: {0}")]
    NarrativeError(String),

    /// Error building tree from file system.
    #[error("tree build error: {0}")]
    TreeBuildError(String),

    /// Garbage collection error.
    #[error("gc error: {0}")]
    GcError(String),

    /// Search index error.
    #[error("search index error: {0}")]
    SearchError(String),

    /// Session lock is held by another process.
    #[error("session lock held by another process (PID: {pid})")]
    SessionLockHeld {
        /// Process ID holding the lock
        pid: u32,
    },
}

impl CtxError {
    /// Returns a user-friendly recovery suggestion for the error, if available.
    pub fn recovery_suggestion(&self) -> Option<&'static str> {
        match self {
            Self::CorruptedObject { .. } => {
                Some("Run 'ctx verify' to identify all corrupted objects, then 'ctx gc' to clean them up.")
            }
            Self::ObjectNotFound(_) => {
                Some("Repository might be corrupted. Run 'ctx verify' to check.")
            }
            Self::IndexCorrupted { .. } => Some("Run 'ctx rebuild' to regenerate the index."),
            Self::SessionLockHeld { .. } => {
                Some("Another process might be using this repo. Check for stale locks with 'ctx stage recover'.")
            }
            Self::StagingCorrupted { .. } => {
                Some("Try 'ctx stage recover' to recover from a crashed session, or 'ctx stage abort' to discard.")
            }
            Self::RepositoryLocked => {
                Some("Wait for the other process to finish, or manually remove .ctx/LOCK if the process is dead.")
            }
            Self::NoActiveSession => Some("Start a new session with 'ctx stage start <task>'."),
            Self::SessionAlreadyActive(_) => {
                Some("Complete the current session with 'ctx stage compact' or abort it with 'ctx stage abort'.")
            }
            Self::RefNotFound(_) => {
                Some("This might indicate a corrupted repository. Try 'ctx verify --full'.")
            }
            _ => None,
        }
    }
}

/// Convenience Result type for ctx_core operations.
pub type Result<T> = std::result::Result<T, CtxError>;
