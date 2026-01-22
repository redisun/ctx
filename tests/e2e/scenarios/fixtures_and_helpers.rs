use crate::harness::{Assertion, Scenario};

/// Test that fixture loading works correctly
#[test]
fn test_fixture_loading() {
    Scenario::new("fixture_loading")
        .from_fixture("default")
        .user_starts_task("Verify fixture files exist")
        // Files from fixture should be available in workspace
        // We need to write them to CTX to track them
        .agent_writes("Cargo.toml", b"[package]\nname = \"test_project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n")
        .agent_writes("src/main.rs", b"fn main() {\n    println!(\"Hello, world!\");\n}\n")
        .agent_writes("src/lib.rs", b"pub fn greet(name: &str) -> String {\n    format!(\"Hello, {}!\", name)\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn test_greet() {\n        assert_eq!(greet(\"world\"), \"Hello, world!\");\n    }\n}\n")
        .agent_writes("README.md", b"# Test Project\n\nThis is a default fixture for CTX e2e tests.\n")
        .agent_flushes()
        .agent_completes("Verified all fixture files")
        .user_confirms()
        .assert_file_committed("Cargo.toml")
        .assert_file_committed("src/main.rs")
        .assert_file_committed("src/lib.rs")
        .assert_file_committed("README.md")
        .run()
        .unwrap();
}

/// Test that agent_work_steps helper works correctly
#[test]
fn test_agent_work_steps() {
    Scenario::new("agent_work_steps")
        .from_fixture("default")
        .user_starts_task("Perform multiple work steps")
        .agent_work_steps(5) // Creates 5 files and flushes
        .agent_completes("Completed 5 work steps")
        .user_confirms()
        .assert_commit_count(2) // Initial commit + task commit
        .assert(Assertion::StagingChainLengthGte(1)) // At least 1 flush happened
        .run()
        .unwrap();
}

/// Test that assert_staging_exists works correctly
#[test]
fn test_assert_staging_exists() {
    Scenario::new("assert_staging_exists")
        .from_fixture("default")
        .user_starts_task("Test staging assertions")
        .agent_writes("test.txt", b"content")
        .agent_flushes()
        // Staging should exist after flush
        .assert_staging_exists()
        .agent_completes("Done")
        .user_confirms()
        // After compaction, staging should be gone
        .assert_no_staging()
        .run()
        .unwrap();
}

/// Test staging exists during active session
#[test]
fn test_staging_exists_during_session() {
    Scenario::new("staging_during_session")
        .from_fixture("default")
        .user_starts_task("Task with staging")
        // Staging should exist immediately after starting session
        .assert_staging_exists()
        .agent_writes("file1.txt", b"content1")
        .agent_flushes()
        // Staging still exists after flush
        .assert_staging_exists()
        .agent_writes("file2.txt", b"content2")
        .agent_flushes()
        // Staging still exists
        .assert_staging_exists()
        .agent_completes("Done")
        .user_confirms()
        // After compaction, staging is gone
        .assert_no_staging()
        .run()
        .unwrap();
}

/// Test fixture loading with file modifications
#[test]
fn test_fixture_with_modifications() {
    Scenario::new("fixture_modifications")
        .from_fixture("default")
        .user_starts_task("Modify fixture files")
        // Read existing fixture file
        .agent_reads("src/main.rs")
        // Modify a file from the fixture
        .agent_writes("src/main.rs", b"fn main() {\n    println!(\"Modified!\");\n}")
        .agent_writes("src/new_file.rs", b"pub fn new() {}")
        .agent_flushes()
        .agent_completes("Modified and added files")
        .user_confirms()
        .assert_file_committed("src/main.rs")
        .assert_file_committed("src/new_file.rs")
        // Note: Original fixture files exist in workspace but aren't in HEAD
        // unless explicitly written/observed during the session
        .run()
        .unwrap();
}

/// Test that agent_work_steps creates the expected files
#[test]
fn test_agent_work_steps_creates_files() {
    Scenario::new("work_steps_files")
        .from_fixture("default")
        .user_starts_task("Create work step files")
        .agent_work_steps(3)
        .agent_completes("Created 3 work steps")
        .user_confirms()
        // Verify the work step files were created
        .assert_file_committed("work_step_0.txt")
        .assert_file_committed("work_step_1.txt")
        .assert_file_committed("work_step_2.txt")
        .run()
        .unwrap();
}

/// Test fixture loading with nested directories
#[test]
fn test_fixture_nested_structure() {
    Scenario::new("fixture_nested")
        .from_fixture("default")
        .user_starts_task("Verify nested structure")
        // Verify nested paths work correctly - files exist in workspace
        .agent_reads("src/main.rs")
        .agent_reads("src/lib.rs")
        // Write them to track in CTX
        .agent_writes("src/main.rs", b"fn main() {\n    println!(\"Hello, world!\");\n}\n")
        .agent_writes("src/lib.rs", b"pub fn greet(name: &str) -> String {\n    format!(\"Hello, {}!\", name)\n}\n")
        .agent_flushes()
        .agent_completes("Verified nested structure")
        .user_confirms()
        // All nested files should be committed
        .assert_file_committed("src/main.rs")
        .assert_file_committed("src/lib.rs")
        .run()
        .unwrap();
}

/// Test that staging exists after crash and recovery
#[test]
fn test_staging_exists_after_recovery() {
    Scenario::new("staging_after_recovery")
        .from_fixture("default")
        .user_starts_task("Task with crash")
        .agent_writes("important.txt", b"data")
        .agent_flushes()
        .assert_staging_exists()
        .crash()
        .restart()
        // After recovery, staging should still exist
        .assert_staging_exists()
        .assert(Assertion::SessionRecovered)
        .agent_completes("Recovered and completed")
        .user_confirms()
        // After compaction, staging is gone
        .assert_no_staging()
        .run()
        .unwrap();
}
