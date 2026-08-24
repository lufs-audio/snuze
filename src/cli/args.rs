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
    /// Duration string (e.g. 30s, 15m, 1h, 90m, 1h30m). Mutually exclusive with --jules, --signal, --herdr-pane.
    #[arg(long)]
    pub after: Option<String>,

    /// Comma-separated Jules session IDs. Mutually exclusive with --after, --signal, --herdr-pane.
    #[arg(long)]
    pub jules: Option<String>,

    /// External match key for signal triggering. Mutually exclusive with --after, --jules, --herdr-pane.
    #[arg(long)]
    pub signal: Option<String>,

    /// Herdr watched pane ID. Mutually exclusive with --after, --jules, --signal.
    #[arg(long)]
    pub herdr_pane: Option<String>,

    /// Comma-separated list of Herdr statuses to watch for (idle, done, blocked, working). Required with --herdr-pane.
    #[arg(long)]
    pub watch_for: Option<String>,

    /// Condition to trigger Jules alarm (all-terminal, any-terminal, any-completed). Required with --jules.
    #[arg(long)]
    pub condition: Option<String>,

    /// Timeout duration for Jules session condition check. Required with --jules.
    #[arg(long)]
    pub timeout: Option<String>,

    /// Polling interval for Jules condition checks (defaults to 30s).
    #[arg(long)]
    pub poll_interval: Option<String>,

    /// Herdr target pane ID (e.g. w1:p3)
    #[arg(long)]
    pub pane: String,

    /// Prompt delivery payload to inject into target agent
    #[arg(long)]
    pub prompt: Option<String>,

    /// Command delivery payload to execute in target pane (not supported with --jules)
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
