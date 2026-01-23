//! Graph operations including traversal and SCC computation.

use crate::error::Result;
use crate::index::Index;
use crate::types::{EdgeBatch, EdgeLabel, NodeId, NodeKind};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

/// In-memory adjacency list for algorithms requiring full graph view.
#[derive(Debug, Clone)]
pub struct AdjacencyList {
    /// Forward edges: node -> [(label, target)]
    forward: BTreeMap<NodeId, Vec<(EdgeLabel, NodeId)>>,
    /// Backward edges: node -> [(label, source)]
    backward: BTreeMap<NodeId, Vec<(EdgeLabel, NodeId)>>,
    /// All nodes in the graph
    nodes: BTreeSet<NodeId>,
}

impl AdjacencyList {
    /// Build adjacency list from edge batches.
    pub fn from_edge_batches<'a>(batches: impl Iterator<Item = &'a EdgeBatch>) -> Self {
        let mut forward: BTreeMap<NodeId, Vec<(EdgeLabel, NodeId)>> = BTreeMap::new();
        let mut backward: BTreeMap<NodeId, Vec<(EdgeLabel, NodeId)>> = BTreeMap::new();
        let mut nodes = BTreeSet::new();

        for batch in batches {
            for edge in &batch.edges {
                nodes.insert(edge.from.clone());
                nodes.insert(edge.to.clone());

                forward
                    .entry(edge.from.clone())
                    .or_default()
                    .push((edge.label, edge.to.clone()));

                backward
                    .entry(edge.to.clone())
                    .or_default()
                    .push((edge.label, edge.from.clone()));
            }
        }

        Self {
            forward,
            backward,
            nodes,
        }
    }

    /// Get outgoing edges for a node.
    pub fn outgoing(&self, node: &NodeId) -> &[(EdgeLabel, NodeId)] {
        self.forward.get(node).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Get incoming edges for a node.
    pub fn incoming(&self, node: &NodeId) -> &[(EdgeLabel, NodeId)] {
        self.backward.get(node).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Get all nodes in the graph.
    pub fn nodes(&self) -> impl Iterator<Item = &NodeId> {
        self.nodes.iter()
    }

    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Number of edges.
    pub fn edge_count(&self) -> usize {
        self.forward.values().map(|v| v.len()).sum()
    }
}

/// Configuration for graph expansion.
#[derive(Debug, Clone)]
pub struct ExpansionConfig {
    /// Maximum depth to expand from seeds.
    pub max_depth: u32,
    /// Edge labels to follow during expansion.
    pub follow_labels: Vec<EdgeLabel>,
    /// Maximum number of nodes to expand.
    pub max_nodes: usize,
    /// Whether to follow edges bidirectionally.
    pub bidirectional: bool,
}

impl Default for ExpansionConfig {
    fn default() -> Self {
        Self {
            max_depth: 2,
            follow_labels: vec![
                EdgeLabel::Imports,
                EdgeLabel::References,
                EdgeLabel::DependsOn,
                EdgeLabel::Contains,
            ],
            max_nodes: 50,
            bidirectional: false,
        }
    }
}

/// Result of graph expansion.
#[derive(Debug, Clone)]
pub struct ExpansionResult {
    /// All nodes reached during expansion.
    pub expanded_nodes: Vec<NodeId>,
    /// Depth at which each node was discovered.
    pub node_depths: HashMap<NodeId, u32>,
    /// Seeds that were used.
    pub seeds: Vec<NodeId>,
    /// Whether expansion was truncated due to max_nodes limit.
    pub truncated: bool,
}

/// Expand graph from seed nodes using BFS.
///
/// Uses the Index for efficient edge lookups rather than loading
/// the full graph into memory.
///
/// # Examples
///
/// ```no_run
/// use ctx_core::{CtxRepo, NodeId, NodeKind, ExpansionConfig, EdgeLabel, expand_from_seeds};
///
/// # fn main() -> ctx_core::Result<()> {
/// let mut repo = CtxRepo::open(".")?;
/// let index = repo.index()?;
///
/// // Start from a specific file
/// let seeds = vec![NodeId {
///     kind: NodeKind::File,
///     id: "src/main.rs".to_string(),
/// }];
///
/// // Expand following imports and dependencies
/// let config = ExpansionConfig {
///     max_depth: 3,
///     max_nodes: 100,
///     follow_labels: vec![EdgeLabel::Imports, EdgeLabel::DependsOn],
///     bidirectional: false,
/// };
///
/// let result = expand_from_seeds(&index, seeds, &config)?;
/// println!("Expanded to {} nodes", result.expanded_nodes.len());
/// # Ok(())
/// # }
/// ```
pub fn expand_from_seeds(
    index: &Index,
    seeds: Vec<NodeId>,
    config: &ExpansionConfig,
) -> Result<ExpansionResult> {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    let mut depths = HashMap::new();
    let mut result = Vec::new();

    // Initialize with seeds
    for seed in &seeds {
        if visited.insert(seed.clone()) {
            queue.push_back((seed.clone(), 0));
            depths.insert(seed.clone(), 0);
        }
    }

    let mut truncated = false;

    while let Some((node, depth)) = queue.pop_front() {
        result.push(node.clone());

        // Check max nodes limit
        if result.len() >= config.max_nodes {
            truncated = !queue.is_empty();
            break;
        }

        // Check depth limit
        if depth >= config.max_depth {
            continue;
        }

        // Expand edges
        for label in &config.follow_labels {
            // Outgoing edges
            if let Ok(neighbors) = index.get_edges_from(&node, *label) {
                for neighbor in neighbors {
                    if visited.insert(neighbor.clone()) {
                        queue.push_back((neighbor.clone(), depth + 1));
                        depths.insert(neighbor.clone(), depth + 1);
                    }
                }
            }

            // Incoming edges (if bidirectional)
            if config.bidirectional {
                if let Ok(neighbors) = index.get_edges_to(&node, *label) {
                    for neighbor in neighbors {
                        if visited.insert(neighbor.clone()) {
                            queue.push_back((neighbor.clone(), depth + 1));
                            depths.insert(neighbor.clone(), depth + 1);
                        }
                    }
                }
            }
        }
    }

    Ok(ExpansionResult {
        expanded_nodes: result,
        node_depths: depths,
        seeds,
        truncated,
    })
}

/// Strongly Connected Component identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SccId(pub u32);

/// View of the graph compressed by SCCs.
#[derive(Debug, Clone)]
pub struct SccView {
    /// Map each node to its SCC ID.
    node_to_scc: HashMap<NodeId, SccId>,
    /// List of nodes in each SCC (index = SccId).
    scc_members: Vec<Vec<NodeId>>,
    /// DAG edges between SCCs (from_scc -> [to_scc]).
    scc_dag: BTreeMap<SccId, Vec<SccId>>,
    /// Topological order of SCCs (if DAG).
    topo_order: Vec<SccId>,
}

impl SccView {
    /// Get the SCC ID for a node.
    pub fn scc_of(&self, node: &NodeId) -> Option<SccId> {
        self.node_to_scc.get(node).copied()
    }

    /// Get all nodes in an SCC.
    pub fn members(&self, scc: SccId) -> &[NodeId] {
        self.scc_members
            .get(scc.0 as usize)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get SCCs that this SCC depends on (outgoing DAG edges).
    pub fn dependencies(&self, scc: SccId) -> &[SccId] {
        self.scc_dag.get(&scc).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Get SCCs that depend on this SCC (incoming DAG edges).
    pub fn dependents(&self, scc: SccId) -> Vec<SccId> {
        self.scc_dag
            .iter()
            .filter_map(|(from, targets)| {
                if targets.contains(&scc) {
                    Some(*from)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get topological ordering of SCCs.
    pub fn topo_order(&self) -> &[SccId] {
        &self.topo_order
    }

    /// Number of SCCs.
    pub fn scc_count(&self) -> usize {
        self.scc_members.len()
    }

    /// Check if two nodes are in the same SCC (mutual recursion / cycle).
    pub fn same_component(&self, a: &NodeId, b: &NodeId) -> bool {
        match (self.node_to_scc.get(a), self.node_to_scc.get(b)) {
            (Some(scc_a), Some(scc_b)) => scc_a == scc_b,
            _ => false,
        }
    }

    /// Export SCC DAG to DOT format for visualization.
    pub fn to_dot(&self) -> String {
        let mut output = String::from("digraph SCC {\n");
        output.push_str("  rankdir=LR;\n");
        output.push_str("  node [shape=box];\n\n");

        // Nodes (SCCs)
        for (i, members) in self.scc_members.iter().enumerate() {
            let _scc_id = SccId(i as u32);
            let label = if members.len() > 1 {
                format!("SCC{} ({} nodes)", i, members.len())
            } else {
                format!("SCC{}", i)
            };
            output.push_str(&format!("  scc{} [label=\"{}\"];\n", i, label));
        }

        output.push('\n');

        // Edges between SCCs
        for (from, targets) in &self.scc_dag {
            for to in targets {
                output.push_str(&format!("  scc{} -> scc{};\n", from.0, to.0));
            }
        }

        output.push_str("}\n");
        output
    }
}

/// Compute SCCs using Tarjan's algorithm.
///
/// This requires the full adjacency list and is O(V + E).
pub fn compute_scc(adjacency: &AdjacencyList) -> SccView {
    struct TarjanState {
        index_counter: u32,
        stack: Vec<NodeId>,
        on_stack: HashSet<NodeId>,
        indices: HashMap<NodeId, u32>,
        lowlinks: HashMap<NodeId, u32>,
        sccs: Vec<Vec<NodeId>>,
    }

    impl TarjanState {
        fn new() -> Self {
            Self {
                index_counter: 0,
                stack: Vec::new(),
                on_stack: HashSet::new(),
                indices: HashMap::new(),
                lowlinks: HashMap::new(),
                sccs: Vec::new(),
            }
        }

        fn strongconnect(&mut self, v: NodeId, adjacency: &AdjacencyList) {
            // Set the depth index for v
            self.indices.insert(v.clone(), self.index_counter);
            self.lowlinks.insert(v.clone(), self.index_counter);
            self.index_counter += 1;
            self.stack.push(v.clone());
            self.on_stack.insert(v.clone());

            // Consider successors of v
            for (_, w) in adjacency.outgoing(&v) {
                if !self.indices.contains_key(w) {
                    // Successor w has not yet been visited; recurse on it
                    self.strongconnect(w.clone(), adjacency);
                    let v_lowlink = *self.lowlinks.get(&v).unwrap();
                    let w_lowlink = *self.lowlinks.get(w).unwrap();
                    self.lowlinks.insert(v.clone(), v_lowlink.min(w_lowlink));
                } else if self.on_stack.contains(w) {
                    // Successor w is in stack and hence in the current SCC
                    let v_lowlink = *self.lowlinks.get(&v).unwrap();
                    let w_index = *self.indices.get(w).unwrap();
                    self.lowlinks.insert(v.clone(), v_lowlink.min(w_index));
                }
            }

            // If v is a root node, pop the stack and print an SCC
            let v_lowlink = *self.lowlinks.get(&v).unwrap();
            let v_index = *self.indices.get(&v).unwrap();

            if v_lowlink == v_index {
                let mut scc = Vec::new();
                loop {
                    let w = self.stack.pop().unwrap();
                    self.on_stack.remove(&w);
                    scc.push(w.clone());
                    if w == v {
                        break;
                    }
                }
                self.sccs.push(scc);
            }
        }
    }

    let mut state = TarjanState::new();

    // Run Tarjan's algorithm on all nodes
    for node in adjacency.nodes() {
        if !state.indices.contains_key(node) {
            state.strongconnect(node.clone(), adjacency);
        }
    }

    // Build node_to_scc map
    let mut node_to_scc = HashMap::new();
    for (i, scc) in state.sccs.iter().enumerate() {
        let scc_id = SccId(i as u32);
        for node in scc {
            node_to_scc.insert(node.clone(), scc_id);
        }
    }

    // Build SCC DAG
    let mut scc_dag: BTreeMap<SccId, BTreeSet<SccId>> = BTreeMap::new();
    for node in adjacency.nodes() {
        let from_scc = node_to_scc[node];
        for (_, target) in adjacency.outgoing(node) {
            let to_scc = node_to_scc[target];
            if from_scc != to_scc {
                scc_dag.entry(from_scc).or_default().insert(to_scc);
            }
        }
    }

    // Convert to Vec
    let scc_dag: BTreeMap<SccId, Vec<SccId>> = scc_dag
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect();

    // Compute topological order using Kahn's algorithm
    let topo_order = compute_topo_order(&scc_dag, state.sccs.len());

    SccView {
        node_to_scc,
        scc_members: state.sccs,
        scc_dag,
        topo_order,
    }
}

/// Compute topological order of SCCs using Kahn's algorithm.
fn compute_topo_order(scc_dag: &BTreeMap<SccId, Vec<SccId>>, scc_count: usize) -> Vec<SccId> {
    // Compute in-degree for each SCC
    let mut in_degree: HashMap<SccId, usize> = HashMap::new();
    for i in 0..scc_count {
        in_degree.insert(SccId(i as u32), 0);
    }
    for targets in scc_dag.values() {
        for target in targets {
            *in_degree.entry(*target).or_insert(0) += 1;
        }
    }

    // Start with nodes having in-degree 0
    let mut queue: VecDeque<SccId> = in_degree
        .iter()
        .filter_map(|(scc, &degree)| if degree == 0 { Some(*scc) } else { None })
        .collect();

    let mut topo_order = Vec::new();

    while let Some(scc) = queue.pop_front() {
        topo_order.push(scc);

        // Reduce in-degree for successors
        if let Some(targets) = scc_dag.get(&scc) {
            for target in targets {
                let degree = in_degree.get_mut(target).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(*target);
                }
            }
        }
    }

    topo_order
}

/// Export full graph to DOT format.
pub fn adjacency_to_dot(adjacency: &AdjacencyList) -> String {
    let mut output = String::from("digraph G {\n");
    output.push_str("  rankdir=LR;\n");
    output.push_str("  node [shape=box];\n\n");

    // Create node labels
    let mut node_labels: HashMap<&NodeId, String> = HashMap::new();
    for (i, node) in adjacency.nodes().enumerate() {
        let label = match node.kind {
            NodeKind::File => format!("File: {}", node.id),
            NodeKind::Module => format!("Mod: {}", node.id),
            NodeKind::Item => format!("Item: {}", node.id),
            NodeKind::Package => format!("Pkg: {}", node.id),
            _ => format!("{:?}: {}", node.kind, node.id),
        };
        node_labels.insert(node, format!("n{}", i));
        output.push_str(&format!(
            "  n{} [label=\"{}\"];\n",
            i,
            escape_dot_label(&label)
        ));
    }

    output.push('\n');

    // Edges
    for node in adjacency.nodes() {
        let from = &node_labels[node];
        for (label, target) in adjacency.outgoing(node) {
            let to = &node_labels[target];
            output.push_str(&format!("  {} -> {} [label=\"{:?}\"];\n", from, to, label));
        }
    }

    output.push_str("}\n");
    output
}

/// Export subgraph around seeds to DOT format.
pub fn expansion_to_dot(
    index: &Index,
    seeds: &[NodeId],
    config: &ExpansionConfig,
) -> Result<String> {
    let expansion = expand_from_seeds(index, seeds.to_vec(), config)?;

    let mut output = String::from("digraph Expansion {\n");
    output.push_str("  rankdir=LR;\n");
    output.push_str("  node [shape=box];\n\n");

    // Create node labels
    let mut node_labels: HashMap<NodeId, String> = HashMap::new();
    for (i, node) in expansion.expanded_nodes.iter().enumerate() {
        let label = match node.kind {
            NodeKind::File => format!("File: {}", node.id),
            NodeKind::Module => format!("Mod: {}", node.id),
            NodeKind::Item => format!("Item: {}", node.id),
            NodeKind::Package => format!("Pkg: {}", node.id),
            _ => format!("{:?}: {}", node.kind, node.id),
        };
        node_labels.insert(node.clone(), format!("n{}", i));

        // Mark seeds differently
        let is_seed = seeds.contains(node);
        let depth = expansion.node_depths.get(node).unwrap_or(&0);
        let color = if is_seed {
            ", style=filled, fillcolor=lightblue"
        } else {
            ""
        };

        output.push_str(&format!(
            "  n{} [label=\"{}\\nd={}\"{  }];\n",
            i,
            escape_dot_label(&label),
            depth,
            color
        ));
    }

    output.push('\n');

    // Edges (only within expanded set)
    let expanded_set: HashSet<_> = expansion.expanded_nodes.iter().collect();
    for node in &expansion.expanded_nodes {
        if let Some(from) = node_labels.get(node) {
            for label in &config.follow_labels {
                if let Ok(neighbors) = index.get_edges_from(node, *label) {
                    for neighbor in neighbors {
                        if expanded_set.contains(&neighbor) {
                            if let Some(to) = node_labels.get(&neighbor) {
                                output.push_str(&format!(
                                    "  {} -> {} [label=\"{:?}\"];\n",
                                    from, to, label
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    output.push_str("}\n");
    Ok(output)
}

/// Escape special characters in DOT labels.
fn escape_dot_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adjacency_from_edge_batches() {
        use crate::types::Edge;

        let edges = vec![Edge {
            from: NodeId {
                kind: NodeKind::File,
                id: "a.rs".to_string(),
            },
            to: NodeId {
                kind: NodeKind::File,
                id: "b.rs".to_string(),
            },
            label: EdgeLabel::Imports,
            weight: None,
            evidence: dummy_evidence(),
        }];

        let batch = EdgeBatch {
            edges,
            created_at: 0,
        };

        let adj = AdjacencyList::from_edge_batches(std::iter::once(&batch));

        assert_eq!(adj.node_count(), 2);
        assert_eq!(adj.edge_count(), 1);
    }

    #[test]
    fn test_scc_no_cycles() {
        use crate::types::Edge;

        // A -> B -> C (DAG)
        let edges = vec![
            Edge {
                from: node_file("a.rs"),
                to: node_file("b.rs"),
                label: EdgeLabel::Imports,
                weight: None,
                evidence: dummy_evidence(),
            },
            Edge {
                from: node_file("b.rs"),
                to: node_file("c.rs"),
                label: EdgeLabel::Imports,
                weight: None,
                evidence: dummy_evidence(),
            },
        ];

        let batch = EdgeBatch {
            edges,
            created_at: 0,
        };

        let adj = AdjacencyList::from_edge_batches(std::iter::once(&batch));
        let scc_view = compute_scc(&adj);

        // Each node should be in its own SCC
        assert_eq!(scc_view.scc_count(), 3);
    }

    #[test]
    fn test_scc_single_cycle() {
        use crate::types::Edge;

        // A -> B -> C -> A (cycle)
        let edges = vec![
            Edge {
                from: node_file("a.rs"),
                to: node_file("b.rs"),
                label: EdgeLabel::Imports,
                weight: None,
                evidence: dummy_evidence(),
            },
            Edge {
                from: node_file("b.rs"),
                to: node_file("c.rs"),
                label: EdgeLabel::Imports,
                weight: None,
                evidence: dummy_evidence(),
            },
            Edge {
                from: node_file("c.rs"),
                to: node_file("a.rs"),
                label: EdgeLabel::Imports,
                weight: None,
                evidence: dummy_evidence(),
            },
        ];

        let batch = EdgeBatch {
            edges,
            created_at: 0,
        };

        let adj = AdjacencyList::from_edge_batches(std::iter::once(&batch));
        let scc_view = compute_scc(&adj);

        // All nodes should be in the same SCC
        assert_eq!(scc_view.scc_count(), 1);
        assert_eq!(scc_view.members(SccId(0)).len(), 3);
    }

    #[test]
    fn test_expansion_depth_1() {
        // This would require a full Index setup, so skipping for now
        // Integration tests will cover this
    }

    fn node_file(id: &str) -> NodeId {
        NodeId {
            kind: NodeKind::File,
            id: id.to_string(),
        }
    }

    fn dummy_evidence() -> crate::types::Evidence {
        use crate::types::{Confidence, Evidence, EvidenceTool};
        Evidence {
            commit_id: crate::ObjectId::from_bytes([0; 32]),
            tool: EvidenceTool::Parser,
            confidence: Confidence::High,
            span: None,
            blob_id: None,
        }
    }
}
