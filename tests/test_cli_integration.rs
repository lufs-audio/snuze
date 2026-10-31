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
            .and_then(|d| d.get("target"))
            .and_then(|t| t.get("type"))
            .and_then(|s| s.as_str()),
        Some("herdr")
    );
    assert_eq!(
        json.get("data")
            .and_then(|d| d.get("target"))
            .and_then(|t| t.get("pane_id"))
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
    assert_eq!(
        list_json.get("status").and_then(|s| s.as_str()),
        Some("success")
    );
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
    assert_eq!(
        status_json.get("status").and_then(|s| s.as_str()),
        Some("success")
    );
    assert_eq!(
        status_json
            .get("data")
            .and_then(|d| d.get("id"))
            .and_then(|s| s.as_str()),
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

/// Unit 01: delivery-target resolution (explicit flags, `--target` specs, and
/// `$HERDR_ENV`/`$TMUX` auto-detection) plus the USAGE error when nothing resolves.
#[test]
fn test_cli_delivery_target_resolution() -> Result<(), Box<dyn std::error::Error>> {
    fn run_set(
        extra: &[&str],
        env: &[(&str, &str)],
    ) -> Result<(i32, serde_json::Value), Box<dyn std::error::Error>> {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
        cmd.env_remove("HERDR_ENV")
            .env_remove("HERDR_PANE_ID")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env_remove("FAKE_HERDR_PATH");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.arg("set").arg("--after").arg("10s").args(extra);
        let out = cmd.output()?;
        let code = out.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let json = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Null);
        Ok((code, json))
    }

    // 1. Explicit --tmux works with no Herdr anywhere in sight.
    let (code, json) = run_set(
        &["--tmux", "%0", "--prompt", "hi", "--dry-run", "--json"],
        &[],
    )?;
    assert_eq!(code, 0);
    assert_eq!(json["data"]["target"]["type"], "tmux");
    assert_eq!(json["data"]["target"]["target"], "%0");

    // 2. --target tmux:<id> is equivalent.
    let (code, json) = run_set(
        &[
            "--target",
            "tmux:%2",
            "--prompt",
            "hi",
            "--dry-run",
            "--json",
        ],
        &[],
    )?;
    assert_eq!(code, 0);
    assert_eq!(json["data"]["target"]["type"], "tmux");
    assert_eq!(json["data"]["target"]["target"], "%2");

    // 3. --target wait needs no payload at all.
    let (code, json) = run_set(&["--target", "wait", "--dry-run", "--json"], &[])?;
    assert_eq!(code, 0);
    assert_eq!(json["data"]["target"]["type"], "wait");

    // 4. No target flag and no environment -> USAGE (exit 2).
    let (code, _) = run_set(&["--prompt", "hi", "--dry-run", "--json"], &[])?;
    assert_eq!(code, 2);

    // 5. $TMUX + $TMUX_PANE auto-detect a tmux target.
    let (code, json) = run_set(
        &["--prompt", "hi", "--dry-run", "--json"],
        &[
            ("TMUX", "/tmp/tmux-1000/default,123,0"),
            ("TMUX_PANE", "%7"),
        ],
    )?;
    assert_eq!(code, 0);
    assert_eq!(json["data"]["target"]["type"], "tmux");
    assert_eq!(json["data"]["target"]["target"], "%7");

    // 6. $HERDR_ENV=1 + $HERDR_PANE_ID auto-detect a Herdr target.
    let (code, json) = run_set(
        &["--prompt", "hi", "--dry-run", "--json"],
        &[
            ("HERDR_ENV", "1"),
            ("HERDR_PANE_ID", "w1:p5"),
            ("FAKE_HERDR_PATH", fake_herdr_path().to_str().unwrap()),
            ("FAKE_HERDR_BEHAVIOR", "delivered"),
        ],
    )?;
    assert_eq!(code, 0);
    assert_eq!(json["data"]["target"]["type"], "herdr");
    assert_eq!(json["data"]["target"]["pane_id"], "w1:p5");

    // 7. Conflicting target flags -> USAGE (exit 2).
    let (code, _) = run_set(
        &[
            "--pane",
            "w1:p3",
            "--tmux",
            "%0",
            "--prompt",
            "hi",
            "--dry-run",
            "--json",
        ],
        &[("FAKE_HERDR_PATH", fake_herdr_path().to_str().unwrap())],
    )?;
    assert_eq!(code, 2);
    // 8. Malformed --target -> USAGE (exit 2).
    let (code, _) = run_set(
        &[
            "--target",
            "nope:x",
            "--prompt",
            "hi",
            "--dry-run",
            "--json",
        ],
        &[],
    )?;
    assert_eq!(code, 2);

    Ok(())
}

/// Unit 02: `--pid` process-exit trigger — scheduling, preflight, and conflicts.
#[test]
fn test_cli_pid_trigger() -> Result<(), Box<dyn std::error::Error>> {
    fn run_set(
        extra: &[&str],
    ) -> Result<(i32, String, serde_json::Value), Box<dyn std::error::Error>> {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
        cmd.env_remove("HERDR_ENV")
            .env_remove("HERDR_PANE_ID")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        cmd.arg("set").args(extra);
        let out = cmd.output()?;
        let code = out.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let json = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Null);
        Ok((code, stdout, json))
    }

    // 1. A PID that does not exist fails immediately with an actionable error.
    let (code, stdout, json) = run_set(&[
        "--pid",
        "999999",
        "--prompt",
        "done",
        "--tmux",
        "%0",
        "--dry-run",
        "--json",
    ])?;
    assert_eq!(code, 3, "expected TargetUnavailable, got: {stdout}");
    assert_eq!(json["code"], 3);
    assert!(
        json["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not running"),
        "unexpected message: {stdout}"
    );

    // 2. A live PID schedules a process_exit alarm; --timeout is optional.
    let mut child = Command::new("sh").arg("-c").arg("sleep 30").spawn()?;
    let pid = child.id().to_string();

    let (code, stdout, json) = run_set(&[
        "--pid",
        &pid,
        "--prompt",
        "process exited",
        "--tmux",
        "%0",
        "--dry-run",
        "--json",
    ])?;
    assert_eq!(code, 0, "expected success, got: {stdout}");
    assert_eq!(json["data"]["trigger"]["kind"], "process_exit");
    assert_eq!(
        json["data"]["trigger"]["pid"].as_u64(),
        Some(pid.parse::<u64>().unwrap())
    );

    // 3. An explicit --timeout is honoured.
    let (code, stdout, json) = run_set(&[
        "--pid",
        &pid,
        "--timeout",
        "90m",
        "--prompt",
        "x",
        "--tmux",
        "%0",
        "--dry-run",
        "--json",
    ])?;
    assert_eq!(code, 0, "expected success, got: {stdout}");
    assert_eq!(json["data"]["trigger"]["kind"], "process_exit");

    // 4. --pid combined with another trigger mode is a USAGE error.
    let (code, _, _) = run_set(&[
        "--pid",
        &pid,
        "--after",
        "10s",
        "--prompt",
        "x",
        "--tmux",
        "%0",
        "--dry-run",
        "--json",
    ])?;
    assert_eq!(code, 2);

    // 5. Malformed --timeout with --pid is a USAGE error.
    let (code, _, _) = run_set(&[
        "--pid",
        &pid,
        "--timeout",
        "soon",
        "--prompt",
        "x",
        "--tmux",
        "%0",
        "--dry-run",
        "--json",
    ])?;
    assert_eq!(code, 2);

    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

/// Unit 03: `--watch-file` / `--watch-mode` artifact trigger.
#[test]
fn test_cli_watch_file_trigger() -> Result<(), Box<dyn std::error::Error>> {
    fn run_set(
        extra: &[&str],
    ) -> Result<(i32, String, serde_json::Value), Box<dyn std::error::Error>> {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
        cmd.env_remove("HERDR_ENV")
            .env_remove("HERDR_PANE_ID")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        cmd.arg("set").args(extra);
        let out = cmd.output()?;
        let code = out.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let json = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Null);
        Ok((code, stdout, json))
    }

    // 1. --watch-file defaults to `exists` mode.
    let (code, stdout, json) = run_set(&[
        "--watch-file",
        "/tmp/take.json",
        "--prompt",
        "ready",
        "--tmux",
        "%0",
        "--dry-run",
        "--json",
    ])?;
    assert_eq!(code, 0, "expected success, got: {stdout}");
    assert_eq!(json["data"]["trigger"]["kind"], "file_artifact");
    assert_eq!(json["data"]["trigger"]["mode"], "exists");
    assert_eq!(json["data"]["trigger"]["path"], "/tmp/take.json");

    // 2. --watch-mode modified is honoured.
    let (code, stdout, json) = run_set(&[
        "--watch-file",
        "/tmp/*.json",
        "--watch-mode",
        "modified",
        "--prompt",
        "ready",
        "--tmux",
        "%0",
        "--dry-run",
        "--json",
    ])?;
    assert_eq!(code, 0, "expected success, got: {stdout}");
    assert_eq!(json["data"]["trigger"]["mode"], "modified");

    // 3. An unrecognized --watch-mode is a USAGE error.
    let (code, _, _) = run_set(&[
        "--watch-file",
        "/tmp/x.json",
        "--watch-mode",
        "sideways",
        "--prompt",
        "ready",
        "--tmux",
        "%0",
        "--dry-run",
        "--json",
    ])?;
    assert_eq!(code, 2);

    // 4. --watch-mode without --watch-file is a USAGE error.
    let (code, _, _) = run_set(&[
        "--after",
        "10s",
        "--watch-mode",
        "modified",
        "--prompt",
        "ready",
        "--tmux",
        "%0",
        "--dry-run",
        "--json",
    ])?;
    assert_eq!(code, 2);

    // 5. --watch-file conflicts with --after.
    let (code, _, _) = run_set(&[
        "--after",
        "10s",
        "--watch-file",
        "/tmp/x.json",
        "--prompt",
        "ready",
        "--tmux",
        "%0",
        "--dry-run",
        "--json",
    ])?;
    assert_eq!(code, 2);

    Ok(())
}

/// Unit 04: `snuze wait <ALARM_ID>` blocks on the daemon and maps the terminal
/// state onto the process exit code.
#[test]
fn test_cli_wait_command() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    fn snuze(
        state_dir: &std::path::Path,
        args: &[&str],
    ) -> Result<std::process::Output, Box<dyn std::error::Error>> {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
        cmd.env("XDG_STATE_HOME", state_dir)
            .env_remove("HERDR_ENV")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE");
        cmd.args(args);
        Ok(cmd.output()?)
    }

    // Start the daemon.
    let start = snuze(&state_dir, &["daemon", "start", "--json"])?;
    assert!(start.status.success(), "daemon failed to start");

    // 1. Schedule a `wait`-target alarm one second out, then block on it.
    let set = snuze(
        &state_dir,
        &["set", "--target", "wait", "--after", "1s", "--json"],
    )?;
    assert_eq!(set.status.code(), Some(0));
    let set_json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&set.stdout))?;
    let alarm_id = set_json["data"]["id"].as_str().unwrap().to_string();

    let wait = snuze(&state_dir, &["wait", &alarm_id, "--json"])?;
    assert_eq!(
        wait.status.code(),
        Some(0),
        "expected Fired exit code 0, stdout={}",
        String::from_utf8_lossy(&wait.stdout)
    );
    let wait_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&wait.stdout))?;
    assert_eq!(wait_json["status"], "success");
    assert_eq!(wait_json["data"]["alarm_id"], alarm_id);
    assert_eq!(wait_json["data"]["status"], "fired");

    // 2. An unknown alarm is a TargetUnavailable (exit 3).
    let missing = snuze(
        &state_dir,
        &["wait", "01HGWY37VZ8Q0000", "--timeout", "5s", "--json"],
    )?;
    assert_eq!(missing.status.code(), Some(3));

    // 3. Waiting past the timeout on a long alarm is a ContractViolated (exit 5).
    let long = snuze(
        &state_dir,
        &["set", "--target", "wait", "--after", "1h", "--json"],
    )?;
    assert_eq!(long.status.code(), Some(0));
    let long_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&long.stdout))?;
    let long_id = long_json["data"]["id"].as_str().unwrap().to_string();

    let timed_out = snuze(&state_dir, &["wait", &long_id, "--timeout", "1s", "--json"])?;
    assert_eq!(
        timed_out.status.code(),
        Some(5),
        "expected exit 5 on wait timeout, stdout={}",
        String::from_utf8_lossy(&timed_out.stdout)
    );
    let timeout_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&timed_out.stdout))?;
    assert_eq!(timeout_json["status"], "error");
    assert_eq!(timeout_json["code"], 5);

    // 4. A malformed --timeout is a USAGE error (exit 2).
    let bad = snuze(
        &state_dir,
        &["wait", &long_id, "--timeout", "soon", "--json"],
    )?;
    assert_eq!(bad.status.code(), Some(2));

    let stop = snuze(&state_dir, &["daemon", "stop"])?;
    assert!(stop.status.success());
    Ok(())
}
