# Unit 03 — Scheduler and the `HerdrAdapter` Trait

## Objective
Implement the in-memory scheduling engine (min-heap + dedicated thread) that fires alarms at their deadlines, and publish the `HerdrAdapter` trait that Unit 04 will implement. This unit must be fully testable with a fake adapter and an injectable clock — no real Herdr socket, no real system clock dependency in tests.

## Context
See `SPEC.md` §3.1 (Scheduler) and §3.3 (Herdr Integration, delivery verification). Depends on Unit 01's `Alarm`, `AlarmStatus`, `SnuzeError`.
This unit does **not** depend on Unit 04. It defines the `HerdrAdapter` trait as an interface contract; Unit 04 implements it independently, in parallel, against the signature published here. Publishing a wrong or unstable signature here is the single biggest way to break that parallelization — treat the trait signature below as frozen once this unit lands.

## Acceptance criteria
- [ ] `HerdrAdapter` trait defined with a `deliver` method taking a resolved `Alarm` and returning a `DeliveryOutcome` (not a bare `Result<(), Error>`) — per `SPEC.md` §3.3, Herdr's real agent-lifecycle semantics distinguish more than "worked" vs. "didn't": `Delivered`, `AgentBlocked` (target already blocked on something else, input never sent), `PromptStalled` (text sent, but Herdr observed no lifecycle change within its own 5-second window — confirmed real per the `herdr` skill, not a hypothetical), `PaneGone` (target no longer resolves), `HerdrUnavailable` (herdr itself unreachable, timed out, or `snuze` invoked it incorrectly). From this unit's perspective all four non-`Delivered` variants are terminal `Failed` with a specific reason string — this unit does not need per-variant branching logic, it just needs the enum to carry the right reason through to persistence (Unit 05), so Unit 04 can produce an accurate one without losing information on the way.
- [ ] `Scheduler` holds a `BinaryHeap` (or equivalent) keyed by `deadline`, ordered earliest-first (use `Reverse` or an inverted `Ord` impl — Rust's `BinaryHeap` is a max-heap by default; get this backwards and every alarm fires in the wrong order, so this is an explicit acceptance criterion, not an implementation detail to leave to chance).
- [ ] `Scheduler::new(adapter: Box<dyn HerdrAdapter>, clock: Box<dyn Clock>) -> Scheduler` — the clock is injectable (a `SystemClock` for production, a `FakeClock` for tests that can be advanced programmatically without real sleeps).
- [ ] `Scheduler::arm(&self, alarm: Alarm)` inserts an alarm into the heap. Called both for freshly-created alarms and when the daemon rebuilds the heap from the store on startup (Unit 05 calls this in a loop over `AlarmStore::list(Some(Pending))`).
- [ ] `Scheduler::cancel(&self, id: &str) -> bool` removes a pending alarm from the heap before it fires; returns `false` if the alarm was not found pending (already fired, already canceled, or unknown). Cancellation must be effective even if the alarm is the very next one due to fire — no race where a cancel-in-flight still fires.
- [ ] `Scheduler::run(&self)` blocks the calling thread: sleeps until `min(next_deadline, wake_signal)` via a condvar, and on wake either processes a due alarm or re-evaluates because `arm`/`cancel` changed the heap. Must not busy-poll.
- [ ] On firing, the scheduler calls `adapter.deliver(&alarm)`, applies the missed-alarm grace-window rule (SPEC §3.4) for alarms whose deadline is already more than the configured grace window in the past *at arm time* (this covers the daemon-restart case; Unit 05 owns re-arming on startup but this unit owns the grace-window decision itself, since it's fundamentally a scheduling policy, not a daemon-lifecycle concern), and emits the resulting terminal status (`Fired`, `Failed`, or `Missed`) via a callback/channel that Unit 05 uses to persist the update through `AlarmStore::put`. This unit does not call `AlarmStore` directly — it has no knowledge of persistence, only of scheduling and delivery. Keep that boundary; it's what makes this unit testable without a filesystem.
- [ ] Unit tests (all using `FakeClock`, zero real `sleep` calls, zero real Herdr access): an armed alarm fires exactly once when the fake clock crosses its deadline; canceling a pending alarm before the fake clock reaches its deadline results in zero `deliver` calls ever, even after advancing the clock far past the deadline; two alarms with deadlines 1ms apart fire in deadline order; an alarm whose deadline (at arm time) is already past the grace window is immediately marked `Missed` without any `deliver` call; a `deliver` call returning any of `PaneGone`, `AgentBlocked`, `PromptStalled`, or `HerdrUnavailable` results in status `Failed`, not `Fired`, in every case, with the outcome's reason string preserved through to the `FiredEvent::Failed` sent to the `on_terminal` callback.

## Interface contract
```rust
use std::time::SystemTime;
use snuze_kernel::Alarm;

pub enum DeliveryOutcome {
    Delivered,
    AgentBlocked(String),     // reason; --prompt only, never produced by --command
    PromptStalled(String),    // reason; --prompt only, never produced by --command
    PaneGone(String),         // reason
    HerdrUnavailable(String), // reason (includes snuze-invoked-herdr-incorrectly cases)
}

pub trait HerdrAdapter: Send + Sync {
    fn deliver(&self, alarm: &Alarm) -> DeliveryOutcome;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
    /// Blocks the calling thread until `deadline` or until externally woken
    /// (e.g. by a new `arm`/`cancel` call). Production impl sleeps for real;
    /// test impl (`FakeClock`) is advanced explicitly by the test and wakes
    /// waiters synchronously — no real time may elapse in unit tests.
    fn wait_until(&self, deadline: SystemTime);
}

pub enum FiredEvent {
    Fired { id: String, at: SystemTime },
    Failed { id: String, reason: String },
    Missed { id: String },
}

pub struct Scheduler { /* opaque */ }

impl Scheduler {
    pub fn new(
        adapter: Box<dyn HerdrAdapter>,
        clock: Box<dyn Clock>,
        grace_window: std::time::Duration,
        on_terminal: Box<dyn Fn(FiredEvent) + Send + Sync>,
    ) -> Self;
    pub fn arm(&self, alarm: Alarm);
    pub fn cancel(&self, id: &str) -> bool;
    pub fn run(&self); // blocks; intended to be the body of the daemon's scheduler thread
}
```

## Boundaries — do NOT touch
Do not implement the real `herdr` CLI shell-out (that's Unit 04 — depend only on the `HerdrAdapter` trait above, and provide a `tests`-only fake, not a production stub, in this unit). Do not implement file persistence (`src/store/`) or the socket/IPC server (`src/daemon/`).

## Output
- `src/scheduler/mod.rs`
- `src/scheduler/heap.rs`
- `src/scheduler/clock.rs` (includes both `SystemClock` and, behind `#[cfg(test)]` or a `testing` feature, `FakeClock`)
- `tests/test_scheduler.rs`

## Verification
```bash
cargo test --test test_scheduler
cargo clippy --all-targets -- -D warnings
```
