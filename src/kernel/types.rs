use std::time::SystemTime;

/// Status lifecycle for an alarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlarmStatus {
    Pending,
    Fired,
    Failed,
    Missed,
    Canceled,
}

/// Mutually exclusive delivery payload for an alarm wake-up.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum DeliveryMode {
    Prompt { text: String },
    Command { text: String },
}

/// Durable alarm domain model.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Alarm {
    /// Unique identifier (ULID or `UUIDv7`, lexicographically sortable by creation time).
    pub id: String,
    /// Herdr pane identifier (e.g. `w1:p3`).
    pub pane_id: String,
    /// Provenance: the calling pane's `$HERDR_PANE_ID` at schedule time, if any.
    /// Populated by the CLI layer (Unit 06), not here — this struct just owns
    /// the field and its serde round-trip.
    pub created_by_pane: Option<String>,
    /// Delivery payload (prompt or shell command).
    pub delivery: DeliveryMode,
    /// Optional human/agent label.
    pub label: Option<String>,
    /// Timestamp when the alarm was scheduled.
    pub created_at: SystemTime,
    /// Timestamp when the alarm should fire.
    pub deadline: SystemTime,
    /// Current lifecycle status.
    pub status: AlarmStatus,
    /// Timestamp when delivery was attempted, if any.
    pub fired_at: Option<SystemTime>,
    /// Reason string if delivery failed, missed, or was canceled.
    pub failure_reason: Option<String>,
}
