use crate::daemon::outbox::OutboxEvent;
use crate::daemon::protocol::{Request, Response};
use crate::kernel::{Alarm, AlarmStatus, SnuzeError};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// The client-side handle Unit 06's CLI uses to talk to a running daemon.
pub struct DaemonClient {
    sock_path: PathBuf,
}

impl DaemonClient {
    /// Connects to the daemon socket at `<state_dir>/daemon.sock`.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` if the socket file does not exist or connection fails.
    pub fn connect(state_dir: &Path) -> Result<Self, SnuzeError> {
        let sock_path = state_dir.join("daemon.sock");
        if !sock_path.exists() {
            return Err(SnuzeError::DaemonUnavailable(
                "daemon socket not found".to_string(),
            ));
        }

        // Test connectivity
        if UnixStream::connect(&sock_path).is_err() {
            return Err(SnuzeError::DaemonUnavailable(
                "daemon socket connection refused".to_string(),
            ));
        }

        Ok(Self { sock_path })
    }

    /// Sends an IPC request to the daemon and parses the single-line JSON response.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` on I/O or protocol serialization errors.
    pub fn send_request(&self, req: &Request) -> Result<Response, SnuzeError> {
        let mut stream = UnixStream::connect(&self.sock_path)
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("could not connect to socket: {e}")))?;

        let mut json = serde_json::to_string(req)
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("request serialization failed: {e}")))?;
        json.push('\n');

        stream
            .write_all(json.as_bytes())
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("write to socket failed: {e}")))?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("read from socket failed: {e}")))?;

        let resp: Response = serde_json::from_str(&line)
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("response deserialization failed: {e}")))?;

        Ok(resp)
    }

    /// Submits an alarm to the daemon for durable persistence and scheduling.
    ///
    /// # Errors
    /// Returns `SnuzeError::PersistFailed` if the daemon could not persist the alarm,
    /// or `SnuzeError::DaemonUnavailable` if IPC communication fails.
    pub fn set_alarm(&self, alarm: Alarm) -> Result<(), SnuzeError> {
        match self.send_request(&Request::SetAlarm(alarm))? {
            Response::Ok => Ok(()),
            Response::Error(msg) => Err(SnuzeError::PersistFailed(msg)),
            _ => Err(SnuzeError::DaemonUnavailable(
                "unexpected daemon response for set_alarm".to_string(),
            )),
        }
    }

    /// Lists alarms matching the optional `status_filter`.
    ///
    /// # Errors
    /// Returns `SnuzeError::PersistFailed` or `SnuzeError::DaemonUnavailable`.
    pub fn list_alarms(&self, status_filter: Option<AlarmStatus>) -> Result<Vec<Alarm>, SnuzeError> {
        match self.send_request(&Request::ListAlarms(status_filter))? {
            Response::AlarmList(alarms) => Ok(alarms),
            Response::Error(msg) => Err(SnuzeError::PersistFailed(msg)),
            _ => Err(SnuzeError::DaemonUnavailable(
                "unexpected daemon response for list_alarms".to_string(),
            )),
        }
    }

    /// Retrieves an alarm by its unique identifier.
    ///
    /// # Errors
    /// Returns `SnuzeError::AlarmNotFound` if absent, or `SnuzeError::DaemonUnavailable`.
    pub fn get_alarm(&self, id: &str) -> Result<Alarm, SnuzeError> {
        match self.send_request(&Request::GetAlarm(id.to_string()))? {
            Response::Alarm(alarm) => Ok(alarm),
            Response::NotFound(id) => Err(SnuzeError::AlarmNotFound(id)),
            Response::Error(msg) => Err(SnuzeError::PersistFailed(msg)),
            _ => Err(SnuzeError::DaemonUnavailable(
                "unexpected daemon response for get_alarm".to_string(),
            )),
        }
    }

    /// Cancels a pending alarm by ID. Returns `true` if it was pending and canceled.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` on IPC error.
    pub fn cancel_alarm(&self, id: &str) -> Result<bool, SnuzeError> {
        match self.send_request(&Request::CancelAlarm(id.to_string()))? {
            Response::CancelResult(found) => Ok(found),
            Response::Error(msg) => Err(SnuzeError::PersistFailed(msg)),
            _ => Err(SnuzeError::DaemonUnavailable(
                "unexpected daemon response for cancel_alarm".to_string(),
            )),
        }
    }

    /// Health check ping to verify daemon responsiveness.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` if ping fails.
    pub fn ping(&self) -> Result<(), SnuzeError> {
        match self.send_request(&Request::Ping)? {
            Response::Ok => Ok(()),
            _ => Err(SnuzeError::DaemonUnavailable(
                "unexpected daemon response for ping".to_string(),
            )),
        }
    }

    /// Drains terminal events from the local outbox via the daemon.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` or `PersistFailed` on error.
    pub fn drain_events(&self) -> Result<Vec<OutboxEvent>, SnuzeError> {
        match self.send_request(&Request::DrainEvents)? {
            Response::EventsDrained(events) => Ok(events),
            Response::Error(msg) => Err(SnuzeError::PersistFailed(msg)),
            _ => Err(SnuzeError::DaemonUnavailable(
                "unexpected daemon response for drain_events".to_string(),
            )),
        }
    }
}
