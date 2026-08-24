pub mod clock;
pub mod heap;

use crate::jules::JulesAdapter;
use crate::kernel::duration::format_duration;
use crate::kernel::{
    Alarm, DeliveryMode, JulesSessionState, TemplateVars, Trigger,
};
use crate::scheduler::heap::AlarmHeap;
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

pub use clock::{Clock, FakeClock, SystemClock};

/// Outcome of attempting to deliver an alarm to a Herdr pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryOutcome {
    /// Alarm delivered successfully to target pane.
    Delivered,
    /// Target agent was blocked on another interactive input or state (`--prompt` only).
    AgentBlocked(String),
    /// Target agent prompt did not trigger an observed lifecycle change within the window (`--prompt` only).
    PromptStalled(String),
    /// Target pane or agent no longer resolves or does not exist.
    PaneGone(String),
    /// Herdr daemon was unavailable, command timed out, or invocation was malformed.
    HerdrUnavailable(String),
}

/// Interface contract for dispatching alarm payloads to Herdr.
pub trait HerdrAdapter: Send + Sync {
    /// Deliver an alarm payload to the target Herdr pane.
    fn deliver(&self, alarm: &Alarm) -> DeliveryOutcome;
}

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
    /// Non-terminal update to a pending condition alarm's known state.
    Progress {
        /// Alarm identifier.
        id: String,
        /// Observed session states from recent poll.
        last_known: Vec<JulesSessionState>,
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
    adapter: Box<dyn HerdrAdapter>,
    jules_adapter: Box<dyn JulesAdapter>,
    clock: Box<dyn Clock>,
    grace_window: Duration,
    on_event: Box<dyn Fn(FiredEvent) + Send + Sync>,
}

impl Scheduler {
    /// Creates a new `Scheduler`.
    #[must_use]
    pub fn new(
        adapter: Box<dyn HerdrAdapter>,
        jules_adapter: Box<dyn JulesAdapter>,
        clock: Box<dyn Clock>,
        grace_window: Duration,
        on_event: Box<dyn Fn(FiredEvent) + Send + Sync>,
    ) -> Self {
        Self {
            state: Mutex::new(SchedulerState::default()),
            adapter,
            jules_adapter,
            clock,
            grace_window,
            on_event,
        }
    }

    /// Arms an alarm. For `Deadline`, checks the grace window; for `JulesCondition`, checks `timeout_at`.
    pub fn arm(&self, alarm: Alarm) {
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
            Trigger::JulesCondition { timeout_at, .. } => {
                if now >= *timeout_at {
                    (self.on_event)(FiredEvent::Failed {
                        id: alarm.id,
                        reason: "timeout".to_string(),
                    });
                    return;
                }
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
                    Trigger::JulesCondition { .. } => self.process_jules_alarm(alarm, now),
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

    /// Processes a `Trigger::Deadline` alarm: checks grace window, interpolates
    /// template tokens, delivers via Herdr adapter.
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

        let outcome = self.adapter.deliver(&delivered_alarm);

        if self.finish_in_flight(&alarm.id) {
            return;
        }

        let event = match outcome {
            DeliveryOutcome::Delivered => FiredEvent::Fired {
                id: alarm.id,
                at: now,
            },
            DeliveryOutcome::AgentBlocked(reason)
            | DeliveryOutcome::PromptStalled(reason)
            | DeliveryOutcome::PaneGone(reason)
            | DeliveryOutcome::HerdrUnavailable(reason) => FiredEvent::Failed {
                id: alarm.id,
                reason,
            },
        };
        (self.on_event)(event);
    }

    /// Processes a `Trigger::JulesCondition` alarm: polls Jules sessions,
    /// evaluates the condition, fires on satisfaction, emits `Progress` for
    /// intermediate polls, or `Failed("timeout")` when the timeout ceiling is reached.
    fn process_jules_alarm(&self, alarm: Alarm, now: SystemTime) {
        if self.is_canceled(&alarm.id) {
            self.finish_in_flight(&alarm.id);
            return;
        }

        let (session_ids, condition, poll_interval_secs, timeout_at) = match &alarm.trigger {
            Trigger::JulesCondition {
                session_ids,
                condition,
                poll_interval_secs,
                timeout_at,
                ..
            } => (
                session_ids.clone(),
                *condition,
                *poll_interval_secs,
                *timeout_at,
            ),
            Trigger::Deadline => {
                self.finish_in_flight(&alarm.id);
                return;
            }
        };

        let check_result = self.jules_adapter.check_sessions(&session_ids);

        if self.is_canceled(&alarm.id) {
            self.finish_in_flight(&alarm.id);
            return;
        }

        match check_result {
            Err(crate::kernel::SnuzeError::JulesCredentialMissing) => {
                if self.finish_in_flight(&alarm.id) {
                    return;
                }
                (self.on_event)(FiredEvent::Failed {
                    id: alarm.id,
                    reason: crate::kernel::SnuzeError::JulesCredentialMissing.to_string(),
                });
            }
            Err(_) => {
                if self.finish_in_flight(&alarm.id) {
                    return;
                }
                if now >= timeout_at {
                    (self.on_event)(FiredEvent::Failed {
                        id: alarm.id,
                        reason: "timeout".to_string(),
                    });
                } else {
                    self.rearm_progress(&alarm, now, timeout_at, poll_interval_secs, Vec::new());
                }
            }
            Ok(states) => {
                if condition.is_satisfied(&states) {
                    self.deliver_jules_alarm(alarm, now, &states);
                } else {
                    if self.finish_in_flight(&alarm.id) {
                        return;
                    }
                    if now >= timeout_at {
                        (self.on_event)(FiredEvent::Failed {
                            id: alarm.id,
                            reason: "timeout".to_string(),
                        });
                    } else {
                        self.rearm_progress(&alarm, now, timeout_at, poll_interval_secs, states);
                    }
                }
            }
        }
    }

    fn deliver_jules_alarm(&self, alarm: Alarm, now: SystemTime, states: &[JulesSessionState]) {
        let dur = now.duration_since(alarm.created_at).unwrap_or_default();
        let dur_str = format_duration(dur);

        let (session_id_str, status_str, pr_urls_str) =
            crate::kernel::format_jules_template_vars(states);
        let pr_urls_val = if pr_urls_str.is_empty() {
            None
        } else {
            Some(pr_urls_str)
        };

        let vars = TemplateVars {
            duration: Some(dur_str),
            session_id: Some(session_id_str),
            status: Some(status_str),
            pr_urls: pr_urls_val,
        };

        let mut delivered_alarm = alarm.clone();
        match &mut delivered_alarm.delivery {
            DeliveryMode::Prompt { text } | DeliveryMode::Command { text } => {
                *text = crate::kernel::interpolate(text, &vars);
            }
        }

        let outcome = self.adapter.deliver(&delivered_alarm);
        if self.finish_in_flight(&alarm.id) {
            return;
        }
        let event = match outcome {
            DeliveryOutcome::Delivered => FiredEvent::Fired {
                id: alarm.id,
                at: now,
            },
            DeliveryOutcome::AgentBlocked(reason)
            | DeliveryOutcome::PromptStalled(reason)
            | DeliveryOutcome::PaneGone(reason)
            | DeliveryOutcome::HerdrUnavailable(reason) => FiredEvent::Failed {
                id: alarm.id,
                reason,
            },
        };
        (self.on_event)(event);
    }

    /// Emits a `Progress` event and re-pushes the alarm onto the heap with an updated
    /// `deadline` (next-check-time) and `last_known` session states.
    fn rearm_progress(
        &self,
        alarm: &Alarm,
        now: SystemTime,
        timeout_at: SystemTime,
        poll_interval_secs: u64,
        last_known: Vec<JulesSessionState>,
    ) {
        let next_check_at =
            std::cmp::min(now + Duration::from_secs(poll_interval_secs), timeout_at);
        (self.on_event)(FiredEvent::Progress {
            id: alarm.id.clone(),
            last_known: last_known.clone(),
            next_check_at,
        });
        let mut mut_alarm = alarm.clone();
        mut_alarm.deadline = next_check_at;
        if let Trigger::JulesCondition {
            last_known: ref mut lk,
            ..
        } = &mut mut_alarm.trigger
        {
            *lk = last_known;
        }
        let mut state = match self.state.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if !state.canceled.contains(&alarm.id) {
            state.heap.push(mut_alarm);
        }
    }
}
