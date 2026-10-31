use crate::kernel::{DeliveryMode, DeliveryTarget, SnuzeError};
use crate::scheduler::adapter::{
    run_command_with_timeout, DeliveryAdapter, DeliveryOutcome, SubprocessError,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Maximum characters stored in a delivery failure reason.
const MAX_REASON_LEN: usize = 256;

/// Default subprocess timeout for tmux CLI calls (5 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Delivery adapter that injects prompts and commands into a tmux pane with
/// `tmux send-keys -t <target> "<text>" Enter`.
pub struct TmuxAdapter {
    tmux_path: PathBuf,
    call_timeout: Duration,
}

impl TmuxAdapter {
    /// Locates `tmux` on `PATH` with the default 5-second timeout.
    ///
    /// # Errors
    /// Returns `SnuzeError::BinaryMissing` if no `tmux` executable can be found.
    pub fn new() -> Result<Self, SnuzeError> {
        Self::with_timeout(DEFAULT_TIMEOUT)
    }

    /// Locates `tmux` on `PATH` with a custom subprocess timeout.
    ///
    /// # Errors
    /// Returns `SnuzeError::BinaryMissing` if no `tmux` executable can be found.
    pub fn with_timeout(timeout: Duration) -> Result<Self, SnuzeError> {
        let tmux_path =
            find_tmux_in_path().ok_or_else(|| SnuzeError::BinaryMissing("tmux".into()))?;
        Ok(Self {
            tmux_path,
            call_timeout: timeout,
        })
    }

    /// Builds an adapter against a specific binary path and timeout.
    ///
    /// # Errors
    /// Returns `SnuzeError::BinaryMissing` if `tmux_path` is not a file.
    pub fn with_path_and_timeout(
        tmux_path: PathBuf,
        call_timeout: Duration,
    ) -> Result<Self, SnuzeError> {
        if !tmux_path.is_file() {
            return Err(SnuzeError::BinaryMissing("tmux".into()));
        }
        Ok(Self {
            tmux_path,
            call_timeout,
        })
    }

    /// Returns the resolved path to the `tmux` executable.
    #[must_use]
    pub fn tmux_path(&self) -> &Path {
        &self.tmux_path
    }

    /// Returns the configured subprocess call timeout.
    #[must_use]
    pub const fn call_timeout(&self) -> Duration {
        self.call_timeout
    }
}

/// The exact argv `TmuxAdapter` uses to inject `text` into `target`.
///
/// Exposed so the wire format can be asserted without a live tmux server.
#[must_use]
pub fn send_keys_args(target: &str, text: &str) -> Vec<String> {
    vec![
        "send-keys".to_string(),
        "-t".to_string(),
        target.to_string(),
        text.to_string(),
        "Enter".to_string(),
    ]
}

impl DeliveryAdapter for TmuxAdapter {
    fn deliver(
        &self,
        target: &DeliveryTarget,
        delivery: &DeliveryMode,
    ) -> Result<DeliveryOutcome, SnuzeError> {
        let DeliveryTarget::Tmux {
            target: tmux_target,
        } = target
        else {
            return Err(SnuzeError::ConflictingFlags(
                "TmuxAdapter can only deliver to a `tmux:` target".to_string(),
            ));
        };

        let text = match delivery {
            DeliveryMode::Prompt { text } | DeliveryMode::Command { text } => text,
        };

        let mut cmd = Command::new(&self.tmux_path);
        cmd.args(send_keys_args(tmux_target, text));

        // Grace buffer so tmux can return its own error text rather than being killed.
        let subprocess_timeout = self.call_timeout + Duration::from_secs(2);
        let output = match run_command_with_timeout(cmd, subprocess_timeout) {
            Ok(out) => out,
            Err(SubprocessError::Timeout) => {
                return Ok(DeliveryOutcome::HerdrUnavailable("timeout".to_string()));
            }
            Err(SubprocessError::Spawn(e)) => {
                return Ok(DeliveryOutcome::HerdrUnavailable(truncate_reason(
                    &format!("failed to spawn tmux: {e}"),
                )));
            }
            Err(SubprocessError::Io(e)) => {
                return Ok(DeliveryOutcome::HerdrUnavailable(truncate_reason(
                    &format!("io error waiting on tmux: {e}"),
                )));
            }
        };

        if output.status.success() {
            return Ok(DeliveryOutcome::Delivered);
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_target_gone(&stderr) {
            Ok(DeliveryOutcome::PaneGone(truncate_reason(stderr.trim())))
        } else if stderr.trim().is_empty() {
            Ok(DeliveryOutcome::HerdrUnavailable(format!(
                "tmux exited with status {}",
                output
                    .status
                    .code()
                    .map_or_else(|| "signal".to_string(), |c| c.to_string())
            )))
        } else {
            Ok(DeliveryOutcome::HerdrUnavailable(truncate_reason(
                stderr.trim(),
            )))
        }
    }
}

/// Heuristic for "this pane/session/window does not exist" in tmux stderr.
fn is_target_gone(stderr: &str) -> bool {
    let lowered = stderr.to_lowercase();
    [
        "can't find pane",
        "can't find session",
        "can't find window",
        "no such session",
        "no such window",
        "no server running",
        "session not found",
        "pane not found",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

fn truncate_reason(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() > MAX_REASON_LEN {
        let mut truncated: String = trimmed.chars().take(MAX_REASON_LEN - 3).collect();
        truncated.push_str("...");
        truncated
    } else {
        trimmed.to_string()
    }
}

/// Resolves the `tmux` executable, honouring `FAKE_TMUX_PATH` for tests.
fn find_tmux_in_path() -> Option<PathBuf> {
    if let Ok(path_str) = std::env::var("FAKE_TMUX_PATH") {
        let candidate = PathBuf::from(path_str);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("tmux");
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = path.metadata() {
        metadata.is_file() && (metadata.permissions().mode() & 0o111 != 0)
    } else {
        false
    }
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
