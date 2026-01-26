//! Repository handle providing the main CTX API.

use crate::config::{CleanupReport, StaleSessionConfig, StaleSessionStatus};
use crate::error::{CtxError, Result};
use crate::index::Index;
use crate::refs::Refs;
use crate::session::Session;
use crate::staging;
use crate::types::{Commit, CommitType, Tree};
use crate::{ObjectId, ObjectStore};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::warn;

/// CTX repository handle.
///
/// Provides the main API for interacting with a CTX repository.
pub struct CtxRepo {
    /// Root directory containing the repository (parent of .ctx).
    root: PathBuf,
    /// Object store for content-addressed storage.
    object_store: ObjectStore,
    /// Reference management.
    refs: Refs,
    /// Index for fast lookups (lazy-loaded).
    index: Option<Index>,
    /// Active session (if any).
    active_session: Option<Session>,
    /// Session lock guard (held while session is active to prevent concurrent access).
    session_lock: Option<LockGuard>,
    /// Time provider for testing (None = use system time).
    time_provider: Option<std::sync::Arc<dyn Fn() -> i64 + Send + Sync>>,
}

impl CtxRepo {
    /// Opens an existing CTX repository.
    ///
    /// # Errors
    ///
    /// Returns an error if the .ctx directory doesn't exist or is invalid.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ctx_core::CtxRepo;
    ///
    /// let repo = CtxRepo::open(".").unwrap();
    /// ```
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let root = path.as_ref().to_path_buf();
        let ctx_dir = root.join(".ctx");

        if !ctx_dir.exists() {
            return Err(CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Not a CTX repository: {}", root.display()),
            )));
        }

        let object_store = ObjectStore::new(ctx_dir.join("objects"));
        let refs = Refs::new(&ctx_dir);

        Ok(Self {
            root,
            object_store,
            refs,
            index: None,
            active_session: None,
            session_lock: None,
            time_provider: None,
        })
    }

    /// Sets a custom time provider for testing.
    ///
    /// This allows injecting controlled time for testing stale session detection
    /// and other time-dependent behavior. In production, just use `open()` or `init()`
    /// without calling this method to get normal system time.
    pub fn with_time_provider(
        mut self,
        provider: impl Fn() -> i64 + Send + Sync + 'static,
    ) -> Self {
        self.time_provider = Some(std::sync::Arc::new(provider));
        self
    }

    /// Initializes a new CTX repository.
    ///
    /// Creates the .ctx directory structure and initial commit.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The .ctx directory already exists
    /// - Directory creation fails
    /// - Initial commit creation fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ctx_core::CtxRepo;
    ///
    /// let repo = CtxRepo::init(".").unwrap();
    /// ```
    pub fn init(path: impl AsRef<Path>) -> Result<Self> {
        let root = path.as_ref().to_path_buf();
        let ctx_dir = root.join(".ctx");

        if ctx_dir.exists() {
            return Err(CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "CTX repository already exists in this directory",
            )));
        }

        // Create directory structure
        fs::create_dir_all(ctx_dir.join("objects"))?;
        fs::create_dir_all(ctx_dir.join("refs"))?;
        fs::create_dir_all(ctx_dir.join("narrative/log"))?;
        fs::create_dir_all(ctx_dir.join("narrative/tasks"))?;
        fs::create_dir_all(ctx_dir.join("index"))?;

        // Create default config
        let config = r#"# CTX Configuration
[repository]
version = "1"

[ingestion]
snapshot_on_read = true
extract_on_read = true
parse_diagnostics = true

[session]
idle_timeout_hours = 24
stale_timeout_days = 7
"#;
        fs::write(ctx_dir.join("config.toml"), config)?;

        // Create .gitignore for rebuildable content
        let gitignore = r#"# CTX rebuildable indexes
index/
DERIVED/
LOCK
*.tmp
"#;
        fs::write(ctx_dir.join(".gitignore"), gitignore)?;

        // Create initial narrative README
        let readme = r#"# Project Context

This directory contains context management data for coding agents.

## Structure

- `objects/` - Content-addressed immutable objects
- `refs/` - Pointers to commits
- `narrative/` - Human-readable documentation
- `index/` - Rebuildable indexes (gitignored)

## Usage

The CTX system is designed to be used by coding agents, not directly by humans.
However, you can inspect and edit narrative files in the `narrative/` directory.

## Getting Started

This repository has been initialized. Coding agents can now use the CTX API
to store and retrieve context across sessions.
"#;
        fs::write(ctx_dir.join("narrative/README.md"), readme)?;

        // Initialize object store and refs
        let object_store = ObjectStore::new(ctx_dir.join("objects"));
        let refs = Refs::new(&ctx_dir);

        // Create empty tree
        let empty_tree = Tree::new(vec![]);
        let tree_id = object_store.put_typed(&empty_tree)?;

        // Create initial commit
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before Unix epoch")
            .as_secs();

        let initial_commit = Commit {
            parents: vec![],
            timestamp_unix: now,
            message: "Initial commit".to_string(),
            root_tree: tree_id,
            edge_batches: vec![],
            narrative_refs: vec![],
            cargo_snapshot: None,
            rust_snapshot: None,
            diagnostics_snapshot: None,
            commit_type: None,
        };

        let commit_id = object_store.put_typed(&initial_commit)?;

        // Set HEAD and refs/main
        refs.write_head(commit_id)?;
        refs.write_ref("main", commit_id)?;

        Ok(Self {
            root,
            object_store,
            refs,
            index: None,
            active_session: None,
            session_lock: None,
            time_provider: None,
        })
    }

    /// Returns the repository root (parent of `.ctx` directory).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the .ctx directory path.
    pub fn ctx_dir(&self) -> PathBuf {
        self.root.join(".ctx")
    }

    /// Returns a reference to the content-addressed object store.
    ///
    /// Use this to directly access stored objects when you already have ObjectIds.
    pub fn object_store(&self) -> &ObjectStore {
        &self.object_store
    }

    /// Returns a mutable reference to the object store.
    pub fn object_store_mut(&mut self) -> &mut ObjectStore {
        &mut self.object_store
    }

    /// Returns a reference to the refs manager.
    pub fn refs(&self) -> &Refs {
        &self.refs
    }

    /// Returns the current HEAD commit ID.
    ///
    /// # Errors
    ///
    /// Returns an error if HEAD doesn't exist or is invalid.
    pub fn head_id(&self) -> Result<ObjectId> {
        self.refs.read_head()
    }

    /// Returns the current HEAD commit.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - HEAD doesn't exist or is invalid
    /// - The commit object can't be read
    /// - The commit object is corrupted
    pub fn head(&self) -> Result<Commit> {
        let id = self.head_id()?;
        self.object_store.get_typed(id)
    }

    /// Returns a NarrativeSpace for this repository.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ctx_core::CtxRepo;
    ///
    /// let repo = CtxRepo::open(".").unwrap();
    /// let ns = repo.narrative();
    /// ns.ensure_structure().unwrap();
    /// ```
    pub fn narrative(&self) -> crate::narrative::NarrativeSpace {
        crate::narrative::NarrativeSpace::new(self.ctx_dir())
    }

    /// Creates a new commit with the given message and optional narrative refs.
    ///
    /// If `narrative_refs` is `None`, automatically snapshots changed narrative files.
    /// If `narrative_refs` is `Some(vec)`, uses the provided refs directly.
    ///
    /// # Arguments
    ///
    /// * `message` - Commit message
    /// * `narrative_refs` - Optional explicit narrative refs (auto-detected if None)
    /// * `role` - Role string for auto-detected narrative refs (e.g., "user", "agent")
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ctx_core::CtxRepo;
    ///
    /// let repo = CtxRepo::open(".").unwrap();
    /// let commit_id = repo.commit("Update docs", None, "user").unwrap();
    /// println!("Created commit: {}", commit_id.as_hex());
    /// ```
    pub fn commit(
        &self,
        message: &str,
        narrative_refs: Option<Vec<crate::types::NarrativeRef>>,
        role: &str,
    ) -> Result<ObjectId> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before Unix epoch")
            .as_secs();

        // Get current HEAD
        let parent_id = self.head_id()?;
        let parent_commit: Commit = self.object_store.get_typed(parent_id)?;

        // Determine narrative refs
        let refs = match narrative_refs {
            Some(r) => r,
            None => {
                let ns = self.narrative();
                ns.snapshot_changed(&self.object_store, &parent_commit.narrative_refs, role)?
            }
        };

        // Create new commit
        let new_commit = Commit {
            parents: vec![parent_id],
            timestamp_unix: now,
            message: message.to_string(),
            root_tree: parent_commit.root_tree, // Unchanged for now
            edge_batches: vec![],               // Empty for basic narrative commits
            narrative_refs: refs,
            cargo_snapshot: parent_commit.cargo_snapshot,
            rust_snapshot: parent_commit.rust_snapshot,
            diagnostics_snapshot: parent_commit.diagnostics_snapshot,
            commit_type: None,
        };

        let commit_id = self.object_store.put_typed(&new_commit)?;

        // Update HEAD and refs/main
        self.refs.write_head(commit_id)?;
        self.refs.write_ref("main", commit_id)?;

        Ok(commit_id)
    }

    /// Returns the index, creating it if it doesn't exist.
    ///
    /// The index is lazily loaded on first access.
    ///
    /// # Errors
    ///
    /// Returns an error if the index can't be loaded or created.
    pub fn index(&mut self) -> Result<&Index> {
        if self.index.is_none() {
            let index_path = self.ctx_dir().join("index/index.redb");

            // Try to open existing index
            match Index::open(&index_path)? {
                Some(idx) => self.index = Some(idx),
                None => {
                    // Rebuild if missing
                    let head = self.head_id()?;
                    let idx = Index::rebuild_from_objects(&index_path, &self.object_store, head)?;
                    self.index = Some(idx);
                }
            }
        }

        Ok(self.index.as_ref().unwrap())
    }

    /// Gets a mutable reference to the index, loading it if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the index can't be loaded or rebuilt.
    pub fn index_mut(&mut self) -> Result<&mut Index> {
        if self.index.is_none() {
            let index_path = self.ctx_dir().join("index/index.redb");

            // Try to open existing index
            match Index::open(&index_path)? {
                Some(idx) => self.index = Some(idx),
                None => {
                    // Rebuild if missing
                    let head = self.head_id()?;
                    let idx = Index::rebuild_from_objects(&index_path, &self.object_store, head)?;
                    self.index = Some(idx);
                }
            }
        }

        Ok(self.index.as_mut().unwrap())
    }

    /// Rebuilds the index from scratch.
    ///
    /// This is useful if the index is corrupted or out of date.
    ///
    /// # Errors
    ///
    /// Returns an error if the index can't be rebuilt.
    pub fn rebuild_index(&mut self) -> Result<()> {
        let index_path = self.ctx_dir().join("index/index.redb");
        let head = self.head_id()?;

        // Drop existing index handle
        self.index = None;

        // Rebuild
        let idx = Index::rebuild_from_objects(&index_path, &self.object_store, head)?;
        self.index = Some(idx);

        Ok(())
    }

    /// Starts a new session for the given task.
    ///
    /// Creates initial WorkCommit with SessionStart step kind.
    /// Updates STAGE pointer.
    ///
    /// # Errors
    /// Returns error if a session is already active.
    pub fn start_session(&mut self, task: &str) -> Result<&mut Session> {
        if self.active_session.is_some() {
            return Err(CtxError::SessionAlreadyActive(task.to_string()));
        }

        // Acquire lock and store it to keep it alive
        let lock = self.acquire_lock()?;

        // Get current HEAD as base
        let base_commit = self.head_id()?;

        // Generate session ID
        let session_id = uuid::Uuid::new_v4().to_string();

        // Create session
        let mut session = Session::new(
            task.to_string(),
            base_commit,
            session_id,
            self.time_provider.clone(),
        );

        // Create initial WorkCommit (SessionStart)
        session.flush_step(&self.object_store, &self.refs)?;

        self.active_session = Some(session);
        self.session_lock = Some(lock);

        Ok(self.active_session.as_mut().unwrap())
    }

    /// Compacts the current session into a canonical commit.
    ///
    /// Walks staging chain, aggregates work, creates Commit,
    /// updates HEAD and refs/main, deletes STAGE.
    pub fn compact_session(&mut self, message: &str) -> Result<ObjectId> {
        self.compact_session_with_type(message, CommitType::Normal)
    }

    /// Compacts with a specific commit type.
    pub fn compact_session_with_type(
        &mut self,
        message: &str,
        commit_type: CommitType,
    ) -> Result<ObjectId> {
        let session = self
            .active_session
            .as_ref()
            .ok_or(CtxError::NoActiveSession)?;

        let staging_head = session.staging_head();
        let base_commit = session.base_commit();

        // Gather session info for narrative before compacting
        let task_desc = session.task_description().to_string();
        let (files_read, files_written) = session.files_touched(&self.object_store);

        // Compact staging into canonical commit
        let commit = staging::compact_staging(
            staging_head,
            base_commit,
            message,
            commit_type,
            &self.object_store,
        )?;

        // Store the commit
        let commit_id = self.object_store.put_typed(&commit)?;

        // Update HEAD and refs/main
        self.refs.write_head(commit_id)?;
        self.refs.write_ref("main", commit_id)?;

        // Delete STAGE
        self.refs.delete_stage()?;

        // Write narrative log entry for this session
        // This makes session history available for future build_pack() retrieval.
        self.write_session_narrative(&task_desc, &files_read, &files_written, message);

        // Clear active session and release lock
        self.active_session = None;
        self.session_lock = None;

        Ok(commit_id)
    }

    /// Writes a narrative log entry summarizing a completed session.
    ///
    /// Creates or appends to a daily log file in `.ctx/narrative/log/`.
    /// Failures are logged as warnings but do not propagate — narrative
    /// is best-effort and should never block session compaction.
    fn write_session_narrative(
        &self,
        task: &str,
        files_read: &[String],
        files_written: &[String],
        message: &str,
    ) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();

        // Compute date and time strings from Unix timestamp
        // (simple arithmetic to avoid chrono dependency)
        let days_since_epoch = secs / 86400;
        let time_of_day = secs % 86400;
        let hours = time_of_day / 3600;
        let minutes = (time_of_day % 3600) / 60;

        let (year, month, day) = days_to_ymd(days_since_epoch);
        let date = format!("{:04}-{:02}-{:02}", year, month, day);
        let time = format!("{:02}:{:02}", hours, minutes);

        // Build narrative entry
        let mut entry = format!("**Task:** {}\n", task);
        entry.push_str(&format!("**Result:** {}\n", message));

        if !files_written.is_empty() {
            entry.push_str("\n**Files modified:**\n");
            for f in files_written.iter().take(20) {
                entry.push_str(&format!("- `{}`\n", f));
            }
            if files_written.len() > 20 {
                entry.push_str(&format!("- ... and {} more\n", files_written.len() - 20));
            }
        }

        if !files_read.is_empty() {
            entry.push_str("\n**Files read:**\n");
            for f in files_read.iter().take(20) {
                entry.push_str(&format!("- `{}`\n", f));
            }
            if files_read.len() > 20 {
                entry.push_str(&format!("- ... and {} more\n", files_read.len() - 20));
            }
        }

        let narrative = self.narrative();
        if let Err(e) = narrative.append_log(&date, &time, &entry) {
            warn!("Failed to write session narrative log: {}", e);
        }
    }

    /// Aborts the current session, discarding all work.
    ///
    /// # Warning
    ///
    /// This operation creates a commit marked as `Abandoned`, which preserves
    /// the work history for debugging but marks it as intentionally discarded.
    /// The work is NOT lost (it's committed), but it's marked as abandoned.
    ///
    /// If you want to preserve the work for later continuation, use
    /// `flush_active_session()` instead.
    ///
    /// # Arguments
    ///
    /// * `reason` - Explanation for why the session was aborted
    pub fn abort_session(&mut self, reason: &str) -> Result<ObjectId> {
        let message = format!("Aborted: {}", reason);
        self.compact_session_with_type(&message, CommitType::Abandoned)
    }

    /// Recovers a session from staging (e.g., after crash).
    ///
    /// Returns None if no STAGE pointer exists.
    pub fn recover_session(&mut self) -> Result<Option<&mut Session>> {
        if let Some(staging_head) = self.refs.read_stage()? {
            let session = Session::from_staging(
                staging_head,
                &self.object_store,
                self.time_provider.clone(),
            )?;

            self.active_session = Some(session);
            Ok(self.active_session.as_mut())
        } else {
            Ok(None)
        }
    }

    /// Checks if there's an active session.
    pub fn has_active_session(&self) -> bool {
        self.active_session.is_some()
    }

    /// Returns reference to active session.
    pub fn active_session(&self) -> Option<&Session> {
        self.active_session.as_ref()
    }

    /// Returns mutable reference to active session.
    pub fn active_session_mut(&mut self) -> Option<&mut Session> {
        self.active_session.as_mut()
    }

    /// Flushes the active session's current step.
    ///
    /// Convenience method that handles the borrowing internally.
    pub fn flush_active_session(&mut self) -> Result<ObjectId> {
        let session = self
            .active_session
            .as_mut()
            .ok_or(CtxError::NoActiveSession)?;
        session.flush_step(&self.object_store, &self.refs)
    }

    /// Observes a file write in the active session.
    ///
    /// Convenience method that handles the borrowing internally.
    pub fn observe_file_write(&mut self, path: &str, content: &[u8]) -> Result<ObjectId> {
        let session = self
            .active_session
            .as_mut()
            .ok_or(CtxError::NoActiveSession)?;
        session.observe_file_write(path, content, &self.object_store)
    }

    /// Observes a file read in the active session.
    pub fn observe_file_read(&mut self, path: &str) -> Result<()> {
        let session = self
            .active_session
            .as_mut()
            .ok_or(CtxError::NoActiveSession)?;
        session.observe_file_read(path)
    }

    /// Observes a file read with content in the active session.
    ///
    /// This captures the exact content the agent read, enabling:
    /// - Temporal reconstruction ("what did the agent see at step 5?")
    /// - True context for decision analysis
    /// - Reproducible agent behavior
    pub fn observe_file_read_with_content(&mut self, path: &str, content: &[u8]) -> Result<()> {
        let session = self
            .active_session
            .as_mut()
            .ok_or(CtxError::NoActiveSession)?;
        session.observe_file_read_with_content(path, content, &self.object_store)
    }

    /// Observes a note in the active session.
    pub fn observe_note(&mut self, note: &str) -> Result<()> {
        let session = self
            .active_session
            .as_mut()
            .ok_or(CtxError::NoActiveSession)?;
        session.observe_note(note)
    }

    /// Observes a command in the active session.
    pub fn observe_command(
        &mut self,
        command: &str,
        exit_code: Option<i32>,
        output: Option<&[u8]>,
    ) -> Result<()> {
        let session = self
            .active_session
            .as_mut()
            .ok_or(CtxError::NoActiveSession)?;
        session.observe_command(command, exit_code, output, &self.object_store)
    }

    /// Checks if current session is stale.
    pub fn check_stale_session(&self, config: &StaleSessionConfig) -> StaleSessionStatus {
        let session = match &self.active_session {
            Some(s) => s,
            None => return StaleSessionStatus::NoSession,
        };

        let idle_secs = session.idle_time().as_secs();
        let task = session.task_description().to_string();

        if idle_secs >= config.auto_compact_threshold_secs {
            StaleSessionStatus::ShouldAutoCompact { task, idle_secs }
        } else if idle_secs >= config.ask_threshold_secs {
            StaleSessionStatus::ShouldAsk { task, idle_secs }
        } else {
            StaleSessionStatus::Fresh { task, idle_secs }
        }
    }

    /// Cleans up sessions that exceed max idle time.
    pub fn cleanup_stale_sessions(&mut self, max_age: Duration) -> Result<CleanupReport> {
        let mut report = CleanupReport::default();

        if let Some(session) = &self.active_session {
            if session.idle_time() > max_age {
                let task = session.task_description().to_string();
                let idle_duration_secs = session.idle_time().as_secs();

                let message = format!(
                    "Auto-saved stale session (idle for {} seconds): {}",
                    idle_duration_secs, task
                );

                self.compact_session_with_type(
                    &message,
                    CommitType::StaleAutoCompact { idle_duration_secs },
                )?;

                report.sessions_compacted += 1;
                report.compacted_tasks.push(task);
            }
        }

        Ok(report)
    }

    /// Load edge batches from the object store.
    ///
    /// Edge batches are stored as separate objects to avoid duplication.
    /// Commits and the index only store ObjectIds that reference them.
    ///
    /// # Errors
    ///
    /// Returns an error if any edge batch cannot be loaded.
    pub fn load_edge_batches(
        &self,
        batch_ids: &[ObjectId],
    ) -> Result<Vec<crate::types::EdgeBatch>> {
        let mut batches = Vec::with_capacity(batch_ids.len());
        for batch_id in batch_ids {
            let batch = self.object_store.get_typed(*batch_id)?;
            batches.push(batch);
        }
        Ok(batches)
    }

    /// Build a prompt pack from a query.
    ///
    /// This runs the retrieval pipeline to compile relevant context for an LLM.
    ///
    /// # Errors
    ///
    /// Returns an error if the pack can't be built.
    pub fn build_pack(
        &mut self,
        query: &str,
        config: &crate::pack::RetrievalConfig,
    ) -> Result<crate::pack::PromptPack> {
        crate::pack::build_pack(self, query, config)
    }

    /// Analyze all Rust files in the project using rust-analyzer.
    ///
    /// Spawns rust-analyzer, analyzes all .rs files, extracts semantic edges,
    /// and stores them as an EdgeBatch.
    ///
    /// # Returns
    ///
    /// AnalysisReport with statistics about what was analyzed.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - rust-analyzer is not installed
    /// - Analysis fails
    /// - Edge storage fails
    pub fn analyze_rust(&mut self) -> Result<AnalysisReport> {
        use crate::lsp::{build_edges_from_analysis, RustAnalyzer};
        use crate::types::EdgeBatch;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Check availability
        if !RustAnalyzer::is_available() {
            return Err(CtxError::RustAnalyzerNotFound);
        }

        // Load previous analysis hashes for incremental analysis
        let hash_cache_path = self.root.join(".ctx").join("analysis_hashes.json");
        let prev_hashes: std::collections::HashMap<String, u64> =
            if let Ok(data) = std::fs::read_to_string(&hash_cache_path) {
                serde_json::from_str(&data).unwrap_or_default()
            } else {
                std::collections::HashMap::new()
            };

        // Start rust-analyzer
        let mut analyzer = RustAnalyzer::start(&self.root)?;

        // Find all Rust files
        let rust_files = Self::find_rust_files(&self.root)?;

        let mut all_edges = Vec::new();
        let mut files_analyzed = 0;
        let mut files_skipped = 0;
        let mut symbols_found = 0;
        let mut calls_resolved = 0;
        let mut file_blobs: Vec<(String, ObjectId)> = Vec::new();
        let mut new_hashes: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        for file in rust_files {
            // Read file content and compute hash for incremental analysis
            let file_content = match std::fs::read(&file) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Warning: Failed to read {}: {}", file.display(), e);
                    continue;
                }
            };

            let mut hasher = DefaultHasher::new();
            file_content.hash(&mut hasher);
            let content_hash = hasher.finish();

            let file_canonical = file.canonicalize()?;
            let file_path = file_canonical.to_string_lossy().to_string();

            // Store hash for next run
            new_hashes.insert(file_path.clone(), content_hash);

            // Skip unchanged files
            if prev_hashes.get(&file_path) == Some(&content_hash) {
                files_skipped += 1;
                continue;
            }

            match analyzer.analyze_file(&file) {
                Ok(analysis) => {
                    files_analyzed += 1;
                    symbols_found += analysis.items.len();
                    calls_resolved += analysis.calls.len();

                    // Store file content as blob (FIX for prompt pack retrieval)
                    let file_blob_id = self.object_store.put_blob(&file_content)?;
                    file_blobs.push((file_path.clone(), file_blob_id));

                    let commit_id = self.head_id()?;
                    let edges =
                        build_edges_from_analysis(&analysis, &file_path, &file_content, commit_id);
                    all_edges.extend(edges);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to analyze {}: {}", file.display(), e);
                }
            }
        }

        tracing::info!(
            "Incremental analysis: {} files analyzed, {} skipped (unchanged)",
            files_analyzed,
            files_skipped
        );

        // Shutdown analyzer
        analyzer.shutdown()?;

        // Save analysis hashes for next incremental run
        if let Ok(json) = serde_json::to_string(&new_hashes) {
            if let Err(e) = std::fs::write(&hash_cache_path, json) {
                warn!("Failed to save analysis hash cache: {}", e);
            }
        }

        // If no files were analyzed (all unchanged), return early
        if files_analyzed == 0 {
            let head = self.head_id()?;
            return Ok(AnalysisReport {
                files_analyzed: 0,
                symbols_found: 0,
                calls_resolved: 0,
                edges_generated: 0,
                edge_batch_id: head, // placeholder
                commit_id: head,
            });
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before Unix epoch")
            .as_secs();

        // Store edges as EdgeBatch
        let edge_batch = EdgeBatch {
            edges: all_edges.clone(),
            created_at: now,
        };

        let batch_id = self.object_store.put_typed(&edge_batch)?;

        // Create commit with edge batch
        let parent_id = self.head_id()?;
        let parent_commit: Commit = self.object_store.get_typed(parent_id)?;

        let commit = Commit {
            parents: vec![parent_id],
            timestamp_unix: now,
            message: format!(
                "Rust analysis: {} files, {} symbols, {} calls",
                files_analyzed, symbols_found, calls_resolved
            ),
            root_tree: parent_commit.root_tree,
            edge_batches: vec![batch_id],
            narrative_refs: vec![],
            cargo_snapshot: parent_commit.cargo_snapshot,
            rust_snapshot: parent_commit.rust_snapshot,
            diagnostics_snapshot: parent_commit.diagnostics_snapshot,
            commit_type: None,
        };

        let commit_id = self.object_store.put_typed(&commit)?;

        // Update HEAD and refs/main
        self.refs.write_head(commit_id)?;
        self.refs.write_ref("main", commit_id)?;

        // Load edge batches before we borrow the index mutably
        let edge_batches: Vec<_> = commit
            .edge_batches
            .iter()
            .map(|id| self.object_store.get_typed(*id))
            .collect::<Result<_>>()?;

        // Incrementally add edges from this commit to the index and index file paths
        // This is far more efficient than rebuilding the entire index
        let index = self.index_mut()?;
        index.add_commit_edges(commit_id, &commit, &edge_batches)?;
        index.index_file_paths(&file_blobs)?;

        Ok(AnalysisReport {
            files_analyzed,
            symbols_found,
            calls_resolved,
            edges_generated: all_edges.len(),
            edge_batch_id: batch_id,
            commit_id,
        })
    }

    /// Analyze a single Rust file.
    pub fn analyze_rust_file(&mut self, path: &Path) -> Result<FileAnalysisReport> {
        use crate::lsp::{build_edges_from_analysis, RustAnalyzer};
        use crate::types::EdgeBatch;

        if !RustAnalyzer::is_available() {
            return Err(CtxError::RustAnalyzerNotFound);
        }

        let mut analyzer = RustAnalyzer::start(&self.root)?;
        let analysis = analyzer.analyze_file(path)?;
        analyzer.shutdown()?;

        // Read file content for ObjectId computation
        // Canonicalize to ensure absolute paths (FIX for path matching)
        let path_canonical = path.canonicalize()?;
        let file_path = path_canonical.to_string_lossy().to_string();
        let file_content = std::fs::read(path)?;

        // Store file content as blob (FIX for prompt pack retrieval)
        let file_blob_id = self.object_store.put_blob(&file_content)?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before Unix epoch")
            .as_secs();

        let current_head = self.head_id()?;
        let edges = build_edges_from_analysis(&analysis, &file_path, &file_content, current_head);

        let edge_batch = EdgeBatch {
            edges: edges.clone(),
            created_at: now,
        };

        let batch_id = self.object_store.put_typed(&edge_batch)?;

        // Create commit with edge batch
        let parent_id = self.head_id()?;
        let parent_commit: Commit = self.object_store.get_typed(parent_id)?;

        let commit = Commit {
            parents: vec![parent_id],
            timestamp_unix: now,
            message: format!(
                "Rust analysis: {} ({} symbols, {} calls)",
                file_path,
                analysis.items.len(),
                analysis.calls.len()
            ),
            root_tree: parent_commit.root_tree,
            edge_batches: vec![batch_id],
            narrative_refs: vec![],
            cargo_snapshot: parent_commit.cargo_snapshot,
            rust_snapshot: parent_commit.rust_snapshot,
            diagnostics_snapshot: parent_commit.diagnostics_snapshot,
            commit_type: None,
        };

        let new_commit_id = self.object_store.put_typed(&commit)?;

        // Update HEAD and refs/main
        self.refs.write_head(new_commit_id)?;
        self.refs.write_ref("main", new_commit_id)?;

        // Load edge batches before we borrow the index mutably
        let edge_batches: Vec<_> = commit
            .edge_batches
            .iter()
            .map(|id| self.object_store.get_typed(*id))
            .collect::<Result<_>>()?;

        // Incrementally add edges from this commit to the index
        self.index_mut()?
            .add_commit_edges(new_commit_id, &commit, &edge_batches)?;

        // Index the file path → blob mapping for retrieval (FIX for prompt pack)
        self.index_mut()?
            .index_file_path(&file_path, file_blob_id)?;

        Ok(FileAnalysisReport {
            path: path.to_path_buf(),
            symbols: analysis.items.len(),
            calls: analysis.calls.len(),
            edges: edges.len(),
            edge_batch_id: batch_id,
            commit_id: new_commit_id,
        })
    }

    /// Find all Rust source files in a directory.
    fn find_rust_files(dir: &Path) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();

        if !dir.is_dir() {
            return Ok(files);
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Skip target directory
                if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                    continue;
                }

                // Recurse
                files.extend(Self::find_rust_files(&path)?);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }

        Ok(files)
    }

    /// Analyze Cargo workspace and extract dependency graph.
    ///
    /// Runs `cargo metadata`, parses the output, extracts edges,
    /// and creates a commit with the edge batch.
    ///
    /// # Returns
    ///
    /// CargoAnalysisReport with statistics about what was analyzed.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - cargo is not installed
    /// - No Cargo.toml found
    /// - cargo metadata fails
    /// - Edge storage fails
    pub fn analyze_cargo(&mut self) -> Result<crate::cargo::CargoAnalysisReport> {
        use crate::cargo::{extract_cargo_edges, parse_cargo_metadata, run_cargo_metadata};
        use crate::types::EdgeBatch;

        // Check availability
        if !crate::cargo::is_available() {
            return Err(CtxError::CargoNotFound);
        }

        // Run cargo metadata
        let json = run_cargo_metadata(&self.root)?;
        let snapshot = parse_cargo_metadata(&json)?;

        // Store snapshot as typed object
        let snapshot_id = self.object_store.put_typed(&snapshot)?;

        // Extract edges
        let commit_id = self.head_id()?;
        let edges = extract_cargo_edges(&snapshot, commit_id);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before Unix epoch")
            .as_secs();

        // Store edge batch
        let edge_batch = EdgeBatch {
            edges: edges.clone(),
            created_at: now,
        };
        let batch_id = self.object_store.put_typed(&edge_batch)?;

        // Create commit
        let parent_id = self.head_id()?;
        let parent_commit: Commit = self.object_store.get_typed(parent_id)?;

        let commit = Commit {
            parents: vec![parent_id],
            timestamp_unix: now,
            message: format!(
                "Cargo analysis: {} packages, {} targets",
                snapshot.packages.len(),
                snapshot
                    .packages
                    .iter()
                    .map(|p| p.targets.len())
                    .sum::<usize>()
            ),
            root_tree: parent_commit.root_tree,
            edge_batches: vec![batch_id],
            narrative_refs: vec![],
            cargo_snapshot: Some(snapshot_id), // Store snapshot reference
            rust_snapshot: parent_commit.rust_snapshot,
            diagnostics_snapshot: parent_commit.diagnostics_snapshot,
            commit_type: None,
        };

        let new_commit_id = self.object_store.put_typed(&commit)?;

        // Update refs
        self.refs.write_head(new_commit_id)?;
        self.refs.write_ref("main", new_commit_id)?;

        // Load edge batches before we borrow the index mutably
        let edge_batches: Vec<_> = commit
            .edge_batches
            .iter()
            .map(|id| self.object_store.get_typed(*id))
            .collect::<Result<_>>()?;

        // Incrementally add edges from this commit to the index
        self.index_mut()?
            .add_commit_edges(new_commit_id, &commit, &edge_batches)?;

        Ok(crate::cargo::CargoAnalysisReport {
            packages_found: snapshot.packages.len(),
            targets_found: snapshot.packages.iter().map(|p| p.targets.len()).sum(),
            dependencies_found: snapshot.packages.iter().map(|p| p.dependencies.len()).sum(),
            edges_generated: edges.len(),
            snapshot_id,
            edge_batch_id: batch_id,
            commit_id: new_commit_id,
        })
    }

    /// Acquires exclusive lock on repository.
    ///
    /// The lock file contains the PID of the owning process. If the lock is held
    /// by a dead process (stale lock), it will be automatically cleaned up.
    fn acquire_lock(&self) -> Result<LockGuard> {
        let lock_path = self.ctx_dir().join("LOCK");
        self.acquire_lock_with_retry(&lock_path, 0)
    }

    /// Internal helper for lock acquisition with retry count.
    fn acquire_lock_with_retry(&self, lock_path: &Path, retry_count: u32) -> Result<LockGuard> {
        // Limit retries to prevent infinite loops
        if retry_count > 2 {
            return Err(CtxError::RepositoryLocked);
        }

        // Try to create the lock file exclusively (fails if already exists)
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(mut file) => {
                // Write our PID to the lock file
                let pid = std::process::id();
                writeln!(file, "{}", pid)?;
                file.flush()?;

                // Acquire file lock for additional safety
                file.try_lock_exclusive()
                    .map_err(|_| CtxError::RepositoryLocked)?;

                Ok(LockGuard {
                    file: Some(file),
                    path: lock_path.to_path_buf(),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Lock file exists - check if the holder is still alive
                self.handle_existing_lock(lock_path, retry_count)
            }
            Err(e) => Err(CtxError::Io(e)),
        }
    }

    /// Handle the case where a lock file already exists.
    fn handle_existing_lock(&self, lock_path: &Path, retry_count: u32) -> Result<LockGuard> {
        // Try to read the PID from the lock file
        match fs::read_to_string(lock_path) {
            Ok(content) => {
                if let Ok(pid) = content.trim().parse::<u32>() {
                    if is_process_alive(pid) {
                        // Process is still alive - lock is legitimately held
                        return Err(CtxError::SessionLockHeld { pid });
                    }

                    // Process is dead - stale lock
                    warn!(
                        pid = pid,
                        "Detected stale lock from dead process, cleaning up"
                    );

                    // Remove the stale lock and retry
                    if let Err(e) = fs::remove_file(lock_path) {
                        // If removal fails, it might have been cleaned up by another process
                        if e.kind() != std::io::ErrorKind::NotFound {
                            return Err(CtxError::Io(e));
                        }
                    }

                    // Retry acquiring the lock
                    return self.acquire_lock_with_retry(lock_path, retry_count + 1);
                }

                // Lock file exists but has invalid content
                // This could be a race condition or corruption - try to clean up
                warn!("Lock file has invalid content, attempting cleanup");
                let _ = fs::remove_file(lock_path);
                self.acquire_lock_with_retry(lock_path, retry_count + 1)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Lock file was removed between our check and read - retry
                self.acquire_lock_with_retry(lock_path, retry_count + 1)
            }
            Err(_) => {
                // Can't read lock file - assume it's locked
                Err(CtxError::RepositoryLocked)
            }
        }
    }

    /// Run garbage collection on the repository.
    ///
    /// See `crate::gc::gc` for details.
    pub fn gc(&mut self, config: crate::gc::GcConfig) -> Result<crate::gc::GcReport> {
        crate::gc::gc(&self.refs, &mut self.object_store, config, None)
    }

    /// Run garbage collection with progress reporting.
    pub fn gc_with_progress(
        &mut self,
        config: crate::gc::GcConfig,
        progress: &crate::gc::GcProgressCallback,
    ) -> Result<crate::gc::GcReport> {
        crate::gc::gc(&self.refs, &mut self.object_store, config, Some(progress))
    }

    /// Verify repository integrity.
    ///
    /// See `crate::verify::verify` for details.
    pub fn verify(
        &self,
        config: crate::verify::VerifyConfig,
    ) -> Result<crate::verify::VerifyReport> {
        crate::verify::verify(&self.refs, &self.object_store, config)
    }
}

/// RAII guard for repository lock.
///
/// Holds an exclusive lock on the repository's LOCK file. The lock is
/// automatically released when dropped, and the lock file is removed.
struct LockGuard {
    /// The open lock file (holds the file lock).
    /// Wrapped in Option to allow taking ownership in Drop.
    file: Option<File>,
    /// Path to the lock file (for cleanup on drop).
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Take ownership of the file to close it (releases the lock)
        if let Some(file) = self.file.take() {
            drop(file);
        }

        // Remove the lock file - ignore errors (file might already be gone)
        let _ = fs::remove_file(&self.path);
    }
}

/// Check if a process with the given PID is still alive.
///
/// On Linux, uses /proc/{pid}/stat to check process existence.
/// On other Unix systems, uses /proc/{pid} directory existence.
/// On non-Unix systems, conservatively assumes the process is alive.
#[cfg(target_os = "linux")]
fn is_process_alive(pid: u32) -> bool {
    // On Linux, check if /proc/{pid}/stat exists
    // This is more reliable than just /proc/{pid} because zombie processes
    // still have a /proc entry but their stat file shows they're defunct
    std::path::Path::new(&format!("/proc/{}/stat", pid)).exists()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn is_process_alive(pid: u32) -> bool {
    // On other Unix systems (macOS, BSD), /proc may not exist
    // Use a command-based approach
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(true) // Conservative: assume alive if we can't check
}

#[cfg(not(unix))]
fn is_process_alive(_pid: u32) -> bool {
    // On non-Unix systems (Windows), conservatively assume process is alive
    // This means stale locks won't be auto-cleaned on Windows
    // Users can manually delete the LOCK file if needed
    true
}

/// Report from analyzing all Rust files in a project.
#[derive(Debug, Clone)]
pub struct AnalysisReport {
    /// Number of files successfully analyzed.
    pub files_analyzed: usize,
    /// Total symbols found (functions, structs, etc.).
    pub symbols_found: usize,
    /// Total function calls resolved.
    pub calls_resolved: usize,
    /// Total edges generated.
    pub edges_generated: usize,
    /// ObjectId of the stored EdgeBatch.
    pub edge_batch_id: ObjectId,
    /// ObjectId of the created commit.
    pub commit_id: ObjectId,
}

/// Report from analyzing a single Rust file.
#[derive(Debug, Clone)]
pub struct FileAnalysisReport {
    /// Path to the analyzed file.
    pub path: PathBuf,
    /// Number of symbols found.
    pub symbols: usize,
    /// Number of calls resolved.
    pub calls: usize,
    /// Number of edges generated.
    pub edges: usize,
    /// ObjectId of the stored EdgeBatch.
    pub edge_batch_id: ObjectId,
    /// ObjectId of the created commit.
    pub commit_id: ObjectId,
}

/// Converts days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Civil calendar algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_creates_structure() {
        let tmp = TempDir::new().unwrap();
        CtxRepo::init(tmp.path()).unwrap();

        // Check all directories exist
        assert!(tmp.path().join(".ctx/objects").exists());
        assert!(tmp.path().join(".ctx/refs").exists());
        assert!(tmp.path().join(".ctx/narrative/log").exists());
        assert!(tmp.path().join(".ctx/narrative/tasks").exists());
        assert!(tmp.path().join(".ctx/index").exists());

        // Check files exist
        assert!(tmp.path().join(".ctx/config.toml").exists());
        assert!(tmp.path().join(".ctx/.gitignore").exists());
        assert!(tmp.path().join(".ctx/narrative/README.md").exists());
        assert!(tmp.path().join(".ctx/HEAD").exists());
    }

    #[test]
    fn test_init_creates_initial_commit() {
        let tmp = TempDir::new().unwrap();
        let repo = CtxRepo::init(tmp.path()).unwrap();

        let commit = repo.head().unwrap();

        assert_eq!(commit.parents.len(), 0);
        assert_eq!(commit.message, "Initial commit");
        assert!(commit.edge_batches.is_empty());
        assert!(commit.narrative_refs.is_empty());
    }

    #[test]
    fn test_init_head_matches_main() {
        let tmp = TempDir::new().unwrap();
        let repo = CtxRepo::init(tmp.path()).unwrap();

        let head_id = repo.refs().read_head().unwrap();
        let main_id = repo.refs().read_ref("main").unwrap();

        assert_eq!(head_id, main_id);
    }

    #[test]
    fn test_init_fails_if_exists() {
        let tmp = TempDir::new().unwrap();
        CtxRepo::init(tmp.path()).unwrap();

        let result = CtxRepo::init(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_open_existing() {
        let tmp = TempDir::new().unwrap();
        CtxRepo::init(tmp.path()).unwrap();

        // Should be able to reopen
        let repo = CtxRepo::open(tmp.path()).unwrap();
        assert!(repo.head().is_ok());
    }

    #[test]
    fn test_open_nonexistent_fails() {
        let tmp = TempDir::new().unwrap();
        let result = CtxRepo::open(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_initial_tree_is_empty() {
        let tmp = TempDir::new().unwrap();
        let repo = CtxRepo::init(tmp.path()).unwrap();

        let commit = repo.head().unwrap();
        let tree: Tree = repo.object_store().get_typed(commit.root_tree).unwrap();

        assert!(tree.entries.is_empty());
    }

    #[test]
    fn test_root_and_ctx_dir() {
        let tmp = TempDir::new().unwrap();
        let repo = CtxRepo::init(tmp.path()).unwrap();

        assert_eq!(repo.root(), tmp.path());
        assert_eq!(repo.ctx_dir(), tmp.path().join(".ctx"));
    }

    #[test]
    fn test_session_start_and_compact() {
        let tmp = TempDir::new().unwrap();
        let mut repo = CtxRepo::init(tmp.path()).unwrap();

        // Start session
        let session = repo.start_session("Test task").unwrap();
        assert_eq!(session.task_description(), "Test task");
        assert_eq!(session.step_count(), 1); // Initial flush

        // Verify STAGE exists
        assert!(repo.refs().read_stage().unwrap().is_some());

        // Compact session
        let commit_id = repo.compact_session("Completed test").unwrap();

        // Verify commit exists
        let commit: crate::types::Commit = repo.object_store().get_typed(commit_id).unwrap();
        assert_eq!(commit.message, "Completed test");

        // Verify STAGE is gone
        assert!(repo.refs().read_stage().unwrap().is_none());

        // Verify no active session
        assert!(!repo.has_active_session());
    }

    #[test]
    fn test_session_crash_recovery() {
        use crate::types::SessionState;

        let tmp = TempDir::new().unwrap();

        // Start session and do some work
        {
            let mut repo = CtxRepo::init(tmp.path()).unwrap();
            repo.start_session("Recovery test").unwrap();

            // Add some observations using the convenience methods
            repo.observe_note("Before crash").unwrap();
            repo.observe_file_write("test.rs", b"fn main() {}").unwrap();

            // Flush the step
            repo.flush_active_session().unwrap();

            // Verify session has 2 steps (initial + this one)
            assert_eq!(repo.active_session().unwrap().step_count(), 2);

            // Simulate crash - drop repo without compacting
        }

        // Reopen and recover
        {
            let mut repo = CtxRepo::open(tmp.path()).unwrap();

            // Should have no active session initially
            assert!(!repo.has_active_session());

            // But STAGE should exist
            assert!(repo.refs().read_stage().unwrap().is_some());

            // Recover the session
            repo.recover_session().unwrap();
            assert!(repo.has_active_session());

            // Verify recovered state
            {
                let session = repo.active_session().unwrap();
                assert_eq!(session.task_description(), "Recovery test");
                assert_eq!(session.step_count(), 2);
                assert!(matches!(session.state(), SessionState::Running));
            }

            // Can continue working
            {
                let session = repo.active_session_mut().unwrap();
                session.observe_note("After recovery").unwrap();
            }
            repo.flush_active_session().unwrap();
            assert_eq!(repo.active_session().unwrap().step_count(), 3);

            // Compact successfully
            repo.compact_session("Recovered and completed").unwrap();
        }

        // Verify commit was created
        {
            let repo = CtxRepo::open(tmp.path()).unwrap();
            let commit = repo.head().unwrap();
            assert_eq!(commit.message, "Recovered and completed");
        }
    }

    #[test]
    fn test_concurrent_session_prevention() {
        let tmp = TempDir::new().unwrap();
        let mut repo = CtxRepo::init(tmp.path()).unwrap();

        // Start first session
        repo.start_session("First task").unwrap();

        // Try to start second session - should fail
        let result = repo.start_session("Second task");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CtxError::SessionAlreadyActive(_)
        ));

        // Verify first session is still active
        assert!(repo.has_active_session());
        assert_eq!(
            repo.active_session().unwrap().task_description(),
            "First task"
        );
    }

    #[test]
    fn test_file_locking_prevents_concurrent_access() {
        let tmp = TempDir::new().unwrap();
        let mut repo1 = CtxRepo::init(tmp.path()).unwrap();

        // Start session in first repo (acquires lock)
        repo1.start_session("Task in repo1").unwrap();

        // Try to open second repo instance and start session
        let mut repo2 = CtxRepo::open(tmp.path()).unwrap();

        // This should fail due to lock
        // With PID-based locking, we get SessionLockHeld when we can detect
        // the holding process, or RepositoryLocked if we can't
        let result = repo2.start_session("Task in repo2");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                CtxError::RepositoryLocked | CtxError::SessionLockHeld { .. }
            ),
            "Expected RepositoryLocked or SessionLockHeld, got: {:?}",
            err
        );
    }

    #[test]
    fn test_tree_preserved_in_compaction() {
        let tmp = TempDir::new().unwrap();
        let mut repo = CtxRepo::init(tmp.path()).unwrap();

        // Start session
        repo.start_session("Build tree test").unwrap();

        // Write multiple files with nested directory structure
        let file1_id = repo
            .observe_file_write("src/main.rs", b"fn main() { println!(\"Hello\"); }")
            .unwrap();

        let file2_id = repo
            .observe_file_write("src/lib.rs", b"pub fn hello() {}")
            .unwrap();

        let file3_id = repo
            .observe_file_write("tests/test.rs", b"#[test] fn test_hello() {}")
            .unwrap();

        let file4_id = repo
            .observe_file_write("README.md", b"# My Project")
            .unwrap();

        // Flush the observations
        repo.flush_active_session().unwrap();

        // Compact into canonical commit
        let commit_id = repo.compact_session("Added project files").unwrap();

        // Verify tree was built correctly
        let commit: crate::types::Commit = repo.object_store().get_typed(commit_id).unwrap();
        let root_tree: crate::types::Tree =
            repo.object_store().get_typed(commit.root_tree).unwrap();

        // Root should have: README.md, src/, tests/ (sorted alphabetically)
        assert_eq!(root_tree.entries.len(), 3);
        assert_eq!(root_tree.entries[0].name, "README.md");
        assert_eq!(root_tree.entries[0].kind, crate::types::TreeEntryKind::Blob);
        assert_eq!(root_tree.entries[0].id, file4_id);

        assert_eq!(root_tree.entries[1].name, "src");
        assert_eq!(root_tree.entries[1].kind, crate::types::TreeEntryKind::Tree);

        assert_eq!(root_tree.entries[2].name, "tests");
        assert_eq!(root_tree.entries[2].kind, crate::types::TreeEntryKind::Tree);

        // Verify src/ subtree
        let src_tree: crate::types::Tree = repo
            .object_store()
            .get_typed(root_tree.entries[1].id)
            .unwrap();
        assert_eq!(src_tree.entries.len(), 2);

        // Should be sorted: lib.rs, main.rs
        assert_eq!(src_tree.entries[0].name, "lib.rs");
        assert_eq!(src_tree.entries[0].kind, crate::types::TreeEntryKind::Blob);
        assert_eq!(src_tree.entries[0].id, file2_id);

        assert_eq!(src_tree.entries[1].name, "main.rs");
        assert_eq!(src_tree.entries[1].kind, crate::types::TreeEntryKind::Blob);
        assert_eq!(src_tree.entries[1].id, file1_id);

        // Verify tests/ subtree
        let tests_tree: crate::types::Tree = repo
            .object_store()
            .get_typed(root_tree.entries[2].id)
            .unwrap();
        assert_eq!(tests_tree.entries.len(), 1);

        assert_eq!(tests_tree.entries[0].name, "test.rs");
        assert_eq!(
            tests_tree.entries[0].kind,
            crate::types::TreeEntryKind::Blob
        );
        assert_eq!(tests_tree.entries[0].id, file3_id);

        // Verify content is preserved
        let main_content = repo.object_store().get_blob(file1_id).unwrap();
        assert_eq!(main_content, b"fn main() { println!(\"Hello\"); }");

        let lib_content = repo.object_store().get_blob(file2_id).unwrap();
        assert_eq!(lib_content, b"pub fn hello() {}");
    }

    #[test]
    fn test_stale_session_detection() {
        use std::thread;
        use std::time::Duration;

        let tmp = TempDir::new().unwrap();
        let mut repo = CtxRepo::init(tmp.path()).unwrap();

        // Start session
        repo.start_session("Stale test").unwrap();

        // Check immediately - should be fresh
        let config = crate::config::StaleSessionConfig {
            ask_threshold_secs: 2,
            auto_compact_threshold_secs: 5,
        };

        let status = repo.check_stale_session(&config);
        assert!(matches!(
            status,
            crate::config::StaleSessionStatus::Fresh { .. }
        ));

        // Wait for it to become stale (ask threshold)
        thread::sleep(Duration::from_secs(3));

        let status = repo.check_stale_session(&config);
        assert!(matches!(
            status,
            crate::config::StaleSessionStatus::ShouldAsk { .. }
        ));
    }

    #[test]
    fn test_abort_session() {
        let tmp = TempDir::new().unwrap();
        let mut repo = CtxRepo::init(tmp.path()).unwrap();

        // Start session
        repo.start_session("Aborted task").unwrap();

        // Abort it
        let commit_id = repo.abort_session("User cancelled").unwrap();

        // Verify abort commit
        let commit: crate::types::Commit = repo.object_store().get_typed(commit_id).unwrap();
        assert!(commit.message.contains("Aborted"));
        assert!(commit.message.contains("User cancelled"));
        assert_eq!(
            commit.commit_type,
            Some(crate::types::CommitType::Abandoned)
        );

        // Verify session is gone
        assert!(!repo.has_active_session());
        assert!(repo.refs().read_stage().unwrap().is_none());
    }

    #[test]
    fn test_edge_batches_created_on_compact() {
        let tmp = TempDir::new().unwrap();
        let mut repo = CtxRepo::init(tmp.path()).unwrap();

        // Start session
        repo.start_session("Test edge extraction").unwrap();

        // Observe file writes
        repo.observe_file_write("src/main.rs", b"fn main() {}")
            .unwrap();
        repo.observe_file_write("src/lib.rs", b"pub fn test() {}")
            .unwrap();

        // Flush observations
        repo.flush_active_session().unwrap();

        // Compact
        let commit_id = repo.compact_session("Added source files").unwrap();

        // Load commit
        let commit: crate::types::Commit = repo.object_store().get_typed(commit_id).unwrap();

        // Should have edge batches
        assert_eq!(commit.edge_batches.len(), 1, "Should have 1 EdgeBatch");

        // Load the edge batch
        let edge_batch_id = commit.edge_batches[0];
        let edge_batch: crate::types::EdgeBatch =
            repo.object_store().get_typed(edge_batch_id).unwrap();

        // Should have 2 edges (one per file)
        assert_eq!(edge_batch.edges.len(), 2, "Should have 2 edges");

        // Verify edges point to the files
        assert_eq!(edge_batch.edges[0].from.kind, crate::types::NodeKind::File);
        assert_eq!(edge_batch.edges[0].from.id, "src/lib.rs");
        assert_eq!(
            edge_batch.edges[0].label,
            crate::types::EdgeLabel::UpdatedIn
        );

        assert_eq!(edge_batch.edges[1].from.kind, crate::types::NodeKind::File);
        assert_eq!(edge_batch.edges[1].from.id, "src/main.rs");
        assert_eq!(
            edge_batch.edges[1].label,
            crate::types::EdgeLabel::UpdatedIn
        );

        // Verify evidence points to base commit (where session started)
        // Note: Evidence records where the observations were made (during session),
        // not the final compacted commit ID
        let base_commit = commit.parents[0];
        assert_eq!(edge_batch.edges[0].evidence.commit_id, base_commit);
        assert_eq!(
            edge_batch.edges[0].evidence.tool,
            crate::types::EvidenceTool::Human
        );
        assert_eq!(
            edge_batch.edges[0].evidence.confidence,
            crate::types::Confidence::High
        );

        // Note: To find which commit introduced this EdgeBatch, we would query
        // which commit's edge_batches field contains edge_batch_id.
        // This avoids self-reference issues in content-addressed storage.
    }

    #[test]
    fn test_observe_file_read_with_content() {
        let tmp = TempDir::new().unwrap();
        let mut repo = CtxRepo::init(tmp.path()).unwrap();

        // Start session
        repo.start_session("Test file read observation").unwrap();

        // Observe file read with content
        let file_content = b"fn main() { println!(\"Hello\"); }";
        repo.observe_file_read_with_content("src/main.rs", file_content)
            .unwrap();

        // Flush to create WorkCommit
        repo.flush_active_session().unwrap();

        // Compact to verify content is preserved
        let commit_id = repo.compact_session("Read file").unwrap();

        // Verify commit was created
        let commit: crate::types::Commit = repo.object_store().get_typed(commit_id).unwrap();
        assert!(commit.message.contains("Read file"));

        // The content should be stored in the object store
        // We can verify this by checking the staging observations
        // (This is implicitly tested by the compaction succeeding)

        // More importantly: in a real scenario, we'd walk the staging chain
        // and find the FileRead observation with content_id
        // For now, just verify the API works without panicking
        assert!(repo.object_store().exists(commit.root_tree));
    }
}
