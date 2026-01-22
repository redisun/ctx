use crate::harness::{Scenario, SessionStateMatch};

#[test]
fn test_running_to_awaiting_user() {
    Scenario::new("running_to_awaiting")
        .user_starts_task("Need clarification")
        .assert_session_state(SessionStateMatch::Running)
        .agent_asks("Redis or Memcached?")
        .assert_session_state(SessionStateMatch::AwaitingUser)
        .run()
        .unwrap();
}

#[test]
fn test_awaiting_user_to_running() {
    Scenario::new("awaiting_to_running")
        .user_starts_task("Task")
        .agent_asks("Which option?")
        .assert_session_state(SessionStateMatch::AwaitingUser)
        .user_responds("Option A")
        .assert_session_state(SessionStateMatch::Running)
        .run()
        .unwrap();
}

#[test]
fn test_running_to_interrupted() {
    Scenario::new("running_to_interrupted")
        .user_starts_task("Initial task")
        .agent_writes("file.txt", b"content")
        .agent_flushes()
        .assert_session_state(SessionStateMatch::Running)
        .user_intervenes("Actually, change the approach")
        .assert_session_state(SessionStateMatch::Interrupted)
        .run()
        .unwrap();
}

#[test]
fn test_interrupted_to_running() {
    Scenario::new("interrupted_to_running")
        .user_starts_task("Task")
        .agent_flushes()
        .user_intervenes("Change this")
        .assert_session_state(SessionStateMatch::Interrupted)
        .agent_notes("Acknowledged user feedback")
        .agent_resumes()
        .assert_session_state(SessionStateMatch::Running)
        .run()
        .unwrap();
}

#[test]
fn test_running_to_pending_complete() {
    Scenario::new("running_to_pending")
        .user_starts_task("Task")
        .agent_flushes()
        .assert_session_state(SessionStateMatch::Running)
        .agent_completes("Done")
        .assert_session_state(SessionStateMatch::PendingComplete)
        .run()
        .unwrap();
}

#[test]
fn test_pending_to_complete() {
    Scenario::new("pending_to_complete")
        .user_starts_task("Task")
        .agent_completes("Done")
        .assert_session_state(SessionStateMatch::PendingComplete)
        .user_confirms()
        .assert_no_session() // Session ended
        .assert_commit_count(2) // Initial commit + task commit
        .run()
        .unwrap();
}

#[test]
fn test_pending_to_running_on_rejection() {
    Scenario::new("pending_to_running")
        .user_starts_task("Task")
        .agent_completes("First attempt")
        .assert_session_state(SessionStateMatch::PendingComplete)
        .user_rejects("Not quite right")
        .assert_session_state(SessionStateMatch::Running)
        .agent_completes("Second attempt")
        .user_confirms()
        .assert_commit_count(2) // Initial commit + task commit
        .run()
        .unwrap();
}

#[test]
fn test_multiple_questions_and_answers() {
    Scenario::new("multiple_qa")
        .user_starts_task("Complex task")
        .agent_asks("Question 1?")
        .user_responds("Answer 1")
        .agent_asks("Question 2?")
        .user_responds("Answer 2")
        .agent_completes("Done with both answers")
        .user_confirms()
        .assert_commit_count(2) // Initial commit + task commit
        .run()
        .unwrap();
}
