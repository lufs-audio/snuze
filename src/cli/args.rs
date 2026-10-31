use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "snuze",
    version,
    about = "Asynchronous agent alarm clock primitive for Herdr panes"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// Schedule a future wake-up alarm
    Set(SetArgs),
    /// List scheduled and historical alarms
    List(ListArgs),
    /// Inspect details of a specific alarm
    Status(StatusArgs),
    /// Cancel a pending alarm
    Cancel(CancelArgs),
    /// Control the background snuze daemon
    Daemon(DaemonArgs),
    /// Manage and drain the local event outbox
    Events(EventsArgs),
    /// Run diagnostic health checks
    Doctor(DoctorArgs),
    /// Signal a pending external alarm
    Signal(SignalArgs),
    /// Block until an alarm reaches a terminal state
    Wait(WaitArgs),
}

#[derive(Args, Debug, Clone)]
pub struct SignalArgs {
    /// Match key to resolve the alarm
    pub match_key: String,

    /// Validate without signaling
    #[arg(long)]
    pub dry_run: bool,

    /// Output in structured JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct SetArgs {
    /// Duration string (e.g. 30s, 15m, 1h, 90m, 1h30m). Mutually exclusive with the other trigger flags.
    #[arg(long)]
    pub after: Option<String>,

    /// External match key for signal triggering. Mutually exclusive with the other trigger flags.
    #[arg(long)]
    pub signal: Option<String>,

    /// Herdr watched pane ID. Mutually exclusive with the other trigger flags.
    #[arg(long)]
    pub herdr_pane: Option<String>,

    /// Monitor a process id and fire as soon as it exits. Mutually exclusive with the other trigger flags.
    #[arg(long)]
    pub pid: Option<u32>,

    /// Watch a file/directory path and fire when the artifact contract is met.
    /// Mutually exclusive with the other trigger flags.
    #[arg(long)]
    pub watch_file: Option<std::path::PathBuf>,

    /// Artifact condition to wait for: `exists` (default) or `modified`.
    #[arg(long)]
    pub watch_mode: Option<String>,

    /// Comma-separated list of Herdr statuses to watch for (idle, done, blocked, working). Required with --herdr-pane.
    #[arg(long)]
    pub watch_for: Option<String>,

    /// Timeout duration for the condition/trigger ceiling. Required with --signal and
    /// --herdr-pane; optional with --pid/--watch-file (defaults to 24h).
    #[arg(long)]
    pub timeout: Option<String>,

    /// Herdr target pane ID (e.g. w1:p3). Shorthand for `--target herdr:<id>`.
    #[arg(long)]
    pub pane: Option<String>,

    /// Herdr target pane ID (e.g. w1:p3). Alias for --pane.
    #[arg(long)]
    pub herdr: Option<String>,

    /// tmux target (e.g. %0, work:0.1). Shorthand for `--target tmux:<id>`.
    #[arg(long)]
    pub tmux: Option<String>,

    /// Explicit delivery target: `herdr:<pane>`, `tmux:<target>`, `command:<cmd>`, or `wait`.
    #[arg(long)]
    pub target: Option<String>,

    /// Prompt delivery payload to inject into target agent
    #[arg(long)]
    pub prompt: Option<String>,

    /// Command delivery payload to execute in the target
    #[arg(long)]
    pub command: Option<String>,

    /// Optional descriptive label
    #[arg(long)]
    pub label: Option<String>,

    /// Validate duration and target without arming
    #[arg(long)]
    pub dry_run: bool,

    /// Output in structured JSON format
    #[arg(long)]
    pub json: bool,
}

/// `snuze wait <ALARM_ID>`: block until the alarm reaches a terminal state.
#[derive(Args, Debug, Clone)]
pub struct WaitArgs {
    /// Unique alarm identifier to block on
    pub alarm_id: String,

    /// Maximum time to wait (e.g. 30s, 10m, 1h). Waits indefinitely when omitted.
    #[arg(long)]
    pub timeout: Option<String>,

    /// Output in structured JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Filter alarms by status (pending, fired, failed, missed, canceled)
    #[arg(long)]
    pub status: Option<String>,

    /// Output in structured JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Unique alarm identifier
    pub alarm_id: String,

    /// Output in structured JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct CancelArgs {
    /// Specific alarm ID to cancel
    #[arg(required_unless_present = "all", conflicts_with = "all")]
    pub alarm_id: Option<String>,

    /// Cancel all currently pending alarms
    #[arg(long)]
    pub all: bool,

    /// Output in structured JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub action: DaemonAction,

    /// Output in structured JSON format
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Clone, Debug)]
pub enum DaemonAction {
    /// Start the background daemon
    Start {
        /// Run in foreground blocking mode instead of detached background
        #[arg(long)]
        foreground: bool,
    },
    /// Stop the background daemon
    Stop,
    /// Query background daemon status
    Status,
    /// Restart the background daemon
    Restart,
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// Output in structured JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct EventsArgs {
    #[command(subcommand)]
    pub action: EventsAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum EventsAction {
    /// Drain all terminal alarm events from the daemon's local outbox
    Drain {
        /// Output in structured JSON format
        #[arg(long)]
        json: bool,
    },
}
