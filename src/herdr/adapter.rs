use crate::kernel::{Alarm, DeliveryMode, SnuzeError};
use crate::scheduler::{DeliveryOutcome, HerdrAdapter};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Maximum characters stored in an alarm failure reason.
const MAX_REASON_LEN: usize = 256;

/// Default subprocess timeout for Herdr CLI calls (5 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Production `HerdrAdapter` implementation that shells out to the `herdr` CLI binary.
pub struct RealHerdrAdapter {
    herdr_path: PathBuf,
    call_timeout: Duration,
}

impl RealHerdrAdapter {
    /// Create a new adapter by locating `herdr` on `PATH` with the default 5-second timeout.
    ///
    /// # Errors
    /// Returns `SnuzeError::HerdrBinaryMissing` if the `herdr` executable cannot be found on `PATH`.
    pub fn new() -> Result<Self, SnuzeError> {
        Self::with_timeout(DEFAULT_TIMEOUT)
    }

    /// Create a new adapter by locating `herdr` on `PATH` with a custom subprocess timeout.
    ///
    /// # Errors
    /// Returns `SnuzeError::HerdrBinaryMissing` if the `herdr` executable cannot be found on `PATH`.
    pub fn with_timeout(timeout: Duration) -> Result<Self, SnuzeError> {
        let herdr_path = find_herdr_in_path().ok_or(SnuzeError::HerdrBinaryMissing)?;
        Ok(Self {
            herdr_path,
            call_timeout: timeout,
        })
    }

    /// Create an adapter targeting a specific binary path and custom timeout.
    ///
    /// # Errors
    /// Returns `SnuzeError::HerdrBinaryMissing` if the specified path does not exist or is not a file.
    pub fn with_path_and_timeout(
        herdr_path: PathBuf,
        call_timeout: Duration,
    ) -> Result<Self, SnuzeError> {
        if !herdr_path.is_file() {
            return Err(SnuzeError::HerdrBinaryMissing);
        }
        Ok(Self {
            herdr_path,
            call_timeout,
        })
    }

    /// Returns the resolved path to the `herdr` executable.
    #[must_use]
    pub fn herdr_path(&self) -> &Path {
        &self.herdr_path
    }

    /// Returns the configured subprocess call timeout.
    #[must_use]
    pub const fn call_timeout(&self) -> Duration {
        self.call_timeout
    }

    /// Checks if a Herdr pane currently exists before scheduling an alarm.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` if `herdr` fails unexpectedly (timeout, crash, syntax error, or non-404 failure).
    pub fn pane_exists(&self, pane_id: &str) -> Result<bool, SnuzeError> {
        let mut cmd = Command::new(&self.herdr_path);
        cmd.args(["pane", "get", pane_id]);

        let output = run_command_with_timeout(cmd, self.call_timeout).map_err(|e| match e {
            SubprocessError::Timeout => SnuzeError::DaemonUnavailable("timeout".to_string()),
            SubprocessError::Spawn(err) => {
                SnuzeError::DaemonUnavailable(format!("failed to spawn herdr: {err}"))
            }
            SubprocessError::Io(err) => {
                SnuzeError::DaemonUnavailable(format!("io error running herdr: {err}"))
            }
        })?;

        let status = output.status;
        if status.success() {
            return Ok(true);
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        match status.code() {
            Some(1) => {
                let parsed = parse_herdr_error(&stderr, &stdout);
                if is_not_found(&parsed, &stderr) || is_not_found(&parsed, &stdout) {
                    Ok(false)
                } else {
                    let reason = if parsed.message.is_empty() {
                        truncate_reason(&stderr, MAX_REASON_LEN)
                    } else {
                        parsed.message
                    };
                    Err(SnuzeError::DaemonUnavailable(reason))
                }
            }
            Some(2) => Err(SnuzeError::DaemonUnavailable(
                "snuze invoked herdr incorrectly".to_string(),
            )),
            Some(code) => Err(SnuzeError::DaemonUnavailable(format!(
                "herdr exited with status code {code}"
            ))),
            None => Err(SnuzeError::DaemonUnavailable(
                "herdr process terminated by signal".to_string(),
            )),
        }
    }
}

impl HerdrAdapter for RealHerdrAdapter {
    fn deliver(&self, alarm: &Alarm) -> DeliveryOutcome {
        let mut cmd = Command::new(&self.herdr_path);

        let is_prompt = match &alarm.delivery {
            DeliveryMode::Prompt { text } => {
                let timeout_str = self.call_timeout.as_millis().to_string();
                cmd.args([
                    "agent",
                    "prompt",
                    &alarm.pane_id,
                    text,
                    "--wait",
                    "--timeout",
                    &timeout_str,
                ]);
                true
            }
            DeliveryMode::Command { text } => {
                cmd.args(["pane", "run", &alarm.pane_id, text]);
                false
            }
        };

        let output = match run_command_with_timeout(cmd, self.call_timeout) {
            Ok(out) => out,
            Err(SubprocessError::Timeout) => {
                return DeliveryOutcome::HerdrUnavailable("timeout".to_string());
            }
            Err(SubprocessError::Spawn(e)) => {
                return DeliveryOutcome::HerdrUnavailable(truncate_reason(
                    &format!("failed to spawn herdr: {e}"),
                    MAX_REASON_LEN,
                ));
            }
            Err(SubprocessError::Io(e)) => {
                return DeliveryOutcome::HerdrUnavailable(truncate_reason(
                    &format!("io error waiting on herdr: {e}"),
                    MAX_REASON_LEN,
                ));
            }
        };

        let status = output.status;
        if status.success() {
            return DeliveryOutcome::Delivered;
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);

        match status.code() {
            Some(2) => {
                DeliveryOutcome::HerdrUnavailable("snuze invoked herdr incorrectly".to_string())
            }
            Some(1) => {
                let parsed = parse_herdr_error(&stderr, &stdout);
                if is_prompt {
                    if is_agent_blocked(&parsed) {
                        DeliveryOutcome::AgentBlocked(parsed.message)
                    } else if is_prompt_stalled(&parsed) {
                        DeliveryOutcome::PromptStalled(parsed.message)
                    } else if is_not_found(&parsed, &stderr) || is_not_found(&parsed, &stdout) {
                        DeliveryOutcome::PaneGone(parsed.message)
                    } else {
                        let reason = if parsed.message.is_empty() {
                            truncate_reason(&stderr, MAX_REASON_LEN)
                        } else {
                            parsed.message
                        };
                        DeliveryOutcome::HerdrUnavailable(reason)
                    }
                } else if is_not_found(&parsed, &stderr) || is_not_found(&parsed, &stdout) {
                    DeliveryOutcome::PaneGone(parsed.message)
                } else {
                    let reason = if parsed.message.is_empty() {
                        truncate_reason(&stderr, MAX_REASON_LEN)
                    } else {
                        parsed.message
                    };
                    DeliveryOutcome::HerdrUnavailable(reason)
                }
            }
            Some(code) => {
                let fallback = format!("herdr exited with status {code}");
                let reason = if stderr.trim().is_empty() {
                    fallback
                } else {
                    truncate_reason(&stderr, MAX_REASON_LEN)
                };
                DeliveryOutcome::HerdrUnavailable(reason)
            }
            None => {
                DeliveryOutcome::HerdrUnavailable("herdr process terminated by signal".to_string())
            }
        }
    }
}

enum SubprocessError {
    Timeout,
    Spawn(std::io::Error),
    Io(std::io::Error),
}

fn run_command_with_timeout(
    mut cmd: Command,
    timeout: Duration,
) -> Result<std::process::Output, SubprocessError> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(SubprocessError::Spawn)?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_end(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_end(&mut stderr);
                }
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait(); // reap zombie process
                    return Err(SubprocessError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SubprocessError::Io(e));
            }
        }
    }
}

#[derive(Debug, Default)]
struct ParsedHerdrError {
    code: String,
    message: String,
}

fn parse_herdr_error(stderr: &str, stdout: &str) -> ParsedHerdrError {
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        let code = val
            .get("error")
            .and_then(|e| e.get("code").and_then(|c| c.as_str()))
            .or_else(|| val.get("code").and_then(|c| c.as_str()))
            .unwrap_or_default()
            .to_string();

        let message = val
            .get("error")
            .and_then(|e| e.get("message").and_then(|m| m.as_str()))
            .or_else(|| val.get("message").and_then(|m| m.as_str()))
            .or_else(|| val.get("error").and_then(|e| e.as_str()))
            .unwrap_or_default()
            .to_string();

        if !code.is_empty() || !message.is_empty() {
            let msg = if message.is_empty() {
                code.clone()
            } else {
                message
            };
            return ParsedHerdrError {
                code,
                message: truncate_reason(&msg, MAX_REASON_LEN),
            };
        }
    }

    let trimmed = text.trim();
    ParsedHerdrError {
        code: String::new(),
        message: truncate_reason(trimmed, MAX_REASON_LEN),
    }
}

fn is_not_found(err: &ParsedHerdrError, raw_text: &str) -> bool {
    err.code == "pane_not_found"
        || err.code == "agent_not_found"
        || err.code == "not_found"
        || err.message.to_lowercase().contains("not found")
        || raw_text.to_lowercase().contains("not found")
}

fn is_agent_blocked(err: &ParsedHerdrError) -> bool {
    err.code == "agent_blocked" || err.message.contains("agent_blocked")
}

fn is_prompt_stalled(err: &ParsedHerdrError) -> bool {
    err.code == "agent_prompt_stalled" || err.message.contains("agent_prompt_stalled")
}

fn truncate_reason(s: &str, max_len: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() > max_len {
        let mut truncated: String = trimmed.chars().take(max_len.saturating_sub(3)).collect();
        truncated.push_str("...");
        truncated
    } else {
        trimmed.to_string()
    }
}

fn find_herdr_in_path() -> Option<PathBuf> {
    if let Ok(path_str) = std::env::var("FAKE_HERDR_PATH") {
        let candidate = PathBuf::from(path_str);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("herdr");
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
