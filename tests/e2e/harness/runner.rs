use super::assertions::{Assertion, SessionStateMatch};
use super::clock::MockClock;
use super::steps::ScenarioStep;
use super::workspace::TestWorkspace;
use anyhow::{anyhow, Context, Result};
use ctx_core::{Commit, CtxRepo, SessionState, Tree, TreeEntryKind};
use std::collections::HashMap;
use std::time::Duration;

/// Executes scenarios against real CTX instance
pub struct ScenarioRunner {
    workspace: TestWorkspace,
    ctx: Option<CtxRepo>,
    clock: MockClock,
    crashed: bool,
    current_step: usize,
    session_was_recovered: bool,
}

impl ScenarioRunner {
    /// Create a new runner with initial files
    pub fn new(initial_files: HashMap<String, Vec<u8>>) -> Result<Self> {
        let workspace = TestWorkspace::with_files(initial_files)?;
        let clock = MockClock::new();
        let ctx = workspace
            .init_ctx()?
            .with_time_provider(clock.as_provider());

        Ok(Self {
            workspace,
            ctx: Some(ctx),
            clock,
            crashed: false,
            current_step: 0,
            session_was_recovered: false,
        })
    }

    /// Get current step number
    pub fn current_step(&self) -> usize {
        self.current_step
    }

    /// Execute all steps in sequence
    pub fn execute(&mut self, steps: &[ScenarioStep]) -> Result<()> {
        for (i, step) in steps.iter().enumerate() {
            self.current_step = i;
            self.execute_step(step)
                .with_context(|| format!("Step {}: {:?}", i, step))?;
        }
        Ok(())
    }

    /// Execute a single step
    fn execute_step(&mut self, step: &ScenarioStep) -> Result<()> {
        match step {
            ScenarioStep::UserStartTask { description } => self.handle_user_start_task(description),
            ScenarioStep::UserResponse { text } => self.handle_user_response(text),
            ScenarioStep::UserIntervention { message } => self.handle_user_intervention(message),
            ScenarioStep::UserConfirmation => self.handle_user_confirmation(),
            ScenarioStep::UserRejection { feedback } => self.handle_user_rejection(feedback),

            ScenarioStep::AgentReadFile { path } => self.handle_agent_read(path),
            ScenarioStep::AgentWriteFile { path, content } => {
                self.handle_agent_write(path, content)
            }
            ScenarioStep::AgentRunCommand {
                command,
                exit_code,
                output,
            } => self.handle_agent_command(command, *exit_code, output),
            ScenarioStep::AgentNote { text } => self.handle_agent_note(text),
            ScenarioStep::AgentFlush => self.handle_agent_flush(),
            ScenarioStep::AgentAskQuestion { question } => self.handle_agent_ask(question),
            ScenarioStep::AgentComplete { summary } => self.handle_agent_complete(summary),
            ScenarioStep::AgentResume => self.handle_agent_resume(),
            ScenarioStep::AgentAbandon { reason } => self.handle_agent_abandon(reason),

            ScenarioStep::Wait { duration } => self.handle_wait(*duration),
            ScenarioStep::WaitHours { hours } => {
                self.handle_wait(Duration::from_secs(hours * 3600))
            }
            ScenarioStep::WaitDays { days } => self.handle_wait(Duration::from_secs(days * 86400)),

            ScenarioStep::Crash => self.handle_crash(),
            ScenarioStep::Restart => self.handle_restart(),

            ScenarioStep::Assert { assertion } => self.handle_assertion(assertion),
        }
    }

    // ===== User action handlers =====

    fn handle_user_start_task(&mut self, description: &str) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available (crashed?)"))?;

        ctx.start_session(description)?;
        Ok(())
    }

    fn handle_user_response(&mut self, text: &str) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        let session = ctx
            .active_session_mut()
            .ok_or_else(|| anyhow!("No active session"))?;

        // Transition from AwaitingUser to Running
        session.set_state(SessionState::Running)?;

        // Record the user response as a note
        ctx.observe_note(&format!("User responded: {}", text))?;

        Ok(())
    }

    fn handle_user_intervention(&mut self, message: &str) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        let session = ctx
            .active_session_mut()
            .ok_or_else(|| anyhow!("No active session"))?;

        // Transition to Interrupted
        session.set_state(SessionState::Interrupted {
            user_message: message.to_string(),
        })?;

        Ok(())
    }

    fn handle_user_confirmation(&mut self) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        // Get summary from PendingComplete state
        let session = ctx
            .active_session()
            .ok_or_else(|| anyhow!("No active session"))?;

        let summary = match session.state() {
            SessionState::PendingComplete { summary } => summary.clone(),
            _ => return Err(anyhow!("Session not in PendingComplete state")),
        };

        // Compact the session
        ctx.compact_session(&summary)?;

        Ok(())
    }

    fn handle_user_rejection(&mut self, feedback: &str) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        let session = ctx
            .active_session_mut()
            .ok_or_else(|| anyhow!("No active session"))?;

        // Transition from PendingComplete back to Running
        session.set_state(SessionState::Running)?;

        // Record the feedback
        ctx.observe_note(&format!("User feedback: {}", feedback))?;

        Ok(())
    }

    // ===== Agent action handlers =====

    fn handle_agent_read(&mut self, path: &str) -> Result<()> {
        // Read actual file content from workspace
        let content = self.workspace.read_file(path)?;

        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        // Observe the read WITH content to capture exactly what the agent saw
        // This enables temporal reconstruction and true context analysis
        ctx.observe_file_read_with_content(path, &content)?;

        Ok(())
    }

    fn handle_agent_write(&mut self, path: &str, content: &[u8]) -> Result<()> {
        // Write to workspace filesystem
        self.workspace.write_file(path, content)?;

        // Observe the write
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        ctx.observe_file_write(path, content)?;

        Ok(())
    }

    fn handle_agent_command(&mut self, cmd: &str, exit_code: i32, output: &str) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        ctx.observe_command(cmd, Some(exit_code), Some(output.as_bytes()))?;

        Ok(())
    }

    fn handle_agent_note(&mut self, text: &str) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        ctx.observe_note(text)?;

        Ok(())
    }

    fn handle_agent_flush(&mut self) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        ctx.flush_active_session()?;

        Ok(())
    }

    fn handle_agent_ask(&mut self, question: &str) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        let session = ctx
            .active_session_mut()
            .ok_or_else(|| anyhow!("No active session"))?;

        let now = self.clock.now();

        // Transition to AwaitingUser
        session.set_state(SessionState::AwaitingUser {
            question: question.to_string(),
            asked_at: now,
        })?;

        Ok(())
    }

    fn handle_agent_complete(&mut self, summary: &str) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        let session = ctx
            .active_session_mut()
            .ok_or_else(|| anyhow!("No active session"))?;

        // Transition to PendingComplete
        session.set_state(SessionState::PendingComplete {
            summary: summary.to_string(),
        })?;

        Ok(())
    }

    fn handle_agent_resume(&mut self) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        let session = ctx
            .active_session_mut()
            .ok_or_else(|| anyhow!("No active session"))?;

        // Transition from Interrupted to Running
        session.set_state(SessionState::Running)?;

        Ok(())
    }

    fn handle_agent_abandon(&mut self, reason: &str) -> Result<()> {
        let ctx = self
            .ctx
            .as_mut()
            .ok_or_else(|| anyhow!("CTX not available"))?;

        ctx.abort_session(reason)?;

        Ok(())
    }

    // ===== Time control =====

    fn handle_wait(&mut self, duration: Duration) -> Result<()> {
        self.clock.advance(duration);
        Ok(())
    }

    // ===== Failure simulation =====

    fn handle_crash(&mut self) -> Result<()> {
        // Drop CTX without cleanup
        self.ctx = None;
        self.crashed = true;
        Ok(())
    }

    fn handle_restart(&mut self) -> Result<()> {
        if !self.crashed {
            return Err(anyhow!("Cannot restart - not crashed"));
        }

        // Reopen CTX with time provider
        let mut ctx = self
            .workspace
            .open_ctx()?
            .with_time_provider(self.clock.as_provider());

        // Try to recover session
        if let Some(_session) = ctx.recover_session()? {
            self.session_was_recovered = true;
        }

        self.ctx = Some(ctx);
        self.crashed = false;

        Ok(())
    }

    // ===== Assertions =====

    fn handle_assertion(&mut self, assertion: &Assertion) -> Result<()> {
        match assertion {
            // Most assertions only need immutable access
            Assertion::SessionState(expected) => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_session_state(ctx, expected)
            }
            Assertion::NoSession => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_no_session(ctx)
            }
            Assertion::SessionExists => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_session_exists(ctx)
            }
            Assertion::CommitCount(n) => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_commit_count(ctx, *n)
            }
            Assertion::CommitCountGte(n) => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_commit_count_gte(ctx, *n)
            }
            Assertion::HeadMessageContains(text) => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_head_message_contains(ctx, text)
            }
            Assertion::FileInHead { path } => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_file_in_head(ctx, path)
            }
            Assertion::FileContentContains { path, content } => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_file_content_contains(ctx, path, content)
            }
            Assertion::FileNotInHead { path } => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_file_not_in_head(ctx, path)
            }
            Assertion::StagingExists => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_staging_exists(ctx)
            }
            Assertion::NoStaging => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_no_staging(ctx)
            }
            Assertion::StagingChainLengthGte(n) => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_staging_chain_length_gte(ctx, *n)
            }
            Assertion::StagingContainsFile { path } => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_staging_contains_file(ctx, path)
            }
            Assertion::StagingContainsNote { text } => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_staging_contains_note(ctx, text)
            }
            Assertion::NoteContains(text) => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_note_contains(ctx, text)
            }
            Assertion::EdgeExists { from, to, label } => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_edge_exists(ctx, from, to, label)
            }
            Assertion::QueryReturnsPath { query, path } => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_query_returns_path(ctx, query, path)
            }
            Assertion::QueryTokensWithinBudget { query, budget } => {
                let ctx = self
                    .ctx
                    .as_ref()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                self.assert_query_tokens_within_budget(ctx, query, *budget)
            }
            Assertion::SessionRecovered => self.assert_session_recovered(),
            Assertion::NoPanic => Ok(()), // If we're here, we didn't panic
            // Custom assertions get mutable access
            Assertion::Custom(f) => {
                let ctx = self
                    .ctx
                    .as_mut()
                    .ok_or_else(|| anyhow!("CTX not available"))?;
                f(ctx)
            }
        }
    }

    fn assert_session_state(&self, ctx: &CtxRepo, expected: &SessionStateMatch) -> Result<()> {
        let session = ctx
            .active_session()
            .ok_or_else(|| anyhow!("No active session"))?;

        let actual = session.state();

        let matches = match (expected, actual) {
            (SessionStateMatch::Running, SessionState::Running) => true,
            (SessionStateMatch::AwaitingUser, SessionState::AwaitingUser { .. }) => true,
            (SessionStateMatch::Interrupted, SessionState::Interrupted { .. }) => true,
            (SessionStateMatch::PendingComplete, SessionState::PendingComplete { .. }) => true,
            (SessionStateMatch::Complete, SessionState::Complete) => true,
            (SessionStateMatch::Aborted, SessionState::Aborted { .. }) => true,
            _ => false,
        };

        if !matches {
            return Err(anyhow!(
                "Session state mismatch: expected {:?}, got {:?}",
                expected,
                actual
            ));
        }

        Ok(())
    }

    fn assert_no_session(&self, ctx: &CtxRepo) -> Result<()> {
        if ctx.has_active_session() {
            return Err(anyhow!("Expected no session, but session exists"));
        }
        Ok(())
    }

    fn assert_session_exists(&self, ctx: &CtxRepo) -> Result<()> {
        if !ctx.has_active_session() {
            return Err(anyhow!("Expected session to exist, but no session found"));
        }
        Ok(())
    }

    fn assert_commit_count(&self, ctx: &CtxRepo, expected: usize) -> Result<()> {
        let count = self.count_commits(ctx)?;
        if count != expected {
            return Err(anyhow!(
                "Commit count mismatch: expected {}, got {}",
                expected,
                count
            ));
        }
        Ok(())
    }

    fn assert_commit_count_gte(&self, ctx: &CtxRepo, min: usize) -> Result<()> {
        let count = self.count_commits(ctx)?;
        if count < min {
            return Err(anyhow!(
                "Commit count too low: expected >= {}, got {}",
                min,
                count
            ));
        }
        Ok(())
    }

    fn assert_head_message_contains(&self, ctx: &CtxRepo, text: &str) -> Result<()> {
        let head = ctx.head()?;
        if !head.message.contains(text) {
            return Err(anyhow!(
                "HEAD message doesn't contain '{}': {}",
                text,
                head.message
            ));
        }
        Ok(())
    }

    fn assert_file_in_head(&self, ctx: &CtxRepo, path: &str) -> Result<()> {
        let head = ctx.head()?;
        let tree: Tree = ctx.object_store().get_typed(head.root_tree)?;

        if !self.tree_contains_path(&tree, path, ctx)? {
            return Err(anyhow!("File '{}' not found in HEAD commit", path));
        }

        Ok(())
    }

    fn assert_file_content_contains(&self, ctx: &CtxRepo, path: &str, content: &str) -> Result<()> {
        let head = ctx.head()?;
        let tree: Tree = ctx.object_store().get_typed(head.root_tree)?;

        let blob_id = self
            .find_file_in_tree(&tree, path, ctx)?
            .ok_or_else(|| anyhow!("File '{}' not found in HEAD commit", path))?;

        let blob = ctx.object_store().get_blob(blob_id)?;
        let blob_text = String::from_utf8_lossy(&blob);

        if !blob_text.contains(content) {
            return Err(anyhow!(
                "File '{}' content doesn't contain '{}'",
                path,
                content
            ));
        }

        Ok(())
    }

    fn assert_file_not_in_head(&self, ctx: &CtxRepo, path: &str) -> Result<()> {
        let head = ctx.head()?;
        let tree: Tree = ctx.object_store().get_typed(head.root_tree)?;

        if self.tree_contains_path(&tree, path, ctx)? {
            return Err(anyhow!("File '{}' unexpectedly found in HEAD commit", path));
        }

        Ok(())
    }

    fn assert_staging_exists(&self, ctx: &CtxRepo) -> Result<()> {
        match ctx.refs().read_stage()? {
            Some(_) => Ok(()),
            None => Err(anyhow!(
                "Expected staging to exist, but STAGE ref not found"
            )),
        }
    }

    fn assert_no_staging(&self, ctx: &CtxRepo) -> Result<()> {
        match ctx.refs().read_stage()? {
            Some(_) => Err(anyhow!("Expected no staging, but STAGE ref exists")),
            None => Ok(()),
        }
    }

    fn assert_staging_chain_length_gte(&self, _ctx: &CtxRepo, _min: usize) -> Result<()> {
        // TODO: Implement staging chain length checking
        // Would need to walk the staging chain and count WorkCommits
        Ok(())
    }

    fn assert_staging_contains_file(&self, _ctx: &CtxRepo, _path: &str) -> Result<()> {
        // TODO: Implement staging file checking
        // Would need to walk staging chain and collect observations
        Ok(())
    }

    fn assert_staging_contains_note(&self, _ctx: &CtxRepo, _text: &str) -> Result<()> {
        // TODO: Implement staging note checking
        // Would need to walk staging chain and check observations
        Ok(())
    }

    fn assert_note_contains(&self, _ctx: &CtxRepo, _text: &str) -> Result<()> {
        // TODO: Implement narrative checking
        // Would need to read narrative log and check content
        Ok(())
    }

    fn assert_edge_exists(
        &self,
        _ctx: &CtxRepo,
        _from: &str,
        _to: &str,
        _label: &str,
    ) -> Result<()> {
        // TODO: Implement edge checking
        // Would need to load edge batches and search
        Ok(())
    }

    fn assert_query_returns_path(&self, _ctx: &CtxRepo, _query: &str, _path: &str) -> Result<()> {
        // TODO: Implement query result checking
        // Would need to build pack and check retrieved files
        Ok(())
    }

    fn assert_query_tokens_within_budget(
        &self,
        _ctx: &CtxRepo,
        _query: &str,
        _budget: usize,
    ) -> Result<()> {
        // TODO: Implement token budget checking
        // Would need to build pack and sum token estimates
        Ok(())
    }

    fn assert_session_recovered(&self) -> Result<()> {
        if !self.session_was_recovered {
            return Err(anyhow!("Expected session to be recovered, but it wasn't"));
        }
        Ok(())
    }

    // ===== Helper methods =====

    fn count_commits(&self, ctx: &CtxRepo) -> Result<usize> {
        use std::collections::HashSet;

        let mut count = 0;
        let mut visited = HashSet::new();
        let mut stack = vec![ctx.head_id()?];

        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }

            count += 1;

            let commit: Commit = ctx.object_store().get_typed(id)?;
            stack.extend(commit.parents);
        }

        Ok(count)
    }

    fn tree_contains_path(&self, tree: &Tree, path: &str, ctx: &CtxRepo) -> Result<bool> {
        Ok(self.find_file_in_tree(tree, path, ctx)?.is_some())
    }

    fn find_file_in_tree(
        &self,
        tree: &Tree,
        path: &str,
        ctx: &CtxRepo,
    ) -> Result<Option<ctx_core::ObjectId>> {
        let parts: Vec<&str> = path.split('/').collect();

        let mut current_tree = tree.clone();

        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;

            if let Some(entry) = current_tree.entries.iter().find(|e| e.name == *part) {
                if is_last {
                    // Found the file
                    match entry.kind {
                        TreeEntryKind::Blob => return Ok(Some(entry.id)),
                        TreeEntryKind::Tree => return Ok(None), // Path points to tree, not file
                    }
                } else {
                    // Need to descend into subtree
                    match entry.kind {
                        TreeEntryKind::Tree => {
                            current_tree = ctx.object_store().get_typed(entry.id)?;
                        }
                        TreeEntryKind::Blob => return Ok(None), // Path component is a file
                    }
                }
            } else {
                return Ok(None); // Path component not found
            }
        }

        Ok(None)
    }
}
