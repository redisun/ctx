//! Session (staging area) management commands.

use anyhow::Result;
use ctx_core::CtxRepo;

/// Ensures the repository has an active session, recovering from STAGE if needed.
///
/// This allows CLI commands to work across multiple invocations by reconstructing
/// the session state from the staging area.
fn ensure_session_recovered(repo: &mut CtxRepo) -> Result<()> {
    if !repo.has_active_session() && repo.recover_session()?.is_none() {
        return Err(anyhow::anyhow!(
            "No active session. Use 'ctx stage start' first."
        ));
    }
    Ok(())
}

pub fn start(task: &str) -> Result<()> {
    let mut repo = CtxRepo::open(".")?;

    // Check for stale sessions
    if repo.has_active_session() {
        return Err(anyhow::anyhow!("A session is already active. Use 'ctx stage compact' or 'ctx stage abort' to finish it."));
    }

    // Try to recover existing session first
    if repo.recover_session()?.is_some() {
        println!("Recovered existing session from staging area");
        println!("Use 'ctx stage status' to see details");
        return Ok(());
    }

    // Start new session
    let session = repo.start_session(task)?;
    println!("Started new session: {}", session.task_description());
    println!("Session ID: {}", session.session_id());

    Ok(())
}

pub fn status() -> Result<()> {
    let mut repo = CtxRepo::open(".")?;

    // Try to recover session if one exists
    let _ = repo.recover_session()?;

    match repo.active_session() {
        Some(session) => {
            println!("Active session:");
            println!("  Task: {}", session.task_description());
            println!("  Session ID: {}", session.session_id());
            println!("  State: {:?}", session.state());
            println!("  Steps completed: {}", session.step_count());
            println!("  Created: {} (Unix timestamp)", session.created_at());
            println!(
                "  Idle time: {:.0} seconds",
                session.idle_time().as_secs_f64()
            );

            // Show progress summary if available
            if let Ok(summary) = session.generate_progress_summary(repo.object_store()) {
                println!("\n{}", summary);
            }
        }
        None => {
            println!("No active session");
        }
    }

    Ok(())
}

pub fn flush() -> Result<()> {
    let mut repo = CtxRepo::open(".")?;
    ensure_session_recovered(&mut repo)?;

    let work_id = repo.flush_active_session()?;
    let step_count = repo.active_session().unwrap().step_count();

    println!("Flushed step to staging: {}", work_id.as_hex());
    println!("Steps completed: {}", step_count);

    Ok(())
}

pub fn compact(message: &str) -> Result<()> {
    let mut repo = CtxRepo::open(".")?;
    ensure_session_recovered(&mut repo)?;

    let commit_id = repo.compact_session(message)?;

    println!("Compacted session into commit: {}", commit_id.as_hex());
    println!("Session complete!");

    Ok(())
}

pub fn abort(reason: Option<String>) -> Result<()> {
    let mut repo = CtxRepo::open(".")?;
    ensure_session_recovered(&mut repo)?;

    let reason_text = reason.unwrap_or_else(|| "User aborted".to_string());
    let commit_id = repo.abort_session(&reason_text)?;

    println!("Aborted session: {}", reason_text);
    println!("Created abort commit: {}", commit_id.as_hex());

    Ok(())
}

pub fn recover() -> Result<()> {
    let mut repo = CtxRepo::open(".")?;

    match repo.recover_session()? {
        Some(session) => {
            println!("Recovered session from staging area:");
            println!("  Task: {}", session.task_description());
            println!("  Session ID: {}", session.session_id());
            println!("  Steps completed: {}", session.step_count());
            println!("  State: {:?}", session.state());
            Ok(())
        }
        None => {
            println!("No staging area found - nothing to recover");
            Ok(())
        }
    }
}
