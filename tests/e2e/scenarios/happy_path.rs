use crate::harness::{Assertion, Scenario};

#[test]
fn test_single_task_complete() {
    Scenario::new("single_task_complete")
        .from_fixture("default")
        .user_starts_task("Add login button")
        .agent_writes("src/button.rs", b"pub fn login() {}")
        .agent_flushes()
        .agent_completes("Added login button")
        .user_confirms()
        .assert_no_session()
        .assert_commit_count(2) // Initial commit + task commit
        .assert_head_contains("login button")
        .run()
        .expect("scenario should pass");
}

#[test]
fn test_task_with_multiple_files() {
    Scenario::new("task_with_multiple_files")
        .from_fixture("default")
        .user_starts_task("Add error handling")
        .agent_reads("src/main.rs")
        .agent_writes("src/main.rs", b"fn main() -> Result<()> { Ok(()) }")
        .agent_writes("src/error.rs", b"pub type Error = anyhow::Error;")
        .agent_flushes()
        .agent_completes("Added Result types")
        .user_confirms()
        .assert_file_committed("src/main.rs")
        .assert_file_committed("src/error.rs")
        .run()
        .unwrap();
}

#[test]
fn test_task_with_commands() {
    Scenario::new("task_with_commands")
        .from_fixture("default")
        .user_starts_task("Fix failing tests")
        .agent_runs("cargo test", 1, "test failed: assertion error")
        .agent_writes("src/lib.rs", b"// fixed")
        .agent_runs("cargo test", 0, "test passed")
        .agent_flushes()
        .agent_completes("Fixed tests")
        .user_confirms()
        .assert_commit_count(2) // Initial commit + task commit
        .run()
        .unwrap();
}

#[test]
fn test_task_with_notes() {
    Scenario::new("task_with_notes")
        .from_fixture("default")
        .user_starts_task("Refactor auth")
        .agent_notes("Decided to use JWT instead of sessions")
        .agent_notes("Chose RS256 for signing")
        .agent_flushes()
        .agent_completes("Refactored to JWT")
        .user_confirms()
        .assert_note_contains("JWT")
        .assert_note_contains("RS256")
        .run()
        .unwrap();
}

#[test]
fn test_multiple_flush_steps() {
    Scenario::new("multiple_flushes")
        .from_fixture("default")
        .user_starts_task("Multi-step task")
        .agent_writes("step1.txt", b"step 1")
        .agent_flushes()
        .agent_writes("step2.txt", b"step 2")
        .agent_flushes()
        .agent_writes("step3.txt", b"step 3")
        .agent_flushes()
        .agent_completes("Done in 3 steps")
        .user_confirms()
        .assert_commit_count(2) // Initial commit + task commit
        .assert(Assertion::StagingChainLengthGte(3))
        .run()
        .unwrap();
}
