use super::JulesAdapter;
use crate::kernel::{JulesSessionState, SnuzeError};
use std::sync::Mutex;

/// Test double recording every call it receives.
pub struct MockJulesAdapter {
    response: Mutex<Result<Vec<JulesSessionState>, String>>, // String stand-in for a cloneable error reason
    calls: Mutex<Vec<Vec<String>>>, // one entry per call, the session_ids it was asked about
}

impl MockJulesAdapter {
    /// Constructs a mock adapter primed to return the given session states.
    #[must_use]
    pub fn new(initial_response: Vec<JulesSessionState>) -> Self {
        Self {
            response: Mutex::new(Ok(initial_response)),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Replace the response returned by subsequent calls without touching call history.
    pub fn set_response(&self, response: Vec<JulesSessionState>) {
        let mut guard = match self.response.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *guard = Ok(response);
    }

    /// Make subsequent calls fail with `SnuzeError::DaemonUnavailable(reason)`.
    pub fn set_failure(&self, reason: &str) {
        let mut guard = match self.response.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *guard = Err(reason.to_string());
    }

    /// Number of times `check_sessions` has been called so far.
    #[must_use]
    pub fn call_count(&self) -> usize {
        let guard = match self.calls.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        guard.len()
    }

    /// The session ID list passed on the Nth call (0-indexed), if it happened.
    #[must_use]
    pub fn call_args(&self, n: usize) -> Option<Vec<String>> {
        let guard = match self.calls.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        guard.get(n).cloned()
    }
}

impl Default for MockJulesAdapter {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl JulesAdapter for MockJulesAdapter {
    fn check_sessions(&self, session_ids: &[String]) -> Result<Vec<JulesSessionState>, SnuzeError> {
        {
            let mut guard = match self.calls.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            guard.push(session_ids.to_vec());
        }
        let guard = match self.response.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        match &*guard {
            Ok(states) => Ok(states.clone()),
            Err(reason) => Err(SnuzeError::DaemonUnavailable(reason.clone())),
        }
    }
}
