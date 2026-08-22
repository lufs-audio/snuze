#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]

use snuze::snuze_jules::JulesAdapter;
use snuze::snuze_jules::mock::MockJulesAdapter;
use snuze::snuze_kernel::{JulesSessionState, JulesState, SnuzeError};

#[test]
fn test_mock_jules_adapter_returns_configured_response() {
    let initial_response = vec![
        JulesSessionState {
            session_id: "session-1".to_string(),
            state: JulesState::InProgress,
            pull_request_url: None,
        },
        JulesSessionState {
            session_id: "session-2".to_string(),
            state: JulesState::Completed,
            pull_request_url: Some("https://github.com/foo/bar/pull/1".to_string()),
        },
    ];

    let mock = MockJulesAdapter::new(initial_response.clone());
    let result = mock.check_sessions(&["session-1".to_string(), "session-2".to_string()]);

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), initial_response);
}

#[test]
fn test_mock_jules_adapter_records_call_history() {
    let mock = MockJulesAdapter::new(vec![]);

    assert_eq!(mock.call_count(), 0);

    let _ = mock.check_sessions(&["session-1".to_string(), "session-2".to_string()]);
    assert_eq!(mock.call_count(), 1);
    assert_eq!(
        mock.call_args(0).unwrap(),
        vec!["session-1".to_string(), "session-2".to_string()]
    );

    let _ = mock.check_sessions(&["session-3".to_string()]);
    assert_eq!(mock.call_count(), 2);
    assert_eq!(
        mock.call_args(0).unwrap(),
        vec!["session-1".to_string(), "session-2".to_string()]
    );
    assert_eq!(
        mock.call_args(1).unwrap(),
        vec!["session-3".to_string()]
    );

    assert!(mock.call_args(2).is_none());
}

#[test]
fn test_mock_jules_adapter_set_response() {
    let mock = MockJulesAdapter::new(vec![]);

    let result1 = mock.check_sessions(&["session-1".to_string()]);
    assert!(result1.is_ok());
    assert!(result1.unwrap().is_empty());

    let new_response = vec![
        JulesSessionState {
            session_id: "session-1".to_string(),
            state: JulesState::Completed,
            pull_request_url: Some("https://github.com/foo/bar/pull/1".to_string()),
        },
    ];
    mock.set_response(new_response.clone());

    let result2 = mock.check_sessions(&["session-1".to_string()]);
    assert!(result2.is_ok());
    assert_eq!(result2.unwrap(), new_response);

    assert_eq!(mock.call_count(), 2);
}

#[test]
fn test_mock_jules_adapter_set_failure() {
    let mock = MockJulesAdapter::new(vec![]);

    let result1 = mock.check_sessions(&["session-1".to_string()]);
    assert!(result1.is_ok());

    mock.set_failure("API unavailable");

    let result2 = mock.check_sessions(&["session-1".to_string()]);
    assert!(result2.is_err());

    match result2 {
        Err(SnuzeError::DaemonUnavailable(reason)) => assert_eq!(reason, "API unavailable"),
        _ => panic!("Expected SnuzeError::DaemonUnavailable"),
    }

    assert_eq!(mock.call_count(), 2);
}

#[test]
fn test_set_response_does_not_affect_call_history() {
    let mock = MockJulesAdapter::new(vec![]);

    let _ = mock.check_sessions(&["session-a".to_string()]);
    mock.set_response(vec![JulesSessionState {
        session_id: "session-b".to_string(),
        state: JulesState::Failed,
        pull_request_url: None,
    }]);
    let _ = mock.check_sessions(&["session-a".to_string(), "session-b".to_string()]);

    assert_eq!(mock.call_count(), 2);
    assert_eq!(
        mock.call_args(0).unwrap(),
        vec!["session-a".to_string()]
    );
    assert_eq!(
        mock.call_args(1).unwrap(),
        vec!["session-a".to_string(), "session-b".to_string()]
    );
}

#[test]
fn test_default_mock_returns_empty() {
    let mock = MockJulesAdapter::default();
    let result = mock.check_sessions(&["s1".to_string()]);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}
