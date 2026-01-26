//! Session lifecycle management for staging work.

use crate::error::{CtxError, Result};
use crate::types::{Observation, SessionState, SessionStats, StepKind, WorkCommit};
use std::collections::HashSet;
use crate::{ObjectId, ObjectStore, Refs};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Active session handle for tracking work in progress.
///
/// A Session represents a unit of work on a single task. It accumulates
/// observations (file reads, writes, commands, notes) and periodically
/// flushes them as WorkCommits to the staging area.
pub struct Session {
    /// Task description for this session.
    task_description: String,

    /// Current session state.
    state: SessionState,

    /// Base canonical commit when session started.
    base_commit: ObjectId,

    /// Current head of staging chain.
    staging_head: ObjectId,

    /// Session identifier.
    session_id: String,

    /// When session was created (Unix timestamp).
    created_at: i64,

    /// Last activity timestamp.
    last_activity: i64,

    /// Pending observations not yet flushed.
    pending_observations: Vec<Observation>,

    /// Step counter for this session.
    step_count: u32,

    /// Time provider for testing (None = use system time).
    time_provider: Option<std::sync::Arc<dyn Fn() -> i64 + Send + Sync>>,
}

impl Session {
    /// Creates a new session.
    ///
    /// The `time_provider` parameter allows injecting controlled time for testing.
    /// Pass `None` to use system time (production default).
    pub(crate) fn new(
        task_description: String,
        base_commit: ObjectId,
        session_id: String,
        time_provider: Option<std::sync::Arc<dyn Fn() -> i64 + Send + Sync>>,
    ) -> Self {
        let now = if let Some(ref provider) = time_provider {
            provider()
        } else {
            current_timestamp()
        };

        Self {
            task_description,
            state: SessionState::Running,
            base_commit,
            staging_head: base_commit,
            session_id,
            created_at: now,
            last_activity: now,
            pending_observations: Vec::new(),
            step_count: 0,
            time_provider,
        }
    }

    /// Recovers a session from an existing staging chain.
    ///
    /// Walks back the staging chain to reconstruct the session state.
    ///
    /// The `time_provider` parameter allows injecting controlled time for testing.
    /// Pass `None` to use system time (production default).
    pub(crate) fn from_staging(
        staging_head: ObjectId,
        object_store: &ObjectStore,
        time_provider: Option<std::sync::Arc<dyn Fn() -> i64 + Send + Sync>>,
    ) -> Result<Self> {
        // Load the staging head WorkCommit
        let head_work: WorkCommit = object_store.get_typed(staging_head)?;

        // Count steps by walking backwards from staging_head to base
        let mut step_count = 1;
        let mut current = staging_head;
        let base = head_work.base;

        // Walk the chain to count steps
        loop {
            let work: WorkCommit = object_store.get_typed(current)?;

            // If this work commit's parent is the base, we're done
            if work.parents.is_empty() || work.parents[0] == base {
                break;
            }

            // Move to parent and increment count
            current = work.parents[0];
            step_count += 1;
        }

        let last_activity = if let Some(ref provider) = time_provider {
            provider()
        } else {
            current_timestamp()
        };

        Ok(Self {
            task_description: head_work.task_description.clone(),
            state: head_work.session_state,
            base_commit: head_work.base,
            staging_head,
            session_id: head_work.session_id,
            created_at: head_work.created_at as i64,
            last_activity,
            pending_observations: Vec::new(),
            step_count,
            time_provider,
        })
    }

    /// Records that the agent read a file (path only, no content).
    pub fn observe_file_read(&mut self, path: &str) -> Result<()> {
        self.update_last_activity();
        self.pending_observations.push(Observation::FileRead {
            path: path.to_string(),
            content_id: None,
        });
        Ok(())
    }

    /// Records that the agent read a file and stores its content in the object store.
    pub fn observe_file_read_with_content(
        &mut self,
        path: &str,
        content: &[u8],
        object_store: &ObjectStore,
    ) -> Result<()> {
        self.update_last_activity();
        let content_id = object_store.put_blob(content)?;
        self.pending_observations.push(Observation::FileRead {
            path: path.to_string(),
            content_id: Some(content_id),
        });
        Ok(())
    }

    /// Records that the agent wrote a file, storing its content in the object store.
    pub fn observe_file_write(
        &mut self,
        path: &str,
        content: &[u8],
        object_store: &ObjectStore,
    ) -> Result<ObjectId> {
        self.update_last_activity();
        let content_id = object_store.put_blob(content)?;
        self.pending_observations.push(Observation::FileWrite {
            path: path.to_string(),
            content_id,
        });
        Ok(content_id)
    }

    /// Record that a command was executed.
    pub fn observe_command(
        &mut self,
        command: &str,
        exit_code: Option<i32>,
        output: Option<&[u8]>,
        object_store: &ObjectStore,
    ) -> Result<()> {
        self.update_last_activity();
        let output_id = if let Some(out) = output {
            Some(object_store.put_blob(out)?)
        } else {
            None
        };

        self.pending_observations.push(Observation::Command {
            command: command.to_string(),
            exit_code,
            output_id,
        });
        Ok(())
    }

    /// Record an agent note.
    pub fn observe_note(&mut self, note: &str) -> Result<()> {
        self.update_last_activity();
        self.pending_observations.push(Observation::Note {
            content: note.to_string(),
        });
        Ok(())
    }

    /// Record an agent plan.
    pub fn observe_plan(&mut self, plan: &str) -> Result<()> {
        self.update_last_activity();
        self.pending_observations.push(Observation::Plan {
            content: plan.to_string(),
        });
        Ok(())
    }

    /// Flushes pending observations to a WorkCommit.
    ///
    /// Creates a new WorkCommit with all pending observations,
    /// updates STAGE pointer, and clears the pending buffer.
    pub fn flush_step(&mut self, object_store: &ObjectStore, refs: &Refs) -> Result<ObjectId> {
        self.update_last_activity();

        // Serialize observations
        let payload = self.encode_observations()?;

        // Determine step kind based on observations
        let step_kind = self.infer_step_kind();

        // Create WorkCommit
        let work_commit = WorkCommit {
            parents: vec![self.staging_head],
            base: self.base_commit,
            session_id: self.session_id.clone(),
            created_at: self.now() as u64,
            step_kind,
            payload,
            narrative_refs: vec![],
            session_state: self.state.clone(),
            task_description: self.task_description.clone(),
        };

        // Store WorkCommit
        let work_id = object_store.put_typed(&work_commit)?;

        // Update STAGE pointer atomically
        refs.write_stage(work_id)?;

        // Update session state
        self.staging_head = work_id;
        self.step_count += 1;
        self.pending_observations.clear();

        Ok(work_id)
    }

    /// Transitions session state with validation.
    pub fn set_state(&mut self, new_state: SessionState) -> Result<()> {
        if !self.is_valid_transition(&new_state) {
            return Err(CtxError::InvalidStateTransition {
                from: format!("{:?}", self.state),
                to: format!("{:?}", new_state),
            });
        }

        self.update_last_activity();
        self.state = new_state;
        Ok(())
    }

    /// Returns current session state.
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Returns task description.
    pub fn task_description(&self) -> &str {
        &self.task_description
    }

    /// Returns time since last activity.
    pub fn idle_time(&self) -> Duration {
        let now = self.now();
        let idle_secs = (now - self.last_activity).max(0) as u64;
        Duration::from_secs(idle_secs)
    }

    /// Returns last activity timestamp.
    pub fn last_activity(&self) -> i64 {
        self.last_activity
    }

    /// Returns session creation time.
    pub fn created_at(&self) -> i64 {
        self.created_at
    }

    /// Returns session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Returns current staging head.
    pub fn staging_head(&self) -> ObjectId {
        self.staging_head
    }

    /// Returns base commit.
    pub fn base_commit(&self) -> ObjectId {
        self.base_commit
    }

    /// Returns step count.
    pub fn step_count(&self) -> u32 {
        self.step_count
    }

    /// Computes statistics about the session's observations.
    ///
    /// This walks the staging chain to count all flushed observations,
    /// plus any pending observations not yet flushed.
    pub fn stats(&self, object_store: &ObjectStore) -> SessionStats {
        let mut stats = SessionStats {
            steps_flushed: self.step_count,
            pending_observations: self.pending_observations.len(),
            ..Default::default()
        };

        let mut files_read = HashSet::new();
        let mut files_written = HashSet::new();

        // Count pending observations
        for obs in &self.pending_observations {
            Self::count_observation(&mut stats, obs, &mut files_read, &mut files_written);
        }

        // Walk staging chain and count flushed observations
        let mut current = self.staging_head;
        while current != self.base_commit {
            if let Ok(work) = object_store.get_typed::<WorkCommit>(current) {
                if let Ok(observations) = self.decode_observations(&work.payload) {
                    for obs in &observations {
                        Self::count_observation(&mut stats, obs, &mut files_read, &mut files_written);
                    }
                }
                if let Some(&parent) = work.parents.first() {
                    current = parent;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        stats.unique_files_read = files_read.len();
        stats.unique_files_written = files_written.len();
        stats
    }

    /// Helper to count a single observation.
    fn count_observation(
        stats: &mut SessionStats,
        obs: &Observation,
        files_read: &mut HashSet<String>,
        files_written: &mut HashSet<String>,
    ) {
        match obs {
            Observation::FileRead { path, .. } => {
                stats.file_reads += 1;
                files_read.insert(path.clone());
            }
            Observation::FileWrite { path, .. } => {
                stats.file_writes += 1;
                files_written.insert(path.clone());
            }
            Observation::Command { .. } => {
                stats.commands += 1;
            }
            Observation::Note { .. } => {
                stats.notes += 1;
            }
            Observation::Plan { .. } => {
                stats.plans += 1;
            }
        }
    }

    /// Returns the set of file paths touched during this session.
    ///
    /// Walks the staging chain and pending observations to collect
    /// all file paths that were read or written.
    pub fn files_touched(&self, object_store: &ObjectStore) -> (Vec<String>, Vec<String>) {
        let mut files_read = HashSet::new();
        let mut files_written = HashSet::new();

        // Pending observations
        for obs in &self.pending_observations {
            match obs {
                Observation::FileRead { path, .. } => { files_read.insert(path.clone()); }
                Observation::FileWrite { path, .. } => { files_written.insert(path.clone()); }
                _ => {}
            }
        }

        // Walk staging chain
        let mut current = self.staging_head;
        while current != self.base_commit {
            if let Ok(work) = object_store.get_typed::<WorkCommit>(current) {
                if let Ok(observations) = self.decode_observations(&work.payload) {
                    for obs in &observations {
                        match obs {
                            Observation::FileRead { path, .. } => { files_read.insert(path.clone()); }
                            Observation::FileWrite { path, .. } => { files_written.insert(path.clone()); }
                            _ => {}
                        }
                    }
                }
                if let Some(&parent) = work.parents.first() {
                    current = parent;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        let mut reads: Vec<String> = files_read.into_iter().collect();
        let mut writes: Vec<String> = files_written.into_iter().collect();
        reads.sort();
        writes.sort();
        (reads, writes)
    }

    /// Generates a progress summary from the staging chain.
    pub fn generate_progress_summary(&self, object_store: &ObjectStore) -> Result<String> {
        let mut summary = format!("Task: {}\n", self.task_description);
        summary.push_str(&format!("Steps completed: {}\n", self.step_count));
        summary.push_str(&format!("Current state: {:?}\n", self.state));

        // Walk the staging chain and summarize observations
        let mut current = self.staging_head;
        let mut step_summaries = Vec::new();

        while current != self.base_commit {
            let work: WorkCommit = object_store.get_typed(current)?;

            if let Ok(observations) = self.decode_observations(&work.payload) {
                let obs_summary = summarize_observations(&observations);
                if !obs_summary.is_empty() {
                    step_summaries.push(obs_summary);
                }
            }

            if let Some(&parent) = work.parents.first() {
                current = parent;
            } else {
                break;
            }
        }

        step_summaries.reverse(); // Show oldest first
        if !step_summaries.is_empty() {
            summary.push_str("\nRecent activity:\n");
            for (i, step_sum) in step_summaries.iter().take(5).enumerate() {
                summary.push_str(&format!("  {}. {}\n", i + 1, step_sum));
            }
        }

        Ok(summary)
    }

    fn update_last_activity(&mut self) {
        self.last_activity = if let Some(ref provider) = self.time_provider {
            provider()
        } else {
            current_timestamp()
        };
    }

    fn now(&self) -> i64 {
        if let Some(ref provider) = self.time_provider {
            provider()
        } else {
            current_timestamp()
        }
    }

    fn encode_observations(&self) -> Result<Vec<u8>> {
        postcard::to_allocvec(&self.pending_observations)
            .map_err(|e| CtxError::Serialization(format!("Failed to encode observations: {}", e)))
    }

    fn decode_observations(&self, payload: &[u8]) -> Result<Vec<Observation>> {
        postcard::from_bytes(payload)
            .map_err(|e| CtxError::Deserialization(format!("Failed to decode observations: {}", e)))
    }

    fn infer_step_kind(&self) -> StepKind {
        // Determine step kind based on observations
        for obs in &self.pending_observations {
            match obs {
                Observation::FileWrite { .. } => return StepKind::FileWrite,
                Observation::Command { .. } => return StepKind::CommandRun,
                Observation::Plan { .. } => return StepKind::Plan,
                _ => {}
            }
        }

        // Default to Note if only reads or notes
        for obs in &self.pending_observations {
            match obs {
                Observation::FileRead { .. } => return StepKind::FileRead,
                Observation::Note { .. } => return StepKind::Note,
                _ => {}
            }
        }

        StepKind::Note
    }

    fn is_valid_transition(&self, new_state: &SessionState) -> bool {
        use SessionState::*;

        match (&self.state, new_state) {
            // From Running
            (Running, AwaitingUser { .. })
            | (Running, Interrupted { .. })
            | (Running, PendingComplete { .. })
            | (Running, Aborted { .. }) => true,

            // From AwaitingUser
            (AwaitingUser { .. }, Running) | (AwaitingUser { .. }, Aborted { .. }) => true,

            // From Interrupted
            (Interrupted { .. }, Running) => true,

            // From PendingComplete
            (PendingComplete { .. }, Complete)
            | (PendingComplete { .. }, Running)
            | (PendingComplete { .. }, Aborted { .. }) => true,

            // All other transitions are invalid
            _ => false,
        }
    }
}

/// Returns the current Unix timestamp in seconds.
fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before Unix epoch")
        .as_secs() as i64
}

// Manual Debug implementation to skip time_provider field
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("task_description", &self.task_description)
            .field("state", &self.state)
            .field("base_commit", &self.base_commit)
            .field("staging_head", &self.staging_head)
            .field("session_id", &self.session_id)
            .field("created_at", &self.created_at)
            .field("last_activity", &self.last_activity)
            .field("pending_observations", &self.pending_observations)
            .field("step_count", &self.step_count)
            .finish()
    }
}

/// Summarizes a list of observations into a short string.
fn summarize_observations(observations: &[Observation]) -> String {
    let mut parts = Vec::new();

    let file_reads = observations
        .iter()
        .filter(|o| matches!(o, Observation::FileRead { .. }))
        .count();
    let file_writes = observations
        .iter()
        .filter(|o| matches!(o, Observation::FileWrite { .. }))
        .count();
    let commands = observations
        .iter()
        .filter(|o| matches!(o, Observation::Command { .. }))
        .count();
    let notes = observations
        .iter()
        .filter(|o| matches!(o, Observation::Note { .. }))
        .count();

    if file_reads > 0 {
        parts.push(format!("{} file read(s)", file_reads));
    }
    if file_writes > 0 {
        parts.push(format!("{} file write(s)", file_writes));
    }
    if commands > 0 {
        parts.push(format!("{} command(s)", commands));
    }
    if notes > 0 {
        parts.push(format!("{} note(s)", notes));
    }

    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_session_creation() {
        let session = Session::new(
            "Test task".to_string(),
            ObjectId::from_bytes([0; 32]),
            "session-123".to_string(),
            None,
        );

        assert_eq!(session.task_description(), "Test task");
        assert!(matches!(session.state(), SessionState::Running));
        assert_eq!(session.step_count(), 0);
    }

    #[test]
    fn test_observation_accumulation() {
        let mut session = Session::new(
            "Test".to_string(),
            ObjectId::from_bytes([0; 32]),
            "s1".to_string(),
            None,
        );

        session.observe_file_read("test.rs").unwrap();
        session.observe_note("Working on feature").unwrap();

        assert_eq!(session.pending_observations.len(), 2);
    }

    #[test]
    fn test_flush_creates_work_commit() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));
        let refs = Refs::new(tmp.path());

        let mut session = Session::new(
            "Test task".to_string(),
            ObjectId::from_bytes([0; 32]),
            "session-123".to_string(),
            None,
        );

        session.observe_note("Test").unwrap();

        let work_id = session.flush_step(&store, &refs).unwrap();

        // Verify WorkCommit was stored
        let work: WorkCommit = store.get_typed(work_id).unwrap();
        assert_eq!(work.session_id, session.session_id());

        // Verify STAGE was updated
        assert_eq!(refs.read_stage().unwrap(), Some(work_id));

        // Verify pending observations cleared
        assert_eq!(session.pending_observations.len(), 0);
        assert_eq!(session.step_count(), 1);
    }

    #[test]
    fn test_state_transitions() {
        let mut session = Session::new(
            "Test".to_string(),
            ObjectId::from_bytes([0; 32]),
            "s1".to_string(),
            None,
        );

        // Running -> AwaitingUser
        session
            .set_state(SessionState::AwaitingUser {
                question: "What color?".to_string(),
                asked_at: 12345,
            })
            .unwrap();

        assert!(matches!(session.state(), SessionState::AwaitingUser { .. }));

        // AwaitingUser -> Running
        session.set_state(SessionState::Running).unwrap();

        assert!(matches!(session.state(), SessionState::Running));
    }

    #[test]
    fn test_invalid_state_transition() {
        let mut session = Session::new(
            "Test".to_string(),
            ObjectId::from_bytes([0; 32]),
            "s1".to_string(),
            None,
        );

        // Cannot go directly from Running to Complete
        let result = session.set_state(SessionState::Complete);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CtxError::InvalidStateTransition { .. }
        ));
    }

    #[test]
    fn test_idle_time() {
        let session = Session::new(
            "Test".to_string(),
            ObjectId::from_bytes([0; 32]),
            "s1".to_string(),
            None,
        );

        let idle = session.idle_time();
        assert!(idle.as_secs() < 2); // Should be very fresh
    }
}
