use super::trigger::Trigger;
use serde::{Deserialize, Deserializer, Serialize};
use std::time::SystemTime;

const fn default_schema_version() -> u32 {
    1
}

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

/// Pluggable delivery destination for a scheduled alarm.
///
/// Serialized internally tagged on `type`, so the on-disk alarm format is
/// self-describing:
///
/// ```json
/// { "target": { "type": "tmux", "target": "%3" } }
/// ```
///
/// Before this enum existed, `Alarm` carried a bare `pane_id: String`. That
/// legacy shape is still accepted on read (see `Alarm`'s custom `Deserialize`),
/// which is what keeps alarm files written by an older `snuze` readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeliveryTarget {
    /// Deliver through the `herdr` CLI to a Herdr pane (e.g. `w1:p3`).
    Herdr { pane_id: String },
    /// Deliver through `tmux send-keys` to a tmux target (e.g. `%0`, `work:0.1`).
    Tmux { target: String },
    /// Run a shell command as the delivery action.
    Command { command: String },
    /// No external delivery; the alarm exists purely to be observed by `snuze wait`.
    Wait,
}

impl DeliveryTarget {
    /// Stable machine-readable kind name: `herdr`, `tmux`, `command`, or `wait`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Herdr { .. } => "herdr",
            Self::Tmux { .. } => "tmux",
            Self::Command { .. } => "command",
            Self::Wait => "wait",
        }
    }

    /// The destination string itself (`pane_id`, tmux target, or command body).
    ///
    /// For `Wait` this is the literal `"wait"`, since it has no external address.
    #[must_use]
    pub fn destination(&self) -> String {
        match self {
            Self::Herdr { pane_id } => pane_id.clone(),
            Self::Tmux { target } => target.clone(),
            Self::Command { command } => command.clone(),
            Self::Wait => "wait".to_string(),
        }
    }

    /// `kind:destination` rendering, as accepted by `--target`.
    #[must_use]
    pub fn spec(&self) -> String {
        match self {
            Self::Wait => "wait".to_string(),
            other => format!("{}:{}", other.kind(), other.destination()),
        }
    }
}

/// Durable alarm domain model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Alarm {
    /// Unique identifier (ULID or `UUIDv7`, lexicographically sortable by creation time).
    pub id: String,
    /// Where the wake-up payload is delivered.
    pub target: DeliveryTarget,
    /// Provenance: the calling pane's `$HERDR_PANE_ID` at schedule time, if any.
    /// Populated by the CLI layer, not here — this struct just owns the field and
    /// its serde round-trip.
    pub created_by_pane: Option<String>,
    /// Delivery payload (prompt or shell command).
    pub delivery: DeliveryMode,
    /// Optional human/agent label.
    pub label: Option<String>,
    /// Timestamp when the alarm was scheduled.
    pub created_at: SystemTime,
    /// Timestamp when the alarm should next be evaluated.
    ///
    /// For `Deadline` this is the fire time; for condition/poll triggers it is the
    /// next check time.
    pub deadline: SystemTime,
    /// Current lifecycle status.
    pub status: AlarmStatus,
    /// Timestamp when delivery was attempted, if any.
    pub fired_at: Option<SystemTime>,
    /// Reason string if delivery failed, missed, or was canceled.
    pub failure_reason: Option<String>,
    /// Trigger that fires this alarm.
    #[serde(default)]
    pub trigger: Trigger,
    /// Schema version for on-disk persistence compatibility.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
}

/// Wire form used only to read an `Alarm` back off disk.
///
/// It accepts **both** the current `target` object and the legacy pre-Unit-01
/// `pane_id` string, so alarm files written by an older `snuze` still load.
#[derive(Deserialize)]
struct AlarmWire {
    id: String,
    #[serde(default)]
    target: Option<DeliveryTarget>,
    /// Legacy (pre-`DeliveryTarget`) destination field.
    #[serde(default)]
    pane_id: Option<String>,
    #[serde(default)]
    created_by_pane: Option<String>,
    delivery: DeliveryMode,
    #[serde(default)]
    label: Option<String>,
    created_at: SystemTime,
    deadline: SystemTime,
    status: AlarmStatus,
    #[serde(default)]
    fired_at: Option<SystemTime>,
    #[serde(default)]
    failure_reason: Option<String>,
    #[serde(default)]
    trigger: Trigger,
    #[serde(default = "default_schema_version")]
    schema_version: u32,
}

impl<'de> Deserialize<'de> for Alarm {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AlarmWire::deserialize(deserializer)?;

        // Migration path: a legacy record has `pane_id` and no `target`. Upgrade it
        // in place to the Herdr variant rather than failing the whole alarm.
        let target = match (wire.target, wire.pane_id) {
            (Some(target), _) => target,
            (None, Some(pane_id)) => DeliveryTarget::Herdr { pane_id },
            (None, None) => return Err(serde::de::Error::missing_field("target")),
        };

        Ok(Self {
            id: wire.id,
            target,
            created_by_pane: wire.created_by_pane,
            delivery: wire.delivery,
            label: wire.label,
            created_at: wire.created_at,
            deadline: wire.deadline,
            status: wire.status,
            fired_at: wire.fired_at,
            failure_reason: wire.failure_reason,
            trigger: wire.trigger,
            schema_version: wire.schema_version,
        })
    }
}
