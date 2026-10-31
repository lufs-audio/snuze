use crate::daemon::outbox::OutboxEvent;
use crate::kernel::{Alarm, AlarmStatus};
use serde::{Deserialize, Serialize};

/// Internal IPC request format between `snuze` CLI client and background daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum Request {
    SetAlarm(Alarm),
    ListAlarms(Option<AlarmStatus>),
    GetAlarm(String),
    CancelAlarm(String),
    SignalAlarm {
        match_key: String,
    },
    /// Block until `alarm_id` reaches a terminal state (or `timeout_secs` elapses).
    WaitAlarm {
        alarm_id: String,
        timeout_secs: Option<u64>,
    },
    DrainEvents,
    Shutdown,
    Ping,
}

/// Internal IPC response format from background daemon to `snuze` CLI client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum Response {
    Ok,
    Alarm(Alarm),
    AlarmList(Vec<Alarm>),
    CancelResult(bool),
    SignalResult(Vec<String>),
    /// The awaited alarm reached `Fired`.
    AlarmFired {
        /// Alarm identifier that fired.
        alarm_id: String,
    },
    NotFound(String),
    Error(String),
    EventsDrained(Vec<OutboxEvent>),
}
