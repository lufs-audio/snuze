#![allow(clippy::unwrap_used)]
#![allow(clippy::indexing_slicing)]

use snuze::daemon::{Daemon, DaemonClient};
use snuze::kernel::{Alarm, AlarmStatus, DeliveryMode, JulesCondition, SnuzeError, Trigger};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

fn setup_fake_herdr() {
    let fake_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_herdr.sh");
    std::env::set_var("FAKE_HERDR_PATH", fake_path.to_str().unwrap());
    std::env::set_var("FAKE_HERDR_BEHAVIOR", "delivered");
}

fn create_test_alarm(id: &str, trigger: Trigger, deadline: SystemTime) -> Alarm {
    let now = SystemTime::now();
    Alarm {
        id: id.to_string(),
        pane_id: "w1:p1".to_string(),
        created_by_pane: Some("w1:p0".to_string()),
        delivery: DeliveryMode::Prompt {
            text: "Wake up agent".to_string(),
        },
        label: Some("test alarm".to_string()),
        created_at: now,
        deadline,
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger,
        schema_version: 1,
    }
}

fn spawn_daemon(state_dir: PathBuf) -> thread::JoinHandle<Result<(), SnuzeError>> {
    setup_fake_herdr();
    thread::spawn(move || Daemon::run_foreground(&state_dir))
}

fn wait_for_daemon(state_dir: &std::path::Path) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        if Daemon::is_running(state_dir) {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
fn test_outbox_emission_fired_and_canceled_call_sites() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    let handle = spawn_daemon(state_dir.clone());
    assert!(wait_for_daemon(&state_dir), "daemon failed to start");

    let client = DaemonClient::connect(&state_dir)?;
    client.ping()?;

    // 1. Arm an alarm that will fire immediately (past deadline)
    let fired_alarm_id = format!("fired-{}", Uuid::now_v7());
    let fired_alarm = create_test_alarm(
        &fired_alarm_id,
        Trigger::Deadline,
        SystemTime::now() - Duration::from_secs(1),
    );
    client.set_alarm(fired_alarm)?;

    // Wait for scheduler to pop and deliver the fired alarm
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        if let Ok(alarm) = client.get_alarm(&fired_alarm_id) {
            if alarm.status == AlarmStatus::Fired {
                break;
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    // 2. Arm a pending alarm and cancel it explicitly to test the CancelAlarm call site
    let canceled_alarm_id = format!("canceled-{}", Uuid::now_v7());
    let canceled_alarm = create_test_alarm(
        &canceled_alarm_id,
        Trigger::Deadline,
        SystemTime::now() + Duration::from_mins(1),
    );
    client.set_alarm(canceled_alarm)?;
    let cancel_res = client.cancel_alarm(&canceled_alarm_id)?;
    assert!(cancel_res, "alarm should be canceled successfully");

    // 3. Drain events and verify both events were recorded in the outbox
    let events = client.drain_events()?;
    assert_eq!(
        events.len(),
        2,
        "Expected 2 outbox events, got {}: {:?}",
        events.len(),
        events
    );

    let fired_event = events.iter().find(|e| e.alarm_id == fired_alarm_id);
    assert!(
        fired_event.is_some(),
        "Fired alarm event missing from outbox"
    );
    let fired_event = fired_event.unwrap();
    assert_eq!(fired_event.status, "fired");
    assert_eq!(fired_event.trigger_kind, "deadline");
    assert_eq!(fired_event.tool, "snuze");
    assert_eq!(fired_event.delivery_target_pane, "w1:p1");

    let canceled_event = events.iter().find(|e| e.alarm_id == canceled_alarm_id);
    assert!(
        canceled_event.is_some(),
        "Canceled alarm event missing from outbox"
    );
    let canceled_event = canceled_event.unwrap();
    assert_eq!(canceled_event.status, "canceled");
    assert_eq!(canceled_event.trigger_kind, "deadline");

    // 4. Second drain should be empty
    let empty_events = client.drain_events()?;
    assert!(empty_events.is_empty());

    Daemon::stop(&state_dir)?;
    let _ = handle.join();
    Ok(())
}

#[test]
fn test_outbox_progress_tick_not_emitted() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    let handle = spawn_daemon(state_dir.clone());
    assert!(wait_for_daemon(&state_dir), "daemon failed to start");

    let client = DaemonClient::connect(&state_dir)?;
    client.ping()?;

    // Create a JulesCondition alarm with a poll interval and long timeout
    let jules_alarm_id = format!("jules-{}", Uuid::now_v7());
    let jules_alarm = create_test_alarm(
        &jules_alarm_id,
        Trigger::JulesCondition {
            session_ids: vec!["nonexistent-session".to_string()],
            condition: JulesCondition::AllTerminal,
            poll_interval_secs: 1,
            timeout_at: SystemTime::now() + Duration::from_mins(1),
            last_known: vec![],
        },
        SystemTime::now() + Duration::from_secs(1),
    );
    client.set_alarm(jules_alarm)?;

    // Give scheduler time to poll (mock adapter might fail or progress)
    thread::sleep(Duration::from_millis(500));

    // Drain events - pending / progress events must never be in outbox
    let events = client.drain_events()?;
    for ev in &events {
        assert_ne!(
            ev.status, "progress",
            "Progress ticks must never produce an outbox event"
        );
        assert_ne!(
            ev.status, "pending",
            "Pending states must never produce an outbox event"
        );
    }

    Daemon::stop(&state_dir)?;
    let _ = handle.join();
    Ok(())
}
