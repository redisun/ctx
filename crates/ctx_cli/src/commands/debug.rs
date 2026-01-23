//! Debug and inspection commands.

use anyhow::{Context, Result};
use chrono::DateTime;
use ctx_core::{Commit, CtxRepo, ObjectId, ObjectStore};
use std::collections::{HashSet, VecDeque};
use std::path::Path;

/// Print the raw contents of an object.
///
/// Handles both Blob (raw data) and Typed (structured) objects.
/// For blobs, prints the raw content (UTF-8 or hex dump).
/// For typed objects, pretty-prints the deserialized structure.
pub fn cat(object_id: &str) -> Result<()> {
    let ctx_dir = Path::new(".ctx");
    if !ctx_dir.exists() {
        anyhow::bail!("Not a CTX repository (no .ctx directory found)");
    }

    let store = ObjectStore::new(ctx_dir.join("objects"));
    let id = ObjectId::from_hex(object_id).context("Invalid object ID format")?;

    // Check if object exists
    if !store.exists(id) {
        anyhow::bail!("Object {} not found", object_id);
    }

    // Try as blob first
    match store.get_blob(id) {
        Ok(data) => {
            // It's a blob - print raw content
            print_blob_content(&data);
            Ok(())
        }
        Err(_) => {
            // Not a blob, try typed objects
            print_typed_object(&store, id)
        }
    }
}

/// Print blob content as UTF-8 or hex dump.
fn print_blob_content(data: &[u8]) {
    if let Ok(s) = std::str::from_utf8(data) {
        println!("{}", s);
    } else {
        println!("(binary data, {} bytes)", data.len());
        println!();

        // Print hex dump for first 512 bytes
        for (i, chunk) in data
            .iter()
            .take(512)
            .collect::<Vec<_>>()
            .chunks(16)
            .enumerate()
        {
            print!("{:04x}  ", i * 16);

            // Hex representation
            for (j, b) in chunk.iter().enumerate() {
                print!("{:02x}", b);
                if j % 2 == 1 {
                    print!(" ");
                }
            }

            // Padding for short lines
            for _ in 0..(16 - chunk.len()) {
                print!("  ");
                if chunk.len() % 2 == 0 {
                    print!(" ");
                }
            }

            print!(" ");

            // ASCII representation
            for b in chunk {
                let c = **b as char;
                if c.is_ascii_graphic() || c == ' ' {
                    print!("{}", c);
                } else {
                    print!(".");
                }
            }

            println!();
        }

        if data.len() > 512 {
            println!();
            println!("... ({} more bytes)", data.len() - 512);
        }
    }
}

/// Try to deserialize and print typed objects.
fn print_typed_object(store: &ObjectStore, id: ObjectId) -> Result<()> {
    use ctx_core::{Commit, EdgeBatch, NarrativeRef, Tree, WorkCommit};

    // Try each known type
    // Start with most common types

    // Try Commit
    if let Ok(commit) = store.get_typed::<Commit>(id) {
        println!("Type: Commit");
        println!(
            "Parents: {:?}",
            commit
                .parents
                .iter()
                .map(|p| p.as_hex())
                .collect::<Vec<_>>()
        );
        println!(
            "Timestamp: {} ({})",
            commit.timestamp_unix,
            DateTime::from_timestamp(commit.timestamp_unix as i64, 0)
                .unwrap_or_default()
                .format("%Y-%m-%d %H:%M:%S UTC")
        );
        println!("Message: {}", commit.message);
        println!("Root tree: {}", commit.root_tree.as_hex());
        println!("Edge batches: {} batch(es)", commit.edge_batches.len());
        for (i, batch_id) in commit.edge_batches.iter().enumerate() {
            println!("  [{}] {}", i, batch_id.as_hex());
        }
        println!("Narrative refs: {} ref(s)", commit.narrative_refs.len());
        for (i, nref) in commit.narrative_refs.iter().enumerate() {
            println!("  [{}] {} ({})", i, nref.path, nref.role);
        }
        if let Some(commit_type) = &commit.commit_type {
            println!("Commit type: {:?}", commit_type);
        }
        if let Some(cargo) = commit.cargo_snapshot {
            println!("Cargo snapshot: {}", cargo.as_hex());
        }
        if let Some(rust) = commit.rust_snapshot {
            println!("Rust snapshot: {}", rust.as_hex());
        }
        if let Some(diag) = commit.diagnostics_snapshot {
            println!("Diagnostics snapshot: {}", diag.as_hex());
        }
        return Ok(());
    }

    // Try WorkCommit
    if let Ok(work) = store.get_typed::<WorkCommit>(id) {
        println!("Type: WorkCommit");
        println!(
            "Parents: {:?}",
            work.parents.iter().map(|p| p.as_hex()).collect::<Vec<_>>()
        );
        println!("Base: {}", work.base.as_hex());
        println!("Session ID: {}", work.session_id);
        println!("Created at: {}", work.created_at);
        println!("Step kind: {:?}", work.step_kind);
        println!("Payload: {} bytes", work.payload.len());
        println!("Narrative refs: {} ref(s)", work.narrative_refs.len());
        println!("Session state: {:?}", work.session_state);
        println!("Task: {}", work.task_description);
        return Ok(());
    }

    // Try Tree
    if let Ok(tree) = store.get_typed::<Tree>(id) {
        println!("Type: Tree");
        println!("Entries: {} item(s)", tree.entries.len());
        for entry in &tree.entries {
            let kind_char = match entry.kind {
                ctx_core::TreeEntryKind::Blob => 'B',
                ctx_core::TreeEntryKind::Tree => 'T',
            };
            println!("  {} {} {}", kind_char, entry.id.as_hex(), entry.name);
        }
        return Ok(());
    }

    // Try EdgeBatch
    if let Ok(batch) = store.get_typed::<EdgeBatch>(id) {
        println!("Type: EdgeBatch");
        println!("Created at: {}", batch.created_at);
        println!("Edges: {} edge(s)", batch.edges.len());
        println!(
            "(Note: To find introducing commit, query which commit references this EdgeBatch)"
        );
        for (i, edge) in batch.edges.iter().enumerate() {
            println!(
                "  [{}] {:?}:{} --{:?}--> {:?}:{}",
                i, edge.from.kind, edge.from.id, edge.label, edge.to.kind, edge.to.id
            );
            println!(
                "      Evidence: {:?} (confidence: {:?})",
                edge.evidence.tool, edge.evidence.confidence
            );
        }
        return Ok(());
    }

    // Try NarrativeRef (unlikely to be stored directly, but possible)
    if let Ok(nref) = store.get_typed::<NarrativeRef>(id) {
        println!("Type: NarrativeRef");
        println!("Path: {}", nref.path);
        println!("Stream: {:?}", nref.stream);
        println!("Role: {}", nref.role);
        println!("Blob ID: {}", nref.blob_id.as_hex());
        return Ok(());
    }

    // Unknown typed object
    anyhow::bail!("Object {} is a typed object but type could not be determined. Try using the raw object inspection tools.", id.as_hex())
}

/// List all references (HEAD, STAGE, and named refs).
pub fn refs() -> Result<()> {
    let repo = CtxRepo::open(".").context("Not a CTX repository (no .ctx directory found)")?;

    // Show HEAD
    match repo.refs().read_head() {
        Ok(head_id) => {
            println!("HEAD -> {}", head_id.as_hex());
        }
        Err(_) => {
            println!("HEAD -> (not set)");
        }
    }

    // Show STAGE if it exists
    match repo.refs().read_stage()? {
        Some(stage_id) => {
            println!("STAGE -> {}", stage_id.as_hex());
        }
        None => {
            println!("STAGE -> (not set)");
        }
    }

    println!();

    // Show all named refs
    let refs = repo.refs().list_refs()?;

    if refs.is_empty() {
        println!("No named refs found.");
    } else {
        println!("Named refs:");
        for (name, id) in refs {
            println!("  refs/{} -> {}", name, id.as_hex());
        }
    }

    Ok(())
}

/// Show commit history from HEAD.
pub fn history(limit: Option<usize>) -> Result<()> {
    let repo = CtxRepo::open(".").context("Not a CTX repository (no .ctx directory found)")?;
    let head_id = repo.head_id().context("HEAD not found")?;

    let mut count = 0;
    let max_count = limit.unwrap_or(usize::MAX);

    let mut queue = VecDeque::new();
    let mut seen = HashSet::new();

    queue.push_back(head_id);

    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) || count >= max_count {
            continue;
        }

        let commit: Commit = repo
            .object_store()
            .get_typed(id)
            .with_context(|| format!("Failed to read commit {}", id.as_hex()))?;

        // Format timestamp
        let timestamp =
            DateTime::from_timestamp(commit.timestamp_unix as i64, 0).unwrap_or_default();
        let formatted_time = timestamp.format("%Y-%m-%d %H:%M:%S UTC");

        // Print commit info
        println!("commit {}", id.as_hex());
        if !commit.parents.is_empty() {
            print!("Parents:");
            for parent in &commit.parents {
                print!(" {}", &parent.as_hex()[..8]);
            }
            println!();
        }
        println!("Date:   {}", formatted_time);
        println!();
        println!("    {}", commit.message);
        println!();

        count += 1;

        // Add parents to queue
        for parent in &commit.parents {
            queue.push_back(*parent);
        }
    }

    if count == 0 {
        println!("No commits found.");
    }

    Ok(())
}

/// Look up a file path in the index.
pub fn index_path(path: &str) -> Result<()> {
    let mut repo = CtxRepo::open(".").context("Not a CTX repository")?;

    let index = repo.index().context("Failed to load index")?;

    match index.lookup_path(path)? {
        Some(id) => {
            println!("Path: {}", path);
            println!("ObjectId: {}", id.as_hex());
        }
        None => {
            println!("Path not found in index: {}", path);
        }
    }

    Ok(())
}

/// Look up entities by name in the index.
pub fn index_name(namespace: &str, name: &str) -> Result<()> {
    let mut repo = CtxRepo::open(".").context("Not a CTX repository")?;

    let ns = parse_name_namespace(namespace)?;
    let index = repo.index().context("Failed to load index")?;

    let results = index.lookup_name(ns, name)?;

    if results.is_empty() {
        println!("No {} named '{}' found in index", namespace, name);
    } else {
        println!("Found {} {}(s) named '{}':", results.len(), namespace, name);
        for id in results {
            println!("  {}", id.as_hex());
        }
    }

    Ok(())
}

/// Show edges for a node in the index.
pub fn index_edges(kind: &str, id: &str, label: Option<&str>) -> Result<()> {
    use ctx_core::{EdgeLabel, NodeId};

    let mut repo = CtxRepo::open(".").context("Not a CTX repository")?;

    let node_kind = parse_node_kind(kind)?;
    let node = NodeId {
        kind: node_kind,
        id: id.to_string(),
    };

    let index = repo.index().context("Failed to load index")?;

    println!("Node: {:?} \"{}\"", node_kind, id);
    println!();

    let labels_to_check = if let Some(l) = label {
        vec![parse_edge_label(l)?]
    } else {
        // All labels
        vec![
            EdgeLabel::Contains,
            EdgeLabel::Defines,
            EdgeLabel::HasVersion,
            EdgeLabel::DependsOn,
            EdgeLabel::TargetOf,
            EdgeLabel::CrateFromTarget,
            EdgeLabel::Imports,
            EdgeLabel::References,
            EdgeLabel::Calls,
            EdgeLabel::Implements,
            EdgeLabel::UsesType,
            EdgeLabel::Mentions,
            EdgeLabel::UpdatedIn,
            EdgeLabel::DerivedFrom,
        ]
    };

    println!("Outgoing edges:");
    for lbl in &labels_to_check {
        let targets = index.get_edges_from(&node, *lbl)?;
        if !targets.is_empty() {
            println!("  {:?}:", lbl);
            for target in targets {
                println!("    -> {:?} \"{}\"", target.kind, target.id);
            }
        }
    }

    println!();
    println!("Incoming edges:");
    for lbl in &labels_to_check {
        let sources = index.get_edges_to(&node, *lbl)?;
        if !sources.is_empty() {
            println!("  {:?}:", lbl);
            for source in sources {
                println!("    <- {:?} \"{}\"", source.kind, source.id);
            }
        }
    }

    Ok(())
}

/// Show index statistics.
pub fn index_stats() -> Result<()> {
    use ctx_core::INDEX_SCHEMA_VERSION;

    let mut repo = CtxRepo::open(".").context("Not a CTX repository")?;

    let index = repo.index().context("Failed to load index")?;

    println!("Index path: {}", index.path().display());
    println!("Schema version: {}", INDEX_SCHEMA_VERSION);

    Ok(())
}

/// Parse a node kind from a string.
fn parse_node_kind(s: &str) -> Result<ctx_core::NodeKind> {
    use ctx_core::NodeKind;

    match s.to_lowercase().as_str() {
        "file" => Ok(NodeKind::File),
        "module" => Ok(NodeKind::Module),
        "item" => Ok(NodeKind::Item),
        "package" => Ok(NodeKind::Package),
        "target" => Ok(NodeKind::Target),
        "crate" => Ok(NodeKind::Crate),
        "task" => Ok(NodeKind::Task),
        "note" => Ok(NodeKind::Note),
        "decision" => Ok(NodeKind::Decision),
        "diagnostic" => Ok(NodeKind::Diagnostic),
        _ => anyhow::bail!("Unknown node kind: {}. Valid kinds: file, module, item, package, target, crate, task, note, decision, diagnostic", s),
    }
}

/// Parse an edge label from a string.
fn parse_edge_label(s: &str) -> Result<ctx_core::EdgeLabel> {
    use ctx_core::EdgeLabel;

    match s.to_lowercase().as_str() {
        "contains" => Ok(EdgeLabel::Contains),
        "defines" => Ok(EdgeLabel::Defines),
        "hasversion" => Ok(EdgeLabel::HasVersion),
        "dependson" => Ok(EdgeLabel::DependsOn),
        "targetof" => Ok(EdgeLabel::TargetOf),
        "cratefromtarget" => Ok(EdgeLabel::CrateFromTarget),
        "imports" => Ok(EdgeLabel::Imports),
        "references" => Ok(EdgeLabel::References),
        "calls" => Ok(EdgeLabel::Calls),
        "implements" => Ok(EdgeLabel::Implements),
        "usestype" => Ok(EdgeLabel::UsesType),
        "mentions" => Ok(EdgeLabel::Mentions),
        "updatedin" => Ok(EdgeLabel::UpdatedIn),
        "derivedfrom" => Ok(EdgeLabel::DerivedFrom),
        _ => anyhow::bail!("Unknown edge label: {}. Valid labels: contains, defines, hasversion, dependson, targetof, cratefromtarget, imports, references, calls, implements, usestype, mentions, updatedin, derivedfrom", s),
    }
}

/// Parse a name namespace from a string.
fn parse_name_namespace(s: &str) -> Result<ctx_core::NameNamespace> {
    use ctx_core::NameNamespace;

    match s.to_lowercase().as_str() {
        "package" => Ok(NameNamespace::Package),
        "module" => Ok(NameNamespace::Module),
        "item" => Ok(NameNamespace::Item),
        "task" => Ok(NameNamespace::Task),
        "note" => Ok(NameNamespace::Note),
        _ => anyhow::bail!(
            "Unknown namespace: {}. Valid namespaces: package, module, item, task, note",
            s
        ),
    }
}

/// Export graph for visualization.
pub fn graph(format: &str, _labels: Option<&str>, _max_nodes: usize) -> Result<()> {
    use ctx_core::{adjacency_to_dot, AdjacencyList};

    let mut repo = CtxRepo::open(".")?;

    // Get edge batch IDs from HEAD commit
    // Note: CommitInfo stores ObjectIds that reference EdgeBatch objects
    // in the object store to avoid data duplication in the index
    let edge_batch_ids = {
        let head_id = repo.head_id()?;
        let index = repo.index()?;
        let commit_info = index
            .get_commit_info(head_id)?
            .context("HEAD commit not in index")?;
        commit_info.edge_batches.clone()
    };

    // Load edge batches from object store
    let all_batches = repo.load_edge_batches(&edge_batch_ids)?;

    // Build adjacency list from edge batches
    let adjacency = AdjacencyList::from_edge_batches(all_batches.iter());

    // Export in requested format
    match format {
        "dot" => {
            let dot = adjacency_to_dot(&adjacency);
            println!("{}", dot);
        }
        "json" => {
            // TODO: Implement JSON export
            anyhow::bail!("JSON format not yet implemented. Use 'dot' format.");
        }
        _ => {
            anyhow::bail!("Unsupported format: {}. Use 'dot' or 'json'.", format);
        }
    }

    Ok(())
}

/// Show SCC analysis.
pub fn scc(show_members: bool) -> Result<()> {
    use ctx_core::{compute_scc, AdjacencyList};

    let mut repo = CtxRepo::open(".")?;

    // Get edge batch IDs from HEAD commit
    let edge_batch_ids = {
        let head_id = repo.head_id()?;
        let index = repo.index()?;
        let commit_info = index
            .get_commit_info(head_id)?
            .context("HEAD commit not in index")?;
        commit_info.edge_batches.clone()
    };

    // Load edge batches from object store
    let all_batches = repo.load_edge_batches(&edge_batch_ids)?;

    // Build adjacency list and compute SCCs
    let adjacency = AdjacencyList::from_edge_batches(all_batches.iter());
    let scc_view = compute_scc(&adjacency);

    println!("Strongly Connected Components: {}", scc_view.scc_count());
    println!();

    if show_members {
        for i in 0..scc_view.scc_count() {
            let scc_id = ctx_core::SccId(i as u32);
            let members = scc_view.members(scc_id);
            let deps = scc_view.dependencies(scc_id);

            println!("SCC {} ({} nodes):", i, members.len());
            for member in members {
                println!("  {:?}::{}", member.kind, member.id);
            }

            if !deps.is_empty() {
                println!("  Dependencies: {:?}", deps);
            }

            println!();
        }
    } else {
        // Just print summary
        for i in 0..scc_view.scc_count() {
            let scc_id = ctx_core::SccId(i as u32);
            let members = scc_view.members(scc_id);
            println!("SCC {}: {} nodes", i, members.len());
        }
    }

    Ok(())
}

/// Show parsed Cargo workspace structure.
pub fn cargo_show() -> Result<()> {
    use ctx_core::CargoMetadataSnapshot;

    let repo = ctx_core::CtxRepo::open(".")?;
    let head = repo.head()?;

    match head.cargo_snapshot {
        Some(snapshot_id) => {
            let snapshot: CargoMetadataSnapshot = repo.object_store().get_typed(snapshot_id)?;

            println!("Workspace root: {}", snapshot.workspace_root);
            println!("Packages ({}):", snapshot.packages.len());
            println!();

            for pkg in &snapshot.packages {
                println!("  {} v{}", pkg.name, pkg.version);
                println!("    Edition: {}", pkg.edition);
                println!("    Manifest: {}", pkg.manifest_path);

                if !pkg.targets.is_empty() {
                    println!("    Targets:");
                    for target in &pkg.targets {
                        println!("      {} ({:?})", target.name, target.kind);
                        println!("        Path: {}", target.src_path);
                    }
                }

                if !pkg.dependencies.is_empty() {
                    println!("    Dependencies ({}):", pkg.dependencies.len());
                    for dep in pkg.dependencies.iter().take(5) {
                        let kind = match dep.kind {
                            ctx_core::DepKind::Normal => "",
                            ctx_core::DepKind::Dev => " [dev]",
                            ctx_core::DepKind::Build => " [build]",
                        };
                        println!("      {} {}{}", dep.name, dep.req, kind);
                    }
                    if pkg.dependencies.len() > 5 {
                        println!("      ... and {} more", pkg.dependencies.len() - 5);
                    }
                }

                if !pkg.features.is_empty() {
                    println!("    Features: {}", pkg.features.len());
                }

                println!();
            }
        }
        None => {
            println!("No Cargo snapshot in HEAD. Run `ctx analyze cargo` first.");
        }
    }

    Ok(())
}

/// Show dependencies for a specific package.
pub fn cargo_deps(package_name: &str) -> Result<()> {
    use ctx_core::CargoMetadataSnapshot;

    let repo = ctx_core::CtxRepo::open(".")?;
    let head = repo.head()?;

    match head.cargo_snapshot {
        Some(snapshot_id) => {
            let snapshot: CargoMetadataSnapshot = repo.object_store().get_typed(snapshot_id)?;

            match snapshot.packages.iter().find(|p| p.name == package_name) {
                Some(pkg) => {
                    println!("Dependencies for {}:", pkg.name);
                    println!();

                    if pkg.dependencies.is_empty() {
                        println!("  No dependencies");
                    } else {
                        for dep in &pkg.dependencies {
                            let kind = match dep.kind {
                                ctx_core::DepKind::Normal => "",
                                ctx_core::DepKind::Dev => " [dev]",
                                ctx_core::DepKind::Build => " [build]",
                            };
                            let optional = if dep.optional { " (optional)" } else { "" };
                            let target = if let Some(t) = &dep.target {
                                format!(" (target: {})", t)
                            } else {
                                String::new()
                            };

                            println!("  {} {}{}{}{}", dep.name, dep.req, kind, optional, target);

                            if !dep.features.is_empty() {
                                println!("    Features: {}", dep.features.join(", "));
                            }
                        }
                    }
                }
                None => {
                    println!("Package '{}' not found in workspace.", package_name);
                    println!();
                    println!("Available packages:");
                    for pkg in &snapshot.packages {
                        println!("  {}", pkg.name);
                    }
                }
            }
        }
        None => {
            println!("No Cargo snapshot in HEAD. Run `ctx analyze cargo` first.");
        }
    }

    Ok(())
}
