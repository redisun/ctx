use anyhow::{Context, Result};
use ctx_core::CtxRepo;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Manages isolated test environments with tempfile
pub struct TestWorkspace {
    dir: TempDir,
}

impl TestWorkspace {
    /// Create an empty workspace
    pub fn empty() -> Result<Self> {
        let dir = TempDir::new().context("Failed to create temp directory")?;
        Ok(Self { dir })
    }

    /// Create workspace with initial files
    pub fn with_files(files: HashMap<String, Vec<u8>>) -> Result<Self> {
        let workspace = Self::empty()?;
        for (path, content) in files {
            workspace.write_file(&path, &content)?;
        }
        Ok(workspace)
    }

    /// Load workspace from fixtures directory
    pub fn from_fixture(name: &str) -> Result<Self> {
        let workspace = Self::empty()?;
        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name);

        if !fixture_path.exists() {
            anyhow::bail!("Fixture not found: {}", fixture_path.display());
        }

        // Recursively copy fixture files
        copy_dir_recursive(&fixture_path, workspace.path())?;

        Ok(workspace)
    }

    /// Get workspace path
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Initialize CTX repository in workspace
    pub fn init_ctx(&self) -> Result<CtxRepo> {
        Ok(CtxRepo::init(self.path())?)
    }

    /// Open existing CTX repository
    pub fn open_ctx(&self) -> Result<CtxRepo> {
        Ok(CtxRepo::open(self.path())?)
    }

    /// Write file to workspace
    pub fn write_file(&self, path: &str, content: &[u8]) -> Result<()> {
        let full_path = self.path().join(path);

        // Create parent directories
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directories for {}", path))?;
        }

        fs::write(&full_path, content)
            .with_context(|| format!("Failed to write file: {}", path))?;

        Ok(())
    }

    /// Read file from workspace
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        let full_path = self.path().join(path);
        fs::read(&full_path).with_context(|| format!("Failed to read file: {}", path))
    }

    /// Check if file exists
    pub fn file_exists(&self, path: &str) -> bool {
        self.path().join(path).exists()
    }
}

/// Recursively copy directory contents
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            fs::create_dir_all(&dst_path)?;
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
