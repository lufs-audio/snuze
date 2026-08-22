# Unit 01 — Signal Trigger Kernel Extension

## Objective
Extend the kernel with two new `Trigger` variants (`HerdrStatus`, `ExternalSignal`), their supporting types, and one new error variant — the foundation every other unit in this phase depends on. Depends only on already-shipped kernel types; ships nothing that talks to Herdr, a socket, or a file.

## Context
See `SPEC.md` §3.1 (Trigger generalization) and §3.5 (CLI/exit-code reuse). This unit extends types defined in the **archived, previous-phase** units — read them before changing anything:
- `docs/specs/2026-08-22T2145Z_initial-build-and-condition-mode/units/01-kernel-types-and-exit-codes.md` (`Alarm`, `SnuzeError`, `ExitCode`, the `From<&SnuzeError> for ExitCode` mapping)
- `docs/specs/2026-08-22T2145Z_initial-build-and-condition-mode/units/07-jules-kernel-extension.md` (the `Trigger` enum this unit adds variants to, and the `schema_version`/`#[serde(default)]` backward-compat discipline this unit must continue)

Do not restate or modify `Trigger::Deadline` or `Trigger::JulesCondition` — this unit is purely additive, exactly as `JulesCondition` was additive to `Deadline` one phase ago.

## Acceptance criteria
- [ ] Add `HerdrWatchStatus` enum: `Idle, Done, Blocked, Working`, `#[serde(rename_all = "snake_case")]` (so CLI/wire values are `idle`/`done`/`blocked`/`working`).
- [ ] Add `Trigger::HerdrStatus { watched_pane_id: String, watch_for: Vec<HerdrWatchStatus>, timeout_at: SystemTime, last_known: Option<HerdrWatchStatus> }` and `Trigger::ExternalSignal { match_key: String, timeout_at: SystemTime }` to the existing `Trigger` enum, under the existing `#[serde(tag = "kind", rename_all = "snake_case")]` representation — do not change the tagging scheme, only add variants to it.
- [ ] Bump the CLI-constructed default `schema_version` this phase's own alarms carry to `3` (do not touch the `Default`/`#[serde(default)]` behavior itself — an on-disk record from either prior phase must continue to deserialize exactly as it does today; this criterion is about what NEW alarms this phase's CLI writes, handled in Unit 03, not a kernel-level default change).
- [ ] Implement `fn herdr_status_satisfied(watch_for: &[HerdrWatchStatus], current: HerdrWatchStatus) -> bool` — `true` iff `current` is a member of `watch_for`. Simpler than `JulesCondition::is_satisfied` by construction: there is exactly one watched pane per alarm, so there is no all-of/any-of/batch distinction to make — only "is the one current status one I care about."
- [ ] Add `SnuzeError::WatchedPaneNotFound(String)` as a **distinct** variant from the existing `SnuzeError::PaneNotFound(String)` (which remains the delivery-pane error), with its own `#[error("...")]` message that says "watched Herdr pane" explicitly. Map it to the same `ExitCode::TargetUnavailable` (`3`) as `PaneNotFound` in the existing `From<&SnuzeError> for ExitCode` match — reuse the code, never reuse the message. This split exists specifically because Ciani's Phase 5 audit of the previous phase (issue snuze#14, item 2) found a real bug caused by exactly this kind of conflation: a Jules-session-not-found error was mapped through `PaneNotFound`'s wording via string-matching, producing a message that said "herdr pane not found" about something that was never a Herdr pane. Do not repeat that pattern here by reusing `PaneNotFound` for a second, different kind of "pane" (watched vs. delivery).
- [ ] Unit tests: `herdr_status_satisfied` true/false cases including a status not in the watch list and an empty watch list (must be `false`, never vacuously true — same "an empty set of criteria is not satisfied" discipline `JulesCondition::is_satisfied` already established for an empty `states` slice); `Trigger::HerdrStatus` and `Trigger::ExternalSignal` both round-trip through `serde_json` losslessly; an `Alarm` JSON blob with no `trigger` field at all (simulating a phase-1 on-disk record) still deserializes to `Trigger::Deadline` — a regression test proving this phase's additions didn't disturb the two-phases-old default; `WatchedPaneNotFound` maps to exit code `3` and its `Display` string contains "watched", `PaneNotFound`'s does not.

## Interface contract
```rust
// Additive to the existing Trigger enum (previous phase, src/kernel/trigger.rs).
// Do not restate Deadline/JulesCondition -- add exactly these two variants:
//
//   HerdrStatus {
//       watched_pane_id: String,
//       watch_for: Vec<HerdrWatchStatus>,
//       timeout_at: SystemTime,
//       last_known: Option<HerdrWatchStatus>,
//   },
//   ExternalSignal {
//       match_key: String,
//       timeout_at: SystemTime,
//   },

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HerdrWatchStatus { Idle, Done, Blocked, Working }

#[must_use]
pub fn herdr_status_satisfied(watch_for: &[HerdrWatchStatus], current: HerdrWatchStatus) -> bool;

// Additive to the existing SnuzeError enum (previous phase, src/kernel/errors.rs).
// Add exactly this variant, and extend the existing ExitCode match arm:
//
//   #[error("watched Herdr pane not found: {0}")]
//   WatchedPaneNotFound(String),
//
// ...
//   SnuzeError::PaneNotFound(_) | SnuzeError::WatchedPaneNotFound(_) | SnuzeError::AlarmNotFound(_)
//       => ExitCode::TargetUnavailable,
```

## Boundaries — do NOT touch
Do not implement the Herdr subscription listener (Unit 02), the `SignalAlarm` IPC path or CLI flags (Unit 03), or the event outbox (Unit 04). Do not modify `Trigger::Deadline`, `Trigger::JulesCondition`, or any existing `Alarm` field. Do not touch `src/herdr/`, `src/daemon/`, `src/scheduler/`, or `src/cli/`.

## Output
- `src/kernel/trigger.rs` (modified: two new `Trigger` variants, `HerdrWatchStatus`, `herdr_status_satisfied`)
- `src/kernel/errors.rs` (modified: `WatchedPaneNotFound` variant + exit code mapping)
- `src/kernel/mod.rs` (modified: export new types)
- `tests/test_kernel_signal.rs` (new)

## Verification
```bash
cargo test --test test_kernel --test test_kernel_jules --test test_kernel_signal
cargo clippy --all-targets -- -D warnings
```
