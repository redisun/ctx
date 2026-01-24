//! Convert LSP analysis results to CTX Edge objects.
//!
//! This module bridges the gap between LSP protocol types and CTX's graph representation,
//! generating high-confidence edges from rust-analyzer's semantic analysis.

use crate::lsp::analyzer::{FileAnalysis, ItemKind};
use crate::lsp::protocol::Range;
use crate::types::{Confidence, Edge, EdgeLabel, Evidence, EvidenceTool, NodeId, NodeKind, Span};
use crate::ObjectId;
use tracing::debug;

#[cfg(test)]
use crate::lsp::analyzer::CallInfo;

/// Generate CTX edges from LSP file analysis.
///
/// Converts LSP analysis results (symbols, calls) into CTX Edge objects
/// with high confidence evidence.
///
/// # Arguments
///
/// * `analysis` - LSP analysis result
/// * `file_path` - Path to the analyzed file
/// * `file_content` - Content of the analyzed file
/// * `commit_id` - Current commit ID for evidence tracking
///
/// # Returns
///
/// Vector of edges representing:
/// - File --Defines--> Item (for each symbol in the file)
/// - Item --Calls--> Item (for resolved function calls)
pub fn build_edges_from_analysis(
    analysis: &FileAnalysis,
    file_path: &str,
    file_content: &[u8],
    commit_id: ObjectId,
) -> Vec<Edge> {
    let mut edges = Vec::new();

    // Compute ObjectIds for this file
    let file_id = ObjectId::hash_blob(file_path.as_bytes());
    let file_version_id = ObjectId::hash_blob(file_content);

    // Generate Defines edges: File -> Item
    for item in &analysis.items {
        // Create edge from file to item
        edges.push(Edge {
            from: NodeId {
                kind: NodeKind::File,
                id: file_path.to_string(),
            },
            to: NodeId {
                kind: node_kind_for_item(item.kind),
                id: item.qualified_name.clone(),
            },
            label: EdgeLabel::Defines,
            weight: None,
            evidence: Evidence {
                commit_id,
                tool: EvidenceTool::RustAnalyzer,
                confidence: Confidence::High,
                span: Some(lsp_range_to_span(&item.range, file_id, file_version_id)),
                blob_id: Some(file_version_id),
            },
        });
    }

    // Generate Calls edges: Item -> Item
    for call in &analysis.calls {
        // For calls, we use the caller's file context (since that's where the call happens)
        edges.push(Edge {
            from: NodeId {
                kind: NodeKind::Item,
                id: call.caller.clone(),
            },
            to: NodeId {
                kind: NodeKind::Item,
                id: call.callee.clone(),
            },
            label: EdgeLabel::Calls,
            weight: None,
            evidence: Evidence {
                commit_id,
                tool: EvidenceTool::RustAnalyzer,
                confidence: Confidence::High, // LSP resolution = high confidence
                span: Some(lsp_range_to_span(
                    &call.caller_location.range,
                    file_id,
                    file_version_id,
                )),
                blob_id: Some(file_version_id),
            },
        });
    }

    // Generate References edges: File -> Item for each reference location
    for ref_info in &analysis.references {
        for ref_loc in &ref_info.reference_locations {
            // Compute ObjectIds for the reference location's file
            let ref_file_path = if ref_loc.uri.scheme() == "file" {
                ref_loc.uri.path()
            } else {
                ref_loc.uri.as_str()
            };
            let ref_file_id = ObjectId::hash_blob(ref_file_path.as_bytes());

            // For the reference file content, we need to read it if it's a different file
            // For same-file references, use the current file's version
            let ref_file_version_id = if ref_file_path == file_path {
                file_version_id
            } else {
                // Read actual file content for proper version tracking
                match std::fs::read(ref_file_path) {
                    Ok(content) => ObjectId::hash_blob(&content),
                    Err(e) => {
                        // Log at debug level - external crates are expected to fail
                        debug!(
                            path = ref_file_path,
                            error = %e,
                            "Could not read cross-file reference, using placeholder"
                        );
                        ObjectId::hash_blob(b"")
                    }
                }
            };

            edges.push(Edge {
                from: NodeId {
                    kind: NodeKind::File,
                    id: ref_file_path.to_string(),
                },
                to: NodeId {
                    kind: NodeKind::Item,
                    id: ref_info.referenced_item.clone(),
                },
                label: EdgeLabel::References,
                weight: None,
                evidence: Evidence {
                    commit_id,
                    tool: EvidenceTool::RustAnalyzer,
                    confidence: Confidence::High, // LSP resolution = high confidence
                    span: Some(lsp_range_to_span(
                        &ref_loc.range,
                        ref_file_id,
                        ref_file_version_id,
                    )),
                    blob_id: Some(ref_file_version_id),
                },
            });
        }
    }

    // Generate Implements edges: Type -> Trait
    for impl_info in &analysis.implements {
        // Compute ObjectIds for the implementor's file
        let impl_file_path = if impl_info.implementor_location.uri.scheme() == "file" {
            impl_info.implementor_location.uri.path()
        } else {
            impl_info.implementor_location.uri.as_str()
        };
        let impl_file_id = ObjectId::hash_blob(impl_file_path.as_bytes());

        // Use current file's version if it's the same file, otherwise read content
        let impl_file_version_id = if impl_file_path == file_path {
            file_version_id
        } else {
            // Read actual file content for proper version tracking
            match std::fs::read(impl_file_path) {
                Ok(content) => ObjectId::hash_blob(&content),
                Err(e) => {
                    // Log at debug level - external crates are expected to fail
                    debug!(
                        path = impl_file_path,
                        error = %e,
                        "Could not read cross-file implements, using placeholder"
                    );
                    ObjectId::hash_blob(b"")
                }
            }
        };

        edges.push(Edge {
            from: NodeId {
                kind: NodeKind::Item,
                id: impl_info.implementor.clone(),
            },
            to: NodeId {
                kind: NodeKind::Item,
                id: impl_info.trait_name.clone(),
            },
            label: EdgeLabel::Implements,
            weight: None,
            evidence: Evidence {
                commit_id,
                tool: EvidenceTool::RustAnalyzer,
                confidence: Confidence::High, // LSP resolution = high confidence
                span: Some(lsp_range_to_span(
                    &impl_info.implementor_location.range,
                    impl_file_id,
                    impl_file_version_id,
                )),
                blob_id: Some(impl_file_version_id),
            },
        });
    }

    edges
}

/// Map ItemKind to NodeKind.
fn node_kind_for_item(item_kind: ItemKind) -> NodeKind {
    match item_kind {
        ItemKind::Module => NodeKind::Module,
        _ => NodeKind::Item, // Functions, structs, enums, traits all map to Item
    }
}

/// Convert LSP Range to CTX Span.
///
/// Note: LSP provides line/column but not byte offsets. We set byte offsets to 0.
fn lsp_range_to_span(range: &Range, file_id: ObjectId, file_version_id: ObjectId) -> Span {
    Span {
        file_id,
        file_version_id,
        start_byte: 0, // Not provided by LSP
        end_byte: 0,   // Not provided by LSP
        start_line: range.start.line,
        start_col: range.start.character,
        end_line: range.end.line,
        end_col: range.end.character,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::analyzer::AnalyzedItem;
    use crate::lsp::protocol::{Location, Position, Range};
    use std::path::PathBuf;

    #[test]
    fn test_defines_edge_generation() {
        let commit_id = ObjectId::from_bytes([1; 32]);
        let file_path = "src/main.rs";
        let file_content = b"fn main() { println!(\"hello\"); }";

        let analysis = FileAnalysis {
            items: vec![AnalyzedItem {
                name: "main".to_string(),
                qualified_name: "crate::main".to_string(),
                kind: ItemKind::Function,
                path: PathBuf::from(file_path),
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 5,
                        character: 1,
                    },
                },
            }],
            calls: vec![],
            references: vec![],
            implements: vec![],
            warnings: vec![],
        };

        let edges = build_edges_from_analysis(&analysis, file_path, file_content, commit_id);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from.kind, NodeKind::File);
        assert_eq!(edges[0].from.id, "src/main.rs");
        assert_eq!(edges[0].to.kind, NodeKind::Item);
        assert_eq!(edges[0].to.id, "crate::main");
        assert_eq!(edges[0].label, EdgeLabel::Defines);
        assert_eq!(edges[0].evidence.tool, EvidenceTool::RustAnalyzer);
        assert_eq!(edges[0].evidence.confidence, Confidence::High);
        assert!(edges[0].evidence.blob_id.is_some());
    }

    #[test]
    fn test_calls_edge_generation() {
        use lsp_types::Url;
        let commit_id = ObjectId::from_bytes([2; 32]);
        let file_path = "src/main.rs";
        let file_content = b"fn main() { helper(); }\nfn helper() {}";

        let analysis = FileAnalysis {
            items: vec![],
            calls: vec![CallInfo {
                caller: "crate::main".to_string(),
                caller_location: Location {
                    uri: Url::parse("file:///src/main.rs").unwrap(),
                    range: Range {
                        start: Position {
                            line: 1,
                            character: 4,
                        },
                        end: Position {
                            line: 1,
                            character: 14,
                        },
                    },
                },
                callee: "crate::helper".to_string(),
                callee_location: Location {
                    uri: Url::parse("file:///src/main.rs").unwrap(),
                    range: Range {
                        start: Position {
                            line: 5,
                            character: 0,
                        },
                        end: Position {
                            line: 7,
                            character: 1,
                        },
                    },
                },
                call_sites: vec![],
            }],
            references: vec![],
            implements: vec![],
            warnings: vec![],
        };

        let edges = build_edges_from_analysis(&analysis, file_path, file_content, commit_id);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from.kind, NodeKind::Item);
        assert_eq!(edges[0].from.id, "crate::main");
        assert_eq!(edges[0].to.kind, NodeKind::Item);
        assert_eq!(edges[0].to.id, "crate::helper");
        assert_eq!(edges[0].label, EdgeLabel::Calls);
        assert_eq!(edges[0].evidence.confidence, Confidence::High);
    }

    #[test]
    fn test_lsp_range_to_span() {
        let range = Range {
            start: Position {
                line: 10,
                character: 5,
            },
            end: Position {
                line: 15,
                character: 10,
            },
        };

        let file_id = ObjectId::from_bytes([3; 32]);
        let version_id = ObjectId::from_bytes([4; 32]);

        let span = lsp_range_to_span(&range, file_id, version_id);

        assert_eq!(span.file_id, file_id);
        assert_eq!(span.file_version_id, version_id);
        assert_eq!(span.start_line, 10);
        assert_eq!(span.start_col, 5);
        assert_eq!(span.end_line, 15);
        assert_eq!(span.end_col, 10);
        assert_eq!(span.start_byte, 0); // Not provided by LSP
        assert_eq!(span.end_byte, 0); // Not provided by LSP
    }

    #[test]
    fn test_node_kind_mapping() {
        assert_eq!(node_kind_for_item(ItemKind::Module), NodeKind::Module);
        assert_eq!(node_kind_for_item(ItemKind::Function), NodeKind::Item);
        assert_eq!(node_kind_for_item(ItemKind::Struct), NodeKind::Item);
        assert_eq!(node_kind_for_item(ItemKind::Enum), NodeKind::Item);
        assert_eq!(node_kind_for_item(ItemKind::Trait), NodeKind::Item);
    }

    #[test]
    fn test_references_edge_generation() {
        use crate::lsp::analyzer::ReferenceInfo;
        use lsp_types::Url;

        let commit_id = ObjectId::from_bytes([5; 32]);
        let file_path = "src/lib.rs";
        let file_content = b"pub fn helper() {}\nfn main() { helper(); }";

        let analysis = FileAnalysis {
            items: vec![],
            calls: vec![],
            references: vec![ReferenceInfo {
                referenced_item: "crate::helper".to_string(),
                definition_location: Location {
                    uri: Url::parse("file:///src/lib.rs").unwrap(),
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 7,
                        },
                        end: Position {
                            line: 0,
                            character: 13,
                        },
                    },
                },
                reference_locations: vec![
                    Location {
                        uri: Url::parse("file:///src/lib.rs").unwrap(),
                        range: Range {
                            start: Position {
                                line: 1,
                                character: 12,
                            },
                            end: Position {
                                line: 1,
                                character: 18,
                            },
                        },
                    },
                    Location {
                        uri: Url::parse("file:///src/other.rs").unwrap(),
                        range: Range {
                            start: Position {
                                line: 5,
                                character: 4,
                            },
                            end: Position {
                                line: 5,
                                character: 10,
                            },
                        },
                    },
                ],
            }],
            implements: vec![],
            warnings: vec![],
        };

        let edges = build_edges_from_analysis(&analysis, file_path, file_content, commit_id);

        // Should have 2 References edges (one per reference location)
        assert_eq!(edges.len(), 2);

        // First reference (same file)
        assert_eq!(edges[0].from.kind, NodeKind::File);
        assert_eq!(edges[0].from.id, "/src/lib.rs"); // Note: includes leading /
        assert_eq!(edges[0].to.kind, NodeKind::Item);
        assert_eq!(edges[0].to.id, "crate::helper");
        assert_eq!(edges[0].label, EdgeLabel::References);
        assert_eq!(edges[0].evidence.confidence, Confidence::High);

        // Second reference (different file)
        assert_eq!(edges[1].from.kind, NodeKind::File);
        assert_eq!(edges[1].from.id, "/src/other.rs"); // Note: includes leading /
        assert_eq!(edges[1].to.kind, NodeKind::Item);
        assert_eq!(edges[1].to.id, "crate::helper");
        assert_eq!(edges[1].label, EdgeLabel::References);
        assert_eq!(edges[1].evidence.confidence, Confidence::High);
    }

    #[test]
    fn test_implements_edge_generation() {
        use crate::lsp::analyzer::ImplementsInfo;
        use lsp_types::Url;

        let commit_id = ObjectId::from_bytes([6; 32]);
        let file_path = "src/types.rs";
        let file_content = b"struct MyStruct;\nimpl Display for MyStruct {}";

        let analysis = FileAnalysis {
            items: vec![],
            calls: vec![],
            references: vec![],
            implements: vec![ImplementsInfo {
                implementor: "crate::types::MyStruct".to_string(),
                implementor_location: Location {
                    uri: Url::parse("file:///src/types.rs").unwrap(),
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 7,
                        },
                        end: Position {
                            line: 0,
                            character: 15,
                        },
                    },
                },
                trait_name: "std::fmt::Display".to_string(),
                trait_location: Location {
                    uri: Url::parse("file:///rustlib/src/rust/library/core/src/fmt/mod.rs")
                        .unwrap(),
                    range: Range {
                        start: Position {
                            line: 600,
                            character: 0,
                        },
                        end: Position {
                            line: 650,
                            character: 1,
                        },
                    },
                },
            }],
            warnings: vec![],
        };

        let edges = build_edges_from_analysis(&analysis, file_path, file_content, commit_id);

        // Should have 1 Implements edge
        assert_eq!(edges.len(), 1);

        // Check the Implements edge
        assert_eq!(edges[0].from.kind, NodeKind::Item);
        assert_eq!(edges[0].from.id, "crate::types::MyStruct");
        assert_eq!(edges[0].to.kind, NodeKind::Item);
        assert_eq!(edges[0].to.id, "std::fmt::Display");
        assert_eq!(edges[0].label, EdgeLabel::Implements);
        assert_eq!(edges[0].evidence.confidence, Confidence::High);
        assert_eq!(edges[0].evidence.tool, EvidenceTool::RustAnalyzer);
    }
}
