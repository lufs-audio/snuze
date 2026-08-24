#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::panic
)]

use snuze::jules::{JulesAdapter, RealJulesAdapter};
use snuze::kernel::{JulesState, SnuzeError};
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

static PORT: AtomicU16 = AtomicU16::new(3010);

/// Minimal mock HTTP server bound to 127.0.0.1 on an ephemeral port.
fn start_mock_server(
    response_handler: fn(&str) -> (u16, String),
) -> String {
    let port = PORT.fetch_add(1, Ordering::SeqCst);
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).expect("bind");
    let url = format!("http://127.0.0.1:{port}");

    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let mut reader = BufReader::new(&stream);
            let mut request_line = String::new();
            let _ = reader.read_line(&mut request_line);

            if request_line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = request_line.split_whitespace().collect();
            let path = parts.get(1).unwrap_or(&"/");

            let (status, body) = response_handler(path);

            let response = format!(
                "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                status,
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    url
}

static ENV_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn test_real_jules_adapter_new_missing_or_empty_key() {
    let _guard = ENV_MUTEX.lock().unwrap();
    env::remove_var("JULES_API_KEY");
    let result = RealJulesAdapter::new();
    assert!(matches!(result, Err(SnuzeError::JulesCredentialMissing)));

    env::set_var("JULES_API_KEY", "   ");
    let result = RealJulesAdapter::new();
    assert!(matches!(result, Err(SnuzeError::JulesCredentialMissing)));

    env::remove_var("JULES_API_KEY");
}

#[test]
fn test_real_jules_adapter_new_with_valid_key() {
    let _guard = ENV_MUTEX.lock().unwrap();
    env::set_var("JULES_API_KEY", "test-valid-key-12345");
    let result = RealJulesAdapter::new();
    assert!(result.is_ok());
    env::remove_var("JULES_API_KEY");
}

#[test]
fn test_real_jules_adapter_successful_multi_session() {
    let server = start_mock_server(|path| {
        if path.contains("sess-1") {
            (
                200,
                r#"{"state": "COMPLETED", "outputs": [{"pullRequest": {"url": "https://github.com/lufs-audio/snuze/pull/42"}}]}"#.to_string(),
            )
        } else if path.contains("sess-2") {
            (200, r#"{"state": "IN_PROGRESS"}"#.to_string())
        } else {
            (404, r#"{"error": "not found"}"#.to_string())
        }
    });

    let adapter = RealJulesAdapter::with_base_url_and_timeout(
        "mock-key".to_string(),
        server,
        Duration::from_secs(2),
    );

    let states = adapter
        .check_sessions(&["sess-1".to_string(), "sess-2".to_string()])
        .expect("check_sessions should succeed");

    assert_eq!(states.len(), 2);
    assert_eq!(states[0].session_id, "sess-1");
    assert_eq!(states[0].state, JulesState::Completed);
    assert_eq!(
        states[0].pull_request_url,
        Some("https://github.com/lufs-audio/snuze/pull/42".to_string())
    );
    assert_eq!(states[1].session_id, "sess-2");
    assert_eq!(states[1].state, JulesState::InProgress);
    assert_eq!(states[1].pull_request_url, None);
}

#[test]
fn test_real_jules_adapter_404_session_not_found() {
    let server = start_mock_server(|_path| (404, r#"{"error": "not found"}"#.to_string()));

    let adapter = RealJulesAdapter::with_base_url_and_timeout(
        "mock-key".to_string(),
        server,
        Duration::from_secs(2),
    );

    let err = adapter
        .check_sessions(&["missing-sess".to_string()])
        .expect_err("should fail on 404");

    match err {
        SnuzeError::JulesSessionNotFound(session_id) => {
            assert_eq!(session_id, "missing-sess");
        }
        _ => panic!("expected JulesSessionNotFound, got {err:?}"),
    }
}

#[test]
fn test_real_jules_adapter_401_and_403_auth_error() {
    let server = start_mock_server(|_path| (401, r#"{"error": "unauthorized"}"#.to_string()));

    let adapter = RealJulesAdapter::with_base_url_and_timeout(
        "bad-key".to_string(),
        server,
        Duration::from_secs(2),
    );

    let err = adapter
        .check_sessions(&["sess-1".to_string()])
        .expect_err("should fail on 401");
    assert!(matches!(err, SnuzeError::JulesCredentialMissing));

    // Also verify 403
    let server2 = start_mock_server(|_path| (403, r#"{"error": "forbidden"}"#.to_string()));
    let adapter2 = RealJulesAdapter::with_base_url_and_timeout(
        "bad-key".to_string(),
        server2,
        Duration::from_secs(2),
    );
    let err2 = adapter2
        .check_sessions(&["sess-1".to_string()])
        .expect_err("should fail on 403");
    assert!(matches!(err2, SnuzeError::JulesCredentialMissing));
}

#[test]
fn test_real_jules_adapter_429_rate_limited() {
    let server = start_mock_server(|_path| (429, r#"{"error": "too many requests"}"#.to_string()));

    let adapter = RealJulesAdapter::with_base_url_and_timeout(
        "mock-key".to_string(),
        server,
        Duration::from_secs(2),
    );

    let err = adapter
        .check_sessions(&["sess-1".to_string()])
        .expect_err("should fail on 429");

    match err {
        SnuzeError::DaemonUnavailable(msg) => {
            assert!(
                msg.contains("rate limited"),
                "msg should mention rate limiting: {msg}"
            );
        }
        _ => panic!("expected DaemonUnavailable, got {err:?}"),
    }
}

#[test]
fn test_real_jules_adapter_unrecognized_state_maps_to_unknown() {
    let server = start_mock_server(|_path| {
        (200, r#"{"state": "NEW_ALPHA_STATE"}"#.to_string())
    });

    let adapter = RealJulesAdapter::with_base_url_and_timeout(
        "mock-key".to_string(),
        server,
        Duration::from_secs(2),
    );

    let states = adapter
        .check_sessions(&["sess-1".to_string()])
        .expect("should succeed");

    assert_eq!(
        states[0].state,
        JulesState::Unknown("NEW_ALPHA_STATE".to_string())
    );
}

#[test]
fn test_real_jules_adapter_malformed_json_response() {
    let server = start_mock_server(|_path| {
        (200, "this is not json".to_string())
    });

    let adapter = RealJulesAdapter::with_base_url_and_timeout(
        "mock-key".to_string(),
        server,
        Duration::from_secs(2),
    );

    let err = adapter
        .check_sessions(&["sess-1".to_string()])
        .expect_err("should fail on malformed JSON");

    assert!(matches!(err, SnuzeError::DaemonUnavailable(_)));
}

#[test]
fn test_real_jules_adapter_timeout() {
    let port = PORT.fetch_add(1, Ordering::SeqCst);
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).expect("Expected bind");
    let url = format!("http://127.0.0.1:{port}");

    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let mut reader = BufReader::new(&stream);
            let mut request_line = String::new();
            let _ = reader.read_line(&mut request_line);
            thread::sleep(Duration::from_secs(5));
        }
    });

    let adapter = RealJulesAdapter::with_base_url_and_timeout(
        "test-key".to_string(),
        url,
        Duration::from_millis(100),
    );

    let err = adapter
        .check_sessions(&["sess-1".to_string()])
        .expect_err("should time out");

    assert!(matches!(err, SnuzeError::DaemonUnavailable(_)));
}
