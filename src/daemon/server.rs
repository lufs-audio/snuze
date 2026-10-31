use crate::daemon::lock::DaemonLock;
use crate::daemon::outbox::{EventOutbox, OutboxEvent};
use crate::daemon::protocol::{Request, Response};
use crate::herdr::RealHerdrAdapter;
use crate::kernel::{AlarmStatus, SnuzeError};
use crate::scheduler::{DeliveryAdapter, FiredEvent, Scheduler, SystemClock, TargetDispatcher};
use crate::store::AlarmStore;
use crate::tmux::TmuxAdapter;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Safety-net wake-up for blocked `snuze wait` connections.
///
/// Terminal transitions notify this condvar directly; the short timeout only
/// guards against a notification racing a waiter's arrival, so a wait never
/// depends on a client-side polling loop.
const WAIT_RECHECK_INTERVAL: Duration = Duration::from_millis(250);

/// Wakes connections blocked in `snuze wait` when an alarm becomes terminal.
#[derive(Default)]
struct TerminalNotifier {
    inner: Mutex<()>,
    cvar: Condvar,
}

impl TerminalNotifier {
    fn notify(&self) {
        let _guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.cvar.notify_all();
    }

    /// Blocks for at most `max_wait`, returning early on a notification.
    fn wait(&self, max_wait: Duration) {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = self.cvar.wait_timeout(guard, max_wait);
    }
}

/// State shared by the accept loop and every per-connection thread.
struct DaemonContext {
    store: Arc<AlarmStore>,
    outbox: Arc<EventOutbox>,
    scheduler: Arc<Scheduler>,
    notifier: Arc<TerminalNotifier>,
    shutdown: Arc<AtomicBool>,
    sock_path: PathBuf,
}

/// Lowercase status name for human-facing IPC error messages.
const fn status_name(status: AlarmStatus) -> &'static str {
    match status {
        AlarmStatus::Pending => "pending",
        AlarmStatus::Fired => "fired",
        AlarmStatus::Failed => "failed",
        AlarmStatus::Missed => "missed",
        AlarmStatus::Canceled => "canceled",
    }
}

fn respond(stream: &mut UnixStream, resp: &Response) {
    if let Ok(json) = serde_json::to_string(resp) {
        let _ = writeln!(stream, "{json}");
    }
}

/// Background daemon service for managing alarm storage, scheduling, and IPC.
pub struct Daemon;

impl Daemon {
    /// Blocks the calling thread for the daemon's lifetime, serving IPC requests
    /// and running the alarm scheduler loop.
    ///
    /// Each accepted connection is served on its own thread, so a blocking
    /// `Request::WaitAlarm` never stalls other clients.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` or `SnuzeError::PersistFailed` if initialization fails.
    #[allow(clippy::too_many_lines)]
    pub fn run_foreground(state_dir: &Path) -> Result<(), SnuzeError> {
        let _lock = DaemonLock::acquire(state_dir)?;
        let store = Arc::new(AlarmStore::open(state_dir)?);
        let outbox = Arc::new(EventOutbox::open(state_dir)?);

        let sock_path = state_dir.join("daemon.sock");
        if sock_path.exists() {
            let _ = std::fs::remove_file(&sock_path);
        }
        let listener = UnixListener::bind(&sock_path)
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("could not bind socket: {e}")))?;

        let notifier = Arc::new(TerminalNotifier::default());

        // Initialize the delivery dispatcher and clock.
        //
        // Neither the Herdr nor the tmux binary is mandatory: a host running only
        // tmux (or neither, with `--target command:`/`wait`) must still be able to
        // start the daemon. A missing backend is reported at delivery time.
        let herdr: Option<Box<dyn DeliveryAdapter>> = RealHerdrAdapter::new()
            .ok()
            .map(|a| Box::new(a) as Box<dyn DeliveryAdapter>);
        let tmux: Option<Box<dyn DeliveryAdapter>> = TmuxAdapter::new()
            .ok()
            .map(|a| Box::new(a) as Box<dyn DeliveryAdapter>);
        let adapter = Box::new(TargetDispatcher::new(herdr, tmux));
        let clock = Box::new(SystemClock::default());

        let store_clone = Arc::clone(&store);
        let outbox_clone = Arc::clone(&outbox);
        let notifier_clone = Arc::clone(&notifier);
        let on_event = Box::new(move |event: FiredEvent| {
            let id = match &event {
                FiredEvent::Fired { id, .. }
                | FiredEvent::Failed { id, .. }
                | FiredEvent::Missed { id }
                | FiredEvent::Progress { id, .. } => id.clone(),
            };
            if let Ok(mut alarm) = store_clone.get(&id) {
                if alarm.status != AlarmStatus::Pending {
                    return;
                }
                let mut is_terminal = false;
                match event {
                    FiredEvent::Fired { at, .. } => {
                        alarm.status = AlarmStatus::Fired;
                        alarm.fired_at = Some(at);
                        is_terminal = true;
                    }
                    FiredEvent::Failed { reason, .. } => {
                        alarm.status = AlarmStatus::Failed;
                        alarm.failure_reason = Some(reason);
                        is_terminal = true;
                    }
                    FiredEvent::Missed { .. } => {
                        alarm.status = AlarmStatus::Missed;
                        alarm.failure_reason =
                            Some("deadline exceeded grace window during downtime".to_string());
                        is_terminal = true;
                    }
                    FiredEvent::Progress { next_check_at, .. } => {
                        alarm.deadline = next_check_at;
                    }
                }
                let _ = store_clone.put(&alarm);

                if is_terminal {
                    let outbox_event = OutboxEvent::from_terminal_alarm(&alarm, alarm.status);
                    let _ = outbox_clone.append(&outbox_event);
                    notifier_clone.notify();
                }
            }
        });

        // 10-minute default grace window per SPEC §3.4
        let scheduler = Arc::new(Scheduler::new(
            adapter,
            clock,
            Duration::from_mins(10),
            on_event,
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

        // Spawn dedicated Herdr event listener thread
        if let Ok(listener) = crate::herdr::HerdrEventListener::new(Arc::clone(&scheduler)) {
            std::thread::spawn(move || {
                listener.run();
            });
        }

        let ctx = Arc::new(DaemonContext {
            store,
            outbox,
            scheduler,
            notifier,
            shutdown: Arc::new(AtomicBool::new(false)),
            sock_path: sock_path.clone(),
        });

        // Main IPC accept loop. One thread per connection.
        for stream in listener.incoming() {
            if ctx.shutdown.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else {
                continue;
            };
            let ctx = Arc::clone(&ctx);
            std::thread::spawn(move || handle_connection(stream, &ctx));
        }

        let _ = std::fs::remove_file(sock_path);
        Ok(())
    }
}

/// Serves one client connection until it disconnects or requests shutdown.
#[allow(clippy::too_many_lines)]
fn handle_connection(stream: UnixStream, ctx: &DaemonContext) {
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader_stream);
    let mut writer = stream;
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
                respond(
                    &mut writer,
                    &Response::Error(format!("malformed request: {e}")),
                );
                line.clear();
                continue;
            }
        };

        line.clear();

        match req {
            Request::Shutdown => {
                respond(&mut writer, &Response::Ok);
                ctx.shutdown.store(true, Ordering::SeqCst);
                // Wake the blocking accept loop so it observes the flag and exits.
                let _ = UnixStream::connect(&ctx.sock_path);
                return;
            }
            Request::Ping => respond(&mut writer, &Response::Ok),
            Request::SetAlarm(alarm) => {
                // 1. Durability before ACK: write to disk before responding Ok
                if let Err(e) = ctx.store.put(&alarm) {
                    respond(&mut writer, &Response::Error(e.to_string()));
                    continue;
                }

                // 2. Arm in scheduler
                ctx.scheduler.arm(alarm);

                // 3. Respond Ok to client
                respond(&mut writer, &Response::Ok);
            }
            Request::ListAlarms(filter) => {
                let resp = match ctx.store.list(filter) {
                    Ok(alarms) => Response::AlarmList(alarms),
                    Err(e) => Response::Error(e.to_string()),
                };
                respond(&mut writer, &resp);
            }
            Request::GetAlarm(id) => {
                let resp = match ctx.store.get(&id) {
                    Ok(alarm) => Response::Alarm(alarm),
                    Err(SnuzeError::AlarmNotFound(id)) => Response::NotFound(id),
                    Err(e) => Response::Error(e.to_string()),
                };
                respond(&mut writer, &resp);
            }
            Request::CancelAlarm(id) => {
                let removed_from_scheduler = ctx.scheduler.cancel(&id);
                let mut found = removed_from_scheduler;

                if let Ok(mut alarm) = ctx.store.get(&id) {
                    if alarm.status == AlarmStatus::Pending {
                        alarm.status = AlarmStatus::Canceled;
                        let _ = ctx.store.put(&alarm);
                        let outbox_event =
                            OutboxEvent::from_terminal_alarm(&alarm, AlarmStatus::Canceled);
                        let _ = ctx.outbox.append(&outbox_event);
                        found = true;
                        ctx.notifier.notify();
                    }
                }

                respond(&mut writer, &Response::CancelResult(found));
            }
            Request::SignalAlarm { match_key } => {
                let resolved_ids = ctx
                    .scheduler
                    .signal(crate::scheduler::PushSignal::ExternalSignal { match_key });
                respond(&mut writer, &Response::SignalResult(resolved_ids));
            }
            Request::WaitAlarm {
                alarm_id,
                timeout_secs,
            } => {
                handle_wait_alarm(&mut writer, ctx, &alarm_id, timeout_secs);
            }
            Request::DrainEvents => {
                let resp = match ctx.outbox.drain() {
                    Ok(events) => Response::EventsDrained(events),
                    Err(e) => Response::Error(e.to_string()),
                };
                respond(&mut writer, &resp);
            }
        }
    }
}

/// Blocks the calling connection thread until `alarm_id` is terminal or the
/// optional timeout elapses.
fn handle_wait_alarm(
    writer: &mut UnixStream,
    ctx: &DaemonContext,
    alarm_id: &str,
    timeout_secs: Option<u64>,
) {
    let deadline = timeout_secs.map(|secs| Instant::now() + Duration::from_secs(secs));

    loop {
        match ctx.store.get(alarm_id) {
            Ok(alarm) if alarm.status == AlarmStatus::Pending => {}
            Ok(alarm) if alarm.status == AlarmStatus::Fired => {
                respond(
                    writer,
                    &Response::AlarmFired {
                        alarm_id: alarm_id.to_string(),
                    },
                );
                return;
            }
            Ok(alarm) => {
                respond(
                    writer,
                    &Response::Error(format!(
                        "alarm {alarm_id} reached terminal status {}",
                        status_name(alarm.status)
                    )),
                );
                return;
            }
            Err(SnuzeError::AlarmNotFound(id)) => {
                respond(writer, &Response::NotFound(id));
                return;
            }
            Err(e) => {
                respond(writer, &Response::Error(e.to_string()));
                return;
            }
        }

        let slice = match deadline {
            Some(at) => {
                let now = Instant::now();
                if now >= at {
                    respond(
                        writer,
                        &Response::Error(format!("timed out waiting for alarm {alarm_id}")),
                    );
                    return;
                }
                std::cmp::min(at - now, WAIT_RECHECK_INTERVAL)
            }
            None => WAIT_RECHECK_INTERVAL,
        };

        ctx.notifier.wait(slice);
    }
}
