//! Unit tests for `TmuxAdapter`, `DeliveryTarget` auto-detection, and the
//! delivery dispatcher introduced by Unit 01.
//!
//! NOTE: `TmuxAdapter` is exercised against a fake `tmux` fixture, so these tests
//! prove argv construction and tmux-error mapping. They are NOT a substitute for
//! an end-to-end test against a real tmux server, which requires a live tmux
//! session and is therefore left to manual/CI-with-tmux verification.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]

use snuze::kernel::{DeliveryMode, DeliveryTarget, SnuzeError};
use snuze::scheduler::{
    select_default_target, CommandAdapter, DeliveryAdapter, DeliveryOutcome, TargetDispatcher,
};
use snuze::tmux::{send_keys_args, TmuxAdapter};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tempfile::TempDir;

// tmux adapter construction reads process-wide environment variables, so tests
// that touch them must not run concurrently.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn fake_tmux_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fake_tmux.sh")
}

fn adapter_with(behavior: &str, log: Option<&Path>, timeout: Duration) -> TmuxAdapter {
    env::set_var("FAKE_TMUX_PATH", fake_tmux_path());
    env::set_var("FAKE_TMUX_BEHAVIOR", behavior);
    match log {
        Some(path) => env::set_var("FAKE_TMUX_LOG", path),
        None => env::remove_var("FAKE_TMUX_LOG"),
    }
    TmuxAdapter::with_timeout(timeout).unwrap()
}

fn read_log(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn tmux_target(target: &str) -> DeliveryTarget {
    DeliveryTarget::Tmux {
        target: target.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Argv construction
// ---------------------------------------------------------------------------

#[test]
fn test_send_keys_args_shape() {
    assert_eq!(
        send_keys_args("%0", "hello"),
        vec![
            "send-keys".to_string(),
            "-t".to_string(),
            "%0".to_string(),
            "hello".to_string(),
            "Enter".to_string(),
        ]
    );
}

#[test]
fn test_deliver_prompt_invokes_send_keys_with_enter() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = TempDir::new().unwrap();
    let log = dir.path().join("argv.log");
    let adapter = adapter_with("delivered", Some(&log), Duration::from_millis(500));

    let outcome = adapter
        .deliver(
            &tmux_target("%3"),
            &DeliveryMode::Prompt {
                text: "wake up agent".to_string(),
            },
        )
        .unwrap();

    assert_eq!(outcome, DeliveryOutcome::Delivered);
    assert_eq!(
        read_log(&log),
        vec!["send-keys", "-t", "%3", "wake up agent", "Enter"]
    );
}

#[test]
fn test_deliver_command_invokes_send_keys_with_enter() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = TempDir::new().unwrap();
    let log = dir.path().join("argv.log");
    let adapter = adapter_with("delivered", Some(&log), Duration::from_millis(500));

    let outcome = adapter
        .deliver(
            &tmux_target("work:0.1"),
            &DeliveryMode::Command {
                text: "cargo test".to_string(),
            },
        )
        .unwrap();

    assert_eq!(outcome, DeliveryOutcome::Delivered);
    assert_eq!(
        read_log(&log),
        vec!["send-keys", "-t", "work:0.1", "cargo test", "Enter"]
    );
}

#[test]
fn test_deliver_keeps_payload_as_a_single_argument() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = TempDir::new().unwrap();
    let log = dir.path().join("argv.log");
    let adapter = adapter_with("delivered", Some(&log), Duration::from_millis(500));

    adapter
        .deliver(
            &tmux_target("%0"),
            &DeliveryMode::Prompt {
                text: "multi word payload with 'quotes' and $VARS".to_string(),
            },
        )
        .unwrap();

    let args = read_log(&log);
    assert_eq!(args.len(), 5, "payload must not be word-split: {args:?}");
    assert_eq!(args[3], "multi word payload with 'quotes' and $VARS");
    assert_eq!(args[4], "Enter");
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

#[test]
fn test_deliver_pane_not_found_maps_to_pane_gone() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = adapter_with("pane_not_found", None, Duration::from_millis(500));

    let outcome = adapter
        .deliver(
            &tmux_target("%99"),
            &DeliveryMode::Prompt {
                text: "hi".to_string(),
            },
        )
        .unwrap();

    assert!(
        matches!(outcome, DeliveryOutcome::PaneGone(_)),
        "{outcome:?}"
    );
}

#[test]
fn test_deliver_session_not_found_maps_to_pane_gone() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = adapter_with("session_not_found", None, Duration::from_millis(500));

    let outcome = adapter
        .deliver(
            &tmux_target("work:0.1"),
            &DeliveryMode::Prompt {
                text: "hi".to_string(),
            },
        )
        .unwrap();

    assert!(
        matches!(outcome, DeliveryOutcome::PaneGone(_)),
        "{outcome:?}"
    );
}

#[test]
fn test_deliver_no_server_maps_to_pane_gone() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = adapter_with("no_server", None, Duration::from_millis(500));

    let outcome = adapter
        .deliver(
            &tmux_target("%0"),
            &DeliveryMode::Prompt {
                text: "hi".to_string(),
            },
        )
        .unwrap();

    assert!(
        matches!(outcome, DeliveryOutcome::PaneGone(_)),
        "{outcome:?}"
    );
}

#[test]
fn test_deliver_generic_error_maps_to_unavailable() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = adapter_with("generic_error", None, Duration::from_millis(500));

    let outcome = adapter
        .deliver(
            &tmux_target("%0"),
            &DeliveryMode::Prompt {
                text: "hi".to_string(),
            },
        )
        .unwrap();

    match outcome {
        DeliveryOutcome::HerdrUnavailable(reason) => {
            assert!(reason.contains("unexpected failure"), "{reason}");
        }
        other => panic!("expected HerdrUnavailable, got {other:?}"),
    }
}

#[test]
fn test_deliver_timeout_is_bounded() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = adapter_with("timeout", None, Duration::from_millis(100));

    let outcome = adapter
        .deliver(
            &tmux_target("%0"),
            &DeliveryMode::Prompt {
                text: "hi".to_string(),
            },
        )
        .unwrap();

    assert_eq!(
        outcome,
        DeliveryOutcome::HerdrUnavailable("timeout".to_string())
    );
}

#[test]
fn test_deliver_rejects_non_tmux_target() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = adapter_with("delivered", None, Duration::from_millis(500));

    let result = adapter.deliver(
        &DeliveryTarget::Herdr {
            pane_id: "w1:p1".to_string(),
        },
        &DeliveryMode::Prompt {
            text: "hi".to_string(),
        },
    );

    assert!(matches!(result, Err(SnuzeError::ConflictingFlags(_))));
}

// ---------------------------------------------------------------------------
// Target auto-detection
// ---------------------------------------------------------------------------

#[test]
fn test_select_default_target_herdr_wins() {
    assert_eq!(
        select_default_target(Some("1"), Some("w1:p3"), Some("/tmp/tmux"), Some("%0")),
        Some(DeliveryTarget::Herdr {
            pane_id: "w1:p3".to_string()
        })
    );
}

#[test]
fn test_select_default_target_tmux_when_no_herdr() {
    assert_eq!(
        select_default_target(None, None, Some("/tmp/tmux-1000/default,1,0"), Some("%4")),
        Some(DeliveryTarget::Tmux {
            target: "%4".to_string()
        })
    );
}

#[test]
fn test_select_default_target_herdr_env_without_pane_falls_through_to_tmux() {
    assert_eq!(
        select_default_target(Some("1"), None, Some("/tmp/tmux"), Some("%1")),
        Some(DeliveryTarget::Tmux {
            target: "%1".to_string()
        })
    );
}

#[test]
fn test_select_default_target_none_when_no_environment() {
    assert_eq!(select_default_target(None, None, None, None), None);
    assert_eq!(select_default_target(Some("0"), None, None, None), None);
    assert_eq!(select_default_target(None, Some("w1:p1"), None, None), None);
    // $HERDR_ENV must be exactly "1".
    assert_eq!(
        select_default_target(Some("true"), Some("w1:p1"), None, None),
        None
    );
    // tmux present but no pane resolvable.
    assert_eq!(
        select_default_target(None, None, Some("/tmp/tmux"), None),
        None
    );
    // Empty values are treated as absent.
    assert_eq!(
        select_default_target(Some("1"), Some("  "), Some("  "), Some("  ")),
        None
    );
}

// ---------------------------------------------------------------------------
// Dispatcher
// ---------------------------------------------------------------------------

#[test]
fn test_dispatcher_routes_tmux_and_handles_wait() {
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = TempDir::new().unwrap();
    let log = dir.path().join("argv.log");
    let tmux = adapter_with("delivered", Some(&log), Duration::from_millis(500));

    // No Herdr adapter installed: Herdr targets report unavailability, not a panic.
    let dispatcher = TargetDispatcher::new(None, Some(Box::new(tmux)));

    let tmux_outcome = dispatcher
        .deliver(
            &tmux_target("%5"),
            &DeliveryMode::Prompt {
                text: "hello".to_string(),
            },
        )
        .unwrap();
    assert_eq!(tmux_outcome, DeliveryOutcome::Delivered);
    assert_eq!(
        read_log(&log),
        vec!["send-keys", "-t", "%5", "hello", "Enter"]
    );

    let wait_outcome = dispatcher
        .deliver(
            &DeliveryTarget::Wait,
            &DeliveryMode::Prompt {
                text: String::new(),
            },
        )
        .unwrap();
    assert_eq!(wait_outcome, DeliveryOutcome::Delivered);

    match dispatcher
        .deliver(
            &DeliveryTarget::Herdr {
                pane_id: "w1:p1".to_string(),
            },
            &DeliveryMode::Prompt {
                text: "hello".to_string(),
            },
        )
        .unwrap()
    {
        DeliveryOutcome::HerdrUnavailable(reason) => {
            assert!(reason.contains("herdr binary"), "{reason}");
        }
        other => panic!("expected HerdrUnavailable, got {other:?}"),
    }
}

#[test]
fn test_command_adapter_runs_command_payload() {
    let adapter = CommandAdapter::with_timeout(Duration::from_secs(5));

    let outcome = adapter
        .deliver(
            &DeliveryTarget::Command {
                command: "exit 0".to_string(),
            },
            &DeliveryMode::Command {
                text: "exit 0".to_string(),
            },
        )
        .unwrap();
    assert_eq!(outcome, DeliveryOutcome::Delivered);

    match adapter
        .deliver(
            &DeliveryTarget::Command {
                command: "exit 3".to_string(),
            },
            &DeliveryMode::Command {
                text: "exit 3".to_string(),
            },
        )
        .unwrap()
    {
        DeliveryOutcome::HerdrUnavailable(reason) => {
            assert!(reason.contains("status 3"), "{reason}");
        }
        other => panic!("expected HerdrUnavailable, got {other:?}"),
    }
}

#[test]
fn test_command_adapter_rejects_non_command_target() {
    let adapter = CommandAdapter::new();
    let result = adapter.deliver(
        &DeliveryTarget::Wait,
        &DeliveryMode::Command {
            text: "true".to_string(),
        },
    );
    assert!(matches!(result, Err(SnuzeError::ConflictingFlags(_))));
}

#[test]
fn test_command_adapter_passes_prompt_as_argument() {
    let adapter = CommandAdapter::with_timeout(Duration::from_secs(5));
    let outcome = adapter
        .deliver(
            &DeliveryTarget::Command {
                command: "true".to_string(),
            },
            &DeliveryMode::Prompt {
                text: "ignored argument".to_string(),
            },
        )
        .unwrap();
    assert_eq!(outcome, DeliveryOutcome::Delivered);
}
