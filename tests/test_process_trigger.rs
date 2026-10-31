//! Unit tests for `Trigger::ProcessExit` (Unit 02).
//!
//! Covers the PID liveness probe, firing when a real child process exits, timeout
//! expiry, cancellation, and the non-busy-loop poll backoff.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]
#![allow(clippy::duration_suboptimal_units)]

use snuze::kernel::{Alarm, AlarmStatus, DeliveryMode, DeliveryTarget, SnuzeError, Trigger};
use snuze::scheduler::{
    is_process_running, Clock, DeliveryAdapter, DeliveryOutcome, FakeClock, FiredEvent, Scheduler,
    SystemClock,
};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

#[derive(Clone, Default)]
struct RecordingAdapter {
    deliveries: Arc<Mutex<Vec<(DeliveryTarget, DeliveryMode)>>>,
}

impl RecordingAdapter {
    fn delivered_count(&self) -> usize {
        self.deliveries.lock().unwrap().len()
    }
}

impl DeliveryAdapter for RecordingAdapter {
    fn deliver(
        &self,
        target: &DeliveryTarget,
        delivery: &DeliveryMode,
    ) -> Result<DeliveryOutcome, SnuzeError> {
        self.deliveries
            .lock()
            .unwrap()
            .push((target.clone(), delivery.clone()));
        Ok(DeliveryOutcome::Delivered)
    }
}

fn process_alarm(id: &str, pid: u32, created_at: SystemTime, timeout_at: SystemTime) -> Alarm {
    Alarm {
        id: id.to_string(),
        target: DeliveryTarget::Wait,
        created_by_pane: None,
        delivery: DeliveryMode::Prompt {
            text: format!("process {pid} exited"),
        },
        label: None,
        created_at,
        deadline: timeout_at,
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::ProcessExit { pid, timeout_at },
        schema_version: 4,
    }
}

fn spawn_scheduler(
    clock: Box<dyn snuze::scheduler::Clock>,
    adapter: RecordingAdapter,
) -> (Arc<Scheduler>, Receiver<FiredEvent>) {
    let (tx, rx): (Sender<FiredEvent>, Receiver<FiredEvent>) = channel();
    let scheduler = Arc::new(Scheduler::new(
        Box::new(adapter),
        clock,
        Duration::from_mins(10),
        Box::new(move |event| {
            let _ = tx.send(event);
        }),
    ));
    let bg = Arc::clone(&scheduler);
    thread::spawn(move || bg.run());
    (scheduler, rx)
}

// ---------------------------------------------------------------------------
// Liveness probe
// ---------------------------------------------------------------------------

#[test]
fn test_is_process_running_for_own_pid() {
    assert!(is_process_running(std::process::id()));
}

#[test]
fn test_is_process_running_false_for_reaped_child() {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg("exit 0")
        .spawn()
        .expect("spawn child");
    let pid = child.id();
    assert!(
        is_process_running(pid),
        "child should be live before reaping"
    );
    child.wait().expect("reap child");
    assert!(
        !is_process_running(pid),
        "reaped child must report as not running"
    );
}

#[test]
fn test_is_process_running_rejects_unusable_pids() {
    // pid 0 would address the caller's whole process group.
    assert!(!is_process_running(0));
    // Beyond pid_t range.
    assert!(!is_process_running(u32::MAX));
}

// ---------------------------------------------------------------------------
// Firing on real process exit
// ---------------------------------------------------------------------------

#[test]
fn test_process_exit_alarm_fires_when_child_exits() {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg("sleep 0.3")
        .spawn()
        .expect("spawn child");
    let pid = child.id();

    // Reap the child once it exits, otherwise it stays a zombie and `kill(pid, 0)`
    // keeps reporting it as alive.
    thread::spawn(move || {
        let _ = child.wait();
    });

    let adapter = RecordingAdapter::default();
    let (scheduler, rx) = spawn_scheduler(Box::new(SystemClock::default()), adapter.clone());

    let now = SystemTime::now();
    scheduler.arm(process_alarm(
        "proc-fire",
        pid,
        now,
        now + Duration::from_secs(30),
    ));

    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "alarm did not fire after the child exited"
        );
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(FiredEvent::Fired { id, .. }) => {
                assert_eq!(id, "proc-fire");
                break;
            }
            Ok(FiredEvent::Progress { .. }) => continue,
            Ok(other) => panic!("unexpected event: {other:?}"),
            Err(_) => continue,
        }
    }

    assert_eq!(adapter.delivered_count(), 1);
}

// ---------------------------------------------------------------------------
// Timeout expiry
// ---------------------------------------------------------------------------

#[test]
fn test_process_exit_alarm_times_out_while_process_still_running() {
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let clock = FakeClock::new(base);
    let adapter = RecordingAdapter::default();
    let (scheduler, rx) = spawn_scheduler(Box::new(clock.clone()), adapter.clone());

    // Our own pid is guaranteed to be running for the duration of the test.
    scheduler.arm(process_alarm(
        "proc-timeout",
        std::process::id(),
        base,
        base + Duration::from_hours(1),
    ));

    // First poll happens immediately and reports "still running".
    match rx.recv_timeout(Duration::from_millis(500)).expect("poll") {
        FiredEvent::Progress { .. } => {}
        other => panic!("expected Progress, got {other:?}"),
    }

    clock.advance(Duration::from_hours(1) + Duration::from_secs(1));

    match rx.recv_timeout(Duration::from_millis(500)).expect("event") {
        FiredEvent::Failed { id, reason } => {
            assert_eq!(id, "proc-timeout");
            assert_eq!(reason, "timeout");
        }
        other => panic!("expected Failed(timeout), got {other:?}"),
    }

    assert_eq!(adapter.delivered_count(), 0, "must not deliver on timeout");
}

#[test]
fn test_process_exit_alarm_armed_past_timeout_fails_immediately() {
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let clock = FakeClock::new(base);
    let adapter = RecordingAdapter::default();
    let (scheduler, rx) = spawn_scheduler(Box::new(clock), adapter);

    scheduler.arm(process_alarm(
        "proc-expired",
        std::process::id(),
        base - Duration::from_hours(2),
        base - Duration::from_secs(1),
    ));

    match rx.recv_timeout(Duration::from_millis(500)).expect("event") {
        FiredEvent::Failed { id, reason } => {
            assert_eq!(id, "proc-expired");
            assert_eq!(reason, "timeout");
        }
        other => panic!("expected Failed(timeout), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Cancellation
// ---------------------------------------------------------------------------

#[test]
fn test_process_exit_alarm_cancelled_does_not_fire() {
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let clock = FakeClock::new(base);
    let adapter = RecordingAdapter::default();
    let (scheduler, rx) = spawn_scheduler(Box::new(clock.clone()), adapter.clone());

    scheduler.arm(process_alarm(
        "proc-cancel",
        std::process::id(),
        base,
        base + Duration::from_hours(1),
    ));
    // Drain the initial poll.
    let _ = rx.recv_timeout(Duration::from_millis(500));

    assert!(scheduler.cancel("proc-cancel"));

    clock.advance(Duration::from_hours(2));

    // Nothing but Progress events (if any) may arrive; never Fired/Failed.
    while let Ok(event) = rx.recv_timeout(Duration::from_millis(200)) {
        assert!(
            matches!(event, FiredEvent::Progress { .. }),
            "cancelled alarm produced {event:?}"
        );
    }
    assert_eq!(adapter.delivered_count(), 0);
}

// ---------------------------------------------------------------------------
// Backoff: no busy loop
// ---------------------------------------------------------------------------

/// Drives the scheduler with a fake clock and asserts that the interval between
/// successive PID polls strictly grows, stays bounded, and never degenerates into
/// a tight loop.
#[test]
fn test_process_exit_poll_interval_backs_off_and_is_capped() {
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let clock = FakeClock::new(base);
    let adapter = RecordingAdapter::default();
    let (scheduler, rx) = spawn_scheduler(Box::new(clock.clone()), adapter);

    scheduler.arm(process_alarm(
        "proc-backoff",
        std::process::id(),
        base,
        base + Duration::from_hours(24),
    ));

    let mut intervals = Vec::new();
    for _ in 0..45 {
        let event = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("expected a Progress poll");
        let FiredEvent::Progress { next_check_at, .. } = event else {
            panic!("expected Progress, got {event:?}");
        };
        let interval = next_check_at
            .duration_since(clock.now())
            .expect("next check must be in the future");
        intervals.push(interval);
        clock.advance(interval);
    }

    assert_eq!(intervals[0], Duration::from_secs(1), "first poll interval");
    assert!(
        intervals.last().unwrap() > intervals.first().unwrap(),
        "poll interval must grow: {intervals:?}"
    );
    assert!(
        intervals.iter().all(|d| *d <= Duration::from_secs(30)),
        "poll interval must be capped at 30s: {intervals:?}"
    );
    assert!(
        intervals.iter().all(|d| *d >= Duration::from_secs(1)),
        "poll interval must never be sub-second: {intervals:?}"
    );
}
