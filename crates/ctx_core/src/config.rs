//! Configuration types for CTX session management.

use crate::error::{CtxError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::Duration;

/// Comprehensive configuration for CTX repository.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Storage-related configuration.
    #[serde(default)]
    pub storage: StorageConfig,

    /// Garbage collection configuration.
    #[serde(default)]
    pub gc: GcConfig,

    /// Full-text search configuration.
    #[serde(default)]
    pub search: SearchConfig,

    /// Session management configuration.
    #[serde(default)]
    pub session: SessionConfig,
}

impl Config {
    /// Load configuration from a file.
    pub fn load(ctx_root: &Path) -> Result<Self> {
        let path = ctx_root.join("config.toml");
        if path.exists() {
            let content = fs::read_to_string(&path)
                .map_err(|e| CtxError::ConfigError(format!("failed to read config: {}", e)))?;
            toml::from_str(&content)
                .map_err(|e| CtxError::ConfigError(format!("failed to parse config: {}", e)))
        } else {
            Ok(Config::default())
        }
    }

    /// Save configuration to a file.
    pub fn save(&self, ctx_root: &Path) -> Result<()> {
        let path = ctx_root.join("config.toml");
        let content = toml::to_string_pretty(self)
            .map_err(|e| CtxError::ConfigError(format!("failed to serialize config: {}", e)))?;
        fs::write(&path, content)
            .map_err(|e| CtxError::ConfigError(format!("failed to write config: {}", e)))?;
        Ok(())
    }
}

/// Storage-related configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Compression level for zstd (1-22, default: 3).
    /// Higher values mean better compression but slower performance.
    pub compression_level: i32,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            compression_level: 3,
        }
    }
}

/// Garbage collection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcConfig {
    /// Grace period in days before deleting unreferenced objects (default: 7).
    pub grace_period_days: u32,

    /// Automatically run GC after session compaction (default: false).
    pub auto_gc: bool,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            grace_period_days: 7,
            auto_gc: false,
        }
    }
}

/// Full-text search configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Enable full-text search indexing (default: true).
    pub enabled: bool,

    /// Maximum search results to return (default: 20).
    pub max_results: usize,

    /// Snippet length for search results in characters (default: 150).
    pub snippet_length: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_results: 20,
            snippet_length: 150,
        }
    }
}

/// Session management configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Threshold in hours before asking about stale sessions (default: 24).
    pub stale_session_threshold_hours: u64,

    /// Optional auto-flush interval in seconds.
    /// If set, observations are automatically flushed after this interval.
    pub auto_flush_interval_secs: Option<u64>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            stale_session_threshold_hours: 24,
            auto_flush_interval_secs: None,
        }
    }
}

/// Configuration for stale session handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleSessionConfig {
    /// After this duration, ask user before starting new task.
    /// Default: 24 hours.
    pub ask_threshold_secs: u64,

    /// After this duration, auto-compact without asking.
    /// Default: 7 days.
    pub auto_compact_threshold_secs: u64,
}

impl Default for StaleSessionConfig {
    fn default() -> Self {
        Self {
            ask_threshold_secs: 24 * 60 * 60,              // 24 hours
            auto_compact_threshold_secs: 7 * 24 * 60 * 60, // 7 days
        }
    }
}

impl StaleSessionConfig {
    /// Returns the ask threshold as a Duration.
    pub fn ask_threshold(&self) -> Duration {
        Duration::from_secs(self.ask_threshold_secs)
    }

    /// Returns the auto-compact threshold as a Duration.
    pub fn auto_compact_threshold(&self) -> Duration {
        Duration::from_secs(self.auto_compact_threshold_secs)
    }
}

/// Status of stale session check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleSessionStatus {
    /// No active session exists.
    NoSession,

    /// Session is fresh, no action needed.
    Fresh {
        /// Task description.
        task: String,
        /// Idle duration in seconds.
        idle_secs: u64,
    },

    /// Session is moderately stale, should ask user.
    ShouldAsk {
        /// Task description.
        task: String,
        /// Idle duration in seconds.
        idle_secs: u64,
    },

    /// Session is very stale, should auto-compact.
    ShouldAutoCompact {
        /// Task description.
        task: String,
        /// Idle duration in seconds.
        idle_secs: u64,
    },
}

/// Report from cleanup operation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CleanupReport {
    /// Number of sessions compacted.
    pub sessions_compacted: u32,
    /// Task descriptions of compacted sessions.
    pub compacted_tasks: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = StaleSessionConfig::default();
        assert_eq!(config.ask_threshold_secs, 24 * 60 * 60);
        assert_eq!(config.auto_compact_threshold_secs, 7 * 24 * 60 * 60);
    }

    #[test]
    fn test_duration_conversions() {
        let config = StaleSessionConfig::default();
        assert_eq!(config.ask_threshold(), Duration::from_secs(24 * 60 * 60));
        assert_eq!(
            config.auto_compact_threshold(),
            Duration::from_secs(7 * 24 * 60 * 60)
        );
    }
}
