use crate::kernel::{HerdrWatchStatus, SnuzeError};
use crate::scheduler::{PushSignal, Scheduler};
use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Persistent push listener that subscribes to Herdr's event stream
/// and forwards pane agent status changes to the `Scheduler`.
pub struct HerdrEventListener {
    scheduler: Arc<Scheduler>,
    socket_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct HerdrEventEnvelope {
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    data: Option<HerdrEventData>,
    #[serde(default)]
    pane_id: Option<String>,
    #[serde(default)]
    agent_status: Option<String>,
    #[serde(default)]
    new_status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HerdrEventData {
    #[serde(default)]
    pane_id: Option<String>,
    #[serde(default)]
    agent_status: Option<String>,
    #[serde(default)]
    new_status: Option<String>,
}

impl HerdrEventListener {
    /// Creates a new `HerdrEventListener` with default socket path resolution.
    ///
    /// # Errors
    /// Returns `SnuzeError` if initialization fails.
    pub fn new(scheduler: Arc<Scheduler>) -> Result<Self, SnuzeError> {
        Ok(Self {
            scheduler,
            socket_path: None,
        })
    }

    /// Creates a new `HerdrEventListener` targeting a specific Unix domain socket path.
    #[must_use]
    pub fn with_socket_path(scheduler: Arc<Scheduler>, socket_path: PathBuf) -> Self {
        Self {
            scheduler,
            socket_path: Some(socket_path),
        }
    }

    /// Blocks the calling thread for the listener's lifetime, reconnecting
    /// with capped exponential backoff on any connection failure.
    pub fn run(&self) {
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);

        loop {
            let sock_path = self
                .socket_path
                .clone()
                .unwrap_or_else(resolve_herdr_socket_path);

            match self.connect_and_listen(&sock_path) {
                Ok(()) => {
                    eprintln!("Herdr subscription connection ended normally. Reconnecting...");
                    backoff = Duration::from_secs(1);
                }
                Err(e) => {
                    eprintln!(
                        "Herdr subscription disconnected or failed to connect to {}: {}. Retrying in {:?}...",
                        sock_path.display(),
                        e,
                        backoff
                    );
                }
            }

            std::thread::sleep(backoff);
            backoff = std::cmp::min(backoff.saturating_mul(2), max_backoff);
        }
    }

    /// Connects to the Herdr Unix socket, issues the `events.subscribe` request, and processes incoming events.
    fn connect_and_listen(&self, sock_path: &Path) -> Result<(), std::io::Error> {
        let mut stream = UnixStream::connect(sock_path)?;

        // Send events.subscribe request (empty subscriptions array subscribes to all events)
        let subscribe_req = serde_json::json!({
            "id": "snuze-sub",
            "method": "events.subscribe",
            "params": {
                "subscriptions": []
            }
        });

        let mut req_str = serde_json::to_string(&subscribe_req)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        req_str.push('\n');
        stream.write_all(req_str.as_bytes())?;
        stream.flush()?;

        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            self.handle_event_line(trimmed);
        }

        Ok(())
    }

    /// Parses and processes a single event line from Herdr.
    pub fn handle_event_line(&self, line: &str) {
        let Ok(envelope) = serde_json::from_str::<HerdrEventEnvelope>(line) else {
            return;
        };

        // If an event kind is explicitly specified, only handle agent status changes
        if let Some(ref ev) = envelope.event {
            if ev != "pane.agent_status_changed" {
                return;
            }
        }

        // Extract pane_id and status string
        let (pane_id, status_str) = if let Some(ref data) = envelope.data {
            let pid = data.pane_id.clone().or_else(|| envelope.pane_id.clone());
            let st = data
                .agent_status
                .clone()
                .or_else(|| data.new_status.clone())
                .or_else(|| envelope.agent_status.clone())
                .or_else(|| envelope.new_status.clone());
            (pid, st)
        } else {
            let pid = envelope.pane_id;
            let st = envelope.agent_status.or(envelope.new_status);
            (pid, st)
        };

        let (Some(pane_id), Some(status_str)) = (pane_id, status_str) else {
            return;
        };

        // Map status string to HerdrWatchStatus
        let new_status = match status_str.to_lowercase().as_str() {
            "idle" => HerdrWatchStatus::Idle,
            "done" => HerdrWatchStatus::Done,
            "blocked" => HerdrWatchStatus::Blocked,
            "working" => HerdrWatchStatus::Working,
            other => {
                eprintln!("Unrecognized Herdr status '{other}' for pane '{pane_id}', ignoring");
                return;
            }
        };

        // Forward to scheduler
        let _ = self.scheduler.signal(PushSignal::HerdrPaneEvent {
            pane_id,
            new_status,
        });
    }
}

/// Resolves the socket path for the Herdr daemon.
fn resolve_herdr_socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("HERDR_SOCKET_PATH") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }

    if let Ok(config_dir) = std::env::var("HERDR_CONFIG_DIR") {
        if !config_dir.is_empty() {
            return PathBuf::from(config_dir).join("herdr.sock");
        }
    }

    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("herdr").join("herdr.sock");
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home)
                .join(".config")
                .join("herdr")
                .join("herdr.sock");
        }
    }

    PathBuf::from("/tmp/herdr.sock")
}
