//! Rebuild command implementation.

use anyhow::{Context, Result};
use ctx_core::CtxRepo;
use std::time::Instant;

/// Rebuild all indexes from immutable objects.
pub fn run() -> Result<()> {
    let start = Instant::now();

    let mut repo = CtxRepo::open(".").context("Not a CTX repository (no .ctx directory found)")?;

    println!("Rebuilding index...");

    repo.rebuild_index().context("Failed to rebuild index")?;

    let elapsed = start.elapsed();
    println!(
        "Index rebuilt successfully in {:.2}s",
        elapsed.as_secs_f64()
    );

    Ok(())
}
