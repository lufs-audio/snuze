# SPEC — `snuze` Phase 3: Push-Based Triggers & Work-Ledger Emission

**Status:** Ready for Review
**Owner:** Daniel Ramirez
**Drafted by:** Ciani (LUFS CTO), in-thread with Daniel, continuing the All-Hands pilot
**Phase folder:** `docs/specs/2026-08-22T2150Z_signal-triggers-and-ledger-emission/`
**Previous phase (archived, read-only reference — do not edit):** `docs/specs/2026-08-22T2145Z_initial-build-and-condition-mode/` (`SPEC.md` §§0–11, units 01–11: kernel, store, scheduler, Herdr delivery, daemon/IPC, CLI, and Jules condition-mode)
**Companion repos:** `herdr` (external, herdrdev/herdr), `webhook-deploy` (`lufs-audio/webhook-deploy`, service name `webhookd`), `bplate`, `lufs-audio/kb` (`docs/ecosystem/all-hands/09-generalization-and-the-work-registry.md`, `docs/strategy/house-of-process-registries/instantiation-all-hands.md`)

---

## 0. What This Phase Is, and the New Documentation Convention It's the First to Use

This is the first `snuze` phase authored under the timestamped-spec-folder convention (see the updated `speccing` skill). Prior to this restructuring, `snuze` had one `SPEC.md` at the repo root and one `docs/units/` directory that both phases (core build, then condition-mode) shared and kept extending in place. That's been archived unmodified into `docs/specs/2026-08-22T2145Z_initial-build-and-condition-mode/`, and the root `SPEC.md` is now a one-paragraph pointer into `docs/specs/`. Nothing about the archived phase's content changed in that move — same files, same text, new location.

This phase's units are numbered **01–04, restarting from 1**, not 12–15. Each phase folder is meant to be a complete, self-sufficient contract set for someone who opens *only* that folder — dependency-ordering within a phase should read cleanly on its own, without requiring the reader to already know a prior phase's numbering. Where a unit in this phase depends on something from the previous phase, it says so by full relative path (e.g. `docs/specs/2026-08-22T2145Z_initial-build-and-condition-mode/units/07-jules-kernel-extension.md`), never by a bare number. See §11 for the full convention writeup, since per Daniel's instruction this is also going into the `speccing` skill itself.

**Per that same instruction, every tool referenced in this phase that isn't `snuze` — Herdr, `webhookd`, the not-yet-built work ledger, Jules — is described once, here, at the phase level (§10). None of the four unit contracts in `units/` restate ecosystem context; they reference `SPEC.md §10` and stay scoped to `snuze`'s own source tree.**

---

## 1. Problem Statement

`snuze` has two trigger kinds today, both **pull-based**: a fixed deadline (`--after`), or polling Google Jules on an interval (`--jules`, phase 2). Two real gaps remain, and both motivate genuinely new mechanism rather than a bigger poll interval:

**Gap 1 — waiting on local Herdr state is currently only representable as a deadline guess, even though Herdr already pushes the answer.** The single most common wake-up condition in practice — "tell me when that other pane's agent goes idle/needs input/finishes" — has no representation in `snuze` beyond "guess a duration." But Herdr's own socket API already exposes exactly this as a push event (`events.subscribe` / `pane.agent_status_changed`, confirmed against the real `herdr` skill, the same source Unit 04 of the previous phase verified its CLI syntax against). Polling for a condition your dependency is already willing to push you is strictly worse engineering — extra latency, wasted checks, and it's the same class of avoidable toil `snuze` exists to eliminate in the first place.

**Gap 2 — nothing outside this machine can ever wake a `snuze` alarm, even though the fleet already has a working, running mechanism (`webhookd`) for exactly this shape of event.** A PR merging, a CI run finishing, any GitHub webhook — none of it can reach a pending alarm today. `--jules` polling only covers one specific external system (Google Jules); a webhook from anything else has no path in.

**Gap 3, named but only partly this phase's job to close — there is no durable record of what any of this actually accomplished.** `snuze list --status fired` answers "did this specific alarm go off," scoped to one daemon's retention window. It cannot answer "what has All-Hands built, this week, across every machine." [[instantiation-all-hands]] names this as the registry noun All-Hands is missing entirely. This phase does not build that registry — see §10.3 for exactly why not and what does — but it does build the one piece that has to live inside `snuze` regardless of where the registry ends up: a durable local record of every terminal event, ready to be shipped somewhere permanent by infrastructure that already exists.

### 1.1 Compliance with the Seven LUFS Tests — deltas only

The previous phase's SPEC (§1.1, archived) already walked all seven tests for `snuze`'s existing surface; re-litigating it here would be padding. Only the tests this phase's new surface actually touches:

- **Local-first and self-owned** — still true, and worth being exact about *why* it stays true even though this phase is explicitly about cross-machine wake-ups: `snuze` itself gains **zero new network code**. `Trigger::HerdrStatus` talks to the same local Herdr socket every existing trigger already does. `Trigger::ExternalSignal` is satisfied by a purely local IPC call (`snuze signal`, over the existing Unix socket). Anything that crosses an actual machine boundary — a GitHub webhook reaching `webhookd`, `webhookd` reaching the right host's `snuze` — happens entirely in infrastructure `snuze` has no awareness of. `snuze`'s network surface after this phase is identical to its network surface before it: none.
- **Verifiable** — the new outbox (§3.4) inherits the store's durability-before-ack discipline exactly: an event is only "in the outbox" once it's fsynced, same as an alarm is only "scheduled" once its file is fsynced and renamed into place (previous phase, §3.2).
- **Composable** — `snuze signal` and the `--signal`/`--herdr-pane` flavors of `snuze set` all get `--dry-run`, matching the existing convention exactly.
- **Provenance-bearing** — every outbox line carries `created_by_pane`, same as every `Alarm` already does; see §3.4's schema.

---

## 2. Architectural Overview

```
                        ┌───────────────────────────────────────────┐
                        │              snuze daemon                 │
                        │                                            │
  IPC (existing) ──────▶│  socket server thread(s)                  │
  + SignalAlarm (new)   │    │                                       │
  + DrainEvents  (new)  │    ▼                                       │
                        │  Scheduler (existing min-heap, extended)   │
                        │    │  arm() / cancel() / run()  (existing) │
                        │    │  signal()                   (NEW —   │
                        │    │   resolves matching pending alarms   │
                        │    │   outside the normal heap-pop path)  │
                        │    ▼                                       │
                        │  HerdrAdapter.deliver()  (existing)        │
                        │                                            │
                        │  ┌──────────────────────────────────────┐ │
   Herdr's own socket ─▶│  │ HerdrEventListener thread (NEW)      │ │
   events.subscribe     │  │  one persistent subscription;         │ │
   (push, not polled)   │  │  translates pane.agent_status_changed │ │
                        │  │  → Scheduler::signal(HerdrPaneEvent)  │ │
                        │  └──────────────────────────────────────┘ │
                        │                                            │
                        │  ┌──────────────────────────────────────┐ │
                        │  │ EventOutbox (NEW)                     │ │
                        │  │  events.ndjson, append-only            │ │
                        │  │  hooked into on_event + CancelAlarm   │ │
                        │  └──────────────────────────────────────┘ │
                        └───────────────────────────────────────────┘
                                          │
                          (nothing above this line is network code)
                                          │
                    ┌─────────────────────┴─────────────────────┐
                    │         infra outside this repo             │
                    │  webhookd (tambora) ── SSH/tailnet ── other  │
                    │  hosts' `snuze signal` ── (future) ledger    │
                    └───────────────────────────────────────────────┘
```

Two new things run inside the daemon process: a `HerdrEventListener` thread (one persistent, blocking connection to Herdr's own event stream) and an `EventOutbox` (not a thread — a small, lock-guarded file handle called from wherever terminal events already get decided). Neither is a second async runtime and neither polls; see §3.2 for why adding a third thread to a daemon that already coordinates the scheduler thread and the socket-serving thread(s) through a shared lock is a continuation of the existing architecture, not a new category of complexity.

---

## 3. Component Design

### 3.1 `Trigger` gains two more variants — same generalization move as last phase, third time now

`Trigger` was `Deadline` in phase 1, gained `JulesCondition` in phase 2, and gains `HerdrStatus` and `ExternalSignal` here. The justification from phase 2 still holds and gets stronger with each variant: `list`/`status`/`cancel`, the store, the IPC protocol, and the JSON envelope are generic over `Alarm` and need zero changes for a fourth or fifth trigger kind. The only genuinely new work per variant is (a) what "time to check/resolve this" means, and (b) how the daemon actually learns the answer. Three of `snuze`'s now-four trigger kinds share one more thing worth naming explicitly: `Deadline` is neither pull nor push (it's just time), `JulesCondition` is pull (poll on an interval), and `HerdrStatus`/`ExternalSignal` are both **push** — something external tells the daemon the moment the answer is known, rather than the daemon going and checking. Pull and push need different scheduler mechanics (§3.3), which is the actual architectural delta this phase adds — not a fifth data model.

```rust
// Extends the existing Trigger enum (defined in the previous phase's Unit 07,
// src/kernel/trigger.rs). Deadline and JulesCondition are UNCHANGED — this is
// purely additive, exactly like JulesCondition was additive to Deadline.
pub enum Trigger {
    Deadline,                          // unchanged
    JulesCondition { /* unchanged */ },
    HerdrStatus {
        watched_pane_id: String,
        watch_for: Vec<HerdrWatchStatus>,
        timeout_at: SystemTime,
        last_known: Option<HerdrWatchStatus>,
    },
    ExternalSignal {
        match_key: String,
        timeout_at: SystemTime,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HerdrWatchStatus { Idle, Done, Blocked, Working }
```

`schema_version` (introduced in phase 2) advances to `3` for any alarm this phase's CLI constructs; alarms written by phase 1 (`schema_version: 1`) or phase 2 (`schema_version: 2`) code continue to deserialize unchanged, per the same `#[serde(default)]` discipline already established — this phase does not touch that mechanism, only extends what it protects.

### 3.2 Why a `HerdrEventListener` thread doesn't violate "no async runtime, one dedicated thread"

The original SPEC's concurrency rule (§3.1, §8, archived) bans `tokio` and busy-polling; it was never a claim that the daemon is literally single-threaded — it can't have been, since Unit 05 already requires a socket-accept path running concurrently with the scheduler's own blocking `run()` loop, coordinated through the `Mutex`/`Condvar` the scheduler already owns. A `HerdrEventListener` is a third thread of the same kind: one long-lived, blocking read loop on a single Herdr subscription connection, which — on receiving a `pane.agent_status_changed` event — does nothing but call `Scheduler::signal(PushSignal::HerdrPaneEvent{..})` (§3.3) through that same existing lock. It performs no polling (it blocks on `events.wait`, per Herdr's own confirmed API, until Herdr itself has something to say) and pulls in no async runtime. This is additive to the existing thread model, not a new category of it.

**Resilience:** if the Herdr connection drops (Herdr restarts, a socket hiccup), the listener reconnects with a capped exponential backoff and logs the gap. A dropped connection cannot cause an incorrect *early* fire — only a delayed one, bounded by each pending `HerdrStatus` alarm's own `timeout_at`, which sits in the heap the whole time regardless of listener health. This is the same safety property `JulesCondition`'s timeout already provides against a flaky Jules API (phase 2, §11.1) — the ceiling is what makes "the push side might be down" an acceptable, bounded risk rather than an unbounded one.

### 3.3 `Scheduler::signal()` — one shared resolution path for both push trigger kinds

`HerdrStatus` and `ExternalSignal` are satisfied by two different sources (a Herdr event, an explicit CLI/IPC call) but resolve through **one** scheduler method, not two:

```rust
pub enum PushSignal {
    HerdrPaneEvent { pane_id: String, new_status: HerdrWatchStatus },
    ExternalSignal { match_key: String },
}

impl Scheduler {
    /// Finds every PENDING alarm whose trigger matches `signal`, removes each
    /// from the heap (reusing whatever internal mechanism `cancel()` already
    /// uses for arbitrary, non-minimum heap-element removal -- do not implement
    /// a second one), and fires them exactly like a normal heap-pop would: same
    /// interpolation, same HerdrAdapter::deliver() call, same Fired/Failed
    /// terminal-event emission. Zero matches is a valid, non-error outcome --
    /// broadcast semantics, not "exactly one recipient" semantics: if two
    /// alarms share a match_key, or two alarms watch the same pane for
    /// overlapping statuses, all matching alarms resolve on one signal.
    ///
    /// For `HerdrPaneEvent`: a HerdrStatus alarm matches iff
    /// `watched_pane_id == pane_id` AND `new_status` is a member of that
    /// alarm's own `watch_for` list. A pane transitioning to a status NOT in
    /// a given alarm's watch list does not fire that alarm -- it only updates
    /// that alarm's `last_known` for visibility (same "live progress, not
    /// just pending" requirement phase 2 established for Jules alarms).
    ///
    /// Returns the IDs of every alarm this call resolved.
    pub fn signal(&self, signal: PushSignal) -> Vec<String>;
}
```

Both new call sites are thin: the `HerdrEventListener` (§3.2) calls this on every Herdr event it receives, unconditionally — it does no matching of its own, keeping ALL trigger-matching logic in the one place that already owns it. The daemon's new `SignalAlarm` IPC handler (§3.5) calls this once per `snuze signal` invocation. Neither duplicates the match logic; both delegate to it.

`HerdrStatus` and `ExternalSignal` alarms still sit in the heap, keyed by `timeout_at`, exactly like every other trigger kind — `signal()` is a way to resolve them *early*; it is not a replacement for the heap. An alarm that never gets signaled still fires `Failed("timeout")` at its `timeout_at`, through the ordinary `run()` pop path, unchanged from how `JulesCondition`'s timeout already works.

### 3.4 Local Event Outbox — the emitter half of the registry gap

Per [[instantiation-all-hands]] and the new All-Hands page ([[09-generalization-and-the-work-registry]]), the registry/ledger itself is explicitly **not** this phase's job (§10.3). What *is* this phase's job: make sure `snuze` never has to be revisited later to retrofit "durable record of what happened," by emitting one durable, structured line per terminal event now, in a shape a future shipper can consume without knowing anything about `snuze`'s internals.

```
$XDG_STATE_HOME/snuze/events.ndjson
```

One JSON object per line, append-only, written with the same durability discipline as the alarm store (`write, fsync`; append mode makes the rename-into-place trick unnecessary here since a partial trailing line is trivially detectable and skippable by any reader, unlike a partially-written whole-file overwrite):

```json
{"schema_version":1,"emitted_at":"2026-08-22T21:50:00Z","host":"carnyx","tool":"snuze","alarm_id":"01J...","trigger_kind":"herdr_status","status":"fired","created_at":"2026-08-22T21:40:00Z","created_by_pane":"w1:p2","label":"jules-batch-review","failure_reason":null,"delivery_target_pane":"w1:p3","artifact_refs":[]}
```

- `host`: from `libc::gethostname()` — already a direct dependency (`Cargo.toml`), no new crate needed.
- `trigger_kind`: `"deadline" | "jules_condition" | "herdr_status" | "external_signal"` — the same tag `Trigger`'s own serde representation already produces; do not hand-maintain a second mapping (this is exactly the class of bug Ciani's Phase 5 audit found in the previous phase — `JulesState::as_str()` vs. its `Serialize` impl disagreeing on casing, and `format_alarm_human` hand-duplicating `as_str()` — see snuze#14 items 4–5. Reuse one source of truth for this string, on purpose.)
- `status`: `"fired" | "failed" | "missed" | "canceled"` — reuses `AlarmStatus`'s own serde representation, same reasoning.
- `artifact_refs`: always `[]` from `snuze` itself today — `snuze` has nothing to link (an alarm firing isn't an artifact). Reserved, not speculative: the field exists so a future ledger consumer's schema doesn't need a breaking change the day a tool that *does* produce artifacts (land-plane, say, linking a merged PR) starts emitting into the same conceptual shape. `snuze` does nothing with this field beyond always populating it with an empty array — no scope creep on `snuze`'s own implementation.
- **Scope: terminal events only.** `Progress` (phase 2) is explicitly excluded — it's operational, not a record of completed work, and including it would make the outbox chatty in proportion to poll frequency rather than proportion to actual events. This is a deliberate boundary, not an oversight.

**Two call sites, not one — read the current implementation before assuming otherwise.** The existing `on_event` handler (previous phase, Unit 10) covers `Fired`/`Failed`/`Missed` — those already flow through it. `Canceled` does **not**: cancellation resolves synchronously inside the daemon's `CancelAlarm` IPC handler via `Scheduler::cancel()`'s return value, never through `on_event`. A `EventOutbox::append()` call belongs at **both** sites. Missing the second one would silently under-report every canceled alarm in the eventual ledger — exactly the kind of "the design doc only describes the happy path" gap this house's verification doctrine exists to catch before it ships, not after.

**Consumption: `snuze events drain`, destructive by design, and an honestly-flagged limitation.** A new IPC request (`DrainEvents`) atomically swaps the outbox file for a fresh empty one and returns the old file's full contents — same atomic-rename primitive the store already uses, applied to "hand over everything and clear it" instead of "replace one record." This gives any external shipper (a cron job, a webhookd-triggered pull, anything) a simple, safe way to say "give me what's new and mark it consumed," without `snuze` needing to know who's shipping or where to.

This is **at-most-once delivery from `snuze`'s side**: a shipper that drains and then crashes before actually forwarding the batch loses it — `snuze` has already forgotten it by the time the shipper would retry. Acceptable for a v1 whose purpose is an audit trail, not a billing ledger; flagged honestly now rather than discovered later. If the eventual ledger needs stronger guarantees, the fix is a non-destructive `snuze events peek` plus a separate acknowledged-cursor, not a change to `drain`'s existing (simpler, and still useful) contract.

### 3.5 CLI and IPC additions

```text
# New alarm-arming modes (extend `snuze set`, mutually exclusive with
# --after/--jules and with each other -- exactly one trigger mode per call,
# exit 2 otherwise, matching the existing --after/--jules validation order)

snuze set --signal MATCH_KEY --timeout DURATION --pane PANE_ID
          (--prompt TEXT | --command CMD) [--label TEXT] [--dry-run] [--json]
# No pre-flight existence check is possible or needed -- a match_key is just a
# string; nothing to validate against a live system before arming.

snuze set --herdr-pane WATCHED_PANE_ID --watch-for STATUS1,STATUS2
          --timeout DURATION --pane PANE_ID (--prompt TEXT | --command CMD)
          [--label TEXT] [--dry-run] [--json]
# Pre-flight: RealHerdrAdapter::pane_exists(WATCHED_PANE_ID), reusing the
# existing check -- exit 3 on failure, with a message that says "watched pane"
# explicitly, never reusing generic PaneNotFound wording verbatim (see the
# new SnuzeError::WatchedPaneNotFound variant, unit 01 of this phase --
# distinct message, same exit code, precisely to avoid the Jules/Herdr
# pane-confusion bug the previous phase's audit already found once).
# --watch-for values: idle, done, blocked, working (comma-separated, at least
# one required). --pane (delivery) is checked exactly as it already is today.

# New standalone command
snuze signal MATCH_KEY [--dry-run] [--json]
# Resolves every pending ExternalSignal alarm whose match_key matches.
# Zero matches -> exit 0, {"status":"success","data":{"matched":0,"alarm_ids":[]}}.
# --dry-run reports what WOULD match without firing anything.

# New maintenance command
snuze events drain [--json]
# Atomically hands over and clears the local event outbox. Exit 6 if the
# daemon is unreachable (this goes through the daemon, not direct file
# access, so drain never races a concurrent append -- see §3.4).
```

No new exit codes. `WatchedPaneNotFound` and the existing `PaneNotFound` both map to `3` (`TargetUnavailable`), same reuse discipline as `JulesCredentialMissing` sharing `4` with `HerdrBinaryMissing` in phase 2.

---

## 4. Unit Decomposition & Dependency Order

Four units, this phase renumbered from 01 (see §0). Layer 2's two units touch overlapping files in only one place each (both add one match arm to `Scheduler::signal()`'s match-on-`PushSignal`, which is additive per-arm exactly like phase 2's Unit 10/`Scheduler::run()` pattern) — genuinely dispatchable in parallel.

| Layer | Unit | Owns | Depends on |
|---|---|---|---|
| 1 | 01 — Signal Trigger Kernel Extension | `src/kernel/` (extends) | Archived Unit 01 (kernel), Archived Unit 07 (`Trigger`) |
| 2 | 02 — Herdr Event Subscription Listener | `src/herdr/subscription.rs` (new), `src/scheduler/` (extends: `signal()`, `HerdrPaneEvent` arm) | This phase's Unit 01; Archived Units 03, 04 |
| 2 | 03 — External Signal IPC & CLI | `src/daemon/protocol.rs` (extends), `src/cli/` (extends), `src/scheduler/` (extends: `signal()`, `ExternalSignal` arm) | This phase's Unit 01; Archived Units 05, 06 |
| 3 | 04 — Local Event Outbox Emission | `src/daemon/outbox.rs` (new), `src/daemon/server.rs` (extends: both call sites, §3.4) | This phase's Unit 01 (for `trigger_kind` naming); Archived Unit 05's `on_event`/`CancelAlarm` handlers |

---

## 5. Alarm Status Lifecycle

Unchanged from the archived phase (`pending → fired | canceled | failed | missed`). `HerdrStatus` and `ExternalSignal` alarms use the same four terminal states; "timeout" is a `failure_reason` string under `Failed`, exactly like `JulesCondition`'s timeout already is — no new lifecycle state introduced for the third phase in a row, which is itself a small piece of evidence the lifecycle model generalizes cleanly across trigger kinds.

---

## 6. Technology Stack — deltas only

No new dependencies. `libc` (already present) supplies `gethostname()` for the outbox's `host` field. The Herdr subscription listener uses the same `std::process::Command`-or-socket transport Unit 04 (archived) already established for Herdr communication — if Herdr's `events.subscribe`/`events.wait` is only reachable over its raw socket API rather than through the CLI wrapper (verify against the real `herdr` skill before implementation, the same "verify against the installed binary, don't re-guess" discipline archived Unit 04 already models), that's a one-time exception to the "CLI-wrapper-first" preference from phase 1 (§3.3, archived) — a persistent subscription is not a request/response call, and Herdr's own skill documentation should be checked for which transport it recommends for exactly this case before assuming the CLI wrapper extends to it.

---

## 7. Verification Strategy — new contract checks beyond unit tests

- **Push, not poll:** a test asserting a `HerdrStatus` alarm fires within one event-loop tick of a matching mock Herdr event, with zero polling calls made in between (a mock `HerdrAdapter` that panics if queried outside the one delivery call would catch an accidental regression to polling).
- **Broadcast semantics:** two alarms sharing one `match_key`; one `snuze signal` call resolves both, verified via both alarms' terminal status, not just one.
- **Watch-list precision:** a `HerdrStatus` alarm watching only `[Idle]` does not fire on a `Blocked` event for the same pane, but does update `last_known`.
- **Timeout backstop:** a `HerdrStatus`/`ExternalSignal` alarm that never gets signaled fires `Failed("timeout")` at exactly `timeout_at`, identical in shape to the existing `JulesCondition` timeout test.
- **Listener resilience:** killing the mock Herdr connection mid-test and restarting it confirms the listener reconnects and a subsequent matching event still resolves a pending alarm — the daemon does not need to be restarted to recover from a transient Herdr-side disconnect.
- **Outbox durability + both call sites:** a fired alarm and a canceled alarm both produce exactly one outbox line each (this is the specific regression test for §3.4's "two call sites" requirement — a test that only exercises the `on_event` path would pass even if the `CancelAlarm` call site were missing entirely, so the cancellation case needs its own explicit assertion, not incidental coverage).
- **Drain is atomic w.r.t. concurrent append:** a test that appends from one thread while draining from another (or as close a simulation as the daemon's real locking allows) confirms no line is ever lost or duplicated across the swap boundary.

---

## 8. Done Criteria

- [ ] All four units implemented and verified per their own contracts in `units/`.
- [ ] `cargo clippy --all-targets -- -D warnings` clean; zero-panic lints unchanged.
- [ ] Every existing test from both prior phases (units 01–11, archived) passes unmodified.
- [ ] A live, manual runbook entry added to `TESTING.md` for the one thing CI cannot cover here either: subscribing to a *real* Herdr instance's event stream end-to-end (mirrors the existing live-Herdr-pane-delivery gap already documented for phase 1).
- [ ] `docs/ecosystem/all-hands/09-generalization-and-the-work-registry.md` and `docs/strategy/house-of-process-registries/instantiation-all-hands.md` (both `lufs-audio/kb`) stay accurate to whatever actually ships — if implementation reveals either doc's description of `webhookd`'s needed addition (§10.2) is wrong, fix the doc, don't let it silently drift.

---

## 9. Rejected / Deferred Alternatives, Named So They Don't Get Re-Litigated

- **A generic `--poll-cmd --until-match` trigger mode**, generalizing polling the way `--jules` special-cased it: still deferred, for the same reason phase 2 deferred it (§11.2, archived) — no second concrete use case yet beyond Jules, and the architecture (`Trigger` as an open enum, one shared heap) already accommodates adding it later without touching the scheduler's core loop again.
- **A `--webhook` trigger mode built directly into `snuze`** (the daemon itself listening for HTTP callbacks): rejected, not deferred. It would duplicate `webhookd`'s entire job (HMAC verification, route config, a public listening port) inside a tool whose first LUFS-test compliance claim is "no network calls." `ExternalSignal` plus `webhookd` already covers this shape without `snuze` ever opening a socket to the outside world — see §10.2.
- **`snuze` shipping its own outbox directly to a remote ledger** (an HTTP POST from inside the daemon): rejected for the identical reason. `snuze` emits; existing cross-machine infrastructure ships. Keeping this boundary is what keeps `snuze`'s "zero network calls" test true after three phases of feature growth, not just after phase 1.

---

## 10. Ecosystem References (Non-`snuze` Tools) — kept here, deliberately, not in any unit contract

### 10.1 Herdr (`herdrdev/herdr`, external)

Unchanged in kind from the previous phase's usage (CLI-wrapper delivery via `agent prompt`/`pane run`, verified against the real `herdr` skill) — this phase adds a *second* kind of Herdr integration: a long-lived subscription to Herdr's own event stream (`events.subscribe`/`events.wait`, `pane.agent_status_changed`), rather than a one-shot request/response call. `snuze` remains a consumer of Herdr's existing, documented API; nothing about Herdr itself needs to change for this phase to work.

### 10.2 `webhookd` (`lufs-audio/webhook-deploy`)

**Current, real, verified state, as of this writing:** a running service (`tambora:8644`) receiving GitHub webhooks over a Cloudflare Tunnel, verifying `X-Hub-Signature-256` (HMAC-SHA256) per route, dispatching to a configured deploy script per `webhooks.json` (secret, script, allowed events, branch filter per route), with a small ring-buffer event log (`GET /webhook/log`, last 100 events) and a health endpoint (`GET /health`). It has been running unattended, doing its documented job, since it was last touched.

**What this phase needs from it, not yet built:** one new route/handler, matching the exact shape every existing route already uses, that on a matching GitHub event computes a match key and calls `snuze signal <key>` — either directly if co-located, or relayed over the tailnet/SSH path this fleet already uses for cross-machine operations. Given the known, small, largely-static set of machines in play, the simplest correct v1 is a **broadcast** to every known host's `snuze` — `ExternalSignal`'s own no-match-is-a-no-op design (§3.5) absorbs the hosts where nothing was waiting, with no new "which machine has this alarm" directory service required. This is real, scoped, not-yet-started work; it is explicitly **not** part of this phase's four units (`snuze` has no idea `webhookd` exists — it only knows someone might call `snuze signal`), and it is `webhookd`'s repo, not `snuze`'s, that would carry it.

**`webhookd` stays tiny on purpose.** It is not, and should not become, the work ledger (§10.3) — its value is being a small, auditable, single-purpose dispatch core, which is a real design constraint worth protecting rather than a limitation to grow past.

### 10.3 The work ledger / registry — decisively named, deliberately not built here

Per [[instantiation-all-hands]] (`lufs-audio/kb`), All-Hands is currently missing the registry noun entirely: no durable, cross-project, queryable record of every build it has run, with links to what shipped. This phase's Unit 04 (§3.4) builds the emitter half `snuze` needs regardless of how that gets resolved — a local, durable, drainable outbox — but the ledger itself is a **new, separate, dedicated primitive**, not a feature of `snuze` (wrong retention philosophy: alarms are meant to be ephemeral and cleaned up; a ledger is meant to never forget) and not a feature of `webhookd` (see §10.2 — deliberately kept small). Its name, schema, and whether it's a standing service or a `filesystem-as-registry` instance of its own are **not decided by this SPEC** — that is real, acknowledged future work, named here so it doesn't need rediscovering, not specced here because doing so properly is its own phase, for its own repo, with its own SPEC.

### 10.4 Google Jules

Unchanged from phase 2. `Trigger::JulesCondition` is untouched by this phase; `RealJulesAdapter` and its polling model are neither extended nor superseded here — `HerdrStatus`/`ExternalSignal` are additive trigger kinds alongside it, not a replacement for it. Jules sessions still have no push mechanism of their own (no webhooks, per the `jules-api` skill's own verified notes), so polling remains the correct mechanism for that specific dependency until Jules itself changes.

---

## 11. Convention Note for Future Phases

Everything in §0 and this section is also being folded into the `speccing` skill directly (both the Hyperagent copy and the `lufs-audio/kb` mirror), so this isn't a one-off convention only `snuze` follows. Summarized here for this phase's own completeness:

- New work gets a new folder: `docs/specs/<UTC-timestamp>_<slug>/`, containing that phase's own `SPEC.md` and `units/`.
- Timestamp format: `YYYY-MM-DDTHHMMZ` — no colon (colons are illegal in Windows/NTFS filenames, and this codebase's own IPC layer explicitly supports Windows via named pipes, §8 archived), UTC always with an explicit trailing `Z` (never local time — the agents authoring these phases run on different machines in different timezones, sometimes different continents, and lexicographic sort across phases only equals chronological sort if every timestamp shares one reference frame).
- Units restart at `01` per phase folder. Cross-phase dependencies are cited by full relative path, never by bare number.
- Ecosystem references to tools outside the project being specced belong once, in that phase's own `SPEC.md` — never duplicated across unit contracts.
