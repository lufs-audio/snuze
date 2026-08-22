use std::time::{Duration, UNIX_EPOCH};
use snuze::kernel::{
    parse_duration, Alarm, AlarmStatus, DeliveryMode, ExitCode, SnuzeError,
};

#[test]
fn test_parse_duration_valid() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(parse_duration("0s")?, Duration::from_secs(0));
    assert_eq!(parse_duration("30s")?, Duration::from_secs(30));
    assert_eq!(parse_duration("15m")?, Duration::from_mins(15));
    assert_eq!(parse_duration("1h")?, Duration::from_hours(1));
    assert_eq!(parse_duration("90m")?, Duration::from_mins(90));
    assert_eq!(parse_duration("1d")?, Duration::from_hours(24));
    assert_eq!(
        parse_duration("1h30m")?,
        Duration::from_mins(90)
    );
    assert_eq!(
        parse_duration("1h30m15s")?,
        Duration::from_secs(5415)
    );
    assert_eq!(
        parse_duration("2d12h30m5s")?,
        Duration::from_secs(2 * 86400 + 12 * 3600 + 30 * 60 + 5)
    );
    Ok(())
}

#[test]
fn test_parse_duration_invalid() {
    let invalid_inputs = [
        "",
        "   ",
        "-5m",
        "+15m",
        "15z",
        "m15",
        "15",
        "15m30",
        "1h-30m",
        "abc",
        "10sec",
        "999999999999999999999999999999999999999999999999999999999999h",
    ];

    for input in invalid_inputs {
        assert!(
            matches!(parse_duration(input), Err(SnuzeError::InvalidDuration(_))),
            "Expected InvalidDuration for input: {input:?}"
        );
    }
}

#[test]
fn test_exit_code_values() {
    assert_eq!(ExitCode::Success.as_i32(), 0);
    assert_eq!(ExitCode::Usage.as_i32(), 2);
    assert_eq!(ExitCode::TargetUnavailable.as_i32(), 3);
    assert_eq!(ExitCode::BinaryMissing.as_i32(), 4);
    assert_eq!(ExitCode::ContractViolated.as_i32(), 5);
    assert_eq!(ExitCode::DaemonUnavailable.as_i32(), 6);

    assert_eq!(i32::from(ExitCode::Success), 0);
    assert_eq!(i32::from(ExitCode::Usage), 2);
    assert_eq!(i32::from(ExitCode::TargetUnavailable), 3);
    assert_eq!(i32::from(ExitCode::BinaryMissing), 4);
    assert_eq!(i32::from(ExitCode::ContractViolated), 5);
    assert_eq!(i32::from(ExitCode::DaemonUnavailable), 6);
}

#[test]
fn test_snuze_error_to_exit_code_mapping() {
    let cases = [
        (
            SnuzeError::InvalidDuration("bad format".to_string()),
            ExitCode::Usage,
        ),
        (
            SnuzeError::ConflictingFlags("cannot set both".to_string()),
            ExitCode::Usage,
        ),
        (
            SnuzeError::PaneNotFound("w1:p9".to_string()),
            ExitCode::TargetUnavailable,
        ),
        (
            SnuzeError::AlarmNotFound("01HGWY37VZ8Q9998".to_string()),
            ExitCode::TargetUnavailable,
        ),
        (
            SnuzeError::HerdrBinaryMissing,
            ExitCode::BinaryMissing,
        ),
        (
            SnuzeError::PersistFailed("disk write error".to_string()),
            ExitCode::ContractViolated,
        ),
        (
            SnuzeError::DaemonUnavailable("socket not responding".to_string()),
            ExitCode::DaemonUnavailable,
        ),
    ];

    for (err, expected_code) in cases {
        assert_eq!(ExitCode::from(&err), expected_code);
        assert_eq!(ExitCode::from(err), expected_code);
    }
}

#[test]
fn test_alarm_status_serde_representation() -> Result<(), Box<dyn std::error::Error>> {
    let statuses = [
        (AlarmStatus::Pending, "\"pending\""),
        (AlarmStatus::Fired, "\"fired\""),
        (AlarmStatus::Failed, "\"failed\""),
        (AlarmStatus::Missed, "\"missed\""),
        (AlarmStatus::Canceled, "\"canceled\""),
    ];

    for (status, expected_json) in statuses {
        let serialized = serde_json::to_string(&status)?;
        assert_eq!(serialized, expected_json);
        let deserialized: AlarmStatus = serde_json::from_str(&serialized)?;
        assert_eq!(deserialized, status);
    }

    Ok(())
}

#[test]
fn test_delivery_mode_serde_representation() -> Result<(), Box<dyn std::error::Error>> {
    let prompt_mode = DeliveryMode::Prompt {
        text: "wake up agent".to_string(),
    };
    let prompt_json = serde_json::to_string(&prompt_mode)?;
    assert!(prompt_json.contains("\"mode\":\"prompt\""));
    assert!(prompt_json.contains("\"text\":\"wake up agent\""));
    let deserialized_prompt: DeliveryMode = serde_json::from_str(&prompt_json)?;
    assert_eq!(deserialized_prompt, prompt_mode);

    let cmd_mode = DeliveryMode::Command {
        text: "echo 'hello'".to_string(),
    };
    let cmd_json = serde_json::to_string(&cmd_mode)?;
    assert!(cmd_json.contains("\"mode\":\"command\""));
    assert!(cmd_json.contains("\"text\":\"echo 'hello'\""));
    let deserialized_cmd: DeliveryMode = serde_json::from_str(&cmd_json)?;
    assert_eq!(deserialized_cmd, cmd_mode);

    Ok(())
}

#[test]
fn test_alarm_serde_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let created_at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let deadline = created_at + Duration::from_hours(1);
    let fired_at = deadline + Duration::from_secs(1);

    let alarm = Alarm {
        id: "01J5X9M8X2S9C5M94K60M8X2S9".to_string(),
        pane_id: "w1:p3".to_string(),
        created_by_pane: Some("w1:p1".to_string()),
        delivery: DeliveryMode::Prompt {
            text: "Review build output".to_string(),
        },
        label: Some("CI Monitor".to_string()),
        created_at,
        deadline,
        status: AlarmStatus::Fired,
        fired_at: Some(fired_at),
        failure_reason: None,
    };

    let json = serde_json::to_string(&alarm)?;
    let parsed: serde_json::Value = serde_json::from_str(&json)?;
    assert_eq!(
        parsed.get("id").and_then(serde_json::Value::as_str),
        Some("01J5X9M8X2S9C5M94K60M8X2S9")
    );
    assert_eq!(
        parsed.get("pane_id").and_then(serde_json::Value::as_str),
        Some("w1:p3")
    );
    assert_eq!(
        parsed.get("created_by_pane").and_then(serde_json::Value::as_str),
        Some("w1:p1")
    );
    assert_eq!(
        parsed
            .get("delivery")
            .and_then(|d| d.get("mode"))
            .and_then(serde_json::Value::as_str),
        Some("prompt")
    );
    assert_eq!(
        parsed
            .get("delivery")
            .and_then(|d| d.get("text"))
            .and_then(serde_json::Value::as_str),
        Some("Review build output")
    );
    assert_eq!(
        parsed.get("label").and_then(serde_json::Value::as_str),
        Some("CI Monitor")
    );
    assert_eq!(
        parsed.get("status").and_then(serde_json::Value::as_str),
        Some("fired")
    );

    let deserialized: Alarm = serde_json::from_str(&json)?;
    assert_eq!(alarm, deserialized);

    // Minimal alarm without optional fields
    let minimal_alarm = Alarm {
        id: "01J5X9M8X2S9C5M94K60M8X200".to_string(),
        pane_id: "w1:p2".to_string(),
        created_by_pane: None,
        delivery: DeliveryMode::Command {
            text: "cargo test".to_string(),
        },
        label: None,
        created_at,
        deadline,
        status: AlarmStatus::Failed,
        fired_at: None,
        failure_reason: Some("Pane exited unexpectedly".to_string()),
    };

    let minimal_json = serde_json::to_string(&minimal_alarm)?;
    let deserialized_minimal: Alarm = serde_json::from_str(&minimal_json)?;
    assert_eq!(minimal_alarm, deserialized_minimal);

    Ok(())
}
