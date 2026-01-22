use crate::harness::{Assertion, Scenario};

#[test]
fn test_query_finds_committed_files() {
    Scenario::new("query_finds_files")
        .user_starts_task("Add auth module")
        .agent_writes("src/auth.rs", b"pub fn login() { /* auth logic */ }")
        .agent_writes("src/db.rs", b"pub fn connect() { /* db logic */ }")
        .agent_flushes()
        .agent_completes("Added modules")
        .user_confirms()
        .assert(Assertion::QueryReturnsPath {
            query: "authentication login".into(),
            path: "src/auth.rs".into(),
        })
        .run()
        .unwrap();
}

#[test]
fn test_query_respects_token_budget() {
    Scenario::new("token_budget")
        .user_starts_task("Add many files")
        // Add many files to exceed budget
        .agent_writes("src/file1.rs", &vec![b'x'; 10000])
        .agent_writes("src/file2.rs", &vec![b'y'; 10000])
        .agent_writes("src/file3.rs", &vec![b'z'; 10000])
        .agent_flushes()
        .agent_completes("Added files")
        .user_confirms()
        .assert(Assertion::QueryTokensWithinBudget {
            query: "files".into(),
            budget: 1000,
        })
        .run()
        .unwrap();
}

#[test]
fn test_narrative_in_retrieval() {
    Scenario::new("narrative_retrieval")
        .user_starts_task("Make design decision")
        .agent_notes("Chose REST over GraphQL for simplicity")
        .agent_notes("Will use JWT for auth tokens")
        .agent_flushes()
        .agent_completes("Design documented")
        .user_confirms()
        .assert_note_contains("REST")
        .assert_note_contains("JWT")
        .run()
        .unwrap();
}
