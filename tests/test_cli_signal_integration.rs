#![allow(clippy::unwrap_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::expect_used)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::similar_names)]

use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

fn fake_herdr_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_herdr.sh")
}

#[test]
fn test_cli_dry_run_signal_set_json() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    cmd.env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .env("HERDR_PANE_ID", "w1:p0")
        .arg("set")
        .arg("--signal")
        .arg("build-complete")
        .arg("--timeout")
        .arg("15m")
        .arg("--pane")
        .arg("w1:p3")
        .arg("--prompt")
        .arg("Build completed!")
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
            .and_then(|d| d.get("schema_version"))
            .and_then(serde_json::Value::as_i64),
        Some(3)
    );
    assert_eq!(
        json.get("data")
            .and_then(|d| d.get("trigger"))
            .and_then(|t| t.get("kind"))
            .and_then(|s| s.as_str()),
        Some("external_signal")
    );

    Ok(())
}

#[test]
fn test_cli_dry_run_herdr_pane_watched_pane_not_found() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    cmd.env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "pane_not_found")
        .arg("set")
        .arg("--herdr-pane")
        .arg("nonexistent")
        .arg("--watch-for")
        .arg("idle,done")
        .arg("--timeout")
        .arg("10m")
        .arg("--pane")
        .arg("w1:p3")
        .arg("--prompt")
        .arg("Agent done")
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
    assert!(json
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or_default()
        .contains("watched Herdr pane not found"));

    Ok(())
}

#[test]
fn test_cli_signal_end_to_end() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    // 1. Start background daemon
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

    // 2. Schedule an external signal alarm
    let mut set_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    set_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("set")
        .arg("--signal")
        .arg("sig-deploy-42")
        .arg("--timeout")
        .arg("10m")
        .arg("--pane")
        .arg("w1:p3")
        .arg("--prompt")
        .arg("Deployment 42 finished after {duration}")
        .arg("--label")
        .arg("deploy-alarm")
        .arg("--json");

    let set_out = set_cmd.output()?;
    assert_eq!(set_out.status.code(), Some(0));
    let set_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&set_out.stdout))?;
    assert_eq!(
        set_json.get("status").and_then(|s| s.as_str()),
        Some("success")
    );
    let alarm_id = set_json
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(|s| s.as_str())
        .unwrap()
        .to_string();

    // 3. Schedule a herdr-pane alarm (which should NOT be resolved by this signal)
    let mut set_herdr_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    set_herdr_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("set")
        .arg("--herdr-pane")
        .arg("w1:p2")
        .arg("--watch-for")
        .arg("idle,done")
        .arg("--timeout")
        .arg("10m")
        .arg("--pane")
        .arg("w1:p3")
        .arg("--prompt")
        .arg("Pane w1:p2 is idle")
        .arg("--label")
        .arg("herdr-watch-alarm")
        .arg("--json");

    let set_herdr_out = set_herdr_cmd.output()?;
    assert_eq!(set_herdr_out.status.code(), Some(0));
    let set_herdr_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&set_herdr_out.stdout))?;
    let herdr_alarm_id = set_herdr_json
        .get("data")
        .and_then(|d| d.get("id"))
        .and_then(|s| s.as_str())
        .unwrap()
        .to_string();

    // 4. Test `snuze signal` with unmatched key (returns matched: 0, exit 0)
    let mut sig_unmatched_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    sig_unmatched_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("signal")
        .arg("nonexistent-key")
        .arg("--json");
    let sig_unmatched_out = sig_unmatched_cmd.output()?;
    assert_eq!(sig_unmatched_out.status.code(), Some(0));
    let unmatched_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&sig_unmatched_out.stdout))?;
    assert_eq!(
        unmatched_json.get("status").and_then(|s| s.as_str()),
        Some("success")
    );
    assert_eq!(
        unmatched_json
            .get("data")
            .and_then(|d| d.get("matched"))
            .and_then(serde_json::Value::as_i64),
        Some(0)
    );

    // 5. Test `snuze signal` with --dry-run
    let mut sig_dryrun_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    sig_dryrun_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("signal")
        .arg("sig-deploy-42")
        .arg("--dry-run")
        .arg("--json");
    let sig_dryrun_out = sig_dryrun_cmd.output()?;
    assert_eq!(sig_dryrun_out.status.code(), Some(0));
    let dryrun_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&sig_dryrun_out.stdout))?;
    assert_eq!(
        dryrun_json.get("status").and_then(|s| s.as_str()),
        Some("success")
    );
    assert_eq!(
        dryrun_json
            .get("data")
            .and_then(|d| d.get("matched"))
            .and_then(serde_json::Value::as_i64),
        Some(1)
    );

    // Verify alarm is still pending after dry-run
    let mut status_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    status_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("status")
        .arg(&alarm_id)
        .arg("--json");
    let status_out = status_cmd.output()?;
    let status_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&status_out.stdout))?;
    assert_eq!(
        status_json
            .get("data")
            .and_then(|d| d.get("status"))
            .and_then(|s| s.as_str()),
        Some("pending")
    );

    // 6. Signal the alarm for real
    let mut sig_real_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    sig_real_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("signal")
        .arg("sig-deploy-42")
        .arg("--json");
    let sig_real_out = sig_real_cmd.output()?;
    assert_eq!(sig_real_out.status.code(), Some(0));
    let sig_real_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&sig_real_out.stdout))?;
    assert_eq!(
        sig_real_json.get("status").and_then(|s| s.as_str()),
        Some("success")
    );
    assert_eq!(
        sig_real_json
            .get("data")
            .and_then(|d| d.get("matched"))
            .and_then(serde_json::Value::as_i64),
        Some(1)
    );

    // Wait briefly for scheduler/store to update status
    thread::sleep(Duration::from_millis(200));

    // Verify external signal alarm transitioned to fired
    let mut check_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    check_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("status")
        .arg(&alarm_id)
        .arg("--json");
    let check_out = check_cmd.output()?;
    let check_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&check_out.stdout))?;
    assert_eq!(
        check_json
            .get("data")
            .and_then(|d| d.get("status"))
            .and_then(|s| s.as_str()),
        Some("fired")
    );

    // Verify herdr-pane alarm remains pending
    let mut check_herdr_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    check_herdr_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("status")
        .arg(&herdr_alarm_id)
        .arg("--json");
    let check_herdr_out = check_herdr_cmd.output()?;
    let check_herdr_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&check_herdr_out.stdout))?;
    assert_eq!(
        check_herdr_json
            .get("data")
            .and_then(|d| d.get("status"))
            .and_then(|s| s.as_str()),
        Some("pending")
    );

    // 7. Test broadcast semantics: arm two alarms sharing key "broadcast-key"
    let mut set_b1_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    set_b1_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("set")
        .arg("--signal")
        .arg("broadcast-key")
        .arg("--timeout")
        .arg("10m")
        .arg("--pane")
        .arg("w1:p3")
        .arg("--prompt")
        .arg("Broadcast 1")
        .arg("--json");
    let set_b1_out = set_b1_cmd.output()?;
    assert_eq!(set_b1_out.status.code(), Some(0));
    let b1_id =
        serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&set_b1_out.stdout))?
            .get("data")
            .and_then(|d| d.get("id"))
            .and_then(|s| s.as_str())
            .unwrap()
            .to_string();

    let mut set_b2_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    set_b2_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .arg("set")
        .arg("--signal")
        .arg("broadcast-key")
        .arg("--timeout")
        .arg("10m")
        .arg("--pane")
        .arg("w1:p4")
        .arg("--prompt")
        .arg("Broadcast 2")
        .arg("--json");
    let set_b2_out = set_b2_cmd.output()?;
    assert_eq!(set_b2_out.status.code(), Some(0));
    let b2_id =
        serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&set_b2_out.stdout))?
            .get("data")
            .and_then(|d| d.get("id"))
            .and_then(|s| s.as_str())
            .unwrap()
            .to_string();

    // Signal broadcast-key
    let mut sig_b_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    sig_b_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .arg("signal")
        .arg("broadcast-key")
        .arg("--json");
    let sig_b_out = sig_b_cmd.output()?;
    assert_eq!(sig_b_out.status.code(), Some(0));
    let sig_b_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&sig_b_out.stdout))?;
    assert_eq!(
        sig_b_json
            .get("data")
            .and_then(|d| d.get("matched"))
            .and_then(serde_json::Value::as_i64),
        Some(2)
    );

    thread::sleep(Duration::from_millis(200));

    // Both should be fired
    for b_id in [&b1_id, &b2_id] {
        let mut status_b = Command::new(env!("CARGO_BIN_EXE_snuze"));
        status_b
            .env("XDG_STATE_HOME", &state_dir)
            .arg("status")
            .arg(b_id)
            .arg("--json");
        let out = status_b.output()?;
        let check_b_json: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&out.stdout))?;
        assert_eq!(
            check_b_json
                .get("data")
                .and_then(|d| d.get("status"))
                .and_then(|s| s.as_str()),
            Some("fired")
        );
    }

    // 8. Stop daemon cleanly
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
