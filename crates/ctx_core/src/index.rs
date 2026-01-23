//! Rebuildable index for fast lookups.
//!
//! The index system provides fast lookups for paths, names, commits, and graph adjacency.
//! All indexes are stored in a redb database and can be rebuilt from the object store.

#![allow(clippy::io_other_error)]

use crate::error::{CtxError, Result};
use crate::types::{Commit, EdgeBatch, EdgeLabel, NarrativeRef, NodeId, Tree, TreeEntryKind};
use crate::{ObjectId, ObjectStore};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Index schema version for migration support.
pub const INDEX_SCHEMA_VERSION: u32 = 1;

// Table definitions
const METADATA_TABLE: TableDefinition<&str, u32> = TableDefinition::new("metadata");
const PATH_TO_ID_TABLE: TableDefinition<&str, &[u8; 32]> = TableDefinition::new("path_to_id");
const NAME_TO_IDS_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("name_to_ids");
const COMMIT_INFO_TABLE: TableDefinition<&[u8; 32], &[u8]> = TableDefinition::new("commit_info");
const ADJACENCY_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("adjacency");

/// Cached commit information for fast lookup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitInfo {
    /// Root tree snapshot.
    pub root_tree: ObjectId,
    /// Edge batches in this commit.
    pub edge_batches: Vec<ObjectId>,
    /// Narrative references.
    pub narrative_refs: Vec<NarrativeRef>,
    /// Cargo.toml snapshot.
    pub cargo_snapshot: Option<ObjectId>,
    /// Rust file snapshots.
    pub rust_snapshot: Option<ObjectId>,
    /// Diagnostics snapshot.
    pub diagnostics_snapshot: Option<ObjectId>,
}

impl CommitInfo {
    /// Create CommitInfo from a Commit.
    pub fn from_commit(commit: &Commit) -> Self {
        Self {
            root_tree: commit.root_tree,
            edge_batches: commit.edge_batches.clone(),
            narrative_refs: commit.narrative_refs.clone(),
            cargo_snapshot: commit.cargo_snapshot,
            rust_snapshot: commit.rust_snapshot,
            diagnostics_snapshot: commit.diagnostics_snapshot,
        }
    }
}

/// Direction for adjacency queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EdgeDirection {
    /// Outgoing edges (from -> to).
    Outgoing = 0,
    /// Incoming edges (to -> from).
    Incoming = 1,
}

/// Namespace for name lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NameNamespace {
    /// Package names.
    Package = 0,
    /// Module names.
    Module = 1,
    /// Item names (functions, structs, etc.).
    Item = 2,
    /// Task names.
    Task = 3,
    /// Note names.
    Note = 4,
}

/// Encode adjacency key: node_kind + node_id_len + node_id + direction + label.
fn encode_adjacency_key(node: &NodeId, direction: EdgeDirection, label: EdgeLabel) -> Vec<u8> {
    let id_bytes = node.id.as_bytes();
    let mut key = Vec::with_capacity(1 + 2 + id_bytes.len() + 1 + 1);
    key.push(node.kind as u8);
    key.extend_from_slice(&(id_bytes.len() as u16).to_le_bytes());
    key.extend_from_slice(id_bytes);
    key.push(direction as u8);
    key.push(label as u8);
    key
}

/// Encode name index key: namespace_byte + name_utf8.
fn encode_name_key(namespace: NameNamespace, name: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + name.len());
    key.push(namespace as u8);
    key.extend_from_slice(name.as_bytes());
    key
}

/// Extract the simple name from a potentially qualified node ID.
/// Examples: "std::collections::HashMap" → "HashMap", "main" → "main"
fn extract_simple_name(node_id: &str) -> &str {
    node_id.split("::").last().unwrap_or(node_id)
}

/// Map NodeKind to NameNamespace for indexing.
fn node_kind_to_namespace(kind: crate::types::NodeKind) -> Option<NameNamespace> {
    use crate::types::NodeKind;
    match kind {
        NodeKind::Package => Some(NameNamespace::Package),
        NodeKind::Module => Some(NameNamespace::Module),
        NodeKind::Item => Some(NameNamespace::Item),
        NodeKind::Task => Some(NameNamespace::Task),
        NodeKind::Note => Some(NameNamespace::Note),
        _ => None, // File, Target, Crate, Decision, Diagnostic not indexed by name
    }
}

/// Populate name index for a single node.
/// Uses the blob_id from evidence if available, otherwise the commit_id.
fn populate_name_index_for_node(
    node: &NodeId,
    evidence: &crate::types::Evidence,
    name_index: &mut BTreeMap<Vec<u8>, BTreeSet<ObjectId>>,
) {
    // Only index certain node kinds
    if let Some(namespace) = node_kind_to_namespace(node.kind) {
        let simple_name = extract_simple_name(&node.id);

        // Use blob_id if available (file containing the definition),
        // otherwise use commit_id (commit where edge was introduced)
        let object_id = evidence.blob_id.unwrap_or(evidence.commit_id);

        let key = encode_name_key(namespace, simple_name);
        name_index.entry(key).or_default().insert(object_id);
    }
}

/// Rebuildable index for fast lookups.
///
/// The index is stored in `.ctx/index/index.redb` and can be deleted and
/// rebuilt from the object store at any time.
pub struct Index {
    db: Database,
    path: PathBuf,
}

impl Index {
    /// Opens an existing index database.
    ///
    /// Returns `None` if the index doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database exists but can't be opened or has a schema version mismatch.
    pub fn open(path: impl AsRef<Path>) -> Result<Option<Self>> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Ok(None);
        }

        let db = Database::open(&path).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to open index: {}", e),
            ))
        })?;

        // Verify schema version
        let read_txn = db.begin_read().map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to begin read transaction: {}", e),
            ))
        })?;

        if let Ok(table) = read_txn.open_table(METADATA_TABLE) {
            if let Some(version) = table.get("version").ok().flatten() {
                let version_val = version.value();
                if version_val != INDEX_SCHEMA_VERSION {
                    return Err(CtxError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "Index schema version mismatch: found {}, expected {}",
                            version_val, INDEX_SCHEMA_VERSION
                        ),
                    )));
                }
            }
        }

        Ok(Some(Self { db, path }))
    }

    /// Creates a new index database.
    ///
    /// Overwrites any existing database at the path.
    ///
    /// # Errors
    ///
    /// Returns an error if the database can't be created or initialized.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Remove existing database
        if path.exists() {
            std::fs::remove_file(&path)?;
        }

        let db = Database::create(&path).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create index: {}", e),
            ))
        })?;

        // Initialize schema version
        let write_txn = db.begin_write().map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to begin write transaction: {}", e),
            ))
        })?;

        {
            let mut table = write_txn.open_table(METADATA_TABLE).map_err(|e| {
                CtxError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to open metadata table: {}", e),
                ))
            })?;
            table.insert("version", INDEX_SCHEMA_VERSION).map_err(|e| {
                CtxError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to insert version: {}", e),
                ))
            })?;
        }

        write_txn.commit().map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to commit: {}", e),
            ))
        })?;

        Ok(Self { db, path })
    }

    /// Returns the path to the index database.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Adds or updates a file path → blob mapping in the index.
    ///
    /// This is used to manually index files that were analyzed but not yet
    /// part of the commit tree.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ctx_core::{CtxRepo, ObjectId};
    ///
    /// # fn main() -> ctx_core::Result<()> {
    /// let mut repo = CtxRepo::open(".")?;
    /// let blob_id = repo.object_store().put_blob(b"content")?;
    /// repo.index_mut()?.index_file_path("src/lib.rs", blob_id)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn index_file_path(&mut self, path: &str, blob_id: ObjectId) -> Result<()> {
        let write_txn = self.begin_write()?;

        {
            let mut table = write_txn.open_table(PATH_TO_ID_TABLE).map_err(|e| {
                CtxError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to open path table: {}", e),
                ))
            })?;

            table.insert(path, blob_id.as_bytes()).map_err(|e| {
                CtxError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to insert path: {}", e),
                ))
            })?;
        }

        write_txn.commit().map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to commit transaction: {}", e),
            ))
        })?;

        Ok(())
    }

    /// Batch index multiple file paths in a single transaction.
    /// This is more efficient than calling index_file_path() repeatedly.
    pub fn index_file_paths(&mut self, paths: &[(String, ObjectId)]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }

        let write_txn = self.begin_write()?;

        {
            let mut table = write_txn.open_table(PATH_TO_ID_TABLE).map_err(|e| {
                CtxError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to open path table: {}", e),
                ))
            })?;

            for (path, blob_id) in paths {
                table.insert(path.as_str(), blob_id.as_bytes()).map_err(|e| {
                    CtxError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Failed to insert path {}: {}", path, e),
                    ))
                })?;
            }
        }

        write_txn.commit().map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to commit transaction: {}", e),
            ))
        })?;

        Ok(())
    }

    /// Look up a path to get its ObjectId.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ctx_core::CtxRepo;
    ///
    /// # fn main() -> ctx_core::Result<()> {
    /// let mut repo = CtxRepo::open(".")?;
    /// let index = repo.index()?;
    ///
    /// if let Some(id) = index.lookup_path("src/main.rs")? {
    ///     println!("Found file with ID: {}", id);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the index can't be queried.
    pub fn lookup_path(&self, path: &str) -> Result<Option<ObjectId>> {
        let read_txn = self.begin_read()?;
        let table = read_txn.open_table(PATH_TO_ID_TABLE).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to open path table: {}", e),
            ))
        })?;

        match table.get(path).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get path: {}", e),
            ))
        })? {
            Some(bytes) => Ok(Some(ObjectId::from_bytes(*bytes.value()))),
            None => Ok(None),
        }
    }

    /// Look up entities by name within a namespace.
    ///
    /// Returns all entities with the given name in the specified namespace.
    /// For example, looking up a function name in the `RustFunction` namespace
    /// will return all functions with that name across the codebase.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ctx_core::{CtxRepo, NameNamespace};
    ///
    /// # fn main() -> ctx_core::Result<()> {
    /// let mut repo = CtxRepo::open(".")?;
    /// let index = repo.index()?;
    ///
    /// // Find all items (functions, structs, etc.) named "handle_request"
    /// let ids = index.lookup_name(NameNamespace::Item, "handle_request")?;
    /// println!("Found {} items named 'handle_request'", ids.len());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the index can't be queried.
    pub fn lookup_name(&self, namespace: NameNamespace, name: &str) -> Result<Vec<ObjectId>> {
        let key = encode_name_key(namespace, name);
        let read_txn = self.begin_read()?;
        let table = read_txn.open_table(NAME_TO_IDS_TABLE).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to open name table: {}", e),
            ))
        })?;

        match table.get(key.as_slice()).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get name: {}", e),
            ))
        })? {
            Some(bytes) => {
                let ids: Vec<[u8; 32]> = postcard::from_bytes(bytes.value())
                    .map_err(|e| CtxError::Deserialization(e.to_string()))?;
                Ok(ids.into_iter().map(ObjectId::from_bytes).collect())
            }
            None => Ok(vec![]),
        }
    }

    /// Get cached commit info.
    ///
    /// # Errors
    ///
    /// Returns an error if the index can't be queried.
    pub fn get_commit_info(&self, commit_id: ObjectId) -> Result<Option<CommitInfo>> {
        let read_txn = self.begin_read()?;
        let table = read_txn.open_table(COMMIT_INFO_TABLE).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to open commit table: {}", e),
            ))
        })?;

        match table.get(commit_id.as_bytes()).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get commit: {}", e),
            ))
        })? {
            Some(bytes) => {
                let info: CommitInfo = postcard::from_bytes(bytes.value())
                    .map_err(|e| CtxError::Deserialization(e.to_string()))?;
                Ok(Some(info))
            }
            None => Ok(None),
        }
    }

    /// Get adjacent nodes for a given node, direction, and label.
    ///
    /// # Errors
    ///
    /// Returns an error if the index can't be queried.
    pub fn get_adjacent(
        &self,
        node: &NodeId,
        direction: EdgeDirection,
        label: EdgeLabel,
    ) -> Result<Vec<NodeId>> {
        let key = encode_adjacency_key(node, direction, label);
        let read_txn = self.begin_read()?;
        let table = read_txn.open_table(ADJACENCY_TABLE).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to open adjacency table: {}", e),
            ))
        })?;

        match table.get(key.as_slice()).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get adjacency: {}", e),
            ))
        })? {
            Some(bytes) => {
                let nodes: Vec<NodeId> = postcard::from_bytes(bytes.value())
                    .map_err(|e| CtxError::Deserialization(e.to_string()))?;
                Ok(nodes)
            }
            None => Ok(vec![]),
        }
    }

    /// Get all outgoing edges from a node with the given label.
    ///
    /// # Errors
    ///
    /// Returns an error if the index can't be queried.
    pub fn get_edges_from(&self, node: &NodeId, label: EdgeLabel) -> Result<Vec<NodeId>> {
        self.get_adjacent(node, EdgeDirection::Outgoing, label)
    }

    /// Get all incoming edges to a node with the given label.
    ///
    /// # Errors
    ///
    /// Returns an error if the index can't be queried.
    pub fn get_edges_to(&self, node: &NodeId, label: EdgeLabel) -> Result<Vec<NodeId>> {
        self.get_adjacent(node, EdgeDirection::Incoming, label)
    }

    /// Rebuild the entire index from the object store.
    ///
    /// This walks the commit DAG from HEAD and indexes:
    /// - File paths from the HEAD tree
    /// - Edges from all edge batches
    /// - Commit metadata
    ///
    /// # Errors
    ///
    /// Returns an error if the index can't be created or rebuilt.
    pub fn rebuild_from_objects(
        path: impl AsRef<Path>,
        object_store: &ObjectStore,
        head_id: ObjectId,
    ) -> Result<Self> {
        // First, preserve any existing file path mappings before rebuilding
        let preserved_paths: Vec<(String, ObjectId)> = if path.as_ref().exists() {
            match Self::open(&path)? {
                Some(existing_index) => {
                    let mut paths = Vec::new();
                    if let Ok(read_txn) = existing_index.begin_read() {
                        if let Ok(table) = read_txn.open_table(PATH_TO_ID_TABLE) {
                            for result in table.iter().map_err(|e| {
                                CtxError::Io(std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    format!("Failed to iterate paths: {}", e),
                                ))
                            })? {
                                if let Ok((key, value)) = result {
                                    let path_str: &str = key.value();
                                    let obj_id = ObjectId::from_bytes(*value.value());
                                    paths.push((path_str.to_string(), obj_id));
                                }
                            }
                        }
                    }
                    paths
                }
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };

        // Create fresh index
        let index = Self::create(path)?;

        // Collect all data in memory first
        let mut path_index: BTreeMap<String, ObjectId> = BTreeMap::new();
        let mut name_index: BTreeMap<Vec<u8>, BTreeSet<ObjectId>> = BTreeMap::new();
        let mut commit_cache: BTreeMap<ObjectId, CommitInfo> = BTreeMap::new();
        let mut adjacency: BTreeMap<Vec<u8>, BTreeSet<NodeId>> = BTreeMap::new();

        // Walk commit DAG using BFS
        let mut queue = VecDeque::new();
        let mut seen_commits = HashSet::new();
        queue.push_back(head_id);

        while let Some(commit_id) = queue.pop_front() {
            if !seen_commits.insert(commit_id) {
                continue;
            }

            let commit: Commit = object_store.get_typed(commit_id)?;

            // Cache commit info
            commit_cache.insert(commit_id, CommitInfo::from_commit(&commit));

            // Index tree paths (only for HEAD to avoid stale paths)
            if commit_id == head_id {
                index_tree_paths(
                    object_store,
                    commit.root_tree,
                    String::new(),
                    &mut path_index,
                )?;
            }

            // Index edges from all edge batches
            for batch_id in &commit.edge_batches {
                let batch: EdgeBatch = object_store.get_typed(*batch_id)?;

                for edge in &batch.edges {
                    // Build adjacency: outgoing
                    let out_key =
                        encode_adjacency_key(&edge.from, EdgeDirection::Outgoing, edge.label);
                    adjacency
                        .entry(out_key)
                        .or_default()
                        .insert(edge.to.clone());

                    // Build adjacency: incoming
                    let in_key =
                        encode_adjacency_key(&edge.to, EdgeDirection::Incoming, edge.label);
                    adjacency
                        .entry(in_key)
                        .or_default()
                        .insert(edge.from.clone());

                    // Build name index for both from and to nodes
                    populate_name_index_for_node(&edge.from, &edge.evidence, &mut name_index);
                    populate_name_index_for_node(&edge.to, &edge.evidence, &mut name_index);
                }
            }

            // Queue parent commits
            for parent_id in &commit.parents {
                queue.push_back(*parent_id);
            }
        }

        // Add preserved file path mappings back into the index
        for (path, obj_id) in preserved_paths {
            path_index.insert(path, obj_id);
        }

        // Write all collected data in a single transaction
        index.write_batch(&path_index, &name_index, &commit_cache, &adjacency)?;

        Ok(index)
    }

    /// Write all index data in a single transaction.
    fn write_batch(
        &self,
        paths: &BTreeMap<String, ObjectId>,
        names: &BTreeMap<Vec<u8>, BTreeSet<ObjectId>>,
        commits: &BTreeMap<ObjectId, CommitInfo>,
        adjacency: &BTreeMap<Vec<u8>, BTreeSet<NodeId>>,
    ) -> Result<()> {
        let write_txn = self.db.begin_write().map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to begin write transaction: {}", e),
            ))
        })?;

        // Write path index
        {
            let mut table = write_txn.open_table(PATH_TO_ID_TABLE).map_err(|e| {
                CtxError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to open path table: {}", e),
                ))
            })?;
            for (path, id) in paths {
                table.insert(path.as_str(), id.as_bytes()).map_err(|e| {
                    CtxError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Failed to insert path: {}", e),
                    ))
                })?;
            }
        }

        // Write name index
        {
            let mut table = write_txn.open_table(NAME_TO_IDS_TABLE).map_err(|e| {
                CtxError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to open name table: {}", e),
                ))
            })?;
            for (key, ids) in names {
                let ids_bytes: Vec<[u8; 32]> = ids.iter().map(|id| *id.as_bytes()).collect();
                let value = postcard::to_allocvec(&ids_bytes)
                    .map_err(|e| CtxError::Serialization(e.to_string()))?;
                table
                    .insert(key.as_slice(), value.as_slice())
                    .map_err(|e| {
                        CtxError::Io(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to insert name: {}", e),
                        ))
                    })?;
            }
        }

        // Write commit info cache
        {
            let mut table = write_txn.open_table(COMMIT_INFO_TABLE).map_err(|e| {
                CtxError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to open commit table: {}", e),
                ))
            })?;
            for (commit_id, info) in commits {
                let value = postcard::to_allocvec(info)
                    .map_err(|e| CtxError::Serialization(e.to_string()))?;
                table
                    .insert(commit_id.as_bytes(), value.as_slice())
                    .map_err(|e| {
                        CtxError::Io(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to insert commit: {}", e),
                        ))
                    })?;
            }
        }

        // Write adjacency index
        {
            let mut table = write_txn.open_table(ADJACENCY_TABLE).map_err(|e| {
                CtxError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to open adjacency table: {}", e),
                ))
            })?;
            for (key, nodes) in adjacency {
                let nodes_vec: Vec<NodeId> = nodes.iter().cloned().collect();
                let value = postcard::to_allocvec(&nodes_vec)
                    .map_err(|e| CtxError::Serialization(e.to_string()))?;
                table
                    .insert(key.as_slice(), value.as_slice())
                    .map_err(|e| {
                        CtxError::Io(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to insert adjacency: {}", e),
                        ))
                    })?;
            }
        }

        write_txn.commit().map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to commit: {}", e),
            ))
        })?;
        Ok(())
    }

    /// Helper to begin a read transaction.
    fn begin_read(&self) -> Result<redb::ReadTransaction> {
        self.db.begin_read().map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to begin read transaction: {}", e),
            ))
        })
    }

    fn begin_write(&self) -> Result<redb::WriteTransaction> {
        self.db.begin_write().map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to begin write transaction: {}", e),
            ))
        })
    }
}

/// Recursively walk a tree and collect all paths.
fn index_tree_paths(
    store: &ObjectStore,
    tree_id: ObjectId,
    prefix: String,
    paths: &mut BTreeMap<String, ObjectId>,
) -> Result<()> {
    let tree: Tree = store.get_typed(tree_id)?;

    for entry in &tree.entries {
        let full_path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };

        match entry.kind {
            TreeEntryKind::Blob => {
                paths.insert(full_path, entry.id);
            }
            TreeEntryKind::Tree => {
                paths.insert(full_path.clone(), entry.id);
                index_tree_paths(store, entry.id, full_path, paths)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeKind;
    use tempfile::TempDir;

    #[test]
    fn test_create_and_open() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("index.redb");

        // Create
        let idx = Index::create(&path).unwrap();
        drop(idx);

        // Open
        let idx2 = Index::open(&path).unwrap();
        assert!(idx2.is_some());
    }

    #[test]
    fn test_open_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nonexistent.redb");

        let result = Index::open(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_path_index_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let idx = Index::create(tmp.path().join("index.redb")).unwrap();

        let test_id = ObjectId::from_bytes([42u8; 32]);

        // Insert via batch write
        let mut paths = BTreeMap::new();
        paths.insert("src/main.rs".to_string(), test_id);
        idx.write_batch(&paths, &BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new())
            .unwrap();

        // Query
        let result = idx.lookup_path("src/main.rs").unwrap();
        assert_eq!(result, Some(test_id));

        // Not found
        let missing = idx.lookup_path("not/found.rs").unwrap();
        assert_eq!(missing, None);
    }

    #[test]
    fn test_adjacency_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let idx = Index::create(tmp.path().join("index.redb")).unwrap();

        let from_node = NodeId {
            kind: NodeKind::File,
            id: "main.rs".to_string(),
        };
        let to_node = NodeId {
            kind: NodeKind::Item,
            id: "main".to_string(),
        };

        // Build adjacency
        let mut adjacency = BTreeMap::new();
        let out_key = encode_adjacency_key(&from_node, EdgeDirection::Outgoing, EdgeLabel::Defines);
        adjacency
            .entry(out_key)
            .or_insert_with(BTreeSet::new)
            .insert(to_node.clone());

        let in_key = encode_adjacency_key(&to_node, EdgeDirection::Incoming, EdgeLabel::Defines);
        adjacency
            .entry(in_key)
            .or_insert_with(BTreeSet::new)
            .insert(from_node.clone());

        idx.write_batch(
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &adjacency,
        )
        .unwrap();

        // Query outgoing
        let targets = idx.get_edges_from(&from_node, EdgeLabel::Defines).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0], to_node);

        // Query incoming
        let sources = idx.get_edges_to(&to_node, EdgeLabel::Defines).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0], from_node);
    }

    #[test]
    fn test_commit_info_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let idx = Index::create(tmp.path().join("index.redb")).unwrap();

        let commit_id = ObjectId::from_bytes([1u8; 32]);
        let info = CommitInfo {
            root_tree: ObjectId::from_bytes([2u8; 32]),
            edge_batches: vec![ObjectId::from_bytes([3u8; 32])],
            narrative_refs: vec![],
            cargo_snapshot: None,
            rust_snapshot: None,
            diagnostics_snapshot: None,
        };

        let mut commits = BTreeMap::new();
        commits.insert(commit_id, info.clone());

        idx.write_batch(
            &BTreeMap::new(),
            &BTreeMap::new(),
            &commits,
            &BTreeMap::new(),
        )
        .unwrap();

        let result = idx.get_commit_info(commit_id).unwrap();
        assert_eq!(result, Some(info));
    }

    #[test]
    fn test_schema_version() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("index.redb");

        // Create with current version
        Index::create(&path).unwrap();

        // Manually modify version (simulate old schema)
        let db = redb::Database::open(&path).unwrap();
        let txn = db.begin_write().unwrap();
        {
            let mut table = txn.open_table(METADATA_TABLE).unwrap();
            table.insert("version", 0u32).unwrap();
        }
        txn.commit().unwrap();
        drop(db);

        // Should fail to open
        let result = Index::open(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_name_index_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let idx = Index::create(tmp.path().join("index.redb")).unwrap();

        let commit_id = ObjectId::from_bytes([1u8; 32]);
        let blob_id = ObjectId::from_bytes([2u8; 32]);

        // Build name index with multiple entries for the same name
        let mut name_index = BTreeMap::new();
        let key1 = encode_name_key(NameNamespace::Item, "HashMap");
        let key2 = encode_name_key(NameNamespace::Module, "collections");

        let mut ids1 = BTreeSet::new();
        ids1.insert(blob_id);
        ids1.insert(commit_id);
        name_index.insert(key1, ids1);

        let mut ids2 = BTreeSet::new();
        ids2.insert(commit_id);
        name_index.insert(key2, ids2);

        idx.write_batch(
            &BTreeMap::new(),
            &name_index,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();

        // Query by name
        let results = idx.lookup_name(NameNamespace::Item, "HashMap").unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.contains(&blob_id));
        assert!(results.contains(&commit_id));

        let results2 = idx
            .lookup_name(NameNamespace::Module, "collections")
            .unwrap();
        assert_eq!(results2.len(), 1);
        assert_eq!(results2[0], commit_id);

        // Not found
        let missing = idx.lookup_name(NameNamespace::Item, "NotFound").unwrap();
        assert_eq!(missing.len(), 0);
    }

    #[test]
    fn test_rebuild_populates_name_index() {
        use crate::types::{
            Confidence, Edge, EdgeBatch, EdgeLabel, Evidence, EvidenceTool, NodeKind,
        };

        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        let blob_id = ObjectId::from_bytes([42u8; 32]);
        let commit_id = ObjectId::from_bytes([1u8; 32]);

        // Create an edge batch with named nodes
        let edge_batch = EdgeBatch {
            edges: vec![
                Edge {
                    from: NodeId {
                        kind: NodeKind::Module,
                        id: "std::collections".to_string(),
                    },
                    to: NodeId {
                        kind: NodeKind::Item,
                        id: "std::collections::HashMap".to_string(),
                    },
                    label: EdgeLabel::Defines,
                    weight: None,
                    evidence: Evidence {
                        commit_id,
                        tool: EvidenceTool::Parser,
                        confidence: Confidence::High,
                        span: None,
                        blob_id: Some(blob_id),
                    },
                },
                Edge {
                    from: NodeId {
                        kind: NodeKind::Item,
                        id: "std::collections::HashMap".to_string(),
                    },
                    to: NodeId {
                        kind: NodeKind::Item,
                        id: "std::collections::HashMap::new".to_string(),
                    },
                    label: EdgeLabel::Defines,
                    weight: None,
                    evidence: Evidence {
                        commit_id,
                        tool: EvidenceTool::Parser,
                        confidence: Confidence::High,
                        span: None,
                        blob_id: Some(blob_id),
                    },
                },
            ],
            created_at: 1234567890,
        };

        let batch_id = store.put_typed(&edge_batch).unwrap();

        // Create a commit with the edge batch
        let empty_tree = Tree::new(vec![]);
        let tree_id = store.put_typed(&empty_tree).unwrap();

        let commit = Commit {
            parents: vec![],
            timestamp_unix: 1234567890,
            message: "Test commit".to_string(),
            root_tree: tree_id,
            edge_batches: vec![batch_id],
            narrative_refs: vec![],
            cargo_snapshot: None,
            rust_snapshot: None,
            diagnostics_snapshot: None,
            commit_type: None,
        };

        let commit_obj_id = store.put_typed(&commit).unwrap();

        // Rebuild index
        let index_path = tmp.path().join("index.redb");
        let index = Index::rebuild_from_objects(&index_path, &store, commit_obj_id).unwrap();

        // Verify name index was populated
        let collections_results = index
            .lookup_name(NameNamespace::Module, "collections")
            .unwrap();
        assert!(
            !collections_results.is_empty(),
            "Should find 'collections' module"
        );
        assert!(collections_results.contains(&blob_id));

        let hashmap_results = index.lookup_name(NameNamespace::Item, "HashMap").unwrap();
        assert!(!hashmap_results.is_empty(), "Should find 'HashMap' item");
        assert!(hashmap_results.contains(&blob_id));

        let new_results = index.lookup_name(NameNamespace::Item, "new").unwrap();
        assert!(!new_results.is_empty(), "Should find 'new' item");
        assert!(new_results.contains(&blob_id));
    }
}
