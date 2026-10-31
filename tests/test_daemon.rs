#![allow(clippy::unwrap_used)]
#![allow(clippy::indexing_slicing)]

use snuze::daemon::{Daemon, DaemonClient};
use snuze::kernel::{Alarm, AlarmStatus, DeliveryMode, SnuzeError, Trigger};
use snuze::store::AlarmStore;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

fn setup_fake_herdr() {
    let fake_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_herdr.sh");
    std::env::set_var("FAKE_HERDR_PATH", fake_path.to_str().unwrap());
    std::env::set_var("FAKE_HERDR_BEHAVIOR", "delivered");
}

fn create_test_alarm() -> Alarm {
    let now = SystemTime::now();
    Alarm {
        id: Uuid::now_v7().to_string(),
        pane_id: "w1:p1".to_string(),
        created_by_pane: Some("w1:p0".to_string()),
        delivery: DeliveryMode::Prompt {
            text: "Wake up agent".to_string(),
        },
        label: Some("test alarm".to_string()),
        created_at: now,
        deadline: now + Duration::from_hours(1),
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::Deadline,
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
fn test_daemon_lifecycle_and_crud() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    let handle = spawn_daemon(state_dir.clone());
    assert!(wait_for_daemon(&state_dir), "daemon failed to start");

    let client = DaemonClient::connect(&state_dir)?;
    client.ping()?;

    // 1. Set alarm (Durability before ACK)
    let alarm = create_test_alarm();
    client.set_alarm(alarm.clone())?;

    // 2. Get alarm
    let fetched = client.get_alarm(&alarm.id)?;
    assert_eq!(fetched.id, alarm.id);
    assert_eq!(fetched.pane_id, alarm.pane_id);
    assert_eq!(fetched.created_by_pane, alarm.created_by_pane);
    assert_eq!(fetched.status, AlarmStatus::Pending);

    // 3. List alarms
    let all_alarms = client.list_alarms(None)?;
    assert_eq!(all_alarms.len(), 1);
    assert_eq!(all_alarms[0].id, alarm.id);

    let pending_alarms = client.list_alarms(Some(AlarmStatus::Pending))?;
    assert_eq!(pending_alarms.len(), 1);

    let fired_alarms = client.list_alarms(Some(AlarmStatus::Fired))?;
    assert!(fired_alarms.is_empty());

    // 4. Cancel alarm
    let canceled = client.cancel_alarm(&alarm.id)?;
    assert!(canceled);

    let fetched_after_cancel = client.get_alarm(&alarm.id)?;
    assert_eq!(fetched_after_cancel.status, AlarmStatus::Canceled);

    // 5. Get non-existent alarm
    let non_existent = client.get_alarm("01HGWY37VZ8Q0000");
    assert!(matches!(non_existent, Err(SnuzeError::AlarmNotFound(_))));

    // 6. Stop daemon cleanly
    Daemon::stop(&state_dir)?;
    handle.join().unwrap()?;

    assert!(!Daemon::is_running(&state_dir));
    Ok(())
}

#[test]
fn test_daemon_single_instance_enforcement() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    let handle = spawn_daemon(state_dir.clone());
    assert!(wait_for_daemon(&state_dir), "daemon failed to start");

    // Second daemon startup against same state directory must fail cleanly
    let second_instance = Daemon::run_foreground(&state_dir);
    assert!(
        matches!(second_instance, Err(SnuzeError::DaemonUnavailable(_))),
        "Expected DaemonUnavailable for second daemon instance"
    );

    Daemon::stop(&state_dir)?;
    handle.join().unwrap()?;
    Ok(())
}

#[test]
fn test_daemon_stale_pid_recovery() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();
    std::fs::create_dir_all(&state_dir)?;

    // Simulate stale PID file left behind by crashed/killed daemon
    std::fs::write(state_dir.join("daemon.pid"), "99999999")?;

    let handle = spawn_daemon(state_dir.clone());
    assert!(
        wait_for_daemon(&state_dir),
        "daemon should start despite stale PID file"
    );

    Daemon::stop(&state_dir)?;
    handle.join().unwrap()?;
    Ok(())
}

#[test]
fn test_daemon_graceful_restart_and_rearm() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    // 1. Start daemon 1
    let handle1 = spawn_daemon(state_dir.clone());
    assert!(wait_for_daemon(&state_dir), "daemon 1 failed to start");

    let client = DaemonClient::connect(&state_dir)?;
    let alarm = create_test_alarm();
    client.set_alarm(alarm.clone())?;

    // 2. Stop daemon 1 (simulating graceful restart)
    Daemon::stop(&state_dir)?;
    handle1.join().unwrap()?;
    assert!(!Daemon::is_running(&state_dir));

    // 3. Start daemon 2 against same state dir — verify boot-time re-arm from durable store
    let handle2 = spawn_daemon(state_dir.clone());
    assert!(wait_for_daemon(&state_dir), "daemon 2 failed to restart");

    let client2 = DaemonClient::connect(&state_dir)?;
    let alarms = client2.list_alarms(None)?;
    assert_eq!(alarms.len(), 1);
    assert_eq!(alarms[0].id, alarm.id);
    assert_eq!(alarms[0].status, AlarmStatus::Pending);

    Daemon::stop(&state_dir)?;
    handle2.join().unwrap()?;
    Ok(())
}

#[test]
fn test_daemon_sigkill_fault_injection_and_rearm() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let xdg_home = temp_dir.path().to_path_buf();
    let state_dir = xdg_home.join("snuze");

    let fake_herdr = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_herdr.sh");

    // 1. Spawn daemon as a real external OS process
    let mut child1 = std::process::Command::new(env!("CARGO_BIN_EXE_snuze"))
        .env("XDG_STATE_HOME", &xdg_home)
        .env("FAKE_HERDR_PATH", &fake_herdr)
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("daemon")
        .arg("start")
        .arg("--foreground")
        .spawn()?;

    assert!(
        wait_for_daemon(&state_dir),
        "daemon child process failed to start"
    );

    let client = DaemonClient::connect(&state_dir)?;
    let alarm = create_test_alarm();
    client.set_alarm(alarm.clone())?;

    // 2. Inject uncooperative crash: send SIGKILL (Child::kill)
    child1.kill()?;
    let _ = child1.wait()?;

    // Verify daemon process is dead but left dirty disk state (PID file remains)
    assert!(
        state_dir.join("daemon.pid").exists(),
        "PID file should remain after SIGKILL"
    );

    // 3. Spawn replacement daemon process against the same state dir
    let mut child2 = std::process::Command::new(env!("CARGO_BIN_EXE_snuze"))
        .env("XDG_STATE_HOME", &xdg_home)
        .env("FAKE_HERDR_PATH", &fake_herdr)
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("daemon")
        .arg("start")
        .arg("--foreground")
        .spawn()?;

    assert!(
        wait_for_daemon(&state_dir),
        "replacement daemon failed to start after SIGKILL"
    );

    // 4. Verify crash-recovery re-arm on boot from durable store
    let client2 = DaemonClient::connect(&state_dir)?;
    let alarms = client2.list_alarms(None)?;
    assert_eq!(alarms.len(), 1);
    assert_eq!(alarms[0].id, alarm.id);
    assert_eq!(alarms[0].status, AlarmStatus::Pending);

    // 5. Clean shutdown of replacement daemon
    Daemon::stop(&state_dir)?;
    let _ = child2.wait()?;
    Ok(())
}

#[test]
fn test_daemon_missed_alarm_recovery_on_boot() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    // Manually persist an alarm in the store that is 2 hours overdue (> 10m grace window)
    let store = AlarmStore::open(&state_dir)?;
    let past_deadline = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let overdue_alarm = Alarm {
        id: Uuid::now_v7().to_string(),
        pane_id: "w1:p2".to_string(),
        created_by_pane: None,
        delivery: DeliveryMode::Prompt {
            text: "Overdue wake-up".to_string(),
        },
        label: Some("overdue".to_string()),
        created_at: past_deadline - Duration::from_hours(1),
        deadline: past_deadline,
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::Deadline,
        schema_version: 1,
    };
    store.put(&overdue_alarm)?;

    // Boot daemon: should mark the overdue alarm as Missed
    let handle = spawn_daemon(state_dir.clone());
    assert!(wait_for_daemon(&state_dir), "daemon failed to start");

    let client = DaemonClient::connect(&state_dir)?;
    let missed_alarm = client.get_alarm(&overdue_alarm.id)?;
    assert_eq!(missed_alarm.status, AlarmStatus::Missed);
    assert!(missed_alarm.failure_reason.is_some());

    Daemon::stop(&state_dir)?;
    handle.join().unwrap()?;
    Ok(())
}
