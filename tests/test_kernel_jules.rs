#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::duration_suboptimal_units)]
#![allow(clippy::panic)]
#![allow(clippy::unreadable_literal)]

use snuze::kernel::{
    interpolate, Alarm, ExitCode, JulesCondition, JulesSessionState, JulesState, SnuzeError,
    TemplateVars, Trigger,
};
use std::time::{Duration, SystemTime};

#[test]
fn test_jules_condition_empty_slice() {
    assert!(!JulesCondition::AllTerminal.is_satisfied(&[]));
    assert!(!JulesCondition::AnyTerminal.is_satisfied(&[]));
    assert!(!JulesCondition::AnyCompleted.is_satisfied(&[]));
}

#[test]
fn test_jules_condition_spec_11_8_boundary() {
    // SPEC.md §11.8: one Failed + one InProgress satisfies AnyTerminal but not AllTerminal or AnyCompleted
    let states = vec![
        JulesSessionState {
            session_id: "sess-1".to_string(),
            state: JulesState::Failed,
            pull_request_url: None,
        },
        JulesSessionState {
            session_id: "sess-2".to_string(),
            state: JulesState::InProgress,
            pull_request_url: None,
        },
    ];

    assert!(!JulesCondition::AllTerminal.is_satisfied(&states));
    assert!(JulesCondition::AnyTerminal.is_satisfied(&states));
    assert!(!JulesCondition::AnyCompleted.is_satisfied(&states));
}

#[test]
fn test_jules_condition_all_terminal() {
    let wake_worthy_states = vec![
        JulesSessionState {
            session_id: "s1".to_string(),
            state: JulesState::Completed,
            pull_request_url: Some("https://github.com/org/repo/pull/1".to_string()),
        },
        JulesSessionState {
            session_id: "s2".to_string(),
            state: JulesState::Failed,
            pull_request_url: None,
        },
        JulesSessionState {
            session_id: "s3".to_string(),
            state: JulesState::AwaitingUserFeedback,
            pull_request_url: None,
        },
        JulesSessionState {
            session_id: "s4".to_string(),
            state: JulesState::Paused,
            pull_request_url: None,
        },
    ];

    assert!(JulesCondition::AllTerminal.is_satisfied(&wake_worthy_states));
    assert!(JulesCondition::AnyTerminal.is_satisfied(&wake_worthy_states));
    assert!(JulesCondition::AnyCompleted.is_satisfied(&wake_worthy_states));

    // If one is not wake-worthy:
    let mut mixed = wake_worthy_states.clone();
    mixed.push(JulesSessionState {
        session_id: "s5".to_string(),
        state: JulesState::Queued,
        pull_request_url: None,
    });
    assert!(!JulesCondition::AllTerminal.is_satisfied(&mixed));
    assert!(JulesCondition::AnyTerminal.is_satisfied(&mixed));

    let mut mixed_planning = wake_worthy_states.clone();
    mixed_planning.push(JulesSessionState {
        session_id: "s6".to_string(),
        state: JulesState::Planning,
        pull_request_url: None,
    });
    assert!(!JulesCondition::AllTerminal.is_satisfied(&mixed_planning));

    let mut mixed_approval = wake_worthy_states.clone();
    mixed_approval.push(JulesSessionState {
        session_id: "s7".to_string(),
        state: JulesState::AwaitingPlanApproval,
        pull_request_url: None,
    });
    assert!(!JulesCondition::AllTerminal.is_satisfied(&mixed_approval));

    let mut mixed_unknown = wake_worthy_states;
    mixed_unknown.push(JulesSessionState {
        session_id: "s8".to_string(),
        state: JulesState::Unknown("NEW_STATE".to_string()),
        pull_request_url: None,
    });
    assert!(!JulesCondition::AllTerminal.is_satisfied(&mixed_unknown));
}

#[test]
fn test_jules_condition_any_completed() {
    let failed_only = vec![
        JulesSessionState {
            session_id: "s1".to_string(),
            state: JulesState::Failed,
            pull_request_url: None,
        },
        JulesSessionState {
            session_id: "s2".to_string(),
            state: JulesState::Paused,
            pull_request_url: None,
        },
    ];
    assert!(JulesCondition::AllTerminal.is_satisfied(&failed_only));
    assert!(JulesCondition::AnyTerminal.is_satisfied(&failed_only));
    assert!(!JulesCondition::AnyCompleted.is_satisfied(&failed_only));

    let with_completed = vec![
        JulesSessionState {
            session_id: "s1".to_string(),
            state: JulesState::Failed,
            pull_request_url: None,
        },
        JulesSessionState {
            session_id: "s2".to_string(),
            state: JulesState::Completed,
            pull_request_url: Some("https://github.com/org/repo/pull/2".to_string()),
        },
    ];
    assert!(JulesCondition::AnyCompleted.is_satisfied(&with_completed));
}

#[test]
fn test_template_interpolation_all_vars() {
    let vars = TemplateVars {
        duration: Some("12m34s".to_string()),
        session_id: Some("123456789".to_string()),
        status: Some("COMPLETED".to_string()),
        pr_urls: Some("https://github.com/org/repo/pull/42".to_string()),
    };

    let template = "Session {session_id} finished ({status}) after {duration}. PR: {pr_urls}";
    let result = interpolate(template, &vars);
    assert_eq!(
        result,
        "Session 123456789 finished (COMPLETED) after 12m34s. PR: https://github.com/org/repo/pull/42"
    );
}

#[test]
fn test_template_interpolation_unknown_and_none_vars() {
    let vars = TemplateVars {
        duration: None,
        session_id: Some("sess-abc".to_string()),
        status: None,
        pr_urls: None,
    };

    let template =
        "Alert: {session_id} took {duration}, status={status}, unknown={foo} and pr={pr_urls}";
    let result = interpolate(template, &vars);
    assert_eq!(
        result,
        "Alert: sess-abc took {duration}, status={status}, unknown={foo} and pr={pr_urls}"
    );
}

#[test]
fn test_template_interpolation_repeated_tokens() {
    let vars = TemplateVars {
        duration: Some("5m".to_string()),
        session_id: Some("sess-1".to_string()),
        status: Some("FAILED".to_string()),
        pr_urls: None,
    };

    let template =
        "{session_id} is {status}! Repeating: {session_id} status is {status} in {duration}.";
    let result = interpolate(template, &vars);
    assert_eq!(
        result,
        "sess-1 is FAILED! Repeating: sess-1 status is FAILED in 5m."
    );
}

#[test]
fn test_alarm_deserialization_backward_compatibility() {
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

    let alarm: Alarm = serde_json::from_str(old_json).expect("should deserialize old alarm json");
    assert_eq!(alarm.schema_version, 1);
    assert_eq!(alarm.trigger, Trigger::Deadline);
}

#[test]
fn test_trigger_jules_condition_serde_roundtrip() {
    let timeout_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1700001000);
    let trigger = Trigger::JulesCondition {
        session_ids: vec!["s1".to_string(), "s2".to_string()],
        condition: JulesCondition::AnyTerminal,
        poll_interval_secs: 30,
        timeout_at,
        last_known: vec![
            JulesSessionState {
                session_id: "s1".to_string(),
                state: JulesState::InProgress,
                pull_request_url: None,
            },
            JulesSessionState {
                session_id: "s2".to_string(),
                state: JulesState::Unknown("EXPERIMENTAL_STAGE".to_string()),
                pull_request_url: Some("https://github.com/org/repo/pull/99".to_string()),
            },
        ],
    };

    let serialized = serde_json::to_string(&trigger).expect("should serialize trigger");
    let deserialized: Trigger =
        serde_json::from_str(&serialized).expect("should deserialize trigger");
    assert_eq!(trigger, deserialized);

    // Verify condition serde naming
    assert!(serialized.contains(r#""condition":"any-terminal""#));
    assert!(serialized.contains(r#""kind":"jules_condition""#));
}

#[test]
fn test_trigger_deadline_serde() {
    let trigger = Trigger::Deadline;
    let serialized = serde_json::to_string(&trigger).expect("should serialize deadline trigger");
    assert_eq!(serialized, r#"{"kind":"deadline"}"#);
    let deserialized: Trigger =
        serde_json::from_str(&serialized).expect("should deserialize deadline trigger");
    assert_eq!(trigger, deserialized);
}

#[test]
fn test_jules_state_serde_and_unknown_fallback() {
    let cases = [
        ("Queued", JulesState::Queued),
        ("QUEUED", JulesState::Queued),
        ("Planning", JulesState::Planning),
        ("PLANNING", JulesState::Planning),
        ("AwaitingPlanApproval", JulesState::AwaitingPlanApproval),
        ("AWAITING_PLAN_APPROVAL", JulesState::AwaitingPlanApproval),
        ("InProgress", JulesState::InProgress),
        ("IN_PROGRESS", JulesState::InProgress),
        ("Completed", JulesState::Completed),
        ("COMPLETED", JulesState::Completed),
        ("AwaitingUserFeedback", JulesState::AwaitingUserFeedback),
        ("AWAITING_USER_FEEDBACK", JulesState::AwaitingUserFeedback),
        ("Paused", JulesState::Paused),
        ("PAUSED", JulesState::Paused),
        ("Failed", JulesState::Failed),
        ("FAILED", JulesState::Failed),
        (
            "SOME_FUTURE_STATE",
            JulesState::Unknown("SOME_FUTURE_STATE".to_string()),
        ),
    ];

    for (raw_str, expected_state) in cases {
        let json_input = format!(r#""{raw_str}""#);
        let parsed: JulesState = serde_json::from_str(&json_input)
            .unwrap_or_else(|e| panic!("Failed to deserialize {raw_str:?}: {e}"));
        assert_eq!(parsed, expected_state, "Mismatch for input {raw_str:?}");
    }

    // Serializing standard states should produce UPPERCASE strings matching as_str()
    for state in [
        JulesState::Queued,
        JulesState::Planning,
        JulesState::AwaitingPlanApproval,
        JulesState::InProgress,
        JulesState::Completed,
        JulesState::AwaitingUserFeedback,
        JulesState::Paused,
        JulesState::Failed,
    ] {
        let serialized = serde_json::to_string(&state).expect("serialize");
        assert_eq!(serialized, format!("\"{}\"", state.as_str()));
    }

    // Serializing unknown state should produce the original string
    let unknown_state = JulesState::Unknown("MY_CUSTOM_STATE".to_string());
    let serialized = serde_json::to_string(&unknown_state).expect("serialize unknown");
    assert_eq!(serialized, r#""MY_CUSTOM_STATE""#);
    let roundtripped: JulesState = serde_json::from_str(&serialized).expect("deserialize unknown");
    assert_eq!(roundtripped, unknown_state);
}

#[test]
fn test_jules_credential_missing_exit_code() {
    let err = SnuzeError::JulesCredentialMissing;
    assert_eq!(ExitCode::from(&err), ExitCode::BinaryMissing);
    assert_eq!(ExitCode::from(err), ExitCode::BinaryMissing);
}

#[test]
fn test_jules_session_not_found_exit_code() {
    let err = SnuzeError::JulesSessionNotFound("sess-123".to_string());
    assert_eq!(ExitCode::from(&err), ExitCode::TargetUnavailable);
    assert_eq!(err.to_string(), "Jules session not found: sess-123");
}
