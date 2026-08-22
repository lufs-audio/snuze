use crate::kernel::{Alarm, AlarmStatus};
use serde::{Deserialize, Serialize};

/// Internal IPC request format between `snuze` CLI client and background daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    SetAlarm(Alarm),
    ListAlarms(Option<AlarmStatus>),
    GetAlarm(String),
    CancelAlarm(String),
    Shutdown,
    Ping,
}

/// Internal IPC response format from background daemon to `snuze` CLI client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Ok,
    Alarm(Alarm),
    AlarmList(Vec<Alarm>),
    CancelResult(bool),
    NotFound(String),
    Error(String),
}
