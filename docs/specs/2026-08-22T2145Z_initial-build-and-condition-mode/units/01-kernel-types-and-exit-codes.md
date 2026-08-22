# Unit 01 — Kernel Types and Exit Code Taxonomy

## Objective
Establish the foundational domain models, duration parsing, alarm status lifecycle, and exit code contract in Rust for `snuze`. Every other unit depends on this one; it depends on nothing.

## Context
See `SPEC.md` §5 (Exit Code Taxonomy) and §6 (Alarm Status Lifecycle).
Reference shape (not code to copy, just the pattern to match): `apho/src/kernel/types.rs` and `apho/src/kernel/errors.rs` — same discipline of typed domain structs + a dedicated exit-code enum, applied here to a scheduling domain instead of a capture domain.

## Acceptance criteria
- [ ] Define `Duration` parsing: a function accepting strings like `30s`, `15m`, `1h`, `90m`, and simple compounds (`1h30m`), returning `std::time::Duration`. Reject empty strings, negative values, and unknown unit suffixes with a typed error (not a panic).
- [ ] Define `AlarmStatus` enum: `Pending`, `Fired`, `Failed`, `Missed`, `Canceled` (serialized as lowercase strings via `serde`).
- [ ] Define `DeliveryMode` enum: `Prompt(String)`, `Command(String)` — mutually exclusive payloads, matching SPEC §3.3 and §7.
- [ ] Define `Alarm` struct: `id: String` (ULID or UUIDv7 — must be lexicographically sortable by creation time; do not use a random UUIDv4), `pane_id: String`, `created_by_pane: Option<String>` (the calling pane's `$HERDR_PANE_ID` at schedule time, captured by Unit 06's CLI layer, not this unit — this field just needs to exist and round-trip; see `SPEC.md` §1.1 Seven-LUFS-Tests "Provenance-bearing" for why: without it an alarm records where it's going but never who asked, which matters the moment more than one agent is scheduling alarms against shared panes), `delivery: DeliveryMode`, `label: Option<String>`, `created_at: SystemTime`, `deadline: SystemTime`, `status: AlarmStatus`, `fired_at: Option<SystemTime>`, `failure_reason: Option<String>`.
- [ ] Define `SnuzeError` covering: `InvalidDuration(String)`, `ConflictingFlags(String)`, `PaneNotFound(String)`, `AlarmNotFound(String)`, `PersistFailed(String)`, `HerdrBinaryMissing`, `DaemonUnavailable(String)`. Each variant must map to exactly one `ExitCode` (see below) via a `From<SnuzeError> for ExitCode` or equivalent method — this mapping is the load-bearing part of this unit, not an afterthought.
- [ ] Define `ExitCode` enum: `Success = 0`, `Usage = 2`, `TargetUnavailable = 3`, `BinaryMissing = 4`, `ContractViolated = 5`, `DaemonUnavailable = 6`.
- [ ] Unit tests cover: every valid duration string in the examples above parses to the expected `Duration`; every listed invalid duration string returns `SnuzeError::InvalidDuration` (never panics); every `SnuzeError` variant maps to the exit code specified in `SPEC.md` §5; `Alarm` round-trips through `serde_json` (serialize then deserialize is lossless) including all `AlarmStatus` and `DeliveryMode` variants.

## Interface contract
```rust
use std::time::{Duration, SystemTime};

pub fn parse_duration(s: &str) -> Result<Duration, SnuzeError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlarmStatus { Pending, Fired, Failed, Missed, Canceled }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum DeliveryMode {
    Prompt { text: String },
    Command { text: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Alarm {
    pub id: String,
    pub pane_id: String,
    /// Provenance: the calling pane's $HERDR_PANE_ID at schedule time, if any.
    /// Populated by the CLI layer (Unit 06), not here — this struct just owns
    /// the field and its serde round-trip.
    pub created_by_pane: Option<String>,
    pub delivery: DeliveryMode,
    pub label: Option<String>,
    pub created_at: SystemTime,
    pub deadline: SystemTime,
    pub status: AlarmStatus,
    pub fired_at: Option<SystemTime>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SnuzeError {
    #[error("invalid duration: {0}")]
    InvalidDuration(String),
    #[error("conflicting flags: {0}")]
    ConflictingFlags(String),
    #[error("herdr pane not found: {0}")]
    PaneNotFound(String),
    #[error("alarm not found: {0}")]
    AlarmNotFound(String),
    #[error("could not persist alarm: {0}")]
    PersistFailed(String),
    #[error("herdr binary not found on PATH")]
    HerdrBinaryMissing,
    #[error("snuze daemon unavailable: {0}")]
    DaemonUnavailable(String),
}

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

impl From<&SnuzeError> for ExitCode {
    fn from(e: &SnuzeError) -> Self {
        match e {
            SnuzeError::InvalidDuration(_) | SnuzeError::ConflictingFlags(_) => ExitCode::Usage,
            SnuzeError::PaneNotFound(_) | SnuzeError::AlarmNotFound(_) => ExitCode::TargetUnavailable,
            SnuzeError::HerdrBinaryMissing => ExitCode::BinaryMissing,
            SnuzeError::PersistFailed(_) => ExitCode::ContractViolated,
            SnuzeError::DaemonUnavailable(_) => ExitCode::DaemonUnavailable,
        }
    }
}
```

## Boundaries — do NOT touch
Do not implement file I/O, socket I/O, or any call to the `herdr` binary in this unit. No `src/store/`, `src/scheduler/`, `src/herdr/`, `src/daemon/`, or `src/cli/` files.

## Output
- `src/kernel/mod.rs`
- `src/kernel/types.rs`
- `src/kernel/duration.rs`
- `src/kernel/errors.rs`
- `tests/test_kernel.rs`

## Verification
```bash
cargo test --test test_kernel
cargo clippy --all-targets -- -D warnings
```
