# CTX Walkthrough: Building a Feature End-to-End

This document traces exactly what happens when a developer uses CTX to build a "connection retry" feature in a Rust networking crate.

---

## Initial State

The developer has a Rust workspace with this structure:

```
my-project/
├── Cargo.toml
├── crates/
│   ├── core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       └── net/
│   │           ├── mod.rs
│   │           └── client.rs
│   └── cli/
│       ├── Cargo.toml
│       └── src/
│           └── main.rs
└── .ctx/                          # Already initialized
    ├── config.toml
    ├── HEAD                       # Contains: a1b2c3d4... (initial commit)
    ├── refs/
    │   └── main                   # Contains: a1b2c3d4...
    ├── narrative/
    │   ├── README.md
    │   ├── decisions.md
    │   └── log/
    │       └── 2026-01-19.md
    ├── objects/
    │   ├── a1/
    │   │   └── b2c3d4...          # Initial commit
    │   ├── ... (other objects)
    └── index/
        └── index.redb
```

---

## Step 1: Start a New Task

**User action:**
```bash
ctx add task "Add connection retry with exponential backoff" --body "
The Client::connect() method should retry failed connections.
Requirements:
- Max 3 retries
- Exponential backoff starting at 100ms
- Add jitter to prevent thundering herd
"
```

**What happens internally:**

### 1a. Create the task file on disk

```
.ctx/narrative/tasks/task_0003.md
```

Contents:
```markdown
# Add connection retry with exponential backoff

<!-- metadata:
created: 2026-01-20T09:00:00Z
status: active
tags: [networking, reliability]
mentions: [crates/core/src/net/client.rs, core::net::Client::connect]
-->

## Requirements

The Client::connect() method should retry failed connections.
Requirements:
- Max 3 retries
- Exponential backoff starting at 100ms
- Add jitter to prevent thundering herd

## Notes

(Agent will append notes here)
```

### 1b. Store the task file as a blob

```rust
// Pseudocode of what ctx_core does:
let task_content = fs::read(".ctx/narrative/tasks/task_0003.md")?;
let blob_id = object_store.put_blob(&task_content)?;
// blob_id = hash(canonical_bytes(Blob, task_content))
// Let's say blob_id = "b1a2b3c4d5e6f7..."
```

**New object on disk:**
```
.ctx/objects/b1/a2b3c4d5e6f7...    # Task file blob (compressed)
```

### 1c. Create a Task metadata object

```rust
let task_meta = TaskMeta {
    task_id: generate_task_id("task_0003"),  // "t0003..."
    title: "Add connection retry with exponential backoff".into(),
    created_at: now_unix(),
    status: TaskStatus::Active,
    blob_id: blob_id,  // "b1a2b3c4..."
};
let meta_id = object_store.put_typed(&task_meta)?;
// meta_id = "m1a2b3..."
```

**New object on disk:**
```
.ctx/objects/m1/a2b3...            # Task metadata (postcard serialized, compressed)
```

### 1d. Create edges linking task to code

```rust
let edges = EdgeBatch {
    edges: vec![
        Edge {
            from: NodeId::Task("t0003..."),
            to: NodeId::File(file_id_of("crates/core/src/net/client.rs")),
            label: EdgeLabel::Mentions,
            evidence: Evidence {
                commit_id: current_head,
                tool: EvidenceTool::Human,
                confidence: Confidence::High,
                span: None,
                blob_id: Some(blob_id),  // The task file mentions it
            },
        },
    ],
    created_at: now_unix(),
    // Note: To find which commit introduces this batch, query which commit's
    // edge_batches field contains this EdgeBatch's ObjectId.
};
let edge_batch_id = object_store.put_typed(&edges)?;
// edge_batch_id = "e1a2b3..."
```

**New object on disk:**
```
.ctx/objects/e1/a2b3...            # EdgeBatch object
```

### 1e. Append to today's log

```
.ctx/narrative/log/2026-01-20.md
```

Contents (created or appended):
```markdown
# 2026-01-20

## 09:00 - Task Started

Started task: **Add connection retry with exponential backoff**

Target: `crates/core/src/net/client.rs` - `Client::connect()`
```

### 1f. Store log blob

```
.ctx/objects/l1/a2b3...            # Today's log blob
```

**Current object count: 4 new objects**

---

## Step 2: Start Staging Session

**User action:**
```bash
ctx stage start "Add connection retry with exponential backoff"
# Session ID auto-generated: retry-feature-001 (UUID v4)
```

**What happens:**

### 2a. Create initial WorkCommit

```rust
let work_commit = WorkCommit {
    parents: vec![],                        // First in session
    base: ObjectId::from_hex("a1b2c3d4"),  // Current main HEAD
    session_id: "retry-feature-001".into(),
    created_at: now_unix(),
    step_kind: StepKind::SessionStart,
    payload: vec![
        meta_id,        // Task metadata
        edge_batch_id,  // Edges from task
    ],
    narrative_refs: vec![
        NarrativeRef {
            path: "tasks/task_0003.md".into(),
            stream: "main".into(),
            role: "task".into(),
            blob_id: blob_id,
        },
        NarrativeRef {
            path: "log/2026-01-20.md".into(),
            stream: "main".into(),
            role: "log".into(),
            blob_id: log_blob_id,
        },
    ],
};
let work_commit_id = object_store.put_typed(&work_commit)?;
// work_commit_id = "w1a2b3..."
```

**New object:**
```
.ctx/objects/w1/a2b3...            # WorkCommit object
```

### 2b. Write staging pointer

```
.ctx/STAGE
```

Contents:
```
w1a2b3c4d5e6f7890123456789abcdef0123456789abcdef0123456789abcdef
```

**State after Step 2:**

```
.ctx/
├── HEAD                           # a1b2c3d4... (canonical, unchanged)
├── STAGE                          # w1a2b3... (staging head) ← NEW
├── refs/
│   └── main                       # a1b2c3d4... (unchanged)
├── narrative/
│   ├── tasks/
│   │   └── task_0003.md           # NEW
│   └── log/
│       └── 2026-01-20.md          # NEW/UPDATED
├── objects/
│   ├── a1/b2c3d4...               # Initial commit (existing)
│   ├── b1/a2b3...                 # Task file blob
│   ├── m1/a2b3...                 # Task metadata
│   ├── e1/a2b3...                 # EdgeBatch (task mentions)
│   ├── l1/a2b3...                 # Log blob
│   └── w1/a2b3...                 # WorkCommit (staging)
```

---

## Step 3: Agent Reads Files

The coding agent needs to understand the current code before making changes.

**Agent action (via API):**
```rust
ctx.observe_file_read("crates/core/src/net/client.rs")?;
ctx.observe_file_read("crates/core/src/net/mod.rs")?;
```

**What happens:**

### 3a. Snapshot each file

```rust
// For client.rs
let content = fs::read("crates/core/src/net/client.rs")?;
let blob_id = object_store.put_blob(&content)?;
// blob_id = "c1a2b3..."

let file_version = FileVersion {
    file_version_id: hash(file_id, blob_id),
    file_id: file_id_of("crates/core/src/net/client.rs"),
    blob_id: blob_id,
    byte_count: content.len() as u64,
    line_count: Some(count_lines(&content)),
};
let fv_id = object_store.put_typed(&file_version)?;
// fv_id = "fv1a2b3..."
```

**New objects:**
```
.ctx/objects/c1/a2b3...            # client.rs blob
.ctx/objects/fv/1a2b3...           # FileVersion for client.rs
.ctx/objects/c2/a2b3...            # mod.rs blob
.ctx/objects/fv/2a2b3...           # FileVersion for mod.rs
```

### 3b. Extract relations from read files

The system parses the Rust files and extracts:

```rust
// From client.rs:
// pub struct Client { ... }
// impl Client {
//     pub async fn connect(addr: &str) -> Result<Self, Error> { ... }
// }
// use crate::net::Connection;
// use std::time::Duration;

let edges = EdgeBatch {
    edges: vec![
        // Module defines item
        Edge {
            from: NodeId::Module(module_id_of("core::net::client")),
            to: NodeId::Item(item_id_of("core::net::client::Client")),
            label: EdgeLabel::Defines,
            evidence: Evidence { tool: EvidenceTool::Parser, confidence: Confidence::High, ... },
        },
        // Item defines method
        Edge {
            from: NodeId::Item(item_id_of("core::net::client::Client")),
            to: NodeId::Item(item_id_of("core::net::client::Client::connect")),
            label: EdgeLabel::Defines,
            evidence: Evidence { tool: EvidenceTool::Parser, confidence: Confidence::High, ... },
        },
        // Import edge
        Edge {
            from: NodeId::Module(module_id_of("core::net::client")),
            to: NodeId::Item(item_id_of("core::net::Connection")),
            label: EdgeLabel::Imports,
            evidence: Evidence { tool: EvidenceTool::Parser, confidence: Confidence::High, ... },
        },
        // ... more edges
    ],
    created_at: now_unix(),
};
let edge_batch_id = object_store.put_typed(&edges)?;
// edge_batch_id = "e2a2b3..."
```

**New object:**
```
.ctx/objects/e2/a2b3...            # EdgeBatch (extracted relations)
```

### 3c. Create StepRecord

```rust
let step = StepRecord {
    step_id: generate_step_id(),
    kind: StepKind::FileRead,
    timestamp: now_unix(),
    files_read: vec![
        "crates/core/src/net/client.rs".into(),
        "crates/core/src/net/mod.rs".into(),
    ],
    files_written: vec![],
    summary: "Read client.rs and mod.rs to understand current implementation".into(),
};
let step_id = object_store.put_typed(&step)?;
```

**New object:**
```
.ctx/objects/s1/a2b3...            # StepRecord
```

### 3d. Advance staging pointer

```rust
let work_commit = WorkCommit {
    parents: vec![previous_staging_head],   // w1a2b3...
    base: canonical_head,                    // a1b2c3d4...
    session_id: "retry-feature-001".into(),
    created_at: now_unix(),
    step_kind: StepKind::FileRead,
    payload: vec![
        fv_id_client,    // FileVersion for client.rs
        fv_id_mod,       // FileVersion for mod.rs
        edge_batch_id,   // Extracted edges
        step_id,         // StepRecord
    ],
    narrative_refs: vec![],  // No narrative changes this step
};
let new_staging_head = object_store.put_typed(&work_commit)?;
// new_staging_head = "w2a2b3..."

// Atomic pointer update
fs::write(".ctx/STAGE.tmp", new_staging_head.as_hex())?;
fs::rename(".ctx/STAGE.tmp", ".ctx/STAGE")?;
```

**Updated:**
```
.ctx/STAGE                         # Now contains: w2a2b3...
```

---

## Step 4: Agent Writes Code

The agent implements the retry logic.

**Agent action (via API):**
```rust
let new_code = r#"
use std::time::Duration;
use tokio::time::sleep;
use rand::Rng;

impl Client {
    pub async fn connect(addr: &str) -> Result<Self, Error> {
        self.connect_with_retry(addr, 3, Duration::from_millis(100)).await
    }
    
    async fn connect_with_retry(
        &self,
        addr: &str,
        max_retries: u32,
        base_delay: Duration,
    ) -> Result<Self, Error> {
        let mut attempt = 0;
        loop {
            match self.try_connect(addr).await {
                Ok(conn) => return Ok(conn),
                Err(e) if attempt < max_retries => {
                    let delay = self.calculate_backoff(attempt, base_delay);
                    sleep(delay).await;
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }
    
    fn calculate_backoff(&self, attempt: u32, base: Duration) -> Duration {
        let exponential = base * 2_u32.pow(attempt);
        let jitter = rand::thread_rng().gen_range(0..=50);
        exponential + Duration::from_millis(jitter)
    }
}
"#;

ctx.observe_file_write("crates/core/src/net/client.rs", new_code)?;
```

**What happens:**

### 4a. Store new file content

```rust
let blob_id = object_store.put_blob(new_code.as_bytes())?;
// blob_id = "c3a2b3..." (different from before because content changed)

let file_version = FileVersion {
    file_version_id: hash(file_id, blob_id),
    file_id: file_id_of("crates/core/src/net/client.rs"),
    blob_id: blob_id,
    byte_count: new_code.len() as u64,
    line_count: Some(count_lines(new_code)),
};
let fv_id = object_store.put_typed(&file_version)?;
// fv_id = "fv3a2b3..."
```

### 4b. Extract new relations

New edges discovered:
- `Client::connect` → Calls → `Client::connect_with_retry`
- `Client::connect_with_retry` → Calls → `Client::try_connect`
- `Client::connect_with_retry` → Calls → `Client::calculate_backoff`
- `client` module → Imports → `rand::Rng`
- `client` module → Imports → `tokio::time::sleep`

```rust
let edges = EdgeBatch {
    edges: vec![
        Edge {
            from: NodeId::Item(item_id_of("core::net::client::Client::connect")),
            to: NodeId::Item(item_id_of("core::net::client::Client::connect_with_retry")),
            label: EdgeLabel::Calls,
            evidence: Evidence { tool: EvidenceTool::Parser, confidence: Confidence::High, ... },
        },
        // ... more edges
    ],
    ...
};
let edge_batch_id = object_store.put_typed(&edges)?;
// edge_batch_id = "e3a2b3..."
```

### 4c. Update narrative log

Append to `.ctx/narrative/log/2026-01-20.md`:

```markdown
## 09:15 - Implementation

Added retry logic to `Client::connect()`:
- New method `connect_with_retry(addr, max_retries, base_delay)`
- New method `calculate_backoff(attempt, base)` with jitter
- `connect()` now delegates to `connect_with_retry` with defaults

New dependencies: `rand`, `tokio::time`
```

### 4d. Create WorkCommit and advance staging

```rust
let work_commit = WorkCommit {
    parents: vec![ObjectId::from_hex("w2a2b3...")],
    base: canonical_head,
    session_id: "retry-feature-001".into(),
    created_at: now_unix(),
    step_kind: StepKind::FileWrite,
    payload: vec![
        fv_id,           // New FileVersion
        edge_batch_id,   // New call edges
        step_id,         // StepRecord
    ],
    narrative_refs: vec![
        NarrativeRef {
            path: "log/2026-01-20.md".into(),
            stream: "main".into(),
            role: "log".into(),
            blob_id: updated_log_blob_id,
        },
    ],
};
let new_staging_head = object_store.put_typed(&work_commit)?;
// new_staging_head = "w3a2b3..."
```

**New objects:**
```
.ctx/objects/c3/a2b3...            # New client.rs blob
.ctx/objects/fv/3a2b3...           # New FileVersion
.ctx/objects/e3/a2b3...            # EdgeBatch (call edges)
.ctx/objects/s2/a2b3...            # StepRecord
.ctx/objects/l2/a2b3...            # Updated log blob
.ctx/objects/w3/a2b3...            # WorkCommit
```

**Updated:**
```
.ctx/STAGE                         # Now contains: w3a2b3...
```

---

## Step 5: Agent Runs Tests

**Agent action:**
```rust
let output = ctx.observe_command("cargo test -p core", exit_code=1, stdout, stderr)?;
```

The tests fail! Output includes:
```
error[E0433]: failed to resolve: use of undeclared crate or module `rand`
  --> crates/core/src/net/client.rs:42:9
   |
42 |         rand::thread_rng().gen_range(0..=50)
   |         ^^^^ use of undeclared crate or module `rand`
```

**What happens:**

### 5a. Store command output

```rust
let output_blob_id = object_store.put_blob(stderr.as_bytes())?;
// output_blob_id = "o1a2b3..."
```

### 5b. Parse diagnostics

```rust
let diagnostics = DiagnosticsSnapshot {
    tool: DiagnosticTool::Rustc,
    timestamp: now_unix(),
    diagnostics: vec![
        Diagnostic {
            severity: Severity::Error,
            code: Some("E0433".into()),
            message_blob_id: object_store.put_blob(b"failed to resolve: use of undeclared crate or module `rand`")?,
            spans: vec![
                Span {
                    file_id: file_id_of("crates/core/src/net/client.rs"),
                    start_byte: 1250,
                    end_byte: 1254,
                    start_line: Some(42),
                    start_col: Some(9),
                    ...
                },
            ],
            related_blob_id: None,
        },
    ],
};
let diag_id = object_store.put_typed(&diagnostics)?;
// diag_id = "d1a2b3..."
```

### 5c. Create edges linking failure to code

```rust
let edges = EdgeBatch {
    edges: vec![
        Edge {
            from: NodeId::Diagnostic(diag_id),
            to: NodeId::File(file_id_of("crates/core/src/net/client.rs")),
            label: EdgeLabel::References,
            evidence: Evidence {
                tool: EvidenceTool::Rustc,
                confidence: Confidence::High,
                span: Some(Span { start_line: 42, ... }),
                ...
            },
        },
    ],
    ...
};
```

### 5d. Advance staging with failure information

```rust
let work_commit = WorkCommit {
    parents: vec![ObjectId::from_hex("w3a2b3...")],
    base: canonical_head,
    session_id: "retry-feature-001".into(),
    created_at: now_unix(),
    step_kind: StepKind::CommandRun,
    payload: vec![
        output_blob_id,  // Raw output
        diag_id,         // Parsed diagnostics
        edge_batch_id,   // Diagnostic edges
        step_id,         // StepRecord
    ],
    narrative_refs: vec![],
};
// new_staging_head = "w4a2b3..."
```

**New objects:**
```
.ctx/objects/o1/a2b3...            # Command output blob
.ctx/objects/d1/a2b3...            # DiagnosticsSnapshot
.ctx/objects/e4/a2b3...            # EdgeBatch (diagnostic edges)
.ctx/objects/w4/a2b3...            # WorkCommit
```

---

## Step 6: Agent Fixes the Error

**Agent action:**
```rust
// Add rand to Cargo.toml
ctx.observe_file_write("crates/core/Cargo.toml", updated_cargo_toml)?;

// Run tests again
ctx.observe_command("cargo test -p core", exit_code=0, stdout, stderr)?;
```

Tests pass now.

**What happens:**

Similar to before - new FileVersion for Cargo.toml, new WorkCommit, staging advances.

```
.ctx/STAGE                         # Now contains: w6a2b3...
```

---

## Step 7: Agent Adds Decision Record

The agent documents the design decision.

**Agent action:**
```rust
ctx.observe_note("decision: Chose exponential backoff with jitter for retry strategy. \
    Alternatives considered: fixed delay (rejected - thundering herd), \
    linear backoff (rejected - too slow for transient failures). \
    Jitter range 0-50ms chosen to spread reconnection attempts.")?;
```

**What happens:**

### 7a. Append to decisions.md

```markdown
## 2026-01-20: Connection Retry Strategy

**Decision:** Exponential backoff with jitter

**Context:** Need to handle transient connection failures in `Client::connect()`

**Alternatives considered:**
- Fixed delay: Rejected - causes thundering herd on recovery
- Linear backoff: Rejected - too slow to recover from transient failures

**Details:**
- Base delay: 100ms
- Max retries: 3
- Jitter: 0-50ms random addition

**Affects:** `crates/core/src/net/client.rs`
```

### 7b. Create edges

```rust
let edges = EdgeBatch {
    edges: vec![
        Edge {
            from: NodeId::Decision(decision_id),
            to: NodeId::File(file_id_of("crates/core/src/net/client.rs")),
            label: EdgeLabel::Mentions,
            evidence: Evidence { tool: EvidenceTool::Human, ... },
        },
        Edge {
            from: NodeId::Decision(decision_id),
            to: NodeId::Item(item_id_of("core::net::client::Client::connect_with_retry")),
            label: EdgeLabel::Mentions,
            evidence: Evidence { tool: EvidenceTool::Human, ... },
        },
    ],
    ...
};
```

---

## Step 8: Compact Session

The feature is complete. Time to compact staging into a canonical commit.

**User action:**
```bash
ctx stage compact --message "Add connection retry with exponential backoff"
```

**What happens:**

### 8a. Walk staging chain

```rust
// Current staging head: w7a2b3...
// Walk parents: w7 → w6 → w5 → w4 → w3 → w2 → w1
// Base commit: a1b2c3d4...

let staging_chain = walk_staging_chain(staging_head, base)?;
// Returns all WorkCommits in the session
```

### 8b. Collect and deduplicate artifacts

```rust
// Collect all FileVersions - keep only latest per file
let final_file_versions = dedupe_file_versions(staging_chain)?;
// Result: latest client.rs, latest Cargo.toml

// Collect all EdgeBatches
let all_edges = collect_edges(staging_chain)?;

// Merge into single curated EdgeBatch
let curated_edges = EdgeBatch {
    edges: dedupe_and_filter_edges(all_edges)?,
    created_at: now_unix(),
};
let curated_edge_batch_id = object_store.put_typed(&curated_edges)?;
```

### 8c. Generate summary

Either via LLM or heuristic:

```rust
let summary = generate_session_summary(staging_chain)?;
// "Added connection retry logic with exponential backoff and jitter.
//  Modified: crates/core/src/net/client.rs, crates/core/Cargo.toml
//  New methods: connect_with_retry, calculate_backoff
//  New dependency: rand"
```

### 8d. Update task status

```markdown
# task_0003.md

<!-- metadata:
created: 2026-01-20T09:00:00Z
status: completed                    ← CHANGED
completed_at: 2026-01-20T10:30:00Z   ← ADDED
...
-->
```

### 8e. Finalize narrative log

Append completion entry to log:

```markdown
## 10:30 - Task Completed

Completed: **Add connection retry with exponential backoff**

Summary:
- Added `connect_with_retry` method with exponential backoff
- Added `calculate_backoff` helper with jitter
- Added `rand` dependency
- All tests passing
```

### 8f. Create canonical Commit

```rust
let commit = Commit {
    parents: vec![ObjectId::from_hex("a1b2c3d4...")],  // Previous canonical HEAD
    timestamp_unix: now_unix(),
    message: "Add connection retry with exponential backoff".into(),
    
    root_tree: build_tree_with_updated_files(final_file_versions)?,
    
    edge_batches: vec![curated_edge_batch_id],
    
    narrative_refs: vec![
        NarrativeRef {
            path: "tasks/task_0003.md".into(),
            stream: "main".into(),
            role: "task".into(),
            blob_id: final_task_blob_id,
        },
        NarrativeRef {
            path: "log/2026-01-20.md".into(),
            stream: "main".into(),
            role: "log".into(),
            blob_id: final_log_blob_id,
        },
        NarrativeRef {
            path: "decisions.md".into(),
            stream: "main".into(),
            role: "decision".into(),
            blob_id: final_decisions_blob_id,
        },
    ],
    
    cargo_snapshot: Some(cargo_snapshot_id),
    rust_snapshot: Some(rust_snapshot_id),
    diagnostics_snapshot: None,  // Tests pass, no diagnostics to preserve
};
let new_commit_id = object_store.put_typed(&commit)?;
// new_commit_id = "cc1a2b3..."
```

### 8g. Advance refs/main

```rust
// Atomic update
fs::write(".ctx/refs/main.tmp", new_commit_id.as_hex())?;
fs::rename(".ctx/refs/main.tmp", ".ctx/refs/main")?;

// Update HEAD
fs::write(".ctx/HEAD.tmp", new_commit_id.as_hex())?;
fs::rename(".ctx/HEAD.tmp", ".ctx/HEAD")?;
```

### 8h. Reset staging

```rust
// Point staging to new canonical HEAD (or delete STAGE file)
fs::write(".ctx/STAGE.tmp", new_commit_id.as_hex())?;
fs::rename(".ctx/STAGE.tmp", ".ctx/STAGE")?;
// Or: fs::remove_file(".ctx/STAGE")?;
```

---

## Final State

```
.ctx/
├── config.toml
├── HEAD                           # cc1a2b3... (new canonical commit)
├── STAGE                          # cc1a2b3... (reset to canonical)
├── refs/
│   └── main                       # cc1a2b3... (advanced)
│
├── narrative/
│   ├── README.md
│   ├── decisions.md               # Updated with retry decision
│   ├── log/
│   │   ├── 2026-01-19.md
│   │   └── 2026-01-20.md          # Full session log
│   └── tasks/
│       └── task_0003.md           # Marked completed
│
├── objects/
│   │
│   │ # === Original objects ===
│   ├── a1/b2c3d4...               # Initial commit
│   │
│   │ # === Session artifacts (kept for audit/GC later) ===
│   ├── w1/a2b3...                 # WorkCommit: session start
│   ├── w2/a2b3...                 # WorkCommit: file reads
│   ├── w3/a2b3...                 # WorkCommit: implementation
│   ├── w4/a2b3...                 # WorkCommit: failed tests
│   ├── w5/a2b3...                 # WorkCommit: fix Cargo.toml
│   ├── w6/a2b3...                 # WorkCommit: passing tests
│   ├── w7/a2b3...                 # WorkCommit: decision record
│   │
│   │ # === File content blobs ===
│   ├── c1/a2b3...                 # client.rs (original)
│   ├── c3/a2b3...                 # client.rs (with retry)
│   ├── cg/1a2b3...                # Cargo.toml (with rand)
│   │
│   │ # === Curated canonical objects ===
│   ├── cc/1a2b3...                # Canonical Commit ← NEW
│   ├── tr/1a2b3...                # Root Tree ← NEW
│   ├── ec/1a2b3...                # Curated EdgeBatch ← NEW
│   │
│   │ # === Narrative snapshots ===
│   ├── nt/1a2b3...                # task_0003.md final
│   ├── nl/1a2b3...                # log/2026-01-20.md final
│   ├── nd/1a2b3...                # decisions.md final
│   │
│   └── ... (various other objects)
│
└── index/
    └── index.redb                 # Rebuilt with new data
```

---

## What Retrieval Looks Like Later

A week later, someone asks: "How does connection retry work?"

**Query:**
```bash
ctx query "How does connection retry work in the Client?"
```

**Retrieval process:**

1. **Seed identification:**
   - Parse query → mentions "connection", "retry", "Client"
   - Index lookup → finds `core::net::client::Client`
   - Index lookup → finds `connect_with_retry` (fuzzy match on "retry")

2. **Graph expansion:**
   - From `Client` → follow Defines → `connect`, `connect_with_retry`, `calculate_backoff`
   - From `connect_with_retry` → follow Calls → `try_connect`, `calculate_backoff`
   - From decision record → follow Mentions → `client.rs`

3. **Content retrieval:**
   - Load `client.rs` FileVersion blob
   - Load decision entry from `decisions.md`
   - Load relevant log entries

4. **PromptPack result:**

```json
{
  "task": "How does connection retry work in the Client?",
  "head_commit": "cc1a2b3...",
  "retrieved": [
    {
      "title": "File: crates/core/src/net/client.rs",
      "snippet": "impl Client {\n    pub async fn connect(...) -> Result<Self, Error> {\n        self.connect_with_retry(addr, 3, Duration::from_millis(100)).await\n    }\n    \n    async fn connect_with_retry(...) {\n        // ... full implementation\n    }\n    \n    fn calculate_backoff(...) {\n        // ... jitter logic\n    }\n}",
      "relevance_score": 0.95
    },
    {
      "title": "Decision: Connection Retry Strategy",
      "snippet": "**Decision:** Exponential backoff with jitter\n\n**Alternatives considered:**\n- Fixed delay: Rejected - causes thundering herd...",
      "relevance_score": 0.88
    }
  ],
  "graph_context": {
    "seed_nodes": ["core::net::client::Client", "core::net::client::Client::connect_with_retry"],
    "expanded_nodes": ["...", "calculate_backoff", "try_connect"],
    "expansion_depth": 2
  },
  "recent_narrative": "## 2026-01-20\n\n### 10:30 - Task Completed\n\nCompleted: **Add connection retry with exponential backoff**..."
}
```

The agent now has full context about:
- The implementation details
- Why this approach was chosen
- What alternatives were rejected
- When it was implemented

**Without CTX**, the agent would have to:
- Re-read the entire file
- Guess at the design rationale
- Not know about rejected alternatives
- Potentially suggest the same rejected approaches

---

## Object Count Summary

| Phase | New Objects | Cumulative |
|-------|-------------|------------|
| Initial state | - | ~10 |
| Task creation | 4 | 14 |
| Session start | 1 | 15 |
| File reads | 6 | 21 |
| Implementation | 6 | 27 |
| Failed tests | 4 | 31 |
| Fix + retest | 4 | 35 |
| Decision record | 3 | 38 |
| Compaction | 5 | 43 |

Most objects are small (< 1KB compressed). The largest are file content blobs.

After GC runs (keeping only objects reachable from refs/main), the staging WorkCommits and intermediate blobs can be removed, bringing count down to ~25 essential objects.
