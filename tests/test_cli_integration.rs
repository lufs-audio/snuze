#![allow(clippy::unwrap_used)]
#![allow(clippy::indexing_slicing)]

use std::path::PathBuf;
use std::process::Command;

fn fake_herdr_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_herdr.sh")
}

#[test]
fn test_cli_dry_run_set_json() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    cmd.env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .env("HERDR_PANE_ID", "w1:p0")
        .arg("set")
        .arg("--after")
        .arg("10s")
        .arg("--pane")
        .arg("w1:p3")
        .arg("--prompt")
        .arg("Wake up agent")
        .arg("--dry-run")
        .arg("--json");

    let output = cmd.output()?;
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;
    assert_eq!(json.get("status").and_then(|s| s.as_str()), Some("success"));
    assert_eq!(
        json.get("data")
            .and_then(|d| d.get("pane_id"))
            .and_then(|s| s.as_str()),
        Some("w1:p3")
    );
    assert_eq!(
        json.get("data")
            .and_then(|d| d.get("created_by_pane"))
            .and_then(|s| s.as_str()),
        Some("w1:p0")
    );
    assert_eq!(
        json.get("data")
            .and_then(|d| d.get("status"))
            .and_then(|s| s.as_str()),
        Some("pending")
    );
    assert_eq!(
        json.get("data")
            .and_then(|d| d.get("delivery"))
            .and_then(|d| d.get("mode"))
            .and_then(|s| s.as_str()),
        Some("prompt")
    );

    Ok(())
}

#[test]
fn test_cli_dry_run_pane_not_found() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    cmd.env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "pane_not_found")
        .arg("set")
        .arg("--after")
        .arg("10s")
        .arg("--pane")
        .arg("w1:p99")
        .arg("--prompt")
        .arg("Wake up agent")
        .arg("--dry-run")
        .arg("--json");

    let output = cmd.output()?;
    assert_eq!(output.status.code(), Some(3)); // TargetUnavailable

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;
    assert_eq!(json.get("status").and_then(|s| s.as_str()), Some("error"));
    assert_eq!(
        json.get("code").and_then(serde_json::Value::as_i64),
        Some(3)
    );

    Ok(())
}

#[test]
fn test_cli_error_envelope_json() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    cmd.env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("set")
        .arg("--after")
        .arg("not-a-duration")
        .arg("--pane")
        .arg("w1:p3")
        .arg("--prompt")
        .arg("Wake up")
        .arg("--json");

    let output = cmd.output()?;
    assert_eq!(output.status.code(), Some(2)); // Usage

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)?;
    assert_eq!(json.get("status").and_then(|s| s.as_str()), Some("error"));
    assert_eq!(
        json.get("code").and_then(serde_json::Value::as_i64),
        Some(2)
    );
    assert!(json.get("message").is_some());

    Ok(())
}

#[test]
fn test_cli_end_to_end_crud() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    // Start background daemon
    let mut start_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    start_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("daemon")
        .arg("start")
        .arg("--json");
    let start_out = start_cmd.output()?;
    assert!(start_out.status.success());

    // 1. snuze set --json
    let mut set_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    set_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("set")
        .arg("--after")
        .arg("1h")
        .arg("--pane")
        .arg("w1:p3")
        .arg("--prompt")
        .arg("Wake up agent")
        .arg("--label")
        .arg("integration-test")
        .arg("--json");
    let set_out = set_cmd.output()?;
    assert_eq!(set_out.status.code(), Some(0));

    let set_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&set_out.stdout))?;
    assert_eq!(set_json.get("status").and_then(|s| s.as_str()), Some("success"));
    let alarm_id = set_json
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(|s| s.as_str())
        .unwrap()
        .to_string();

    // 2. snuze list --json
    let mut list_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    list_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("list")
        .arg("--json");
    let list_out = list_cmd.output()?;
    assert_eq!(list_out.status.code(), Some(0));

    let list_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&list_out.stdout))?;
    assert_eq!(list_json.get("status").and_then(|s| s.as_str()), Some("success"));
    let list_data = list_json.get("data").and_then(|d| d.as_array()).unwrap();
    assert_eq!(list_data.len(), 1);

    // 3. snuze status <ALARM_ID> --json
    let mut status_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    status_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("status")
        .arg(&alarm_id)
        .arg("--json");
    let status_out = status_cmd.output()?;
    assert_eq!(status_out.status.code(), Some(0));

    let status_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&status_out.stdout))?;
    assert_eq!(status_json.get("status").and_then(|s| s.as_str()), Some("success"));
    assert_eq!(
        status_json.get("data").and_then(|d| d.get("id")).and_then(|s| s.as_str()),
        Some(alarm_id.as_str())
    );

    // 4. snuze status non-existent -> exit 3
    let mut status_missing_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    status_missing_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("status")
        .arg("01HGWY37VZ8Q0000")
        .arg("--json");
    let missing_out = status_missing_cmd.output()?;
    assert_eq!(missing_out.status.code(), Some(3)); // TargetUnavailable

    // 5. snuze cancel <ALARM_ID> --json
    let mut cancel_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    cancel_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("cancel")
        .arg(&alarm_id)
        .arg("--json");
    let cancel_out = cancel_cmd.output()?;
    assert_eq!(cancel_out.status.code(), Some(0));

    // 6. snuze doctor --json (when daemon is running and herdr is present)
    let mut doctor_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    doctor_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("doctor")
        .arg("--json");
    let doctor_out = doctor_cmd.output()?;
    assert_eq!(doctor_out.status.code(), Some(0));

    // 7. snuze daemon stop --json
    let mut stop_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    stop_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("daemon")
        .arg("stop")
        .arg("--json");
    let stop_out = stop_cmd.output()?;
    assert!(stop_out.status.success());

    Ok(())
}

#[test]
fn test_cli_doctor_fails_when_herdr_socket_unreachable() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    // Start daemon
    let mut start_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    start_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("daemon")
        .arg("start")
        .arg("--json");
    let start_out = start_cmd.output()?;
    assert!(start_out.status.success());

    // Run doctor with fake herdr set to fail socket check
    let mut doctor_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    doctor_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "generic_error")
        .arg("doctor")
        .arg("--json");
    let doctor_out = doctor_cmd.output()?;
    assert_eq!(doctor_out.status.code(), Some(6)); // DaemonUnavailable

    // Stop daemon
    let mut stop_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    stop_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("daemon")
        .arg("stop");
    let _ = stop_cmd.output();

    Ok(())
}

#[test]
fn test_cli_doctor_surfaces_scan_errors() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    // Write a corrupt alarm file directly to alarms dir
    let alarms_dir = state_dir.join("snuze/alarms");
    std::fs::create_dir_all(&alarms_dir)?;
    std::fs::write(alarms_dir.join("corrupted.json"), b"not valid json")?;

    // Start daemon
    let mut start_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    start_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("daemon")
        .arg("start");
    let start_out = start_cmd.output()?;
    assert!(start_out.status.success());

    // Run doctor --json
    let mut doctor_json_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    doctor_json_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("doctor")
        .arg("--json");
    let doctor_json_out = doctor_json_cmd.output()?;
    assert_eq!(doctor_json_out.status.code(), Some(0));

    let json_val: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&doctor_json_out.stdout))?;
    let corrupt_array = json_val
        .get("data")
        .and_then(|d| d.get("corrupt_alarms"))
        .and_then(|c| c.as_array())
        .unwrap();
    assert_eq!(corrupt_array.len(), 1);
    assert_eq!(
        corrupt_array[0].get("file").and_then(|f| f.as_str()),
        Some("corrupted.json")
    );

    // Run doctor human format
    let mut doctor_human_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    doctor_human_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("doctor");
    let doctor_human_out = doctor_human_cmd.output()?;
    assert_eq!(doctor_human_out.status.code(), Some(0));
    let human_str = String::from_utf8_lossy(&doctor_human_out.stdout);
    assert!(human_str.contains("Corrupt Alarms: 1"));
    assert!(human_str.contains("corrupted.json"));

    // Stop daemon
    let mut stop_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    stop_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("daemon")
        .arg("stop");
    let _ = stop_cmd.output();

    Ok(())
}
