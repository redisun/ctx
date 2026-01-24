//! User message classification for session management.
//!
//! This module provides heuristic-based classification of user messages
//! to determine appropriate session actions (continue, confirm, abandon, etc.).

use crate::types::SessionState;

/// Classification of a user message for session management.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageClassification {
    /// Direct answer to agent's question.
    /// Example: "Yes", "Option B", "Use exponential backoff"
    Response,

    /// Change request for current work.
    /// Example: "Make it 5 not 3", "Also add logging"
    Modification,

    /// Approval of completed work.
    /// Example: "Looks good", "Perfect", "Ship it", "LGTM"
    Confirmation,

    /// Request to stop current task.
    /// Example: "Never mind", "Cancel", "Stop"
    Abandon,

    /// Unrelated new request (different task).
    /// Example: "Now add pooling", "Different topic..."
    NewTask,

    /// Request for more information (no state change needed).
    /// Example: "What does that mean?", "Show me the code"
    Clarification,
}

/// Context needed for message classification.
#[derive(Debug, Clone)]
pub struct ClassificationContext {
    /// Current session state.
    pub current_state: SessionState,
    /// Description of the current task.
    pub task_description: String,
    /// The question asked by the agent (if in AwaitingUser state).
    pub recent_question: Option<String>,
}

/// Classifies a user message based on context and content.
///
/// The classification considers:
/// 1. Current session state (a "yes" means different things in different states)
/// 2. Keywords and patterns (abandon signals, confirmation phrases)
/// 3. Semantic overlap with current task (for NewTask detection)
///
/// When uncertain, defaults to `Modification` (conservative choice).
pub fn classify_message(message: &str, context: &ClassificationContext) -> MessageClassification {
    let message_lower = message.to_lowercase();
    let message_trimmed = message.trim();

    // Check for explicit abandon signals first (highest priority)
    if is_abandonment(&message_lower) {
        return MessageClassification::Abandon;
    }

    // State-aware classification
    match &context.current_state {
        SessionState::AwaitingUser { .. } => {
            // In AwaitingUser, short affirmative responses are likely Response
            if is_short_affirmative(message_trimmed) {
                return MessageClassification::Response;
            }
            // Any substantive reply to a question is a Response
            if !message_trimmed.is_empty() && message_trimmed.len() < 500 {
                // Could be either Response or NewTask - check for new task signals
                if is_new_task_signal(&message_lower, &context.task_description) {
                    return MessageClassification::NewTask;
                }
                return MessageClassification::Response;
            }
        }

        SessionState::PendingComplete { .. } => {
            // In PendingComplete, check for confirmation or modification
            if is_confirmation(&message_lower) {
                return MessageClassification::Confirmation;
            }
            if requests_changes(&message_lower) {
                return MessageClassification::Modification;
            }
            // Short affirmative in completion context is confirmation
            if is_short_affirmative(message_trimmed) {
                return MessageClassification::Confirmation;
            }
        }

        SessionState::Running | SessionState::Interrupted { .. } => {
            // Check if this looks like a new unrelated task
            if is_new_task_signal(&message_lower, &context.task_description) {
                return MessageClassification::NewTask;
            }
        }

        SessionState::Complete | SessionState::Aborted { .. } => {
            // Session is done, any message is effectively a new task
            return MessageClassification::NewTask;
        }
    }

    // Check for clarification requests
    if is_clarification_request(&message_lower) {
        return MessageClassification::Clarification;
    }

    // Default to Modification (conservative - assumes user wants to adjust current work)
    MessageClassification::Modification
}

/// Checks if the message indicates abandonment.
fn is_abandonment(message: &str) -> bool {
    let abandon_phrases = [
        "cancel",
        "stop",
        "never mind",
        "nevermind",
        "forget it",
        "forget about it",
        "don't bother",
        "abort",
        "quit",
        "drop it",
        "skip it",
        "let's not",
        "actually no",
        "actually, no",
    ];

    for phrase in &abandon_phrases {
        if message.contains(phrase) {
            return true;
        }
    }

    // Check for standalone "no" at the start
    if message.trim() == "no" || message.starts_with("no,") || message.starts_with("no.") {
        return true;
    }

    false
}

/// Checks if message is a short affirmative response.
fn is_short_affirmative(message: &str) -> bool {
    let affirmatives = [
        "yes",
        "yeah",
        "yep",
        "yup",
        "sure",
        "ok",
        "okay",
        "k",
        "y",
        "correct",
        "right",
        "exactly",
        "that's right",
        "that's correct",
        "sounds good",
        "go ahead",
        "do it",
        "proceed",
        "continue",
    ];

    let lower = message.to_lowercase();
    let trimmed = lower.trim();

    // Must be short (< 50 chars) to be considered a simple affirmative
    if trimmed.len() > 50 {
        return false;
    }

    for aff in &affirmatives {
        if trimmed == *aff || trimmed.starts_with(&format!("{},", aff)) {
            return true;
        }
    }

    false
}

/// Checks if message confirms completion.
fn is_confirmation(message: &str) -> bool {
    let confirmation_phrases = [
        "looks good",
        "look good",
        "lgtm",
        "perfect",
        "great",
        "awesome",
        "ship it",
        "merge it",
        "done",
        "good to go",
        "approved",
        "approve",
        "accept",
        "nice",
        "excellent",
        "that works",
        "that's perfect",
        "that's great",
        "well done",
        "good job",
        "thanks",
        "thank you",
    ];

    for phrase in &confirmation_phrases {
        if message.contains(phrase) {
            return true;
        }
    }

    false
}

/// Checks if message requests changes/modifications.
fn requests_changes(message: &str) -> bool {
    let change_signals = [
        "also",
        "but",
        "however",
        "instead",
        "change",
        "modify",
        "update",
        "fix",
        "add",
        "remove",
        "can you",
        "could you",
        "please",
        "actually",
        "wait",
        "one more",
        "another",
    ];

    for signal in &change_signals {
        if message.contains(signal) {
            return true;
        }
    }

    false
}

/// Checks if message signals a new unrelated task.
fn is_new_task_signal(message: &str, task_description: &str) -> bool {
    // Explicit new task markers
    let new_task_markers = [
        "new task",
        "different task",
        "something else",
        "unrelated",
        "change of topic",
        "switching gears",
        "now let's",
        "next task",
        "moving on",
    ];

    for marker in &new_task_markers {
        if message.contains(marker) {
            return true;
        }
    }

    // Check for low semantic overlap with current task
    // This is a simple heuristic - count shared significant words
    let task_lower = task_description.to_lowercase();
    let task_words: std::collections::HashSet<&str> = task_lower
        .split_whitespace()
        .filter(|w| w.len() > 3) // Skip short words
        .collect();

    let message_words: std::collections::HashSet<&str> = message
        .split_whitespace()
        .filter(|w| w.len() > 3)
        .collect();

    // If task has meaningful words but message shares none, might be new task
    if task_words.len() >= 3 && message_words.len() >= 5 {
        let overlap = task_words.intersection(&message_words).count();
        if overlap == 0 {
            // No overlap and message is substantial - likely new task
            // But only if message is long enough to be a task description
            if message.len() > 30 {
                return true;
            }
        }
    }

    false
}

/// Checks if message is a clarification request.
fn is_clarification_request(message: &str) -> bool {
    let clarification_patterns = [
        "what do you mean",
        "what does that mean",
        "can you explain",
        "could you explain",
        "show me",
        "where is",
        "how does",
        "what is",
        "why",
        "?",
    ];

    for pattern in &clarification_patterns {
        if message.contains(pattern) {
            // For "?" pattern, only consider it a clarification if message is short
            // (otherwise it might be a task disguised as a question)
            if *pattern == "?" {
                if message.len() < 100 {
                    return true;
                }
            } else {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_awaiting_user(question: &str, task: &str) -> ClassificationContext {
        ClassificationContext {
            current_state: SessionState::AwaitingUser {
                question: question.to_string(),
                asked_at: 12345,
            },
            task_description: task.to_string(),
            recent_question: Some(question.to_string()),
        }
    }

    fn context_pending_complete(summary: &str, task: &str) -> ClassificationContext {
        ClassificationContext {
            current_state: SessionState::PendingComplete {
                summary: summary.to_string(),
            },
            task_description: task.to_string(),
            recent_question: None,
        }
    }

    fn context_running(task: &str) -> ClassificationContext {
        ClassificationContext {
            current_state: SessionState::Running,
            task_description: task.to_string(),
            recent_question: None,
        }
    }

    #[test]
    fn test_response_in_awaiting_user() {
        let ctx = context_awaiting_user("Should I use exponential backoff?", "Add retry logic");

        assert_eq!(
            classify_message("yes", &ctx),
            MessageClassification::Response
        );
        assert_eq!(
            classify_message("Yeah, sounds good", &ctx),
            MessageClassification::Response
        );
        assert_eq!(
            classify_message("Use exponential backoff with jitter", &ctx),
            MessageClassification::Response
        );
    }

    #[test]
    fn test_confirmation_in_pending_complete() {
        let ctx = context_pending_complete("Added retry logic with exponential backoff", "Add retry logic");

        assert_eq!(
            classify_message("looks good", &ctx),
            MessageClassification::Confirmation
        );
        assert_eq!(
            classify_message("LGTM", &ctx),
            MessageClassification::Confirmation
        );
        assert_eq!(
            classify_message("Perfect, ship it!", &ctx),
            MessageClassification::Confirmation
        );
        assert_eq!(
            classify_message("yes", &ctx),
            MessageClassification::Confirmation
        );
    }

    #[test]
    fn test_modification_in_pending_complete() {
        let ctx = context_pending_complete("Added retry logic", "Add retry logic");

        assert_eq!(
            classify_message("Also add logging", &ctx),
            MessageClassification::Modification
        );
        assert_eq!(
            classify_message("Can you make it 5 retries instead?", &ctx),
            MessageClassification::Modification
        );
    }

    #[test]
    fn test_abandon_keywords() {
        let ctx = context_running("Add retry logic");

        assert_eq!(
            classify_message("cancel", &ctx),
            MessageClassification::Abandon
        );
        assert_eq!(
            classify_message("stop", &ctx),
            MessageClassification::Abandon
        );
        assert_eq!(
            classify_message("never mind", &ctx),
            MessageClassification::Abandon
        );
        assert_eq!(
            classify_message("Actually, forget it", &ctx),
            MessageClassification::Abandon
        );
    }

    #[test]
    fn test_new_task_explicit() {
        let ctx = context_running("Add retry logic to the HTTP client");

        assert_eq!(
            classify_message("New task: add connection pooling", &ctx),
            MessageClassification::NewTask
        );
        assert_eq!(
            classify_message("Let's do something else instead", &ctx),
            MessageClassification::NewTask
        );
    }

    #[test]
    fn test_clarification_request() {
        let ctx = context_running("Add retry logic");

        assert_eq!(
            classify_message("What do you mean by exponential?", &ctx),
            MessageClassification::Clarification
        );
        assert_eq!(
            classify_message("Can you explain that?", &ctx),
            MessageClassification::Clarification
        );
    }

    #[test]
    fn test_default_modification() {
        let ctx = context_running("Add retry logic");

        // Ambiguous messages should default to Modification
        assert_eq!(
            classify_message("Make it better", &ctx),
            MessageClassification::Modification
        );
        assert_eq!(
            classify_message("Use 3 seconds timeout", &ctx),
            MessageClassification::Modification
        );
    }

    #[test]
    fn test_completed_state_treats_as_new_task() {
        let ctx = ClassificationContext {
            current_state: SessionState::Complete,
            task_description: "Old task".to_string(),
            recent_question: None,
        };

        // Any message after completion is a new task
        assert_eq!(
            classify_message("Add something", &ctx),
            MessageClassification::NewTask
        );
    }

    #[test]
    fn test_abandon_overrides_other_signals() {
        // Abandon keywords should take priority even if other signals present
        let ctx = context_pending_complete("Done with task", "Add retry logic");

        // "cancel" should be Abandon, not Confirmation
        assert_eq!(
            classify_message("cancel this", &ctx),
            MessageClassification::Abandon
        );
    }
}
