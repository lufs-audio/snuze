# Unit 03 — External Signal IPC & CLI

## Objective
Implement `Scheduler::signal()`'s `ExternalSignal` resolution arm, the daemon's new `SignalAlarm` IPC request, the standalone `snuze signal` command, and the `--signal`/`--herdr-pane`/`--watch-for` additions to `snuze set`. This is the unit that actually gives outside infrastructure (a script, a webhook handler run by something like `webhookd`) a way to reach a pending alarm — everything else in this phase is upstream of this being possible.

## Context
See `SPEC.md` §3.3 (`Scheduler::signal()`'s shared contract — this unit implements the other half of the method Unit 02 introduces) and §3.5 (full CLI/IPC surface, exit codes). Depends on this phase's Unit 01 (`Trigger::ExternalSignal`, `Trigger::HerdrStatus`, `WatchedPaneNotFound`) and Unit 02 (the `PushSignal` enum and `Scheduler::signal()`'s existence — this unit adds the `ExternalSignal` match arm to a method Unit 02 already created; coordinate the exact merge if both are in flight simultaneously, same as archived Units 09/10 coordinated around `JulesAdapter`'s trait signature). Also depends on the **archived** previous phase's Unit 05 (`Daemon`, `DaemonClient`, the socket protocol's request/response enum — read `src/daemon/protocol.rs` as it exists today) and Unit 06 (`SetArgs`, `handle_set`, the existing `--pane`/`--prompt`/`--command`/`--dry-run` validation ordering).

## Acceptance criteria

### IPC
- [ ] Add `SignalAlarm { match_key: String }` to the daemon's existing internal request enum (`src/daemon/protocol.rs`), with a response carrying the resolved alarm IDs (`Vec<String>`). The daemon's handler for this request calls `scheduler.signal(PushSignal::ExternalSignal{match_key})` and returns its result directly — this handler contains no matching logic of its own, matching the "all matching logic lives in `Scheduler`" discipline `SPEC.md` §3.3 establishes.
- [ ] `Scheduler::signal()`'s `ExternalSignal` arm (extending the method Unit 02 introduced): find every pending alarm whose `trigger` is `Trigger::ExternalSignal` with `match_key` equal to the signaled key. Every match fires — broadcast semantics, identical in spirit to Unit 02's pane-event broadcast (two alarms sharing a `match_key` both resolve on one `snuze signal` call). Same heap-removal reuse requirement as Unit 02 (reuse `cancel()`'s existing arbitrary-removal mechanism).
- [ ] `DaemonClient` (previous phase, Unit 05) gains a `signal_alarm(&self, match_key: &str) -> Result<Vec<String>, SnuzeError>` method following the exact shape of its existing `cancel_alarm`.

### CLI — `snuze set` additions
- [ ] `SetArgs` gains: `--signal <MATCH_KEY>` (`Option<String>`), `--herdr-pane <PANE_ID>` (`Option<String>`, the *watched* pane — distinct flag name from `--pane`, which remains the delivery target), `--watch-for <STATUS_LIST>` (`Option<String>`, comma-separated), `--timeout <DURATION>` (reuse the flag Unit 11 of the archived phase already added for `--jules`, extending its requirement to these two new modes rather than adding a second timeout flag).
- [ ] `handle_set` validation, before any daemon contact, extending the existing `--after`/`--jules` mutual-exclusivity check into a three-way (now effectively four-way counting `--herdr-pane`) exclusivity: exactly one of `--after`, `--jules`, `--signal`, `--herdr-pane` may be present — exit `2` otherwise, with a message naming all the mutually-exclusive flags, not just the two the previous phase knew about.
- [ ] `--signal MATCH_KEY --timeout DURATION --pane PANE_ID (--prompt|--command)`: no pre-flight existence check (a match key is just a string — see `SPEC.md` §3.5). Constructs `Trigger::ExternalSignal { match_key, timeout_at: now + timeout }`. `--command` **is** supported here (unlike `--jules`, which forbids it) — an external signal is a general-purpose wake-up, not definitionally about resuming an agent the way a Jules condition is; no reason to carry over that restriction to a mode it doesn't apply to.
- [ ] `--herdr-pane WATCHED_PANE_ID --watch-for STATUS_LIST --timeout DURATION --pane PANE_ID (--prompt|--command)`: requires `--watch-for` (exit `2` if absent, listing the four valid values in the message) and `--timeout` (exit `2` if absent). Parses `--watch-for`'s comma-separated value into `Vec<HerdrWatchStatus>`, rejecting unrecognized values and empty entries with exit `2` (mirroring the existing `--jules` session-ID-list parsing in `src/cli/jules_parse.rs` — reuse that file's pattern, do not invent a second comma-list parser). Pre-flight: calls `RealHerdrAdapter::pane_exists(WATCHED_PANE_ID)` — exit `3` on failure, returning `SnuzeError::WatchedPaneNotFound` (Unit 01), **never** the existing `SnuzeError::PaneNotFound`, even though both map to the same exit code — the message must say "watched pane," and a caller checking the *delivery* `--pane`'s existence (the existing check, unchanged) must still get the existing `PaneNotFound` wording for that failure. Both checks run in the same `handle_set` call for this mode; do not let one shadow or replace the other's distinct error identity.
- [ ] Both new modes respect the existing `--dry-run` contract exactly: print the fully-constructed alarm and exit `0` without calling `Daemon::ensure_running`/`DaemonClient::set_alarm` — the watched-pane pre-flight check (for `--herdr-pane`) still runs before the dry-run early return, same ordering rationale already established for `--pane`/`--jules`.
- [ ] `schema_version: 3` on any alarm this unit's code constructs (per `SPEC.md` §3.1); `--after`/`--jules`-constructed alarms are untouched by this unit and keep their existing `schema_version`.

### CLI — new `snuze signal` command
- [ ] `snuze signal MATCH_KEY [--dry-run] [--json]`: without `--dry-run`, calls `DaemonClient::signal_alarm` (lazy-autostarting the daemon exactly like `set` does, since signaling requires a running daemon to have anything to signal) and reports the resolved alarm IDs; zero matches is success (exit `0`), not an error — `{"status":"success","data":{"matched":0,"alarm_ids":[]}}`. With `--dry-run`, reports which currently-pending alarms *would* match without calling `signal()`'s firing path at all (requires a read-only daemon query — `DaemonClient::list_alarms` filtered client-side by trigger kind and match key is sufficient; does not require a new daemon-side dry-run code path).
- [ ] Unit tests: argument-parsing tests for the new mutual-exclusivity cases (`--signal` with `--jules`, `--herdr-pane` without `--watch-for`, `--herdr-pane` without `--timeout`, malformed `--watch-for` value) confirming exit `2` before any adapter/daemon call; an end-to-end integration test (using `fake_herdr.sh`, the existing fixture) that arms a `--herdr-pane` alarm and a `--signal` alarm against a real backgrounded daemon, then confirms `snuze signal <key>` resolves the `--signal` alarm (and only that one) via `snuze status`; a broadcast test confirming two `--signal` alarms sharing one match key both resolve on a single `snuze signal` call; a test confirming `snuze signal` on a key with zero pending matches exits `0` with `matched: 0`, never a nonzero exit.

## Interface contract
```rust
// Additive to the existing SetArgs (previous phase, src/cli/args.rs):
//   #[arg(long)]
//   pub signal: Option<String>,
//   #[arg(long)]
//   pub herdr_pane: Option<String>,
//   #[arg(long)]
//   pub watch_for: Option<String>,
//   // --timeout already exists (added for --jules); reused, not redefined.

use snuze_kernel::{HerdrWatchStatus, SnuzeError};

/// Parses --watch-for's comma-separated value.
///
/// # Errors
/// Returns `SnuzeError::ConflictingFlags` on an empty list, an empty entry,
/// or any value other than idle/done/blocked/working.
pub fn parse_watch_statuses(raw: &str) -> Result<Vec<HerdrWatchStatus>, SnuzeError>;

// Additive to the daemon's existing request/response protocol
// (previous phase, src/daemon/protocol.rs):
//   SignalAlarm { match_key: String } -> Vec<String> (resolved alarm IDs)

// Additive to DaemonClient (previous phase, src/daemon/client.rs):
impl DaemonClient {
    pub fn signal_alarm(&self, match_key: &str) -> Result<Vec<String>, SnuzeError>;
}
```

## Boundaries — do NOT touch
Do not implement the Herdr subscription listener or its `PushSignal::HerdrPaneEvent` resolution logic (Unit 02, beyond adding the `ExternalSignal` match arm to the shared `signal()` method it created). Do not implement the event outbox (Unit 04). Do not change `--after`/`--jules`'s existing validation, construction, or behavior — this unit is additive to `handle_set`, in the same spirit archived Unit 11 was additive to archived Unit 06.

## Output
- `src/daemon/protocol.rs` (modified: `SignalAlarm` request/response)
- `src/daemon/client.rs` (modified: `DaemonClient::signal_alarm`)
- `src/daemon/server.rs` (modified: `SignalAlarm` handler, calling `Scheduler::signal`)
- `src/scheduler/mod.rs` (modified: `ExternalSignal` arm of `signal()`, extending Unit 02's method)
- `src/cli/args.rs` (modified: new `SetArgs` fields, new `SignalArgs`)
- `src/cli/handlers.rs` (modified: `handle_set` extensions, new `handle_signal`)
- `src/cli/jules_parse.rs` or a new `src/cli/watch_parse.rs` (new: `parse_watch_statuses`, following the existing comma-list-parsing pattern)
- `tests/test_cli_args.rs` (modified: new argument-parsing cases)
- `tests/test_cli_signal_integration.rs` (new)
- `tests/test_scheduler_signal.rs` (modified — extends Unit 02's file with `ExternalSignal` coverage)

## Verification
```bash
cargo build --release
cargo test --test test_cli_args --test test_cli_signal_integration --test test_scheduler_signal --test test_daemon
cargo clippy --all-targets -- -D warnings
```
