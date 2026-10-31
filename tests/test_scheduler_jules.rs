#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]
#![allow(clippy::duration_suboptimal_units)]
#![allow(clippy::match_wildcard_for_single_variants)]
#![allow(clippy::too_many_arguments)]

use snuze::jules::MockJulesAdapter;
use snuze::kernel::{
    Alarm, AlarmStatus, DeliveryMode, JulesCondition, JulesSessionState, JulesState, Trigger,
};
use snuze::scheduler::{DeliveryOutcome, FakeClock, FiredEvent, HerdrAdapter, Scheduler};

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

#[derive(Clone, Default)]
struct MockHerdrAdapter {
    outcomes: Arc<Mutex<HashMap<String, DeliveryOutcome>>>,
    delivered_alarms: Arc<Mutex<Vec<Alarm>>>,
}

impl MockHerdrAdapter {
    fn new() -> Self {
        Self::default()
    }

    fn delivered_count(&self) -> usize {
        self.delivered_alarms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    fn last_delivered(&self) -> Option<Alarm> {
        self.delivered_alarms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .cloned()
    }
}

impl HerdrAdapter for MockHerdrAdapter {
    fn deliver(&self, alarm: &Alarm) -> DeliveryOutcome {
        self.delivered_alarms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(alarm.clone());

        self.outcomes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&alarm.id)
            .unwrap_or(DeliveryOutcome::Delivered)
    }
}

fn create_jules_alarm(
    id: &str,
    created_at: SystemTime,
    next_check: SystemTime,
    session_ids: Vec<String>,
    condition: JulesCondition,
    poll_interval_secs: u64,
    timeout_at: SystemTime,
    prompt_template: &str,
) -> Alarm {
    Alarm {
        id: id.to_string(),
        pane_id: "w1:p1".to_string(),
        created_by_pane: Some("w1:p0".to_string()),
        delivery: DeliveryMode::Prompt {
            text: prompt_template.to_string(),
        },
        label: Some("jules alarm".to_string()),
        created_at,
        deadline: next_check,
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::JulesCondition {
            session_ids,
            condition,
            poll_interval_secs,
            timeout_at,
            last_known: Vec::new(),
        },
        schema_version: 2,
    }
}

fn setup_scheduler(
    clock: FakeClock,
    herdr_adapter: MockHerdrAdapter,
    jules_adapter: MockJulesAdapter,
    grace_window: Duration,
) -> (Arc<Scheduler>, Receiver<FiredEvent>) {
    let (tx, rx): (Sender<FiredEvent>, Receiver<FiredEvent>) = channel();
    let scheduler = Arc::new(Scheduler::new(
        Box::new(herdr_adapter),
        Box::new(jules_adapter),
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
fn test_jules_condition_progress_then_complete() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();

    let in_progress_states = vec![JulesSessionState {
        session_id: "sess-123".to_string(),
        state: JulesState::InProgress,
        pull_request_url: None,
    }];
    let jules = MockJulesAdapter::new(in_progress_states);

    let (scheduler, rx) = setup_scheduler(
        clock.clone(),
        herdr.clone(),
        jules.clone(),
        Duration::from_mins(10),
    );

    let alarm = create_jules_alarm(
        "alarm-jules-1",
        base_time,
        base_time,
        vec!["sess-123".to_string()],
        JulesCondition::AnyCompleted,
        30,
        base_time + Duration::from_secs(300),
        "Session {session_id} is {status} after {duration}. PR: {pr_urls}",
    );
    scheduler.arm(alarm);

    // Poll 1: at t=0 - InProgress, not satisfied
    let ev1 = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Progress 1");
    match ev1 {
        FiredEvent::Progress {
            id,
            last_known,
            next_check_at,
        } => {
            assert_eq!(id, "alarm-jules-1");
            assert_eq!(last_known[0].state, JulesState::InProgress);
            assert_eq!(next_check_at, base_time + Duration::from_secs(30));
        }
        other => panic!("expected Progress, got {other:?}"),
    }
    assert_eq!(jules.call_count(), 1);
    assert_eq!(herdr.delivered_count(), 0);

    // Poll 2: advance 30s - still InProgress
    clock.advance(Duration::from_secs(30));
    let ev2 = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Progress 2");
    match ev2 {
        FiredEvent::Progress {
            id,
            last_known,
            next_check_at,
        } => {
            assert_eq!(id, "alarm-jules-1");
            assert_eq!(last_known[0].state, JulesState::InProgress);
            assert_eq!(next_check_at, base_time + Duration::from_secs(60));
        }
        other => panic!("expected Progress, got {other:?}"),
    }
    assert_eq!(jules.call_count(), 2);
    assert_eq!(herdr.delivered_count(), 0);

    // Update mock to Completed before Poll 3
    jules.set_response(vec![JulesSessionState {
        session_id: "sess-123".to_string(),
        state: JulesState::Completed,
        pull_request_url: Some("https://github.com/org/repo/pull/42".to_string()),
    }]);

    // Poll 3: advance 30s - now Completed, AnyCompleted satisfied
    clock.advance(Duration::from_secs(30));
    let ev3 = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Fired event");
    match ev3 {
        FiredEvent::Fired { id, at } => {
            assert_eq!(id, "alarm-jules-1");
            assert_eq!(at, base_time + Duration::from_secs(60));
        }
        other => panic!("expected Fired, got {other:?}"),
    }
    assert_eq!(jules.call_count(), 3);
    assert_eq!(herdr.delivered_count(), 1);

    let delivered = herdr.last_delivered().expect("delivered alarm exists");
    match delivered.delivery {
        DeliveryMode::Prompt { text } => {
            assert_eq!(
                text,
                "Session sess-123 is COMPLETED after 1m. PR: https://github.com/org/repo/pull/42"
            );
        }
        _ => panic!("expected Prompt delivery"),
    }
}

#[test]
fn test_jules_condition_any_terminal_fires_first() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(20_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();

    // Session 1 is InProgress (not wake-worthy), Session 2 is Failed (wake-worthy)
    let states = vec![
        JulesSessionState {
            session_id: "sess-a".to_string(),
            state: JulesState::InProgress,
            pull_request_url: None,
        },
        JulesSessionState {
            session_id: "sess-b".to_string(),
            state: JulesState::Failed,
            pull_request_url: None,
        },
    ];
    let jules = MockJulesAdapter::new(states);

    let (scheduler, rx) = setup_scheduler(
        clock.clone(),
        herdr.clone(),
        jules.clone(),
        Duration::from_mins(10),
    );

    let alarm = create_jules_alarm(
        "alarm-any-term",
        base_time,
        base_time,
        vec!["sess-a".to_string(), "sess-b".to_string()],
        JulesCondition::AnyTerminal,
        30,
        base_time + Duration::from_secs(300),
        "Result: {session_id} status: {status}",
    );
    scheduler.arm(alarm);

    let event = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Fired event");
    match event {
        FiredEvent::Fired { id, .. } => assert_eq!(id, "alarm-any-term"),
        other => panic!("expected Fired, got {other:?}"),
    }

    assert_eq!(jules.call_count(), 1);
    assert_eq!(
        jules.call_args(0),
        Some(vec!["sess-a".to_string(), "sess-b".to_string()])
    );
    assert_eq!(herdr.delivered_count(), 1);

    let delivered = herdr.last_delivered().expect("delivered alarm");
    match delivered.delivery {
        DeliveryMode::Prompt { text } => {
            assert_eq!(
                text,
                "Result: sess-a,sess-b status: sess-a:IN_PROGRESS,sess-b:FAILED"
            );
        }
        _ => panic!("expected Prompt delivery"),
    }
}

#[test]
fn test_jules_condition_all_terminal_waits_for_all() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(30_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();

    let initial_states = vec![
        JulesSessionState {
            session_id: "sess-1".to_string(),
            state: JulesState::Completed,
            pull_request_url: Some("https://github.com/org/repo/pull/1".to_string()),
        },
        JulesSessionState {
            session_id: "sess-2".to_string(),
            state: JulesState::InProgress,
            pull_request_url: None,
        },
    ];
    let jules = MockJulesAdapter::new(initial_states);

    let (scheduler, rx) = setup_scheduler(
        clock.clone(),
        herdr.clone(),
        jules.clone(),
        Duration::from_mins(10),
    );

    let alarm = create_jules_alarm(
        "alarm-all-term",
        base_time,
        base_time,
        vec!["sess-1".to_string(), "sess-2".to_string()],
        JulesCondition::AllTerminal,
        30,
        base_time + Duration::from_secs(300),
        "All done: {session_id} - {status} - PRs: {pr_urls}",
    );
    scheduler.arm(alarm);

    // Poll 1 at t=0
    let _ = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Progress 1");
    assert_eq!(herdr.delivered_count(), 0);

    // Advance past several polls while session 2 remains InProgress
    for _ in 0..4 {
        clock.advance(Duration::from_secs(30));
        let _ = rx
            .recv_timeout(Duration::from_millis(500))
            .expect("expected Progress");
        assert_eq!(herdr.delivered_count(), 0);
    }
    assert_eq!(jules.call_count(), 5);

    // Now session 2 also completes
    jules.set_response(vec![
        JulesSessionState {
            session_id: "sess-1".to_string(),
            state: JulesState::Completed,
            pull_request_url: Some("https://github.com/org/repo/pull/1".to_string()),
        },
        JulesSessionState {
            session_id: "sess-2".to_string(),
            state: JulesState::Completed,
            pull_request_url: Some("https://github.com/org/repo/pull/2".to_string()),
        },
    ]);

    // Next poll fires
    clock.advance(Duration::from_secs(30));
    let fired_ev = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Fired event");
    match fired_ev {
        FiredEvent::Fired { id, .. } => assert_eq!(id, "alarm-all-term"),
        other => panic!("expected Fired, got {other:?}"),
    }
    assert_eq!(herdr.delivered_count(), 1);

    let delivered = herdr.last_delivered().expect("delivered alarm");
    match delivered.delivery {
        DeliveryMode::Prompt { text } => {
            assert_eq!(
                text,
                "All done: sess-1,sess-2 - sess-1:COMPLETED,sess-2:COMPLETED - PRs: https://github.com/org/repo/pull/1,https://github.com/org/repo/pull/2"
            );
        }
        _ => panic!("expected Prompt delivery"),
    }
}

#[test]
fn test_jules_condition_timeout_fires_failed_timeout() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(40_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();

    let states = vec![JulesSessionState {
        session_id: "sess-never".to_string(),
        state: JulesState::InProgress,
        pull_request_url: None,
    }];
    let jules = MockJulesAdapter::new(states);

    let (scheduler, rx) = setup_scheduler(
        clock.clone(),
        herdr.clone(),
        jules.clone(),
        Duration::from_mins(10),
    );

    let timeout_at = base_time + Duration::from_secs(60);
    let alarm = create_jules_alarm(
        "alarm-timeout",
        base_time,
        base_time,
        vec!["sess-never".to_string()],
        JulesCondition::AnyCompleted,
        30,
        timeout_at,
        "Will not deliver",
    );
    scheduler.arm(alarm);

    // Poll 1 at t=0
    let _ = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Progress 1");

    // Poll 2 at t=30
    clock.advance(Duration::from_secs(30));
    let _ = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Progress 2");

    // Poll 3 at t=60 (hits timeout_at)
    clock.advance(Duration::from_secs(30));
    let ev = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Failed event");
    match ev {
        FiredEvent::Failed { id, reason } => {
            assert_eq!(id, "alarm-timeout");
            assert_eq!(reason, "timeout");
        }
        other => panic!("expected Failed(timeout), got {other:?}"),
    }
    assert_eq!(herdr.delivered_count(), 0, "never delivered on timeout");
}

#[test]
fn test_jules_condition_transient_error_recovers() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(50_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();

    let jules = MockJulesAdapter::new(vec![JulesSessionState {
        session_id: "sess-transient".to_string(),
        state: JulesState::InProgress,
        pull_request_url: None,
    }]);

    let (scheduler, rx) = setup_scheduler(
        clock.clone(),
        herdr.clone(),
        jules.clone(),
        Duration::from_mins(10),
    );

    let alarm = create_jules_alarm(
        "alarm-transient",
        base_time,
        base_time,
        vec!["sess-transient".to_string()],
        JulesCondition::AnyCompleted,
        30,
        base_time + Duration::from_secs(300),
        "Session done: {session_id} ({status})",
    );
    scheduler.arm(alarm);

    // Poll 1 at t=0 -> Progress
    let _ = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Progress 1");

    // Simulate transient network outage on next poll
    jules.set_failure("temporary 503 backend error");
    clock.advance(Duration::from_secs(30));

    // Should emit Progress (not terminal failure) since timeout hasn't been reached
    let ev_transient = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Progress during blip");
    match ev_transient {
        FiredEvent::Progress { id, .. } => {
            assert_eq!(id, "alarm-transient");
        }
        other => panic!("expected Progress during blip, got {other:?}"),
    }

    // Network recovers and session completes
    jules.set_response(vec![JulesSessionState {
        session_id: "sess-transient".to_string(),
        state: JulesState::Completed,
        pull_request_url: None,
    }]);

    clock.advance(Duration::from_secs(30));
    let fired_ev = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Fired event");
    match fired_ev {
        FiredEvent::Fired { id, .. } => assert_eq!(id, "alarm-transient"),
        other => panic!("expected Fired event, got {other:?}"),
    }
    assert_eq!(herdr.delivered_count(), 1);
}

#[test]
fn test_arm_past_timeout_fires_failed_timeout_immediately() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(60_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let jules = MockJulesAdapter::new(vec![]);

    let (scheduler, rx) =
        setup_scheduler(clock, herdr.clone(), jules.clone(), Duration::from_mins(10));

    // timeout_at is in the past relative to base_time
    let past_timeout = base_time - Duration::from_secs(10);
    let alarm = create_jules_alarm(
        "alarm-already-timed-out",
        base_time - Duration::from_secs(100),
        base_time,
        vec!["sess-1".to_string()],
        JulesCondition::AnyTerminal,
        30,
        past_timeout,
        "text",
    );
    scheduler.arm(alarm);

    let ev = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Failed event");
    match ev {
        FiredEvent::Failed { id, reason } => {
            assert_eq!(id, "alarm-already-timed-out");
            assert_eq!(reason, "timeout");
        }
        other => panic!("expected Failed(timeout), got {other:?}"),
    }
    assert_eq!(jules.call_count(), 0);
    assert_eq!(herdr.delivered_count(), 0);
}

#[test]
fn test_deadline_prompt_without_tokens_interpolates_unchanged() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(70_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let jules = MockJulesAdapter::new(vec![]);

    let (scheduler, rx) =
        setup_scheduler(clock.clone(), herdr.clone(), jules, Duration::from_mins(10));

    let alarm = Alarm {
        id: "plain-alarm".to_string(),
        pane_id: "w1:p1".to_string(),
        created_by_pane: None,
        delivery: DeliveryMode::Prompt {
            text: "Fixed text with no variables and {unrecognized_custom_token}".to_string(),
        },
        label: None,
        created_at: base_time,
        deadline: base_time + Duration::from_secs(10),
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::Deadline,
        schema_version: 1,
    };
    scheduler.arm(alarm);

    clock.advance(Duration::from_secs(10));
    let _ = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Fired");

    let delivered = herdr.last_delivered().expect("delivered alarm");
    match delivered.delivery {
        DeliveryMode::Prompt { text } => {
            assert_eq!(
                text,
                "Fixed text with no variables and {unrecognized_custom_token}"
            );
        }
        _ => panic!("expected Prompt delivery"),
    }
}

#[test]
fn test_deadline_prompt_duration_interpolation() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(80_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let jules = MockJulesAdapter::new(vec![]);

    let (scheduler, rx) =
        setup_scheduler(clock.clone(), herdr.clone(), jules, Duration::from_mins(10));

    let alarm = Alarm {
        id: "duration-alarm".to_string(),
        pane_id: "w1:p1".to_string(),
        created_by_pane: None,
        delivery: DeliveryMode::Prompt {
            text: "Alarm fired after {duration}!".to_string(),
        },
        label: None,
        created_at: base_time,
        deadline: base_time + Duration::from_secs(754),
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::Deadline,
        schema_version: 1,
    };
    scheduler.arm(alarm);

    clock.advance(Duration::from_secs(754));
    let _ = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Fired");

    let delivered = herdr.last_delivered().expect("delivered alarm");
    match delivered.delivery {
        DeliveryMode::Prompt { text } => {
            assert_eq!(text, "Alarm fired after 12m34s!");
        }
        _ => panic!("expected Prompt delivery"),
    }
}

#[test]
fn test_jules_credential_missing_fails_immediately_without_waiting_for_timeout() {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(90_000);
    let clock = FakeClock::new(base_time);
    let herdr = MockHerdrAdapter::new();
    let jules = MockJulesAdapter::new(vec![]);
    jules.set_error(snuze::kernel::SnuzeError::JulesCredentialMissing);

    let (scheduler, rx) = setup_scheduler(
        clock.clone(),
        herdr.clone(),
        jules.clone(),
        Duration::from_mins(10),
    );

    let alarm = create_jules_alarm(
        "cred-missing-alarm",
        base_time,
        base_time + Duration::from_secs(10),
        vec!["sess-1".to_string()],
        JulesCondition::AnyTerminal,
        30,
        base_time + Duration::from_secs(3600), // 1 hour timeout
        "Wake up",
    );
    scheduler.arm(alarm);

    clock.advance(Duration::from_secs(10));

    let ev = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("expected Failed event immediately");
    match ev {
        FiredEvent::Failed { id, reason } => {
            assert_eq!(id, "cred-missing-alarm");
            assert_eq!(reason, "JULES_API_KEY not set in environment");
        }
        other => panic!("expected Failed(JulesCredentialMissing), got {other:?}"),
    }

    assert_eq!(jules.call_count(), 1);
    assert_eq!(herdr.delivered_count(), 0);
}
