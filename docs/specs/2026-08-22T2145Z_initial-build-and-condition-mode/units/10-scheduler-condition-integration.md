# Unit 10 — Scheduler Condition Integration

## Objective
Extend `Scheduler::arm()`/`run()` to handle `Trigger::JulesCondition` alongside the existing `Trigger::Deadline` path, unifying both into the same min-heap loop per `SPEC.md` §11.1 — no second thread, no second wait mechanism.

## Context
See `SPEC.md` §11.1 (mechanism), §11.3 (interpolation happens here, not in the Herdr adapter). Depends on Unit 07 (`Trigger`, `JulesCondition`, `interpolate`) and Unit 08 (`JulesAdapter` trait + `MockJulesAdapter` for this unit's own tests). Existing reference, modified in place: `src/scheduler/mod.rs`'s `Scheduler::arm()` and `Scheduler::run()` — read the current implementation before changing it; every existing acceptance criterion from Unit 03 (min-heap ordering, non-polling condvar wait, missed-alarm grace window for `Deadline` triggers) must still hold and must still be covered by Unit 03's original tests, which must continue to pass unmodified.

**This unit also touches exactly one thing outside `src/scheduler/`:** `Scheduler::new()`'s constructor signature changes (see Interface Contract), so the single call site in `src/daemon/server.rs::run_foreground` must be updated to match — rename the local closure binding if needed and add the one new match arm described below. Nothing else in `src/daemon/` changes.

## Acceptance criteria
- [ ] `Scheduler` gains a `jules_adapter: Box<dyn JulesAdapter>` field, injected via `Scheduler::new()`'s constructor (new parameter — see Interface Contract for exact position).
- [ ] `FiredEvent` gains a `Progress { id: String, last_known: Vec<JulesSessionState>, next_check_at: SystemTime }` variant. The constructor parameter previously named `on_terminal` is renamed to `on_event` (same closure type, `Box<dyn Fn(FiredEvent) + Send + Sync>`) — it is no longer accurate to call it "terminal only" once `Progress` exists, and an inaccurate name here is worse than a one-line rename cost. Update the doc comment accordingly.
- [ ] `Scheduler::arm()`: for `Trigger::Deadline`, behavior is **byte-for-byte unchanged** from Unit 03 — same grace-window-vs-`deadline` check, same `Missed` outcome, same heap push. For `Trigger::JulesCondition{timeout_at, ..}`: check `now >= timeout_at` (not the generic `grace_window` — a condition's own timeout is its ceiling, `grace_window` does not apply to it) and emit `FiredEvent::Failed{id, reason: "timeout"}` immediately without queuing if already past it; otherwise push onto the heap normally (ordered by the alarm's `deadline` field, which for this trigger kind holds the next-check-time, per `SPEC.md` §11.1).
- [ ] `Scheduler::run()`: on popping an alarm whose deadline/next-check-time has arrived, branch on `Trigger`:
  - `Trigger::Deadline`: **unchanged** — build `TemplateVars { duration: Some(format_duration(now - alarm.created_at)), session_id: None, status: None, pr_urls: None }`, clone the alarm with `delivery`'s text replaced by `kernel::interpolate(original_text, &vars)`, call `adapter.deliver()` on the clone, emit `Fired`/`Failed` as today. A delivery prompt with no `{...}` tokens must interpolate to itself unchanged — this is a regression test, not just a new-feature test, since every alarm's delivery now passes through `interpolate()` for the first time.
  - `Trigger::JulesCondition{session_ids, condition, poll_interval_secs, timeout_at, ..}`: call `jules_adapter.check_sessions(session_ids)`.
    - On `Err(_)`: treat as a failed poll attempt, **not** a terminal failure — log/ignore per this unit's own error-visibility choice (propose: still re-arm at the normal `poll_interval`, so one transient network blip doesn't kill an otherwise-fine condition wait; if the adapter is persistently down, `timeout_at` is still the ultimate backstop). Document this choice explicitly in the implementation; do not silently swallow it without at least emitting a `Progress` event carrying the previous `last_known` unchanged, so `snuze status` can still be queried without erroring.
    - On `Ok(states)`: if `condition.is_satisfied(&states)`, build `TemplateVars` (`duration` as above; `session_id`/`status`/`pr_urls` populated per `SPEC.md` §11.3's exact formatting rules — single vs. multi-session), interpolate, `deliver()`, emit `Fired`/`Failed` exactly as the `Deadline` path does. If not satisfied and `now >= timeout_at`: emit `FiredEvent::Failed{id, reason: "timeout"}`, do not re-arm. If not satisfied and `now < timeout_at`: emit `FiredEvent::Progress{id, last_known: states, next_check_at: min(now + poll_interval, timeout_at)}`, update the in-memory alarm's `deadline` to that `next_check_at`, and re-push it onto the heap — do **not** wait for the daemon's persistence round-trip before re-arming in-memory; `Progress` is fire-and-forget from the scheduler's perspective, matching the existing pattern where `on_event`/`on_terminal` calls were already fire-and-forget from `run()`'s point of view.
- [ ] Unit tests (all via `FakeClock` + `MockJulesAdapter`, zero real sleeps, zero real HTTP): a `JulesCondition` alarm whose mock adapter returns `InProgress` on the first two polls and `Completed` on the third fires exactly once, after exactly three `check_sessions` calls, with the delivered text correctly interpolated; an `AnyTerminal` alarm with two session IDs fires as soon as either one becomes wake-worthy, verified via `MockJulesAdapter::call_args`; an `AllTerminal` alarm does not fire while any tracked session remains non-wake-worthy, confirmed by advancing the fake clock well past what would be enough polls for a single-session case; a condition that never resolves fires `Failed("timeout")` at exactly `timeout_at`, never `Missed`; a transient adapter error mid-poll does not kill the wait (confirmed by making the mock fail once then succeed, and asserting the alarm still eventually fires); the existing Unit 03 test suite (`tests/test_scheduler.rs`) passes unmodified against the extended `Scheduler`.

## Interface contract
```rust
use snuze_kernel::{Alarm, Trigger, JulesCondition, JulesSessionState, TemplateVars, interpolate};
use snuze_jules::JulesAdapter; // Unit 08
use std::time::{Duration, SystemTime};

pub enum FiredEvent {
    Fired { id: String, at: SystemTime },
    Failed { id: String, reason: String },
    Missed { id: String },
    /// New: a non-terminal update to a still-pending condition alarm's known state.
    /// The daemon's on_event handler persists this so `snuze status` reflects live
    /// poll progress rather than a stale "pending" with no detail.
    Progress {
        id: String,
        last_known: Vec<JulesSessionState>,
        next_check_at: SystemTime,
    },
}

impl Scheduler {
    pub fn new(
        adapter: Box<dyn HerdrAdapter>,           // existing, unchanged
        jules_adapter: Box<dyn JulesAdapter>,      // NEW
        clock: Box<dyn Clock>,                     // existing, unchanged
        grace_window: Duration,                    // existing, unchanged; applies to Deadline only
        on_event: Box<dyn Fn(FiredEvent) + Send + Sync>, // renamed from on_terminal
    ) -> Self;
    // arm(), cancel(), run() keep their existing public signatures unchanged;
    // only their internal bodies gain the Trigger-aware branches described above.
}
```

## Boundaries — do NOT touch
Do not implement the real `JulesAdapter` HTTP client (Unit 09, beyond importing Unit 08's trait for this unit's own tests via `MockJulesAdapter`). Do not touch CLI argument parsing (Unit 11). Do not touch `src/store/`. In `src/daemon/server.rs`, touch **only** the single `Scheduler::new(...)` call site (add the `jules_adapter` argument, rename the closure binding, add the one `FiredEvent::Progress` match arm) — no other daemon logic changes.

## Output
- `src/scheduler/mod.rs` (modified: `FiredEvent::Progress`, `Scheduler` field + constructor param, `arm()`/`run()` trigger-aware branches)
- `src/scheduler/template.rs` or reuse of Unit 07's `src/kernel/template.rs` (no new template logic here, just calling it)
- `src/daemon/server.rs` (modified: the one `Scheduler::new()` call site + one new match arm, as scoped above)
- `tests/test_scheduler_jules.rs` (new)

## Verification
```bash
cargo test --test test_scheduler --test test_scheduler_jules --test test_daemon
cargo clippy --all-targets -- -D warnings
```
