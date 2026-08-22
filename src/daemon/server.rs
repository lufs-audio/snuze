use crate::daemon::lock::DaemonLock;
use crate::daemon::protocol::{Request, Response};
use crate::herdr::RealHerdrAdapter;
use crate::kernel::{AlarmStatus, SnuzeError};
use crate::scheduler::{FiredEvent, Scheduler, SystemClock};
use crate::store::AlarmStore;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Background daemon service for managing alarm storage, scheduling, and IPC.
pub struct Daemon;

impl Daemon {
    /// Blocks the calling thread for the daemon's lifetime, serving IPC requests
    /// and running the alarm scheduler loop.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` or `SnuzeError::PersistFailed` if initialization fails.
    #[allow(clippy::too_many_lines)]
    pub fn run_foreground(state_dir: &Path) -> Result<(), SnuzeError> {
        let _lock = DaemonLock::acquire(state_dir)?;
        let store = Arc::new(AlarmStore::open(state_dir)?);

        let sock_path = state_dir.join("daemon.sock");
        if sock_path.exists() {
            let _ = std::fs::remove_file(&sock_path);
        }
        let listener = UnixListener::bind(&sock_path)
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("could not bind socket: {e}")))?;

        // Initialize Herdr adapter and clock
        let adapter = Box::new(RealHerdrAdapter::new()?);
        let clock = Box::new(SystemClock::default());

        let store_clone = Arc::clone(&store);
        let on_terminal = Box::new(move |event| {
            let id = match &event {
                FiredEvent::Fired { id, .. }
                | FiredEvent::Failed { id, .. }
                | FiredEvent::Missed { id } => id,
            };
            if let Ok(mut alarm) = store_clone.get(id) {
                match event {
                    FiredEvent::Fired { at, .. } => {
                        alarm.status = AlarmStatus::Fired;
                        alarm.fired_at = Some(at);
                    }
                    FiredEvent::Failed { reason, .. } => {
                        alarm.status = AlarmStatus::Failed;
                        alarm.failure_reason = Some(reason);
                    }
                    FiredEvent::Missed { .. } => {
                        alarm.status = AlarmStatus::Missed;
                        alarm.failure_reason =
                            Some("deadline exceeded grace window during downtime".to_string());
                    }
                }
                let _ = store_clone.put(&alarm);
            }
        });

        // 10-minute default grace window per SPEC §3.4
        let scheduler = Arc::new(Scheduler::new(
            adapter,
            clock,
            Duration::from_mins(10),
            on_terminal,
        ));

        // Rebuild in-memory heap from durable store on boot
        if let Ok(pending_alarms) = store.list(Some(AlarmStatus::Pending)) {
            for alarm in pending_alarms {
                scheduler.arm(alarm);
            }
        }

        // Spawn dedicated scheduler thread
        let scheduler_clone = Arc::clone(&scheduler);
        std::thread::spawn(move || {
            scheduler_clone.run();
        });

        // Main IPC server loop
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };

            let Ok(stream_clone) = stream.try_clone() else {
                continue;
            };

            let mut reader = BufReader::new(stream_clone);
            let mut line = String::new();

            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    line.clear();
                    continue;
                }

                let req: Request = match serde_json::from_str(trimmed) {
                    Ok(r) => r,
                    Err(e) => {
                        let resp = Response::Error(format!("malformed request: {e}"));
                        if let Ok(resp_json) = serde_json::to_string(&resp) {
                            let _ = writeln!(stream, "{resp_json}");
                        }
                        line.clear();
                        continue;
                    }
                };

                line.clear();

                match req {
                    Request::Shutdown => {
                        let resp = Response::Ok;
                        if let Ok(resp_json) = serde_json::to_string(&resp) {
                            let _ = writeln!(stream, "{resp_json}");
                        }
                        let _ = std::fs::remove_file(&sock_path);
                        return Ok(());
                    }
                    Request::Ping => {
                        let resp = Response::Ok;
                        if let Ok(resp_json) = serde_json::to_string(&resp) {
                            let _ = writeln!(stream, "{resp_json}");
                        }
                    }
                    Request::SetAlarm(alarm) => {
                        // 1. Durability before ACK: write to disk before responding Ok
                        if let Err(e) = store.put(&alarm) {
                            let resp = Response::Error(e.to_string());
                            if let Ok(resp_json) = serde_json::to_string(&resp) {
                                let _ = writeln!(stream, "{resp_json}");
                            }
                            continue;
                        }

                        // 2. Arm in scheduler
                        scheduler.arm(alarm);

                        // 3. Respond Ok to client
                        let resp = Response::Ok;
                        if let Ok(resp_json) = serde_json::to_string(&resp) {
                            let _ = writeln!(stream, "{resp_json}");
                        }
                    }
                    Request::ListAlarms(filter) => {
                        let resp = match store.list(filter) {
                            Ok(alarms) => Response::AlarmList(alarms),
                            Err(e) => Response::Error(e.to_string()),
                        };
                        if let Ok(resp_json) = serde_json::to_string(&resp) {
                            let _ = writeln!(stream, "{resp_json}");
                        }
                    }
                    Request::GetAlarm(id) => {
                        let resp = match store.get(&id) {
                            Ok(alarm) => Response::Alarm(alarm),
                            Err(SnuzeError::AlarmNotFound(id)) => Response::NotFound(id),
                            Err(e) => Response::Error(e.to_string()),
                        };
                        if let Ok(resp_json) = serde_json::to_string(&resp) {
                            let _ = writeln!(stream, "{resp_json}");
                        }
                    }
                    Request::CancelAlarm(id) => {
                        let removed_from_scheduler = scheduler.cancel(&id);
                        let mut found = removed_from_scheduler;

                        if let Ok(mut alarm) = store.get(&id) {
                            if alarm.status == AlarmStatus::Pending {
                                alarm.status = AlarmStatus::Canceled;
                                let _ = store.put(&alarm);
                                found = true;
                            }
                        }

                        let resp = Response::CancelResult(found);
                        if let Ok(resp_json) = serde_json::to_string(&resp) {
                            let _ = writeln!(stream, "{resp_json}");
                        }
                    }
                }
            }
        }

        let _ = std::fs::remove_file(sock_path);
        Ok(())
    }
}
