# CTX: Context Management System for Coding Agents

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Problem Statement](#2-problem-statement)
3. [Design Principles](#3-design-principles)
4. [System Architecture](#4-system-architecture)
5. [On-Disk Layout](#5-on-disk-layout)
6. [Content-Addressed Object Store](#6-content-addressed-object-store)
7. [Data Models](#7-data-models)
8. [Narrative System](#8-narrative-system)
9. [Relationship Graph](#9-relationship-graph)
10. [Indexing Strategy](#10-indexing-strategy)
11. [Session and Staging Management](#11-session-and-staging-management)
12. [Prompt Pack Compilation](#12-prompt-pack-compilation)
13. [CLI Interface](#13-cli-interface)
14. [LLM Integration](#14-llm-integration)
15. [Compaction and Garbage Collection](#15-compaction-and-garbage-collection)
16. [Ingestion Policy](#16-ingestion-policy)
17. [Implementation Roadmap](#17-implementation-roadmap)
18. [Appendix](#appendix)

---

## 1. Executive Summary

CTX is a context management system providing coding agents with durable, queryable memory across sessions. It combines Git-like immutable object storage with a semantic relationship graph for intelligent retrieval of code context, decisions, and narrative documentation.

### Core Value Proposition

- **Durable Memory**: Agents can persist and recall decisions, code relationships, and reasoning across sessions
- **Intelligent Retrieval**: Graph-based context expansion surfaces relevant code without loading entire repositories
- **Human-Readable Layer**: Markdown narrative documents remain editable and diffable by humans
- **Git-Friendly**: Immutable objects and tiny pointers merge cleanly without conflicts
- **LLM-Agnostic**: Works with any inference backend (llama.cpp, OpenAI, Anthropic, etc.)

### Target Use Cases

1. Long-running coding sessions where context exceeds model token limits
2. Multi-session projects requiring continuity of decisions and reasoning
3. Repository understanding and navigation for unfamiliar codebases
4. Debugging workflows requiring historical context about past failures
5. Team collaboration where agent knowledge should persist

---

## 2. Problem Statement

### Current State

Coding agents operate with limited context windows and no persistent memory. Each session starts fresh, forcing users to re-explain context and re-discover code relationships and decisions.

### Specific Problems

| Problem | Impact |
|---------|--------|
| **Context amnesia** | Agents forget decisions made in previous sessions |
| **Retrieval blindness** | Without relationships, agents cannot find relevant code efficiently |
| **Decision rot** | Rationale for past choices is lost, leading to conflicting implementations |
| **Diagnostic amnesia** | Agents repeat the same mistakes because they forget past failures |
| **Human opacity** | Binary databases prevent human inspection and debugging |

### Success Criteria

1. An agent can recall specific decisions made 10+ sessions ago
2. Retrieval returns relevant files with >80% precision for typical coding queries
3. Human can read and edit narrative documents with standard text tools
4. Git merges of `.ctx/` directory succeed without manual intervention in >95% of cases
5. Cold start retrieval completes in <500ms for repositories with 10,000 files

---

## 3. Design Principles

### 3.1 Immutable Objects, Tiny Pointers

All durable data is stored as immutable, content-addressed objects. State changes occur by writing new objects and updating tiny pointer files, enabling atomic transitions and trivial merge conflict resolution.

### 3.2 Markdown as Canonical Narrative

Human-readable documentation lives in Markdown files that humans can edit with any text editor. The system snapshots these documents into blobs for historical retrieval but never requires users to interact with binary formats for narrative content.

### 3.3 Rebuildable Indexes

All fast indexes are derived from immutable objects and can be deleted and rebuilt at any time. This means indexes are never tracked in version control and cannot cause merge conflicts.

### 3.4 Graph-Native Retrieval

Relationships between code entities are first-class objects. Retrieval works by seeding with query-relevant nodes and expanding through the relationship graph, not by keyword search alone.

### 3.5 Evidence for Every Edge

Every relationship edge must have provenance: which tool created it, which commit introduced it, and which span or blob supports it. Edges without evidence are treated as hypotheses and stored in narrative, not as hard graph edges.

### 3.6 Value-Based Ingestion

The system does not record everything. It ingests data that changes future agent behavior: constraints, decisions, evidence, and relationships. Ephemeral intermediate outputs are discarded.

### 3.7 Crash Safety

At every step, the system maintains durability invariants. A crash at any point should leave the system in a recoverable state with no data loss for committed work.

---

## 4. System Architecture

### 4.1 High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        Agent Host Process                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐       │
│  │   Agent      │    │   CTX Core   │    │  LLM Client  │       │
│  │   Logic      │◄──►│   Library    │◄──►│  (llama.cpp) │       │
│  └──────────────┘    └──────────────┘    └──────────────┘       │
│                             │                                    │
│                             ▼                                    │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │                    CTX API Contract                       │   │
│  │  • observe_file_read(path)                               │   │
│  │  • observe_file_write(path, content)                     │   │
│  │  • observe_command(cmd, output, exit_code)               │   │
│  │  • observe_note(text)                                    │   │
│  │  • observe_relations(edges)                              │   │
│  │  • build_pack(query, budget) -> PromptPack               │   │
│  │  • flush_step()                                          │   │
│  │  • compact_session()                                     │   │
│  │  • rebuild_index()                                       │   │
│  └──────────────────────────────────────────────────────────┘   │
│                             │                                    │
└─────────────────────────────┼────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                         .ctx/ Directory                          │
├──────────────┬──────────────┬──────────────┬────────────────────┤
│   objects/   │  narrative/  │    index/    │   refs/ + HEAD     │
│  (immutable) │  (markdown)  │ (rebuildable)│   (tiny pointers)  │
└──────────────┴──────────────┴──────────────┴────────────────────┘
```

### 4.2 Separation of Concerns

| Component | Responsibility | Mutability |
|-----------|----------------|------------|
| **Object Store** | Persist immutable content-addressed objects | Write-once |
| **Narrative** | Human-readable documentation | Human-editable |
| **Index** | Fast lookups and adjacency lists | Rebuildable |
| **Refs** | Current state pointers | Atomic update |
| **Graph** | Relationship traversal and SCC computation | Derived |
| **Pack Builder** | Compile retrieval results into prompts | Stateless |

### 4.3 Integration Patterns

**Pattern A: In-Process Library (Recommended for POC)**
- CTX Core is a Rust library linked into the agent host
- Lowest latency, simplest deployment
- Single process, single lock

**Pattern B: Sidecar Daemon**
- CTX runs as a separate process with local RPC
- Better isolation, can be shared across agents
- Adds IPC complexity

**Pattern C: CLI Bridge**
- Agent shells out to `ctx` commands
- Simplest integration, easiest debugging
- Higher latency per operation

---

## 5. On-Disk Layout

### 5.1 Directory Structure

```
.ctx/
├── config.toml                    # Repository configuration
├── HEAD                           # Pointer to current canonical commit
├── STAGE                          # Pointer to current staging head (optional)
├── LOCK                           # Process lock file
│
├── refs/
│   ├── main                       # Canonical commit pointer
│   └── work                       # Staging head pointer (alternative to STAGE)
│
├── objects/
│   ├── ab/
│   │   └── cd1234...              # Content-addressed object files
│   ├── ef/
│   │   └── 567890...
│   └── ...
│
├── narrative/
│   ├── README.md                  # Persistent overview
│   ├── decisions.md               # Architectural decisions
│   ├── log/
│   │   ├── 2026-01-19.md          # Daily journal entries
│   │   └── 2026-01-20.md
│   ├── tasks/
│   │   ├── task_0001.md           # Individual task files
│   │   └── task_0002.md
│   └── work/                      # Session work logs (optional)
│       └── <session_id>/
│           └── notes.md
│
├── index/                         # Rebuildable, gitignored
│   ├── index.redb                 # Key-value index
│   └── tantivy/                   # Full-text search (optional)
│
└── DERIVED/                       # Rebuildable derived objects
    └── scc_latest                 # Current SCC compression view
```

### 5.2 Gitignore Rules

The following paths should be gitignored:

```gitignore
# CTX rebuildable indexes
.ctx/index/
.ctx/DERIVED/
.ctx/LOCK
.ctx/*.tmp
```

### 5.3 File Permissions and Safety

| File | Permission | Safety Mechanism |
|------|------------|------------------|
| `objects/*` | Read-only after creation | Write-once semantics |
| `refs/*` | Read-write | Atomic temp+rename |
| `HEAD`, `STAGE` | Read-write | Atomic temp+rename |
| `index/*` | Read-write | Rebuildable from objects |
| `LOCK` | Exclusive lock | OS file locking |

---

## 6. Content-Addressed Object Store

### 6.1 Core Properties

| Property | Description |
|----------|-------------|
| **Immutability** | Objects are never modified after creation |
| **Deduplication** | Identical content produces identical IDs, stored once |
| **Integrity** | Hash verification on read detects corruption |
| **Merge-friendly** | Independent files, no merge conflicts possible |

### 6.2 Object ID Specification

```
Algorithm: BLAKE3
Length: 32 bytes (256 bits)
Encoding: Lowercase hexadecimal (64 characters)
Sharding: First byte (2 hex chars) as subdirectory
```

**Example:**
```
Object ID: ab12cd34ef56789...
Path: .ctx/objects/ab/ab12cd34ef56789...
```

### 6.3 Canonical Byte Format

All objects use a canonical envelope format to prevent cross-kind collisions:

```
┌─────────────────────────────────────────┐
│ Magic (5 bytes): "CTXO1"                │
├─────────────────────────────────────────┤
│ Kind (1 byte): enum value               │
├─────────────────────────────────────────┤
│ Payload Length (8 bytes): u64 LE        │
├─────────────────────────────────────────┤
│ Payload (variable): raw bytes           │
└─────────────────────────────────────────┘
```

**Critical:** Object ID is computed from uncompressed canonical bytes. Storage may use compression (zstd), but IDs remain stable regardless of compression settings.

### 6.4 Object Kinds

```rust
#[repr(u8)]
pub enum ObjectKind {
    Blob = 1,       // Raw bytes: source files, logs, markdown snapshots
    Typed = 2,      // Serialized struct: commits, edges, metadata
}
```

### 6.5 Storage Operations

**Write Flow:**
1. Compute canonical bytes (envelope + payload)
2. Compute BLAKE3 hash → ObjectId
3. Check if object already exists (dedup)
4. Compress canonical bytes with zstd
5. Write to temporary file
6. fsync temporary file
7. Atomic rename to final path
8. fsync parent directory (Unix)

**Read Flow:**
1. Compute expected path from ObjectId
2. Read compressed bytes
3. Decompress to canonical bytes
4. Verify BLAKE3 hash matches ObjectId
5. Parse envelope, extract payload
6. Return payload bytes

### 6.6 Typed Object Serialization

Typed objects use `postcard` for deterministic binary encoding. **Constraints:**

- No `HashMap` or `HashSet` (non-deterministic iteration order)
- Use `BTreeMap` or `Vec<(K, V)>` sorted by key
- No floating-point NaN values
- All strings must be valid UTF-8

---

## 7. Data Models

### 7.1 Core Object Types

#### 7.1.1 Blob

Raw bytes with no structure. Used for file contents, logs, markdown snapshots, and model outputs.

```rust
// Blobs have no struct - they're just raw bytes
// ID = hash(canonical_bytes(Kind::Blob, raw_bytes))
```

#### 7.1.2 Commit

The history DAG node. References a point-in-time snapshot of all repository state.

```rust
#[derive(Serialize, Deserialize)]
pub struct Commit {
    /// Parent commit IDs (empty for initial commit)
    pub parents: Vec<ObjectId>,
    
    /// Unix timestamp (seconds since epoch)
    pub timestamp_unix: i64,
    
    /// Human-readable commit message
    pub message: String,
    
    /// Root tree snapshot
    pub root_tree: ObjectId,
    
    /// Edge batches introduced in this commit
    pub edge_batches: Vec<ObjectId>,
    
    /// Snapshots of narrative files at commit time
    pub narrative_refs: Vec<NarrativeRef>,
    
    /// Optional: Cargo workspace snapshot
    pub cargo_snapshot: Option<ObjectId>,
    
    /// Optional: Rust semantic graph snapshot
    pub rust_snapshot: Option<ObjectId>,
    
    /// Optional: Diagnostics snapshot
    pub diagnostics_snapshot: Option<ObjectId>,
}
```

#### 7.1.3 Tree

A snapshot of file hierarchy, similar to Git trees.

```rust
#[derive(Serialize, Deserialize)]
pub struct Tree {
    pub entries: Vec<TreeEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct TreeEntry {
    pub name: String,
    pub kind: TreeEntryKind,
    pub id: ObjectId,
}

#[derive(Serialize, Deserialize)]
pub enum TreeEntryKind {
    Blob,
    Tree,
}
```

#### 7.1.4 NarrativeRef

Reference to a narrative file snapshot within a commit.

```rust
#[derive(Serialize, Deserialize)]
pub struct NarrativeRef {
    /// Path relative to .ctx/narrative/
    pub path: String,
    
    /// Stream identifier (e.g., "main", "work", "feature-x")
    pub stream: String,
    
    /// Document role (e.g., "log", "task", "decision", "doc")
    pub role: String,
    
    /// Blob ID containing the markdown content at commit time
    pub blob_id: ObjectId,
}
```

#### 7.1.5 WorkCommit

Staging commit for work-in-progress sessions.

```rust
#[derive(Serialize, Deserialize)]
pub struct WorkCommit {
    /// Previous work commit in this session
    pub parents: Vec<ObjectId>,
    
    /// The canonical commit this session branched from
    pub base: ObjectId,
    
    /// Session identifier
    pub session_id: String,
    
    /// Creation timestamp
    pub created_at: i64,
    
    /// What kind of step this represents
    pub step_kind: StepKind,
    
    /// Object IDs of artifacts created in this step
    pub payload: Vec<ObjectId>,
    
    /// Optional narrative references
    pub narrative_refs: Vec<NarrativeRef>,
}

#[derive(Serialize, Deserialize)]
pub enum StepKind {
    SessionStart,
    FileRead,
    FileWrite,
    CommandRun,
    Note,
    Plan,
    Compact,
}
```

### 7.2 Rust Repository Models

#### 7.2.1 FileNode and FileVersion

```rust
#[derive(Serialize, Deserialize)]
pub struct FileNode {
    /// Stable ID: hash(repo_id, normalized_path)
    pub file_id: ObjectId,
    
    /// Normalized path from repo root
    pub path: String,
    
    /// Language hint (e.g., "rust", "toml", "markdown")
    pub language: Option<String>,
    
    /// Last commit that updated this file
    pub last_seen_commit: ObjectId,
}

#[derive(Serialize, Deserialize)]
pub struct FileVersion {
    /// ID: hash(file_id, blob_id)
    pub file_version_id: ObjectId,
    
    /// Parent file node
    pub file_id: ObjectId,
    
    /// Content blob
    pub blob_id: ObjectId,
    
    /// Content statistics
    pub byte_count: u64,
    pub line_count: Option<u32>,
}
```

#### 7.2.2 Cargo Workspace Graph

```rust
#[derive(Serialize, Deserialize)]
pub struct CargoMetadataSnapshot {
    pub workspace_root: String,
    pub packages: Vec<Package>,
    pub resolve: Vec<PackageDepEdge>,
}

#[derive(Serialize, Deserialize)]
pub struct Package {
    /// Cargo package ID string from `cargo metadata`
    pub package_id: String,
    
    pub name: String,
    pub version: String,
    pub manifest_path: String,
    pub edition: String,
    pub targets: Vec<Target>,
}

#[derive(Serialize, Deserialize)]
pub struct Target {
    /// ID: hash(package_id, name, kind)
    pub target_id: ObjectId,
    
    pub name: String,
    pub kind: TargetKind,
    pub src_path: String,
    pub crate_types: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub enum TargetKind {
    Lib,
    Bin,
    Test,
    Bench,
    Example,
    ProcMacro,
}

#[derive(Serialize, Deserialize)]
pub struct PackageDepEdge {
    pub from_package_id: String,
    pub to_package_id: String,
    pub dep_kind: DepKind,
    pub optional: bool,
    pub features_enabled: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}
```

#### 7.2.3 Rust Semantic Graph

```rust
#[derive(Serialize, Deserialize)]
pub struct Span {
    pub file_id: ObjectId,
    pub file_version_id: Option<ObjectId>,
    pub start_byte: u32,
    pub end_byte: u32,
    pub start_line: Option<u32>,
    pub start_col: Option<u32>,
    pub end_line: Option<u32>,
    pub end_col: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct RustCrate {
    /// ID: hash(package_id, target_id)
    pub crate_id: ObjectId,
    
    pub package_id: String,
    pub target_id: ObjectId,
    pub root_module_id: ObjectId,
}

#[derive(Serialize, Deserialize)]
pub struct RustModule {
    /// ID: hash(crate_id, module_path_joined)
    pub module_id: ObjectId,
    
    pub crate_id: ObjectId,
    pub module_path: Vec<String>,  // e.g., ["core", "net", "client"]
    pub file_id: ObjectId,
    pub declared_span: Option<Span>,
}

#[derive(Serialize, Deserialize)]
pub struct RustItem {
    /// ID: hash(crate_id, kind, module_id, name, span.start_byte)
    pub item_id: ObjectId,
    
    /// Optional stable key for cross-snapshot identity
    pub stable_key: Option<String>,
    
    pub crate_id: ObjectId,
    pub module_id: ObjectId,
    pub kind: ItemKind,
    pub name: String,
    pub visibility: Visibility,
    pub declared_span: Span,
    
    /// Hash of signature for detecting changes
    pub signature_hash: Option<[u8; 16]>,
    
    /// Documentation blob if extracted
    pub doc_blob_id: Option<ObjectId>,
}

#[derive(Serialize, Deserialize)]
pub enum ItemKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
    Const,
    Static,
    TypeAlias,
    Macro,
}

#[derive(Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Crate,
    Super,
    Private,
}

#[derive(Serialize, Deserialize)]
pub struct UseSite {
    pub use_id: ObjectId,
    pub module_id: ObjectId,
    pub span: Span,
    pub import_kind: ImportKind,
    pub resolved_to_item: Option<ObjectId>,
    pub resolved_to_module: Option<ObjectId>,
}

#[derive(Serialize, Deserialize)]
pub enum ImportKind {
    Single,
    Glob,
    SelfImport,
    Super,
    Crate,
}

#[derive(Serialize, Deserialize)]
pub struct ImplBlock {
    pub impl_id: ObjectId,
    pub module_id: ObjectId,
    pub span: Span,
    pub self_type_repr: Option<ObjectId>,  // Blob with type string
    pub trait_repr: Option<ObjectId>,       // Blob with trait string
    pub provides_items: Vec<ObjectId>,
}
```

#### 7.2.4 Diagnostics

```rust
#[derive(Serialize, Deserialize)]
pub struct DiagnosticsSnapshot {
    pub tool: DiagnosticTool,
    pub timestamp: i64,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Serialize, Deserialize)]
pub enum DiagnosticTool {
    Rustc,
    Clippy,
    CargoTest,
    Custom(String),
}

#[derive(Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Option<String>,
    pub message_blob_id: ObjectId,
    pub spans: Vec<Span>,
    pub related_blob_id: Option<ObjectId>,
}

#[derive(Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}
```

---

## 8. Narrative System

### 8.1 Design Philosophy

Narrative is the human-readable layer of the context system. It consists of Markdown documents explaining intent, decisions, and ongoing work. Narrative files are editable by humans with standard text tools.

### 8.2 Narrative Space Structure

The default narrative space lives in `.ctx/narrative/` with the following organization:

```
narrative/
├── README.md           # Persistent overview of the repository
├── decisions.md        # Architectural decisions and rationale
├── log/
│   ├── YYYY-MM-DD.md   # Daily append-only journal entries
│   └── ...
├── tasks/
│   ├── task_NNNN.md    # Individual task files
│   └── ...
└── work/               # Optional session work logs
    └── <session_id>/
        └── notes.md
```

### 8.3 Document Roles

| Role | File Pattern | Purpose | Retrieval Priority |
|------|--------------|---------|-------------------|
| `overview` | `README.md` | High-level repository description | Medium |
| `decision` | `decisions.md` | Architectural choices with rationale | High when relevant |
| `log` | `log/YYYY-MM-DD.md` | Chronological session notes | High for recent |
| `task` | `tasks/task_NNNN.md` | Specific task context | High when active |
| `work` | `work/<session>/notes.md` | Ephemeral session scratch | Low (debugging only) |

### 8.4 Narrative Streams

The system supports multiple narrative streams for different purposes:

| Stream | Location | Use Case |
|--------|----------|----------|
| `main` | `narrative/` | Curated, canonical documentation |
| `work` | `narrative/work/<session>/` | Ephemeral session notes |
| `feature-<slug>` | `narrative/feature/<slug>/` | Feature-specific documentation |

### 8.5 Narrative Snapshots in Commits

When a commit is created, changed narrative files are snapshotted as blobs. The commit stores `NarrativeRef` entries pointing to these snapshots.

**Snapshot policy:**
1. Only snapshot files that changed since the last commit
2. Always snapshot the active task file
3. Always snapshot today's log file if it exists
4. Optionally snapshot session work logs for audit trails

### 8.6 Narrative File Format

All narrative files should follow this structure:

```markdown
# Title

<!-- metadata:
created: 2026-01-20T10:00:00Z
updated: 2026-01-20T15:30:00Z
tags: [rust, async, networking]
mentions: [crates/core/src/net.rs, core::net::Client]
-->

## Summary

Brief overview of what this document covers.

## Content

Main content here...

## References

- Link to related documents
- Link to external resources
```

---

## 9. Relationship Graph

### 9.1 Edge Model

Relationships are stored as immutable edge batches, not as a mutable graph database.

```rust
#[derive(Serialize, Deserialize)]
pub struct EdgeBatch {
    /// Edges created in this batch
    pub edges: Vec<Edge>,

    /// Timestamp when batch was created (Unix seconds)
    pub created_at: u64,

    // Note: To find which commit introduced this EdgeBatch, query commit
    // history to find which commit's edge_batches field contains this
    // EdgeBatch's ObjectId. This avoids self-reference issues in
    // content-addressed storage (chicken-and-egg: EdgeBatch needs commit ID,
    // Commit needs EdgeBatch ID).
}

#[derive(Serialize, Deserialize)]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    pub label: EdgeLabel,
    pub weight: Option<f32>,
    pub evidence: Evidence,
}

#[derive(Serialize, Deserialize)]
pub struct NodeId {
    pub kind: NodeKind,
    pub id: ObjectId,
}

#[derive(Serialize, Deserialize)]
pub enum NodeKind {
    File,
    Module,
    Item,
    Package,
    Target,
    Crate,
    Task,
    Note,
    Decision,
    Diagnostic,
}

#[derive(Serialize, Deserialize)]
pub struct Evidence {
    /// Commit that produced this evidence
    pub commit_id: ObjectId,
    
    /// Span in source if applicable
    pub span: Option<Span>,
    
    /// Blob containing supporting text
    pub blob_id: Option<ObjectId>,
    
    /// Tool that produced this edge
    pub tool: EvidenceTool,
    
    /// Confidence level
    pub confidence: Confidence,
}

#[derive(Serialize, Deserialize)]
pub enum EvidenceTool {
    Cargo,
    Parser,
    RustAnalyzer,
    Human,
    Llm,
}

#[derive(Serialize, Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
}
```

### 9.2 Edge Labels

A controlled vocabulary of edge labels prevents graph sprawl:

#### Tier 1: Structural Truth (Always Relate)

| Label | From | To | Meaning |
|-------|------|-----|---------|
| `Contains` | Directory/Module | File/Module/Item | Containment |
| `Defines` | File/Module | Module/Item | Definition site |
| `HasVersion` | FileNode | FileVersion | Version pointer |

#### Tier 2: Build Graph (Always Relate for Rust)

| Label | From | To | Meaning |
|-------|------|-----|---------|
| `DependsOn` | Package | Package | Cargo dependency |
| `TargetOf` | Target | Package | Target belongs to package |
| `CrateFromTarget` | Crate | Target | Crate derived from target |

#### Tier 3: Semantics (Relate When Feasible)

| Label | From | To | Meaning |
|-------|------|-----|---------|
| `Imports` | Module | Module/Item | Use statement |
| `References` | Item | Item | General reference |
| `Calls` | Function | Function | Function call |
| `Implements` | Impl | Trait | Trait implementation |
| `UsesType` | Item | Type | Type usage |

#### Tier 4: Narrative (Relate Selectively)

| Label | From | To | Meaning |
|-------|------|-----|---------|
| `Mentions` | Note/Task/Decision | Any | Narrative mentions code |
| `UpdatedIn` | FileNode/Item | Commit | History tracking |
| `DerivedFrom` | Derived | Sources | Derivation chain |

### 9.3 Edge Creation Policy

Use this decision filter before creating edges:

1. **Is it likely to help answer "what should I read next?"** If no, skip.
2. **Is it stable enough to survive refactors?** If volatile, scope to snapshot.
3. **Can you attach evidence and provenance?** If no traceable evidence, keep as narrative hypothesis.
4. **Is it cheap to compute and verify?** Expensive edges should be computed lazily.

### 9.4 Graph Reconstruction

The full relationship graph at a commit is reconstructed by:

1. Walk commit ancestry to find all reachable `EdgeBatch` IDs
2. Union all edges from all batches
3. Build adjacency lists (from → [(label, to)] and to → [(label, from)])
4. Store in rebuildable index for fast access

### 9.5 SCC Compression

Since semantic relationships can contain cycles (mutual recursion, circular imports), the system provides a DAG view via Strongly Connected Component compression.

**Computation:**
1. Build directed graph from edges at HEAD
2. Compute SCCs using Tarjan's algorithm
3. Create `SccView` derived object:
   - Map: NodeId → SccId
   - List of SCC super-nodes
   - DAG edges between SCCs
4. Store pointer in `.ctx/DERIVED/scc_latest`

**Use cases:**
- Topological ordering for incremental builds
- Cycle detection and reporting
- Predictable graph traversal

---

## 10. Indexing Strategy

### 10.1 Index Properties

| Property | Value |
|----------|-------|
| Storage | `.ctx/index/` |
| Backend | redb (recommended) or SQLite |
| Durability | Rebuildable from objects |
| Version control | gitignored |

### 10.2 Required Indexes

#### 10.2.1 Path Index

```
Key: normalized_path (String)
Value: FileId
```

Fast lookup: "Give me the FileId for `src/main.rs`"

#### 10.2.2 Name Index

```
Key: (namespace, name) (String, String)
Value: Vec<ObjectId>
```

Namespaces: `package`, `module`, `item`, `task`, `note`

Fast lookup: "Find all items named `connect`"

#### 10.2.3 Stable Key Index

```
Key: stable_key (String)
Value: ItemId
```

Fast lookup: "Find item by fully qualified path"

#### 10.2.4 Snapshot Resolution Index

```
Key: CommitId
Value: SnapshotPointers {
    root_tree: ObjectId,
    cargo_snapshot: Option<ObjectId>,
    rust_snapshot: Option<ObjectId>,
    narrative_refs: Vec<NarrativeRef>,
}
```

Fast lookup: "Get all pointers for commit X"

#### 10.2.5 Adjacency Index

```
Key: (direction, NodeId, EdgeLabel)
Value: Vec<NodeId>

direction: Forward | Backward
```

Fast lookup: "What does module X import?"

### 10.3 Optional Indexes

#### 10.3.1 Full-Text Index (Tantivy)

Index narrative documents and selected code comments for keyword search.

```
Schema:
- id: ObjectId
- kind: "narrative" | "comment" | "doc"
- path: String
- content: Text (indexed)
- updated_at: DateTime
```

#### 10.3.2 Embedding Index (Future)

Vector embeddings for semantic similarity search.

```
Schema:
- id: ObjectId
- embedding: Vec<f32>
- chunk_start: u32
- chunk_end: u32
```

### 10.4 Index Rebuild

The `ctx rebuild` command reconstructs all indexes from immutable objects:

1. Delete `.ctx/index/` directory
2. Walk all commits reachable from refs
3. For each commit, extract and index:
   - File paths from trees
   - Items from Rust snapshots
   - Packages from Cargo snapshots
   - Edges from edge batches
4. Build SCC derived view
5. Optionally rebuild full-text index

---

## 11. Session and Staging Management

### 11.1 Two-Layer State Model

| Layer | Location | Purpose | Persistence |
|-------|----------|---------|-------------|
| **Canonical** | `.ctx/refs/main` | Curated, clean history | Permanent |
| **Staging** | `.ctx/refs/work` or `.ctx/STAGE` | Work-in-progress | Until compaction |

### 11.2 Session Lifecycle

```
┌─────────────────────────────────────────────────────────────────┐
│                        Session Lifecycle                         │
└─────────────────────────────────────────────────────────────────┘

1. START SESSION
   ┌─────────────────────────────────────────┐
   │ Read canonical HEAD                      │
   │ Create initial WorkCommit               │
   │   parent = base = canonical HEAD        │
   │ Write to STAGE pointer                  │
   └─────────────────────────────────────────┘
                      │
                      ▼
2. AGENT STEPS (repeat)
   ┌─────────────────────────────────────────┐
   │ Agent performs action                   │
   │ Store artifacts, create EdgeBatch       │
   │ Create WorkCommit                       │
   │   parent = previous staging HEAD        │
   │   payload = [artifacts, edges]          │
   │ Advance STAGE pointer                   │
   └─────────────────────────────────────────┘
                      │
                      ▼
3. COMPACT SESSION
   ┌─────────────────────────────────────────┐
   │ Walk staging chain, collect artifacts   │
   │ Generate summary, create EdgeBatch     │
   │ Create canonical Commit                 │
   │   parent = base                         │
   │ Advance refs/main, reset STAGE          │
   └─────────────────────────────────────────┘
```

### 11.3 Staging Pointer Design

The staging pointer is a single tiny file containing one object ID:

**Option A: Dedicated file**
```
.ctx/STAGE
Contents: a3f9... (hex object id)
```

**Option B: Ref file (Git-style)**
```
.ctx/refs/work
Contents: a3f9... (hex object id)
```

**Atomic update procedure:**
1. Write new ID to `.ctx/STAGE.tmp`
2. fsync the temp file
3. Rename `.ctx/STAGE.tmp` to `.ctx/STAGE`
4. fsync parent directory

### 11.4 SessionState Object (Optional Enhancement)

For richer session control, the pointer can reference a `SessionState` typed object:

```rust
#[derive(Serialize, Deserialize)]
pub struct SessionState {
    pub session_id: String,
    pub base_commit: ObjectId,
    pub staging_head: ObjectId,
    pub created_at: i64,
    pub mode: SessionMode,
    pub budgets: SessionBudgets,
}

#[derive(Serialize, Deserialize)]
pub enum SessionMode {
    Idle,
    Running,
    Compacting,
}

#[derive(Serialize, Deserialize)]
pub struct SessionBudgets {
    pub max_files_per_step: u32,
    pub max_bytes_per_step: u64,
    pub max_edges_per_step: u32,
}
```

### 11.5 Crash Recovery

If the process crashes:

1. On startup, check if `.ctx/STAGE` exists
2. If yes, read staging HEAD and validate object exists
3. If valid, session can be resumed or compacted
4. If invalid, reset STAGE to canonical HEAD

All artifacts written before crash are preserved because objects are written atomically before pointer update.

### 11.6 Concurrent Access

Use OS file locking to prevent concurrent writers:

```rust
// Pseudocode
let lock = File::create(".ctx/LOCK")?;
lock.try_lock_exclusive()?;
// ... perform operations ...
// Lock released when file handle dropped
```

---

## 12. Prompt Pack Compilation

### 12.1 PromptPack Structure

```rust
#[derive(Serialize, Deserialize)]
pub struct PromptPack {
    /// The task or query being addressed
    pub task: String,
    
    /// Commit this pack was built from
    pub head_commit: ObjectId,
    
    /// Retrieved content chunks
    pub retrieved: Vec<RetrievedChunk>,
    
    /// Graph expansion metadata
    pub graph_context: GraphContext,
    
    /// Recent narrative excerpts
    pub recent_narrative: String,
    
    /// Token budget used / remaining
    pub token_budget: TokenBudget,
}

#[derive(Serialize, Deserialize)]
pub struct RetrievedChunk {
    pub title: String,
    pub object_id: ObjectId,
    pub snippet: String,
    pub relevance_score: f32,
    pub chunk_kind: ChunkKind,
}

#[derive(Serialize, Deserialize)]
pub enum ChunkKind {
    FileContent,
    NarrativeExcerpt,
    Decision,
    DiagnosticOutput,
    SymbolDefinition,
}

#[derive(Serialize, Deserialize)]
pub struct GraphContext {
    pub seed_nodes: Vec<NodeId>,
    pub expanded_nodes: Vec<NodeId>,
    pub expansion_depth: u32,
    pub scc_dag_used: bool,
}

#[derive(Serialize, Deserialize)]
pub struct TokenBudget {
    pub total: u32,
    pub used: u32,
    pub reserved_for_response: u32,
}
```

### 12.2 Retrieval Pipeline

```
┌─────────────────────────────────────────────────────────────────┐
│                      Retrieval Pipeline                          │
└─────────────────────────────────────────────────────────────────┘

1. SEED IDENTIFICATION
   ┌─────────────────────────────────────────┐
   │ Parse query, look up entities           │
   │ Add active task, recent files           │
   │ → Seed set: Vec<NodeId>                 │
   └─────────────────────────────────────────┘
                      │
                      ▼
2. GRAPH EXPANSION
   ┌─────────────────────────────────────────┐
   │ Expand via Imports, References,        │
   │   Contains, DependsOn edges             │
   │ Use SCC DAG, apply depth limit (2)      │
   │ → Expanded set: Vec<NodeId>             │
   └─────────────────────────────────────────┘
                      │
                      ▼
3. CONTENT RETRIEVAL
   ┌─────────────────────────────────────────┐
   │ Resolve to FileVersion, load content    │
   │ Extract spans, score by relevance       │
   │ → Ranked chunks: Vec<RetrievedChunk>   │
   └─────────────────────────────────────────┘
                      │
                      ▼
4. NARRATIVE INCLUSION
   ┌─────────────────────────────────────────┐
   │ Load active task, recent log, decisions │
   │ → Narrative excerpts                    │
   └─────────────────────────────────────────┘
                      │
                      ▼
5. BUDGET ALLOCATION
   ┌─────────────────────────────────────────┐
   │ Sort by relevance, fill budget          │
   │ Reserve tokens for response             │
   │ → Final PromptPack                      │
   └─────────────────────────────────────────┘
```

### 12.3 Default Retrieval Policy

```rust
pub struct RetrievalConfig {
    /// Maximum graph expansion depth
    pub max_depth: u32,                    // default: 2
    
    /// Edge labels to follow during expansion
    pub expand_labels: Vec<EdgeLabel>,     // default: [Imports, References, DependsOn]
    
    /// Maximum nodes to expand
    pub max_expanded_nodes: u32,           // default: 50
    
    /// Include narrative from last N days
    pub narrative_days: u32,               // default: 7
    
    /// Minimum confidence for edges
    pub min_edge_confidence: Confidence,   // default: Medium
    
    /// Token budget allocation
    pub budget: TokenBudget,
}
```

### 12.4 Prompt Pack JSON Format

```json
{
  "task": "Fix the connection timeout bug in core::net::Client",
  "head_commit": "ab12cd34...",
  "retrieved": [
    {
      "title": "File: crates/core/src/net/client.rs",
      "object_id": "ef56789a...",
      "snippet": "impl Client {\n    pub async fn connect(...) {\n        ...\n    }\n}",
      "relevance_score": 0.95,
      "chunk_kind": "FileContent"
    },
    {
      "title": "Decision: Connection timeout handling",
      "object_id": "bc234567...",
      "snippet": "We chose exponential backoff with jitter...",
      "relevance_score": 0.82,
      "chunk_kind": "Decision"
    }
  ],
  "graph_context": {
    "seed_nodes": ["core::net::Client", "task_0042"],
    "expanded_nodes": ["core::net::Client", "core::net::Connection", "core::timeout::Backoff"],
    "expansion_depth": 2,
    "scc_dag_used": true
  },
  "recent_narrative": "## 2026-01-20\n\n- Investigating timeout issues reported in #123\n- Found potential race condition in reconnect logic",
  "token_budget": {
    "total": 16000,
    "used": 12500,
    "reserved_for_response": 3500
  }
}
```

---

## 13. CLI Interface

### 13.1 Command Overview

```
ctx - Context management for coding agents

USAGE:
    ctx <COMMAND>

COMMANDS:
    init        Initialize a new context store
    add         Add files or notes to the store
    relate      Create relationship edges
    commit      Create a canonical commit
    stage       Manage staging state
    query       Build a prompt pack from query
    rebuild     Rebuild indexes from objects
    llm         Send prompt pack to LLM server
    gc          Garbage collect unreferenced objects
    debug       Debug and inspection commands
    help        Print help information
```

### 13.2 Command Specifications

#### 13.2.1 ctx init

Initialize a new context store in the current directory.

```
USAGE:
    ctx init [OPTIONS]

OPTIONS:
    --force         Overwrite existing .ctx directory
    --no-git        Don't add .ctx to git
    --template      Use a template configuration

EFFECTS:
    - Creates .ctx/ directory structure
    - Creates config.toml with defaults
    - Creates initial commit with empty tree
    - Sets refs/main to initial commit
```

#### 13.2.2 ctx add

Add content to the context store.

```
USAGE:
    ctx add <SUBCOMMAND>

SUBCOMMANDS:
    file <PATH>...              Add file snapshots
    note "<TEXT>"               Add a note to today's log
    task "<TITLE>" [--body]     Create a new task file
    decision "<TITLE>" [--body] Add to decisions.md
    cargo                       Snapshot Cargo workspace
    rust [--files <GLOB>]       Parse Rust files for semantics

OPTIONS:
    --commit          Immediately create a staging commit
    --message <MSG>   Commit message (implies --commit)

EXAMPLES:
    ctx add file src/main.rs src/lib.rs
    ctx add note "Fixed the timeout bug by adding retry logic"
    ctx add task "Implement rate limiting" --body "Need to add..."
    ctx add cargo
    ctx add rust --files "crates/core/src/**/*.rs"
```

#### 13.2.3 ctx relate

Create relationship edges.

```
USAGE:
    ctx relate <FROM> <LABEL> <TO> [OPTIONS]

ARGUMENTS:
    <FROM>      Source node (path, item name, or ID)
    <LABEL>     Edge label (Imports, References, Calls, Mentions, etc.)
    <TO>        Target node (path, item name, or ID)

OPTIONS:
    --confidence <LEVEL>    high, medium, low (default: high)
    --evidence <TEXT>       Evidence description
    --span <START:END>      Source span if applicable

EXAMPLES:
    ctx relate src/main.rs Imports src/lib.rs
    ctx relate "core::net::Client" Calls "core::timeout::sleep"
    ctx relate task_0042 Mentions src/net/client.rs
```

#### 13.2.4 ctx commit

Create a canonical commit.

```
USAGE:
    ctx commit [OPTIONS]

OPTIONS:
    -m, --message <MSG>     Commit message (required)
    --amend                 Amend the previous commit
    --no-narrative          Don't snapshot narrative files

EFFECTS:
    - Creates Commit object with current state
    - Snapshots changed narrative files
    - Advances refs/main
```

#### 13.2.5 ctx stage

Manage staging state.

```
USAGE:
    ctx stage <SUBCOMMAND>

SUBCOMMANDS:
    start <TASK>                Start a new staging session (auto-generates UUID session ID)
    flush                       Persist current staging state
    compact --message <MSG>     Compact staging into canonical commit
    abort [--reason <REASON>]   Abort session and create abort commit
    status                      Show staging status
    recover                     Recover session from staging area after crash

EXAMPLES:
    ctx stage start "Fix timeout handling"
    ctx stage flush
    ctx stage compact --message "Completed timeout fix"
    ctx stage abort --reason "Changed approach"
```

#### 13.2.6 ctx query

Build a prompt pack from a query.

```
USAGE:
    ctx query "<QUERY>" [OPTIONS]

OPTIONS:
    --budget <TOKENS>       Token budget (default: 16000)
    --depth <N>             Graph expansion depth (default: 2)
    --output <FILE>         Write pack to file (default: stdout)
    --format <FMT>          json, yaml, or text (default: json)
    --include-narrative     Include narrative excerpts
    --no-graph              Skip graph expansion

EXAMPLES:
    ctx query "What files are involved in the timeout handling?"
    ctx query "Show me the Client struct" --budget 8000 --depth 1
    ctx query "Recent decisions about error handling" --include-narrative
```

#### 13.2.7 ctx rebuild

Rebuild indexes from immutable objects.

```
USAGE:
    ctx rebuild [OPTIONS]

OPTIONS:
    --full              Full rebuild (delete and recreate)
    --incremental       Incremental update (default)
    --scc               Only rebuild SCC derived view
    --tantivy           Only rebuild full-text index

EFFECTS:
    - Deletes and recreates .ctx/index/
    - Rebuilds all key-value indexes
    - Recomputes SCC compression
```

#### 13.2.8 ctx llm

Send a prompt pack to an LLM server.

```
USAGE:
    ctx llm [OPTIONS]

OPTIONS:
    --server <URL>          LLM server URL (default: http://127.0.0.1:8080)
    --pack <FILE>           Prompt pack file (or use stdin)
    --query "<QUERY>"       Build pack from query first
    --model <NAME>          Model name to use
    --max-tokens <N>        Max response tokens
    --temperature <F>       Sampling temperature
    --commit                Commit response as note

EXAMPLES:
    ctx llm --query "Explain the Client struct" --server http://localhost:8080
    ctx query "..." | ctx llm --commit
```

#### 13.2.9 ctx gc

Garbage collect unreferenced objects.

```
USAGE:
    ctx gc [OPTIONS]

OPTIONS:
    --dry-run           Show what would be deleted
    --aggressive        Also remove staging objects
    --keep-days <N>     Keep unreferenced objects for N days (default: 7)

EFFECTS:
    - Marks all objects reachable from refs
    - Deletes unreferenced objects older than threshold
```

#### 13.2.10 ctx debug

Debug and inspection commands.

```
USAGE:
    ctx debug <SUBCOMMAND>

SUBCOMMANDS:
    cat <OBJECT_ID>         Print object contents
    graph [--format dot]    Export relationship graph
    history [--limit N]     Show commit history
    refs                    Show all refs and their targets
    index <KEY>             Query index directly
    verify                  Verify all object integrity
```

---

## 14. LLM Integration

### 14.1 Integration Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                      Agent Host Process                           │
│                                                                   │
│  ┌─────────────────────┐      ┌─────────────────────┐            │
│  │    CTX Core         │      │    LLM Client       │            │
│  │                     │      │                     │            │
│  │  build_pack(query)──┼─────►│  compose_prompt()   │            │
│  │                     │      │         │           │            │
│  │  observe_response()◄┼──────┼─────────┘           │            │
│  └─────────────────────┘      └──────────┬──────────┘            │
│                                          │                        │
└──────────────────────────────────────────┼────────────────────────┘
                                           │ HTTP
                                           ▼
┌──────────────────────────────────────────────────────────────────┐
│                      llama.cpp Server                             │
│                                                                   │
│  POST /completion                                                 │
│  {                                                               │
│    "prompt": "...",                                              │
│    "n_predict": 2048,                                            │
│    "temperature": 0.7                                            │
│  }                                                               │
└──────────────────────────────────────────────────────────────────┘
```

### 14.2 Prompt Composition

The LLM client transforms a PromptPack into a prompt string:

```rust
pub fn compose_prompt(pack: &PromptPack, config: &PromptConfig) -> String {
    let mut prompt = String::new();
    
    // System context
    prompt.push_str(&config.system_prefix);
    
    // Task statement
    prompt.push_str(&format!("## Task\n\n{}\n\n", pack.task));
    
    // Retrieved context
    prompt.push_str("## Retrieved Context\n\n");
    for chunk in &pack.retrieved {
        prompt.push_str(&format!("### {}\n\n```\n{}\n```\n\n", 
            chunk.title, chunk.snippet));
    }
    
    // Recent narrative
    if !pack.recent_narrative.is_empty() {
        prompt.push_str(&format!("## Recent Notes\n\n{}\n\n", 
            pack.recent_narrative));
    }
    
    // Response instruction
    prompt.push_str(&config.response_instruction);
    
    prompt
}
```

### 14.3 Response Processing

After receiving the LLM response:

1. Parse response for structured outputs (code blocks, decisions)
2. Store response blob in object store
3. Extract any mentioned file paths or symbols
4. Create edges: Response → Mentions → extracted entities
5. Optionally append summary to daily log
6. Advance staging pointer

### 14.4 Session State Management

For prompt caching and session continuity with llama.cpp:

```rust
pub struct LlmSessionState {
    /// Session tokens from previous interaction
    pub session_file: Option<PathBuf>,
    
    /// Prompt prefix that's cached
    pub cached_prefix_hash: Option<[u8; 32]>,
    
    /// Number of cached tokens
    pub cached_token_count: u32,
}
```

**Caching strategy:**
1. Save llama.cpp session state after each interaction
2. On next interaction, check if prompt prefix matches
3. If match, restore session and continue from cached state
4. If mismatch, start fresh session

---

## 15. Compaction and Garbage Collection

### 15.1 Compaction Process

Compaction transforms messy staging history into clean canonical commits.

```
┌─────────────────────────────────────────────────────────────────┐
│                      Compaction Process                          │
└─────────────────────────────────────────────────────────────────┘

INPUT:
  - Staging chain: WorkCommit → WorkCommit → ... → base
  - All artifacts referenced by staging commits

PROCESS:
  1. Collect artifacts, deduplicate file versions
  2. Merge edge batches, generate summary
  3. Write summary to narrative log
  4. Create curated EdgeBatch with high-confidence edges
  5. Create canonical Commit:
     - parent = base commit
     - message = session summary
     - edge_batches = [curated batch]
     - narrative_refs = [updated log, task if completed]

OUTPUT:
  - New canonical commit on refs/main
  - Staging reset to new canonical HEAD
  - Session artifacts preserved (for gc later)
```

### 15.2 Compaction Triggers

| Trigger | Condition |
|---------|-----------|
| **Manual** | User runs `ctx stage compact` |
| **Token budget** | Staging artifacts exceed token threshold |
| **Time-based** | Session duration exceeds limit (e.g., 4 hours) |
| **Task completion** | Agent marks task as complete |
| **Context switch** | User starts a different task |

### 15.3 Garbage Collection

GC removes unreferenced objects to reclaim disk space.

**Reachability rules:**
1. All objects reachable from `refs/main` are kept
2. All objects reachable from `refs/work` (staging) are kept
3. Objects unreferenced for more than `keep_days` are candidates
4. Narrative blobs are always kept (small, valuable)

**GC algorithm:**
```
1. Build reachable set:
   - Start from all refs
   - Walk commits, trees, edge batches
   - Mark all referenced object IDs

2. Scan objects directory:
   - For each object file:
     - If ID in reachable set: skip
     - If mtime < now - keep_days: delete
     - Else: skip (grace period)

3. Report statistics:
   - Objects scanned
   - Objects deleted
   - Bytes reclaimed
```

### 15.4 Pack Files (Future Enhancement)

For repositories with many objects, pack multiple small objects into larger pack files:

```
.ctx/objects/pack/
  pack-001.pack    # Concatenated objects
  pack-001.idx     # Index: object_id → offset
```

Benefits: fewer filesystem entries, better compression, faster enumeration.

---

## 16. Ingestion Policy

### 16.1 Core Principle

**Ingest what changes future agent behavior or would be expensive to rediscover.** Think of the context store as a lab notebook plus a graph of facts, not a screen recorder.

### 16.2 Ingestion Decision Rule

Ingest when any criterion applies:

| Criterion | Examples |
|-----------|----------|
| **New constraint** | User requirement, API contract |
| **Decision** | Chosen approach, rejected alternatives |
| **Evidence** | Test failure, benchmark result |
| **Useful relation** | Import discovered, call graph edge |
| **Costly to reproduce** | Long compilation output, model response |

If none apply, keep it ephemeral.

### 16.3 Default Ingestion Policy by Step

| Step | Always Ingest | Usually Ingest | Rarely Ingest |
|------|---------------|----------------|---------------|
| **Task received** | Task object, narrative entry | Initial seed relations | Raw user input |
| **Planning** | Final plan decision | Tradeoffs considered | Brainstorming |
| **File read** | File snapshot (if used for reasoning) | Defines, Imports edges | Unread files |
| **File write** | Patch blob, updated snapshot | Changed symbol edges | Intermediate drafts |
| **Build/test** | Exit code, diagnostics | Failure spans | Verbose logs |
| **Completion** | Outcome note, follow-ups | Session summary | Ephemeral scratch |

### 16.4 Ingestion Gates

#### Gate 1: Content Deduplication

If blob hash matches existing object, reuse without creating FileVersion.

#### Gate 2: Per-Step Budgets

```rust
pub struct StepBudgets {
    pub max_files_per_step: u32,        // default: 10
    pub max_bytes_per_step: u64,        // default: 1MB
    pub max_log_lines: u32,             // default: 500
    pub max_edges_per_step: u32,        // default: 100
}
```

#### Gate 3: Confidence Thresholds

Only persist low-confidence edges if they were used in a decision or referenced by the task.

#### Gate 4: Neighborhood Extraction

When a file changes, extract relations only for:
- Changed modules and items
- Direct imports (1-hop)
- Direct callers/callees (if available)

Do not compute whole-program graphs.

### 16.5 Simple Default Policy

```
ON task_start:
    ingest(Task, user_request)
    ingest(Note, "Task started")
    
ON file_read(path) IF used_for_reasoning:
    ingest(FileVersion, path)
    ingest(Edges, extract_defines_imports(path))
    
ON file_write(path):
    ingest(FileVersion, path)
    ingest(Edges, extract_defines_imports(path))
    
ON command_run(cmd, output, exit_code) IF exit_code != 0 OR explicitly_requested:
    ingest(Blob, output)
    ingest(Diagnostic, parse_diagnostics(output))
    
ON task_complete:
    ingest(Note, summary)
    compact_session()
```

---

## 17. Implementation Roadmap

### 17.1 Phase 1: Foundation (Weeks 1-2)

**Goal:** Working object store with basic CLI

| Task | Priority | Estimate |
|------|----------|----------|
| Object store with BLAKE3 + zstd | P0 | 2 days |
| Blob and Typed object support | P0 | 1 day |
| HEAD and refs pointer management | P0 | 1 day |
| Basic Commit and Tree types | P0 | 2 days |
| `ctx init`, `ctx add file`, `ctx commit` | P0 | 2 days |
| Basic index with redb | P1 | 2 days |

**Deliverable:** Can init repo, add files, create commits, verify integrity.

### 17.2 Phase 2: Narrative and Relations (Weeks 3-4)

**Goal:** Human-readable narrative and basic graph

| Task | Priority | Estimate |
|------|----------|----------|
| Narrative file structure | P0 | 1 day |
| NarrativeRef in commits | P0 | 1 day |
| `ctx add note`, `ctx add task` | P0 | 1 day |
| EdgeBatch and Edge types | P0 | 2 days |
| `ctx relate` command | P0 | 1 day |
| Adjacency index | P0 | 2 days |
| `ctx rebuild` command | P1 | 1 day |

**Deliverable:** Can create notes, tasks, edges; can query adjacency.

### 17.3 Phase 3: Rust Integration (Weeks 5-6)

**Goal:** Parse Rust repos and build semantic graph

| Task | Priority | Estimate |
|------|----------|----------|
| Cargo metadata parsing | P0 | 2 days |
| `ctx add cargo` command | P0 | 1 day |
| Basic Rust parser (tree-sitter or syn) | P0 | 3 days |
| Extract modules, items, imports | P0 | 2 days |
| `ctx add rust` command | P0 | 1 day |
| Build DependsOn, Imports, Defines edges | P0 | 2 days |

**Deliverable:** Can snapshot Cargo workspace and extract basic Rust semantics.

### 17.4 Phase 4: Retrieval and Prompt Pack (Weeks 7-8)

**Goal:** Query system produces useful prompt packs

| Task | Priority | Estimate |
|------|----------|----------|
| Graph expansion algorithm | P0 | 2 days |
| SCC computation and DAG view | P0 | 2 days |
| Prompt pack builder | P0 | 2 days |
| `ctx query` command | P0 | 1 day |
| Token budget management | P1 | 1 day |
| Relevance scoring (basic TF-IDF) | P1 | 2 days |

**Deliverable:** Can build prompt packs from queries with graph expansion.

### 17.5 Phase 5: Staging and LLM Integration (Weeks 9-10)

**Goal:** Full agent workflow with LLM

| Task | Priority | Estimate |
|------|----------|----------|
| WorkCommit and staging pointer | P0 | 2 days |
| `ctx stage` subcommands | P0 | 2 days |
| Compaction logic | P0 | 2 days |
| llama.cpp HTTP client | P0 | 1 day |
| `ctx llm` command | P0 | 1 day |
| Response processing and storage | P1 | 2 days |

**Deliverable:** End-to-end agent workflow with staging and compaction.

### 17.6 Phase 6: Polish and Extensions (Weeks 11-12)

**Goal:** Production-ready POC

| Task | Priority | Estimate |
|------|----------|----------|
| Garbage collection | P1 | 2 days |
| Full-text search with Tantivy | P1 | 2 days |
| Error handling and recovery | P0 | 2 days |
| Documentation and examples | P0 | 2 days |
| Performance benchmarking | P1 | 1 day |
| rust-analyzer integration (optional) | P2 | 3 days |

**Deliverable:** Robust, documented POC ready for real-world testing.

### 17.7 Crate Structure

```
crates/
├── ctx_core/
│   ├── src/
│   │   ├── lib.rs
│   │   ├── object_id.rs      # BLAKE3 object IDs
│   │   ├── object_store.rs   # Content-addressed storage
│   │   ├── types.rs          # Core data types
│   │   ├── refs.rs           # HEAD and refs management
│   │   ├── index.rs          # Rebuildable indexes
│   │   ├── graph.rs          # Graph operations and SCC
│   │   ├── pack.rs           # Prompt pack compilation
│   │   ├── narrative.rs      # Narrative file handling
│   │   ├── staging.rs        # Session and staging
│   │   ├── cargo.rs          # Cargo integration
│   │   └── rust_parse.rs     # Rust semantic extraction
│   └── Cargo.toml
│
├── ctx_cli/
│   ├── src/
│   │   ├── main.rs
│   │   └── commands/
│   │       ├── mod.rs
│   │       ├── init.rs
│   │       ├── add.rs
│   │       ├── relate.rs
│   │       ├── commit.rs
│   │       ├── stage.rs
│   │       ├── query.rs
│   │       ├── rebuild.rs
│   │       ├── llm.rs
│   │       ├── gc.rs
│   │       └── debug.rs
│   └── Cargo.toml
│
├── ctx_llm/
│   ├── src/
│   │   ├── lib.rs
│   │   ├── client.rs         # HTTP client for llama.cpp
│   │   ├── prompt.rs         # Prompt composition
│   │   └── response.rs       # Response processing
│   └── Cargo.toml
│
└── Cargo.toml                 # Workspace manifest
```

### 17.8 Dependencies

```toml
[workspace.dependencies]
# Hashing and compression
blake3 = "1.5"
zstd = "0.13"

# Serialization
serde = { version = "1.0", features = ["derive"] }
postcard = { version = "1.0", features = ["alloc"] }
serde_json = "1.0"

# Storage
redb = "2.0"

# CLI
clap = { version = "4.5", features = ["derive"] }

# Async and HTTP
tokio = { version = "1.36", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }

# Rust parsing
tree-sitter = "0.22"
tree-sitter-rust = "0.21"

# Utilities
anyhow = "1.0"
thiserror = "1.0"
hex = "0.4"
time = { version = "0.3", features = ["serde"] }
walkdir = "2.5"
glob = "0.3"

# Optional: full-text search
tantivy = { version = "0.22", optional = true }
```

---

## Appendix

### A.1 Glossary

| Term | Definition |
|------|------------|
| **Blob** | Raw bytes stored as an object (file content, log, etc.) |
| **Canonical commit** | A curated commit on the main history branch |
| **Compaction** | Process of converting staging work into a canonical commit |
| **Edge** | A directed relationship between two nodes |
| **EdgeBatch** | Immutable collection of edges created in one operation |
| **Evidence** | Provenance information attached to an edge |
| **Narrative** | Human-readable documentation in Markdown |
| **ObjectId** | BLAKE3 hash identifying a content-addressed object |
| **PromptPack** | Compiled retrieval result ready for LLM consumption |
| **SCC** | Strongly Connected Component (for cycle handling) |
| **Staging** | Work-in-progress state before compaction |
| **Typed object** | Serialized struct stored as an object |

### A.2 Edge Label Reference

```rust
pub enum EdgeLabel {
    // Tier 1: Structural
    Contains,
    Defines,
    HasVersion,
    
    // Tier 2: Build
    DependsOn,
    TargetOf,
    CrateFromTarget,
    
    // Tier 3: Semantics
    Imports,
    References,
    Calls,
    Implements,
    UsesType,
    
    // Tier 4: Narrative
    Mentions,
    UpdatedIn,
    DerivedFrom,
    
    // Future extensions
    AffectsTest,
    OwnedBy,
    PerformanceHot,
}
```

### A.3 Configuration Schema

```toml
# .ctx/config.toml

[repo]
# Repository identity (computed from path if not set)
id = "my-project"
# Context system version
ctx_version = "1.0.0"

[storage]
# Compression level for objects (0-22, default 3)
compression_level = 3
# Object ID prefix length for sharding (1-4, default 1)
shard_prefix_bytes = 1

[index]
# Index backend: "redb" or "sqlite"
backend = "redb"
# Enable full-text search
enable_tantivy = false

[ingestion]
# Files to ignore (glob patterns)
ignore_globs = [
    "target/**",
    ".git/**",
    "node_modules/**",
]
# Maximum files per step
max_files_per_step = 10
# Maximum bytes per step
max_bytes_per_step = 1048576

[retrieval]
# Default token budget
default_budget = 16000
# Default graph expansion depth
default_depth = 2
# Include narrative by default
include_narrative = true

[llm]
# Default server URL
server_url = "http://127.0.0.1:8080"
# Default model
model = "default"
# Response token limit
max_response_tokens = 2048
```

### A.4 Error Codes

| Code | Name | Description |
|------|------|-------------|
| E001 | ObjectNotFound | Referenced object does not exist |
| E002 | HashMismatch | Object content does not match ID |
| E003 | InvalidEnvelope | Object envelope format invalid |
| E004 | DeserializationFailed | Cannot deserialize typed object |
| E005 | RefNotFound | Reference file does not exist |
| E006 | LockConflict | Another process holds the lock |
| E007 | IndexCorrupt | Index is corrupt, needs rebuild |
| E008 | StagingConflict | Staging state is inconsistent |
| E009 | CommitOrphan | Commit has unreachable parent |
| E010 | BudgetExceeded | Operation exceeds configured budget |

### A.5 End-to-End Example: Building a Feature

This example walks through a complete feature development session using CTX, showing commands, objects created, and state changes.

#### Scenario

You're adding a **connection retry feature** to a Rust networking library. The task involves:
- Adding exponential backoff to `core::net::Client`
- Creating a new `core::retry::Backoff` module
- Updating tests

#### Initial State

```
Repository: my-network-lib/
├── crates/
│   └── core/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── net/
│           │   ├── mod.rs
│           │   └── client.rs      ← we'll modify this
│           └── retry/             ← we'll create this
└── .ctx/
    ├── HEAD                       → abc123 (current canonical commit)
    ├── refs/main                  → abc123
    ├── narrative/
    │   ├── README.md
    │   ├── log/
    │   │   └── 2026-01-19.md
    │   └── tasks/
    └── objects/
        └── ... (existing objects)
```

---

#### Step 1: Create the Task

**User action:** Define what we're building

```bash
$ ctx add task "Add connection retry with exponential backoff" --body "
## Objective
Add automatic retry logic to Client::connect() with exponential backoff.

## Requirements
- Max 5 retries
- Initial delay: 100ms
- Max delay: 10s
- Add jitter to prevent thundering herd

## Files likely involved
- crates/core/src/net/client.rs
- New: crates/core/src/retry/mod.rs
"
```

**What happens:**

1. Creates `.ctx/narrative/tasks/task_0043.md`
2. Creates blob → `blob:d4e5f6...`, Note metadata → `typed:a1b2c3...`
3. Appends to today's log

```markdown
## 2026-01-20

### 09:15 - Task Created
- Created task_0043: Add connection retry with exponential backoff
```

**Objects created:**
```
objects/
  d4/d4e5f6...  ← Blob: task markdown content
  a1/a1b2c3...  ← Typed: Note metadata
```

---

#### Step 2: Start a Staging Session

**User action:** Begin tracked work session

```bash
$ ctx stage start "Add connection retry with exponential backoff"
# Session ID: retry-feature-001 (auto-generated UUID)
```

**What happens:**

1. Reads HEAD: `abc123`
2. Creates WorkCommit with `base: abc123`, `session_id: "retry-feature-001"`
3. Stores → `typed:w00001...`, writes `.ctx/STAGE`

**State after:**
```
.ctx/
├── HEAD           → abc123
├── STAGE          → w00001... (staging head)
├── refs/main      → abc123
```

---

#### Step 3: Snapshot Cargo Workspace

**User action:** Capture build graph

```bash
$ ctx add cargo
```

**What happens:**

1. Runs `cargo metadata`, parses into `CargoMetadataSnapshot`
2. Stores → `typed:cargo01...`
3. Creates DependsOn edges: core → tokio, core → thiserror
4. Creates EdgeBatch → `typed:eb0001...`, StepRecord → `typed:sr0001...`
5. Creates WorkCommit with `payload: [cargo01..., eb0001..., sr0001...]`
6. Advances STAGE → `w00002...`

---

#### Step 4: Read Existing Code

**User action:** Examine the file we'll modify

```bash
$ ctx add file crates/core/src/net/client.rs --extract-semantics
```

**What happens:**

1. Reads file (500 lines), stores → `blob:client01...`
2. Creates FileVersion, parses with tree-sitter
3. Extracts: Module("core::net::client"), Items (Client, new, connect, send)
4. Creates Defines, Imports, Calls edges
5. Creates EdgeBatch → `typed:eb0002...`, appends to log
6. Creates WorkCommit, advances STAGE → `w00003...`

---

#### Step 5: Create New Module

**User action:** Write the retry logic

```bash
$ cat > /tmp/backoff.rs << 'EOF'
//! Exponential backoff implementation for retry logic.

use std::time::Duration;
use rand::Rng;

/// Configuration for exponential backoff.
#[derive(Debug, Clone)]
pub struct Backoff {
    initial_delay: Duration,
    max_delay: Duration,
    max_retries: u32,
    current_retry: u32,
}

impl Backoff {
    pub fn new(initial_delay: Duration, max_delay: Duration, max_retries: u32) -> Self {
        Self {
            initial_delay,
            max_delay,
            max_retries,
            current_retry: 0,
        }
    }

    /// Returns the next delay, or None if max retries exceeded.
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.current_retry >= self.max_retries {
            return None;
        }
        
        let base_delay = self.initial_delay.as_millis() as u64 
            * 2u64.pow(self.current_retry);
        let capped_delay = base_delay.min(self.max_delay.as_millis() as u64);
        
        // Add jitter: ±25%
        let jitter_range = capped_delay / 4;
        let jitter = rand::thread_rng().gen_range(0..=jitter_range * 2) as i64 
            - jitter_range as i64;
        let final_delay = (capped_delay as i64 + jitter).max(0) as u64;
        
        self.current_retry += 1;
        Some(Duration::from_millis(final_delay))
    }

    pub fn reset(&mut self) {
        self.current_retry = 0;
    }
}
EOF

$ mkdir -p crates/core/src/retry
$ cp /tmp/backoff.rs crates/core/src/retry/mod.rs

$ ctx add file crates/core/src/retry/mod.rs --extract-semantics
```

**What happens:**

1. Stores new file → `blob:backoff01...`, creates FileVersion
2. Parses, extracts Module("core::retry"), Items (Backoff, new, next_delay, reset)
3. Creates Defines, Imports edges
4. Creates EdgeBatch → `typed:eb0003...`, appends to log
5. Creates WorkCommit, advances STAGE → `w00004...`

---

#### Step 6: Modify Client to Use Backoff

**User action:** Integrate retry logic into connect()

```bash
# (User edits client.rs to add retry logic)
$ ctx add file crates/core/src/net/client.rs --extract-semantics
```

**What happens:**

1. Detects file changed, stores → `blob:client02...`
2. Re-parses, finds new import and modified function
3. Creates Imports, Calls edges
4. Creates EdgeBatch → `typed:eb0004...`, appends to log
5. Creates WorkCommit, advances STAGE → `w00005...`

---

#### Step 7: Add Task Relation

**User action:** Link our changes to the task

```bash
$ ctx relate task_0043 Mentions crates/core/src/retry/mod.rs
$ ctx relate task_0043 Mentions "core::net::Client::connect"
```

**What happens:**

1. Creates Mentions edges: task_0043 → File("retry/mod.rs"), task_0043 → Item("connect")
2. Creates EdgeBatch → `typed:eb0005...`, WorkCommit, advances STAGE → `w00006...`

---

#### Step 8: Run Tests (Failure)

**User action:** Check if it works

```bash
$ cargo test 2>&1 | ctx add command --stdin --name "cargo test"
```

**What happens:**

1. Captures output, stores → `blob:testout01...`
2. Parses diagnostics: Error E0433 "could not find 'retry' in 'crate'"
3. Creates References edge: Diagnostic → File("lib.rs")
4. Appends to log, creates WorkCommit, advances STAGE → `w00007...`

---

#### Step 9: Fix the Issue

**User action:** Add module declaration to lib.rs

```bash
# (User adds `pub mod retry;` to lib.rs)
$ ctx add file crates/core/src/lib.rs --extract-semantics
```

**What happens:**

1. Stores updated lib.rs → `blob:lib02...`
2. Extracts module declaration, creates Contains edge
3. Appends to log, creates WorkCommit, advances STAGE → `w00008...`

---

#### Step 10: Run Tests (Success)

**User action:** Verify the fix

```bash
$ cargo test 2>&1 | ctx add command --stdin --name "cargo test"
```

**What happens:**

1. Captures output, stores → `blob:testout02...`
2. Parses: 15 tests passed
3. Appends to log, creates WorkCommit, advances STAGE → `w00009...`

---

#### Step 11: Add Decision Record

**User action:** Document the design choice

```bash
$ ctx add decision "Retry strategy: exponential backoff with jitter" --body "
## Context
Client::connect() can fail transiently. Need automatic retry.

## Decision
Use exponential backoff with jitter:
- Initial delay: 100ms
- Max delay: 10s  
- Max retries: 5
- Jitter: ±25%

## Rationale
- Exponential backoff prevents overwhelming recovering servers
- Jitter prevents thundering herd when many clients retry simultaneously
- 5 retries with 100ms initial gives ~3s before first failure, ~30s total

## Alternatives Considered
- Fixed delay: Too aggressive or too slow
- Linear backoff: Doesn't back off fast enough under load
- No jitter: Causes synchronized retry storms
"
```

**What happens:**

1. Appends to `.ctx/narrative/decisions.md`, stores → `blob:dec01...`
2. Creates Mentions edges: decision → Backoff, decision → connect
3. Appends to log, creates WorkCommit, advances STAGE → `w00010...`

---

#### Step 12: Mark Task Complete and Compact

**User action:** Finish up

```bash
$ ctx add note "Completed task_0043. All tests passing. Ready for review."

$ ctx stage compact --message "feat: Add connection retry with exponential backoff

- Add core::retry::Backoff for exponential backoff with jitter
- Integrate retry logic into Client::connect()
- Add decision record documenting retry strategy

Closes task_0043"
```

**What happens during compaction:**

1. **Walk staging chain:** `w00010 → w00009 → ... → w00001`
2. **Collect all artifacts:**
   - File versions: client.rs (v2), retry/mod.rs (v1), lib.rs (v2)
   - Edge batches: eb0001...eb0005
   - Diagnostics: 2 snapshots
   - Notes: task, decision, log entries
3. **Deduplicate:** Keep only latest file versions
4. **Merge edge batches:** Combine into single curated EdgeBatch
5. **Generate summary:** (from narrative log)
   ```markdown
   ## Session Summary: retry-feature-001
   
   ### Changes
   - Created crates/core/src/retry/mod.rs (Backoff struct)
   - Modified crates/core/src/net/client.rs (added retry to connect)
   - Modified crates/core/src/lib.rs (added mod retry)
   
   ### Key Decisions
   - Exponential backoff with jitter for retry strategy
   
   ### Test Results
   - Initial failure: missing module declaration
   - Final: 15 tests passing
   ```
6. **Create canonical Commit:**
   ```rust
   Commit {
       parents: [abc123],  // previous canonical HEAD
       timestamp_unix: 1737370800,
       message: "feat: Add connection retry...",
       root_tree: tree_new...,
       edge_batches: [eb_curated...],
       narrative_refs: [
           NarrativeRef { path: "log/2026-01-20.md", blob_id: log_new... },
           NarrativeRef { path: "tasks/task_0043.md", blob_id: task_blob... },
           NarrativeRef { path: "decisions.md", blob_id: dec_blob... },
       ],
       cargo_snapshot: Some(cargo01...),
       rust_snapshot: Some(rust_new...),
       diagnostics_snapshot: Some(diag_final...),
   }
   ```
7. **Store canonical commit** → `typed:def456...`
8. **Advance refs/main** → `def456...`
9. **Reset staging:** Update STAGE → `def456...` (or delete STAGE file)

---

#### Final State

```
.ctx/
├── HEAD                    → def456... (new canonical)
├── refs/main               → def456...
├── STAGE                   → def456... (reset to canonical)
│
├── narrative/
│   ├── README.md
│   ├── decisions.md        ← updated with retry decision
│   ├── log/
│   │   ├── 2026-01-19.md
│   │   └── 2026-01-20.md   ← updated with session log
│   └── tasks/
│       └── task_0043.md    ← completed task
│
├── objects/
│   ├── ab/abc123...        ← old canonical commit
│   ├── de/def456...        ← NEW canonical commit
│   ├── d4/d4e5f6...        ← task blob
│   ├── bl/backoff01...     ← retry/mod.rs content
│   ├── cl/client02...      ← client.rs final version
│   ├── eb/eb_curated...    ← curated edge batch
│   ├── w0/w00001...        ← staging commits (will be GC'd later)
│   └── ...
│
└── index/
    └── index.redb          ← rebuilt with new data
```

---

#### What the Agent Can Now Recall

**Query 1:** "What retry strategy do we use?"

```bash
$ ctx query "retry strategy"
```

**Retrieves:**
- Decision: "Retry strategy: exponential backoff with jitter"
- Code: `core::retry::Backoff` struct definition
- Code: `Client::connect` retry loop

**Query 2:** "Why did the tests fail initially?"

```bash
$ ctx query "test failure retry"
```

**Retrieves:**
- Diagnostic: E0433 "could not find 'retry' in 'crate'"
- Narrative log: "Need to add `mod retry;` to lib.rs"
- File: lib.rs showing the fix

**Query 3:** "What files are involved in connection handling?"

```bash
$ ctx query "connection handling"
```

**Graph expansion:**
1. Seed: `Client::connect` (from query)
2. Expand via Calls: → `Backoff::new`, `Backoff::next_delay`, `TcpStream::connect`
3. Expand via Imports: → `core::retry`, `tokio::net`
4. Expand via Contains: → `client.rs`, `retry/mod.rs`

**Retrieves:**
- `crates/core/src/net/client.rs`
- `crates/core/src/retry/mod.rs`
- Decision about retry strategy
- Task context

---

#### Staging Chain Visualization

```
Canonical                     Staging Chain
─────────                     ─────────────

abc123 (old HEAD)
    │
    │                         w00001 (SessionStart)
    │                            │
    │                         w00002 (cargo metadata)
    │                            │
    │                         w00003 (read client.rs)
    │                            │
    │                         w00004 (create retry/mod.rs)
    │                            │
    │                         w00005 (modify client.rs)
    │                            │
    │                         w00006 (add relations)
    │                            │
    │                         w00007 (test FAIL)
    │                            │
    │                         w00008 (fix lib.rs)
    │                            │
    │                         w00009 (test PASS)
    │                            │
    │                         w00010 (decision + note)
    │                            │
    │        ┌───── compact ─────┘
    │        │
    ▼        ▼
def456 (new HEAD) ◄─── squashed from staging
```

After compaction, staging commits (w00001-w00010) are unreferenced and will be garbage collected after the retention period.

---

### A.6 References
2. Content-addressed storage: https://en.wikipedia.org/wiki/Content-addressable_storage
3. BLAKE3 specification: https://github.com/BLAKE3-team/BLAKE3-specs
4. Tarjan's SCC algorithm: https://en.wikipedia.org/wiki/Tarjan%27s_strongly_connected_components_algorithm
5. llama.cpp server API: https://github.com/ggerganov/llama.cpp/blob/master/examples/server/README.md
6. redb (Rust embedded database): https://github.com/cberner/redb
7. Postcard serialization: https://github.com/jamesmunns/postcard

---

*End of Document*
