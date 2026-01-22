//! High-level LSP query wrappers.
//!
//! Provides convenient methods for common LSP operations like getting document symbols,
//! finding references, and navigating call hierarchies.
//!
//! Note: Some methods are currently unused but reserved for future LSP features.

#![allow(dead_code)]

use crate::error::Result;
use crate::lsp::client::LspClient;
use crate::lsp::protocol::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, DocumentSymbol,
    Location, Position, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, Url,
};
use lsp_types::Hover;

/// High-level LSP query operations.
pub struct LspQueries<'a> {
    client: &'a mut LspClient,
}

impl<'a> LspQueries<'a> {
    /// Create a new query wrapper around a client.
    pub fn new(client: &'a mut LspClient) -> Self {
        Self { client }
    }

    /// Open a text document in the LSP server.
    ///
    /// This notifies rust-analyzer that we're working with this file.
    ///
    /// # Arguments
    ///
    /// * `uri` - File URI
    /// * `content` - File content
    /// * `version` - File version number (should start at 1)
    pub fn did_open(&mut self, uri: &Url, content: &str, version: i32) -> Result<()> {
        let params = serde_json::json!({
            "textDocument": TextDocumentItem {
                uri: uri.clone(),
                language_id: "rust".to_string(),
                version,
                text: content.to_string(),
            }
        });

        self.client.notify("textDocument/didOpen", params)
    }

    /// Notify the server that a document's content has changed.
    ///
    /// # Arguments
    ///
    /// * `uri` - File URI
    /// * `content` - New file content
    /// * `version` - New file version number (must be greater than previous version)
    pub fn did_change(&mut self, uri: &Url, content: &str, version: i32) -> Result<()> {
        use lsp_types::{TextDocumentContentChangeEvent, VersionedTextDocumentIdentifier};
        
        let params = serde_json::json!({
            "textDocument": VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version,
            },
            "contentChanges": vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: content.to_string(),
            }],
        });

        self.client.notify("textDocument/didChange", params)
    }

    /// Close a text document.
    pub fn did_close(&mut self, uri: &Url) -> Result<()> {
        let params = serde_json::json!({
            "textDocument": TextDocumentIdentifier {
                uri: uri.clone(),
            }
        });

        self.client.notify("textDocument/didClose", params)
    }

    /// Get document symbols (file outline).
    ///
    /// Returns hierarchical list of all symbols (functions, structs, etc.) in the file.
    pub fn document_symbols(&mut self, uri: &Url) -> Result<Vec<DocumentSymbol>> {
        let params = serde_json::json!({
            "textDocument": TextDocumentIdentifier {
                uri: uri.clone(),
            }
        });

        self.client.request("textDocument/documentSymbol", params)
    }

    /// Go to definition of a symbol at a position.
    ///
    /// Returns the location(s) where the symbol is defined.
    pub fn goto_definition(&mut self, uri: &Url, position: Position) -> Result<Vec<Location>> {
        let params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.clone(),
            },
            position,
        };

        // Response can be Location | Location[] | null
        let response: serde_json::Value = self.client.request("textDocument/definition", params)?;

        match response {
            serde_json::Value::Null => Ok(vec![]),
            serde_json::Value::Array(arr) => {
                let mut locations = Vec::new();
                for item in arr {
                    if let Ok(loc) = serde_json::from_value(item) {
                        locations.push(loc);
                    }
                }
                Ok(locations)
            }
            _ => {
                // Single location
                serde_json::from_value(response)
                    .map(|loc| vec![loc])
                    .map_err(|e| crate::error::CtxError::Deserialization(e.to_string()))
            }
        }
    }

    /// Find all references to a symbol.
    pub fn find_references(
        &mut self,
        uri: &Url,
        position: Position,
        include_declaration: bool,
    ) -> Result<Vec<Location>> {
        let params = serde_json::json!({
            "textDocument": TextDocumentIdentifier {
                uri: uri.clone(),
            },
            "position": position,
            "context": {
                "includeDeclaration": include_declaration
            }
        });

        // textDocument/references can return Location[] | null
        // If result is missing or null, treat as empty array (no references found)
        let response: Option<Vec<Location>> =
            self.client.request("textDocument/references", params)?;

        Ok(response.unwrap_or_default())
    }

    /// Prepare call hierarchy at a position.
    ///
    /// Returns call hierarchy items that can be used with incoming/outgoing calls.
    pub fn prepare_call_hierarchy(
        &mut self,
        uri: &Url,
        position: Position,
    ) -> Result<Vec<CallHierarchyItem>> {
        let params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.clone(),
            },
            position,
        };

        let response: Option<Vec<CallHierarchyItem>> =
            self.client.request("textDocument/prepareCallHierarchy", params)?;

        Ok(response.unwrap_or_default())
    }

    /// Get incoming calls (what calls this function).
    pub fn call_hierarchy_incoming(
        &mut self,
        item: &CallHierarchyItem,
    ) -> Result<Vec<CallHierarchyIncomingCall>> {
        let params = serde_json::json!({
            "item": item
        });

        let response: Option<Vec<CallHierarchyIncomingCall>> =
            self.client.request("callHierarchy/incomingCalls", params)?;

        Ok(response.unwrap_or_default())
    }

    /// Get outgoing calls (what this function calls).
    pub fn call_hierarchy_outgoing(
        &mut self,
        item: &CallHierarchyItem,
    ) -> Result<Vec<CallHierarchyOutgoingCall>> {
        let params = serde_json::json!({
            "item": item
        });

        let response: Option<Vec<CallHierarchyOutgoingCall>> =
            self.client.request("callHierarchy/outgoingCalls", params)?;

        Ok(response.unwrap_or_default())
    }

    /// Get hover information (type, documentation, etc.).
    pub fn hover(&mut self, uri: &Url, position: Position) -> Result<Option<Hover>> {
        let params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: uri.clone(),
            },
            position,
        };

        self.client.request("textDocument/hover", params)
    }

    // Note: rust-analyzer does not support textDocument/prepareTypeHierarchy
    // or typeHierarchy/supertypes methods as of 2025.
    // These LSP 3.17 features are not implemented yet.
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_queries_compiles() {
        // This test just ensures the types are compatible
        // Real testing requires a running rust-analyzer instance
    }
}
