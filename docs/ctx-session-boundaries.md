# CTX Session Boundary Model

## The Problem

When does a session end? Consider these scenarios:

1. Agent asks "Which retry strategy do you prefer?" → User answers → Agent continues
2. User says "actually, make it 5 retries instead of 3" mid-task
3. User closes laptop, comes back tomorrow, says "continue where we left off"
4. User says "also add logging" — is this the same task or a new one?
5. Agent fails, user says "try a different approach"

**The question:** Are these continuations of one session or separate sessions?

---

## Design Decision: Conversation-Scoped Sessions

A **session** maps to a **conversation** (chat thread), not to individual agent runs.

```
┌─────────────────────────────────────────────────────────────────────┐
│                         CONVERSATION                                 │
│                                                                      │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                      SESSION                                 │    │
│  │                                                              │    │
│  │  User: "Add retry logic"                                    │    │
│  │      │                                                       │    │
│  │      ▼                                                       │    │
│  │  Agent: [works] "Should I use exponential backoff?"         │    │
│  │      │                              ▲                        │    │
│  │      ▼                              │                        │    │
│  │  [PAUSE - waiting for user] ───────┘                        │    │
│  │      │                                                       │    │
│  │      ▼                                                       │    │
│  │  User: "Yes, with jitter"                                   │    │
│  │      │                                                       │    │
│  │      ▼                                                       │    │
│  │  Agent: [continues working]                                 │    │
│  │      │                                                       │    │
│  │      ▼                                                       │    │
│  │  User: "Make it 5 retries not 3"  ← INTERVENTION            │    │
│  │      │                                                       │    │
│  │      ▼                                                       │    │
│  │  Agent: [adjusts and continues]                             │    │
│  │      │                                                       │    │
│  │      ▼                                                       │    │
│  │  Agent: "Done! Tests passing."                              │    │
│  │      │                                                       │    │
│  │      ▼                                                       │    │
│  │  [SESSION COMPLETE] ─────────────────────────────────────── │ ──►  COMPACT
│  │                                                              │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                      │
│  User: "Now add connection pooling"  ← NEW TASK                     │
│      │                                                               │
│      ▼                                                               │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                    NEW SESSION                               │    │
│  │  ...                                                         │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Session States

```rust
pub enum SessionState {
    /// Agent is actively working
    Running,
    
    /// Waiting for user input (question asked, clarification needed)
    AwaitingUser {
        question: String,
        asked_at: i64,
    },
    
    /// User interrupted, agent should incorporate feedback
    Interrupted {
        user_message: String,
    },
    
    /// Agent considers task complete, awaiting user confirmation
    PendingComplete {
        summary: String,
    },
    
    /// Session fully complete, ready for compaction
    Complete,
    
    /// Session aborted
    Aborted {
        reason: String,
    },
}
```

---

## State Transitions

```
                    ┌──────────────────────────────────────┐
                    │                                      │
                    ▼                                      │
    ┌─────────┐  task   ┌─────────┐  question  ┌─────────────────┐
    │  None   │ ──────► │ Running │ ─────────► │  AwaitingUser   │
    └─────────┘         └─────────┘            └─────────────────┘
                             │                        │
                             │ user                   │ user
                             │ interrupts             │ responds
                             ▼                        │
                       ┌─────────────┐                │
                       │ Interrupted │                │
                       └─────────────┘                │
                             │                        │
                             │ agent                  │
                             │ incorporates           │
                             ▼                        │
                        ┌─────────┐ ◄─────────────────┘
                        │ Running │
                        └─────────┘
                             │
                             │ agent thinks done
                             ▼
                    ┌─────────────────┐
                    │ PendingComplete │
                    └─────────────────┘
                        │         │
            user says   │         │  user says
            "looks good"│         │  "not quite, also..."
                        ▼         ▼
                  ┌──────────┐  ┌─────────┐
                  │ Complete │  │ Running │ (continues)
                  └──────────┘  └─────────┘
                        │
                        │ auto-compact
                        ▼
                   [New session or none]
```

---

## What Gets Stored at Each State

### Running → AwaitingUser

Agent asked a question, waiting for response.

```rust
// Flush current progress
session.flush_step()?;

// Record the question in narrative
session.observe_note(&format!("Question for user: {}", question))?;

// Update session state
session.set_state(SessionState::AwaitingUser { 
    question: question.clone(),
    asked_at: now(),
})?;

// Staging pointer is valid - session can resume later
```

**On disk:**
- Staging pointer preserved
- All work so far is in WorkCommits
- Narrative log has the question

**If system crashes here:** Session resumes, agent can re-ask or user can see last question.

### AwaitingUser → Running (User Responds)

```rust
// Record user's response
session.observe_note(&format!("User response: {}", response))?;

// Create edge: this response relates to the question/task
session.observe_relations(&[
    Edge {
        from: NodeId::Note(response_note_id),
        to: NodeId::Task(current_task_id),
        label: EdgeLabel::Mentions,
        ...
    }
])?;

// Resume execution
session.set_state(SessionState::Running)?;
```

### Running → Interrupted (User Intervenes)

User sends a message while agent is working.

```rust
// Pause current work
session.flush_step()?;

// Record intervention
session.observe_note(&format!("User intervention: {}", message))?;

// Set state - agent should read this and adjust
session.set_state(SessionState::Interrupted {
    user_message: message.clone(),
})?;
```

**Agent's responsibility:** Check for interruptions, incorporate feedback, continue.

### PendingComplete → Complete (User Confirms)

```rust
// User said "looks good" or similar
session.observe_note("User confirmed task complete")?;

// Mark complete
session.set_state(SessionState::Complete)?;

// Trigger compaction
ctx.compact_session(&summary)?;
```

### PendingComplete → Running (User Wants More)

```rust
// User said "also add X" or "not quite"
session.observe_note(&format!("User requested continuation: {}", request))?;

// Back to running
session.set_state(SessionState::Running)?;

// Agent continues with new information
```

---

## Distinguishing New Task vs Continuation

The agent (not CTX) decides this based on semantic analysis:

```rust
impl CodingAgent {
    fn classify_user_message(&self, message: &str, session: &Session) -> MessageIntent {
        // Use LLM to classify
        let prompt = format!(
            "Current task: {}\n\
             Current state: {:?}\n\
             User message: {}\n\n\
             Is this:\n\
             A) A response to a pending question\n\
             B) A modification to the current task\n\
             C) Confirmation that task is complete\n\
             D) A completely new, unrelated task\n\
             E) Abandoning current task",
            session.task_description(),
            session.state(),
            message
        );
        
        // Parse LLM response into intent
        match llm.classify(&prompt) {
            "A" => MessageIntent::Response,
            "B" => MessageIntent::Modification,
            "C" => MessageIntent::Confirmation,
            "D" => MessageIntent::NewTask,
            "E" => MessageIntent::Abandon,
            _ => MessageIntent::Unclear,
        }
    }
    
    async fn handle_user_message(&mut self, message: &str) {
        let intent = self.classify_user_message(message, &self.session);
        
        match intent {
            MessageIntent::Response | MessageIntent::Modification => {
                // Continue current session
                self.session.incorporate_feedback(message)?;
                self.continue_task().await?;
            }
            
            MessageIntent::Confirmation => {
                // Complete and compact
                self.ctx.compact_session(&self.session.summary())?;
                self.session = None;
            }
            
            MessageIntent::NewTask => {
                // Complete current session first (if any meaningful work)
                if self.session.has_meaningful_work() {
                    self.ctx.compact_session("Interrupted by new task")?;
                }
                // Start fresh
                self.session = self.ctx.start_session(message)?;
                self.run_task().await?;
            }
            
            MessageIntent::Abandon => {
                self.ctx.compact_session("Abandoned by user")?;
                self.session = None;
            }
            
            MessageIntent::Unclear => {
                // Ask for clarification
                self.ask_user("Are you continuing the current task or starting something new?")?;
            }
        }
    }
}
```

---

## Session Timeout and Recovery

### Timeout Policy

```rust
pub struct SessionConfig {
    /// How long a session can be paused before auto-compacting
    pub idle_timeout: Duration,  // default: 24 hours
    
    /// How long before considering session "stale"
    pub stale_timeout: Duration, // default: 7 days
}
```

### On Conversation Resume

When user comes back to a conversation:

```rust
impl CodingAgent {
    async fn on_conversation_resume(&mut self) -> Result<()> {
        // Check if there's a pending session
        if let Some(session) = self.ctx.active_session() {
            let idle_time = now() - session.last_activity();
            
            if idle_time > self.config.stale_timeout {
                // Too old, compact and start fresh
                self.ctx.compact_session("Session expired")?;
                self.prompt_for_new_task().await?;
                
            } else if idle_time > self.config.idle_timeout {
                // Ask user if they want to continue
                let response = self.ask_user(
                    &format!(
                        "You have an unfinished task from {}: '{}'\n\
                         Would you like to continue or start fresh?",
                        format_time(session.started_at()),
                        session.task_description()
                    )
                ).await?;
                
                if response.wants_to_continue() {
                    self.resume_session().await?;
                } else {
                    self.ctx.compact_session("User chose not to continue")?;
                    self.prompt_for_new_task().await?;
                }
                
            } else {
                // Recent session, just resume
                self.resume_session().await?;
            }
        }
        
        Ok(())
    }
    
    async fn resume_session(&mut self) -> Result<()> {
        let session = self.ctx.active_session().unwrap();
        
        match session.state() {
            SessionState::AwaitingUser { question, .. } => {
                // Re-present the question
                self.present_to_user(&format!(
                    "Continuing from where we left off. I had asked:\n\n{}",
                    question
                )).await?;
            }
            
            SessionState::PendingComplete { summary, .. } => {
                // Re-present completion
                self.present_to_user(&format!(
                    "I had finished the task:\n\n{}\n\nDoes this look good?",
                    summary
                )).await?;
            }
            
            SessionState::Running => {
                // Weird state - was interrupted mid-execution
                // Show progress and ask how to proceed
                let progress = session.generate_progress_summary()?;
                self.present_to_user(&format!(
                    "The task was interrupted. Progress so far:\n\n{}\n\n\
                     Should I continue?",
                    progress
                )).await?;
            }
            
            _ => {}
        }
        
        Ok(())
    }
}
```

---

## Multi-Turn Example

Here's a complete conversation flow:

```
┌─────────────────────────────────────────────────────────────────────┐
│ TURN 1: User initiates                                              │
├─────────────────────────────────────────────────────────────────────┤
│ User: "Add retry logic to the connect function"                     │
│                                                                      │
│ Agent internal:                                                      │
│   → classify: NewTask                                               │
│   → ctx.start_session("Add retry logic...")                         │
│   → session.state = Running                                         │
│   → [reads files, makes plan]                                       │
│   → session.observe_file_read(...)                                  │
│   → session.observe_plan(...)                                       │
│                                                                      │
│ Agent: "I'll add retry logic. Should I use:                         │
│         A) Fixed delay (simple but can cause thundering herd)       │
│         B) Exponential backoff (better distributed)                 │
│         C) Exponential with jitter (best practice)"                 │
│                                                                      │
│ Agent internal:                                                      │
│   → session.flush_step()                                            │
│   → session.state = AwaitingUser { question: "..." }                │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                                    │ [could be minutes, hours, or days]
                                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│ TURN 2: User responds                                               │
├─────────────────────────────────────────────────────────────────────┤
│ User: "Option C, and make max retries configurable"                 │
│                                                                      │
│ Agent internal:                                                      │
│   → classify: Response (answers pending question)                   │
│   → session.observe_note("User response: Option C...")              │
│   → session.state = Running                                         │
│   → [implements with exponential backoff + jitter]                  │
│   → [makes max_retries a parameter]                                 │
│   → session.observe_file_write(...)                                 │
│                                                                      │
│ Agent: "I've implemented exponential backoff with jitter.           │
│         Running tests..."                                            │
│                                                                      │
│ Agent internal:                                                      │
│   → [runs cargo test]                                               │
│   → session.observe_command("cargo test", ...)                      │
│                                                                      │
│ Agent: "Tests pass. Here's what I added: [summary]                  │
│         Does this look good?"                                        │
│                                                                      │
│ Agent internal:                                                      │
│   → session.state = PendingComplete { summary: "..." }              │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│ TURN 3: User wants modification                                     │
├─────────────────────────────────────────────────────────────────────┤
│ User: "Looks good but also add logging for retry attempts"          │
│                                                                      │
│ Agent internal:                                                      │
│   → classify: Modification (extends current task)                   │
│   → session.observe_note("User requested: add logging...")          │
│   → session.state = Running                                         │
│   → [adds logging]                                                  │
│   → session.observe_file_write(...)                                 │
│   → [runs tests again]                                              │
│                                                                      │
│ Agent: "Added logging. Each retry attempt now logs:                 │
│         - Attempt number                                             │
│         - Delay before retry                                         │
│         - Error that caused retry                                    │
│         All tests still pass. Done?"                                 │
│                                                                      │
│ Agent internal:                                                      │
│   → session.state = PendingComplete { summary: "..." }              │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│ TURN 4: User confirms                                               │
├─────────────────────────────────────────────────────────────────────┤
│ User: "Perfect, thanks!"                                            │
│                                                                      │
│ Agent internal:                                                      │
│   → classify: Confirmation                                          │
│   → session.observe_note("User confirmed complete")                 │
│   → session.state = Complete                                        │
│   → ctx.compact_session("Added retry logic with exponential         │
│       backoff, jitter, configurable max retries, and logging")      │
│   → session = None                                                  │
│                                                                      │
│ [CANONICAL COMMIT CREATED]                                          │
│                                                                      │
│ Agent: "Great! The changes have been saved. Let me know if you      │
│         need anything else."                                         │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│ TURN 5: User starts new task                                        │
├─────────────────────────────────────────────────────────────────────┤
│ User: "Now let's add connection pooling"                            │
│                                                                      │
│ Agent internal:                                                      │
│   → classify: NewTask                                               │
│   → ctx.start_session("Add connection pooling")                     │
│   → [new session begins, can reference previous work via retrieval] │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Summary

| Scenario | Classification | Action |
|----------|---------------|--------|
| Agent asks question, user answers | Response | Continue session |
| User says "change X to Y" mid-task | Modification | Continue session |
| User says "looks good" / "done" | Confirmation | Compact session |
| User says "never mind" / "stop" | Abandon | Compact (partial), end session |
| User says "now do something unrelated" | NewTask | Compact current, start new |
| User returns after hours/days | Resume | Check state, offer to continue |
| User returns after > stale_timeout | Stale | Compact old, prompt for new |

**The key insight:** A session is the logical unit of work on a task, spanning multiple conversation turns. Staging preserves all intermediate state, so pausing (for user input or overnight) is just a state transition, not a session boundary.
