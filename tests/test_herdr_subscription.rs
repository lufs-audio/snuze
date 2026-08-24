#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]
#![allow(clippy::duration_suboptimal_units)]
#![allow(clippy::needless_raw_string_hashes)]

use snuze::herdr::HerdrEventListener;
use snuze::jules::JulesAdapter;
use snuze::kernel::{
    Alarm, AlarmStatus, DeliveryMode, HerdrWatchStatus, JulesSessionState, SnuzeError, Trigger,
};
use snuze::scheduler::{DeliveryOutcome, FakeClock, FiredEvent, HerdrAdapter, Scheduler};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

#[derive(Clone, Default)]
struct MockHerdrAdapter {
    delivered_alarms: Arc<Mutex<Vec<Alarm>>>,
}

impl HerdrAdapter for MockHerdrAdapter {
    fn deliver(&self, alarm: &Alarm) -> DeliveryOutcome {
        self.delivered_alarms.lock().unwrap().push(alarm.clone());
        DeliveryOutcome::Delivered
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
    target_pane: &str,
    watched_pane: &str,
    watch_for: Vec<HerdrWatchStatus>,
    timeout_at: SystemTime,
) -> Alarm {
    Alarm {
        id: id.to_string(),
        pane_id: target_pane.to_string(),
        created_by_pane: Some("w1:p0".to_string()),
        delivery: DeliveryMode::Prompt {
            text: "Agent finished!".to_string(),
        },
        label: Some(format!("label-{id}")),
        created_at: UNIX_EPOCH,
        deadline: timeout_at,
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::HerdrStatus {
            watched_pane_id: watched_pane.to_string(),
            watch_for,
            timeout_at,
            last_known: None,
        },
        schema_version: 3,
    }
}

#[test]
fn test_herdr_event_listener_end_to_end_socket_delivery() {
    let temp_dir = TempDir::new().unwrap();
    let sock_path = temp_dir.path().join("herdr_test.sock");

    let listener_sock = UnixListener::bind(&sock_path).unwrap();

    let base_time = UNIX_EPOCH + Duration::from_secs(10_000);
    let clock = FakeClock::new(base_time);
    let herdr_adapter = MockHerdrAdapter::default();
    let delivered = Arc::clone(&herdr_adapter.delivered_alarms);
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);

    let scheduler = Arc::new(Scheduler::new(
        Box::new(herdr_adapter),
        Box::new(MockJulesAdapter),
        Box::new(clock),
        Duration::from_mins(10),
        Box::new(move |ev| {
            events_clone.lock().unwrap().push(ev);
        }),
    ));

    let alarm = create_herdr_alarm(
        "alarm-listen-1",
        "w1:p1",
        "w2:p2",
        vec![HerdrWatchStatus::Done],
        base_time + Duration::from_secs(300),
    );
    scheduler.arm(alarm);

    let listener = HerdrEventListener::with_socket_path(Arc::clone(&scheduler), sock_path.clone());

    // Spawn listener run on a background thread
    let listener_handle = std::thread::spawn(move || {
        listener.run();
    });

    // Accept connection on fake herdr server
    let (mut stream, _) = listener_sock.accept().unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("events.subscribe"));

    // Reply with subscription_started
    let resp = serde_json::json!({
        "id": "snuze-sub",
        "result": { "type": "subscription_started" }
    });
    writeln!(stream, "{}", serde_json::to_string(&resp).unwrap()).unwrap();
    stream.flush().unwrap();

    // Send a non-matching event first (different pane)
    let non_matching_event = serde_json::json!({
        "event": "pane.agent_status_changed",
        "data": {
            "pane_id": "w9:p9",
            "agent_status": "done"
        }
    });
    writeln!(stream, "{}", serde_json::to_string(&non_matching_event).unwrap()).unwrap();
    stream.flush().unwrap();

    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(delivered.lock().unwrap().len(), 0);

    // Send matching event for w2:p2 -> done
    let matching_event = serde_json::json!({
        "event": "pane.agent_status_changed",
        "data": {
            "pane_id": "w2:p2",
            "agent_status": "done"
        }
    });
    writeln!(stream, "{}", serde_json::to_string(&matching_event).unwrap()).unwrap();
    stream.flush().unwrap();

    std::thread::sleep(Duration::from_millis(50));

    let del = delivered.lock().unwrap();
    assert_eq!(del.len(), 1);
    assert_eq!(del[0].id, "alarm-listen-1");

    let evs = events.lock().unwrap();
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        FiredEvent::Fired { id, .. } => assert_eq!(id, "alarm-listen-1"),
        other => panic!("expected Fired, got {other:?}"),
    }

    drop(listener_handle);
}

#[test]
fn test_herdr_event_listener_reconnection_resilience() {
    let temp_dir = TempDir::new().unwrap();
    let sock_path = temp_dir.path().join("herdr_reconnect_test.sock");

    let listener_sock = UnixListener::bind(&sock_path).unwrap();

    let base_time = UNIX_EPOCH + Duration::from_secs(20_000);
    let clock = FakeClock::new(base_time);
    let herdr_adapter = MockHerdrAdapter::default();
    let delivered = Arc::clone(&herdr_adapter.delivered_alarms);
    let events = Arc::new(Mutex::new(Vec::new()));
    let events_clone = Arc::clone(&events);

    let scheduler = Arc::new(Scheduler::new(
        Box::new(herdr_adapter),
        Box::new(MockJulesAdapter),
        Box::new(clock),
        Duration::from_mins(10),
        Box::new(move |ev| {
            events_clone.lock().unwrap().push(ev);
        }),
    ));

    let alarm = create_herdr_alarm(
        "alarm-reconnect",
        "w1:p1",
        "w4:p1",
        vec![HerdrWatchStatus::Idle],
        base_time + Duration::from_secs(300),
    );
    scheduler.arm(alarm);

    let listener = HerdrEventListener::with_socket_path(Arc::clone(&scheduler), sock_path.clone());

    let _listener_handle = std::thread::spawn(move || {
        listener.run();
    });

    // 1. First connection: accept and abruptly close
    {
        let (stream, _) = listener_sock.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        // Drop stream without sending matching event
    }

    // Alarm should not have fired during disconnect
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(delivered.lock().unwrap().len(), 0);
    assert_eq!(events.lock().unwrap().len(), 0);

    // 2. Second connection: listener should reconnect (after ~1s backoff)
    let (mut stream2, _) = listener_sock.accept().unwrap();
    let mut reader2 = BufReader::new(stream2.try_clone().unwrap());
    let mut line2 = String::new();
    reader2.read_line(&mut line2).unwrap();
    assert!(line2.contains("events.subscribe"));

    // Send matching event on re-established connection
    let event = serde_json::json!({
        "event": "pane.agent_status_changed",
        "data": {
            "pane_id": "w4:p1",
            "agent_status": "idle"
        }
    });
    writeln!(stream2, "{}", serde_json::to_string(&event).unwrap()).unwrap();
    stream2.flush().unwrap();

    std::thread::sleep(Duration::from_millis(50));

    let del = delivered.lock().unwrap();
    assert_eq!(del.len(), 1);
    assert_eq!(del[0].id, "alarm-reconnect");
}

#[test]
fn test_herdr_event_listener_unrecognized_status_ignored() {
    let scheduler = Arc::new(Scheduler::new(
        Box::new(MockHerdrAdapter::default()),
        Box::new(MockJulesAdapter),
        Box::new(FakeClock::new(UNIX_EPOCH)),
        Duration::from_mins(10),
        Box::new(|_| {}),
    ));

    let listener = HerdrEventListener::new(scheduler).unwrap();

    // Should not panic on unrecognized status or corrupted payload
    listener.handle_event_line(r#"{"event":"pane.agent_status_changed","data":{"pane_id":"w1:p1","agent_status":"future_unknown_state"}}"#);
    listener.handle_event_line(r#"not valid json at all"#);
    listener.handle_event_line(r#"{"event":"unknown_event_type","data":{}}"#);
}
