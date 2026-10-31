use crate::kernel::{DeliveryMode, DeliveryTarget, SnuzeError};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Outcome of attempting to deliver an alarm payload to a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryOutcome {
    /// Alarm delivered successfully to its target.
    Delivered,
    /// Target agent was blocked on another interactive input or state (`--prompt` only).
    AgentBlocked(String),
    /// Target agent prompt did not trigger an observed lifecycle change within the window (`--prompt` only).
    PromptStalled(String),
    /// Target pane or destination no longer resolves or does not exist.
    PaneGone(String),
    /// The delivery backend (Herdr, tmux, …) was unavailable, timed out, or the invocation was malformed.
    HerdrUnavailable(String),
}

impl DeliveryOutcome {
    /// Reason string for a non-`Delivered` outcome, or `None` when delivery succeeded.
    #[must_use]
    pub fn failure_reason(&self) -> Option<&str> {
        match self {
            Self::Delivered => None,
            Self::AgentBlocked(reason)
            | Self::PromptStalled(reason)
            | Self::PaneGone(reason)
            | Self::HerdrUnavailable(reason) => Some(reason),
        }
    }
}

/// Pluggable delivery backend.
///
/// One adapter owns one transport (Herdr CLI, tmux, a shell command). The
/// scheduler only ever sees this trait, so adding a multiplexer means adding an
/// adapter, not editing the scheduler.
pub trait DeliveryAdapter: Send + Sync {
    /// Deliver `delivery` to `target`.
    ///
    /// # Errors
    ///
    /// Returns `SnuzeError` when the invocation is impossible for this adapter
    /// (for example, being handed a `DeliveryTarget` variant it does not own).
    /// A delivery that was attempted but failed is reported as an `Ok` outcome
    /// such as [`DeliveryOutcome::PaneGone`], not as an error.
    fn deliver(
        &self,
        target: &DeliveryTarget,
        delivery: &DeliveryMode,
    ) -> Result<DeliveryOutcome, SnuzeError>;
}

/// The delivery target implied by the ambient environment, if any.
///
/// Precedence matches the phase spec: `$HERDR_ENV=1` plus `$HERDR_PANE_ID` wins,
/// then `$TMUX` (resolving the current pane via `$TMUX_PANE`, or `tmux
/// display-message` when that is unset). Returns `None` when neither is usable,
/// which the CLI turns into a `USAGE` error.
#[must_use]
pub fn detect_default_target() -> Option<DeliveryTarget> {
    let tmux_pane = std::env::var("TMUX_PANE")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let explicit = select_default_target(
        std::env::var("HERDR_ENV").ok().as_deref(),
        std::env::var("HERDR_PANE_ID").ok().as_deref(),
        std::env::var("TMUX").ok().as_deref(),
        tmux_pane.as_deref(),
    );
    if explicit.is_some() {
        return explicit;
    }

    // `$TMUX` is set but `$TMUX_PANE` was not: ask tmux which pane we are in.
    if std::env::var("TMUX").is_ok_and(|v| !v.trim().is_empty()) {
        return current_tmux_pane().map(|target| DeliveryTarget::Tmux { target });
    }

    None
}

/// Pure precedence rule behind [`detect_default_target`], split out so it can be
/// unit-tested without mutating process-global environment state.
#[must_use]
pub fn select_default_target(
    herdr_env: Option<&str>,
    herdr_pane_id: Option<&str>,
    tmux: Option<&str>,
    tmux_pane: Option<&str>,
) -> Option<DeliveryTarget> {
    let herdr_env_on = herdr_env.is_some_and(|v| v.trim() == "1");
    if herdr_env_on {
        if let Some(pane_id) = herdr_pane_id.filter(|p| !p.trim().is_empty()) {
            return Some(DeliveryTarget::Herdr {
                pane_id: pane_id.to_string(),
            });
        }
    }

    let tmux_on = tmux.is_some_and(|v| !v.trim().is_empty());
    if tmux_on {
        if let Some(target) = tmux_pane.filter(|p| !p.trim().is_empty()) {
            return Some(DeliveryTarget::Tmux {
                target: target.to_string(),
            });
        }
    }

    None
}

/// Resolves the current tmux pane id by asking the `tmux` binary.
fn current_tmux_pane() -> Option<String> {
    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{pane_id}"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let pane = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if pane.is_empty() {
        None
    } else {
        Some(pane)
    }
}

/// Routes an alarm to the adapter that owns its [`DeliveryTarget`] variant.
///
/// Adapters for Herdr and tmux are optional because neither binary is required
/// for the daemon to run: a machine with only tmux installed can still deliver,
/// and vice versa. A missing backend surfaces as an `Ok(HerdrUnavailable)` at
/// delivery time rather than preventing daemon start-up.
pub struct TargetDispatcher {
    herdr: Option<Box<dyn DeliveryAdapter>>,
    tmux: Option<Box<dyn DeliveryAdapter>>,
    command: Box<dyn DeliveryAdapter>,
}

impl TargetDispatcher {
    /// Builds a dispatcher from optional Herdr and tmux adapters.
    #[must_use]
    pub fn new(
        herdr: Option<Box<dyn DeliveryAdapter>>,
        tmux: Option<Box<dyn DeliveryAdapter>>,
    ) -> Self {
        Self {
            herdr,
            tmux,
            command: Box::new(CommandAdapter::new()),
        }
    }
}

impl DeliveryAdapter for TargetDispatcher {
    fn deliver(
        &self,
        target: &DeliveryTarget,
        delivery: &DeliveryMode,
    ) -> Result<DeliveryOutcome, SnuzeError> {
        match target {
            DeliveryTarget::Herdr { .. } => match &self.herdr {
                Some(adapter) => adapter.deliver(target, delivery),
                None => Ok(DeliveryOutcome::HerdrUnavailable(
                    "herdr binary not found on PATH".to_string(),
                )),
            },
            DeliveryTarget::Tmux { .. } => match &self.tmux {
                Some(adapter) => adapter.deliver(target, delivery),
                None => Ok(DeliveryOutcome::HerdrUnavailable(
                    "tmux binary not found on PATH".to_string(),
                )),
            },
            DeliveryTarget::Command { .. } => self.command.deliver(target, delivery),
            // Nothing to deliver: the alarm is observed through `snuze wait`.
            DeliveryTarget::Wait => Ok(DeliveryOutcome::Delivered),
        }
    }
}

/// Adapter that runs the alarm payload as a shell command via `sh -c`.
pub struct CommandAdapter {
    call_timeout: Duration,
}

/// Default subprocess timeout for command delivery (30 seconds).
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

impl CommandAdapter {
    /// Creates a command adapter with the default 30-second timeout.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            call_timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }

    /// Creates a command adapter with a custom subprocess timeout.
    #[must_use]
    pub const fn with_timeout(call_timeout: Duration) -> Self {
        Self { call_timeout }
    }
}

impl Default for CommandAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl DeliveryAdapter for CommandAdapter {
    fn deliver(
        &self,
        target: &DeliveryTarget,
        delivery: &DeliveryMode,
    ) -> Result<DeliveryOutcome, SnuzeError> {
        let DeliveryTarget::Command { .. } = target else {
            return Err(SnuzeError::ConflictingFlags(
                "CommandAdapter can only deliver to a `command:` target".to_string(),
            ));
        };

        // A prompt is passed to the command as its final argument; a command
        // payload is the command body itself.
        let (body, extra_arg) = match delivery {
            DeliveryMode::Command { text } => (text.clone(), None),
            DeliveryMode::Prompt { text } => (target.destination(), Some(text.clone())),
        };

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(&body);
        if let Some(arg) = extra_arg {
            cmd.arg(arg);
        }

        match run_command_with_timeout(cmd, self.call_timeout) {
            Ok(output) if output.status.success() => Ok(DeliveryOutcome::Delivered),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let code = output
                    .status
                    .code()
                    .map_or_else(|| "signal".to_string(), |c| c.to_string());
                Ok(DeliveryOutcome::HerdrUnavailable(format!(
                    "command exited with status {code}: {}",
                    stderr.trim()
                )))
            }
            Err(SubprocessError::Timeout) => Ok(DeliveryOutcome::HerdrUnavailable(
                "command delivery timed out".to_string(),
            )),
            Err(SubprocessError::Spawn(e)) => Ok(DeliveryOutcome::HerdrUnavailable(format!(
                "failed to spawn command: {e}"
            ))),
            Err(SubprocessError::Io(e)) => Ok(DeliveryOutcome::HerdrUnavailable(format!(
                "io error running command: {e}"
            ))),
        }
    }
}

/// Failure modes of [`run_command_with_timeout`].
pub(crate) enum SubprocessError {
    /// The child exceeded its deadline and was killed.
    Timeout,
    /// The child could not be spawned.
    Spawn(std::io::Error),
    /// Waiting on the child failed.
    Io(std::io::Error),
}

/// Runs `cmd` to completion, killing it if it outlives `timeout`.
///
/// Shared by every delivery adapter that shells out, so timeout and
/// stdout/stderr handling stay identical across transports.
pub(crate) fn run_command_with_timeout(
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
