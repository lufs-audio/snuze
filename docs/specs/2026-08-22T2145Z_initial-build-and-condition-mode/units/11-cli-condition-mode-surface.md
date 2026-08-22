# Unit 11 — CLI Condition-Mode Surface

## Objective
Extend `snuze set` with `--jules`/`--condition`/`--timeout`/`--poll-interval`, wire the pre-flight Jules session check, and extend `list`/`status` human output to show live poll progress for pending condition alarms.

## Context
See `SPEC.md` §11.2 (CLI surface, condition semantics, pre-flight validation). Depends on Unit 07 (`Trigger`, `JulesCondition`, kernel duration parsing reused for `--timeout`/`--poll-interval`) and Unit 10 (the daemon/scheduler must already understand `Trigger::JulesCondition` for the end-to-end integration test below to mean anything). Existing reference, extended in place: `src/cli/args.rs`'s `SetArgs`, `src/cli/handlers.rs`'s `handle_set`/`handle_list`/`handle_status`, `src/cli/output.rs`'s `format_alarm_human` — read the current implementations before extending; every existing Unit 06 acceptance criterion and test must keep passing unmodified.

## Acceptance criteria
- [ ] `SetArgs` gains: `--jules <SESSION_IDS>` (comma-separated, `Option<String>`), `--condition <all-terminal|any-terminal|any-completed>` (`Option<String>`), `--timeout <DURATION>` (`Option<String>`), `--poll-interval <DURATION>` (`Option<String>`, default handled in the handler, not `clap`, so the default value is visible in one place — the handler — rather than split across the arg definition and the handler).
- [ ] `handle_set` validation, in order, before any daemon contact (mirroring the existing `--dry-run` ordering discipline in `SPEC.md` §7): (1) exactly one of `--after` or `--jules` must be present — exit `2` otherwise; (2) if `--jules`, exactly one of `--prompt` must be present and `--command` must be **absent** — exit `2` if `--command` is given with `--jules` (condition mode does not support command delivery, per `SPEC.md` §11.2 — this is a deliberate scope limit, not an oversight, and the error message must say so, not just "conflicting flags"); (3) if `--jules`, `--condition` and `--timeout` are both **required** — exit `2` if either is missing, with a message naming which one; (4) parse `--condition`'s string into `JulesCondition` (exit `2` on an unrecognized value, listing the three valid ones in the error message); (5) parse `--timeout` and `--poll-interval` (if given) via the existing `parse_duration` — exit `2` on failure, identical to `--after`'s existing handling; (6) split `--jules`'s comma-separated value into session IDs, trimming whitespace, rejecting empty entries (e.g. a trailing comma) with exit `2`.
- [ ] After validation, construct `RealJulesAdapter::new()` (exit per `SnuzeError::JulesCredentialMissing`'s mapping, i.e. `4`, if `JULES_API_KEY` is unset) and call `check_sessions` against all listed session IDs as the pre-flight check — exit `3` (`TargetUnavailable`) if any session ID doesn't resolve, mirroring the existing `--pane` pre-flight check exactly. This happens **before** the `--dry-run` early return, same ordering rationale as the existing pane check.
- [ ] Build the `Alarm` with `trigger: Trigger::JulesCondition { session_ids, condition, poll_interval_secs: poll_interval.unwrap_or(30).as_secs(), timeout_at: now + timeout, last_known: vec![] }` (empty `last_known` at schedule time — first real state comes from the first poll) and `deadline` set to `now + poll_interval` (the first check happens after one interval, not instantly — avoids a wasted immediate poll for a session that was just dispatched and is almost certainly still `QUEUED`). `schema_version: 2` (per `SPEC.md` §11.1) on any alarm this unit constructs, `Trigger::Deadline` alarms keep `schema_version: 1` for now (no behavioral difference yet, but keeps the version field honest about which code path produced a record).
- [ ] `--dry-run` with `--jules` follows the same contract as the existing `--after` path: print the fully-constructed alarm (including its `Trigger::JulesCondition` payload) and exit `0` without calling `Daemon::ensure_running` or `DaemonClient::set_alarm`.
- [ ] `format_alarm_human` (or an equivalent path in `handle_status`/`handle_list`): for an alarm whose `trigger` is `Trigger::JulesCondition` and `status` is `Pending`, render each tracked session's `last_known` state on its own line (e.g. `"  sess1: IN_PROGRESS"`) rather than only the bare `Pending` status — this is the `SPEC.md` §11.8 "live progress, not just pending" requirement. For `--json` output this is free (the full `Alarm` including `trigger.last_known` already serializes), but the human-readable path needs explicit formatting work.
- [ ] Unit tests: argument-parsing tests for every new flag combination's valid and invalid cases (missing `--condition`/`--timeout` with `--jules`, `--command` with `--jules`, malformed `--jules` value, unrecognized `--condition` value) confirming exit `2` before any adapter/daemon call would occur; an integration test using both `fake_herdr.sh` (Unit 04's fixture) and a fake Jules HTTP server (Unit 09's fixture, or a lighter-weight stand-in if the full mock server is heavier than this test needs) that runs a real end-to-end `set --jules ... --condition any-completed --timeout 5m` against a real backgrounded daemon, confirms `snuze status` shows an `IN_PROGRESS`-style `last_known` line while pending, then confirms it transitions to `fired` once the fake Jules server starts returning `COMPLETED` — this is the one test in the whole feature that proves Units 07/08/09/10/11 actually integrate, not just that each compiles in isolation.

## Interface contract
```rust
// Additive to the existing SetArgs (Unit 06) -- do not restate the whole struct,
// add exactly these fields:
//   #[arg(long)]
//   pub jules: Option<String>,
//   #[arg(long)]
//   pub condition: Option<String>,
//   #[arg(long)]
//   pub timeout: Option<String>,
//   #[arg(long)]
//   pub poll_interval: Option<String>,

use snuze_kernel::{Alarm, Trigger, JulesCondition, SnuzeError};
use snuze_jules::RealJulesAdapter;

/// Parses and validates `--jules`'s comma-separated value into individual session IDs.
///
/// # Errors
/// Returns `SnuzeError::ConflictingFlags` (reused: this is a malformed-input case,
/// same family as the existing mutual-exclusivity errors) on an empty list or any
/// empty entry (e.g. a trailing comma).
pub fn parse_jules_session_ids(raw: &str) -> Result<Vec<String>, SnuzeError>;

/// Parses `--condition`'s string value.
///
/// # Errors
/// Returns `SnuzeError::ConflictingFlags` on any value other than the three
/// recognized ones, with the valid options named in the message.
pub fn parse_jules_condition(raw: &str) -> Result<JulesCondition, SnuzeError>;
```

## Boundaries — do NOT touch
Do not implement the real Jules HTTP client (Unit 09, beyond calling `RealJulesAdapter::new()`/`check_sessions` exactly as Unit 06 already calls `RealHerdrAdapter::new()`/`pane_exists`). Do not touch scheduler or daemon internals beyond what Unit 10 already specified. Do not change `--after`/`--pane`/`--prompt`/`--command`'s existing validation or behavior for `Trigger::Deadline` alarms — this unit is additive.

## Output
- `src/cli/args.rs` (modified: new `SetArgs` fields)
- `src/cli/handlers.rs` (modified: `handle_set` validation + construction branch, `handle_list`/`handle_status` progress formatting)
- `src/cli/jules_parse.rs` (new: `parse_jules_session_ids`, `parse_jules_condition`)
- `src/cli/output.rs` (modified: `format_alarm_human` condition-progress rendering)
- `tests/test_cli_args.rs` (modified: new argument-parsing cases, existing cases unchanged)
- `tests/test_cli_jules_integration.rs` (new)

## Verification
```bash
cargo build --release
cargo test --test test_cli_args --test test_cli_integration --test test_cli_jules_integration
cargo clippy --all-targets -- -D warnings
```
