# Unit 02 — Herdr Event Subscription Listener

## Objective
Implement a persistent, push-based listener on Herdr's own event stream, and the `Scheduler::signal()` mechanism plus its `HerdrPaneEvent` resolution arm. This is the daemon's first long-lived Herdr connection (every prior use of Herdr was a one-shot request/response call) and the first time anything resolves a pending alarm outside the scheduler's own heap-pop timing loop.

## Context
See `SPEC.md` §3.2 (why a third thread doesn't break the existing concurrency rule) and §3.3 (`Scheduler::signal()`'s shared contract — read this in full; Unit 03 implements the other half of the same method). Depends on this phase's Unit 01 (`HerdrWatchStatus`, `Trigger::HerdrStatus`, `herdr_status_satisfied`) and the **archived** previous phase's Unit 03 (`Scheduler`, `HerdrAdapter` trait, the existing heap/clock machinery — read `src/scheduler/mod.rs` as it exists today before extending it) and Unit 04 (`RealHerdrAdapter`, confirmed Herdr CLI/socket conventions).

**Verify the transport before implementing.** Every previous Herdr integration in this codebase used the `herdr` CLI wrapper for one-shot calls (`agent prompt`, `pane run`, `pane get`) per the CLI-wrapper-first preference established in the archived Unit 03/04. A persistent subscription is not a one-shot call, and it is not yet confirmed whether Herdr's own `herdr` skill recommends the CLI wrapper or the raw socket API for `events.subscribe`/`events.wait` specifically — check the real `herdr` skill's own guidance for this exact case before choosing a transport, the same "verify against the installed binary/skill, don't re-guess" discipline that already corrected this repo's Herdr syntax twice (the original `--wait` hedge, and its later removal after live behavior contradicted the hedge). If the skill is silent on this, prefer the raw socket API for a subscription specifically (a CLI process wrapping a long-lived subscribe-and-block call is an awkward fit for a wrapper designed around request/response), and document the choice with its reasoning in the implementation, not just in a commit message.

## Acceptance criteria
- [ ] `Scheduler` gains a `signal(&self, signal: PushSignal) -> Vec<String>` method (see Interface Contract) and a `PushSignal` enum with `HerdrPaneEvent { pane_id: String, new_status: HerdrWatchStatus }` (this unit's arm) and `ExternalSignal { match_key: String }` (Unit 03's arm — the variant must exist here since both units touch the same enum, but this unit implements only the `HerdrPaneEvent` match arm's resolution logic; leave `ExternalSignal`'s arm as a documented stub or coordinate the exact split with Unit 03 if both land close together — the enum itself is Layer 1-adjacent shared surface, not exclusively either unit's).
- [ ] `signal()`'s `HerdrPaneEvent` handling: find every **pending** alarm whose `trigger` is `Trigger::HerdrStatus` with `watched_pane_id == pane_id`, and for each, call `herdr_status_satisfied(&watch_for, new_status)` (Unit 01). If `true`: remove it from the heap (reuse whichever internal mechanism `Scheduler::cancel()` already uses for arbitrary, non-minimum heap-element removal — do not implement a second one; read `cancel()`'s current implementation before writing this), interpolate and deliver exactly like a normal heap-pop firing (same `HerdrAdapter::deliver()` call, same `Fired`/`Failed` terminal-event emission via the existing `on_event` callback), and include its ID in the returned `Vec<String>`. If `false`: update that alarm's `last_known` to `new_status` in place (heap position unchanged, still pending) for `snuze status` visibility, and do not include it in the returned IDs. An event matching zero pending alarms returns an empty `Vec` — not an error, not a panic.
- [ ] `Scheduler::arm()` for `Trigger::HerdrStatus`: identical shape to `Trigger::JulesCondition`'s existing timeout handling (archived Unit 10) — if `now >= timeout_at` already, emit `Failed{reason: "timeout"}` immediately without queuing; otherwise push onto the heap keyed by `timeout_at`.
- [ ] `Scheduler::run()`'s heap-pop path, when it pops a `Trigger::HerdrStatus` alarm because its `timeout_at` was reached (never signaled in time): emit `Failed{id, reason: "timeout"}`, identical in shape to `JulesCondition`'s existing timeout path. A `HerdrStatus` alarm is never fired by the normal pop path on anything other than timeout — its only non-timeout resolution is through `signal()`.
- [ ] `HerdrEventListener`: owns one persistent connection to Herdr's event stream (transport per the verification note above), calling the confirmed subscribe/wait primitive in a blocking loop on its own dedicated thread, started once at daemon startup alongside the existing scheduler thread (`src/daemon/server.rs`, one new thread spawn — no other daemon startup sequencing changes). On each received `pane.agent_status_changed`-shaped event, maps Herdr's own status vocabulary to `HerdrWatchStatus` (an unrecognized Herdr status string is logged and ignored, never a panic — same alpha/evolving-API defensiveness `JulesState::Unknown` already established for Jules) and calls `scheduler.signal(PushSignal::HerdrPaneEvent{pane_id, new_status})`, discarding the returned `Vec` (the listener does not need to know which alarms resolved — that's `Scheduler`'s concern, kept in one place).
- [ ] Reconnection: on a dropped or failed subscription connection, the listener retries with capped exponential backoff (propose: start at 1s, cap at 30s) and logs each attempt. It must not busy-loop, and a disconnected listener must not cause any incorrect firing — only a delayed one, bounded by each pending alarm's own `timeout_at` (this is the property to write a test for, not just assert in a comment).
- [ ] Unit tests (via `FakeClock` + a mock Herdr event source — a simple in-test channel or fake socket the listener reads from, not a real Herdr instance): a `HerdrStatus` alarm watching `[Idle, Done]` fires on a `Done` event for its watched pane, and does not fire on a `Blocked` event for the same pane (confirms `last_known` updates instead); two alarms watching the same pane for overlapping statuses both fire on one matching event (broadcast semantics, per `SPEC.md` §3.3); an alarm watching pane `A` does not fire on an event for pane `B`; a `HerdrStatus` alarm with no matching event ever fires `Failed("timeout")` at exactly `timeout_at` via the normal heap-pop path; simulating a listener disconnect-then-reconnect confirms a matching event delivered *after* reconnection still resolves a pending alarm, and confirms the alarm did not fire early or incorrectly during the disconnected window.

## Interface contract
```rust
use snuze_kernel::{HerdrWatchStatus, herdr_status_satisfied};

pub enum PushSignal {
    HerdrPaneEvent { pane_id: String, new_status: HerdrWatchStatus },
    ExternalSignal { match_key: String }, // Unit 03 implements this arm's resolution
}

impl Scheduler {
    /// See SPEC.md §3.3 for the full contract. Broadcast semantics: returns
    /// the IDs of every alarm this call resolved; an empty Vec is a valid,
    /// non-error outcome.
    pub fn signal(&self, signal: PushSignal) -> Vec<String>;
}

/// Runs on its own dedicated thread, started once at daemon startup.
/// Blocks on Herdr's own subscribe/wait primitive; never polls.
pub struct HerdrEventListener { /* opaque */ }

impl HerdrEventListener {
    pub fn new(scheduler: std::sync::Arc<Scheduler>) -> Result<Self, SnuzeError>;

    /// Blocks the calling thread for the listener's lifetime, reconnecting
    /// with capped exponential backoff on any connection failure.
    pub fn run(&self);
}
```

## Boundaries — do NOT touch
Do not implement the `SignalAlarm` IPC request, the `snuze signal`/`--signal` CLI surface, or the `ExternalSignal` arm's resolution logic (Unit 03 — the `PushSignal::ExternalSignal` variant exists in this unit's output only as shared enum surface, not implemented behavior). Do not implement the event outbox (Unit 04). Do not touch `src/cli/` or `src/daemon/protocol.rs`. In `src/daemon/server.rs`, touch only the one new thread-spawn call for `HerdrEventListener::run()` at startup — no other daemon lifecycle changes.

## Output
- `src/herdr/subscription.rs` (new: `HerdrEventListener`)
- `src/scheduler/mod.rs` (modified: `PushSignal` enum, `Scheduler::signal()`, `HerdrPaneEvent` resolution arm, `HerdrStatus` handling in `arm()`/`run()`)
- `src/daemon/server.rs` (modified: one new thread spawn at startup)
- `tests/test_herdr_subscription.rs` (new)
- `tests/test_scheduler_signal.rs` (new — covers `signal()`'s `HerdrPaneEvent` arm; Unit 03 extends this same file for `ExternalSignal`)

## Verification
```bash
cargo test --test test_scheduler --test test_scheduler_jules --test test_scheduler_signal --test test_herdr_subscription --test test_daemon
cargo clippy --all-targets -- -D warnings
```
