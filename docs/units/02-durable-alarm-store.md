# Unit 02 — Durable Alarm Store

## Objective
Implement crash-safe, folder-per-entry persistence for `Alarm` records, following the house `filesystem-as-registry` convention rather than introducing a database dependency.

## Context
See `SPEC.md` §3.2 (Durable Alarm Store). Depends on Unit 01's `Alarm`, `AlarmStatus`, `SnuzeError` types — import them, do not redefine them.
Reference pattern: the `filesystem-as-registry` skill (folder-per-entry + typed manifest); here reduced to its simplest case, one flat JSON file per entry, because an alarm has no sub-files of its own (no audio, no images — just the record).

## Acceptance criteria
- [ ] `AlarmStore::open(state_dir: &Path) -> Result<AlarmStore, SnuzeError>` creates `state_dir/alarms/` if absent and returns a handle. Must not fail if the directory already exists and is populated.
- [ ] `AlarmStore::put(&self, alarm: &Alarm) -> Result<(), SnuzeError>` writes `alarms/<id>.json` via write-to-temp-file-then-rename (never a direct in-place write) so a process crash mid-write cannot leave a corrupt or half-written alarm file. Maps any I/O failure to `SnuzeError::PersistFailed`.
- [ ] `AlarmStore::get(&self, id: &str) -> Result<Alarm, SnuzeError>` reads and deserializes one alarm; returns `SnuzeError::AlarmNotFound` if the file is absent, and a `PersistFailed`-class error (not a panic) if the file exists but fails to deserialize (corrupt file).
- [ ] `AlarmStore::list(&self, status_filter: Option<AlarmStatus>) -> Result<Vec<Alarm>, SnuzeError>` scans the directory and returns all alarms (or only those matching `status_filter`), sorted by `deadline` ascending. A single corrupt/unreadable alarm file must not fail the whole listing — skip it, and surface it via a separate `AlarmStore::scan_errors(&self) -> Vec<(String, String)>` (filename, reason) so `snuze doctor` can report it, rather than either silently dropping it or crashing the whole daemon.
- [ ] `AlarmStore::delete(&self, id: &str) -> Result<(), SnuzeError>` removes an alarm file. Used only for retention cleanup (§SPEC 6), never for `cancel` (canceling sets `AlarmStatus::Canceled` and keeps the record — cancellation must remain visible in `snuze list --status canceled`, not disappear).
- [ ] All paths resolve relative to `$XDG_STATE_HOME/snuze` with fallback to `~/.local/state/snuze` when `XDG_STATE_HOME` is unset — implement this resolution here, once, as the single source of truth other units call into (Unit 05's daemon and Unit 06's CLI both need the same state dir; do not let them each compute it independently).
- [ ] Unit tests cover: put-then-get round-trip; list returns correct sort order; list survives one deliberately corrupted file among several valid ones (write garbage bytes to one `.json` file in a test fixture, assert `list()` still returns the others and `scan_errors()` reports the bad one); a simulated crash-mid-write (write to the temp path, do not rename, assert `list()` never sees a half-written alarm — the untransacted temp file is simply invisible to `list`, not a false-`Pending` alarm).

## Interface contract
```rust
use std::path::{Path, PathBuf};
use snuze_kernel::{Alarm, AlarmStatus, SnuzeError};

pub struct AlarmStore {
    alarms_dir: PathBuf,
}

impl AlarmStore {
    pub fn open(state_dir: &Path) -> Result<Self, SnuzeError>;
    pub fn put(&self, alarm: &Alarm) -> Result<(), SnuzeError>;
    pub fn get(&self, id: &str) -> Result<Alarm, SnuzeError>;
    pub fn list(&self, status_filter: Option<AlarmStatus>) -> Result<Vec<Alarm>, SnuzeError>;
    pub fn delete(&self, id: &str) -> Result<(), SnuzeError>;
    pub fn scan_errors(&self) -> Vec<(String, String)>;
}

/// Single source of truth for the state directory; called by the daemon (Unit 05)
/// and the CLI (Unit 06). Do not reimplement this resolution elsewhere.
pub fn resolve_state_dir() -> PathBuf;
```

## Boundaries — do NOT touch
Do not implement scheduling, heap logic, or Herdr calls in this unit. Do not touch `src/scheduler/`, `src/herdr/`, `src/daemon/`, `src/cli/`. This unit has no knowledge of *when* an alarm should fire beyond storing the `deadline` field it's given — timing logic lives entirely in Unit 03.

## Output
- `src/store/mod.rs`
- `src/store/paths.rs`
- `tests/test_store.rs`
- `tests/fixtures/corrupt_alarm.json` (a deliberately invalid fixture used by the corruption test)

## Verification
```bash
cargo test --test test_store
cargo clippy --all-targets -- -D warnings
```
