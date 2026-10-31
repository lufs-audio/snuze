#![allow(clippy::unwrap_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::expect_used)]
#![allow(clippy::too_many_lines)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

static PORT: AtomicU16 = AtomicU16::new(3150);

type MockHandler = Arc<dyn Fn(&str) -> (u16, String) + Send + Sync>;

fn fake_herdr_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_herdr.sh")
}

fn start_mock_jules_server(handler: MockHandler) -> String {
    let port = PORT.fetch_add(1, Ordering::SeqCst);
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).expect("bind mock server");
    let url = format!("http://127.0.0.1:{port}");

    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let mut reader = BufReader::new(&stream);
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }

            let path = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or("/")
                .to_string();

            let (status, body) = handler(&path);
            let reason = if status == 200 { "OK" } else { "Not Found" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    url
}

#[test]
fn test_cli_dry_run_jules_set_json() -> Result<(), Box<dyn std::error::Error>> {
    let server_url = start_mock_jules_server(Arc::new(|_path| {
        (200, r#"{"state": "IN_PROGRESS"}"#.to_string())
    }));

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    cmd.env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .env("HERDR_PANE_ID", "w1:p0")
        .env("JULES_API_KEY", "test-key-dryrun")
        .env("JULES_BASE_URL", &server_url)
        .arg("set")
        .arg("--jules")
        .arg("sess-alpha")
        .arg("--condition")
        .arg("all-terminal")
        .arg("--timeout")
        .arg("10m")
        .arg("--pane")
        .arg("w1:p3")
        .arg("--prompt")
        .arg("Jules session {session_id} finished")
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
            .and_then(|d| d.get("status"))
            .and_then(|s| s.as_str()),
        Some("pending")
    );

    Ok(())
}

#[test]
fn test_cli_dry_run_jules_session_not_found() -> Result<(), Box<dyn std::error::Error>> {
    let server_url = start_mock_jules_server(Arc::new(|_path| {
        (
            404,
            r#"{"error": "Session nonexistent not found"}"#.to_string(),
        )
    }));

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    cmd.env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .env("JULES_API_KEY", "test-key")
        .env("JULES_BASE_URL", &server_url)
        .arg("set")
        .arg("--jules")
        .arg("nonexistent")
        .arg("--condition")
        .arg("all-terminal")
        .arg("--timeout")
        .arg("10m")
        .arg("--pane")
        .arg("w1:p3")
        .arg("--prompt")
        .arg("Jules session finished")
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
        .contains("Jules session not found: nonexistent"));

    Ok(())
}

#[test]
fn test_cli_jules_end_to_end() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let state_dir = temp_dir.path().to_path_buf();

    let completed_flag = Arc::new(AtomicBool::new(false));
    let flag_clone = Arc::clone(&completed_flag);

    let server_url = start_mock_jules_server(Arc::new(move |_path| {
        if flag_clone.load(Ordering::SeqCst) {
            (
                200,
                r#"{"state": "COMPLETED", "outputs": [{"pullRequest": {"url": "https://github.com/org/repo/pull/1"}}]}"#.to_string(),
            )
        } else {
            (200, r#"{"state": "IN_PROGRESS"}"#.to_string())
        }
    }));

    // 1. Start background daemon
    let mut start_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    start_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .env("JULES_API_KEY", "test-key-e2e")
        .env("JULES_BASE_URL", &server_url)
        .arg("daemon")
        .arg("start")
        .arg("--json");
    let start_out = start_cmd.output()?;
    assert!(start_out.status.success());

    // 2. Schedule Jules alarm: poll interval 1s, timeout 30s
    let mut set_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
    set_cmd
        .env("XDG_STATE_HOME", &state_dir)
        .env("FAKE_HERDR_PATH", fake_herdr_path())
        .env("FAKE_HERDR_BEHAVIOR", "delivered")
        .env("JULES_API_KEY", "test-key-e2e")
        .env("JULES_BASE_URL", &server_url)
        .arg("set")
        .arg("--jules")
        .arg("sess-e2e")
        .arg("--condition")
        .arg("any-completed")
        .arg("--timeout")
        .arg("30s")
        .arg("--poll-interval")
        .arg("1s")
        .arg("--pane")
        .arg("w1:p3")
        .arg("--prompt")
        .arg("Jules session {session_id} ended with {status} ({pr_urls})")
        .arg("--label")
        .arg("jules-e2e-alarm")
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

    // 3. Inspect status while pending
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
        status_json
            .get("data")
            .and_then(|d| d.get("status"))
            .and_then(|s| s.as_str()),
        Some("pending")
    );

    // 4. Mark Jules state as COMPLETED on mock server
    completed_flag.store(true, Ordering::SeqCst);

    // 5. Poll status until transitioned to fired
    let mut fired = false;
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(500));
        let mut check_cmd = Command::new(env!("CARGO_BIN_EXE_snuze"));
        check_cmd
            .env("XDG_STATE_HOME", &state_dir)
            .arg("status")
            .arg(&alarm_id)
            .arg("--json");
        let check_out = check_cmd.output()?;
        if check_out.status.success() {
            let check_json: serde_json::Value =
                serde_json::from_str(&String::from_utf8_lossy(&check_out.stdout))?;
            let current_status = check_json
                .get("data")
                .and_then(|d| d.get("status"))
                .and_then(|s| s.as_str());
            if current_status == Some("fired") {
                fired = true;
                break;
            }
        }
    }

    assert!(fired, "Jules alarm should transition to fired status");

    // 6. Stop daemon cleanly
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
