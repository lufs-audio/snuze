//! Unit tests for `RealHerdrAdapter` using a fake herdr executable fixture.
//!
//! NOTE: These unit tests verify the adapter's subprocess execution, timeout bounding,
//! and exit code mapping logic given known herdr CLI outputs. They are NOT a substitute
//! for the live-Herdr test suite (`SPEC.md` §9), which validates end-to-end integration
//! against the real, running herdr daemon and actual panes.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]

#![allow(clippy::panic)]

use snuze::snuze_herdr::RealHerdrAdapter;
use snuze::snuze_kernel::{Alarm, AlarmStatus, DeliveryMode, SnuzeError, Trigger};
use snuze::snuze_scheduler::{DeliveryOutcome, HerdrAdapter};
use std::env;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

// Prevent concurrent test runs from colliding on process-wide env variables.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn fake_script_path() -> PathBuf {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(manifest_dir)
        .join("tests")
        .join("fixtures")
        .join("fake_herdr.sh")
}

fn create_test_adapter(behavior: &str, timeout: Duration) -> RealHerdrAdapter {
    env::set_var("FAKE_HERDR_PATH", fake_script_path());
    env::set_var("FAKE_HERDR_BEHAVIOR", behavior);
    RealHerdrAdapter::with_timeout(timeout).unwrap()
}

fn sample_alarm(delivery: DeliveryMode) -> Alarm {
    Alarm {
        id: "01J5K4E6F8ZZZZZZZZZZZZZZZZ".to_string(),
        pane_id: "w1:p1".to_string(),
        created_by_pane: Some("w1:p0".to_string()),
        delivery,
        label: Some("test-alarm".to_string()),
        created_at: SystemTime::now(),
        deadline: SystemTime::now() + Duration::from_mins(1),
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::Deadline,
        schema_version: 1,
    }
}

#[test]
fn test_missing_herdr_binary() {
    let _guard = ENV_LOCK.lock().unwrap();
    let old_path = env::var("PATH").unwrap_or_default();
    let old_fake = env::var("FAKE_HERDR_PATH").ok();

    env::remove_var("FAKE_HERDR_PATH");
    env::set_var("PATH", "");

    let result = RealHerdrAdapter::new();
    assert!(matches!(result, Err(SnuzeError::HerdrBinaryMissing)));

    // Restore
    env::set_var("PATH", old_path);
    if let Some(f) = old_fake {
        env::set_var("FAKE_HERDR_PATH", f);
    }
}

#[test]
fn test_pane_exists_true() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("pane_exists_true", Duration::from_millis(500));

    let exists = adapter.pane_exists("w1:p1").unwrap();
    assert!(exists);
}

#[test]
fn test_pane_exists_false_on_not_found() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("pane_not_found", Duration::from_millis(500));

    let exists = adapter.pane_exists("w1:p99").unwrap();
    assert!(!exists);

    // Shorthand format
    env::set_var("FAKE_HERDR_BEHAVIOR", "pane_not_found_shorthand");
    let exists = adapter.pane_exists("w1:p99").unwrap();
    assert!(!exists);

    // Plain text format
    env::set_var("FAKE_HERDR_BEHAVIOR", "pane_not_found_string");
    let exists = adapter.pane_exists("w1:p99").unwrap();
    assert!(!exists);
}

#[test]
fn test_pane_exists_error_on_daemon_failure() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("generic_error", Duration::from_millis(500));

    let result = adapter.pane_exists("w1:p1");
    assert!(result.is_err());
    assert!(matches!(result, Err(SnuzeError::DaemonUnavailable(_))));
}

#[test]
fn test_pane_exists_syntax_error() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("syntax_error", Duration::from_millis(500));

    let result = adapter.pane_exists("w1:p1");
    match result {
        Err(SnuzeError::DaemonUnavailable(msg)) => {
            assert!(msg.contains("snuze invoked herdr incorrectly"));
        }
        other => panic!("expected DaemonUnavailable with invocation error, got {other:?}"),
    }
}

#[test]
fn test_deliver_prompt_delivered() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("delivered", Duration::from_millis(500));
    let alarm = sample_alarm(DeliveryMode::Prompt {
        text: "wake up agent".to_string(),
    });

    let outcome = adapter.deliver(&alarm);
    assert_eq!(outcome, DeliveryOutcome::Delivered);
}

#[test]
fn test_deliver_prompt_agent_blocked() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("agent_blocked", Duration::from_millis(500));
    let alarm = sample_alarm(DeliveryMode::Prompt {
        text: "wake up agent".to_string(),
    });

    let outcome = adapter.deliver(&alarm);
    match outcome {
        DeliveryOutcome::AgentBlocked(reason) => {
            assert!(reason.contains("busy"));
        }
        other => panic!("expected AgentBlocked, got {other:?}"),
    }
}

#[test]
fn test_deliver_prompt_stalled() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("agent_prompt_stalled", Duration::from_millis(500));
    let alarm = sample_alarm(DeliveryMode::Prompt {
        text: "wake up agent".to_string(),
    });

    let outcome = adapter.deliver(&alarm);
    match outcome {
        DeliveryOutcome::PromptStalled(reason) => {
            assert!(reason.contains("no lifecycle change"));
        }
        other => panic!("expected PromptStalled, got {other:?}"),
    }
}

#[test]
fn test_deliver_prompt_pane_gone() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("pane_not_found", Duration::from_millis(500));
    let alarm = sample_alarm(DeliveryMode::Prompt {
        text: "wake up agent".to_string(),
    });

    let outcome = adapter.deliver(&alarm);
    match outcome {
        DeliveryOutcome::PaneGone(reason) => {
            assert!(reason.contains("not found"));
        }
        other => panic!("expected PaneGone, got {other:?}"),
    }
}

#[test]
fn test_deliver_command_delivered() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("delivered", Duration::from_millis(500));
    let alarm = sample_alarm(DeliveryMode::Command {
        text: "echo wake up".to_string(),
    });

    let outcome = adapter.deliver(&alarm);
    assert_eq!(outcome, DeliveryOutcome::Delivered);
}

#[test]
fn test_deliver_command_pane_gone() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("pane_not_found", Duration::from_millis(500));
    let alarm = sample_alarm(DeliveryMode::Command {
        text: "echo wake up".to_string(),
    });

    let outcome = adapter.deliver(&alarm);
    match outcome {
        DeliveryOutcome::PaneGone(reason) => {
            assert!(reason.contains("not found"));
        }
        other => panic!("expected PaneGone, got {other:?}"),
    }
}

#[test]
fn test_deliver_command_never_produces_agent_blocked_or_stalled() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("agent_blocked", Duration::from_millis(500));
    let alarm = sample_alarm(DeliveryMode::Command {
        text: "echo wake up".to_string(),
    });

    // Command mode must never return AgentBlocked or PromptStalled
    let outcome = adapter.deliver(&alarm);
    assert!(matches!(outcome, DeliveryOutcome::HerdrUnavailable(_)));

    env::set_var("FAKE_HERDR_BEHAVIOR", "agent_prompt_stalled");
    let outcome2 = adapter.deliver(&alarm);
    assert!(matches!(outcome2, DeliveryOutcome::HerdrUnavailable(_)));
}

#[test]
fn test_deliver_syntax_error() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("syntax_error", Duration::from_millis(500));
    let alarm = sample_alarm(DeliveryMode::Prompt {
        text: "hello".to_string(),
    });

    let outcome = adapter.deliver(&alarm);
    match outcome {
        DeliveryOutcome::HerdrUnavailable(reason) => {
            assert!(reason.contains("snuze invoked herdr incorrectly"));
        }
        other => panic!("expected HerdrUnavailable with invocation error, got {other:?}"),
    }
}

#[test]
fn test_deliver_timeout() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Use a short 100ms timeout against a script that sleeps 5s
    let adapter = create_test_adapter("timeout", Duration::from_millis(100));
    let alarm = sample_alarm(DeliveryMode::Prompt {
        text: "hello".to_string(),
    });

    let outcome = adapter.deliver(&alarm);
    assert_eq!(
        outcome,
        DeliveryOutcome::HerdrUnavailable("timeout".to_string())
    );
}

#[test]
fn test_deliver_truncation() {
    let _guard = ENV_LOCK.lock().unwrap();
    let adapter = create_test_adapter("unparseable_error", Duration::from_millis(500));
    let alarm = sample_alarm(DeliveryMode::Prompt {
        text: "hello".to_string(),
    });

    let outcome = adapter.deliver(&alarm);
    match outcome {
        DeliveryOutcome::HerdrUnavailable(reason) => {
            assert!(reason.ends_with("..."));
            assert!(reason.chars().count() <= 256);
        }
        other => panic!("expected HerdrUnavailable, got {other:?}"),
    }
}
