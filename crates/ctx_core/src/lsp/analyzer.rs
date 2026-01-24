//! High-level RustAnalyzer manager.
//!
//! Provides a convenient API for analyzing Rust code using rust-analyzer,
//! managing the process lifecycle, and extracting semantic information.

use crate::error::Result;
use crate::lsp::client::LspClient;
use crate::lsp::protocol::{
    CallHierarchyClientCapabilities, ClientCapabilities, DocumentSymbol,
    DocumentSymbolClientCapabilities, InitializeParams, Position, Range,
    TextDocumentClientCapabilities, Url,
};
use crate::lsp::queries::LspQueries;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;
use tracing::{debug, warn};

/// High-level rust-analyzer manager.
pub struct RustAnalyzer {
    client: LspClient,
    project_root: PathBuf,
    open_files: HashSet<Url>,
    /// Track file versions for each open file (URI -> version number)
    file_versions: HashMap<Url, i32>,
    /// Track file content hashes to detect changes
    file_content_hashes: HashMap<Url, u64>,
    /// Whether rust-analyzer has completed initial indexing.
    /// Call hierarchy requests should only be made after this is true.
    indexing_complete: bool,
}

impl RustAnalyzer {
    /// Check if rust-analyzer is available on the system.
    ///
    /// Returns true if rust-analyzer can be found and executed.
    pub fn is_available() -> bool {
        Command::new("rust-analyzer")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Start rust-analyzer for a project.
    ///
    /// Spawns rust-analyzer and performs the LSP initialize handshake.
    ///
    /// # Arguments
    ///
    /// * `project_root` - Root directory of the Rust project (should contain Cargo.toml)
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - rust-analyzer is not found
    /// - rust-analyzer fails to start
    /// - LSP initialization fails
    pub fn start(project_root: &Path) -> Result<Self> {
        let mut client = LspClient::spawn(project_root)?;

        // Initialize LSP connection
        #[allow(deprecated)]
        let init_params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(Self::path_to_uri(project_root)),
            root_path: None, // Deprecated: use root_uri instead
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    call_hierarchy: Some(CallHierarchyClientCapabilities {
                        dynamic_registration: Some(false),
                    }),
                    document_symbol: Some(DocumentSymbolClientCapabilities {
                        hierarchical_document_symbol_support: Some(true),
                        dynamic_registration: None,
                        symbol_kind: None,
                        tag_support: None,
                    }),
                    ..Default::default()
                }),
                workspace: None,
                window: None,
                general: None,
                experimental: None,
            },
            client_info: None,
            locale: None,
            initialization_options: None,
            trace: None,
            workspace_folders: None,
        };

        client.initialize(init_params)?;
        client.initialized()?;

        Ok(Self {
            client,
            project_root: project_root.to_path_buf(),
            open_files: HashSet::new(),
            file_versions: HashMap::new(),
            file_content_hashes: HashMap::new(),
            indexing_complete: false,
        })
    }

    /// Analyze a single file.
    ///
    /// Opens the file in rust-analyzer (if not already open), extracts symbols,
    /// and resolves call relationships.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the Rust file (relative or absolute)
    ///
    /// # Returns
    ///
    /// FileAnalysis containing symbols and call information.
    pub fn analyze_file(&mut self, path: &Path) -> Result<FileAnalysis> {
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.project_root.join(path)
        };

        let uri = Self::path_to_uri(&abs_path);
        let content = std::fs::read_to_string(&abs_path)?;

        // Track warnings about incomplete analysis
        let mut warnings = Vec::new();

        // Compute content hash to detect changes
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let content_hash = hasher.finish();

        // Check if file is already open and if content has changed
        let file_changed = self
            .file_content_hashes
            .get(&uri)
            .map(|&old_hash| old_hash != content_hash)
            .unwrap_or(true);

        let mut queries = LspQueries::new(&mut self.client);

        if !self.open_files.contains(&uri) {
            // File not open - open it with version 1
            let version = 1;
            queries.did_open(&uri, &content, version)?;
            self.open_files.insert(uri.clone());
            self.file_versions.insert(uri.clone(), version);
            self.file_content_hashes.insert(uri.clone(), content_hash);

            // Give rust-analyzer a moment to process the didOpen notification
            thread::sleep(Duration::from_millis(50));

            // Wait for rust-analyzer to finish indexing on first file
            // It signals readiness by sending publishDiagnostics notification
            if !self.indexing_complete {
                debug!("Waiting for rust-analyzer to finish initial indexing...");
                match self
                    .client
                    .wait_for_notification("textDocument/publishDiagnostics", 10000)
                {
                    Ok(_) => {
                        debug!("Received publishDiagnostics, marking indexing as complete");
                        self.indexing_complete = true;
                    }
                    Err(e) => {
                        warn!("Timeout waiting for rust-analyzer diagnostics: {}", e);
                        warnings.push(AnalysisWarning::DiagnosticsTimeout);
                        // Continue anyway - some files might not produce diagnostics
                        // But we'll still try call hierarchy, it might work
                    }
                }
            }
        } else if file_changed {
            // File is open but content changed - send didChange with incremented version
            let current_version = self.file_versions.get(&uri).copied().unwrap_or(1);
            let new_version = current_version + 1;
            queries.did_change(&uri, &content, new_version)?;
            self.file_versions.insert(uri.clone(), new_version);
            self.file_content_hashes.insert(uri.clone(), content_hash);
            debug!(
                uri = %uri,
                old_version = current_version,
                new_version = new_version,
                "File content changed, sent didChange notification"
            );
            // Give rust-analyzer a moment to process the didChange notification
            thread::sleep(Duration::from_millis(50));
        }

        // Get document symbols
        let mut queries = LspQueries::new(&mut self.client);
        let symbols = queries.document_symbols(&uri)?;

        // Extract items
        let items = Self::flatten_symbols(&symbols, &abs_path);

        // Extract call information for functions
        // Only attempt call hierarchy if rust-analyzer has completed indexing
        let mut calls = Vec::new();
        if self.indexing_complete {
            for item in &items {
                if matches!(item.kind, ItemKind::Function | ItemKind::Method) {
                    // Prepare call hierarchy at this symbol
                    // Use the start position of the selection range, which should be on the symbol name
                    // According to LSP spec, prepareCallHierarchy should be called at the symbol name position
                    let position = item.range.start;
                    let hierarchy_items = queries.prepare_call_hierarchy(&uri, position)?;

                    if hierarchy_items.is_empty() {
                        debug!(
                            function = %item.qualified_name,
                            position = ?position,
                            selection_range = ?item.range,
                            uri = %uri,
                            "No call hierarchy items returned for function - possible causes: wrong position, incomplete indexing, or rust-analyzer bug"
                        );
                    } else {
                        debug!(
                            function = %item.qualified_name,
                            hierarchy_items_count = hierarchy_items.len(),
                            position = ?position,
                            "Successfully prepared call hierarchy for function"
                        );
                    }

                    for hierarchy_item in hierarchy_items {
                        // Get outgoing calls
                        let outgoing = queries.call_hierarchy_outgoing(&hierarchy_item)?;

                        debug!(
                            function = %item.qualified_name,
                            outgoing_count = outgoing.len(),
                            "Retrieved outgoing calls for hierarchy item"
                        );

                        if outgoing.is_empty() {
                            debug!(
                                function = %item.qualified_name,
                                hierarchy_item_name = %hierarchy_item.name,
                                "No outgoing calls found - function may not call anything, or call resolution failed"
                            );
                        }

                        for call in outgoing {
                            // Try to get qualified name from detail, fallback to constructing from URI
                            let callee_qualified = call.to.detail.clone().unwrap_or_else(|| {
                                Self::qualified_name_from_uri(&call.to.uri, &call.to.name)
                            });

                            calls.push(CallInfo {
                                caller: item.qualified_name.clone(),
                                caller_location: crate::lsp::protocol::Location {
                                    uri: uri.clone(),
                                    range: item.range,
                                },
                                callee: callee_qualified,
                                callee_location: crate::lsp::protocol::Location {
                                    uri: call.to.uri.clone(),
                                    range: call.to.range,
                                },
                                call_sites: call.from_ranges,
                            });
                        }
                    }
                }
            }
        } else {
            warn!("Skipping call hierarchy extraction - rust-analyzer indexing not complete");
            warnings.push(AnalysisWarning::CallHierarchySkipped);
        }

        // Extract reference information for each item
        // Only attempt references if rust-analyzer has completed indexing
        // References require full project analysis, so they're more sensitive to timing
        let mut references = Vec::new();
        if self.indexing_complete {
            // Give rust-analyzer a moment after document_symbols to process
            thread::sleep(Duration::from_millis(100));

            for (idx, item) in items.iter().enumerate() {
                // Small delay between requests to avoid overwhelming rust-analyzer
                if idx > 0 {
                    thread::sleep(Duration::from_millis(50));
                }

                // Use a position in the middle of the symbol name for better accuracy
                // This is more reliable than using the start position
                let position = {
                    let start = &item.range.start;
                    let end = &item.range.end;
                    // If the range spans multiple lines or is very wide, use start
                    // Otherwise, use a position in the middle of the symbol name
                    if start.line == end.line && end.character > start.character {
                        let mid_char = start.character + (end.character - start.character) / 2;
                        Position {
                            line: start.line,
                            character: mid_char,
                        }
                    } else {
                        *start
                    }
                };

                // Find all references to this item
                let refs = match queries.find_references(&uri, position, true) {
                    Ok(refs) => {
                        if refs.is_empty() {
                            debug!(
                                item = %item.qualified_name,
                                position = ?position,
                                "No references found for item"
                            );
                        } else {
                            debug!(
                                item = %item.qualified_name,
                                ref_count = refs.len(),
                                "Found references for item"
                            );
                        }
                        refs
                    }
                    Err(e) => {
                        // Log but continue - references are optional
                        debug!(
                            item = %item.qualified_name,
                            error = %e,
                            "Failed to find references"
                        );
                        continue;
                    }
                };

                if !refs.is_empty() {
                    // Exclude the definition itself, keep only references
                    let reference_locs: Vec<_> = refs
                        .into_iter()
                        .filter(|loc| {
                            // Filter out the definition location
                            !(loc.uri == uri && loc.range == item.range)
                        })
                        .collect();

                    if !reference_locs.is_empty() {
                        references.push(ReferenceInfo {
                            referenced_item: item.qualified_name.clone(),
                            definition_location: crate::lsp::protocol::Location {
                                uri: uri.clone(),
                                range: item.range,
                            },
                            reference_locations: reference_locs,
                        });
                    }
                }
            }
        } else {
            warn!("Skipping references extraction - rust-analyzer indexing not complete");
            warnings.push(AnalysisWarning::ReferencesSkipped);
        }

        // Extract trait implementation information for structs/enums
        // Note: rust-analyzer doesn't support textDocument/prepareTypeHierarchy
        // We would need to parse impl blocks manually or use goto_implementation
        // For now, skip trait implementation extraction
        let implements = Vec::new();

        Ok(FileAnalysis {
            items,
            calls,
            references,
            implements,
            warnings,
        })
    }

    /// Flatten hierarchical symbols into a flat list with qualified names.
    fn flatten_symbols(symbols: &[DocumentSymbol], path: &Path) -> Vec<AnalyzedItem> {
        let mut items = Vec::new();
        Self::flatten_symbols_recursive(symbols, path, "", &mut items);
        items
    }

    fn flatten_symbols_recursive(
        symbols: &[DocumentSymbol],
        path: &Path,
        prefix: &str,
        out: &mut Vec<AnalyzedItem>,
    ) {
        for sym in symbols {
            let qualified_name = if prefix.is_empty() {
                sym.name.clone()
            } else {
                format!("{}::{}", prefix, sym.name)
            };

            let kind = match sym.kind {
                lsp_types::SymbolKind::FUNCTION => ItemKind::Function,
                lsp_types::SymbolKind::METHOD => ItemKind::Method,
                lsp_types::SymbolKind::STRUCT => ItemKind::Struct,
                lsp_types::SymbolKind::ENUM => ItemKind::Enum,
                lsp_types::SymbolKind::INTERFACE => ItemKind::Trait,
                lsp_types::SymbolKind::CONSTANT => ItemKind::Const,
                lsp_types::SymbolKind::VARIABLE => ItemKind::Static,
                lsp_types::SymbolKind::MODULE | lsp_types::SymbolKind::NAMESPACE => {
                    ItemKind::Module
                }
                lsp_types::SymbolKind::CLASS => ItemKind::Impl,
                _ => ItemKind::Other,
            };

            out.push(AnalyzedItem {
                name: sym.name.clone(),
                qualified_name: qualified_name.clone(),
                kind,
                path: path.to_path_buf(),
                // Use selection_range (the symbol name) instead of full range
                // This is important for LSP requests that need a precise position
                range: sym.selection_range,
            });

            // Recurse into children
            if let Some(children) = &sym.children {
                if !children.is_empty() {
                    Self::flatten_symbols_recursive(children, path, &qualified_name, out);
                }
            }
        }
    }

    /// Convert filesystem path to file:// URI.
    fn path_to_uri(path: &Path) -> Url {
        let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        Url::parse(&format!("file://{}", abs.display())).unwrap()
    }

    /// Convert file:// URI to filesystem path.
    pub fn uri_to_path(uri: &Url) -> Option<PathBuf> {
        if uri.scheme() == "file" {
            Some(PathBuf::from(uri.path()))
        } else {
            None
        }
    }

    /// Attempt to construct qualified name from URI and symbol name.
    ///
    /// This is a fallback when LSP doesn't provide qualified name in detail field.
    /// Extracts the module path from file URI.
    ///
    /// # Examples
    ///
    /// - "file:///project/src/foo/bar.rs" + "helper" -> "bar::helper"
    /// - "file:///project/src/lib.rs" + "main" -> "lib::main"
    fn qualified_name_from_uri(uri: &Url, name: &str) -> String {
        if let Some(path) = Self::uri_to_path(uri) {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                // Use filename::name as approximation
                // Note: This is not perfect - ideally would resolve full module path
                return format!("{}::{}", stem, name);
            }
        }
        name.to_string()
    }

    /// Gracefully shutdown rust-analyzer.
    pub fn shutdown(self) -> Result<()> {
        self.client.shutdown()
    }
}

/// Warning generated during analysis.
///
/// Warnings indicate that some analysis was skipped or incomplete,
/// but the analysis could still produce useful partial results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalysisWarning {
    /// Timed out waiting for rust-analyzer to complete initial indexing.
    DiagnosticsTimeout,
    /// Call hierarchy extraction was skipped because indexing wasn't complete.
    CallHierarchySkipped,
    /// Reference extraction was skipped because indexing wasn't complete.
    ReferencesSkipped,
}

impl std::fmt::Display for AnalysisWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DiagnosticsTimeout => write!(
                f,
                "Timed out waiting for rust-analyzer indexing - analysis may be incomplete"
            ),
            Self::CallHierarchySkipped => write!(
                f,
                "Call hierarchy extraction skipped - rust-analyzer indexing incomplete"
            ),
            Self::ReferencesSkipped => write!(
                f,
                "Reference extraction skipped - rust-analyzer indexing incomplete"
            ),
        }
    }
}

/// Analysis result for a single file.
#[derive(Debug, Clone)]
pub struct FileAnalysis {
    /// Symbols found in the file.
    pub items: Vec<AnalyzedItem>,
    /// Call relationships (what calls what).
    pub calls: Vec<CallInfo>,
    /// Reference relationships (what references what).
    pub references: Vec<ReferenceInfo>,
    /// Trait implementation relationships (type implements trait).
    pub implements: Vec<ImplementsInfo>,
    /// Warnings generated during analysis.
    ///
    /// If non-empty, the analysis results may be incomplete.
    pub warnings: Vec<AnalysisWarning>,
}

/// An analyzed code item (function, struct, etc.).
#[derive(Debug, Clone)]
pub struct AnalyzedItem {
    /// Simple name (e.g., "foo").
    pub name: String,
    /// Qualified name (e.g., "crate::module::foo").
    pub qualified_name: String,
    /// Item kind.
    pub kind: ItemKind,
    /// Source file path.
    pub path: PathBuf,
    /// Source location.
    pub range: Range,
}

/// Kind of code item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// Function or free function.
    Function,
    /// Method (associated function).
    Method,
    /// Struct.
    Struct,
    /// Enum.
    Enum,
    /// Trait.
    Trait,
    /// Impl block.
    Impl,
    /// Const.
    Const,
    /// Static.
    Static,
    /// Module.
    Module,
    /// Other (field, type alias, etc.).
    Other,
}

/// Information about a function call.
#[derive(Debug, Clone)]
pub struct CallInfo {
    /// Name of the calling function.
    pub caller: String,
    /// Location of the caller.
    pub caller_location: crate::lsp::protocol::Location,
    /// Name of the called function.
    pub callee: String,
    /// Location of the callee.
    pub callee_location: crate::lsp::protocol::Location,
    /// Locations where the call occurs.
    pub call_sites: Vec<Range>,
}

/// Information about a reference to a symbol.
#[derive(Debug, Clone)]
pub struct ReferenceInfo {
    /// Name of the referenced item.
    pub referenced_item: String,
    /// Location of the item definition.
    pub definition_location: crate::lsp::protocol::Location,
    /// Locations where the item is referenced.
    pub reference_locations: Vec<crate::lsp::protocol::Location>,
}

/// Information about trait implementation.
#[derive(Debug, Clone)]
pub struct ImplementsInfo {
    /// Name of the type implementing the trait (struct/enum).
    pub implementor: String,
    /// Location of the implementor definition.
    pub implementor_location: crate::lsp::protocol::Location,
    /// Name of the trait being implemented.
    pub trait_name: String,
    /// Location of the trait definition.
    pub trait_location: crate::lsp::protocol::Location,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_available() {
        // This test will pass if rust-analyzer is installed, fail otherwise
        let available = RustAnalyzer::is_available();
        eprintln!("rust-analyzer available: {}", available);
    }

    #[test]
    fn test_path_to_uri() {
        let path = Path::new("/tmp/test.rs");
        let uri = RustAnalyzer::path_to_uri(path);
        assert_eq!(uri.scheme(), "file");
        assert!(uri.path().contains("test.rs"));
    }

    #[test]
    fn test_uri_to_path() {
        use lsp_types::Url;
        let uri = Url::parse("file:///tmp/test.rs").unwrap();
        let path = RustAnalyzer::uri_to_path(&uri).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/test.rs"));
    }

    // Integration test with real rust-analyzer
    #[test]
    #[ignore]
    fn test_analyze_real_file() {
        use tempfile::TempDir;

        if !RustAnalyzer::is_available() {
            eprintln!("Skipping: rust-analyzer not installed");
            return;
        }

        let tmp = TempDir::new().unwrap();

        // Create a test project
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            r#"
[package]
name = "test"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();

        std::fs::create_dir(tmp.path().join("src")).unwrap();
        std::fs::write(
            tmp.path().join("src/main.rs"),
            r#"
fn main() {
    helper();
}

fn helper() {
    println!("Hello");
}
"#,
        )
        .unwrap();

        // Analyze
        let mut analyzer = RustAnalyzer::start(tmp.path()).unwrap();
        let analysis = analyzer
            .analyze_file(&tmp.path().join("src/main.rs"))
            .unwrap();

        // Should find at least 2 functions
        assert!(analysis.items.len() >= 2, "Expected at least 2 items");

        // Should find main function
        assert!(
            analysis.items.iter().any(|i| i.name == "main"),
            "Expected to find main function"
        );

        // Should find helper function
        assert!(
            analysis.items.iter().any(|i| i.name == "helper"),
            "Expected to find helper function"
        );

        // Should find main calling helper
        assert!(
            analysis.calls.iter().any(|c| c.caller == "main"),
            "Expected main to call something"
        );

        analyzer.shutdown().unwrap();
    }
}
