pub mod clock;
pub mod heap;

use crate::kernel::Alarm;
use crate::scheduler::heap::AlarmHeap;
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

/// Terminal events emitted by the scheduler when an alarm transitions to a completed state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FiredEvent {
    /// Alarm fired and was delivered successfully.
    Fired {
        /// Alarm identifier.
        id: String,
        /// Time when delivery completed.
        at: SystemTime,
    },
    /// Alarm delivery failed.
    Failed {
        /// Alarm identifier.
        id: String,
        /// Failure reason returned by the delivery adapter.
        reason: String,
    },
    /// Alarm deadline was missed past the grace window.
    Missed {
        /// Alarm identifier.
        id: String,
    },
}

/// In-memory scheduling engine that triggers alarm delivery at deadlines.
pub struct Scheduler {
    heap: Mutex<AlarmHeap>,
    adapter: Box<dyn HerdrAdapter>,
    clock: Box<dyn Clock>,
    grace_window: Duration,
    on_terminal: Box<dyn Fn(FiredEvent) + Send + Sync>,
}

impl Scheduler {
    /// Creates a new `Scheduler`.
    #[must_use]
    pub fn new(
        adapter: Box<dyn HerdrAdapter>,
        clock: Box<dyn Clock>,
        grace_window: Duration,
        on_terminal: Box<dyn Fn(FiredEvent) + Send + Sync>,
    ) -> Self {
        Self {
            heap: Mutex::new(AlarmHeap::new()),
            adapter,
            clock,
            grace_window,
            on_terminal,
        }
    }

    /// Arms an alarm. If the deadline is already past the grace window at arm time,
    /// emits `FiredEvent::Missed` immediately without queuing.
    pub fn arm(&self, alarm: Alarm) {
        let now = self.clock.now();
        if let Ok(overdue) = now.duration_since(alarm.deadline) {
            if overdue > self.grace_window {
                (self.on_terminal)(FiredEvent::Missed { id: alarm.id });
                return;
            }
        }

        let mut heap = match self.heap.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        heap.push(alarm);
        drop(heap);
        self.clock.wake();
    }

    /// Cancels a pending alarm by ID. Returns `true` if it was found and removed.
    #[must_use]
    pub fn cancel(&self, id: &str) -> bool {
        let mut heap = match self.heap.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let found = heap.remove(id);
        drop(heap);
        if found {
            self.clock.wake();
        }
        found
    }

    /// Runs the scheduler loop on the calling thread. Sleeps via condvar until the next deadline.
    pub fn run(&self) {
        loop {
            let (alarm_to_fire, wait_deadline) = {
                let mut heap = match self.heap.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let now = self.clock.now();
                if let Some(top) = heap.peek() {
                    if top.deadline <= now {
                        (heap.pop(), None)
                    } else {
                        (None, Some(top.deadline))
                    }
                } else {
                    (None, None)
                }
            };

            if let Some(alarm) = alarm_to_fire {
                let now = self.clock.now();
                if let Ok(overdue) = now.duration_since(alarm.deadline) {
                    if overdue > self.grace_window {
                        (self.on_terminal)(FiredEvent::Missed { id: alarm.id });
                        continue;
                    }
                }

                let outcome = self.adapter.deliver(&alarm);
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
                (self.on_terminal)(event);
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
}
