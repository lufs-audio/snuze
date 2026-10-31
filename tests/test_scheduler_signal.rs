#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]
#![allow(clippy::duration_suboptimal_units)]
#![allow(clippy::needless_raw_string_hashes)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::similar_names)]
#![allow(clippy::type_complexity)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::match_wildcard_for_single_variants)]

use snuze::jules::JulesAdapter;
use snuze::kernel::{
    Alarm, AlarmStatus, DeliveryMode, HerdrWatchStatus, JulesSessionState, SnuzeError, Trigger,
};
use snuze::scheduler::{
    DeliveryOutcome, FakeClock, FiredEvent, HerdrAdapter, PushSignal, Scheduler,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Default)]
struct MockHerdrAdapter {
    delivered_alarms: Arc<Mutex<Vec<Alarm>>>,
    outcomes: Arc<Mutex<HashMap<String, DeliveryOutcome>>>,
}

impl MockHerdrAdapter {
    fn new() -> Self {
        Self::default()
    }

    fn delivered_count(&self) -> usize {
        self.delivered_alarms.lock().unwrap().len()
    }

    fn last_delivered(&self) -> Option<Alarm> {
        self.delivered_alarms.lock().unwrap().last().cloned()
    }
}

impl HerdrAdapter for MockHerdrAdapter {
    fn deliver(&self, alarm: &Alarm) -> DeliveryOutcome {
        self.delivered_alarms.lock().unwrap().push(alarm.clone());
        self.outcomes
            .lock()
            .unwrap()
            .remove(&alarm.id)
            .unwrap_or(DeliveryOutcome::Delivered)
    }
}

#[derive(Clone, Default)]
struct MockJulesAdapter;

impl JulesAdapter for MockJulesAdapter {
    fn check_sessions(
        &self,
        _session_ids: &[String],
    ) -> Result<Vec<JulesSessionState>, SnuzeError> {
        Ok(vec![])
    }
}

fn create_herdr_alarm(
    id: &str,
    target_pane_id: &str,
    watched_pane_id: &str,
    watch_for: Vec<HerdrWatchStatus>,
    created_at: SystemTime,
    timeout_at: SystemTime,
    prompt_template: &str,
) -> Alarm {
    Alarm {
        id: id.to_string(),
        pane_id: target_pane_id.to_string(),
        created_by_pane: Some("w1:p0".to_string()),
        delivery: DeliveryMode::Prompt {
            text: prompt_template.to_string(),
        },
        label: Some(format!("alarm-{id}")),
        created_at,
        deadline: timeout_at,
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::HerdrStatus {
            watched_pane_id: watched_pane_id.to_string(),
            watch_for,
            timeout_at,
            last_known: None,
        },
        schema_version: 3,
    }
}

fn create_external_alarm(
    id: &str,
    target_pane_id: &str,
    match_key: &str,
    created_at: SystemTime,
    timeout_at: SystemTime,
    prompt_template: &str,
) -> Alarm {
    Alarm {
        id: id.to_string(),
        pane_id: target_pane_id.to_string(),
        created_by_pane: Some("w1:p0".to_string()),
        delivery: DeliveryMode::Prompt {
            text: prompt_template.to_string(),
        },
        label: Some(format!("alarm-{id}")),
        created_at,
        deadline: timeout_at,
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::ExternalSignal {
            match_key: match_key.to_string(),
            timeout_at,
        },
        schema_version: 3,
    }
}

fn setup_scheduler(
    clock: FakeClock,
    herdr_adapter: MockHerdrAdapter,
    events: Arc<Mutex<Vec<FiredEvent>>>,
) -> Arc<Scheduler> {
    let events_clone = Arc::clone(&events);
    let scheduler = Arc::new(Scheduler::new(
        Box::new(herdr_adapter),
        Box::new(MockJulesAdapter),
        Box::new(clock),
        Duration::from_mins(10),
        Box::new(move |event| {
            events_clone.lock().unwrap().push(event);
        }),
    ));

    let sched_bg = Arc::clone(&scheduler);
    std::thread::spawn(move || {
        sched_bg.run();
    });

    scheduler
}

#[test]
fn test_scheduler_signal_herdr_status_fires_on_match_and_updates_on_non_match() {
    let base_time = UNIX_EPOCH + Duration::from_secs(10_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    let scheduler = setup_scheduler(clock.clone(), herdr.clone(), Arc::clone(&events));

    let timeout_at = base_time + Duration::from_secs(300);
    let alarm = create_herdr_alarm(
        "alarm-1",
        "w1:p1",
        "w2:p2",
        vec![HerdrWatchStatus::Idle, HerdrWatchStatus::Done],
        base_time,
        timeout_at,
        "Target is ready after {duration}!",
    );

    scheduler.arm(alarm);

    // 1. Signal with non-matching status (Blocked) on watched pane
    let resolved = scheduler.signal(PushSignal::HerdrPaneEvent {
        pane_id: "w2:p2".to_string(),
        new_status: HerdrWatchStatus::Blocked,
    });
    assert!(
        resolved.is_empty(),
        "non-matching status should not resolve"
    );
    assert_eq!(herdr.delivered_count(), 0);
    assert_eq!(events.lock().unwrap().len(), 0);

    // 2. Advance time by 30 seconds
    clock.advance(Duration::from_secs(30));

    // 3. Signal with matching status (Done) on watched pane
    let resolved = scheduler.signal(PushSignal::HerdrPaneEvent {
        pane_id: "w2:p2".to_string(),
        new_status: HerdrWatchStatus::Done,
    });
    assert_eq!(resolved, vec!["alarm-1".to_string()]);
    assert_eq!(herdr.delivered_count(), 1);

    let delivered = herdr.last_delivered().expect("delivered alarm");
    match delivered.delivery {
        DeliveryMode::Prompt { text } => {
            assert_eq!(text, "Target is ready after 30s!");
        }
        _ => panic!("expected Prompt delivery"),
    }

    let evs = events.lock().unwrap();
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        FiredEvent::Fired { id, at } => {
            assert_eq!(id, "alarm-1");
            assert_eq!(*at, base_time + Duration::from_secs(30));
        }
        other => panic!("expected FiredEvent::Fired, got {other:?}"),
    }
}

#[test]
fn test_scheduler_signal_herdr_status_broadcast() {
    let base_time = UNIX_EPOCH + Duration::from_secs(20_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    let scheduler = setup_scheduler(clock.clone(), herdr.clone(), Arc::clone(&events));

    let timeout_at = base_time + Duration::from_secs(600);
    let alarm1 = create_herdr_alarm(
        "alarm-h1",
        "w1:p1",
        "w3:p1",
        vec![HerdrWatchStatus::Done, HerdrWatchStatus::Idle],
        base_time,
        timeout_at,
        "Notify pane 1",
    );
    let alarm2 = create_herdr_alarm(
        "alarm-h2",
        "w1:p2",
        "w3:p1",
        vec![HerdrWatchStatus::Done],
        base_time,
        timeout_at,
        "Notify pane 2",
    );

    scheduler.arm(alarm1);
    scheduler.arm(alarm2);

    let mut resolved = scheduler.signal(PushSignal::HerdrPaneEvent {
        pane_id: "w3:p1".to_string(),
        new_status: HerdrWatchStatus::Done,
    });
    resolved.sort();

    assert_eq!(
        resolved,
        vec!["alarm-h1".to_string(), "alarm-h2".to_string()]
    );
    assert_eq!(herdr.delivered_count(), 2);
    assert_eq!(events.lock().unwrap().len(), 2);
}

#[test]
fn test_scheduler_signal_different_pane_ignored() {
    let base_time = UNIX_EPOCH + Duration::from_secs(30_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    let scheduler = setup_scheduler(clock.clone(), herdr.clone(), Arc::clone(&events));

    let timeout_at = base_time + Duration::from_secs(600);
    let alarm = create_herdr_alarm(
        "alarm-pane-a",
        "w1:p1",
        "pane-A",
        vec![HerdrWatchStatus::Done],
        base_time,
        timeout_at,
        "Notify",
    );
    scheduler.arm(alarm);

    let resolved = scheduler.signal(PushSignal::HerdrPaneEvent {
        pane_id: "pane-B".to_string(),
        new_status: HerdrWatchStatus::Done,
    });
    assert!(resolved.is_empty());
    assert_eq!(herdr.delivered_count(), 0);
    assert_eq!(events.lock().unwrap().len(), 0);
}

#[test]
fn test_scheduler_herdr_status_timeout_via_heap_pop() {
    let base_time = UNIX_EPOCH + Duration::from_secs(40_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    let scheduler = setup_scheduler(clock.clone(), herdr.clone(), Arc::clone(&events));

    let timeout_at = base_time + Duration::from_secs(60);
    let alarm = create_herdr_alarm(
        "alarm-h-timeout",
        "w1:p1",
        "w2:p2",
        vec![HerdrWatchStatus::Done],
        base_time,
        timeout_at,
        "Never delivered",
    );
    scheduler.arm(alarm);

    // Advance clock past timeout
    clock.advance(Duration::from_secs(61));

    // Wait briefly for the background thread to pop and process the timeout
    std::thread::sleep(Duration::from_millis(50));

    let evs = events.lock().unwrap();
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        FiredEvent::Failed { id, reason } => {
            assert_eq!(id, "alarm-h-timeout");
            assert_eq!(reason, "timeout");
        }
        other => panic!("expected Failed(timeout), got {other:?}"),
    }
    assert_eq!(herdr.delivered_count(), 0);
}

#[test]
fn test_arm_herdr_status_past_timeout_fails_immediately() {
    let base_time = UNIX_EPOCH + Duration::from_secs(50_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    let scheduler = setup_scheduler(clock.clone(), herdr.clone(), Arc::clone(&events));

    let past_timeout = base_time - Duration::from_secs(10);
    let alarm = create_herdr_alarm(
        "alarm-h-past-timeout",
        "w1:p1",
        "w2:p2",
        vec![HerdrWatchStatus::Done],
        base_time - Duration::from_secs(100),
        past_timeout,
        "Already timed out",
    );
    scheduler.arm(alarm);

    let evs = events.lock().unwrap();
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        FiredEvent::Failed { id, reason } => {
            assert_eq!(id, "alarm-h-past-timeout");
            assert_eq!(reason, "timeout");
        }
        other => panic!("expected Failed(timeout), got {other:?}"),
    }
    assert_eq!(herdr.delivered_count(), 0);
}

#[test]
fn test_scheduler_signal_external_matching() {
    let base_time = UNIX_EPOCH + Duration::from_secs(15_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    let scheduler = setup_scheduler(clock.clone(), herdr.clone(), Arc::clone(&events));

    let timeout_at = base_time + Duration::from_secs(300);
    let alarm = create_external_alarm(
        "alarm-ext-1",
        "w1:p1",
        "deploy-complete",
        base_time,
        timeout_at,
        "Deployment done after {duration}!",
    );

    scheduler.arm(alarm);

    // Advance time by 45 seconds
    clock.advance(Duration::from_secs(45));

    let resolved = scheduler.signal(PushSignal::ExternalSignal {
        match_key: "deploy-complete".to_string(),
    });

    assert_eq!(resolved, vec!["alarm-ext-1".to_string()]);
    assert_eq!(herdr.delivered_count(), 1);

    let delivered = herdr.last_delivered().expect("delivered alarm");
    match delivered.delivery {
        DeliveryMode::Prompt { text } => {
            assert_eq!(text, "Deployment done after 45s!");
        }
        _ => panic!("expected Prompt delivery"),
    }

    let evs = events.lock().unwrap();
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        FiredEvent::Fired { id, at } => {
            assert_eq!(id, "alarm-ext-1");
            assert_eq!(*at, base_time + Duration::from_secs(45));
        }
        other => panic!("expected FiredEvent::Fired, got {other:?}"),
    }
}

#[test]
fn test_scheduler_signal_external_broadcast() {
    let base_time = UNIX_EPOCH + Duration::from_secs(20_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    let scheduler = setup_scheduler(clock.clone(), herdr.clone(), Arc::clone(&events));

    let timeout_at = base_time + Duration::from_secs(600);
    let alarm1 = create_external_alarm(
        "alarm-b1",
        "w1:p1",
        "deploy-complete",
        base_time,
        timeout_at,
        "Notify pane 1",
    );
    let alarm2 = create_external_alarm(
        "alarm-b2",
        "w1:p2",
        "deploy-complete",
        base_time,
        timeout_at,
        "Notify pane 2",
    );

    scheduler.arm(alarm1);
    scheduler.arm(alarm2);

    let mut resolved = scheduler.signal(PushSignal::ExternalSignal {
        match_key: "deploy-complete".to_string(),
    });
    resolved.sort();

    assert_eq!(
        resolved,
        vec!["alarm-b1".to_string(), "alarm-b2".to_string()]
    );
    assert_eq!(herdr.delivered_count(), 2);
    assert_eq!(events.lock().unwrap().len(), 2);
}

#[test]
fn test_scheduler_signal_different_key_ignored() {
    let base_time = UNIX_EPOCH + Duration::from_secs(30_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    let scheduler = setup_scheduler(clock.clone(), herdr.clone(), Arc::clone(&events));

    let timeout_at = base_time + Duration::from_secs(600);
    let alarm = create_external_alarm(
        "alarm-ext-key",
        "w1:p1",
        "expected-key",
        base_time,
        timeout_at,
        "Notify",
    );
    scheduler.arm(alarm);

    let resolved = scheduler.signal(PushSignal::ExternalSignal {
        match_key: "other-key".to_string(),
    });
    assert!(resolved.is_empty());
    assert_eq!(herdr.delivered_count(), 0);
    assert_eq!(events.lock().unwrap().len(), 0);
}

#[test]
fn test_scheduler_external_signal_timeout_via_heap_pop() {
    let base_time = UNIX_EPOCH + Duration::from_secs(40_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    let scheduler = setup_scheduler(clock.clone(), herdr.clone(), Arc::clone(&events));

    let timeout_at = base_time + Duration::from_secs(60);
    let alarm = create_external_alarm(
        "alarm-ext-timeout",
        "w1:p1",
        "never-signaled",
        base_time,
        timeout_at,
        "Never delivered",
    );
    scheduler.arm(alarm);

    // Advance clock past timeout
    clock.advance(Duration::from_secs(61));

    // Wait briefly for the background thread to pop and process the timeout
    std::thread::sleep(Duration::from_millis(50));

    let evs = events.lock().unwrap();
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        FiredEvent::Failed { id, reason } => {
            assert_eq!(id, "alarm-ext-timeout");
            assert_eq!(reason, "timeout");
        }
        other => panic!("expected Failed(timeout), got {other:?}"),
    }
    assert_eq!(herdr.delivered_count(), 0);
}

#[test]
fn test_arm_external_signal_past_timeout_fails_immediately() {
    let base_time = UNIX_EPOCH + Duration::from_secs(50_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let events = Arc::new(Mutex::new(Vec::new()));

    let scheduler = setup_scheduler(clock.clone(), herdr.clone(), Arc::clone(&events));

    let past_timeout = base_time - Duration::from_secs(10);
    let alarm = create_external_alarm(
        "alarm-ext-past-timeout",
        "w1:p1",
        "my-key",
        base_time - Duration::from_secs(100),
        past_timeout,
        "Already timed out",
    );
    scheduler.arm(alarm);

    let evs = events.lock().unwrap();
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        FiredEvent::Failed { id, reason } => {
            assert_eq!(id, "alarm-ext-past-timeout");
            assert_eq!(reason, "timeout");
        }
        other => panic!("expected Failed(timeout), got {other:?}"),
    }
    assert_eq!(herdr.delivered_count(), 0);
}
