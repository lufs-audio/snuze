use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// State of a Google Jules session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JulesState {
    Queued,
    Planning,
    AwaitingPlanApproval,
    InProgress,
    Completed,
    AwaitingUserFeedback,
    Paused,
    Failed,
    /// Fallback for unrecognized state strings in the alpha API.
    Unknown(String),
}

impl Serialize for JulesState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Queued => serializer.serialize_str("Queued"),
            Self::Planning => serializer.serialize_str("Planning"),
            Self::AwaitingPlanApproval => serializer.serialize_str("AwaitingPlanApproval"),
            Self::InProgress => serializer.serialize_str("InProgress"),
            Self::Completed => serializer.serialize_str("Completed"),
            Self::AwaitingUserFeedback => serializer.serialize_str("AwaitingUserFeedback"),
            Self::Paused => serializer.serialize_str("Paused"),
            Self::Failed => serializer.serialize_str("Failed"),
            Self::Unknown(s) => serializer.serialize_str(s),
        }
    }
}

struct JulesStateVisitor;

impl serde::de::Visitor<'_> for JulesStateVisitor {
    type Value = JulesState;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string representing a Jules session state")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(match value {
            "Queued" | "QUEUED" | "queued" => JulesState::Queued,
            "Planning" | "PLANNING" | "planning" => JulesState::Planning,
            "AwaitingPlanApproval" | "AWAITING_PLAN_APPROVAL" | "awaiting_plan_approval" => {
                JulesState::AwaitingPlanApproval
            }
            "InProgress" | "IN_PROGRESS" | "in_progress" => JulesState::InProgress,
            "Completed" | "COMPLETED" | "completed" => JulesState::Completed,
            "AwaitingUserFeedback" | "AWAITING_USER_FEEDBACK" | "awaiting_user_feedback" => {
                JulesState::AwaitingUserFeedback
            }
            "Paused" | "PAUSED" | "paused" => JulesState::Paused,
            "Failed" | "FAILED" | "failed" => JulesState::Failed,
            other => JulesState::Unknown(other.to_owned()),
        })
    }
}

impl<'de> Deserialize<'de> for JulesState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(JulesStateVisitor)
    }
}

/// Observed state of a Jules session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JulesSessionState {
    pub session_id: String,
    pub state: JulesState,
    pub pull_request_url: Option<String>,
}
