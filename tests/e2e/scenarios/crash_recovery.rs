use crate::harness::{Assertion, Scenario, SessionStateMatch};

#[test]
fn test_crash_after_flush_recovers() {
    Scenario::new("crash_after_flush")
        .from_fixture("default")
        .user_starts_task("Important work")
        .agent_writes("important.txt", b"critical data")
        .agent_notes("Key decision made")
        .agent_flushes()
        .crash()
        .restart()
        .assert(Assertion::SessionRecovered)
        .assert_session_state(SessionStateMatch::Running)
        .assert(Assertion::StagingContainsFile {
            path: "important.txt".into(),
        })
        .assert(Assertion::StagingContainsNote {
            text: "Key decision".into(),
        })
        .run()
        .unwrap();
}

#[test]
fn test_crash_before_flush_loses_pending() {
    Scenario::new("crash_before_flush")
        .from_fixture("default")
        .user_starts_task("Work")
        .agent_writes("saved.txt", b"saved")
        .agent_flushes() // This is saved
        .agent_writes("lost.txt", b"lost")
        .agent_notes("Lost note")
        // NO flush before crash
        .crash()
        .restart()
        .assert(Assertion::SessionRecovered)
        .assert(Assertion::StagingContainsFile {
            path: "saved.txt".into(),
        })
        .assert(Assertion::FileNotInHead {
            path: "lost.txt".into(),
        })
        .run()
        .unwrap();
}

#[test]
fn test_crash_while_awaiting_user() {
    Scenario::new("crash_awaiting")
        .from_fixture("default")
        .user_starts_task("Task")
        .agent_asks("Which option?")
        .agent_flushes()
        .crash()
        .restart()
        .assert(Assertion::SessionRecovered)
        .assert_session_state(SessionStateMatch::AwaitingUser)
        .user_responds("Option A")
        .assert_session_state(SessionStateMatch::Running)
        .run()
        .unwrap();
}

#[test]
fn test_crash_while_pending_complete() {
    Scenario::new("crash_pending")
        .from_fixture("default")
        .user_starts_task("Task")
        .agent_writes("file.txt", b"content")
        .agent_completes("Finished")
        .agent_flushes()
        .crash()
        .restart()
        .assert(Assertion::SessionRecovered)
        .assert_session_state(SessionStateMatch::PendingComplete)
        .user_confirms()
        .assert_commit_count(2) // Initial commit + task commit
        .run()
        .unwrap();
}

#[test]
fn test_no_recovery_needed_after_compact() {
    Scenario::new("no_recovery_after_compact")
        .from_fixture("default")
        .user_starts_task("Task")
        .agent_completes("Done")
        .user_confirms() // Compacted
        .assert_no_session()
        .crash()
        .restart()
        .assert_no_session() // No recovery needed
        .assert_no_staging()
        .run()
        .unwrap();
}

#[test]
fn test_abandon_creates_commit() {
    Scenario::new("abandon_creates_commit")
        .from_fixture("default")
        .user_starts_task("Task to abandon")
        .agent_writes("partial.txt", b"partial work")
        .agent_flushes()
        .agent_abandons("User changed their mind")
        .assert_no_session()
        .assert_commit_count(2) // Initial commit + task commit
        .assert_head_contains("Aborted") // Abort message contains "Aborted:"
        .run()
        .unwrap();
}
