pub mod mock;

use crate::kernel::{JulesSessionState, SnuzeError};

/// Interface contract for checking Jules session states.
pub trait JulesAdapter: Send + Sync {
    /// Query current state for each listed session ID in one batch call.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` (reused: "the remote service this
    /// adapter depends on is unreachable") on any network, auth, or malformed-response
    /// failure. Never returns a partial `Vec` — the whole batch succeeds or the whole
    /// call fails, since `AllTerminal` cannot be safely evaluated from partial data.
    fn check_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<JulesSessionState>, SnuzeError>;
}
