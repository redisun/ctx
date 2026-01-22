# CTX PRD Extension: Session Boundaries and Lifecycle

**Version:** 1.1.0  
**Status:** Draft  
**Last Updated:** 2026-01-22  
**Extends:** Section 11 (Session and Staging Management)

---

## Overview

This document extends the CTX PRD to specify session boundary semantics, user interaction handling, and stale session recovery. These specifications define how sessions map to user conversations, how user messages affect session state, and how the system handles sessions that become idle or abandoned.

---

## 11.7 Session Boundary Model

### 11.7.1 Definition

A **session** represents one logical unit of work on a task. Sessions map to **conversations**, not to individual agent execution runs.

A single session may span:

- Multiple user messages
- Multiple agent responses
- Multiple LLM calls
- Pauses of arbitrary duration (minutes, hours, or days)
- User interventions and modifications

A session ends only when:

- The user explicitly confirms task completion
- The user explicitly abandons the task
- The user starts a clearly unrelated new task
- The session becomes stale (exceeds idle threshold)

### 11.7.2 Rationale

Mapping sessions to conversations rather than agent runs provides:

| Benefit | Description |
|---------|-------------|
| **Coherent history** | Related work is grouped in a single commit |
| **Natural checkpoints** | Compaction happens at task boundaries, not arbitrary timeouts |
| **User control** | Users decide when work is "done" |
| **Pause tolerance** | Sessions survive laptop closes, overnight breaks, etc. |

### 11.7.3 Session vs Conversation Diagram

```
+---------------------------------------------------------------------+
|                         CONVERSATION                                 |
|                                                                      |
|  +----------------------------------------------------------------+  |
|  |                      SESSION                                   |  |
|  |                                                                |  |
|  |  User: "Add retry logic"                                       |  |
|  |      |                                                         |  |
|  |      v                                                         |  |
|  |  Agent: [works] "Should I use exponential backoff?"            |  |
|  |      |                                                         |  |
|  |      v                                                         |  |
|  |  [PAUSE - AwaitingUser state]                                  |  |
|  |      |                                                         |  |
|  |      v                                                         |  |
|  |  User: "Yes, with jitter"                                      |  |
|  |      |                                                         |  |
|  |      v                                                         |  |
|  |  Agent: [continues working]                                    |  |
|  |      |                                                         |  |
|  |      v                                                         |  |
|  |  User: "Make it 5 retries not 3"   <-- Interrupted state       |  |
|  |      |                                                         |  |
|  |      v                                                         |  |
|  |  Agent: [adjusts and continues]                                |  |
|  |      |                                                         |  |
|  |      v                                                         |  |
|  |  Agent: "Done! Tests passing."     <-- PendingComplete state   |  |
|  |      |                                                         |  |
|  |      v                                                         |  |
|  |  User: "Looks good!"               <-- Complete state          |  |
|  |      |                                                         |  |
|  |      v                                                         |  |
|  |  [COMPACT] -------------------------------------------------->|  |
|  |                                                                |  |
|  +----------------------------------------------------------------+  |
|                                                                      |
|  User: "Now add connection pooling"   <-- NewTask classification     |
|      |                                                               |
|      v                                                               |
|  +----------------------------------------------------------------+  |
|  |                    NEW SESSION                                 |  |
|  |  ...                                                           |  |
|  +----------------------------------------------------------------+  |
|                                                                      |
+----------------------------------------------------------------------+
```

---

## 11.8 Session State Machine

### 11.8.1 State Definitions

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionState {
    /// Agent is actively working on the task
    Running,
    
    /// Agent has asked a question and is waiting for user response
    AwaitingUser {
        /// The question or clarification request
        question: String,
        /// Unix timestamp when question was asked
        asked_at: i64,
    },
    
    /// User sent a message while agent was working (intervention)
    Interrupted {
        /// The user's message that caused the interruption
        user_message: String,
    },
    
    /// Agent believes task is complete, awaiting user confirmation
    PendingComplete {
        /// Summary of what was accomplished
        summary: String,
    },
    
    /// User has confirmed completion, ready for compaction
    Complete,
    
    /// Session was abandoned or cancelled
    Aborted {
        /// Reason for abortion
        reason: String,
    },
}
```

### 11.8.2 State Transition Diagram

```
                +--------------------------------------+
                |                                      |
                v                                      |
    +-------+  task   +---------+  question  +-----------------+
    | None  | ------> | Running | ---------> |  AwaitingUser   |
    +-------+         +---------+            +-----------------+
                           |                        |
                           | user                   | user
                           | interrupts             | responds
                           v                        |
                     +-----------+                  |
                     | Interrupted|                 |
                     +-----------+                  |
                           |                        |
                           | agent                  |
                           | incorporates           |
                           v                        |
                      +---------+ <-----------------+
                      | Running |
                      +---------+
                           |
                           | agent thinks done
                           v
                  +-----------------+
                  | PendingComplete |
                  +-----------------+
                      |         |
          user says   |         |  user says
          "looks good"|         |  "also add..."
                      v         v
                +----------+  +---------+
                | Complete |  | Running | (continues)
                +----------+  +---------+
                      |
                      | compact_session()
                      v
                 [session = None]
```

### 11.8.3 Valid State Transitions

| From State | To State | Trigger |
|------------|----------|---------|
| None | Running | `start_session(task)` |
| Running | AwaitingUser | Agent asks question, calls `set_state()` |
| Running | PendingComplete | Agent finishes, calls `set_state()` |
| Running | Interrupted | User message arrives while Running |
| Running | Aborted | User says "cancel" or "stop" |
| AwaitingUser | Running | User responds |
| AwaitingUser | Aborted | User says "never mind" |
| Interrupted | Running | Agent incorporates feedback |
| PendingComplete | Complete | User confirms |
| PendingComplete | Running | User requests modifications |
| PendingComplete | Aborted | User rejects completely |
| Complete | None | `compact_session()` called |
| Aborted | None | `compact_session()` called |

### 11.8.4 State Persistence

Session state is persisted on every `flush_step()` call. The state is stored in the WorkCommit:

```rust
#[derive(Serialize, Deserialize)]
pub struct WorkCommit {
    pub parent: ObjectId,
    pub base: ObjectId,
    pub payload: Vec<ObjectId>,
    pub timestamp: i64,
    pub session_state: SessionState,  // Added field
}
```

This ensures that session state survives crashes and can be recovered on restart.

**CLI Session Handling:**

For CLI usage, sessions are not held in memory between commands. Instead, each CLI command:
1. Opens the repository
2. Checks for a STAGE pointer (active session)
3. Reconstructs the session from the staging chain if STAGE exists
4. Performs the operation
5. Exits

This follows the Git model where state is always reconstructed from disk. Commands like `ctx stage status`, `ctx stage flush`, and `ctx stage compact` automatically recover the session from the STAGE pointer, so users don't need to explicitly call `ctx stage recover` unless recovering after a crash.

---

## 11.9 User Message Classification

### 11.9.1 Classification Categories

When a user message arrives, the agent must classify it to determine the appropriate session action:

| Classification | Description | Example Messages |
|----------------|-------------|------------------|
| **Response** | Direct answer to agent's question | "Yes", "Option B", "Use exponential" |
| **Modification** | Change request for current work | "Make it 5 not 3", "Also add logging" |
| **Confirmation** | Approval of completed work | "Looks good", "Perfect", "Ship it" |
| **Abandon** | Request to stop current task | "Never mind", "Cancel", "Stop" |
| **NewTask** | Unrelated new request | "Now add pooling", "Different topic..." |
| **Clarification** | Request for more info (no state change) | "What does that mean?", "Show me the code" |

### 11.9.2 Classification Logic

Classification should consider:

1. **Current session state** - A "yes" in AwaitingUser is a Response; the same in PendingComplete is a Confirmation
2. **Semantic content** - Does the message relate to the current task?
3. **Explicit markers** - Words like "cancel", "stop", "new task", "instead"

```rust
pub fn classify_message(
    message: &str,
    current_state: &SessionState,
    task_context: &str,
) -> MessageClassification {
    // State-aware classification
    match current_state {
        SessionState::AwaitingUser { question, .. } => {
            if is_answer_to(message, question) {
                return MessageClassification::Response;
            }
        }
        SessionState::PendingComplete { .. } => {
            if is_confirmation(message) {
                return MessageClassification::Confirmation;
            }
            if requests_changes(message) {
                return MessageClassification::Modification;
            }
        }
        _ => {}
    }
    
    // Content-based classification
    if is_abandonment(message) {
        return MessageClassification::Abandon;
    }
    
    if is_unrelated_to(message, task_context) {
        return MessageClassification::NewTask;
    }
    
    // Default: treat as modification to current task
    MessageClassification::Modification
}
```

### 11.9.3 Classification to Action Mapping

| Classification | Current State | Action |
|----------------|---------------|--------|
| Response | AwaitingUser | `observe_note()`, transition to Running |
| Response | Other | Treat as Modification |
| Modification | Any active | `observe_note()`, ensure Running, continue |
| Confirmation | PendingComplete | Transition to Complete, `compact_session()` |
| Confirmation | Other | Acknowledge, no state change |
| Abandon | Any active | Transition to Aborted, `compact_session()` |
| NewTask | Any active | `compact_session()` current, `start_session()` new |
| NewTask | None | `start_session()` |
| Clarification | Any | Respond without state change |

---

## 11.10 Stale Session Handling

### 11.10.1 Problem Statement

Sessions can become "stale" when:

- User closes application mid-session
- User forgets about an in-progress task
- Process crashes and user doesn't return immediately
- User returns after extended absence with a new task

Without explicit handling, stale sessions accumulate in staging, blocking new work and consuming resources.

### 11.10.2 Idle Time Thresholds

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleSessionConfig {
    /// After this duration, ask user before starting new task
    /// Default: 24 hours
    pub ask_threshold: Duration,
    
    /// After this duration, auto-compact without asking
    /// Default: 7 days  
    pub auto_compact_threshold: Duration,
    
    /// Message to include in auto-compacted sessions
    /// Default: "Auto-saved: session idle for {duration}"
    pub auto_compact_message_template: String,
}

impl Default for StaleSessionConfig {
    fn default() -> Self {
        Self {
            ask_threshold: Duration::hours(24),
            auto_compact_threshold: Duration::days(7),
            auto_compact_message_template: 
                "Auto-saved: session idle for {duration}".into(),
        }
    }
}
```

### 11.10.3 Idle Time Behavior Matrix

| Idle Duration | User Message Type | System Behavior |
|---------------|-------------------|-----------------|
| < ask_threshold | NewTask | Prompt: "Save current work and start fresh?" |
| < ask_threshold | Continuation | Resume session automatically |
| ask_threshold to auto_compact_threshold | Any | Prompt: "Continue previous work or start fresh?" |
| > auto_compact_threshold | Any | Auto-compact with stale marker, proceed with message |

### 11.10.4 Implementation

```rust
impl CodingAgent {
    pub async fn handle_user_message(&mut self, message: &str) -> Result<Response> {
        // Step 1: Check for existing session and idle time
        if let Some(session) = self.ctx.active_session() {
            let idle_time = Instant::now() - session.last_activity();
            
            // Step 2: Apply idle time rules
            if idle_time > self.config.stale.auto_compact_threshold {
                // Very stale - auto-compact and proceed
                let task_desc = session.task_description().to_string();
                let duration = format_duration(idle_time);
                let message = self.config.stale.auto_compact_message_template
                    .replace("{duration}", &duration);
                
                self.ctx.compact_session(&format!("{}: {}", message, task_desc))?;
                // Fall through to normal handling
                
            } else if idle_time > self.config.stale.ask_threshold {
                // Moderately stale - always ask
                return Ok(Response::Prompt {
                    message: format!(
                        "Welcome back! You were working on: {}\n\n\
                         Would you like to:\n\
                         A) Continue that work\n\
                         B) Save it and start fresh",
                        session.task_description()
                    ),
                    pending_action: PendingAction::StaleSessionChoice {
                        user_message: message.to_string(),
                    },
                });
                
            } else {
                // Recent - use classification
                let classification = self.classify_message(message).await?;
                
                if classification == MessageClassification::NewTask {
                    return Ok(Response::Prompt {
                        message: format!(
                            "You have unfinished work on: {}\n\n\
                             Should I:\n\
                             A) Save that and start on your new request\n\
                             B) Continue where we left off",
                            session.task_description()
                        ),
                        pending_action: PendingAction::NewTaskChoice {
                            new_task: message.to_string(),
                        },
                    });
                }
                // Otherwise, continue with existing session
            }
        }
        
        // Step 3: Normal message handling
        self.handle_message_normally(message).await
    }
}
```

### 11.10.5 Handling User Choices

When the user responds to a stale session prompt:

```rust
impl CodingAgent {
    pub async fn handle_stale_session_choice(
        &mut self,
        choice: StaleSessionChoice,
        original_message: &str,
    ) -> Result<Response> {
        match choice {
            StaleSessionChoice::Continue => {
                // Resume the existing session
                self.ctx.active_session_mut()
                    .observe_note("User chose to continue after idle period")?;
                self.continue_working().await
            }
            
            StaleSessionChoice::StartFresh => {
                // Compact old session, start new
                let old_task = self.ctx.active_session()
                    .map(|s| s.task_description().to_string());
                
                if let Some(task) = old_task {
                    self.ctx.compact_session(
                        &format!("Saved before starting new task: {}", task)
                    )?;
                }
                
                // Now handle original message as new task
                self.start_new_task(original_message).await
            }
        }
    }
}
```

### 11.10.6 Background Cleanup (Optional)

For long-running agent processes, implement periodic cleanup:

```rust
impl CtxRepo {
    /// Clean up sessions that have been idle too long
    /// Intended to be called periodically or on startup
    pub fn cleanup_stale_sessions(
        &self,
        max_idle: Duration,
    ) -> Result<CleanupReport> {
        let mut report = CleanupReport::default();
        
        if let Some(session) = self.recover_session()? {
            let idle = Instant::now() - session.last_activity();
            
            if idle > max_idle {
                let task = session.task_description().to_string();
                let duration = format_duration(idle);
                
                self.compact_session(
                    &format!("Auto-saved stale session (idle {}): {}", duration, task)
                )?;
                
                report.sessions_compacted += 1;
                report.compacted_tasks.push(task);
            }
        }
        
        Ok(report)
    }
}

#[derive(Debug, Default)]
pub struct CleanupReport {
    pub sessions_compacted: u32,
    pub compacted_tasks: Vec<String>,
}
```

### 11.10.7 Stale Session Commit Markers

Auto-compacted sessions should be clearly marked in the commit history:

```rust
pub struct Commit {
    // ... existing fields ...
    
    /// How this commit was created
    pub commit_type: CommitType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommitType {
    /// Normal task completion
    Normal,
    
    /// User explicitly abandoned task
    Abandoned,
    
    /// Session auto-compacted due to staleness
    StaleAutoCompact {
        idle_duration_secs: u64,
    },
    
    /// Session compacted when user started new task
    InterruptedByNewTask {
        new_task_summary: String,
    },
}
```

This allows retrieval to distinguish between completed work and abandoned/interrupted work.

---

## 11.11 Session Recovery

### 11.11.1 Recovery on Startup

When the agent starts, it should check for and handle existing sessions:

```rust
impl CodingAgent {
    pub async fn initialize(&mut self) -> Result<()> {
        // Check for existing session in staging
        if let Some(session) = self.ctx.recover_session()? {
            let idle = Instant::now() - session.last_activity();
            
            // Apply stale session rules
            if idle > self.config.stale.auto_compact_threshold {
                // Too old, just clean it up
                self.ctx.cleanup_stale_sessions(self.config.stale.auto_compact_threshold)?;
                return Ok(());
            }
            
            // Recover and present state to user
            match session.state() {
                SessionState::AwaitingUser { question, .. } => {
                    self.present_to_user(&format!(
                        "Welcome back! I was asking:\n\n{}\n",
                        question
                    ))?;
                }
                
                SessionState::PendingComplete { summary, .. } => {
                    self.present_to_user(&format!(
                        "Welcome back! I had finished:\n\n{}\n\nDoes this look good?",
                        summary
                    ))?;
                }
                
                SessionState::Running => {
                    let progress = session.generate_progress_summary()?;
                    self.present_to_user(&format!(
                        "Welcome back! I was working on: {}\n\n\
                         Progress so far:\n{}\n\n\
                         Should I continue?",
                        session.task_description(),
                        progress
                    ))?;
                }
                
                SessionState::Interrupted { user_message } => {
                    self.present_to_user(&format!(
                        "Welcome back! You had said:\n\n{}\n\n\
                         I hadn't finished processing that. Should I continue?",
                        user_message
                    ))?;
                }
                
                _ => {}
            }
        }
        
        Ok(())
    }
}
```

### 11.11.2 Recovery Guarantees

| Scenario | Data Preserved | State Preserved |
|----------|----------------|-----------------|
| Crash after `observe_*()` but before `flush_step()` | Partial (in-memory only) | No |
| Crash after `flush_step()` | Yes | Yes |
| Crash during `compact_session()` | Staging preserved | Yes (can retry compact) |
| Clean shutdown | Yes | Yes |

The key guarantee: **any work that was flushed to staging is recoverable**.

---

## 11.12 Multi-Agent Handoff

### 11.12.1 Session Transfer

Sessions can be handed off between different agent instances:

```rust
impl CtxRepo {
    /// Export session state for transfer to another agent
    pub fn export_session(&self) -> Result<Option<ExportedSession>> {
        let session = self.active_session().ok_or(Error::NoActiveSession)?;
        
        Ok(Some(ExportedSession {
            task_description: session.task_description().to_string(),
            state: session.state().clone(),
            staging_head: session.staging_head(),
            progress_summary: session.generate_progress_summary()?,
            last_activity: session.last_activity(),
        }))
    }
    
    /// Import and resume a session from another agent
    pub fn import_session(&self, exported: &ExportedSession) -> Result<Session> {
        // Verify staging head exists in our object store
        self.objects.get(&exported.staging_head)?;
        
        // Reconstruct session
        let session = Session::from_staging(
            exported.staging_head,
            exported.state.clone(),
        )?;
        
        Ok(session)
    }
}
```

### 11.12.2 Handoff Scenarios

| Scenario | Procedure |
|----------|-----------|
| User switches from CLI agent to IDE agent | Export session, import in new agent |
| Agent process migrates between machines | Export session state, transfer .ctx directory |
| Different LLM backend needed | Export session, start new agent with same .ctx |

The key insight: session state is fully captured in the staging chain. Any agent that can read the .ctx directory can resume work.

---

## Summary

This extension specifies:

1. **Session boundaries** map to conversations, not agent runs
2. **Session states** form an explicit state machine (Running, AwaitingUser, Interrupted, PendingComplete, Complete, Aborted)
3. **User messages** are classified to determine session actions
4. **Stale sessions** are handled with time-based thresholds (ask after 24h, auto-compact after 7d)
5. **Recovery** restores session state from staging after crashes
6. **Handoff** enables session transfer between agent instances

These specifications ensure sessions have clear boundaries, stale work is preserved rather than lost, and users maintain control over when work is considered "done".
