#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::duration_suboptimal_units)]

use std::fs;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

use snuze::kernel::{Alarm, AlarmStatus, DeliveryMode, SnuzeError, Trigger};
use snuze::store::{resolve_state_dir, AlarmStore};

fn make_alarm(id: &str, deadline_offset_secs: u64, status: AlarmStatus) -> Alarm {
    let now = SystemTime::now();
    Alarm {
        id: id.to_string(),
        pane_id: "w1:p1".to_string(),
        created_by_pane: Some("w1:p0".to_string()),
        delivery: DeliveryMode::Prompt {
            text: format!("wake up {id}"),
        },
        label: Some(format!("alarm-{id}")),
        created_at: now,
        deadline: now + Duration::from_secs(deadline_offset_secs),
        status,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::Deadline,
        schema_version: 1,
    }
}

#[test]
fn test_open_creates_directory_and_idempotent() {
    let temp_dir = TempDir::new().unwrap();
    let store_path = temp_dir.path().join("state_sub");

    // Directory doesn't exist yet
    assert!(!store_path.join("alarms").exists());
    let store = AlarmStore::open(&store_path).unwrap();
    assert!(store_path.join("alarms").is_dir());

    // Second open should succeed on existing directory
    let store2 = AlarmStore::open(&store_path).unwrap();
    let alarm = make_alarm("a1", 10, AlarmStatus::Pending);
    store.put(&alarm).unwrap();

    let fetched = store2.get("a1").unwrap();
    assert_eq!(fetched.id, "a1");
}

#[test]
fn test_put_then_get_round_trip() {
    let temp_dir = TempDir::new().unwrap();
    let store = AlarmStore::open(temp_dir.path()).unwrap();

    let alarm = make_alarm("roundtrip-1", 60, AlarmStatus::Pending);
    store.put(&alarm).unwrap();

    let retrieved = store.get("roundtrip-1").unwrap();
    assert_eq!(retrieved.id, alarm.id);
    assert_eq!(retrieved.pane_id, alarm.pane_id);
    assert_eq!(retrieved.created_by_pane, alarm.created_by_pane);
    assert_eq!(retrieved.delivery, alarm.delivery);
    assert_eq!(retrieved.label, alarm.label);
    assert_eq!(retrieved.status, AlarmStatus::Pending);
    assert_eq!(retrieved.fired_at, None);
    assert_eq!(retrieved.failure_reason, None);
}

#[test]
fn test_get_not_found() {
    let temp_dir = TempDir::new().unwrap();
    let store = AlarmStore::open(temp_dir.path()).unwrap();

    let result = store.get("nonexistent");
    assert!(matches!(result, Err(SnuzeError::AlarmNotFound(id)) if id == "nonexistent"));
}

#[test]
fn test_get_corrupt_file_returns_persist_failed() {
    let temp_dir = TempDir::new().unwrap();
    let store = AlarmStore::open(temp_dir.path()).unwrap();

    let corrupt_file = temp_dir.path().join("alarms").join("bad_alarm.json");
    fs::write(&corrupt_file, b"this is not valid json").unwrap();

    let result = store.get("bad_alarm");
    assert!(matches!(result, Err(SnuzeError::PersistFailed(_))));
}

#[test]
fn test_list_sort_order_and_filter() {
    let temp_dir = TempDir::new().unwrap();
    let store = AlarmStore::open(temp_dir.path()).unwrap();

    let a1 = make_alarm("a1", 100, AlarmStatus::Pending);
    let a2 = make_alarm("a2", 20, AlarmStatus::Fired);
    let a3 = make_alarm("a3", 50, AlarmStatus::Pending);
    let a4 = make_alarm("a4", 10, AlarmStatus::Canceled);

    store.put(&a1).unwrap();
    store.put(&a2).unwrap();
    store.put(&a3).unwrap();
    store.put(&a4).unwrap();

    // List all: sorted ascending by deadline
    let all = store.list(None).unwrap();
    assert_eq!(all.len(), 4);
    assert_eq!(all[0].id, "a4"); // 10s
    assert_eq!(all[1].id, "a2"); // 20s
    assert_eq!(all[2].id, "a3"); // 50s
    assert_eq!(all[3].id, "a1"); // 100s

    // Filter by Pending: only a3 and a1, in ascending deadline order
    let pending = store.list(Some(AlarmStatus::Pending)).unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].id, "a3");
    assert_eq!(pending[1].id, "a1");

    // Filter by Fired
    let fired = store.list(Some(AlarmStatus::Fired)).unwrap();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].id, "a2");
}

#[test]
fn test_list_survives_corrupted_file_and_scan_errors_reports_it() {
    let temp_dir = TempDir::new().unwrap();
    let store = AlarmStore::open(temp_dir.path()).unwrap();

    let good1 = make_alarm("good1", 30, AlarmStatus::Pending);
    let good2 = make_alarm("good2", 60, AlarmStatus::Pending);
    store.put(&good1).unwrap();
    store.put(&good2).unwrap();

    // Copy the corrupt fixture into alarms/
    let corrupt_fixture_path = "tests/fixtures/corrupt_alarm.json";
    let dest_corrupt = temp_dir.path().join("alarms").join("corrupt_alarm.json");
    fs::copy(corrupt_fixture_path, &dest_corrupt).unwrap();

    // Also write a totally garbage byte file
    let dest_garbage = temp_dir.path().join("alarms").join("garbage.json");
    fs::write(&dest_garbage, b"{ corrupt byte sequence \xFF\xFE }").unwrap();

    // List should return only valid alarms
    let alarms = store.list(None).unwrap();
    assert_eq!(alarms.len(), 2);
    assert_eq!(alarms[0].id, "good1");
    assert_eq!(alarms[1].id, "good2");

    // scan_errors should report both corrupted files
    let errors = store.scan_errors();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].0, "corrupt_alarm.json");
    assert_eq!(errors[1].0, "garbage.json");
}

#[test]
fn test_simulated_crash_mid_write_invisible_to_list_and_scan_errors() {
    let temp_dir = TempDir::new().unwrap();
    let store = AlarmStore::open(temp_dir.path()).unwrap();

    let good = make_alarm("good_record", 10, AlarmStatus::Pending);
    store.put(&good).unwrap();

    // Simulate an interrupted write leaving a temp file
    let tmp1 = temp_dir.path().join("alarms").join(".crashed_1.tmp.9999");
    fs::write(&tmp1, b"half-written json payload {").unwrap();

    let tmp2 = temp_dir.path().join("alarms").join("crashed_2.tmp");
    fs::write(&tmp2, b"another broken tmp file").unwrap();

    // List sees only the transacted record
    let alarms = store.list(None).unwrap();
    assert_eq!(alarms.len(), 1);
    assert_eq!(alarms[0].id, "good_record");

    // scan_errors ignores untransacted tmp files (they are not .json records)
    let errors = store.scan_errors();
    assert!(errors.is_empty());
}

#[test]
fn test_delete_removes_file_and_errors_on_missing() {
    let temp_dir = TempDir::new().unwrap();
    let store = AlarmStore::open(temp_dir.path()).unwrap();

    let a1 = make_alarm("to_delete", 30, AlarmStatus::Canceled);
    store.put(&a1).unwrap();

    assert!(store.get("to_delete").is_ok());
    store.delete("to_delete").unwrap();

    assert!(matches!(
        store.get("to_delete"),
        Err(SnuzeError::AlarmNotFound(_))
    ));

    // Second delete should error with AlarmNotFound
    let err = store.delete("to_delete");
    assert!(matches!(err, Err(SnuzeError::AlarmNotFound(_))));
}

#[test]
fn test_resolve_state_dir_xdg_precedence() {
    // When XDG_STATE_HOME is set
    std::env::set_var("XDG_STATE_HOME", "/tmp/custom_state");
    let state_dir = resolve_state_dir();
    assert_eq!(
        state_dir,
        std::path::PathBuf::from("/tmp/custom_state/snuze")
    );

    // When XDG_STATE_HOME is empty string
    std::env::set_var("XDG_STATE_HOME", "");
    let fallback_dir = resolve_state_dir();
    assert!(fallback_dir.ends_with(".local/state/snuze"));

    // Restore / remove test env var
    std::env::remove_var("XDG_STATE_HOME");
    let final_dir = resolve_state_dir();
    assert!(final_dir.ends_with(".local/state/snuze"));
}
