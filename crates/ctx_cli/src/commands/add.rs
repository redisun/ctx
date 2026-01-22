//! Add commands for notes and tasks.

use anyhow::{Context, Result};
use chrono::Local;
use ctx_core::CtxRepo;

/// Add a note to today's log.
pub fn note(text: &str) -> Result<()> {
    let repo = CtxRepo::open(".").context("Not a CTX repository")?;
    let ns = repo.narrative();
    ns.ensure_structure()?;

    let now = Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    let time = now.format("%H:%M").to_string();

    let path = ns.append_log(&date, &time, text)?;

    println!("Added note to {}", path);
    Ok(())
}

/// Create a new task.
pub fn task(title: &str, body: Option<&str>) -> Result<()> {
    let repo = CtxRepo::open(".").context("Not a CTX repository")?;
    let ns = repo.narrative();
    ns.ensure_structure()?;

    let task = ns.create_task(title, body.unwrap_or(""))?;

    println!("Created task #{:04}: {}", task.id, task.relative_path);
    Ok(())
}

/// Update a task's status.
pub fn task_update(id: u32, status: &str, note: Option<&str>) -> Result<()> {
    let repo = CtxRepo::open(".").context("Not a CTX repository")?;
    let ns = repo.narrative();

    let path = ns.update_task(id, status, note.unwrap_or(""))?;

    println!("Updated task #{:04}: status -> {}", id, status);
    println!("  File: {}", path);
    Ok(())
}
