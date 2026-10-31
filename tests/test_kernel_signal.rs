#![allow(clippy::unwrap_used)]

use snuze::kernel::{
    herdr_status_satisfied, Alarm, ExitCode, HerdrWatchStatus, SnuzeError, Trigger,
};
use std::time::SystemTime;

#[test]
fn test_herdr_status_satisfied() {
    let watch_for = vec![HerdrWatchStatus::Done, HerdrWatchStatus::Working];

    // Satisfied cases
    assert!(herdr_status_satisfied(&watch_for, HerdrWatchStatus::Done));
    assert!(herdr_status_satisfied(
        &watch_for,
        HerdrWatchStatus::Working
    ));

    // Not satisfied cases
    assert!(!herdr_status_satisfied(&watch_for, HerdrWatchStatus::Idle));
    assert!(!herdr_status_satisfied(
        &watch_for,
        HerdrWatchStatus::Blocked
    ));

    // Empty watch list
    assert!(!herdr_status_satisfied(&[], HerdrWatchStatus::Done));
}

#[test]
fn test_serde_roundtrip_herdr_status() {
    let trigger = Trigger::HerdrStatus {
        watched_pane_id: "pane_123".to_string(),
        watch_for: vec![HerdrWatchStatus::Done, HerdrWatchStatus::Blocked],
        timeout_at: SystemTime::UNIX_EPOCH,
        last_known: Some(HerdrWatchStatus::Working),
    };

    let serialized = serde_json::to_string(&trigger).unwrap();
    let expected = r#"{"kind":"herdr_status","watched_pane_id":"pane_123","watch_for":["done","blocked"],"timeout_at":{"secs_since_epoch":0,"nanos_since_epoch":0},"last_known":"working"}"#;
    assert_eq!(serialized, expected);

    let deserialized: Trigger = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized, trigger);
}

#[test]
fn test_serde_roundtrip_external_signal() {
    let trigger = Trigger::ExternalSignal {
        match_key: "sig_456".to_string(),
        timeout_at: SystemTime::UNIX_EPOCH,
    };

    let serialized = serde_json::to_string(&trigger).unwrap();
    let expected = r#"{"kind":"external_signal","match_key":"sig_456","timeout_at":{"secs_since_epoch":0,"nanos_since_epoch":0}}"#;
    assert_eq!(serialized, expected);

    let deserialized: Trigger = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized, trigger);
}

#[test]
fn test_alarm_trigger_backward_compat() {
    // Missing trigger falls back to Default (Deadline)
    let old_json = r#"{
        "id": "01J5X9M8X2S9C5M94K60M8X2S9",
        "pane_id": "w1:p3",
        "created_by_pane": "w1:p1",
        "delivery": {
            "mode": "prompt",
            "text": "wake up"
        },
        "label": "old alarm",
        "created_at": {
            "secs_since_epoch": 1700000000,
            "nanos_since_epoch": 0
        },
        "deadline": {
            "secs_since_epoch": 1700000300,
            "nanos_since_epoch": 0
        },
        "status": "pending",
        "fired_at": null,
        "failure_reason": null
    }"#;

    let alarm: Alarm = serde_json::from_str(old_json).unwrap();
    assert_eq!(alarm.trigger, Trigger::Deadline);
}

#[test]
fn test_error_mapping() {
    let err = SnuzeError::WatchedPaneNotFound("pane_123".to_string());

    // Test Display format
    assert_eq!(err.to_string(), "watched Herdr pane not found: pane_123");

    // Verify it doesn't match standard PaneNotFound display
    let other_err = SnuzeError::PaneNotFound("pane_123".to_string());
    assert_ne!(err.to_string(), other_err.to_string());

    // Verify mapped ExitCode is TargetUnavailable
    let code: ExitCode = (&err).into();
    assert_eq!(code, ExitCode::TargetUnavailable);
}
