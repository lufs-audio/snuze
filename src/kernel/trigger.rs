use super::jules_state::{JulesSessionState, JulesState};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HerdrWatchStatus {
    Idle,
    Done,
    Blocked,
    Working,
}

#[must_use]
pub fn herdr_status_satisfied(watch_for: &[HerdrWatchStatus], current: HerdrWatchStatus) -> bool {
    if watch_for.is_empty() {
        return false;
    }
    watch_for.contains(&current)
}

/// Condition evaluated against tracked Jules session states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JulesCondition {
    AllTerminal,
    AnyTerminal,
    AnyCompleted,
}

fn is_wake_worthy(state: &JulesState) -> bool {
    matches!(
        state,
        JulesState::Completed
            | JulesState::Failed
            | JulesState::AwaitingUserFeedback
            | JulesState::Paused
    )
}

impl JulesCondition {
    /// Returns true if the condition is satisfied by the given session states.
    ///
    /// Always returns false for an empty slice (never vacuously true).
    #[must_use]
    pub fn is_satisfied(&self, states: &[JulesSessionState]) -> bool {
        if states.is_empty() {
            return false;
        }

        match self {
            Self::AllTerminal => states.iter().all(|s| is_wake_worthy(&s.state)),
            Self::AnyTerminal => states.iter().any(|s| is_wake_worthy(&s.state)),
            Self::AnyCompleted => states.iter().any(|s| s.state == JulesState::Completed),
        }
    }
}

/// Trigger mechanism defining when an alarm fires or re-polls.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    #[default]
    Deadline,
    JulesCondition {
        session_ids: Vec<String>,
        condition: JulesCondition,
        poll_interval_secs: u64,
        timeout_at: SystemTime,
        last_known: Vec<JulesSessionState>,
    },
    HerdrStatus {
        watched_pane_id: String,
        watch_for: Vec<HerdrWatchStatus>,
        timeout_at: SystemTime,
        last_known: Option<HerdrWatchStatus>,
    },
    ExternalSignal {
        match_key: String,
        timeout_at: SystemTime,
    },
}
