use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::kernel::{Alarm, AlarmStatus, SnuzeError};
use serde::{Deserialize, Serialize};

fn get_system_hostname() -> String {
    let mut buf = [0u8; 256];
    let res = unsafe { libc::gethostname(buf.as_mut_ptr().cast::<libc::c_char>(), buf.len()) };
    if res == 0 {
        if let Some(pos) = buf.iter().position(|&x| x == 0) {
            String::from_utf8_lossy(buf.get(..pos).unwrap_or(&buf)).into_owned()
        } else {
            String::from_utf8_lossy(&buf).into_owned()
        }
    } else {
        "unknown".to_string()
    }
}

/// Schema-versioned representation of a terminal alarm event persisted to the local outbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxEvent {
    pub schema_version: u32,
    pub emitted_at: SystemTime,
    pub host: String,
    pub tool: String,
    pub alarm_id: String,
    pub trigger_kind: String,
    pub status: String,
    pub created_at: SystemTime,
    pub created_by_pane: Option<String>,
    pub label: Option<String>,
    pub failure_reason: Option<String>,
    /// Destination string for the alarm's `DeliveryTarget` (`pane_id` for Herdr,
    /// the tmux target, or the command body). The field name is retained for
    /// backwards compatibility with the Phase 3 ledger schema; it is no longer
    /// necessarily a pane id.
    pub delivery_target_pane: String,
    pub artifact_refs: Vec<String>,
}

impl OutboxEvent {
    /// Constructs an event from a terminal alarm. `status` must be one of
    /// `Fired`/`Failed`/`Missed`/`Canceled` -- callers are responsible for never
    /// calling this for a `Pending` alarm or a `Progress` tick.
    #[must_use]
    pub fn from_terminal_alarm(alarm: &Alarm, status: AlarmStatus) -> Self {
        let trigger_kind = serde_json::to_value(&alarm.trigger)
            .ok()
            .and_then(|v| match v {
                serde_json::Value::Object(map) => {
                    map.get("kind").and_then(|k| k.as_str().map(String::from))
                }
                serde_json::Value::String(s) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| "unknown".to_string());

        let status_str = serde_json::to_value(status)
            .ok()
            .and_then(|v| match v {
                serde_json::Value::String(s) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| "unknown".to_string());

        let host = get_system_hostname();

        Self {
            schema_version: 1,
            emitted_at: SystemTime::now(),
            host,
            tool: "snuze".to_string(),
            alarm_id: alarm.id.clone(),
            trigger_kind,
            status: status_str,
            created_at: alarm.created_at,
            created_by_pane: alarm.created_by_pane.clone(),
            label: alarm.label.clone(),
            failure_reason: alarm.failure_reason.clone(),
            delivery_target_pane: alarm.target.destination(),
            artifact_refs: vec![],
        }
    }
}

/// Durable, append-only outbox for local alarm terminal events (`events.ndjson`).
pub struct EventOutbox {
    path: PathBuf,
    lock: std::sync::Mutex<()>,
}

impl EventOutbox {
    /// Opens `<state_dir>/events.ndjson`. Creates the file if absent; does not fail or truncate if it already exists and has content.
    ///
    /// # Errors
    /// Returns `SnuzeError::PersistFailed` on I/O error.
    pub fn open(state_dir: &Path) -> Result<Self, SnuzeError> {
        let path = state_dir.join("events.ndjson");
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    SnuzeError::PersistFailed(format!("failed to create state dir: {e}"))
                })?;
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| {
                    SnuzeError::PersistFailed(format!("failed to create outbox file: {e}"))
                })?;
        }
        Ok(Self {
            path,
            lock: std::sync::Mutex::new(()),
        })
    }

    /// Serializes `event` to one JSON line and appends it to the file with an `fsync`.
    ///
    /// # Errors
    /// Returns `SnuzeError::PersistFailed` on I/O or serialization error.
    pub fn append(&self, event: &OutboxEvent) -> Result<(), SnuzeError> {
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| {
                SnuzeError::PersistFailed(format!("failed to open outbox file for append: {e}"))
            })?;

        let json = serde_json::to_string(event).map_err(|e| {
            SnuzeError::PersistFailed(format!("failed to serialize outbox event: {e}"))
        })?;

        writeln!(file, "{json}")
            .map_err(|e| SnuzeError::PersistFailed(format!("failed to append event: {e}")))?;

        file.sync_all()
            .map_err(|e| SnuzeError::PersistFailed(format!("failed to fsync outbox: {e}")))?;

        Ok(())
    }

    /// Atomically swaps the current outbox for a fresh empty one and returns every event parsed from the old file.
    /// Never errors on a missing/empty file -- returns `Ok(vec![])`.
    ///
    /// # Errors
    /// Returns `SnuzeError::PersistFailed` on atomic swap I/O failure.
    pub fn drain(&self) -> Result<Vec<OutboxEvent>, SnuzeError> {
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let old_content = match std::fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(SnuzeError::PersistFailed(format!(
                    "failed to read outbox: {e}"
                )))
            }
        };

        let temp_name = format!("events.ndjson.tmp.{}", uuid::Uuid::now_v7());
        let temp_path = self.path.with_file_name(temp_name);

        let temp_file = std::fs::File::create(&temp_path)
            .map_err(|e| SnuzeError::PersistFailed(format!("failed to create temp outbox: {e}")))?;
        temp_file
            .sync_all()
            .map_err(|e| SnuzeError::PersistFailed(format!("failed to fsync temp outbox: {e}")))?;

        std::fs::rename(&temp_path, &self.path)
            .map_err(|e| SnuzeError::PersistFailed(format!("failed to atomic swap outbox: {e}")))?;

        if let Some(parent) = self.path.parent() {
            if let Ok(dir) = std::fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }

        let mut events = Vec::new();
        for line in old_content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<OutboxEvent>(line) {
                events.push(event);
            }
        }

        Ok(events)
    }
}
