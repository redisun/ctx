//! Language Server Protocol (LSP) integration for Rust semantic analysis.
//!
//! This module provides LSP-based semantic analysis using rust-analyzer to extract
//! high-confidence semantic relationships like function calls, type references, and
//! trait implementations.

pub mod analyzer;
pub mod client;
pub mod edges;
pub mod protocol;
pub mod queries;

pub use analyzer::{AnalyzedItem, CallInfo, FileAnalysis, ItemKind, RustAnalyzer};
pub use edges::build_edges_from_analysis;
