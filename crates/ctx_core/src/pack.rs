//! Prompt pack compilation for LLM context.

use crate::error::Result;
use crate::graph::{expand_from_seeds, ExpansionConfig};
use crate::types::{EdgeLabel, NodeId, NodeKind};
use crate::{CtxRepo, Index, NameNamespace, ObjectId};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Compiled retrieval result ready for LLM consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptPack {
    /// The task or query being addressed.
    pub task: String,
    /// Commit this pack was built from.
    pub head_commit: ObjectId,
    /// Retrieved content chunks.
    pub retrieved: Vec<RetrievedChunk>,
    /// Graph expansion metadata.
    pub graph_context: GraphContext,
    /// Recent narrative excerpts.
    pub recent_narrative: String,
    /// Token budget accounting.
    pub token_budget: TokenBudget,
}

/// A chunk of retrieved content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedChunk {
    /// Human-readable title for this chunk.
    pub title: String,
    /// ObjectId of the source (blob or typed object).
    pub object_id: ObjectId,
    /// The actual content snippet.
    pub snippet: String,
    /// Relevance score as fixed-point u32 (1000 = 1.0, 500 = 0.5, etc).
    /// This allows deterministic serialization while avoiding floating point.
    /// Range: 0-1000 representing 0.0-1.0.
    pub relevance_score: u32,
    /// What kind of content this is.
    pub chunk_kind: ChunkKind,
}

/// Categorizes the type of content in a retrieved chunk.
///
/// Used during pack building to organize and prioritize different types of context.
/// For example, `FileContent` chunks contain actual source code, while `NarrativeExcerpt`
/// chunks contain agent session logs and task descriptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChunkKind {
    /// Source code or configuration file content.
    FileContent,
    /// Agent session logs, task descriptions, or decision records.
    NarrativeExcerpt,
    /// Architectural decision records (ADRs).
    Decision,
    /// Compiler errors, warnings, or LSP diagnostics.
    DiagnosticOutput,
    /// Function, struct, or type definitions from code analysis.
    SymbolDefinition,
}

/// Graph expansion context for debugging/transparency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphContext {
    /// Seed nodes used to start expansion.
    pub seed_nodes: Vec<String>,
    /// Nodes reached during expansion.
    pub expanded_nodes: Vec<String>,
    /// Depth of expansion.
    pub expansion_depth: u32,
    /// Whether SCC DAG was used.
    pub scc_dag_used: bool,
}

/// Token budget tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudget {
    /// Total budget.
    pub total: u32,
    /// Tokens used by retrieved content.
    pub used: u32,
    /// Tokens reserved for LLM response.
    pub reserved_for_response: u32,
}

/// Configuration for retrieval pipeline.
#[derive(Debug, Clone)]
pub struct RetrievalConfig {
    /// Total token budget.
    pub token_budget: u32,
    /// Tokens to reserve for LLM response.
    pub response_reserve: u32,
    /// Graph expansion depth.
    pub expansion_depth: u32,
    /// Edge labels to follow during expansion.
    pub expand_labels: Vec<EdgeLabel>,
    /// Maximum nodes to expand.
    pub max_expanded_nodes: usize,
    /// Include narrative from last N days.
    pub narrative_days: u32,
    /// Include active task content.
    pub include_active_task: bool,
    /// Include daily log entries.
    pub include_log: bool,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            token_budget: 16000,
            response_reserve: 4000,
            expansion_depth: 2,
            expand_labels: vec![
                EdgeLabel::Imports,
                EdgeLabel::References,
                EdgeLabel::DependsOn,
                EdgeLabel::Defines, // Follow File -> Item edges to find source files
            ],
            max_expanded_nodes: 50,
            narrative_days: 7,
            include_active_task: true,
            include_log: true,
        }
    }
}

impl PromptPack {
    /// Serialize to JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| crate::error::CtxError::Serialization(e.to_string()))
    }

    /// Format as human-readable text.
    pub fn to_text(&self) -> String {
        let mut output = String::new();

        output.push_str(&format!("# Prompt Pack: {}\n\n", self.task));
        output.push_str(&format!("**Commit:** {}\n", self.head_commit));
        output.push_str(&format!(
            "**Tokens:** {}/{} (reserved: {})\n\n",
            self.token_budget.used,
            self.token_budget.total,
            self.token_budget.reserved_for_response
        ));

        output.push_str("## Graph Context\n\n");
        output.push_str(&format!(
            "- Seeds: {}\n",
            self.graph_context.seed_nodes.join(", ")
        ));
        output.push_str(&format!(
            "- Expanded: {} nodes\n",
            self.graph_context.expanded_nodes.len()
        ));
        output.push_str(&format!(
            "- Depth: {}\n\n",
            self.graph_context.expansion_depth
        ));

        if !self.recent_narrative.is_empty() {
            output.push_str("## Recent Narrative\n\n");
            output.push_str(&self.recent_narrative);
            output.push_str("\n\n");
        }

        output.push_str("## Retrieved Content\n\n");
        for chunk in &self.retrieved {
            output.push_str(&format!(
                "### {} (score: {:.3}, kind: {:?})\n\n",
                chunk.title,
                chunk.relevance_score as f32 / 1000.0,
                chunk.chunk_kind
            ));
            output.push_str(&chunk.snippet);
            output.push_str("\n\n");
        }

        output
    }
}

/// Parse query to identify seed nodes.
pub fn parse_query_for_seeds(query: &str, index: &Index) -> Result<Vec<NodeId>> {
    let mut seeds = Vec::new();
    let mut seen = HashSet::new();

    // Split query into tokens
    let tokens: Vec<&str> = query
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|s| !s.is_empty())
        .collect();

    for token in tokens {
        // Check if it looks like a file path
        if looks_like_path(token) {
            let normalized = normalize_path(token);
            if let Ok(Some(_obj_id)) = index.lookup_path(&normalized) {
                let node = NodeId {
                    kind: NodeKind::File,
                    id: normalized.clone(),
                };
                if seen.insert(node.clone()) {
                    seeds.push(node);
                }
            }
        }

        // Extract identifiers (alphanumeric + underscore)
        for ident in extract_identifiers(token) {
            // Try Item namespace (most common)
            if let Ok(obj_ids) = index.lookup_name(NameNamespace::Item, ident) {
                for _obj_id in obj_ids {
                    // Create a node from the identifier
                    let node = NodeId {
                        kind: NodeKind::Item,
                        id: ident.to_string(),
                    };
                    if seen.insert(node.clone()) {
                        seeds.push(node);
                    }
                }
            }

            // Try Module namespace
            if let Ok(obj_ids) = index.lookup_name(NameNamespace::Module, ident) {
                for _obj_id in obj_ids {
                    let node = NodeId {
                        kind: NodeKind::Module,
                        id: ident.to_string(),
                    };
                    if seen.insert(node.clone()) {
                        seeds.push(node);
                    }
                }
            }
        }
    }

    Ok(seeds)
}

/// Check if a string looks like a file path.
fn looks_like_path(s: &str) -> bool {
    s.contains('/')
        || s.ends_with(".rs")
        || s.ends_with(".py")
        || s.ends_with(".js")
        || s.ends_with(".ts")
        || s.ends_with(".toml")
        || s.ends_with(".md")
}

/// Normalize file path.
fn normalize_path(path: &str) -> String {
    path.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// Extract identifiers from a token (alphanumeric + underscore sequences).
fn extract_identifiers(s: &str) -> Vec<&str> {
    let mut idents = Vec::new();
    let mut start = None;

    for (i, c) in s.char_indices() {
        if c.is_alphanumeric() || c == '_' {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s_idx) = start {
            if i > s_idx {
                idents.push(&s[s_idx..i]);
            }
            start = None;
        }
    }

    // Handle trailing identifier
    if let Some(s_idx) = start {
        if s.len() > s_idx {
            idents.push(&s[s_idx..]);
        }
    }

    idents
}

/// Estimate token count using chars/4 heuristic.
pub fn estimate_tokens(text: &str) -> u32 {
    (text.chars().count() / 4) as u32
}

/// Build a prompt pack from a query.
///
/// This is the main entry point for the retrieval pipeline. Given a natural language
/// query, it identifies relevant files, expands the context graph, and assembles
/// a structured pack of information for LLM consumption.
///
/// # Examples
///
/// ```no_run
/// use ctx_core::{CtxRepo, RetrievalConfig, EdgeLabel, build_pack};
///
/// # fn main() -> ctx_core::Result<()> {
/// let mut repo = CtxRepo::open(".")?;
///
/// let config = RetrievalConfig {
///     token_budget: 10000,
///     response_reserve: 2000,
///     expansion_depth: 2,
///     expand_labels: vec![EdgeLabel::Imports, EdgeLabel::Calls],
///     max_expanded_nodes: 50,
///     narrative_days: 7,
///     include_active_task: true,
///     include_log: false,
/// };
///
/// let pack = build_pack(
///     &mut repo,
///     "authentication middleware",
///     &config,
/// )?;
///
/// println!("Retrieved {} chunks", pack.retrieved.len());
/// # Ok(())
/// # }
/// ```
///
/// # Borrow Checker Notes
///
/// This function needs to access multiple parts of CtxRepo (index, object_store, narrative).
/// Since `repo.index()` takes `&mut self` (for lazy loading), we can't hold that borrow
/// while accessing other parts. The pattern used is:
/// 1. Borrow index, collect what we need, drop the borrow
/// 2. Borrow object_store or narrative as needed
/// 3. The scoped blocks make these borrow lifetimes explicit
pub fn build_pack(repo: &mut CtxRepo, query: &str, config: &RetrievalConfig) -> Result<PromptPack> {
    let head_commit = repo.head_id()?;

    // Step 1: Identify seeds from the query
    // Note: repo.index() takes &mut self for lazy loading, so we scope it
    // to drop the borrow before subsequent operations
    let seeds = {
        let index = repo.index()?;
        parse_query_for_seeds(query, index)?
    };

    // Step 2: Expand graph from seeds
    let expansion_config = ExpansionConfig {
        max_depth: config.expansion_depth,
        follow_labels: config.expand_labels.clone(),
        max_nodes: config.max_expanded_nodes,
        bidirectional: true, // Follow edges in both directions to find files that define items
    };

    let expansion = if seeds.is_empty() {
        // No seeds found - return empty expansion
        crate::graph::ExpansionResult {
            expanded_nodes: Vec::new(),
            node_depths: std::collections::HashMap::new(),
            seeds: Vec::new(),
            truncated: false,
        }
    } else {
        // Scope for index borrow
        let index = repo.index()?;
        expand_from_seeds(index, seeds.clone(), &expansion_config)?
    };

    // Step 3: Retrieve file content for expanded nodes
    // Strategy: First collect ObjectIds (requires index), then load content (requires object_store)
    // We can't hold both borrows simultaneously, so we do it in two passes
    let file_metadata: Vec<(NodeId, ObjectId, u32)> = {
        let index = repo.index()?;
        expansion
            .expanded_nodes
            .iter()
            .filter_map(|node| {
                if node.kind == NodeKind::File {
                    let depth = expansion.node_depths.get(node).copied().unwrap_or(0);
                    // Compute relevance as fixed-point: 1000 / (1 + depth)
                    // depth=0: 1000 (1.0), depth=1: 500 (0.5), depth=2: 333 (0.333), etc.
                    let relevance_score = 1000 / (1 + depth);
                    if let Ok(Some(obj_id)) = index.lookup_path(&node.id) {
                        Some((node.clone(), obj_id, relevance_score))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    };

    // Load file content using the collected ObjectIds
    let object_store = repo.object_store();
    let mut chunks = Vec::new();
    for (node, obj_id, relevance_score) in file_metadata {
        if let Ok(content_bytes) = object_store.get_blob(obj_id) {
            if let Ok(content) = String::from_utf8(content_bytes) {
                chunks.push(RetrievedChunk {
                    title: node.id.clone(),
                    object_id: obj_id,
                    snippet: content,
                    relevance_score,
                    chunk_kind: ChunkKind::FileContent,
                });
            }
        }
    }

    // Step 4: Include narrative
    let mut narrative_content = String::new();

    if config.include_active_task || config.include_log {
        let narrative = repo.narrative();
        if let Ok(files) = narrative.list_files() {
            // Look for task files
            if config.include_active_task {
                for file in &files {
                    if file.starts_with("tasks/") && file.ends_with(".md") {
                        if let Ok(content_bytes) = narrative.read_file(file) {
                            if let Ok(content) = String::from_utf8(content_bytes) {
                                narrative_content.push_str(&format!("## Task: {}\n\n", file));
                                narrative_content.push_str(&content);
                                narrative_content.push_str("\n\n");
                                break; // Just include first task for now
                            }
                        }
                    }
                }
            }

            // Look for log files
            if config.include_log {
                let mut log_files: Vec<_> = files
                    .iter()
                    .filter(|f| f.starts_with("log/") && f.ends_with(".md"))
                    .collect();
                log_files.sort();
                log_files.reverse(); // Most recent first

                for file in log_files.iter().take(5) {
                    if let Ok(content_bytes) = narrative.read_file(file) {
                        if let Ok(content) = String::from_utf8(content_bytes) {
                            narrative_content.push_str(&format!("## Log: {}\n\n", file));
                            narrative_content.push_str(&content);
                            narrative_content.push_str("\n\n");
                        }
                    }
                }
            }
        }
    }

    // Step 5: Budget allocation
    let available_tokens = config.token_budget.saturating_sub(config.response_reserve);
    let narrative_tokens = estimate_tokens(&narrative_content);

    // Sort chunks by relevance score (descending)
    chunks.sort_by(|a, b| b.relevance_score.cmp(&a.relevance_score));

    // Greedily fill budget
    let mut selected_chunks = Vec::new();
    let mut tokens_used = narrative_tokens;

    for chunk in chunks {
        let chunk_tokens = estimate_tokens(&chunk.snippet);
        if tokens_used + chunk_tokens <= available_tokens {
            tokens_used += chunk_tokens;
            selected_chunks.push(chunk);
        } else {
            // Budget exceeded
            break;
        }
    }

    // Build graph context
    let graph_context = GraphContext {
        seed_nodes: seeds
            .iter()
            .map(|n| format!("{:?}::{}", n.kind, n.id))
            .collect(),
        expanded_nodes: expansion
            .expanded_nodes
            .iter()
            .map(|n| format!("{:?}::{}", n.kind, n.id))
            .collect(),
        expansion_depth: config.expansion_depth,
        scc_dag_used: false,
    };

    Ok(PromptPack {
        task: query.to_string(),
        head_commit,
        retrieved: selected_chunks,
        graph_context,
        recent_narrative: narrative_content,
        token_budget: TokenBudget {
            total: config.token_budget,
            used: tokens_used,
            reserved_for_response: config.response_reserve,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_path() {
        assert!(looks_like_path("src/main.rs"));
        assert!(looks_like_path("test.py"));
        assert!(looks_like_path("/absolute/path.js"));
        assert!(!looks_like_path("function_name"));
        assert!(!looks_like_path("SomeType"));
    }

    #[test]
    fn test_extract_identifiers() {
        assert_eq!(extract_identifiers("hello_world"), vec!["hello_world"]);
        assert_eq!(
            extract_identifiers("foo::bar::baz"),
            vec!["foo", "bar", "baz"]
        );
        assert_eq!(extract_identifiers("fn test() {}"), vec!["fn", "test"]);
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("1234"), 1); // 4 chars / 4 = 1
        assert_eq!(estimate_tokens("12345678"), 2); // 8 chars / 4 = 2
        assert_eq!(estimate_tokens("hello world"), 2); // 11 chars / 4 = 2
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("\"src/main.rs\""), "src/main.rs");
        assert_eq!(normalize_path("'test.py'"), "test.py");
        assert_eq!(normalize_path("normal.rs"), "normal.rs");
    }
}
