//! Commit command for creating canonical commits.

use anyhow::{Context, Result};
use ctx_core::CtxRepo;

/// Create a new commit with the current narrative state.
pub fn run(message: &str, no_narrative: bool) -> Result<()> {
    let repo = CtxRepo::open(".").context("Not a CTX repository")?;

    let narrative_refs = if no_narrative {
        Some(vec![]) // Explicit empty
    } else {
        None // Auto-detect
    };

    let commit_id = repo.commit(message, narrative_refs, "user")?;

    println!("Created commit {}", commit_id.as_hex());

    // Show what was included
    let commit: ctx_core::Commit = repo.object_store().get_typed(commit_id)?;

    if !commit.narrative_refs.is_empty() {
        println!("\nNarrative files snapshotted:");
        for nr in &commit.narrative_refs {
            println!("  {} ({})", nr.path, &nr.blob_id.as_hex()[..8]);
        }
    }

    Ok(())
}
