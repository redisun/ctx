//! Narrative system for human-readable documentation.
//!
//! Manages the `.ctx/narrative/` directory containing Markdown documents
//! for logs, tasks, decisions, and other human-readable content.

use crate::error::{CtxError, Result};
use crate::types::NarrativeRef;
use crate::{ObjectId, ObjectStore};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Manages the narrative documentation space.
///
/// The narrative space lives in `.ctx/narrative/` and contains:
/// - `log/` - Daily journal entries (YYYY-MM-DD.md)
/// - `tasks/` - Task documentation (task_NNNN.md)
/// - `README.md` - Repository overview
/// - `decisions.md` - Architectural decisions
pub struct NarrativeSpace {
    /// Root path to .ctx/narrative/
    root: PathBuf,
}

/// Information about a task file.
#[derive(Debug, Clone)]
pub struct TaskInfo {
    /// Task ID (numeric part, e.g., 42 for task_0042.md)
    pub id: u32,
    /// Full path to the task file
    pub path: PathBuf,
    /// Relative path from narrative root (e.g., "tasks/task_0042.md")
    pub relative_path: String,
}

impl NarrativeSpace {
    /// Creates a new NarrativeSpace for the given .ctx directory.
    ///
    /// Note: This does not create the directory structure.
    /// Use `ensure_structure()` to create directories if needed.
    pub fn new(ctx_dir: impl AsRef<Path>) -> Self {
        Self {
            root: ctx_dir.as_ref().join("narrative"),
        }
    }

    /// Reads narrative content from a blob ID.
    ///
    /// This is useful for retrieving historical narrative content from commits.
    ///
    /// # Arguments
    ///
    /// * `store` - ObjectStore to read from
    /// * `blob_id` - The blob ID from a NarrativeRef
    ///
    /// # Returns
    ///
    /// The narrative content as a UTF-8 string.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The blob doesn't exist
    /// - The blob content is not valid UTF-8
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ctx_core::{CtxRepo, NarrativeSpace};
    ///
    /// let repo = CtxRepo::open(".").unwrap();
    /// let commit = repo.head().unwrap();
    ///
    /// if let Some(nr) = commit.narrative_refs.first() {
    ///     let content = NarrativeSpace::read_from_blob(
    ///         repo.object_store(),
    ///         nr.blob_id
    ///     ).unwrap();
    ///     println!("Historical content: {}", content);
    /// }
    /// ```
    pub fn read_from_blob(store: &ObjectStore, blob_id: ObjectId) -> Result<String> {
        let bytes = store.get_blob(blob_id)?;
        String::from_utf8(bytes).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Narrative blob contains invalid UTF-8: {}", e),
            ))
        })
    }

    /// Returns the root path of the narrative space.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Ensures the narrative directory structure exists.
    ///
    /// Creates `log/` and `tasks/` subdirectories if missing.
    pub fn ensure_structure(&self) -> Result<()> {
        fs::create_dir_all(self.root.join("log"))?;
        fs::create_dir_all(self.root.join("tasks"))?;
        Ok(())
    }

    /// Appends an entry to the daily log file.
    ///
    /// Creates the log file if it doesn't exist.
    /// Entries are appended with a timestamp header.
    ///
    /// # Arguments
    ///
    /// * `date` - Date string in YYYY-MM-DD format
    /// * `time` - Time string in HH:MM format (for the entry header)
    /// * `entry` - The log entry content (can be multi-line)
    ///
    /// # Returns
    ///
    /// The relative path to the log file (e.g., "log/2026-01-22.md")
    pub fn append_log(&self, date: &str, time: &str, entry: &str) -> Result<String> {
        let filename = format!("{}.md", date);
        let path = self.root.join("log").join(&filename);
        let relative_path = format!("log/{}", filename);

        // Create or append to file
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        // If file is empty/new, add date header
        let metadata = file.metadata()?;
        if metadata.len() == 0 {
            writeln!(file, "# {}\n", date)?;
        }

        // Write timestamped entry
        writeln!(file, "### {}\n", time)?;
        writeln!(file, "{}\n", entry)?;
        file.sync_all()?;

        Ok(relative_path)
    }

    /// Creates a new task file.
    ///
    /// Generates a unique task ID by finding the maximum existing ID + 1.
    ///
    /// # Arguments
    ///
    /// * `title` - Task title (used as markdown heading)
    /// * `body` - Task description/body (optional, can be empty)
    ///
    /// # Returns
    ///
    /// Information about the created task including its ID and path.
    pub fn create_task(&self, title: &str, body: &str) -> Result<TaskInfo> {
        let id = self.next_task_id()?;
        let filename = format!("task_{:04}.md", id);
        let path = self.root.join("tasks").join(&filename);
        let relative_path = format!("tasks/{}", filename);

        // Create task content
        let content = if body.is_empty() {
            format!("# {}\n\n**Status:** open\n", title)
        } else {
            format!("# {}\n\n**Status:** open\n\n{}\n", title, body)
        };

        // Write atomically
        atomic_write(&path, content.as_bytes())?;

        Ok(TaskInfo {
            id,
            path,
            relative_path,
        })
    }

    /// Updates an existing task file.
    ///
    /// # Arguments
    ///
    /// * `id` - Task ID (numeric part)
    /// * `status` - New status (e.g., "open", "in_progress", "done")
    /// * `note` - Optional note to append (can be empty)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The task file doesn't exist
    /// - The task file is not readable/writable
    /// - The task file doesn't contain a status line
    pub fn update_task(&self, id: u32, status: &str, note: &str) -> Result<String> {
        let filename = format!("task_{:04}.md", id);
        let path = self.root.join("tasks").join(&filename);
        let relative_path = format!("tasks/{}", filename);

        if !path.exists() {
            // Check if tasks directory exists to give better error
            let tasks_dir = self.root.join("tasks");
            if !tasks_dir.exists() {
                return Err(CtxError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "Task #{:04} not found: tasks directory doesn't exist. Try running 'ctx add task' first.",
                        id
                    ),
                )));
            }
            return Err(CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "Task #{:04} not found at {}. Available tasks: {}",
                    id,
                    relative_path,
                    self.list_task_ids()
                        .unwrap_or_else(|_| vec![])
                        .iter()
                        .map(|i| format!("#{:04}", i))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )));
        }

        // Read current content
        let mut content = fs::read_to_string(&path).map_err(|e| {
            CtxError::Io(std::io::Error::new(
                e.kind(),
                format!("Failed to read task #{:04}: {}", id, e),
            ))
        })?;

        // Update status (replace **Status:** line)
        if let Some(start) = content.find("**Status:**") {
            if let Some(end) = content[start..].find('\n') {
                let before = &content[..start];
                let after = &content[start + end..];
                content = format!("{}**Status:** {}{}", before, status, after);
            }
        } else {
            return Err(CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Task #{:04} is malformed: missing '**Status:**' line in {}",
                    id, relative_path
                ),
            )));
        }

        // Append note if provided
        if !note.is_empty() {
            content.push_str("\n---\n\n");
            content.push_str(note);
            content.push('\n');
        }

        // Write atomically
        atomic_write(&path, content.as_bytes())?;

        Ok(relative_path)
    }

    /// Reads a narrative file's content.
    ///
    /// # Arguments
    ///
    /// * `relative_path` - Path relative to narrative root (e.g., "log/2026-01-22.md")
    pub fn read_file(&self, relative_path: &str) -> Result<Vec<u8>> {
        let path = self.root.join(relative_path);
        if !path.exists() {
            return Err(CtxError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Narrative file not found: {}", relative_path),
            )));
        }
        Ok(fs::read(&path)?)
    }

    /// Lists all narrative files.
    ///
    /// Returns relative paths for all .md files in the narrative space.
    /// Results are sorted for deterministic output.
    pub fn list_files(&self) -> Result<Vec<String>> {
        let mut files = Vec::new();
        self.collect_files(&self.root, &mut files)?;
        files.sort();
        Ok(files)
    }

    /// Computes NarrativeRefs for files changed since a commit.
    ///
    /// Compares current file hashes against the `narrative_refs` in the
    /// provided commit's refs. Returns refs for new or modified files.
    ///
    /// # Arguments
    ///
    /// * `store` - ObjectStore for storing blobs and comparing hashes
    /// * `previous_refs` - NarrativeRefs from the previous commit (empty for initial)
    /// * `role` - Role string for the NarrativeRef (e.g., "agent", "user")
    ///
    /// # Returns
    ///
    /// Vector of NarrativeRefs for all changed files, with blobs stored.
    pub fn snapshot_changed(
        &self,
        store: &ObjectStore,
        previous_refs: &[NarrativeRef],
        role: &str,
    ) -> Result<Vec<NarrativeRef>> {
        // Build lookup map from previous refs
        let previous_blobs: std::collections::HashMap<&str, ObjectId> = previous_refs
            .iter()
            .map(|r| (r.path.as_str(), r.blob_id))
            .collect();

        let mut changed_refs = Vec::new();

        // Walk all narrative files
        for relative_path in self.list_files()? {
            let content = self.read_file(&relative_path)?;

            // Compute what the blob ID would be
            let blob_id = store.put_blob(&content)?;

            // Check if changed (different ID or new file)
            let is_changed = match previous_blobs.get(relative_path.as_str()) {
                Some(prev_id) => *prev_id != blob_id,
                None => true, // New file
            };

            if is_changed {
                changed_refs.push(NarrativeRef {
                    path: relative_path,
                    stream: None, // Could be extracted from path in future
                    role: role.to_string(),
                    blob_id,
                });
            }
        }

        // Sort for determinism
        changed_refs.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(changed_refs)
    }

    /// Lists all task IDs found in the tasks directory.
    ///
    /// Returns a sorted vector of task IDs.
    fn list_task_ids(&self) -> Result<Vec<u32>> {
        let tasks_dir = self.root.join("tasks");

        if !tasks_dir.exists() {
            return Ok(vec![]);
        }

        let mut ids = Vec::new();

        for entry in fs::read_dir(&tasks_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Parse task_NNNN.md format
            if let Some(num_str) = name_str
                .strip_prefix("task_")
                .and_then(|s| s.strip_suffix(".md"))
            {
                if let Ok(num) = num_str.parse::<u32>() {
                    ids.push(num);
                }
            }
        }

        ids.sort_unstable();
        Ok(ids)
    }

    /// Finds the next available task ID.
    fn next_task_id(&self) -> Result<u32> {
        let ids = self.list_task_ids()?;
        Ok(ids.last().copied().unwrap_or(0) + 1)
    }

    /// Recursively collects all .md files.
    fn collect_files(&self, dir: &Path, files: &mut Vec<String>) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                self.collect_files(&path, files)?;
            } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
                // Compute relative path
                if let Ok(relative) = path.strip_prefix(&self.root) {
                    if let Some(rel_str) = relative.to_str() {
                        files.push(rel_str.to_string());
                    }
                }
            }
        }

        Ok(())
    }
}

/// Writes data atomically using temp file + rename.
fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("tmp");

    // Write to temp file
    {
        let mut file = File::create(&tmp_path)?;
        file.write_all(data)?;
        file.sync_all()?;
    }

    // Atomic rename
    fs::rename(&tmp_path, path)?;

    // fsync parent directory (Unix)
    #[cfg(unix)]
    {
        if let Some(parent) = path.parent() {
            if let Ok(dir_file) = File::open(parent) {
                let _ = dir_file.sync_all();
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_ensure_structure() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());

        ns.ensure_structure().unwrap();

        assert!(tmp.path().join("narrative/log").exists());
        assert!(tmp.path().join("narrative/tasks").exists());
    }

    #[test]
    fn test_append_log() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        let path = ns.append_log("2026-01-22", "10:30", "Test entry").unwrap();

        assert_eq!(path, "log/2026-01-22.md");

        let content = fs::read_to_string(tmp.path().join("narrative/log/2026-01-22.md")).unwrap();
        assert!(content.contains("# 2026-01-22"));
        assert!(content.contains("### 10:30"));
        assert!(content.contains("Test entry"));
    }

    #[test]
    fn test_append_log_multiple_entries() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        ns.append_log("2026-01-22", "10:00", "First entry").unwrap();
        ns.append_log("2026-01-22", "11:00", "Second entry")
            .unwrap();

        let content = fs::read_to_string(tmp.path().join("narrative/log/2026-01-22.md")).unwrap();

        // Should only have one date header
        assert_eq!(content.matches("# 2026-01-22").count(), 1);
        // Should have both entries
        assert!(content.contains("First entry"));
        assert!(content.contains("Second entry"));
    }

    #[test]
    fn test_create_task() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        let task = ns.create_task("Fix the bug", "Description here").unwrap();

        assert_eq!(task.id, 1);
        assert_eq!(task.relative_path, "tasks/task_0001.md");

        let content = fs::read_to_string(&task.path).unwrap();
        assert!(content.contains("# Fix the bug"));
        assert!(content.contains("**Status:** open"));
        assert!(content.contains("Description here"));
    }

    #[test]
    fn test_create_task_increments_id() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        let task1 = ns.create_task("Task 1", "").unwrap();
        let task2 = ns.create_task("Task 2", "").unwrap();
        let task3 = ns.create_task("Task 3", "").unwrap();

        assert_eq!(task1.id, 1);
        assert_eq!(task2.id, 2);
        assert_eq!(task3.id, 3);
    }

    #[test]
    fn test_update_task() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        let task = ns.create_task("Test task", "").unwrap();
        ns.update_task(task.id, "in_progress", "Started working")
            .unwrap();

        let content = fs::read_to_string(&task.path).unwrap();
        assert!(content.contains("**Status:** in_progress"));
        assert!(content.contains("Started working"));
    }

    #[test]
    fn test_list_files() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create some files
        ns.append_log("2026-01-20", "10:00", "Entry 1").unwrap();
        ns.append_log("2026-01-21", "10:00", "Entry 2").unwrap();
        ns.create_task("Task", "").unwrap();

        // Create README
        fs::write(tmp.path().join("narrative/README.md"), "# Test").unwrap();

        let files = ns.list_files().unwrap();

        assert!(files.contains(&"README.md".to_string()));
        assert!(files.contains(&"log/2026-01-20.md".to_string()));
        assert!(files.contains(&"log/2026-01-21.md".to_string()));
        assert!(files.contains(&"tasks/task_0001.md".to_string()));
    }

    #[test]
    fn test_snapshot_changed_new_files() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create a log entry
        ns.append_log("2026-01-22", "10:00", "Test").unwrap();

        // Snapshot with no previous refs
        let refs = ns.snapshot_changed(&store, &[], "agent").unwrap();

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "log/2026-01-22.md");
        assert_eq!(refs[0].role, "agent");
    }

    #[test]
    fn test_snapshot_changed_detects_modifications() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create initial file
        ns.append_log("2026-01-22", "10:00", "Initial").unwrap();

        // Get initial snapshot
        let refs1 = ns.snapshot_changed(&store, &[], "agent").unwrap();

        // Modify file
        ns.append_log("2026-01-22", "11:00", "Added").unwrap();

        // Snapshot should detect change
        let refs2 = ns.snapshot_changed(&store, &refs1, "agent").unwrap();

        assert_eq!(refs2.len(), 1);
        assert_ne!(refs2[0].blob_id, refs1[0].blob_id);
    }

    #[test]
    fn test_snapshot_changed_skips_unchanged() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create file
        ns.append_log("2026-01-22", "10:00", "Content").unwrap();

        // Get snapshot
        let refs1 = ns.snapshot_changed(&store, &[], "agent").unwrap();

        // No modifications - should return empty
        let refs2 = ns.snapshot_changed(&store, &refs1, "agent").unwrap();

        assert!(refs2.is_empty());
    }

    // Edge case tests

    #[test]
    fn test_read_from_blob() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create a log entry
        ns.append_log("2026-01-22", "10:00", "Test content")
            .unwrap();

        // Snapshot it
        let refs = ns.snapshot_changed(&store, &[], "agent").unwrap();
        let blob_id = refs[0].blob_id;

        // Read from blob
        let content = NarrativeSpace::read_from_blob(&store, blob_id).unwrap();

        assert!(content.contains("# 2026-01-22"));
        assert!(content.contains("Test content"));
    }

    #[test]
    fn test_read_from_blob_invalid_utf8() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));

        // Store invalid UTF-8
        let invalid_utf8 = vec![0xFF, 0xFE, 0xFD];
        let blob_id = store.put_blob(&invalid_utf8).unwrap();

        // Should error
        let result = NarrativeSpace::read_from_blob(&store, blob_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_narrative_directory() {
        let tmp = TempDir::new().unwrap();
        let store = ObjectStore::new(tmp.path().join("objects"));
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // List files should be empty
        let files = ns.list_files().unwrap();
        assert!(files.is_empty());

        // Snapshot should be empty
        let refs = ns.snapshot_changed(&store, &[], "agent").unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn test_task_with_special_characters() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create task with special characters
        let task = ns
            .create_task(
                "Fix bug: \"quotes\" & <brackets> [test]",
                "Body with émojis 🚀 and unicode: 中文",
            )
            .unwrap();

        let content = fs::read_to_string(&task.path).unwrap();
        assert!(content.contains("Fix bug: \"quotes\" & <brackets> [test]"));
        assert!(content.contains("Body with émojis 🚀 and unicode: 中文"));
    }

    #[test]
    fn test_very_large_log_entry() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create a large entry (10KB)
        let large_entry = "x".repeat(10_000);
        let path = ns.append_log("2026-01-22", "10:00", &large_entry).unwrap();

        let content = fs::read_to_string(tmp.path().join("narrative").join(path)).unwrap();
        assert!(content.contains(&large_entry));
    }

    #[test]
    fn test_tasks_with_gaps_in_ids() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create task 1
        let task1 = ns.create_task("Task 1", "").unwrap();
        assert_eq!(task1.id, 1);

        // Manually create task 5 (skip 2, 3, 4)
        fs::write(
            tmp.path().join("narrative/tasks/task_0005.md"),
            "# Task 5\n\n**Status:** open\n",
        )
        .unwrap();

        // Next task should be 6, not 2
        let task6 = ns.create_task("Task 6", "").unwrap();
        assert_eq!(task6.id, 6);
    }

    #[test]
    fn test_malformed_task_files_ignored() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create valid task
        ns.create_task("Valid", "").unwrap();

        // Create files with malformed names (should be ignored)
        let tasks_dir = tmp.path().join("narrative/tasks");
        fs::write(tasks_dir.join("task_abc.md"), "invalid").unwrap();
        fs::write(tasks_dir.join("task_.md"), "invalid").unwrap();
        fs::write(tasks_dir.join("other.md"), "invalid").unwrap();
        fs::write(tasks_dir.join("task_0002.txt"), "wrong extension").unwrap();

        // Next task should still be 2
        let task2 = ns.create_task("Task 2", "").unwrap();
        assert_eq!(task2.id, 2);
    }

    #[test]
    fn test_update_nonexistent_task_error_message() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create task 1
        ns.create_task("Task 1", "").unwrap();

        // Try to update task 99 (doesn't exist)
        let result = ns.update_task(99, "done", "");
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Task #0099 not found"));
        assert!(err_msg.contains("#0001")); // Should suggest available task
    }

    #[test]
    fn test_update_task_with_no_status_line() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Manually create malformed task (no Status line)
        let malformed_path = tmp.path().join("narrative/tasks/task_0001.md");
        fs::write(&malformed_path, "# Malformed Task\n\nNo status line!").unwrap();

        // Try to update it
        let result = ns.update_task(1, "done", "");
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("malformed"));
        assert!(err_msg.contains("missing '**Status:**'"));
    }

    #[test]
    fn test_empty_task_title_and_body() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create task with empty title
        let task = ns.create_task("", "").unwrap();
        let content = fs::read_to_string(&task.path).unwrap();

        // Should still have proper structure
        assert!(content.contains("# "));
        assert!(content.contains("**Status:** open"));
    }

    #[test]
    fn test_multiline_task_body() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        let multiline_body = "Line 1\n\nLine 2\n\n- Item 1\n- Item 2";
        let task = ns.create_task("Test", multiline_body).unwrap();

        let content = fs::read_to_string(&task.path).unwrap();
        assert!(content.contains("Line 1"));
        assert!(content.contains("Line 2"));
        assert!(content.contains("- Item 1"));
    }

    #[test]
    fn test_update_task_preserves_content() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create task with specific body
        let task = ns
            .create_task("Test Task", "Important content\n\nDon't lose this!")
            .unwrap();

        // Update status
        ns.update_task(task.id, "in_progress", "").unwrap();

        let content = fs::read_to_string(&task.path).unwrap();
        assert!(content.contains("Important content"));
        assert!(content.contains("Don't lose this!"));
        assert!(content.contains("**Status:** in_progress"));
    }

    #[test]
    fn test_task_update_with_multiline_note() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        let task = ns.create_task("Test", "").unwrap();

        let multiline_note = "Update 1\n\nUpdate 2\n- Point A\n- Point B";
        ns.update_task(task.id, "in_progress", multiline_note)
            .unwrap();

        let content = fs::read_to_string(&task.path).unwrap();
        assert!(content.contains("Update 1"));
        assert!(content.contains("- Point A"));
        assert!(content.contains("---")); // Separator
    }

    #[test]
    fn test_list_files_with_subdirectories() {
        let tmp = TempDir::new().unwrap();
        let ns = NarrativeSpace::new(tmp.path());
        ns.ensure_structure().unwrap();

        // Create log files
        ns.append_log("2026-01-20", "10:00", "Entry").unwrap();
        ns.append_log("2026-01-21", "10:00", "Entry").unwrap();

        // Create tasks
        ns.create_task("Task 1", "").unwrap();
        ns.create_task("Task 2", "").unwrap();

        let files = ns.list_files().unwrap();

        // Should find files in subdirectories
        assert!(files.iter().any(|f| f.contains("log/")));
        assert!(files.iter().any(|f| f.contains("tasks/")));
        assert!(files.len() >= 4);
    }
}
