//! Initialize a new CTX repository.

use anyhow::{Context, Result};
use ctx_core::CtxRepo;

/// Initialize a new CTX repository in the current directory.
pub fn run() -> Result<()> {
    let repo = CtxRepo::init(".").context("Failed to initialize CTX repository")?;

    // Get initial commit info
    let head_id = repo.head_id()?;
    let head = repo.head()?;

    println!("Initialized CTX repository in .ctx/");
    println!();
    println!("Directory structure:");
    println!("  .ctx/objects/          - Content-addressed object storage");
    println!("  .ctx/refs/             - Commit pointers");
    println!("  .ctx/narrative/        - Human-readable documentation");
    println!("  .ctx/narrative/log/    - Daily journal entries");
    println!("  .ctx/narrative/tasks/  - Task documentation");
    println!("  .ctx/index/            - Rebuildable indexes (gitignored)");
    println!();
    println!("Configuration written to .ctx/config.toml");
    println!();
    println!("Initial commit: {}", head_id.as_hex());
    println!("  Message: {}", head.message);
    println!("  Timestamp: {}", head.timestamp_unix);

    Ok(())
}
