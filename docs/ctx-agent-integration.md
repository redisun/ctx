# CTX Agent Integration Specification

## Overview

CTX is primarily a **library**, not a CLI tool. The coding agent imports `ctx_core` and calls it programmatically at specific lifecycle points. The CLI exists for debugging and manual operations, but users never need to touch it during normal agent operation.

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Coding Agent                                 │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │                    Agent Orchestrator                          │ │
│  │                                                                │ │
│  │  on_task_received() ──────► ctx.start_session()               │ │
│  │  on_planning_complete() ──► ctx.observe_plan()                │ │
│  │  on_file_read() ──────────► ctx.observe_file_read()           │ │
│  │  on_file_write() ─────────► ctx.observe_file_write()          │ │
│  │  on_command_run() ────────► ctx.observe_command()             │ │
│  │  on_step_complete() ──────► ctx.flush_step()                  │ │
│  │  on_task_complete() ──────► ctx.compact_session()             │ │
│  │  before_llm_call() ───────► ctx.build_pack()                  │ │
│  │                                                                │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                              │                                       │
│                              ▼                                       │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │                      ctx_core (library)                        │ │
│  │                                                                │ │
│  │  • CtxRepo::open(path)                                        │ │
│  │  • Session management                                          │ │
│  │  • Object storage                                              │ │
│  │  • Graph operations                                            │ │
│  │  • Prompt pack compilation                                     │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                              │                                       │
└──────────────────────────────┼───────────────────────────────────────┘
                               │
                               ▼
                         .ctx/ directory
```

---

## Library API

### Core Handle

```rust
use ctx_core::{CtxRepo, Session, PromptPack, RetrievalConfig};

// Open or initialize a context repo
let ctx = CtxRepo::open("./my-project")?;
// or
let ctx = CtxRepo::init("./my-project", Config::default())?;
```

### Session Lifecycle

```rust
impl CtxRepo {
    /// Start a new session, returns session handle
    /// Called: on_task_received
    pub fn start_session(&self, task: &str) -> Result<Session>;
    
    /// Get active session if one exists
    pub fn active_session(&self) -> Option<&Session>;
    
    /// Compact session into canonical commit
    /// Called: on_task_complete
    pub fn compact_session(&self, message: &str) -> Result<CommitId>;
    
    /// Abort session, discard staging
    pub fn abort_session(&self) -> Result<()>;
}
```

### Observation API

The agent "observes" what it does, and CTX decides what to persist.

```rust
impl Session {
    /// Record that agent read a file (for reasoning, not just listing)
    /// Called: on_file_read (when file content enters the prompt)
    pub fn observe_file_read(&mut self, path: &str) -> Result<()>;
    
    /// Record that agent wrote/modified a file
    /// Called: on_file_write
    pub fn observe_file_write(&mut self, path: &str, content: &[u8]) -> Result<()>;
    
    /// Record command execution and output
    /// Called: on_command_run
    pub fn observe_command(&mut self, cmd: &str, output: &CommandOutput) -> Result<()>;
    
    /// Record a plan or decision
    /// Called: on_planning_complete
    pub fn observe_plan(&mut self, plan: &Plan) -> Result<()>;
    
    /// Record a note (goes to narrative log)
    pub fn observe_note(&mut self, note: &str) -> Result<()>;
    
    /// Record discovered relationships
    pub fn observe_relations(&mut self, edges: &[Edge]) -> Result<()>;
    
    /// Flush current step to staging (crash safety checkpoint)
    /// Called: on_step_complete
    pub fn flush_step(&mut self) -> Result<()>;
}

pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration_ms: u64,
}

pub struct Plan {
    pub summary: String,
    pub steps: Vec<String>,
    pub constraints: Vec<String>,
    pub alternatives_rejected: Vec<(String, String)>, // (alternative, reason)
}
```

### Retrieval API

```rust
impl CtxRepo {
    /// Build a prompt pack for the given query
    /// Called: before_llm_call
    pub fn build_pack(&self, query: &str, config: &RetrievalConfig) -> Result<PromptPack>;
    
    /// Rebuild indexes from objects
    pub fn rebuild_index(&self) -> Result<()>;
}

pub struct RetrievalConfig {
    pub token_budget: u32,
    pub expansion_depth: u32,
    pub include_narrative: bool,
    pub min_confidence: Confidence,
}
```

---

## Agent Lifecycle Hooks

### Hook Definitions

```rust
/// Trait that the agent orchestrator implements
pub trait AgentLifecycle {
    /// User has given the agent a task
    fn on_task_received(&mut self, task: &str);
    
    /// Agent has finished planning, ready to execute
    fn on_planning_complete(&mut self, plan: &Plan);
    
    /// Agent is about to read a file for reasoning
    fn on_file_read(&mut self, path: &str);
    
    /// Agent has written/modified a file
    fn on_file_write(&mut self, path: &str, content: &[u8]);
    
    /// Agent has executed a command
    fn on_command_run(&mut self, cmd: &str, output: &CommandOutput);
    
    /// Agent has completed one reasoning step
    fn on_step_complete(&mut self);
    
    /// Agent needs to call the LLM
    fn before_llm_call(&mut self, query: &str) -> PromptPack;
    
    /// Agent has received LLM response
    fn on_llm_response(&mut self, response: &str);
    
    /// Task is complete
    fn on_task_complete(&mut self, summary: &str);
    
    /// Task failed or was cancelled
    fn on_task_abort(&mut self, reason: &str);
}
```

### Reference Implementation

```rust
pub struct CtxAwareAgent {
    ctx: CtxRepo,
    session: Option<Session>,
    llm_client: LlmClient,
    // ... other agent state
}

impl AgentLifecycle for CtxAwareAgent {
    fn on_task_received(&mut self, task: &str) {
        // Start a CTX session
        self.session = Some(self.ctx.start_session(task).unwrap());
        
        // Task is automatically recorded in narrative
        // Edges linking task to mentioned files are created
    }
    
    fn on_planning_complete(&mut self, plan: &Plan) {
        if let Some(session) = &mut self.session {
            // Record the plan - this is a decision point
            session.observe_plan(plan).unwrap();
            
            // Plan goes into narrative log
            // Decision edges are created
        }
    }
    
    fn on_file_read(&mut self, path: &str) {
        if let Some(session) = &mut self.session {
            // Only called when file content is actually used for reasoning
            // Not called for directory listings or file existence checks
            session.observe_file_read(path).unwrap();
            
            // File is snapshotted
            // Defines/Imports edges are extracted
        }
    }
    
    fn on_file_write(&mut self, path: &str, content: &[u8]) {
        if let Some(session) = &mut self.session {
            session.observe_file_write(path, content).unwrap();
            
            // New file version is stored
            // Relations are re-extracted
            // Narrative log is updated
        }
    }
    
    fn on_command_run(&mut self, cmd: &str, output: &CommandOutput) {
        if let Some(session) = &mut self.session {
            session.observe_command(cmd, output).unwrap();
            
            // Output is stored (especially if non-zero exit)
            // Diagnostics are parsed and linked to code
        }
    }
    
    fn on_step_complete(&mut self) {
        if let Some(session) = &mut self.session {
            // Checkpoint for crash safety
            session.flush_step().unwrap();
            
            // Staging pointer is advanced
            // All artifacts since last flush are committed to staging
        }
    }
    
    fn before_llm_call(&mut self, query: &str) -> PromptPack {
        // Build context-aware prompt
        let config = RetrievalConfig {
            token_budget: 16000,
            expansion_depth: 2,
            include_narrative: true,
            min_confidence: Confidence::Medium,
        };
        
        self.ctx.build_pack(query, &config).unwrap()
    }
    
    fn on_llm_response(&mut self, response: &str) {
        if let Some(session) = &mut self.session {
            // Optionally store LLM response for audit
            // Parse response for any decisions or notes to extract
        }
    }
    
    fn on_task_complete(&mut self, summary: &str) {
        if let Some(session) = &mut self.session {
            // Compact staging into canonical commit
            self.ctx.compact_session(summary).unwrap();
        }
        self.session = None;
    }
    
    fn on_task_abort(&mut self, reason: &str) {
        if let Some(session) = &mut self.session {
            // Record why task was aborted
            session.observe_note(&format!("Task aborted: {}", reason)).unwrap();
            
            // Still compact - the partial work is valuable
            self.ctx.compact_session(&format!("Aborted: {}", reason)).unwrap();
        }
        self.session = None;
    }
}
```

---

## Hook Trigger Points

Here's exactly when each hook fires during a typical agent loop:

```
User: "Add retry logic to the connect function"
                    │
                    ▼
            ┌───────────────┐
            │ on_task_      │ ─── ctx.start_session("Add retry logic...")
            │ received()    │     → Creates task in narrative
            └───────┬───────┘     → Starts staging session
                    │
                    ▼
            ┌───────────────┐
            │ before_llm_   │ ─── ctx.build_pack("understand current code")
            │ call()        │     → Returns PromptPack with relevant context
            └───────┬───────┘
                    │
                    ▼
              [LLM thinks]
                    │
                    ▼
            ┌───────────────┐
            │ on_planning_  │ ─── ctx.observe_plan(plan)
            │ complete()    │     → Records plan in narrative
            └───────┬───────┘     → Creates decision edges
                    │
                    ▼
        ┌───────────────────────┐
        │   Agent Execute Loop   │
        │                        │
        │  ┌─────────────────┐  │
        │  │ on_file_read()  │──┼── ctx.observe_file_read("src/net/client.rs")
        │  └────────┬────────┘  │   → Snapshots file, extracts relations
        │           │           │
        │           ▼           │
        │  ┌─────────────────┐  │
        │  │ before_llm_     │──┼── ctx.build_pack("implement retry")
        │  │ call()          │  │   → PromptPack includes read files
        │  └────────┬────────┘  │
        │           │           │
        │     [LLM generates]   │
        │           │           │
        │           ▼           │
        │  ┌─────────────────┐  │
        │  │ on_file_write() │──┼── ctx.observe_file_write("src/net/client.rs", code)
        │  └────────┬────────┘  │   → Stores new version, extracts edges
        │           │           │
        │           ▼           │
        │  ┌─────────────────┐  │
        │  │ on_command_     │──┼── ctx.observe_command("cargo test", output)
        │  │ run()           │  │   → Stores output, parses diagnostics
        │  └────────┬────────┘  │
        │           │           │
        │           ▼           │
        │  ┌─────────────────┐  │
        │  │ on_step_        │──┼── ctx.flush_step()
        │  │ complete()      │  │   → Advances staging pointer
        │  └────────┬────────┘  │
        │           │           │
        │     [loop if needed]  │
        │                        │
        └───────────┬────────────┘
                    │
                    ▼
            ┌───────────────┐
            │ on_task_      │ ─── ctx.compact_session("Added retry logic")
            │ complete()    │     → Squashes staging → canonical commit
            └───────────────┘     → Updates refs/main
```

---

## What Triggers What

| Agent Event | CTX Operation | What Gets Stored |
|-------------|---------------|------------------|
| User gives task | `start_session(task)` | Task note, narrative entry, session WorkCommit |
| Agent makes plan | `observe_plan(plan)` | Decision record, constraint edges |
| Agent reads file (for reasoning) | `observe_file_read(path)` | FileVersion, Defines/Imports edges |
| Agent writes file | `observe_file_write(path, content)` | FileVersion, updated edges, narrative entry |
| Agent runs command | `observe_command(cmd, output)` | Output blob, DiagnosticsSnapshot, diagnostic edges |
| Agent completes step | `flush_step()` | WorkCommit advancing staging pointer |
| Before LLM call | `build_pack(query)` | Nothing stored (read-only retrieval) |
| Task complete | `compact_session(summary)` | Canonical Commit, curated edges, final narrative |

---

## Automatic vs Manual Operations

### Automatic (Agent Managed)

These happen without user intervention:

| Operation | Trigger |
|-----------|---------|
| Session start | Task received |
| File snapshots | File read/write during reasoning |
| Edge extraction | File snapshot |
| Diagnostic parsing | Command with non-zero exit |
| Staging advancement | Step completion |
| Session compaction | Task completion |
| Narrative updates | Various lifecycle events |

### Semi-Automatic (Agent Prompted)

The agent decides when to do these:

| Operation | When Agent Does It |
|-----------|-------------------|
| Record decision | After making architectural choice |
| Add note | When discovering something important |
| Create task | When identifying follow-up work |

### Manual (User/Developer)

These are for debugging and maintenance:

| Operation | CLI Command | When Used |
|-----------|-------------|-----------|
| Rebuild index | `ctx rebuild` | After corruption or manual edits |
| Garbage collection | `ctx gc` | Disk space cleanup |
| Inspect objects | `ctx debug cat <id>` | Debugging - pretty-prints Blobs, Commits, Trees, EdgeBatches, WorkCommits |
| View history | `ctx debug history` | Understanding past work |
| Export graph | `ctx debug graph` | Visualization |

---

## Integration Example: Full Agent Implementation

```rust
use ctx_core::{CtxRepo, Session, PromptPack, RetrievalConfig};
use llm_client::{LlmClient, Message};

pub struct CodingAgent {
    ctx: CtxRepo,
    llm: LlmClient,
    session: Option<Session>,
}

impl CodingAgent {
    pub fn new(repo_path: &str, llm_endpoint: &str) -> Result<Self> {
        Ok(Self {
            ctx: CtxRepo::open(repo_path)?,
            llm: LlmClient::new(llm_endpoint),
            session: None,
        })
    }
    
    /// Main entry point - user gives a task
    pub async fn run_task(&mut self, task: &str) -> Result<String> {
        // === LIFECYCLE: on_task_received ===
        self.session = Some(self.ctx.start_session(task)?);
        
        // === LIFECYCLE: before_llm_call (planning) ===
        let pack = self.ctx.build_pack(task, &RetrievalConfig::default())?;
        let plan = self.plan_task(&pack, task).await?;
        
        // === LIFECYCLE: on_planning_complete ===
        self.session.as_mut().unwrap().observe_plan(&plan)?;
        
        // Execute the plan
        let result = self.execute_plan(&plan).await;
        
        // === LIFECYCLE: on_task_complete or on_task_abort ===
        match &result {
            Ok(summary) => {
                self.ctx.compact_session(summary)?;
            }
            Err(e) => {
                self.session.as_mut().unwrap()
                    .observe_note(&format!("Failed: {}", e))?;
                self.ctx.compact_session(&format!("Failed: {}", e))?;
            }
        }
        self.session = None;
        
        result
    }
    
    async fn plan_task(&self, pack: &PromptPack, task: &str) -> Result<Plan> {
        let prompt = format!(
            "{}\n\n## Task\n{}\n\nCreate a plan to accomplish this task.",
            pack.to_prompt_string(),
            task
        );
        
        let response = self.llm.complete(&prompt).await?;
        Plan::parse(&response)
    }
    
    async fn execute_plan(&mut self, plan: &Plan) -> Result<String> {
        for step in &plan.steps {
            self.execute_step(step).await?;
            
            // === LIFECYCLE: on_step_complete ===
            self.session.as_mut().unwrap().flush_step()?;
        }
        
        Ok(plan.summary.clone())
    }
    
    async fn execute_step(&mut self, step: &str) -> Result<()> {
        // === LIFECYCLE: before_llm_call ===
        let pack = self.ctx.build_pack(step, &RetrievalConfig::default())?;
        
        let prompt = format!(
            "{}\n\n## Current Step\n{}\n\nExecute this step.",
            pack.to_prompt_string(),
            step
        );
        
        let response = self.llm.complete(&prompt).await?;
        
        // Parse response for actions
        for action in parse_actions(&response) {
            match action {
                Action::ReadFile(path) => {
                    // === LIFECYCLE: on_file_read ===
                    self.session.as_mut().unwrap()
                        .observe_file_read(&path)?;
                }
                Action::WriteFile(path, content) => {
                    // Actually write the file
                    fs::write(&path, &content)?;
                    
                    // === LIFECYCLE: on_file_write ===
                    self.session.as_mut().unwrap()
                        .observe_file_write(&path, content.as_bytes())?;
                }
                Action::RunCommand(cmd) => {
                    // Actually run the command
                    let output = run_command(&cmd)?;
                    
                    // === LIFECYCLE: on_command_run ===
                    self.session.as_mut().unwrap()
                        .observe_command(&cmd, &output)?;
                }
            }
        }
        
        Ok(())
    }
}
```

---

## Configuration: What Gets Observed

The agent can configure what CTX captures:

```rust
pub struct ObservationConfig {
    /// Snapshot files on read?
    pub snapshot_on_read: bool,           // default: true
    
    /// Extract relations from read files?
    pub extract_on_read: bool,            // default: true
    
    /// Store all command output or only failures?
    pub store_all_command_output: bool,   // default: false
    
    /// Parse diagnostics from compiler output?
    pub parse_diagnostics: bool,          // default: true
    
    /// Auto-flush after each observation?
    pub auto_flush: bool,                 // default: false
    
    /// Minimum severity to store diagnostics
    pub min_diagnostic_severity: Severity, // default: Warning
}

impl Session {
    pub fn with_config(config: ObservationConfig) -> Self;
}
```

---

## CLI: For Debugging Only

The CLI wraps the library for manual operations:

```bash
# These are for developers debugging CTX, not for normal agent operation

# Manually inspect what the agent stored
ctx debug history --limit 5
ctx debug cat a1b2c3d4...

# Manually rebuild if something went wrong
ctx rebuild --full

# Check integrity
ctx debug verify

# Export for visualization
ctx debug graph --format dot > graph.dot

# Manual garbage collection
ctx gc --dry-run
ctx gc

# Emergency: abort stuck session
ctx stage abort

# Emergency: manual compaction
ctx stage compact --message "Manual compaction"
```

The user **never** runs these during normal operation. The agent handles everything through the library API.
