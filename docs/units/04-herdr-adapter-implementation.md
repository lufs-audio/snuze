# Unit 04 — Herdr Adapter Implementation

## Objective
Implement the real `HerdrAdapter` (defined in Unit 03) by shelling out to the `herdr` CLI, plus a pre-flight pane-existence check used by `snuze set` before an alarm is ever armed.

## Context
See `SPEC.md` §3.3 (Herdr Integration). Depends on Unit 01 (`Alarm`, `SnuzeError`) and Unit 03's published `HerdrAdapter` trait signature only — do not depend on Unit 03's internal heap/scheduler implementation.

**Revised from the first draft.** The first pass of this unit hedged on the exact `herdr` CLI syntax pending verification. That verification is now done, against Herdr's own agent skill (the authoritative, installed-binary-matched reference — not the marketing docs site), and the real syntax is:

```bash
herdr agent prompt <name-or-pane> "<text>" --wait --timeout <ms>   # for --prompt
herdr pane run <pane> "<command>"                                   # for --command
```

Still verify the exact flag set against the installed binary (`herdr --help`, `herdr agent`) before implementation — Herdr versions independently of `snuze` and a flag could be renamed between the version this was checked against and whatever's actually on the build machine — but do not treat "verify the version" as license to re-guess the base syntax; it's now confirmed, not inferred.

## Herdr Outcomes, In Full

The first draft's `HerdrAdapter::deliver` only distinguished "delivered," "pane gone," and "herdr unavailable." That was wrong — Herdr's own agent skill documents a fourth, load-bearing case: "A prompt sent from a non-working state must produce an observed lifecycle change within five seconds. Otherwise Herdr returns `agent_prompt_stalled` instead of waiting indefinitely." A stalled prompt is neither delivered nor a dead pane — the text went in, nothing observably happened. Folding it into either neighbor would misreport what actually occurred, which is precisely the failure class this whole project exists to eliminate. Unit 03's `DeliveryOutcome` must therefore carry (at minimum): `Delivered`, `AgentBlocked(reason)`, `PromptStalled(reason)`, `PaneGone(reason)`, `HerdrUnavailable(reason)` — flag this back to whoever implements Unit 03 if it published a narrower enum before this revision landed.

**Herdr's own exit-code convention** (per the `herdr` skill): "CLI server errors are JSON on stderr with exit status 1. CLI syntax errors exit with status 2." This unit must distinguish the two: `herdr` exiting `2` means *this adapter invoked `herdr` incorrectly* — a bug in `snuze`, not a delivery failure — and should surface loudly (log the full invocation, map to `SnuzeError::DaemonUnavailable` with a reason that says "snuze invoked herdr incorrectly," not a generic timeout message that would send a future debugger looking in the wrong place). `herdr` exiting `1` is the real "something about the request failed" case, whose JSON stderr payload is parsed to distinguish `agent_blocked` / `agent_prompt_stalled` / pane-not-found from a truly generic error.

## Acceptance criteria
- [ ] `RealHerdrAdapter::new() -> Result<Self, SnuzeError>` locates the `herdr` binary on `PATH` (e.g. via `which`/`where` equivalent, or attempting `herdr --version` and checking the exit code); returns `SnuzeError::HerdrBinaryMissing` if absent. This is a constructor-time check, not deferred to first use.
- [ ] `RealHerdrAdapter::pane_exists(&self, pane_id: &str) -> Result<bool, SnuzeError>` shells out to whatever `herdr` subcommand answers "does this pane exist" (candidate: `herdr pane get <pane_id> --json`, confirm against the real CLI) and returns `true`/`false` based on the command's success/failure, mapping an unexpected `herdr` failure (non-zero exit that isn't "not found") to `SnuzeError::DaemonUnavailable`, not silently `false`. This is used by `snuze set` (Unit 06) to fail fast with exit code `3` (`TargetUnavailable`) at schedule time — an alarm should never be armed against a pane that's already gone.
- [ ] `RealHerdrAdapter::deliver`, implementing the `HerdrAdapter` trait from Unit 03:
  - For `DeliveryMode::Prompt`, runs `herdr agent prompt <pane_id> "<text>" --wait --timeout <ms>` (timeout from `self.call_timeout`, converted to milliseconds). Maps its outcome to exactly one of: `Delivered` (success); `AgentBlocked` (herdr's JSON stderr reports `agent_blocked` — target already blocked on something else, input not sent); `PromptStalled` (herdr's JSON stderr reports `agent_prompt_stalled` — text was sent but no lifecycle change was observed within Herdr's own 5-second window); `PaneGone` (herdr reports the pane/agent no longer resolves).
  - For `DeliveryMode::Command`, runs `herdr pane run <pane_id> "<command>"` against `alarm.pane_id`. Only `Delivered`, `PaneGone`, and `HerdrUnavailable` apply here — a plain pane has no agent-lifecycle states, so `AgentBlocked`/`PromptStalled` are not reachable through this path and must not be produced by it.
  - `herdr` exiting `2` (its own syntax-error convention) always maps to `HerdrUnavailable` with a reason string that explicitly says the invocation was malformed — never mapped to `PaneGone` or any alarm-side failure, since a `2` means the bug is in this adapter's shell-out, not in the target pane.
  - `herdr` exiting `1` (its own server-error convention) is where `agent_blocked`/`agent_prompt_stalled`/pane-not-found are actually distinguished, by parsing the JSON stderr payload.
  - Any timeout, unparseable output, or exit code neither `0`/`1`/`2` maps to `DeliveryOutcome::HerdrUnavailable`, reason truncated to a reasonable length (do not persist unbounded subprocess output into the alarm's `failure_reason` field).
- [ ] All subprocess calls run with an explicit timeout (propose 5 seconds — a local socket call to a running daemon should never legitimately take longer; a hang here would block the scheduler's single delivery thread indefinitely, stalling every subsequent alarm). A timed-out call is `DeliveryOutcome::HerdrUnavailable("timeout")`.
- [ ] Unit tests: since this unit's entire job is talking to a real external binary, unit tests use a fake `herdr` executable (a small test-only shell script placed on `PATH` via the test harness) configurable to: exit 0 (delivered), exit 1 with `agent_blocked` in its JSON stderr, exit 1 with `agent_prompt_stalled` in its JSON stderr, exit 1 with a pane-not-found payload, exit 2 (simulating a `snuze`-side invocation bug), and hang past the timeout — exercising every `DeliveryOutcome` branch, including confirming a `--command` delivery never produces `AgentBlocked`/`PromptStalled`, without requiring a real Herdr installation in CI. Document in the test file that these are **not** a substitute for the live-Herdr `TESTING.md` runbook (SPEC §9) — they prove this unit's error-mapping logic is correct given a known `herdr` response, not that the real `herdr` binary actually behaves as assumed.

## Interface contract
```rust
use snuze_kernel::SnuzeError;
use snuze_scheduler::{HerdrAdapter, DeliveryOutcome};
use snuze_kernel::Alarm;
use std::time::Duration;

pub struct RealHerdrAdapter {
    herdr_path: std::path::PathBuf,
    call_timeout: Duration,
}

impl RealHerdrAdapter {
    pub fn new() -> Result<Self, SnuzeError>;
    pub fn with_timeout(timeout: Duration) -> Result<Self, SnuzeError>;
    pub fn pane_exists(&self, pane_id: &str) -> Result<bool, SnuzeError>;
}

impl HerdrAdapter for RealHerdrAdapter {
    fn deliver(&self, alarm: &Alarm) -> DeliveryOutcome;
}
```

## Boundaries — do NOT touch
Do not implement scheduling/heap logic (`src/scheduler/`, beyond importing its published trait), persistence (`src/store/`), or the daemon socket server (`src/daemon/`). Do not modify Unit 03's trait definition — if the published signature is insufficient, flag it back to the coordinator rather than changing it unilaterally, since Unit 03 may already be in flight in parallel against the original signature.

## Output
- `src/herdr/mod.rs`
- `src/herdr/adapter.rs`
- `tests/test_herdr_adapter.rs`
- `tests/fixtures/fake_herdr.sh` (configurable fake binary for tests)

## Verification
```bash
cargo test --test test_herdr_adapter
cargo clippy --all-targets -- -D warnings
```
