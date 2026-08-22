# Unit 04 — Local Event Outbox Emission

## Objective
Implement the durable, append-only local event outbox (`events.ndjson`) and its `drain` operation — the emitter half of the registry gap named in `[[instantiation-all-hands]]`. This unit does not build a ledger, a shipper, or anything that leaves this machine; it makes sure every terminal alarm event, regardless of trigger kind, is durably recorded somewhere a future shipping process can consume, and that nothing about `snuze`'s own "zero network calls" property changes to get there.

## Context
See `SPEC.md` §3.4 (full schema, the two-call-sites requirement, the `drain` design and its explicitly-flagged at-most-once-delivery limitation) and §10.3 (why the ledger itself is out of scope here). Depends on this phase's Unit 01 (for `trigger_kind`/`status` string values — reuse `Trigger`'s and `AlarmStatus`'s own serde tag representations directly; do not hand-maintain a second string mapping, which is exactly the class of bug Ciani's Phase 5 audit of the previous phase found twice — see `SPEC.md` §3.4's citation of snuze#14 items 4–5). Also depends on the **archived** previous phase's Unit 05 (`Daemon`, the existing `on_event` callback wiring in `src/daemon/server.rs`, and the `CancelAlarm` IPC handler — read both call sites as they exist today before adding to them).

**Read `src/daemon/server.rs`'s current `on_event` handler and `CancelAlarm` handler in full before writing anything.** This unit's single most important requirement is adding a call to *both*, not just the more obvious one (`on_event`) — see the acceptance criteria below and `SPEC.md` §3.4 for exactly why the `CancelAlarm` path is easy to miss and would silently under-report cancellations if it were.

## Acceptance criteria
- [ ] `EventOutbox::open(state_dir: &Path) -> Result<EventOutbox, SnuzeError>` resolves to `<state_dir>/events.ndjson` (using the same `resolve_state_dir()` single-source-of-truth the store already uses — do not compute the state directory a second, independent way). Creates the file if absent; does not fail or truncate if it already exists and has content.
- [ ] `EventOutbox::append(&self, event: &OutboxEvent) -> Result<(), SnuzeError>` serializes `event` to one JSON line and appends it to the file, followed by a single `\n`, with an `fsync` after the write — durability-before-ack, matching the store's own discipline, applied to append rather than write-then-rename (a torn/partial trailing line from a crash mid-write is trivially detectable and skippable by any reader that splits on newlines and skips a line that fails to parse; this is why append mode doesn't need the store's temp-file-then-rename trick — a full-file overwrite would need it, an append does not). Maps any I/O failure to `SnuzeError::PersistFailed` (reuse — this is a persistence failure like any other, no new variant needed).
- [ ] `EventOutbox::drain(&self) -> Result<Vec<OutboxEvent>, SnuzeError>` atomically swaps the current file for a fresh, empty one (write a new empty temp file, `fsync`, rename over the original — the same primitive the store already uses for atomic replacement, applied here to "replace with empty" instead of "replace with an updated record") and returns every event successfully parsed from the *old* file's contents, skipping (not failing on) any unparseable trailing line. An outbox file that doesn't exist yet drains to an empty `Vec` successfully — never an error; a tool that has never fired an alarm has nothing to drain, and that is not a failure state.
- [ ] `OutboxEvent` struct and its exact JSON shape, per `SPEC.md` §3.4:
  ```json
  {"schema_version":1,"emitted_at":"...","host":"...","tool":"snuze","alarm_id":"...","trigger_kind":"...","status":"...","created_at":"...","created_by_pane":null,"label":null,"failure_reason":null,"delivery_target_pane":"...","artifact_refs":[]}
  ```
  `host` via `libc::gethostname()` (already a dependency — do not add a `hostname` crate). `trigger_kind` and `status` serialize using `Trigger`'s and `AlarmStatus`'s own existing serde tag strings directly (construct the `OutboxEvent` from a `&Alarm` plus the terminal outcome, extracting these via the existing types' own `Serialize` impls or an equivalent shared helper — not a hand-written `match` that could drift from them). `artifact_refs` is always `vec![]` from this unit's own code — the field exists for forward schema compatibility with future non-`snuze` emitters, and this unit does nothing further with it.
  - [ ] Scope: only `AlarmStatus::Fired | Failed | Missed | Canceled` produce an outbox line. `FiredEvent::Progress` (previous phase) is explicitly excluded — never call `append` from anywhere in the `Progress` handling path.
- [ ] **Two call sites, both required:**
  1. The daemon's existing `on_event` handler (`src/daemon/server.rs`) — already the persistence point for `Fired`/`Failed`/`Missed` via `AlarmStore::put` (previous phase, Unit 05) — gains one additional call: after persisting the terminal status, construct an `OutboxEvent` from the alarm's final state and `EventOutbox::append` it.
  2. The daemon's `CancelAlarm` IPC handler — which resolves a cancellation synchronously via `Scheduler::cancel()`'s return value and does **not** go through `on_event` at all — gains its own `EventOutbox::append` call for the newly-`Canceled` alarm, immediately after `AlarmStore::put` persists the cancellation. This is a second, independent integration point; do not assume routing everything through `on_event` covers it.
- [ ] New IPC request `DrainEvents` (`src/daemon/protocol.rs`) with no parameters, returning the drained `Vec<OutboxEvent>`. Handler calls `EventOutbox::drain()` directly — no additional logic in the handler itself.
- [ ] New CLI command `snuze events drain [--json]`: calls `DaemonClient::drain_events` (new method, same shape as existing daemon-client calls); exit `6` (`DaemonUnavailable`) if the daemon can't be reached — this command requires the daemon (it owns the only safe, lock-serialized access to the outbox file; see `SPEC.md` §3.4 for why direct file access from the CLI is deliberately not offered). Human output: a count and a short per-line summary; `--json` output: the full array of drained events.
- [ ] Unit tests: `append` then `drain` round-trips events losslessly, in order; `drain` against a nonexistent file returns an empty `Vec`, not an error; a deliberately-truncated trailing line (simulating a crash mid-append) is skipped by `drain` without failing the rest of the batch (mirror the store's own existing "one corrupt record doesn't break listing everything else" test); a fired alarm and a **canceled** alarm each independently produce exactly one outbox event — this specific test must exercise the `CancelAlarm` path directly (calling `cancel` on a pending alarm and then draining), not just `on_event`, since a test that only exercises `on_event` would pass even if the `CancelAlarm` call site were never added; a `Progress` event (previous phase's Jules polling) never produces an outbox line, confirmed by polling a `JulesCondition` alarm through several non-terminal ticks and draining zero events until it actually resolves; a concurrent-append-during-drain test (append from one path while a drain is in flight, using the daemon's existing internal locking, not a bespoke test-only lock) confirms no event is ever lost or duplicated across the swap boundary.

## Interface contract
```rust
use std::path::Path;
use std::time::SystemTime;
use snuze_kernel::{Alarm, AlarmStatus, SnuzeError};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OutboxEvent {
    pub schema_version: u32, // 1
    pub emitted_at: SystemTime,
    pub host: String,
    pub tool: String, // always "snuze"
    pub alarm_id: String,
    pub trigger_kind: String,   // Trigger's own tag string -- not a hand-written mapping
    pub status: String,          // AlarmStatus's own tag string -- not a hand-written mapping
    pub created_at: SystemTime,
    pub created_by_pane: Option<String>,
    pub label: Option<String>,
    pub failure_reason: Option<String>,
    pub delivery_target_pane: String,
    pub artifact_refs: Vec<String>, // always empty from snuze; reserved for future emitters
}

impl OutboxEvent {
    /// Constructs an event from a terminal alarm. `status` must be one of
    /// Fired/Failed/Missed/Canceled -- callers are responsible for never
    /// calling this for a Pending alarm or a Progress tick.
    #[must_use]
    pub fn from_terminal_alarm(alarm: &Alarm, status: AlarmStatus) -> Self;
}

pub struct EventOutbox { /* opaque */ }

impl EventOutbox {
    pub fn open(state_dir: &Path) -> Result<Self, SnuzeError>;
    pub fn append(&self, event: &OutboxEvent) -> Result<(), SnuzeError>;
    /// Atomically swaps the outbox for an empty one; returns everything that
    /// was in it. Never errors on a missing/empty file -- returns Ok(vec![]).
    pub fn drain(&self) -> Result<Vec<OutboxEvent>, SnuzeError>;
}

// Additive to DaemonClient (previous phase, src/daemon/client.rs):
impl DaemonClient {
    pub fn drain_events(&self) -> Result<Vec<OutboxEvent>, SnuzeError>;
}
```

## Boundaries — do NOT touch
Do not implement anything that ships outbox contents anywhere off this machine — no HTTP client, no `webhookd` call, nothing. `SPEC.md` §9 rejects an in-`snuze` shipping path outright; this unit's entire job ends at "durably recorded and drainable on the local filesystem." Do not implement the `HerdrEventListener` (Unit 02) or the `SignalAlarm`/`--signal`/`--herdr-pane` surface (Unit 03), beyond reading `trigger_kind` values their `Trigger` variants already produce. Do not modify `AlarmStore`'s own on-disk format or the alarm retention/cleanup logic — the outbox is a separate file with a separate lifecycle.

## Output
- `src/daemon/outbox.rs` (new: `EventOutbox`, `OutboxEvent`)
- `src/daemon/server.rs` (modified: both call sites — `on_event` handler and `CancelAlarm` handler — plus `DrainEvents` request handler)
- `src/daemon/protocol.rs` (modified: `DrainEvents` request/response)
- `src/daemon/client.rs` (modified: `DaemonClient::drain_events`)
- `src/cli/handlers.rs` (modified: `handle_events_drain`)
- `src/cli/args.rs` (modified: `events drain` subcommand)
- `tests/test_outbox.rs` (new)
- `tests/test_daemon_outbox_integration.rs` (new — covers both call sites explicitly)

## Verification
```bash
cargo test --test test_outbox --test test_daemon_outbox_integration --test test_daemon
cargo clippy --all-targets -- -D warnings
```
