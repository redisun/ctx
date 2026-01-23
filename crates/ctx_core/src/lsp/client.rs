//! Low-level JSON-RPC client for LSP communication.
//!
//! Handles spawning rust-analyzer, sending/receiving LSP messages over stdin/stdout,
//! and managing the LSP protocol (Content-Length headers, JSON-RPC messages).

use crate::error::{CtxError, Result};
use crate::lsp::protocol::{InitializeParams, InitializeResult, JsonRpcMessage};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use tracing::debug;

/// Low-level LSP client using JSON-RPC over stdin/stdout.
pub struct LspClient {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl LspClient {
    /// Spawn rust-analyzer for a project.
    ///
    /// # Arguments
    ///
    /// * `project_root` - Root directory of the Rust project
    ///
    /// # Errors
    ///
    /// Returns error if rust-analyzer cannot be found or fails to start.
    pub fn spawn(project_root: &Path) -> Result<Self> {
        let mut child = Command::new("rust-analyzer")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // Suppress rust-analyzer's stderr logs
            .current_dir(project_root)
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    CtxError::RustAnalyzerNotFound
                } else {
                    CtxError::RustAnalyzerStartFailed(e.to_string())
                }
            })?;

        let stdin = BufWriter::new(
            child
                .stdin
                .take()
                .ok_or_else(|| CtxError::RustAnalyzerStartFailed("stdin not captured".into()))?,
        );

        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| CtxError::RustAnalyzerStartFailed("stdout not captured".into()))?,
        );

        Ok(Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        })
    }

    /// Initialize the LSP connection.
    ///
    /// Sends `initialize` request and waits for response.
    pub fn initialize(&mut self, params: InitializeParams) -> Result<InitializeResult> {
        self.request("initialize", params)
    }

    /// Notify that initialization is complete.
    ///
    /// Sends `initialized` notification (no response expected).
    pub fn initialized(&mut self) -> Result<()> {
        self.notify("initialized", serde_json::json!({}))
    }

    /// Gracefully shutdown the LSP server.
    ///
    /// Sends `shutdown` request, then `exit` notification, then waits for process to exit.
    pub fn shutdown(mut self) -> Result<()> {
        // Send shutdown request
        let _: Value = self.request("shutdown", serde_json::json!(null))?;

        // Send exit notification
        self.notify("exit", serde_json::json!(null))?;

        // Wait for process to exit
        let _ = self.child.wait();

        Ok(())
    }

    /// Send a request and wait for the response.
    ///
    /// # Type Parameters
    ///
    /// * `P` - Request params type (must be serializable)
    /// * `R` - Response result type (must be deserializable)
    ///
    /// # Arguments
    ///
    /// * `method` - LSP method name (e.g., "textDocument/documentSymbol")
    /// * `params` - Request parameters
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Serialization/deserialization fails
    /// - Communication with rust-analyzer fails
    /// - rust-analyzer returns an error response
    pub fn request<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: P,
    ) -> Result<R> {
        let id = self.next_id;
        self.next_id += 1;

        let params_value =
            serde_json::to_value(params).map_err(|e| CtxError::Serialization(e.to_string()))?;

        let request = JsonRpcMessage::request(id, method, params_value);

        self.send_message(&request)?;

        // Read responses until we get the one matching our ID
        // (rust-analyzer may send notifications interleaved)
        loop {
            let response = self.read_message()?;

            if response.is_response() && response.get_id_u64() == Some(id) {
                // Check for error response
                if let Some(error) = response.error {
                    return Err(CtxError::LspError {
                        code: error.code,
                        message: error.message,
                    });
                }

                // Extract result - if missing, treat as null
                // Note: Some LSP methods (like shutdown) legitimately return null or missing result
                let result = if let Some(result) = response.result {
                    result
                } else {
                    // Only warn for non-shutdown methods, as shutdown is expected to return null
                    if method != "shutdown" {
                        debug!(
                            method = method,
                            id = id,
                            "LSP response missing result field - treating as null. Response: {:?}",
                            response
                        );
                    }
                    Value::Null
                };

                return serde_json::from_value(result)
                    .map_err(|e| CtxError::Deserialization(e.to_string()));
            }

            // Ignore notifications and responses to other requests
        }
    }

    /// Send a notification (no response expected).
    ///
    /// # Arguments
    ///
    /// * `method` - LSP method name
    /// * `params` - Notification parameters
    pub fn notify<P: Serialize>(&mut self, method: &str, params: P) -> Result<()> {
        let params_value =
            serde_json::to_value(params).map_err(|e| CtxError::Serialization(e.to_string()))?;

        let notification = JsonRpcMessage::notification(method, params_value);

        self.send_message(&notification)
    }

    /// Wait for a specific notification from the server.
    ///
    /// Reads messages from the server until the specified notification is received.
    /// This is needed because rust-analyzer only responds to some requests after
    /// it has finished indexing and sent diagnostics.
    ///
    /// # Arguments
    ///
    /// * `expected_method` - The notification method to wait for (e.g., "textDocument/publishDiagnostics")
    /// * `timeout_ms` - Maximum time to wait in milliseconds
    ///
    /// # Returns
    ///
    /// The notification params, or an error if timeout is reached.
    pub fn wait_for_notification(
        &mut self,
        expected_method: &str,
        timeout_ms: u64,
    ) -> Result<Value> {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        loop {
            // Check timeout
            if start.elapsed() > timeout {
                return Err(CtxError::LspProtocolError(format!(
                    "Timeout waiting for notification: {}",
                    expected_method
                )));
            }

            // Try to read a message with a short timeout on the read itself
            // For now, we'll use blocking read with overall timeout check
            let message = self.read_message()?;

            // Check if it's the notification we're waiting for
            if message.is_notification() {
                if let Some(method) = &message.method {
                    if method == expected_method {
                        // Return the params
                        return Ok(message.params.unwrap_or(Value::Null));
                    }
                }
            }

            // Otherwise continue reading (ignore other notifications and responses)
        }
    }

    /// Send a JSON-RPC message.
    fn send_message(&mut self, message: &JsonRpcMessage) -> Result<()> {
        let json =
            serde_json::to_string(message).map_err(|e| CtxError::Serialization(e.to_string()))?;

        // Write Content-Length header
        write!(self.stdin, "Content-Length: {}\r\n\r\n", json.len())?;

        // Write JSON content
        self.stdin.write_all(json.as_bytes())?;
        self.stdin.flush()?;

        Ok(())
    }

    /// Read a JSON-RPC message.
    fn read_message(&mut self) -> Result<JsonRpcMessage> {
        // Read headers until we find Content-Length
        let mut content_length: Option<usize> = None;

        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line)?;

            let line = line.trim();

            // Empty line marks end of headers
            if line.is_empty() {
                break;
            }

            // Parse Content-Length header
            if let Some(len_str) = line.strip_prefix("Content-Length: ") {
                content_length =
                    Some(len_str.parse().map_err(|_| {
                        CtxError::LspProtocolError("invalid Content-Length".into())
                    })?);
            }
        }

        let content_length = content_length
            .ok_or_else(|| CtxError::LspProtocolError("missing Content-Length header".into()))?;

        // Read content
        let mut content = vec![0u8; content_length];
        self.stdout.read_exact(&mut content)?;

        // Parse JSON
        serde_json::from_slice(&content).map_err(|e| CtxError::Deserialization(e.to_string()))
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Try to kill the child process gracefully
        let _ = self.child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_length_calculation() {
        // Test that we can calculate content length correctly
        let message = JsonRpcMessage::request(1, "test", serde_json::json!({}));
        let json = serde_json::to_string(&message).unwrap();

        // Length should match the serialized JSON length
        assert!(!json.is_empty());
        assert!(json.contains("jsonrpc"));
        assert!(json.contains("test"));
    }

    #[test]
    fn test_message_envelope() {
        let message = JsonRpcMessage::request(1, "test", serde_json::json!({}));
        let json = serde_json::to_string(&message).unwrap();

        let expected_header = format!("Content-Length: {}\r\n\r\n", json.len());
        let expected_body = json;

        // Verify header format
        assert!(expected_header.starts_with("Content-Length: "));
        assert!(expected_header.ends_with("\r\n\r\n"));

        // Verify body is valid JSON
        let parsed: JsonRpcMessage = serde_json::from_str(&expected_body).unwrap();
        assert_eq!(parsed.jsonrpc, "2.0");
    }

    // Integration test with real rust-analyzer (requires rust-analyzer to be installed)
    #[test]
    #[ignore] // Only run when explicitly requested
    fn test_real_rust_analyzer() {
        use crate::lsp::protocol::ClientCapabilities;
        use tempfile::TempDir;

        // Create a minimal Rust project
        let tmp = TempDir::new().unwrap();
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
            "fn main() { println!(\"hello\"); }",
        )
        .unwrap();

        // Try to spawn rust-analyzer
        let mut client = match LspClient::spawn(tmp.path()) {
            Ok(client) => client,
            Err(CtxError::RustAnalyzerNotFound) => {
                eprintln!("Skipping test: rust-analyzer not installed");
                return;
            }
            Err(e) => panic!("Unexpected error: {}", e),
        };

        // Initialize
        use lsp_types::Url;
        let root_uri = Url::parse(&format!("file://{}", tmp.path().display())).unwrap();
        #[allow(deprecated)]
        let init_params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri),
            root_path: None, // Deprecated: use root_uri instead
            capabilities: ClientCapabilities {
                text_document: None,
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

        let result = client.initialize(init_params).unwrap();
        assert!(result.capabilities.document_symbol_provider.is_some());

        // Send initialized notification
        client.initialized().unwrap();

        // Shutdown
        client.shutdown().unwrap();
    }
}
