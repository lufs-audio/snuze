# Unit 04 — jules-deprecation-and-wait-command

## Objective
Remove the deprecated `jules/` module and `Trigger::JulesCondition` from the `snuze` codebase, and introduce `snuze wait <alarm_id>` to allow synchronous scripts to block cleanly without polling loops.

## Context
- Google Jules condition mode was Phase 2 of `snuze` (`src/jules/`, `ureq` HTTP polling).
- With Jules deprecation and external signals (`snuze signal`) handling generic notifications, vendor-specific polling in the daemon is obsolete.
- Synchronous automation scripts need a clean command (`snuze wait <alarm_id>`) that blocks on the local IPC socket until the alarm reaches a terminal state.
- See phase spec: `docs/specs/2026-10-31T0045Z_generalized-triggers-and-control-planes/SPEC.md`.

## Acceptance criteria
- [ ] Remove `src/jules/` module, `ureq` HTTP dependencies in `Cargo.toml`, and `Trigger::JulesCondition`.
- [ ] Migrate any remaining Jules notification needs to `snuze signal <KEY>`.
- [ ] Add `snuze wait <alarm_id> [--timeout <dur>]` CLI command:
  - Connects to the local daemon socket and blocks until the specified alarm fires, fails, or is canceled.
  - Exits with `0` on `Fired`, or corresponding exit code on timeout/failure.
- [ ] Update root `SPEC.md` pointer to list this phase under `docs/specs/`.
- [ ] Ensure all remaining tests pass cleanly with zero warnings or dead-code clippy warnings.

## Interface contract
```rust
// CLI surface:
// snuze wait <ALARM_ID> [--timeout <DURATION>]

// Daemon IPC addition:
pub enum Request {
    // ...
    WaitAlarm { alarm_id: String, timeout_secs: Option<u64> },
}

pub enum Response {
    // ...
    AlarmFired { alarm_id: String },
}
```

## Boundaries — do NOT touch
- **Touches:**
  - `Cargo.toml` (remove `ureq`)
  - `src/lib.rs` (remove `pub mod jules`)
  - `src/jules/` (deleted)
  - `src/kernel/`
  - `src/cli/`
  - `src/daemon/`
  - `SPEC.md`
- **Does NOT touch:**
  - `src/herdr/`
  - `src/tmux/`

## Output
- Deleted: `src/jules/`; removed `pub mod jules` from `src/lib.rs`; dropped `ureq` from `Cargo.toml` (snuze returns to zero-network).
- Deleted tests: `tests/test_jules_adapter.rs`, `tests/test_jules_mock.rs`, `tests/test_scheduler_jules.rs`, `tests/test_kernel_jules.rs`, `tests/test_cli_jules_integration.rs`.
- Modified: `src/kernel/` (remove `Trigger::JulesCondition`), `src/cli/` (`snuze wait <id>`), `src/daemon/`; `SPEC.md` updated to record the removal.
- Commit message: `feat!: remove Jules condition mode; add snuze wait`

## Verification
```bash
cargo clippy --all-targets -- -D warnings
cargo test
```
