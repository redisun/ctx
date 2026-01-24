//! Intelligent session handling for user message processing.
//!
//! This module provides the `SessionHandler` which coordinates message classification
//! with stale session detection to provide actionable guidance to agent orchestrators.

use crate::classification::{classify_message, ClassificationContext, MessageClassification};
use crate::config::{StaleSessionConfig, StaleSessionStatus};
use crate::error::Result;
use crate::types::SessionState;
use crate::CtxRepo;
use std::time::Duration;

/// Response from session intelligence layer.
///
/// This tells the agent orchestrator what action to take based on the
/// combination of user message classification and session state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionResponse {
    /// Proceed with the classified action.
    /// The agent should handle the message according to its classification.
    Proceed {
        /// How the message was classified.
        classification: MessageClassification,
    },

    /// Prompt the user for a decision before proceeding.
    /// The agent should present the prompt and wait for user choice.
    Prompt {
        /// Message to display to the user.
        message: String,
        /// The pending action awaiting user decision.
        pending: PendingAction,
    },

    /// Session was auto-compacted due to staleness.
    /// The agent should acknowledge and proceed with the new task.
    AutoCompacted {
        /// Description of the old task that was compacted.
        old_task: String,
        /// The user's message that triggered this.
        user_message: String,
    },

    /// No session exists; start a new one with this message as the task.
    StartNew {
        /// The user's message (potential task description).
        user_message: String,
    },
}

/// Pending action awaiting user decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    /// User needs to choose: continue old work or start fresh.
    /// Triggered when session is stale (idle > ask_threshold).
    StaleSessionChoice {
        /// The user's message that triggered this.
        user_message: String,
        /// The old task description.
        old_task: String,
        /// How long the session has been idle.
        idle_duration: Duration,
    },

    /// User needs to confirm: save current work and start new task, or continue current.
    /// Triggered when message classified as NewTask but session is active.
    NewTaskChoice {
        /// The new task from user's message.
        new_task: String,
        /// The current task description.
        current_task: String,
    },
}

/// User's choice in response to a pending action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserChoice {
    /// Continue with the current/old session.
    Continue,
    /// Start fresh (compact old session and start new).
    StartFresh,
}

/// Handles user messages with session intelligence.
///
/// The handler coordinates:
/// 1. Message classification (what kind of message is this?)
/// 2. Stale session detection (is the session too old?)
/// 3. Action determination (what should the agent do?)
pub struct SessionHandler {
    stale_config: StaleSessionConfig,
}

impl SessionHandler {
    /// Creates a new session handler with the given stale session configuration.
    pub fn new(stale_config: StaleSessionConfig) -> Self {
        Self { stale_config }
    }

    /// Creates a session handler with default configuration.
    pub fn with_defaults() -> Self {
        Self {
            stale_config: StaleSessionConfig::default(),
        }
    }

    /// Process an incoming user message with full session intelligence.
    ///
    /// This is the main entry point for handling user messages. It:
    /// 1. Checks for stale sessions
    /// 2. Classifies the message
    /// 3. Returns guidance on what action to take
    ///
    /// # Arguments
    /// * `message` - The user's message
    /// * `repo` - Reference to the CTX repository
    ///
    /// # Returns
    /// A `SessionResponse` indicating what the agent should do.
    pub fn handle_message(&self, message: &str, repo: &CtxRepo) -> Result<SessionResponse> {
        // Check stale session status
        let stale_status = repo.check_stale_session(&self.stale_config);

        match stale_status {
            StaleSessionStatus::NoSession => {
                // No active session - any message starts a new one
                Ok(SessionResponse::StartNew {
                    user_message: message.to_string(),
                })
            }

            StaleSessionStatus::ShouldAutoCompact { task, idle_secs: _ } => {
                // Very stale (> 7 days default) - auto-compact without asking
                Ok(SessionResponse::AutoCompacted {
                    old_task: task,
                    user_message: message.to_string(),
                })
            }

            StaleSessionStatus::ShouldAsk { task, idle_secs } => {
                // Moderately stale (24h-7d default) - prompt user
                let idle_duration = Duration::from_secs(idle_secs);
                Ok(SessionResponse::Prompt {
                    message: format!(
                        "You have an unfinished task from {} ago: \"{}\". \
                         Would you like to continue it or start fresh?",
                        format_duration(idle_duration),
                        task
                    ),
                    pending: PendingAction::StaleSessionChoice {
                        user_message: message.to_string(),
                        old_task: task,
                        idle_duration,
                    },
                })
            }

            StaleSessionStatus::Fresh { task, idle_secs: _ } => {
                // Session is fresh - classify message and decide
                self.handle_fresh_session(message, &task, repo)
            }
        }
    }

    /// Handle a message when session is fresh (not stale).
    fn handle_fresh_session(
        &self,
        message: &str,
        task: &str,
        repo: &CtxRepo,
    ) -> Result<SessionResponse> {
        // Get current session state for classification context
        let current_state = repo
            .session()
            .map(|s| s.state().clone())
            .unwrap_or(SessionState::Running);

        let recent_question = match &current_state {
            SessionState::AwaitingUser { question, .. } => Some(question.clone()),
            _ => None,
        };

        let context = ClassificationContext {
            current_state,
            task_description: task.to_string(),
            recent_question,
        };

        let classification = classify_message(message, &context);

        // Check if this is a new task that needs confirmation
        if classification == MessageClassification::NewTask {
            return Ok(SessionResponse::Prompt {
                message: format!(
                    "You're currently working on: \"{}\". \
                     This looks like a new task. Save current work and switch?",
                    task
                ),
                pending: PendingAction::NewTaskChoice {
                    new_task: message.to_string(),
                    current_task: task.to_string(),
                },
            });
        }

        // For all other classifications, proceed normally
        Ok(SessionResponse::Proceed { classification })
    }

    /// Process user's response to a pending action.
    ///
    /// Call this after presenting a prompt to the user and receiving their choice.
    ///
    /// # Arguments
    /// * `choice` - The user's choice (Continue or StartFresh)
    /// * `pending` - The pending action that was presented
    /// * `repo` - Mutable reference to the CTX repository
    ///
    /// # Returns
    /// A `SessionResponse` indicating what to do next.
    pub fn handle_pending_response(
        &self,
        choice: UserChoice,
        pending: &PendingAction,
        repo: &mut CtxRepo,
    ) -> Result<SessionResponse> {
        match (choice, pending) {
            (UserChoice::Continue, PendingAction::StaleSessionChoice { .. }) => {
                // User wants to continue old session - classify their message
                Ok(SessionResponse::Proceed {
                    classification: MessageClassification::Modification,
                })
                // Note: The actual message handling is done by the orchestrator
                // We just indicate they should proceed with the message as a modification
            }

            (
                UserChoice::StartFresh,
                PendingAction::StaleSessionChoice {
                    user_message,
                    old_task,
                    ..
                },
            ) => {
                // User wants to start fresh - compact old session
                repo.compact_session(&format!("Saved: {} (user started new task)", old_task))?;

                Ok(SessionResponse::StartNew {
                    user_message: user_message.clone(),
                })
            }

            (UserChoice::Continue, PendingAction::NewTaskChoice { .. }) => {
                // User wants to continue current task - treat message as modification
                Ok(SessionResponse::Proceed {
                    classification: MessageClassification::Modification,
                })
            }

            (
                UserChoice::StartFresh,
                PendingAction::NewTaskChoice {
                    new_task,
                    current_task: _,
                },
            ) => {
                // User confirms new task - compact current and start new
                repo.compact_for_new_task(new_task)?;

                Ok(SessionResponse::StartNew {
                    user_message: new_task.clone(),
                })
            }
        }
    }
}

/// Formats a duration in human-readable form.
fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();

    if secs < 60 {
        format!("{} seconds", secs)
    } else if secs < 3600 {
        let mins = secs / 60;
        if mins == 1 {
            "1 minute".to_string()
        } else {
            format!("{} minutes", mins)
        }
    } else if secs < 86400 {
        let hours = secs / 3600;
        if hours == 1 {
            "1 hour".to_string()
        } else {
            format!("{} hours", hours)
        }
    } else {
        let days = secs / 86400;
        if days == 1 {
            "1 day".to_string()
        } else {
            format!("{} days", days)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_repo() -> (TempDir, CtxRepo) {
        let tmp = TempDir::new().unwrap();
        let repo = CtxRepo::init(tmp.path()).unwrap();
        (tmp, repo)
    }

    #[test]
    fn test_no_session_starts_new() {
        let (_tmp, repo) = setup_repo();
        let handler = SessionHandler::with_defaults();

        let response = handler.handle_message("Add retry logic", &repo).unwrap();

        assert!(matches!(response, SessionResponse::StartNew { .. }));
        if let SessionResponse::StartNew { user_message } = response {
            assert_eq!(user_message, "Add retry logic");
        }
    }

    #[test]
    fn test_fresh_session_proceeds() {
        let (_tmp, mut repo) = setup_repo();
        repo.start_session("Add retry logic").unwrap();

        let handler = SessionHandler::with_defaults();
        let response = handler.handle_message("Use exponential backoff", &repo).unwrap();

        assert!(matches!(response, SessionResponse::Proceed { .. }));
    }

    #[test]
    fn test_new_task_prompts() {
        let (_tmp, mut repo) = setup_repo();
        repo.start_session("Add retry logic").unwrap();

        let handler = SessionHandler::with_defaults();
        let response = handler
            .handle_message("New task: add connection pooling to the database layer", &repo)
            .unwrap();

        assert!(matches!(response, SessionResponse::Prompt { .. }));
        if let SessionResponse::Prompt { pending, .. } = response {
            assert!(matches!(pending, PendingAction::NewTaskChoice { .. }));
        }
    }

    #[test]
    fn test_abandon_proceeds() {
        let (_tmp, mut repo) = setup_repo();
        repo.start_session("Add retry logic").unwrap();

        let handler = SessionHandler::with_defaults();
        let response = handler.handle_message("cancel", &repo).unwrap();

        assert!(matches!(response, SessionResponse::Proceed { classification: MessageClassification::Abandon }));
    }

    #[test]
    fn test_stale_session_prompts() {
        let (_tmp, mut repo) = setup_repo();
        repo.start_session("Add retry logic").unwrap();

        // Use a very short threshold for testing
        let handler = SessionHandler::new(StaleSessionConfig {
            ask_threshold_secs: 0, // Immediately stale
            auto_compact_threshold_secs: 86400 * 30, // 30 days for auto
        });

        let response = handler.handle_message("hello", &repo).unwrap();

        assert!(matches!(response, SessionResponse::Prompt { .. }));
        if let SessionResponse::Prompt { pending, .. } = response {
            assert!(matches!(pending, PendingAction::StaleSessionChoice { .. }));
        }
    }

    #[test]
    fn test_very_stale_auto_compacts() {
        let (_tmp, mut repo) = setup_repo();
        repo.start_session("Add retry logic").unwrap();

        // Use thresholds that make session immediately very stale
        let handler = SessionHandler::new(StaleSessionConfig {
            ask_threshold_secs: 0,
            auto_compact_threshold_secs: 0, // Immediately auto-compact
        });

        let response = handler.handle_message("hello", &repo).unwrap();

        assert!(matches!(response, SessionResponse::AutoCompacted { .. }));
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(30)), "30 seconds");
        assert_eq!(format_duration(Duration::from_secs(60)), "1 minute");
        assert_eq!(format_duration(Duration::from_secs(120)), "2 minutes");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1 hour");
        assert_eq!(format_duration(Duration::from_secs(7200)), "2 hours");
        assert_eq!(format_duration(Duration::from_secs(86400)), "1 day");
        assert_eq!(format_duration(Duration::from_secs(172800)), "2 days");
    }

    #[test]
    fn test_pending_response_continue() {
        let (_tmp, mut repo) = setup_repo();
        repo.start_session("Add retry logic").unwrap();

        let handler = SessionHandler::with_defaults();
        let pending = PendingAction::StaleSessionChoice {
            user_message: "hello".to_string(),
            old_task: "Add retry logic".to_string(),
            idle_duration: Duration::from_secs(3600),
        };

        let response = handler
            .handle_pending_response(UserChoice::Continue, &pending, &mut repo)
            .unwrap();

        assert!(matches!(response, SessionResponse::Proceed { .. }));
    }

    #[test]
    fn test_pending_response_start_fresh() {
        let (_tmp, mut repo) = setup_repo();
        repo.start_session("Add retry logic").unwrap();
        // Flush so there's something to compact
        repo.flush_session().unwrap();

        let handler = SessionHandler::with_defaults();
        let pending = PendingAction::StaleSessionChoice {
            user_message: "New feature".to_string(),
            old_task: "Add retry logic".to_string(),
            idle_duration: Duration::from_secs(3600),
        };

        let response = handler
            .handle_pending_response(UserChoice::StartFresh, &pending, &mut repo)
            .unwrap();

        assert!(matches!(response, SessionResponse::StartNew { .. }));
        // Session should be compacted
        assert!(repo.session().is_none());
    }
}
