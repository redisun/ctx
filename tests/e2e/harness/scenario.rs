use super::assertions::{Assertion, SessionStateMatch};
use super::runner::ScenarioRunner;
use super::steps::ScenarioStep;
use std::collections::HashMap;
use std::time::Duration;

/// Fluent DSL for building test scenarios
pub struct Scenario {
    name: String,
    initial_files: HashMap<String, Vec<u8>>,
    steps: Vec<ScenarioStep>,
}

impl Scenario {
    /// Create a new scenario with the given name
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            initial_files: HashMap::new(),
            steps: Vec::new(),
        }
    }

    // ===== Initial setup =====

    /// Add a single file to initial workspace
    pub fn with_file(mut self, path: &str, content: &[u8]) -> Self {
        self.initial_files
            .insert(path.to_string(), content.to_vec());
        self
    }

    /// Add multiple files to initial workspace
    pub fn with_files(mut self, files: HashMap<&str, &[u8]>) -> Self {
        for (path, content) in files {
            self.initial_files
                .insert(path.to_string(), content.to_vec());
        }
        self
    }

    /// Load initial workspace from fixtures
    pub fn from_fixture(self, _fixture_name: &str) -> Self {
        // TODO: Implement fixture loading
        // For now, just return self
        self
    }

    // ===== User actions =====

    /// User sends message to start a new task
    pub fn user_starts_task(mut self, description: &str) -> Self {
        self.steps.push(ScenarioStep::UserStartTask {
            description: description.to_string(),
        });
        self
    }

    /// User responds to agent question
    pub fn user_responds(mut self, text: &str) -> Self {
        self.steps.push(ScenarioStep::UserResponse {
            text: text.to_string(),
        });
        self
    }

    /// User intervenes mid-task with new instructions
    pub fn user_intervenes(mut self, message: &str) -> Self {
        self.steps.push(ScenarioStep::UserIntervention {
            message: message.to_string(),
        });
        self
    }

    /// User confirms completion
    pub fn user_confirms(mut self) -> Self {
        self.steps.push(ScenarioStep::UserConfirmation);
        self
    }

    /// User rejects completion with feedback
    pub fn user_rejects(mut self, feedback: &str) -> Self {
        self.steps.push(ScenarioStep::UserRejection {
            feedback: feedback.to_string(),
        });
        self
    }

    // ===== Agent actions =====

    /// Agent reads a file
    pub fn agent_reads(mut self, path: &str) -> Self {
        self.steps.push(ScenarioStep::AgentReadFile {
            path: path.to_string(),
        });
        self
    }

    /// Agent writes a file
    pub fn agent_writes(mut self, path: &str, content: &[u8]) -> Self {
        self.steps.push(ScenarioStep::AgentWriteFile {
            path: path.to_string(),
            content: content.to_vec(),
        });
        self
    }

    /// Agent runs a command
    pub fn agent_runs(mut self, cmd: &str, exit_code: i32, output: &str) -> Self {
        self.steps.push(ScenarioStep::AgentRunCommand {
            command: cmd.to_string(),
            exit_code,
            output: output.to_string(),
        });
        self
    }

    /// Agent adds a note
    pub fn agent_notes(mut self, text: &str) -> Self {
        self.steps.push(ScenarioStep::AgentNote {
            text: text.to_string(),
        });
        self
    }

    /// Agent flushes current step
    pub fn agent_flushes(mut self) -> Self {
        self.steps.push(ScenarioStep::AgentFlush);
        self
    }

    /// Agent asks user a question
    pub fn agent_asks(mut self, question: &str) -> Self {
        self.steps.push(ScenarioStep::AgentAskQuestion {
            question: question.to_string(),
        });
        self
    }

    /// Agent marks task as complete
    pub fn agent_completes(mut self, summary: &str) -> Self {
        self.steps.push(ScenarioStep::AgentComplete {
            summary: summary.to_string(),
        });
        self
    }

    /// Agent resumes from interruption
    pub fn agent_resumes(mut self) -> Self {
        self.steps.push(ScenarioStep::AgentResume);
        self
    }

    /// Agent abandons task
    pub fn agent_abandons(mut self, reason: &str) -> Self {
        self.steps.push(ScenarioStep::AgentAbandon {
            reason: reason.to_string(),
        });
        self
    }

    // ===== Compound agent actions =====

    /// Agent performs N generic work steps
    pub fn agent_work_steps(mut self, count: usize) -> Self {
        for i in 0..count {
            self = self.agent_writes(&format!("work_step_{}.txt", i), b"work");
        }
        self.agent_flushes()
    }

    // ===== Time control =====

    /// Wait for a duration
    pub fn wait(mut self, duration: Duration) -> Self {
        self.steps.push(ScenarioStep::Wait { duration });
        self
    }

    /// Wait for N hours
    pub fn wait_hours(mut self, hours: u64) -> Self {
        self.steps.push(ScenarioStep::WaitHours { hours });
        self
    }

    /// Wait for N days
    pub fn wait_days(mut self, days: u64) -> Self {
        self.steps.push(ScenarioStep::WaitDays { days });
        self
    }

    // ===== Failure simulation =====

    /// Simulate a crash
    pub fn crash(mut self) -> Self {
        self.steps.push(ScenarioStep::Crash);
        self
    }

    /// Restart after crash
    pub fn restart(mut self) -> Self {
        self.steps.push(ScenarioStep::Restart);
        self
    }

    // ===== Assertions =====

    /// Add a general assertion
    pub fn assert(mut self, assertion: Assertion) -> Self {
        self.steps.push(ScenarioStep::Assert { assertion });
        self
    }

    /// Assert session is in specific state
    pub fn assert_session_state(mut self, state: SessionStateMatch) -> Self {
        self.steps.push(ScenarioStep::Assert {
            assertion: Assertion::SessionState(state),
        });
        self
    }

    /// Assert no active session
    pub fn assert_no_session(mut self) -> Self {
        self.steps.push(ScenarioStep::Assert {
            assertion: Assertion::NoSession,
        });
        self
    }

    /// Assert specific commit count
    pub fn assert_commit_count(mut self, count: usize) -> Self {
        self.steps.push(ScenarioStep::Assert {
            assertion: Assertion::CommitCount(count),
        });
        self
    }

    /// Assert HEAD commit message contains text
    pub fn assert_head_contains(mut self, text: &str) -> Self {
        self.steps.push(ScenarioStep::Assert {
            assertion: Assertion::HeadMessageContains(text.to_string()),
        });
        self
    }

    /// Assert file is committed in HEAD
    pub fn assert_file_committed(mut self, path: &str) -> Self {
        self.steps.push(ScenarioStep::Assert {
            assertion: Assertion::FileInHead {
                path: path.to_string(),
            },
        });
        self
    }

    /// Assert staging area exists
    pub fn assert_staging_exists(mut self) -> Self {
        self.steps.push(ScenarioStep::Assert {
            assertion: Assertion::StagingExists,
        });
        self
    }

    /// Assert no staging area
    pub fn assert_no_staging(mut self) -> Self {
        self.steps.push(ScenarioStep::Assert {
            assertion: Assertion::NoStaging,
        });
        self
    }

    /// Assert narrative contains text
    pub fn assert_note_contains(mut self, text: &str) -> Self {
        self.steps.push(ScenarioStep::Assert {
            assertion: Assertion::NoteContains(text.to_string()),
        });
        self
    }

    // ===== Execution =====

    /// Execute the scenario and return results
    pub fn run(self) -> ScenarioResult {
        let mut runner = match ScenarioRunner::new(self.initial_files.clone()) {
            Ok(r) => r,
            Err(e) => {
                return ScenarioResult {
                    name: self.name.clone(),
                    success: false,
                    steps_executed: 0,
                    failure_step: Some(0),
                    error: Some(format!("Failed to create runner: {}", e)),
                }
            }
        };

        match runner.execute(&self.steps) {
            Ok(()) => ScenarioResult {
                name: self.name,
                success: true,
                steps_executed: self.steps.len(),
                failure_step: None,
                error: None,
            },
            Err(e) => {
                // Try to determine which step failed
                let failure_step = runner.current_step();
                ScenarioResult {
                    name: self.name,
                    success: false,
                    steps_executed: failure_step,
                    failure_step: Some(failure_step),
                    error: Some(format!("{:?}", e)),
                }
            }
        }
    }
}

/// Result of running a scenario
#[derive(Debug)]
pub struct ScenarioResult {
    pub name: String,
    pub success: bool,
    pub steps_executed: usize,
    pub failure_step: Option<usize>,
    pub error: Option<String>,
}

impl ScenarioResult {
    /// Unwrap the result, panicking if it failed
    pub fn unwrap(self) {
        if !self.success {
            panic!(
                "Scenario '{}' failed at step {}: {}",
                self.name,
                self.failure_step.unwrap_or(0),
                self.error.unwrap_or_else(|| "unknown error".to_string())
            );
        }
    }

    /// Expect the result to be successful
    pub fn expect(self, msg: &str) {
        if !self.success {
            panic!(
                "{}: Scenario '{}' failed at step {}: {}",
                msg,
                self.name,
                self.failure_step.unwrap_or(0),
                self.error.unwrap_or_else(|| "unknown error".to_string())
            );
        }
    }
}
