use crate::harness::{Assertion, Scenario};
use ctx_core::{StaleSessionConfig, StaleSessionStatus};

#[test]
fn test_fresh_session_not_stale() {
    Scenario::new("fresh_not_stale")
        .user_starts_task("Fresh task")
        .agent_flushes()
        .wait_hours(1) // Only 1 hour
        .assert(Assertion::Custom(Box::new(|ctx| {
            let status = ctx.check_stale_session(&StaleSessionConfig::default());
            assert!(matches!(status, StaleSessionStatus::Fresh { .. }));
            Ok(())
        })))
        .run()
        .unwrap();
}

#[test]
fn test_moderately_stale_session() {
    Scenario::new("moderately_stale")
        .user_starts_task("Old task")
        .agent_flushes()
        .wait_hours(36) // Past 24h threshold
        .assert(Assertion::Custom(Box::new(|ctx| {
            let status = ctx.check_stale_session(&StaleSessionConfig::default());
            assert!(matches!(status, StaleSessionStatus::ShouldAsk { .. }));
            Ok(())
        })))
        .run()
        .unwrap();
}

#[test]
fn test_very_stale_auto_compacts() {
    Scenario::new("very_stale")
        .user_starts_task("Ancient task")
        .agent_writes("old.txt", b"old content")
        .agent_flushes()
        .wait_days(10) // Past 7d auto-compact threshold
        .assert(Assertion::Custom(Box::new(|ctx| {
            let status = ctx.check_stale_session(&StaleSessionConfig::default());
            assert!(matches!(
                status,
                StaleSessionStatus::ShouldAutoCompact { .. }
            ));
            Ok(())
        })))
        .run()
        .unwrap();
}

#[test]
fn test_cleanup_stale_removes_session() {
    Scenario::new("cleanup_stale")
        .user_starts_task("Task to cleanup")
        .agent_writes("file.txt", b"content")
        .agent_flushes()
        .wait_days(10)
        .assert(Assertion::Custom(Box::new(|ctx| {
            let config = StaleSessionConfig::default();
            let report = ctx.cleanup_stale_sessions(config.auto_compact_threshold())?;
            assert!(report.sessions_compacted > 0);
            Ok(())
        })))
        .assert_no_session()
        .assert_commit_count(2) // Initial commit + task commit
        .run()
        .unwrap();
}
