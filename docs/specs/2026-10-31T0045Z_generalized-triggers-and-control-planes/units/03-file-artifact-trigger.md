# Unit 03 — file-artifact-trigger

## Objective
Implement `Trigger::FileArtifact` in the `snuze` kernel and scheduler, allowing alarms to fire when a specified file path is created or modified.

## Context
- Local pipeline tools like `lrex` signal completion by writing contract files (e.g. `take.json`).
- `tscribe` signals transcription completion by writing `transcript.md`.
- Snuze can monitor path existence or modification without requiring active agent polling.
- See phase spec: `docs/specs/2026-10-31T0045Z_generalized-triggers-and-control-planes/SPEC.md`.

## Acceptance criteria
- [ ] `Trigger` enum includes `FileArtifact { path: PathBuf, mode: FileWatchMode, timeout_at: SystemTime }` where `FileWatchMode` supports `Exists` and `Modified`.
- [ ] Scheduler checks paths periodically with backoff, firing immediately once the path satisfies the condition.
- [ ] Supports glob matching or exact path resolution.
- [ ] Times out cleanly if the artifact does not appear before `timeout_at`.
- [ ] CLI command `snuze set --watch-file <PATH> [--watch-mode exists|modified] --prompt <PROMPT>` schedules the alarm.
- [ ] Tests verify firing when a mock file is written by an independent thread/process.

## Interface contract
```rust
// In src/kernel/trigger.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileWatchMode {
    Exists,
    Modified,
}

pub enum Trigger {
    // ...
    FileArtifact {
        path: std::path::PathBuf,
        mode: FileWatchMode,
        timeout_at: std::time::SystemTime,
    },
}
```

## Boundaries — do NOT touch
- **Touches:**
  - `src/kernel/trigger.rs`
  - `src/scheduler/mod.rs`
  - `src/daemon/mod.rs`
  - `src/cli/`
  - `tests/`
- **Does NOT touch:**
  - `src/store/`
  - `src/herdr/`

## Output
- Modified: `src/kernel/trigger.rs` (`Trigger::FileArtifact`), `src/scheduler/mod.rs` (path watcher + backoff), `src/daemon/mod.rs`, `src/cli/` (`--watch-file`).
- Tests added under `tests/` for file-appearance firing, directory contracts, and timeout expiry.
- Commit message: `feat(trigger): fire on file artifact via --watch-file`

## Verification
```bash
cargo test --test test_file_artifact_trigger
```
