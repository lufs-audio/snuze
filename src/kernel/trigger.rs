use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HerdrWatchStatus {
    Idle,
    Done,
    Blocked,
    Working,
}

#[must_use]
pub fn herdr_status_satisfied(watch_for: &[HerdrWatchStatus], current: HerdrWatchStatus) -> bool {
    if watch_for.is_empty() {
        return false;
    }
    watch_for.contains(&current)
}

/// Condition a watched file artifact must satisfy before the alarm fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileWatchMode {
    /// Fire as soon as the path exists.
    Exists,
    /// Fire once the path exists and has been written at or after the alarm was created.
    Modified,
}

impl FileWatchMode {
    /// Stable lowercase name (`exists`, `modified`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exists => "exists",
            Self::Modified => "modified",
        }
    }
}

/// Trigger mechanism defining when an alarm fires or re-polls.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", from = "TriggerWire")]
pub enum Trigger {
    #[default]
    Deadline,
    /// Fires as soon as process `pid` is no longer running.
    ///
    /// The scheduler re-polls with a capped exponential backoff derived from the
    /// alarm's age, and gives up with a `Failed("timeout")` at `timeout_at`.
    ProcessExit {
        /// Monitored process id.
        pid: u32,
        /// Hard ceiling after which the alarm fails instead of firing.
        timeout_at: SystemTime,
    },
    /// Fires once a file/directory artifact satisfies a [`FileWatchMode`].
    ///
    /// `path` may be exact, or use `*`/`?` wildcards in its final component to
    /// match any entry in the parent directory. Re-polled with the same capped
    /// exponential backoff as [`Trigger::ProcessExit`].
    FileArtifact {
        /// Watched path (exact, or globbed in the final component).
        path: PathBuf,
        /// Condition the artifact must satisfy.
        mode: FileWatchMode,
        /// Hard ceiling after which the alarm fails instead of firing.
        timeout_at: SystemTime,
    },
    HerdrStatus {
        watched_pane_id: String,
        watch_for: Vec<HerdrWatchStatus>,
        timeout_at: SystemTime,
        last_known: Option<HerdrWatchStatus>,
    },
    ExternalSignal {
        match_key: String,
        timeout_at: SystemTime,
    },
}

/// Wire form used only to read a [`Trigger`] back off disk.
///
/// Unlike `Trigger` itself, this tolerates trigger kinds that no longer exist in
/// the current build — most importantly the removed `jules_condition`. Without
/// it, every alarm file written by an older `snuze` on a live machine would fail
/// to parse, and the whole record would be discarded as corrupt.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TriggerWire {
    Deadline,
    ProcessExit {
        pid: u32,
        timeout_at: SystemTime,
    },
    FileArtifact {
        path: PathBuf,
        mode: FileWatchMode,
        timeout_at: SystemTime,
    },
    HerdrStatus {
        watched_pane_id: String,
        watch_for: Vec<HerdrWatchStatus>,
        timeout_at: SystemTime,
        last_known: Option<HerdrWatchStatus>,
    },
    ExternalSignal {
        match_key: String,
        timeout_at: SystemTime,
    },
    /// Any tag this build does not know about.
    #[serde(other)]
    Unrecognized,
}

impl From<TriggerWire> for Trigger {
    fn from(wire: TriggerWire) -> Self {
        match wire {
            TriggerWire::Deadline => Self::Deadline,
            TriggerWire::ProcessExit { pid, timeout_at } => Self::ProcessExit { pid, timeout_at },
            TriggerWire::FileArtifact {
                path,
                mode,
                timeout_at,
            } => Self::FileArtifact {
                path,
                mode,
                timeout_at,
            },
            TriggerWire::HerdrStatus {
                watched_pane_id,
                watch_for,
                timeout_at,
                last_known,
            } => Self::HerdrStatus {
                watched_pane_id,
                watch_for,
                timeout_at,
                last_known,
            },
            TriggerWire::ExternalSignal {
                match_key,
                timeout_at,
            } => Self::ExternalSignal {
                match_key,
                timeout_at,
            },
            // A trigger kind this build no longer implements (for example the
            // removed `jules_condition`): degrade to a plain deadline alarm driven
            // by the alarm's own `deadline` field, rather than dropping the record.
            TriggerWire::Unrecognized => Self::Deadline,
        }
    }
}
