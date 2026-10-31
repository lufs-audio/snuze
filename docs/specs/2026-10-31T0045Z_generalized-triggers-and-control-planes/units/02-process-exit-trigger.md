# Unit 02 — process-exit-trigger

## Objective
Implement `Trigger::ProcessExit` in the `snuze` kernel and daemon scheduler, allowing alarms to fire immediately when a specified PID terminates.

## Context
- Long-running local CLI tasks (like `lrex` audio recording or builds) run as background or foreground child processes.
- Agents currently have to estimate a fixed duration (`--after 30m`) or run busy polling loops to detect process completion.
- See phase spec: `docs/specs/2026-10-31T0045Z_generalized-triggers-and-control-planes/SPEC.md`.

## Acceptance criteria
- [ ] `Trigger` enum includes variant `ProcessExit { pid: u32, timeout_at: SystemTime }`.
- [ ] Daemon tracks monitored PIDs using non-blocking checks (`libc::kill(pid, 0)` or `/proc/<pid>`).
- [ ] When the target process is no longer running, the alarm is popped and delivered immediately.
- [ ] If the process does not exit before `timeout_at` (default 24h, configurable via `--timeout`), the alarm fires with status `Failed` or `Missed`.
- [ ] CLI command `snuze set --pid <PID> --prompt <PROMPT>` schedules the process-exit alarm.
- [ ] If the specified PID does not exist at schedule time, `snuze set` fails immediately with an actionable error.
- [ ] Unit and integration tests verify alarm firing when a spawned test process terminates.

## Interface contract
```rust
// In src/kernel/trigger.rs

pub enum Trigger {
    Deadline,
    ProcessExit {
        pid: u32,
        timeout_at: SystemTime,
    },
    // ...
}

// In src/cli/args.rs
// snuze set [--pid <PID>] [--timeout <DURATION>] ...
```

## Boundaries — do NOT touch
- **Touches:**
  - `src/kernel/trigger.rs`
  - `src/scheduler/mod.rs`
  - `src/daemon/mod.rs`
  - `src/cli/`
  - `tests/`
- **Does NOT touch:**
  - `src/herdr/`
  - `src/store/`

## Output
- Modified: `src/kernel/trigger.rs` (`Trigger::ProcessExit`), `src/scheduler/mod.rs` (PID monitoring + backoff), `src/daemon/mod.rs`, `src/cli/` (`--pid`).
- Tests added under `tests/` for PID-exit firing, timeout expiry, and the no-busy-loop backoff.
- Commit message: `feat(trigger): fire on process exit via --pid`

## Verification
```bash
cargo test --test test_process_trigger
```
