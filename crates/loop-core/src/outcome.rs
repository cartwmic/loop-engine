//! Semantic operation outcomes.

use serde_json::Value;

use crate::EvaluationFeedback;

/// The three semantic classes exposed by every core operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationStatus {
    Completed,
    Rejected,
    Error,
}

/// Actionable information associated with a rejected or errored operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutcomeIssue {
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
}

impl OutcomeIssue {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn from_feedback(feedback: EvaluationFeedback) -> Self {
        Self {
            code: feedback.code,
            message: feedback.message,
            details: feedback.details,
        }
    }
}

/// A semantic result rather than an infrastructure `Result`.
///
/// `Completed` carries the operation's successful value.  `Rejected` means
/// the request was understood but denied by lifecycle/workflow semantics;
/// `Error` means the operation could not be reliably evaluated or committed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationOutcome<T> {
    Completed(T),
    Rejected(OutcomeIssue),
    Error(OutcomeIssue),
}

impl<T> OperationOutcome<T> {
    pub fn completed(value: T) -> Self {
        Self::Completed(value)
    }

    pub fn rejected(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Rejected(OutcomeIssue::new(code, message))
    }

    pub fn rejected_with_issue(issue: OutcomeIssue) -> Self {
        Self::Rejected(issue)
    }

    pub fn rejected_feedback(feedback: EvaluationFeedback) -> Self {
        Self::Rejected(OutcomeIssue::from_feedback(feedback))
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Error(OutcomeIssue::new(code, message))
    }

    pub fn error_with_issue(issue: OutcomeIssue) -> Self {
        Self::Error(issue)
    }

    pub const fn status(&self) -> OperationStatus {
        match self {
            Self::Completed(_) => OperationStatus::Completed,
            Self::Rejected(_) => OperationStatus::Rejected,
            Self::Error(_) => OperationStatus::Error,
        }
    }

    pub const fn is_completed(&self) -> bool {
        matches!(self, Self::Completed(_))
    }

    pub const fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected(_))
    }

    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Completed(value) => Some(value),
            Self::Rejected(_) | Self::Error(_) => None,
        }
    }

    pub fn into_value(self) -> Option<T> {
        match self {
            Self::Completed(value) => Some(value),
            Self::Rejected(_) | Self::Error(_) => None,
        }
    }

    pub fn issue(&self) -> Option<&OutcomeIssue> {
        match self {
            Self::Completed(_) => None,
            Self::Rejected(issue) | Self::Error(issue) => Some(issue),
        }
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> OperationOutcome<U> {
        match self {
            Self::Completed(value) => OperationOutcome::Completed(map(value)),
            Self::Rejected(issue) => OperationOutcome::Rejected(issue),
            Self::Error(issue) => OperationOutcome::Error(issue),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn outcome_status_distinguishes_all_three_semantic_classes() {
        let completed = OperationOutcome::completed(42_u8);
        let rejected = OperationOutcome::<u8>::rejected("event-unavailable", "No event");
        let error = OperationOutcome::<u8>::error("provider-failed", "Provider exited");

        assert_eq!(completed.status(), OperationStatus::Completed);
        assert_eq!(rejected.status(), OperationStatus::Rejected);
        assert_eq!(error.status(), OperationStatus::Error);
        assert_eq!(completed.value(), Some(&42));
        assert!(rejected.issue().is_some());
        assert!(error.value().is_none());
    }

    #[test]
    fn denial_feedback_can_be_carried_without_losing_opaque_details() {
        let outcome = OperationOutcome::<()>::rejected_feedback(
            EvaluationFeedback::new("policy-failed", "Revise the document")
                .with_details(json!({"finding": ["heading"]})),
        );
        let issue = outcome.issue().unwrap();

        assert_eq!(issue.code, "policy-failed");
        assert_eq!(issue.message, "Revise the document");
        assert_eq!(issue.details, Some(json!({"finding": ["heading"]})));
    }
}
