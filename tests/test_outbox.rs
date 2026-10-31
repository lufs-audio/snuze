#![allow(clippy::unwrap_used)]
#![allow(clippy::indexing_slicing)]

use snuze::daemon::outbox::{EventOutbox, OutboxEvent};
use snuze::kernel::{Alarm, AlarmStatus, DeliveryMode, DeliveryTarget, Trigger};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};
use tempfile::tempdir;

fn make_dummy_alarm(id: &str, trigger: Trigger) -> Alarm {
    Alarm {
        id: id.to_string(),
        target: DeliveryTarget::Herdr {
            pane_id: "w1:p1".to_string(),
        },
        created_by_pane: Some("w1:p0".to_string()),
        delivery: DeliveryMode::Prompt {
            text: "Wake up".to_string(),
        },
        label: Some("test label".to_string()),
        created_at: SystemTime::now(),
        deadline: SystemTime::now(),
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger,
        schema_version: 1,
    }
}

#[test]
fn test_outbox_round_trip() {
    let dir = tempdir().unwrap();
    let outbox = EventOutbox::open(dir.path()).unwrap();

    let a1 = make_dummy_alarm("alarm-1", Trigger::Deadline);
    let event1 = OutboxEvent::from_terminal_alarm(&a1, AlarmStatus::Fired);
    assert_eq!(event1.schema_version, 1);
    assert_eq!(event1.tool, "snuze");
    assert_eq!(event1.alarm_id, "alarm-1");
    assert_eq!(event1.trigger_kind, "deadline");
    assert_eq!(event1.status, "fired");
    assert_eq!(event1.delivery_target_pane, "w1:p1");
    assert!(event1.artifact_refs.is_empty());

    let a2 = make_dummy_alarm("alarm-2", Trigger::Deadline);
    let event2 = OutboxEvent::from_terminal_alarm(&a2, AlarmStatus::Failed);
    assert_eq!(event2.status, "failed");

    outbox.append(&event1).unwrap();
    outbox.append(&event2).unwrap();

    let drained = outbox.drain().unwrap();
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].alarm_id, "alarm-1");
    assert_eq!(drained[0].status, "fired");
    assert_eq!(drained[1].alarm_id, "alarm-2");
    assert_eq!(drained[1].status, "failed");

    // Consecutive drain on drained file should be empty
    let empty = outbox.drain().unwrap();
    assert!(empty.is_empty());
}

#[test]
fn test_outbox_drain_nonexistent_file() {
    let dir = tempdir().unwrap();
    let outbox = EventOutbox::open(dir.path()).unwrap();

    // Remove file if created
    let file_path = dir.path().join("events.ndjson");
    if file_path.exists() {
        fs::remove_file(&file_path).unwrap();
    }

    let drained = outbox.drain().unwrap();
    assert!(drained.is_empty());
}

#[test]
fn test_outbox_skip_truncated_trailing_line() {
    let dir = tempdir().unwrap();
    let outbox = EventOutbox::open(dir.path()).unwrap();

    let a1 = make_dummy_alarm("alarm-1", Trigger::Deadline);
    let event1 = OutboxEvent::from_terminal_alarm(&a1, AlarmStatus::Fired);
    outbox.append(&event1).unwrap();

    // Simulate crash mid-write producing corrupt / truncated trailing lines
    let file_path = dir.path().join("events.ndjson");
    let mut file = OpenOptions::new().append(true).open(&file_path).unwrap();
    writeln!(file, "{{\"schema_version\":1,\"emitted_at\":").unwrap();
    writeln!(file, "not a valid json line at all").unwrap();
    file.sync_all().unwrap();
    drop(file);

    let a2 = make_dummy_alarm("alarm-2", Trigger::Deadline);
    let event2 = OutboxEvent::from_terminal_alarm(&a2, AlarmStatus::Canceled);
    outbox.append(&event2).unwrap();

    let drained = outbox.drain().unwrap();
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].alarm_id, "alarm-1");
    assert_eq!(drained[1].alarm_id, "alarm-2");
}

#[test]
fn test_outbox_concurrent_append_and_drain() {
    let dir = tempdir().unwrap();
    let outbox = Arc::new(EventOutbox::open(dir.path()).unwrap());

    let total_appends = 50;
    let mut handles = Vec::new();

    for i in 0..5 {
        let outbox_clone = Arc::clone(&outbox);
        handles.push(thread::spawn(move || {
            for j in 0..10 {
                let id = format!("alarm-{i}-{j}");
                let alarm = make_dummy_alarm(&id, Trigger::Deadline);
                let event = OutboxEvent::from_terminal_alarm(&alarm, AlarmStatus::Fired);
                outbox_clone.append(&event).unwrap();
                thread::sleep(Duration::from_millis(5));
            }
        }));
    }

    let collected_count = Arc::new(AtomicUsize::new(0));
    let outbox_drain_clone = Arc::clone(&outbox);
    let collected_count_clone = Arc::clone(&collected_count);

    let drain_handle = thread::spawn(move || {
        for _ in 0..10 {
            if let Ok(batch) = outbox_drain_clone.drain() {
                collected_count_clone.fetch_add(batch.len(), Ordering::SeqCst);
            }
            thread::sleep(Duration::from_millis(15));
        }
    });

    for h in handles {
        h.join().unwrap();
    }
    drain_handle.join().unwrap();

    // Final drain to pick up any leftovers
    let final_batch = outbox.drain().unwrap();
    collected_count.fetch_add(final_batch.len(), Ordering::SeqCst);

    assert_eq!(collected_count.load(Ordering::SeqCst), total_appends);
}
