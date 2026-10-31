/// Domain errors for `snuze`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SnuzeError {
    #[error("invalid duration: {0}")]
    InvalidDuration(String),
    #[error("conflicting flags: {0}")]
    ConflictingFlags(String),
    #[error("herdr pane not found: {0}")]
    PaneNotFound(String),
    #[error("watched Herdr pane not found: {0}")]
    WatchedPaneNotFound(String),
    #[error("alarm not found: {0}")]
    AlarmNotFound(String),
    #[error("process {0} is not running")]
    ProcessNotFound(u32),
    #[error("alarm {id} did not fire: {reason}")]
    AlarmNotFired {
        /// Alarm identifier that was being awaited.
        id: String,
        /// Why it did not reach `Fired` (terminal status, or wait timeout).
        reason: String,
    },
    #[error("could not persist alarm: {0}")]
    PersistFailed(String),
    #[error("herdr binary not found on PATH")]
    HerdrBinaryMissing,
    #[error("required binary not found on PATH: {0}")]
    BinaryMissing(String),
    #[error("snuze daemon unavailable: {0}")]
    DaemonUnavailable(String),
}

/// Standard process exit codes for `snuze`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    Usage = 2,
    TargetUnavailable = 3,
    BinaryMissing = 4,
    ContractViolated = 5,
    DaemonUnavailable = 6,
}

impl ExitCode {
    /// Returns the integer exit code value.
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

impl From<&SnuzeError> for ExitCode {
    fn from(e: &SnuzeError) -> Self {
        match e {
            SnuzeError::InvalidDuration(_) | SnuzeError::ConflictingFlags(_) => ExitCode::Usage,
            SnuzeError::PaneNotFound(_)
            | SnuzeError::WatchedPaneNotFound(_)
            | SnuzeError::ProcessNotFound(_)
            | SnuzeError::AlarmNotFound(_) => ExitCode::TargetUnavailable,
            SnuzeError::HerdrBinaryMissing | SnuzeError::BinaryMissing(_) => {
                ExitCode::BinaryMissing
            }
            SnuzeError::PersistFailed(_) | SnuzeError::AlarmNotFired { .. } => {
                ExitCode::ContractViolated
            }
            SnuzeError::DaemonUnavailable(_) => ExitCode::DaemonUnavailable,
        }
    }
}

impl From<SnuzeError> for ExitCode {
    fn from(e: SnuzeError) -> Self {
        Self::from(&e)
    }
}

impl From<ExitCode> for i32 {
    fn from(code: ExitCode) -> Self {
        code.as_i32()
    }
}
