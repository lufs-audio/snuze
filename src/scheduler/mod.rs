pub mod adapter;
pub mod clock;
pub mod heap;

use crate::kernel::duration::format_duration;
use crate::kernel::{
    herdr_status_satisfied, Alarm, DeliveryMode, FileWatchMode, HerdrWatchStatus, SnuzeError,
    TemplateVars, Trigger,
};
use crate::scheduler::heap::AlarmHeap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

pub use adapter::{
    detect_default_target, select_default_target, CommandAdapter, DeliveryAdapter, DeliveryOutcome,
    TargetDispatcher,
};
pub use clock::{Clock, FakeClock, SystemClock};

/// First poll interval for process/file condition triggers.
const CONDITION_INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// Ceiling for the exponential poll backoff of condition triggers.
const CONDITION_MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Age after which the condition poll interval doubles.
const CONDITION_BACKOFF_STEP: Duration = Duration::from_secs(30);

/// True when a watched artifact currently satisfies `mode`.
///
/// `path` may be exact or contain `*`/`?` wildcards in its final component, in
/// which case any matching entry in the parent directory counts.
#[must_use]
pub fn file_artifact_satisfied(path: &Path, mode: FileWatchMode, since: SystemTime) -> bool {
    match mode {
        FileWatchMode::Exists => !matching_artifacts(path).is_empty(),
        FileWatchMode::Modified => matching_artifacts(path)
            .iter()
            .any(|candidate| modified_since(candidate, since)),
    }
}

/// Expands a possibly-globbed path into the set of existing matches.
///
/// A path with no wildcard in its final component is itself the only candidate,
/// so this reduces to an existence check.
fn matching_artifacts(path: &Path) -> Vec<PathBuf> {
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        // No final component to glob (e.g. a root or `..`); the path is its own artifact.
        return if path.exists() {
            vec![path.to_path_buf()]
        } else {
            Vec::new()
        };
    };

    if !file_name.contains(['*', '?']) {
        return if path.exists() {
            vec![path.to_path_buf()]
        } else {
            Vec::new()
        };
    }

    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };

    let Ok(entries) = std::fs::read_dir(&parent) else {
        return Vec::new();
    };

    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if glob_match(file_name, name) {
            matches.push(entry.path());
        }
    }
    matches.sort();
    matches
}

/// True when `path` exists and was modified at or after `since`.
fn modified_since(path: &Path, since: SystemTime) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    modified >= since
}

/// Minimal shell-style glob matcher supporting `*` (any run, including empty)
/// and `?` (exactly one character).
///
/// Deliberately dependency-free: only the final path component is ever matched,
/// so character classes, `**`, and brace expansion are out of scope.
#[must_use]
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();

    let (mut pi, mut ti) = (0_usize, 0_usize);
    let mut star: Option<usize> = None;
    let mut resume_from = 0_usize;

    while let Some(&current) = text.get(ti) {
        match pattern.get(pi) {
            Some('*') => {
                star = Some(pi);
                resume_from = ti;
                pi += 1;
            }
            Some(&pc) if pc == '?' || pc == current => {
                pi += 1;
                ti += 1;
            }
            _ => match star {
                Some(s) => {
                    pi = s + 1;
                    resume_from += 1;
                    ti = resume_from;
                }
                None => return false,
            },
        }
    }

    while pattern.get(pi) == Some(&'*') {
        pi += 1;
    }
    pi == pattern.len()
}

/// Non-blocking liveness probe for a process id.
///
/// Uses `kill(pid, 0)`, which performs permission and existence checks without
/// sending a signal. `EPERM` means the process exists but belongs to another
/// user, so it still counts as running.
///
/// Returns `false` for pid `0` (which would address the caller's own process
/// group) and for pids that cannot be represented as a `pid_t`.
///
/// Note: a zombie child that has exited but not yet been reaped still reports as
/// running, exactly as `kill(pid, 0)` does. Callers that own the child should reap
/// it (`wait`) to make the exit observable.
#[must_use]
pub fn is_process_running(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }

    #[cfg(unix)]
    {
        let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if ret == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    #[cfg(not(unix))]
    {
        false
    }
}

/// Exponential poll backoff derived deterministically from how long a condition
/// alarm has been pending.
///
/// Stateless on purpose: `Trigger::ProcessExit`/`Trigger::FileArtifact` carry no
/// backoff field, so the interval is a pure function of the alarm's age. It
/// doubles every [`CONDITION_BACKOFF_STEP`] and saturates at
/// [`CONDITION_MAX_BACKOFF`], which keeps polling cheap without a busy loop.
#[must_use]
fn condition_poll_backoff(elapsed: Duration) -> Duration {
    let steps = (elapsed.as_secs() / CONDITION_BACKOFF_STEP.as_secs()).min(5);
    let shift = u32::try_from(steps).unwrap_or(0);
    std::cmp::min(
        CONDITION_INITIAL_BACKOFF.saturating_mul(1u32 << shift),
        CONDITION_MAX_BACKOFF,
    )
}

/// Push signal delivered to the scheduler to resolve matching pending alarms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushSignal {
    HerdrPaneEvent {
        pane_id: String,
        new_status: HerdrWatchStatus,
    },
    ExternalSignal {
        match_key: String,
    },
}

/// Interface contract for dispatching alarm payloads to their delivery target.
///
/// Re-exported from [`adapter`] so callers can name it as `snuze::scheduler::DeliveryAdapter`.
///
/// Events emitted by the scheduler when an alarm transitions or updates progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FiredEvent {
    /// Alarm fired and was delivered successfully.
    Fired {
        /// Alarm identifier.
        id: String,
        /// Time when delivery completed.
        at: SystemTime,
    },
    /// Alarm delivery failed or timed out.
    Failed {
        /// Alarm identifier.
        id: String,
        /// Failure reason returned by the delivery adapter or "timeout".
        reason: String,
    },
    /// Alarm deadline was missed past the grace window.
    Missed {
        /// Alarm identifier.
        id: String,
    },
    /// Non-terminal update to a pending condition alarm: the alarm was re-armed
    /// for a later check, which the daemon must persist across restarts.
    Progress {
        /// Alarm identifier.
        id: String,
        /// Next scheduled poll time.
        next_check_at: SystemTime,
    },
}

#[derive(Default)]
struct SchedulerState {
    heap: AlarmHeap,
    in_flight: HashSet<String>,
    canceled: HashSet<String>,
}

/// In-memory scheduling engine that triggers alarm delivery at deadlines.
pub struct Scheduler {
    state: Mutex<SchedulerState>,
    adapter: Box<dyn DeliveryAdapter>,
    clock: Box<dyn Clock>,
    grace_window: Duration,
    on_event: Box<dyn Fn(FiredEvent) + Send + Sync>,
}

/// Maps an adapter result onto the scheduler's fired/failed event shape.
fn delivery_event(
    id: &str,
    now: SystemTime,
    result: Result<DeliveryOutcome, SnuzeError>,
) -> FiredEvent {
    match result {
        Ok(DeliveryOutcome::Delivered) => FiredEvent::Fired {
            id: id.to_string(),
            at: now,
        },
        Ok(other) => FiredEvent::Failed {
            id: id.to_string(),
            reason: other
                .failure_reason()
                .unwrap_or("delivery failed")
                .to_string(),
        },
        Err(e) => FiredEvent::Failed {
            id: id.to_string(),
            reason: e.to_string(),
        },
    }
}

impl Scheduler {
    /// Creates a new `Scheduler`.
    #[must_use]
    pub fn new(
        adapter: Box<dyn DeliveryAdapter>,
        clock: Box<dyn Clock>,
        grace_window: Duration,
        on_event: Box<dyn Fn(FiredEvent) + Send + Sync>,
    ) -> Self {
        Self {
            state: Mutex::new(SchedulerState::default()),
            adapter,
            clock,
            grace_window,
            on_event,
        }
    }

    /// Arms an alarm.
    ///
    /// `Deadline` alarms are checked against the grace window; every condition
    /// trigger is checked against its own `timeout_at` ceiling.
    pub fn arm(&self, mut alarm: Alarm) {
        let now = self.clock.now();
        match &alarm.trigger {
            Trigger::Deadline => {
                if let Ok(overdue) = now.duration_since(alarm.deadline) {
                    if overdue > self.grace_window {
                        (self.on_event)(FiredEvent::Missed { id: alarm.id });
                        return;
                    }
                }
            }
            Trigger::HerdrStatus { timeout_at, .. }
            | Trigger::ExternalSignal { timeout_at, .. } => {
                if now >= *timeout_at {
                    (self.on_event)(FiredEvent::Failed {
                        id: alarm.id,
                        reason: "timeout".to_string(),
                    });
                    return;
                }
                alarm.deadline = *timeout_at;
            }
            Trigger::ProcessExit { timeout_at, .. } => {
                if now >= *timeout_at {
                    (self.on_event)(FiredEvent::Failed {
                        id: alarm.id,
                        reason: "timeout".to_string(),
                    });
                    return;
                }
                // Check the monitored process immediately, then re-poll with backoff.
                alarm.deadline = now;
            }
            Trigger::FileArtifact { timeout_at, .. } => {
                if now >= *timeout_at {
                    (self.on_event)(FiredEvent::Failed {
                        id: alarm.id,
                        reason: "timeout".to_string(),
                    });
                    return;
                }
                // Evaluate the artifact immediately, then re-poll with backoff.
                alarm.deadline = now;
            }
        }

        let mut state = match self.state.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        state.canceled.remove(&alarm.id);
        state.heap.push(alarm);
        drop(state);
        self.clock.wake();
    }

    /// See SPEC.md §3.3 for the full contract. Broadcast semantics: returns
    /// the IDs of every alarm this call resolved; an empty Vec is a valid,
    /// non-error outcome.
    pub fn signal(&self, signal: PushSignal) -> Vec<String> {
        let mut resolved_ids = Vec::new();
        let mut to_deliver = Vec::new();
        let now = self.clock.now();

        {
            let mut state = match self.state.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };

            match signal {
                PushSignal::ExternalSignal { match_key } => {
                    let removed = state.heap.extract_matching(|alarm| {
                        if let Trigger::ExternalSignal {
                            match_key: ref trigger_key,
                            ..
                        } = alarm.trigger
                        {
                            trigger_key == &match_key
                        } else {
                            false
                        }
                    });
                    for alarm in removed {
                        resolved_ids.push(alarm.id.clone());
                        state.in_flight.insert(alarm.id.clone());
                        to_deliver.push(alarm);
                    }
                }
                PushSignal::HerdrPaneEvent {
                    pane_id,
                    new_status,
                } => {
                    let removed = state.heap.extract_matching(|alarm| {
                        if let Trigger::HerdrStatus {
                            watched_pane_id: ref wpid,
                            watch_for: ref wf,
                            ref mut last_known,
                            ..
                        } = alarm.trigger
                        {
                            if wpid == &pane_id {
                                if herdr_status_satisfied(wf, new_status) {
                                    return true;
                                }
                                *last_known = Some(new_status);
                            }
                        }
                        false
                    });

                    for alarm in removed {
                        resolved_ids.push(alarm.id.clone());
                        state.in_flight.insert(alarm.id.clone());
                        to_deliver.push(alarm);
                    }
                }
            }
        }

        for alarm in to_deliver {
            self.deliver_condition_alarm(alarm, now);
        }

        resolved_ids
    }

    /// Cancels a pending alarm by ID. Returns `true` if it was found and removed or marked canceled.
    #[must_use]
    pub fn cancel(&self, id: &str) -> bool {
        let mut state = match self.state.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let heap_found = state.heap.remove(id);
        let in_flight_found = state.in_flight.contains(id);
        if in_flight_found {
            state.canceled.insert(id.to_string());
        }
        drop(state);
        if heap_found || in_flight_found {
            self.clock.wake();
            true
        } else {
            false
        }
    }

    fn is_canceled(&self, id: &str) -> bool {
        let state = match self.state.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        state.canceled.contains(id)
    }

    fn finish_in_flight(&self, id: &str) -> bool {
        let mut state = match self.state.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        state.in_flight.remove(id);
        state.canceled.remove(id)
    }

    /// Runs the scheduler loop on the calling thread. Sleeps via condvar until the next deadline.
    pub fn run(&self) {
        loop {
            let (alarm_to_fire, wait_deadline) = {
                let mut state = match self.state.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let now = self.clock.now();
                if let Some(top) = state.heap.peek() {
                    if top.deadline <= now {
                        let alarm = state.heap.pop();
                        if let Some(ref a) = alarm {
                            state.in_flight.insert(a.id.clone());
                        }
                        (alarm, None)
                    } else {
                        (None, Some(top.deadline))
                    }
                } else {
                    (None, None)
                }
            };

            if let Some(alarm) = alarm_to_fire {
                let now = self.clock.now();
                match &alarm.trigger {
                    Trigger::Deadline => self.process_deadline_alarm(alarm, now),
                    Trigger::ProcessExit { .. } => self.process_process_exit_alarm(alarm, now),
                    Trigger::FileArtifact { .. } => self.process_file_artifact_alarm(alarm, now),
                    Trigger::HerdrStatus { .. } | Trigger::ExternalSignal { .. } => {
                        if self.is_canceled(&alarm.id) {
                            self.finish_in_flight(&alarm.id);
                        } else if !self.finish_in_flight(&alarm.id) {
                            (self.on_event)(FiredEvent::Failed {
                                id: alarm.id,
                                reason: "timeout".to_string(),
                            });
                        }
                    }
                }
            } else if let Some(deadline) = wait_deadline {
                self.clock.wait_until(deadline);
            } else {
                let far_future = match self.clock.now().checked_add(Duration::from_hours(8760)) {
                    Some(t) => t,
                    None => self.clock.now(),
                };
                self.clock.wait_until(far_future);
            }
        }
    }

    /// Delivers a condition alarm that has just been satisfied (same delivery and
    /// event shape as a normally-popped alarm).
    fn deliver_condition_alarm(&self, alarm: Alarm, now: SystemTime) {
        if self.is_canceled(&alarm.id) {
            self.finish_in_flight(&alarm.id);
            return;
        }

        let dur = now.duration_since(alarm.created_at).unwrap_or_default();
        let dur_str = format_duration(dur);
        let vars = TemplateVars {
            duration: Some(dur_str),
            session_id: None,
            status: None,
            pr_urls: None,
        };
        let mut delivered_alarm = alarm.clone();
        match &mut delivered_alarm.delivery {
            DeliveryMode::Prompt { text } | DeliveryMode::Command { text } => {
                *text = crate::kernel::interpolate(text, &vars);
            }
        }

        let result = self
            .adapter
            .deliver(&delivered_alarm.target, &delivered_alarm.delivery);
        if self.finish_in_flight(&alarm.id) {
            return;
        }

        let event = delivery_event(&alarm.id, now, result);
        (self.on_event)(event);
    }

    /// Processes a `Trigger::Deadline` alarm: checks grace window, interpolates
    /// template tokens, delivers via the configured adapter.
    fn process_deadline_alarm(&self, alarm: Alarm, now: SystemTime) {
        if self.is_canceled(&alarm.id) {
            self.finish_in_flight(&alarm.id);
            return;
        }

        if let Ok(overdue) = now.duration_since(alarm.deadline) {
            if overdue > self.grace_window {
                if !self.finish_in_flight(&alarm.id) {
                    (self.on_event)(FiredEvent::Missed { id: alarm.id });
                }
                return;
            }
        }

        let dur = now.duration_since(alarm.created_at).unwrap_or_default();
        let dur_str = format_duration(dur);
        let vars = TemplateVars {
            duration: Some(dur_str),
            session_id: None,
            status: None,
            pr_urls: None,
        };
        let mut delivered_alarm = alarm.clone();
        match &mut delivered_alarm.delivery {
            DeliveryMode::Prompt { text } | DeliveryMode::Command { text } => {
                *text = crate::kernel::interpolate(text, &vars);
            }
        }

        let result = self
            .adapter
            .deliver(&delivered_alarm.target, &delivered_alarm.delivery);

        if self.finish_in_flight(&alarm.id) {
            return;
        }

        let event = delivery_event(&alarm.id, now, result);
        (self.on_event)(event);
    }

    /// Processes a `Trigger::ProcessExit` alarm: re-checks the monitored PID and
    /// either delivers (process gone), fails (timeout ceiling reached), or re-arms
    /// with the capped poll backoff.
    fn process_process_exit_alarm(&self, alarm: Alarm, now: SystemTime) {
        if self.is_canceled(&alarm.id) {
            self.finish_in_flight(&alarm.id);
            return;
        }

        let Trigger::ProcessExit { pid, timeout_at } = alarm.trigger else {
            self.finish_in_flight(&alarm.id);
            return;
        };

        if !is_process_running(pid) {
            self.deliver_condition_alarm(alarm, now);
            return;
        }

        if self.finish_in_flight(&alarm.id) {
            return;
        }

        if now >= timeout_at {
            (self.on_event)(FiredEvent::Failed {
                id: alarm.id,
                reason: "timeout".to_string(),
            });
            return;
        }

        self.rearm_condition(&alarm, now, timeout_at);
    }

    /// Processes a `Trigger::FileArtifact` alarm: re-evaluates the watched path
    /// and either delivers (condition satisfied), fails (timeout ceiling reached),
    /// or re-arms with the capped poll backoff.
    fn process_file_artifact_alarm(&self, alarm: Alarm, now: SystemTime) {
        if self.is_canceled(&alarm.id) {
            self.finish_in_flight(&alarm.id);
            return;
        }

        let Trigger::FileArtifact {
            path,
            mode,
            timeout_at,
        } = &alarm.trigger
        else {
            self.finish_in_flight(&alarm.id);
            return;
        };

        if file_artifact_satisfied(path, *mode, alarm.created_at) {
            self.deliver_condition_alarm(alarm, now);
            return;
        }

        let timeout_at = *timeout_at;

        if self.finish_in_flight(&alarm.id) {
            return;
        }

        if now >= timeout_at {
            (self.on_event)(FiredEvent::Failed {
                id: alarm.id,
                reason: "timeout".to_string(),
            });
            return;
        }

        self.rearm_condition(&alarm, now, timeout_at);
    }

    /// Re-pushes a condition alarm with an exponentially backed-off next-check
    /// time, and emits `Progress` so the daemon can persist it across restarts.
    ///
    /// The alarm is re-armed *before* `Progress` is emitted, so a `cancel` racing
    /// with the notification always finds the alarm on the heap.
    fn rearm_condition(&self, alarm: &Alarm, now: SystemTime, timeout_at: SystemTime) {
        let elapsed = now.duration_since(alarm.created_at).unwrap_or_default();
        let next_check_at = std::cmp::min(now + condition_poll_backoff(elapsed), timeout_at);

        let mut rearmed = alarm.clone();
        rearmed.deadline = next_check_at;

        {
            let mut state = match self.state.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            if state.canceled.contains(&alarm.id) {
                return;
            }
            state.heap.push(rearmed);
        }

        (self.on_event)(FiredEvent::Progress {
            id: alarm.id.clone(),
            next_check_at,
        });
    }
}
