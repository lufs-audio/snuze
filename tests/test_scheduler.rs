#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use snuze::kernel::{Alarm, AlarmStatus, DeliveryMode, DeliveryTarget, SnuzeError, Trigger};
use snuze::scheduler::{DeliveryAdapter, DeliveryOutcome, FakeClock, FiredEvent, Scheduler};

#[derive(Clone, Default)]
struct MockHerdrAdapter {
    /// Scripted outcome, keyed by the delivery payload text.
    outcomes: Arc<Mutex<HashMap<String, DeliveryOutcome>>>,
    deliveries: Arc<Mutex<Vec<(DeliveryTarget, DeliveryMode)>>>,
}

impl MockHerdrAdapter {
    fn new() -> Self {
        Self::default()
    }

    fn set_outcome(&self, payload_text: &str, outcome: DeliveryOutcome) {
        self.outcomes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(payload_text.to_string(), outcome);
    }

    fn delivered_count(&self) -> usize {
        self.deliveries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

impl DeliveryAdapter for MockHerdrAdapter {
    fn deliver(
        &self,
        target: &DeliveryTarget,
        delivery: &DeliveryMode,
    ) -> Result<DeliveryOutcome, SnuzeError> {
        self.deliveries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((target.clone(), delivery.clone()));

        let text = match delivery {
            DeliveryMode::Prompt { text } | DeliveryMode::Command { text } => text,
        };

        Ok(self
            .outcomes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(text)
            .unwrap_or(DeliveryOutcome::Delivered))
    }
}

fn create_test_alarm(id: &str, deadline: SystemTime) -> Alarm {
    Alarm {
        id: id.to_string(),
        target: DeliveryTarget::Herdr {
            pane_id: "pane_test".to_string(),
        },
        created_by_pane: Some("pane_creator".to_string()),
        delivery: DeliveryMode::Prompt {
            text: format!("wake up {id}"),
        },
        label: Some(format!("label_{id}")),
        created_at: SystemTime::UNIX_EPOCH,
        deadline,
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::Deadline,
        schema_version: 1,
    }
}

fn setup_test_scheduler(
    clock: FakeClock,
    adapter: MockHerdrAdapter,
    grace_window: Duration,
) -> (Arc<Scheduler>, Receiver<FiredEvent>) {
    let (tx, rx): (Sender<FiredEvent>, Receiver<FiredEvent>) = channel();
    let scheduler = Arc::new(Scheduler::new(
        Box::new(adapter),
        Box::new(clock),
        grace_window,
        Box::new(move |event| {
            let _ = tx.send(event);
        }),
    ));

    let sched_bg = Arc::clone(&scheduler);
    thread::spawn(move || {
        sched_bg.run();
    });

    (scheduler, rx)
}

#[test]
fn test_scheduler_fire_success() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let clock = FakeClock::new(base_time);
    let adapter = MockHerdrAdapter::new();
    let (scheduler, rx) =
        setup_test_scheduler(clock.clone(), adapter.clone(), Duration::from_mins(10));

    let deadline = base_time + Duration::from_secs(30);
    scheduler.arm(create_test_alarm("alarm-1", deadline));

    // Advance halfway — should not fire yet
    clock.advance(Duration::from_secs(15));
    assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    assert_eq!(adapter.delivered_count(), 0);

    // Advance past deadline — should fire exactly once
    clock.advance(Duration::from_secs(15));
    let event = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected FiredEvent");

    match event {
        FiredEvent::Fired { id, at } => {
            assert_eq!(id, "alarm-1");
            assert_eq!(at, base_time + Duration::from_secs(30));
        }
        _ => panic!("unexpected event: {event:?}"),
    }

    assert_eq!(adapter.delivered_count(), 1);

    // Advance further — should not fire again
    clock.advance(Duration::from_secs(100));
    assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    assert_eq!(adapter.delivered_count(), 1);
}

#[test]
fn test_scheduler_cancel_pending() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let clock = FakeClock::new(base_time);
    let adapter = MockHerdrAdapter::new();
    let (scheduler, rx) =
        setup_test_scheduler(clock.clone(), adapter.clone(), Duration::from_mins(10));

    let deadline = base_time + Duration::from_secs(30);
    scheduler.arm(create_test_alarm("alarm-cancel", deadline));

    // Cancel before deadline
    let canceled = scheduler.cancel("alarm-cancel");
    assert!(canceled, "cancel should return true for pending alarm");

    let cancel_missing = scheduler.cancel("non-existent");
    assert!(
        !cancel_missing,
        "cancel should return false for unknown alarm"
    );

    // Advance far past deadline
    clock.advance(Duration::from_secs(1000));
    assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    assert_eq!(
        adapter.delivered_count(),
        0,
        "zero deliver calls after cancellation"
    );
}

#[test]
fn test_scheduler_deadline_ordering() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let clock = FakeClock::new(base_time);
    let adapter = MockHerdrAdapter::new();
    let (scheduler, rx) =
        setup_test_scheduler(clock.clone(), adapter.clone(), Duration::from_mins(10));

    // Arm alarms out of order, 1ms apart
    let d1 = base_time + Duration::from_millis(100);
    let d2 = base_time + Duration::from_millis(101);
    let d3 = base_time + Duration::from_millis(102);

    scheduler.arm(create_test_alarm("third", d3));
    scheduler.arm(create_test_alarm("first", d1));
    scheduler.arm(create_test_alarm("second", d2));

    // Advance to d3
    clock.advance(Duration::from_millis(150));

    let ev1 = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected first event");
    let ev2 = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected second event");
    let ev3 = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected third event");

    match (ev1, ev2, ev3) {
        (
            FiredEvent::Fired { id: id1, .. },
            FiredEvent::Fired { id: id2, .. },
            FiredEvent::Fired { id: id3, .. },
        ) => {
            assert_eq!(id1, "first");
            assert_eq!(id2, "second");
            assert_eq!(id3, "third");
        }
        other => panic!("unexpected event sequence: {other:?}"),
    }
}

#[test]
fn test_scheduler_missed_past_grace_window_at_arm_time() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let clock = FakeClock::new(base_time);
    let adapter = MockHerdrAdapter::new();
    let grace_window = Duration::from_mins(10);
    let (scheduler, rx) = setup_test_scheduler(clock.clone(), adapter.clone(), grace_window);

    // Deadline is 700s in the past (> 600s grace window)
    let past_deadline = base_time - Duration::from_secs(700);
    scheduler.arm(create_test_alarm("missed-alarm", past_deadline));

    let event = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Missed event");

    match event {
        FiredEvent::Missed { id } => {
            assert_eq!(id, "missed-alarm");
        }
        _ => panic!("expected Missed event, got {event:?}"),
    }

    assert_eq!(
        adapter.delivered_count(),
        0,
        "no deliver call for missed alarm"
    );
}

#[test]
fn test_scheduler_delivery_failure_variants() {
    let failure_outcomes = vec![
        (
            "pane-gone-1",
            DeliveryOutcome::PaneGone("pane w1:p9 disappeared".to_string()),
            "pane w1:p9 disappeared",
        ),
        (
            "blocked-1",
            DeliveryOutcome::AgentBlocked("agent blocked waiting on user input".to_string()),
            "agent blocked waiting on user input",
        ),
        (
            "stalled-1",
            DeliveryOutcome::PromptStalled("no lifecycle change within 5s".to_string()),
            "no lifecycle change within 5s",
        ),
        (
            "herdr-unavail-1",
            DeliveryOutcome::HerdrUnavailable("socket connection refused".to_string()),
            "socket connection refused",
        ),
    ];

    for (id, outcome, expected_reason) in failure_outcomes {
        let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let clock = FakeClock::new(base_time);
        let adapter = MockHerdrAdapter::new();
        adapter.set_outcome(&format!("wake up {id}"), outcome);

        let (scheduler, rx) =
            setup_test_scheduler(clock.clone(), adapter.clone(), Duration::from_mins(10));

        scheduler.arm(create_test_alarm(id, base_time + Duration::from_secs(10)));
        clock.advance(Duration::from_secs(10));

        let event = rx
            .recv_timeout(Duration::from_millis(500))
            .expect("expected event");

        match event {
            FiredEvent::Failed {
                id: event_id,
                reason,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(reason, expected_reason);
            }
            _ => panic!("expected Failed event, got {event:?}"),
        }
    }
}

#[test]
fn test_scheduler_cancel_next_due_alarm_rearms_next() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let clock = FakeClock::new(base_time);
    let adapter = MockHerdrAdapter::new();
    let (scheduler, rx) =
        setup_test_scheduler(clock.clone(), adapter.clone(), Duration::from_mins(10));

    let d_early = base_time + Duration::from_secs(10);
    let d_late = base_time + Duration::from_secs(30);

    scheduler.arm(create_test_alarm("early", d_early));
    scheduler.arm(create_test_alarm("late", d_late));

    // Cancel early alarm before it fires
    assert!(scheduler.cancel("early"));

    // Advance past early deadline but before late deadline
    clock.advance(Duration::from_secs(15));
    assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    assert_eq!(adapter.delivered_count(), 0);

    // Advance past late deadline
    clock.advance(Duration::from_secs(20));
    let event = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected late alarm event");

    match event {
        FiredEvent::Fired { id, .. } => assert_eq!(id, "late"),
        _ => panic!("expected Fired for late alarm"),
    }
    assert_eq!(adapter.delivered_count(), 1);
}

struct BlockingHerdrAdapter {
    tx_start: Sender<()>,
    rx_release: Mutex<Receiver<()>>,
}

impl DeliveryAdapter for BlockingHerdrAdapter {
    fn deliver(
        &self,
        _target: &DeliveryTarget,
        _delivery: &DeliveryMode,
    ) -> Result<DeliveryOutcome, SnuzeError> {
        let _ = self.tx_start.send(());
        let rx = self.rx_release.lock().unwrap();
        let _ = rx.recv();
        Ok(DeliveryOutcome::Delivered)
    }
}

#[test]
fn test_scheduler_cancel_in_flight_delivery_suppresses_fired_event() {
    let (tx_start, rx_start) = channel();
    let (tx_release, rx_release) = channel();

    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let clock = FakeClock::new(base_time);
    let adapter = BlockingHerdrAdapter {
        tx_start,
        rx_release: Mutex::new(rx_release),
    };

    let (tx, rx): (Sender<FiredEvent>, Receiver<FiredEvent>) = channel();
    let scheduler = Arc::new(Scheduler::new(
        Box::new(adapter),
        Box::new(clock.clone()),
        Duration::from_mins(10),
        Box::new(move |event| {
            let _ = tx.send(event);
        }),
    ));

    let sched_bg = Arc::clone(&scheduler);
    thread::spawn(move || {
        sched_bg.run();
    });

    scheduler.arm(create_test_alarm(
        "inflight-alarm",
        base_time + Duration::from_secs(10),
    ));
    clock.advance(Duration::from_secs(10));

    // Wait until deliver() is actively executing
    rx_start
        .recv_timeout(Duration::from_millis(500))
        .expect("deliver did not start");

    // Cancel while in-flight
    assert!(
        scheduler.cancel("inflight-alarm"),
        "cancel should return true for in-flight alarm"
    );

    // Unblock deliver()
    tx_release.send(()).unwrap();

    // Verify no FiredEvent is delivered
    assert!(
        rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "in-flight canceled alarm should not emit FiredEvent"
    );
}
