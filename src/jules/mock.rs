use crate::kernel::{JulesSessionState, SnuzeError};
use std::sync::{Arc, Mutex};

/// Trait defining the contract for checking Jules session states.
pub trait JulesAdapter: Send + Sync {
    /// Query current state for each listed session ID in one batch call.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` or `SnuzeError::JulesCredentialMissing` on failure.
    fn check_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<JulesSessionState>, SnuzeError>;
}

/// Inner state shared (and cloned) across `MockJulesAdapter` instances.
struct MockInner {
    response: Mutex<Result<Vec<JulesSessionState>, SnuzeError>>,
    calls: Mutex<Vec<Vec<String>>>,
}

/// Test double recording every call it receives.
#[derive(Clone)]
pub struct MockJulesAdapter {
    inner: Arc<MockInner>,
}

impl MockJulesAdapter {
    /// Constructs a mock adapter primed to return the given session states.
    #[must_use]
    pub fn new(initial_response: Vec<JulesSessionState>) -> Self {
        Self {
            inner: Arc::new(MockInner {
                response: Mutex::new(Ok(initial_response)),
                calls: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Replace the response returned by subsequent calls without touching call history.
    pub fn set_response(&self, response: Vec<JulesSessionState>) {
        let mut guard = match self.inner.response.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *guard = Ok(response);
    }

    /// Make subsequent calls fail with `SnuzeError::DaemonUnavailable(reason)`.
    pub fn set_failure(&self, reason: &str) {
        let mut guard = match self.inner.response.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *guard = Err(SnuzeError::DaemonUnavailable(reason.to_string()));
    }

    /// Make subsequent calls fail with a specific `SnuzeError`.
    pub fn set_error(&self, error: SnuzeError) {
        let mut guard = match self.inner.response.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *guard = Err(error);
    }

    /// Number of times `check_sessions` has been called so far.
    #[must_use]
    pub fn call_count(&self) -> usize {
        let guard = match self.inner.calls.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        guard.len()
    }

    /// The session ID list passed on the Nth call (0-indexed), if it happened.
    #[must_use]
    pub fn call_args(&self, n: usize) -> Option<Vec<String>> {
        let guard = match self.inner.calls.lock() {
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
    fn check_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<JulesSessionState>, SnuzeError> {
        {
            let mut guard = match self.inner.calls.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            guard.push(session_ids.to_vec());
        }
        let guard = match self.inner.response.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        match &*guard {
            Ok(states) => Ok(states.clone()),
            Err(err) => Err(err.clone()),
        }
    }
}
