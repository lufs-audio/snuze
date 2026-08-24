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
    SignalAlarm { match_key: String },
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
    NotFound(String),
    Error(String),
}
