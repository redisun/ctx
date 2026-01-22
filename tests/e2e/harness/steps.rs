use std::time::Duration;

use super::assertions::Assertion;

/// All possible actions in a test scenario
#[derive(Debug)]
pub enum ScenarioStep {
    // User actions
    UserStartTask {
        description: String,
    },
    UserResponse {
        text: String,
    },
    UserIntervention {
        message: String,
    },
    UserConfirmation,
    UserRejection {
        feedback: String,
    },

    // Agent actions
    AgentReadFile {
        path: String,
    },
    AgentWriteFile {
        path: String,
        content: Vec<u8>,
    },
    AgentRunCommand {
        command: String,
        exit_code: i32,
        output: String,
    },
    AgentNote {
        text: String,
    },
    AgentFlush,
    AgentAskQuestion {
        question: String,
    },
    AgentComplete {
        summary: String,
    },
    AgentResume,
    AgentAbandon {
        reason: String,
    },

    // Time control
    Wait {
        duration: Duration,
    },
    WaitHours {
        hours: u64,
    },
    WaitDays {
        days: u64,
    },

    // Failure simulation
    Crash,
    Restart,

    // Assertions (can be interspersed)
    Assert {
        assertion: Assertion,
    },
}
