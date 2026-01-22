//! Reference (pointer) management for HEAD, STAGE, and named refs.

use crate::error::{CtxError, Result};
use crate::ObjectId;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Manages references to commits.
///
/// References are stored as single-line text files containing hex-encoded ObjectIds.
/// All write operations are atomic using temp file + rename.
pub struct Refs {
    root: PathBuf,
}

impl Refs {
    /// Creates a new Refs manager for the given .ctx directory.
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Reads the HEAD reference.
    ///
    /// # Errors
    ///
    /// Returns `RefNotFound` if HEAD doesn't exist.
    /// Returns `InvalidRef` if the content is malformed.
    pub fn read_head(&self) -> Result<ObjectId> {
        let path = self.root.join("HEAD");
        self.read_ref_file(&path)
    }

    /// Writes the HEAD reference atomically.
    pub fn write_head(&self, id: ObjectId) -> Result<()> {
        let path = self.root.join("HEAD");
        self.write_ref_file(&path, id)
    }

    /// Reads a named reference (e.g., "main", "heads/feature").
    ///
    /// # Errors
    ///
    /// Returns `RefNotFound` if the ref doesn't exist.
    /// Returns `InvalidRef` if the content is malformed.
    pub fn read_ref(&self, name: &str) -> Result<ObjectId> {
        let path = self.root.join("refs").join(name);
        self.read_ref_file(&path)
    }

    /// Writes a named reference atomically.
    ///
    /// Creates parent directories as needed.
    pub fn write_ref(&self, name: &str, id: ObjectId) -> Result<()> {
        let path = self.root.join("refs").join(name);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        self.write_ref_file(&path, id)
    }

    /// Deletes a named reference.
    ///
    /// # Errors
    ///
    /// Returns `RefNotFound` if the ref doesn't exist.
    pub fn delete_ref(&self, name: &str) -> Result<()> {
        let path = self.root.join("refs").join(name);

        if !path.exists() {
            return Err(CtxError::RefNotFound(name.to_string()));
        }

        fs::remove_file(&path)?;
        Ok(())
    }

    /// Lists all named references.
    ///
    /// Returns a sorted list of (name, ObjectId) pairs.
    pub fn list_refs(&self) -> Result<Vec<(String, ObjectId)>> {
        let refs_dir = self.root.join("refs");

        if !refs_dir.exists() {
            return Ok(vec![]);
        }

        let mut refs = Vec::new();
        self.collect_refs(&refs_dir, &refs_dir, &mut refs)?;

        // Sort by name for deterministic output
        refs.sort_by(|a, b| a.0.cmp(&b.0));

        Ok(refs)
    }

    /// Reads the STAGE reference (optional staging area).
    ///
    /// Returns `None` if STAGE doesn't exist.
    pub fn read_stage(&self) -> Result<Option<ObjectId>> {
        let path = self.root.join("STAGE");

        if !path.exists() {
            return Ok(None);
        }

        self.read_ref_file(&path).map(Some)
    }

    /// Writes the STAGE reference atomically.
    pub fn write_stage(&self, id: ObjectId) -> Result<()> {
        let path = self.root.join("STAGE");
        self.write_ref_file(&path, id)
    }

    /// Deletes the STAGE reference.
    ///
    /// Does nothing if STAGE doesn't exist.
    pub fn delete_stage(&self) -> Result<()> {
        let path = self.root.join("STAGE");

        if path.exists() {
            fs::remove_file(&path)?;
        }

        Ok(())
    }

    /// Reads an ObjectId from a ref file.
    fn read_ref_file(&self, path: &Path) -> Result<ObjectId> {
        if !path.exists() {
            return Err(CtxError::RefNotFound(
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string(),
            ));
        }

        let content = fs::read_to_string(path)?;
        let trimmed = content.trim();

        if trimmed.len() != 64 {
            return Err(CtxError::InvalidRef {
                path: path.to_path_buf(),
                reason: format!("expected 64 hex chars, got {}", trimmed.len()),
            });
        }

        ObjectId::from_hex(trimmed).map_err(|_| CtxError::InvalidRef {
            path: path.to_path_buf(),
            reason: "invalid hex string".to_string(),
        })
    }

    /// Writes an ObjectId to a ref file atomically.
    ///
    /// Uses temp file + fsync + rename for crash safety.
    fn write_ref_file(&self, path: &Path, id: ObjectId) -> Result<()> {
        let tmp_path = path.with_extension("tmp");

        // Write to temp file
        {
            let mut file = File::create(&tmp_path)?;
            writeln!(file, "{}", id.as_hex())?;
            file.sync_all()?;
        }

        // Atomic rename
        fs::rename(&tmp_path, path)?;

        // fsync parent directory (Unix-specific for crash safety)
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

    /// Recursively collects all refs under a directory.
    fn collect_refs(
        &self,
        current: &Path,
        base: &Path,
        refs: &mut Vec<(String, ObjectId)>,
    ) -> Result<()> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                self.collect_refs(&path, base, refs)?;
            } else if path.is_file() {
                // Skip .tmp files
                if path.extension().and_then(|s| s.to_str()) == Some("tmp") {
                    continue;
                }

                // Read the ref
                if let Ok(id) = self.read_ref_file(&path) {
                    // Compute relative name
                    if let Ok(rel_path) = path.strip_prefix(base) {
                        if let Some(name) = rel_path.to_str() {
                            refs.push((name.to_string(), id));
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_head_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let refs = Refs::new(tmp.path());

        let id = ObjectId::from_bytes([42; 32]);
        refs.write_head(id).unwrap();
        let retrieved = refs.read_head().unwrap();

        assert_eq!(id, retrieved);
    }

    #[test]
    fn test_head_not_found() {
        let tmp = TempDir::new().unwrap();
        let refs = Refs::new(tmp.path());

        let result = refs.read_head();
        assert!(matches!(result, Err(CtxError::RefNotFound(_))));
    }

    #[test]
    fn test_named_ref_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let refs = Refs::new(tmp.path());

        let id = ObjectId::from_bytes([123; 32]);
        refs.write_ref("main", id).unwrap();
        let retrieved = refs.read_ref("main").unwrap();

        assert_eq!(id, retrieved);
    }

    #[test]
    fn test_nested_ref() {
        let tmp = TempDir::new().unwrap();
        let refs = Refs::new(tmp.path());

        let id = ObjectId::from_bytes([99; 32]);
        refs.write_ref("heads/feature/test", id).unwrap();
        let retrieved = refs.read_ref("heads/feature/test").unwrap();

        assert_eq!(id, retrieved);
    }

    #[test]
    fn test_delete_ref() {
        let tmp = TempDir::new().unwrap();
        let refs = Refs::new(tmp.path());

        let id = ObjectId::from_bytes([1; 32]);
        refs.write_ref("temp", id).unwrap();

        assert!(refs.read_ref("temp").is_ok());

        refs.delete_ref("temp").unwrap();

        assert!(matches!(
            refs.read_ref("temp"),
            Err(CtxError::RefNotFound(_))
        ));
    }

    #[test]
    fn test_list_refs() {
        let tmp = TempDir::new().unwrap();
        let refs = Refs::new(tmp.path());

        refs.write_ref("main", ObjectId::from_bytes([1; 32]))
            .unwrap();
        refs.write_ref("develop", ObjectId::from_bytes([2; 32]))
            .unwrap();
        refs.write_ref("heads/feature", ObjectId::from_bytes([3; 32]))
            .unwrap();

        let list = refs.list_refs().unwrap();

        assert_eq!(list.len(), 3);
        // Should be sorted
        assert_eq!(list[0].0, "develop");
        assert_eq!(list[1].0, "heads/feature");
        assert_eq!(list[2].0, "main");
    }

    #[test]
    fn test_stage_operations() {
        let tmp = TempDir::new().unwrap();
        let refs = Refs::new(tmp.path());

        // Initially no stage
        assert_eq!(refs.read_stage().unwrap(), None);

        // Write stage
        let id = ObjectId::from_bytes([55; 32]);
        refs.write_stage(id).unwrap();
        assert_eq!(refs.read_stage().unwrap(), Some(id));

        // Delete stage
        refs.delete_stage().unwrap();
        assert_eq!(refs.read_stage().unwrap(), None);

        // Delete again (should be no-op)
        refs.delete_stage().unwrap();
    }

    #[test]
    fn test_atomic_write() {
        let tmp = TempDir::new().unwrap();
        let refs = Refs::new(tmp.path());

        let id = ObjectId::from_bytes([77; 32]);
        refs.write_ref("test", id).unwrap();

        // Check no .tmp files left behind
        let refs_dir = tmp.path().join("refs");
        for entry in fs::read_dir(&refs_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            assert_ne!(
                path.extension().and_then(|s| s.to_str()),
                Some("tmp"),
                "Found leftover .tmp file: {:?}",
                path
            );
        }
    }

    #[test]
    fn test_invalid_ref_content() {
        let tmp = TempDir::new().unwrap();
        let refs = Refs::new(tmp.path());

        // Write invalid content directly
        let path = tmp.path().join("refs").join("bad");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not a valid object id").unwrap();

        let result = refs.read_ref("bad");
        assert!(matches!(result, Err(CtxError::InvalidRef { .. })));
    }
}
