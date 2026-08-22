# SPEC — `snuze`: The Asynchronous Agent Alarm Clock Primitive

**Status:** Ready for Review
**Owner:** Daniel Ramirez
**Drafted by:** Ciani (LUFS CTO), Phase 1 of the All-Hands pilot
**Companion repos:** `herdr` (external, herdrdev/herdr), `bplate`, `apho`, `lufs-audio/kb` (`docs/ecosystem/all-hands/`)

---

## 0. Deviations From the Kickoff Brief — Read This First

The brief asked for exit codes `0/2/3/4/5` and a "timer wheel." I'm shipping `0/2/3/4/5/6` and a min-heap, not a timer wheel. Reasons are in §5 and §3.1. Both are reversible in one conversation if Daniel wants strict adherence instead — flagging rather than silently deviating.

I also did not fit `snuze` into the House of Process Registries' Six Nouns (kernel/process/registry/recipe/handoff/verification). It doesn't manage a registry of interchangeable process entries with recipes; it's a scheduling daemon with one job. Forcing the frame on would be cargo-culting a pattern that doesn't apply — see the candidacy admission test in `docs/strategy/house-of-process-registries/10-candidacy.md`, which this project would likely fail on the "is there a real *registry* here" question. It still gets a kernel (types + errors) and a real verification contract; it just isn't a process registry.

---

## 1. Problem Statement

An agent operating in a Herdr pane dispatches a long-running background task (a Google Jules cloud session, a CI build, a batch job) and has no cheap way to resume when that work completes. Its only two options today are a synchronous polling loop (`sleep 30 && check`, burning turns and tokens for zero productive work) or ending its turn permanently and waiting on a human to notice and re-prompt it.

`snuze` is the missing primitive: a CLI an agent calls once, synchronously, to durably schedule a future wake-up, then exits immediately. Some time later, unattended, `snuze`'s background daemon delivers a prompt into the requesting agent's Herdr pane via Herdr's own agent-aware socket API. The agent's turn ends the moment it calls `snuze set`; it resumes only when the alarm actually fires.

The contract `snuze` sells is **not** "I will try to send a message later." It is: **"I will durably record this obligation before I return control to you, and I will tell you — via `snuze list`/`snuze status`, not via a swallowed background error — if I could not keep it."** The distinction matters because the whole reason this primitive exists is to let an agent walk away with confidence; a scheduler that can silently drop an alarm is worse than no scheduler, because it converts a visible polling loop into an invisible one.

### 1.1 Compliance With the Seven LUFS Tests

Per `bplate/SPEC.md` §2, every LUFS Primitive is held to seven tests. Checking `snuze` against the actual text rather than my own paraphrase of it (which is what Daniel asked for) surfaced two real gaps in the first draft, now fixed in this revision:

1. **Local-first and self-owned** — Yes. No network calls; the daemon talks only to a local Herdr socket via the `herdr` CLI. Zero telemetry.
2. **Verifiable** — Yes, and it's the load-bearing test for this primitive specifically: §3.3's delivery-verification design and §6's alarm status lifecycle exist so "the daemon said it fired" is a checkable claim, not an assumption.
3. **Agent-ergonomic** — Yes: `--json` on every subcommand, explicit exit codes (§5), zero interactive prompts. Unlike `bplate`, there's no TUI wizard — a scheduling daemon has no human-configuration step to wizard through, so this is an absent requirement, not a gap.
4. **Composable** — Gap in the first draft, fixed here: `snuze set` now takes `--dry-run` (validates duration + pane existence, prints what would be scheduled, arms nothing) — see §7 — matching `apho capture --dry-run` and `bplate new --dry-run`.
5. **Provenance-bearing** — Gap in the first draft, fixed here: every `Alarm` now records `created_by_pane: Option<String>`, captured from `$HERDR_PANE_ID` at schedule time (the real env var Herdr injects into every managed pane, per the `herdr` skill). Without this an alarm only knows *where it's going*, never *who asked* — a real provenance hole for a primitive whose entire point is trustworthy delegation. See Unit 01.
6. **Craft-grounded** — Yes: alarms, daemons, sockets, PID files are the native vocabulary of this domain.
7. **Automates toil, not taste** — Yes: scheduling is pure mechanical delegation; no creative judgment is in scope.

---

## 2. Architectural Overview

```
┌──────────────┐   snuze set --after 15m --pane w1:p3 --prompt "..."
│  CLI process │───────────────────────────────────────────────────┐
│  (snuze)     │                                                    │
└──────────────┘                                                    ▼
       │ exits 0 the instant the daemon                    ┌─────────────────┐
       │ ACKs durable persistence                          │  snuze daemon   │
       ▼                                                    │  (lazy-spawned  │
  agent's turn ends                                         │  on first use,  │
                                                             │  one per user)  │
                                                             ├─────────────────┤
                                                             │ alarm store     │
                                                             │ (durable, on    │
                                                             │  disk)          │
                                                             ├─────────────────┤
                                                             │ scheduler       │
                                                             │ (min-heap +     │
                                                             │  single thread) │
                                                             ├─────────────────┤
                                                             │ Herdr adapter   │
                                                             │ (shells out to  │
                                                             │  `herdr` CLI)   │
                                                             └────────┬────────┘
                                                                      │ agent.prompt via
                                                                      │ `herdr agent prompt <pane> "..."`
                                                                      ▼
                                                             ┌─────────────────┐
                                                             │  Herdr pane     │
                                                             │  w1:p3          │
                                                             │  (agent resumes)│
                                                             └─────────────────┘
```

`snuze` is a **CLI + daemon pair**, the first primitive in the LUFS family that requires a genuinely persistent background process rather than a stateless, single-shot invocation (contrast `apho`, explicitly "Zero Heavyweight Daemons"). That's a real architectural departure and it's why daemon lifecycle gets its own unit (§4, Unit 05) instead of being an afterthought.

---

## 3. Component Design

### 3.1 Scheduler — min-heap, not a timer wheel

The brief calls for a "timer wheel." A hierarchical timer wheel (Linux kernel timers, Netty's `HashedWheelTimer`, Kafka's purgatory) earns its complexity at tens of thousands to millions of in-flight timers, where O(1) amortized insert/cancel beats O(log n). `snuze`'s realistic working set is dozens of alarms per machine — the number of Herdr panes a single human/agent fleet keeps open at once. A binary min-heap keyed by deadline gives O(log n) insert/pop and is dramatically simpler to implement, reason about, and verify correctly. I'm building the min-heap and naming the module `scheduler`, not `timer_wheel`, so the code doesn't claim a data structure it isn't using. If alarm volume ever grows by two orders of magnitude, the heap is swappable behind the same interface — that's a real upgrade path, not a hand-wave.

The heap is an **in-memory, rebuildable index**, never the source of truth. On daemon startup it is rebuilt by scanning the durable alarm store (§3.2). A single dedicated OS thread owns the heap and sleeps until `min(next_deadline, wake_signal)` using a condvar — not a polling loop, and not a dependency on an async runtime. `snuze` has no other need for concurrency (Herdr delivery calls are sequential, one at a time, off the same thread); pulling in `tokio` for one sleeping thread would be a dependency the project doesn't need. This is a plain, blocking, single-purpose Rust binary in the spirit of `apho` and `lrex`.

### 3.2 Durable Alarm Store — filesystem-as-registry, reused not reinvented

Every other LUFS primitive that manages a collection of small records (Workchain components, `process` compositions, `apho`'s own `processes/`) uses the same convention: folder-per-entry plus a typed manifest, documented in the `filesystem-as-registry` skill. An alarm is exactly that kind of record — small, flat, human-legible — so `snuze` reuses the pattern instead of reaching for SQLite:

```
$XDG_STATE_HOME/snuze/alarms/<alarm-id>.json      # one file per alarm, atomic write-then-rename
$XDG_STATE_HOME/snuze/daemon.pid                   # PID file, advisory-locked
$XDG_STATE_HOME/snuze/daemon.sock                   # Unix domain socket (named pipe on Windows)
```//
(`$XDG_STATE_HOME` falls back to `~/.local/state` per the XDG Base Directory spec.)

Writes are durability-before-ack: the daemon fsyncs the alarm file and renames it into place *before* it ACKs the `snuze set` request, so a client that receives exit 0 has a guarantee stronger than "I asked" — it has "this is on disk." No separate index file is required for correctness (the heap is rebuilt by directory scan on boot); a generated `index.json` may be added later purely as a `snuze list` speed optimization once the directory scan is measurably slow, but is explicitly out of scope for v1 — do not build it speculatively.

### 3.3 Herdr Integration — CLI-wrapper-first, `agent prompt` as the primitive

**Revised from the first draft** — the first pass hedged on the exact CLI syntax pending verification; that verification is now done (against the real `herdr` agent skill, not the marketing docs page), so the hedge is gone and this is the actual contract.

Herdr's own agent skill documents the real invocation directly:

```bash
herdr agent prompt <name-or-pane> "<text>" [--wait] [--timeout <ms>]
```

`agent prompt` "atomically submits text and encoded Enter while honoring the pane's live bracketed-paste mode." With `--wait`, it waits for the first settled `idle`, `done`, or `blocked` state. Three outcomes matter to `snuze`, not the two the first draft assumed:

- **Delivered** — the call succeeds; if `--wait` was given, the target reached a settled state.
- **`agent_blocked`** — returned "without sending input or starting the wait" when the target is already blocked on something else. Not retried automatically in v1; treated as a failed delivery (a blocked agent needs a human/different intervention, not a queued prompt).
- **`agent_prompt_stalled`** — a real, distinct failure mode confirmed by the skill: "A prompt sent from a non-working state must produce an observed lifecycle change within five seconds. Otherwise Herdr returns `agent_prompt_stalled` instead of waiting indefinitely." This is neither "delivered" nor "pane gone" — the text went in, but nothing observably happened. It gets its own `DeliveryOutcome` variant (Unit 03) rather than being folded into either neighbor, because conflating it with "delivered" would be exactly the false-positive this whole SPEC exists to prevent, and conflating it with "pane gone" would misdiagnose a live-but-unresponsive pane as a dead one.

For `--command CMD` (target is a plain shell, not an agent session — no lifecycle semantics apply), the equivalent is `herdr pane run <pane> "<command>"`, confirmed by the same skill ("`pane run` atomically sends command text and Enter").

| Flag | Herdr command | Outcomes this maps to |
|---|---|---|
| `--prompt TEXT` | `herdr agent prompt <pane> "TEXT" --wait --timeout <ms>` | `Delivered`, `AgentBlocked`, `PromptStalled`, `HerdrUnavailable` |
| `--command CMD` | `herdr pane run <pane> "CMD"` | `Delivered`, `HerdrUnavailable` (no agent-lifecycle states apply to a plain pane) |

Both go through a `HerdrAdapter` trait (defined in Unit 03, implemented in Unit 04) so the scheduler can be tested with a fake adapter that never touches a real Herdr socket, and so a raw-socket transport can replace the CLI-shim transport later without touching scheduler or CLI code.

**Herdr's own exit-code convention, which the adapter must respect rather than reinterpret:** per the `herdr` skill, "CLI server errors are JSON on stderr with exit status 1. CLI syntax errors exit with status 2." A `herdr` exit code of `2` means *`snuze` invoked `herdr` incorrectly* — a `snuze` bug, surfaced as `DaemonUnavailable` with the stderr attached so it's loud, never silently swallowed as a normal delivery failure. A `herdr` exit code of `1` is the real "something about the request failed" signal, parsed from its JSON stderr payload to distinguish `agent_blocked`/`agent_prompt_stalled`/pane-not-found from a generic error.

**Delivery is verified, not assumed.** When the daemon fires an alarm, it checks the Herdr call's actual result — pane gone, blocked, stalled, or `herdr` itself unreachable all resolve to `failed` with a specific reason, never silently to `fired`. This is the specific gap in the All-Hands brief's own description of step 5 ("snuze daemon calls Herdr IPC/CLI to inject... into Pane 3") — it describes the happy path only. A primitive whose entire purpose is trustworthy async wake-up cannot fire-and-forget on the one call that matters; that would be exactly the "exit 0 but wrong" failure mode this company's whole verification doctrine exists to kill, just relocated into the one component nobody was watching because it runs unattended in the background.

`snuze` never calls anything destructive on Herdr — no `server stop`, no closing panes/tabs/workspaces it didn't create. Its entire Herdr footprint is one read (`pane get`/pane existence check) and one write (`agent prompt` or `pane run`), matching the `herdr` skill's own safety rules by construction, not by a special-cased guard.

### 3.4 Missed-Alarm Semantics

If the daemon was down when an alarm's deadline passed (machine slept, daemon crashed, host rebooted), firing it late and silently is not obviously correct — a "Jules finished, go check" prompt injected two hours late into a pane the agent has since repurposed is actively confusing. On daemon startup, any overdue alarm is handled by a grace window (default 10 minutes, configurable):

- Overdue by less than the grace window → fire normally, on daemon startup.
- Overdue by more than the grace window → mark `missed` (not `fired`, not silently dropped), visible in `snuze list --status missed`. Never injected.

### 3.5 Daemon Lifecycle

One daemon per machine/user, lazily spawned by the first `snuze set` if none is running (advisory lock on `daemon.pid` prevents two daemons racing the same store), with explicit `snuze daemon {start,stop,status,restart}` for anyone who wants to manage it directly rather than rely on lazy autostart. Startup sequence: acquire lock → scan alarm store → apply missed-alarm rule (§3.4) → rebuild heap → open socket → serve.

---

## 4. Unit Decomposition & Dependency Order

Six units, disjoint file boundaries, dependency-ordered for `land-plane`:

| Layer | Unit | Owns | Depends on |
|---|---|---|---|
| 1 | 01 — Kernel Types & Exit Codes | `src/kernel/` | — |
| 2 | 02 — Durable Alarm Store | `src/store/` | 01 |
| 2 | 03 — Scheduler & Herdr Adapter Trait | `src/scheduler/` | 01 |
| 2 | 04 — Herdr Adapter Implementation | `src/herdr/` | 01, 03 (trait signature only) |
| 3 | 05 — Daemon Lifecycle & IPC | `src/daemon/` | 01, 02, 03, 04 |
| 4 | 06 — CLI Surface & JSON Output | `src/cli/`, `src/main.rs` | 01, 05 |

Layer 2's three units can be dispatched in parallel — this is a real, concrete instance of the All-Hands fan-out, not a hypothetical one. 03 must publish the `HerdrAdapter` trait signature in its own contract (not defer it to 04) precisely so 04 can be built against a stable interface without waiting on 03's internals, per the interface-first discipline the `speccing` skill already calls for.

---

## 5. Exit Code Taxonomy

The brief asked for `0/2/3/4/5`. I'm proposing `0/2/3/4/5/6` — one more than asked, because "the `herdr` binary is missing" (a static, pre-flight fact) and "the daemon could not be reached or started" (a runtime failure that can happen even when the binary is present — stale lock from a dead process, socket refused after a spawn attempt) are different failure classes an operator needs to distinguish, and collapsing them costs real debuggability for no savings. Happy to fold `6` back into `4` if Daniel prefers strict adherence to the five-code ask.

| Code | Name | Meaning |
|---|---|---|
| `0` | `SUCCESS` | Command completed; for `set`, the alarm is durably persisted and the daemon confirmed it's armed |
| `2` | `USAGE` | Invalid arguments — bad duration syntax, missing required target, conflicting flags |
| `3` | `TARGET_UNAVAILABLE` | `set`: the Herdr pane doesn't exist per a live check at schedule time. `cancel`/`status`: alarm ID not found |
| `4` | `BINARY_MISSING` | `herdr` binary not found on `PATH` |
| `5` | `CONTRACT_VIOLATED` | Alarm could not be durably persisted (disk write failure, unresolvable lock contention, daemon rejected the request) |
| `6` | `DAEMON_UNAVAILABLE` | The `herdr`/`snuze` daemon could not be reached or spawned (distinct from 4: the binary exists, the daemon doesn't respond) |

This table only governs the **foreground CLI's** exit code. Whether an alarm eventually *fires* successfully is a separate, async fact recorded as alarm status (§3.4, §6) and surfaced through `snuze list`/`snuze status` — never through a process exit code, because by the time an alarm fires, the process that scheduled it is long gone and nothing is waiting on its exit status. Conflating "did the CLI call work" with "did the wake-up actually happen" is the single biggest correctness trap in this design, and it's the trap the brief's own CLI sketch doesn't visibly account for.

**Update, post-Daniel-review:** I went and read `bplate`'s own exit-code table (`docs/units/04-agent-cli-surface-and-exit-codes.md`) rather than relying on secondhand memory of it, and now have four real data points instead of three:

| Code | `lrex` | `apho` | `bplate` | `snuze` (this SPEC) |
|---|---|---|---|---|
| 0 | `SUCCESS` | `SUCCESS` | `SUCCESS` | `SUCCESS` |
| 2 | (usage) | `USAGE` | `USAGE` | `USAGE` |
| 3 | ? | `SOURCE_UNAVAILABLE` | `TARGET_EXISTS` | `TARGET_UNAVAILABLE` |
| 4 | ? | `BINARY_MISSING` | `PROFILE_NOT_FOUND` | `BINARY_MISSING` |
| 5 | ? | `CONTRACT_VIOLATED` | `CONTRACT_VIOLATED` | `CONTRACT_VIOLATED` |
| 6 | ? | `INTERRUPTED` | `REGISTRY_DRIFT` | `DAEMON_UNAVAILABLE` |
| 7 | ? | `UNVERIFIABLE` | `INTERRUPTED` | — |
| 70 | (distinct 7th code) | — | — | — |

The real finding isn't "everything has drifted" — it's the opposite: **`0`/`2`/`5` have converged independently, without anyone deciding that on purpose**, while `3`/`4`/`6`/`7` are legitimately domain-specific (a scaffolder needs `TARGET_EXISTS`; a capture tool needs `SOURCE_UNAVAILABLE`; a scheduler needs `DAEMON_UNAVAILABLE`) and forcing them into identical meanings across tools would be over-standardization for no benefit. Per Daniel's go-ahead, I've written the actual convention doc — `bplate/docs/units/10-exit-code-and-json-envelope-standard.md` — codifying the confirmed floor (`0`/`2`/`5` fixed, `1` reserved and never assigned, `3`/`4`/`6`+ open but must be fully documented per-tool) plus the JSON envelope shape from §7. It's a separate PR against `bplate`, referenced here rather than duplicated.

---

## 6. Alarm Status Lifecycle

```
pending ──(fires, herdr call succeeds)──> fired
        ──(canceled before firing)──────> canceled
        ──(fires, herdr call fails)─────> failed
        ──(daemon restarts, overdue > grace window)──> missed
```

All four terminal-ish states (`fired`/`canceled`/`failed`/`missed`) are retained in the store (not deleted) for a configurable retention window, so `snuze list --status failed` after the fact is how an agent or human discovers a wake-up silently didn't happen — the entire reason this section exists.

---

## 7. CLI Command Contract

Binary name `snuze`. Every subcommand supports `--json`. JSON output matches `bplate`'s own confirmed envelope shape (`bplate/docs/units/04-agent-cli-surface-and-exit-codes.md`) rather than inventing a different one — the first draft of this SPEC used a different, incompatible shape (`{"error": {"code": "...", ...}}`); fixed here for cross-primitive consistency:

```json
{"status": "success", "data": { /* subcommand-specific payload, e.g. an Alarm or a list of Alarms */ }}
{"status": "error", "code": 3, "message": "herdr pane w1:p9 not found"}
```

`code` in the error case is the numeric exit code from §5, not a string — an agent parsing `--json` output gets the same number on stdout as the process's own exit status, no second vocabulary to learn.

```text
# 0. Version
snuze --version

# 1. Schedule an alarm
snuze set --after DURATION (--pane PANE_ID) (--prompt TEXT | --command CMD)
          [--label TEXT] [--dry-run] [--json]
# Duration syntax: 30s, 15m, 1h, 90m, and simple compounds (1h30m)
# PANE_ID is Herdr's own workspace:pane form, e.g. w1:p3 — obtained by the
# caller via `herdr pane current --json`, never invented by snuze.
# --dry-run validates duration syntax and pane existence and prints what
# would be scheduled; it arms nothing and touches neither the store nor the
# daemon. Required for Seven-LUFS-Tests Composable compliance (SPEC §1.1).

# 2. List alarms
snuze list [--status pending|fired|failed|missed|canceled] [--json]

# 3. Inspect one alarm
snuze status ALARM_ID [--json]

# 4. Cancel
snuze cancel (ALARM_ID | --all) [--json]

# 5. Daemon control
snuze daemon start|stop|status|restart [--json]

# 6. Health check
snuze doctor [--json]
# Checks: herdr binary + version on PATH, herdr socket reachable
# (`herdr api schema` succeeds), snuze daemon running + socket reachable,
# state directory writable, count of pending/overdue/failed/missed alarms.
```

`--pane` and `--prompt`/`--command` are mutually exclusive with each other only in the sense that `--prompt` and `--command` cannot both be given; `--pane` is required for both. This mirrors `apho`'s own precedent of mutual-exclusivity validation happening synchronously before any work starts, exit code `2`.

---

## 8. Technology Stack

1. **Language:** Rust (edition 2021, `mise`-managed, matching `bplate rust-cli` profile defaults).
2. **CLI framework:** `clap`, derive macros, subcommands.
3. **Serialization:** `serde`, `serde_json`.
4. **Scheduling:** `std::collections::BinaryHeap` + `std::sync::{Mutex, Condvar}` + one dedicated thread. No async runtime.
5. **IPC:** Unix domain socket (Windows: named pipe), newline-delimited JSON — deliberately the same shape Herdr itself uses, for consistency of mental model, not because of a shared implementation.
6. **Herdr integration:** `std::process::Command` shelling out to the `herdr` CLI, matching Herdr's own documented recommendation to prefer CLI wrappers over the raw socket API for simple request/response use.
7. **Lints:** same zero-panic gate as every other LUFS Rust primitive — `unwrap_used = "deny"`, `expect_used = "deny"`, `panic = "deny"` in `[lints.clippy]`.

---

## 9. Verification Strategy

Per-unit `cargo test` coverage is necessary but not sufficient for a scheduler whose entire value proposition is "I keep my promises while unattended." Contract checks beyond unit tests:

- **Read-your-writes:** `snuze set` followed immediately by `snuze list` shows the alarm `pending` with the correct deadline.
- **Cancellation is real:** cancel a `pending` alarm, then fast-forward past its deadline (test clock injection, not a real sleep) — assert zero Herdr calls were made. Verifiable against the mock `HerdrAdapter` from Unit 03 without touching a real Herdr instance.
- **Timing tolerance:** an alarm set for a short duration fires within an explicitly bounded jitter window — the same empirical-tolerance discipline `lrex` already applies to its named-profile auto-stop timing.
- **Crash/restart durability (fault injection, not just unit tests):** schedule an alarm, `kill -9` the daemon before it fires, restart it, confirm the alarm still fires at its *original* deadline — not late, not never, not early. This is the `snuze`-specific analogue of the multi-device fault-injection test that already validated `lrex`'s per-device xrun attribution on real hardware; the same "prove it on a genuine fault, not a contrived one" standard applies here.
- **Delivery failure is visible:** point an alarm at a pane ID that doesn't exist; assert the alarm ends in `failed` with a reason, never `fired`.

What cannot be verified in CI: actually injecting a prompt into a *live* Herdr pane end-to-end requires a running Herdr instance and a real terminal, the same category of limitation `photo-process-registry`/`apho` hit with `--select`'s completed-drag path. That gets a `TESTING.md` runbook for a human (or a local agent with real Herdr access) to run once Unit 05 lands, not a CI job — flagged honestly now rather than discovered later.

---

## 10. Done Criteria

- [ ] Complete Rust crate compiling cleanly under `cargo build --release` with zero warnings.
- [ ] All 6 unit contracts implemented and verified per their own Verification sections.
- [ ] `cargo clippy --all-targets -- -D warnings` clean, zero-panic lints enforced via `Cargo.toml`.
- [ ] Fault-injection test (daemon kill -9 + restart) passes on real hardware, not just mocked.
- [ ] `snuze doctor --json` accurately reports herdr/daemon/store health.
- [ ] `TESTING.md` written for the one thing CI cannot cover: live Herdr pane delivery.
- [ ] This SPEC's two flagged deviations (exit code count, min-heap vs. timer wheel) either ratified or explicitly overridden by Daniel before Phase 2 scaffolding begins.

---

## 11. Condition-Driven Wake-Up Mode (Jules Polling) — v2 Feature

**Status:** Ready for Review — Phase 1 spec for `feature/jules-condition-mode`.
**Built against:** `main` at the point Units 01–06 landed, plus two already-merged-to-branch fixes (`6fdb422`, `a9c31502`) that removed `--wait` from prompt delivery — see §11.7.

### 11.0 Problem

`--after DURATION` is a blind guess. For a fixed local task you can estimate well; for a Google Jules cloud session you can't — sessions in practice range from under a minute (docs sweeps) to many minutes (real feature units), so a fixed alarm is either too early (agent wakes, polls, finds nothing, has to reschedule — the exact toil `snuze` exists to remove) or too late (the agent sits idle after Jules already finished). `snuze` should be able to poll Jules on the caller's behalf and fire the instant the real condition is true, with a hard ceiling so it never polls forever.

### 11.1 Architecture: generalize `Trigger`, not the whole alarm model

`Alarm` gets one new field, not a parallel data structure:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", default)]
pub enum Trigger {
    Deadline,
    JulesCondition {
        session_ids: Vec<String>,
        condition: JulesCondition,
        poll_interval_secs: u64,
        timeout_at: SystemTime,
        last_known: Vec<JulesSessionState>,
    },
}

impl Default for Trigger {
    fn default() -> Self { Trigger::Deadline }
}
```

`Alarm.deadline: SystemTime` (existing field, unchanged type) is repurposed in meaning rather than renamed, to avoid unnecessary churn across six already-shipped, tested units: for `Trigger::Deadline` it is (as today) the exact fire time; for `Trigger::JulesCondition` it becomes **"the next time the scheduler should check this alarm"**, recomputed after every poll and capped at `timeout_at`. Document this dual meaning directly on the field, not just here.

`Alarm` also gains `#[serde(default)] pub schema_version: u32` (default `1`). Every field added by this feature is `#[serde(default)]`, so an on-disk alarm record written by the currently-shipped binary deserializes unchanged with `trigger: Trigger::Deadline, schema_version: 1` — zero migration, purely additive. New records this feature writes set `schema_version: 2`. This is the first time `Alarm`'s on-disk shape has changed since it shipped; start the versioning discipline now rather than after it's already painful, matching this house's general "bump schema_version when extending a schema" rule.

**Why generalize `Trigger` instead of a second parallel primitive:** `list`/`status`/`cancel`, the store, the daemon's IPC protocol, and the JSON envelope are already generic over `Alarm` — none of them need to change to support this feature. The only genuinely new work is (a) what "is it time to act on this alarm" means for the new kind, and (b) how to actually check external state. Keeping one `Alarm` type means a mixed `snuze list` (some deadline alarms, some condition alarms) already works for free.

**Scheduler integration — the actual mechanism (Unit 10 owns this):** the min-heap stays a single, unified priority queue ordered by `deadline`/next-check-time, for *both* trigger kinds. When `Scheduler::run()` pops an alarm whose `Trigger` is `JulesCondition`, instead of unconditionally firing it:
1. Call `JulesAdapter::check_sessions(&session_ids)`.
2. If `condition.is_satisfied(&states)` → interpolate the template (§11.3) and `deliver()` through the existing `HerdrAdapter`, exactly like a deadline alarm.
3. Else if `now >= timeout_at` → terminal `Failed("timeout")`. Not `Missed` — `Missed` is specifically for "deadline passed while nobody was listening"; a condition that never resolved is a genuine, expected `timeout`, semantically distinct.
4. Else → persist updated `last_known` (so `snuze status` shows live progress on a pending poll, not just "pending"), set `deadline = min(now + poll_interval, timeout_at)`, re-push onto the same heap.

No second thread, no second wait mechanism, no parallel poll-loop subsystem — this is a genuine generalization of the existing scheduler, not a bolt-on. If the daemon restarts mid-poll, boot-time re-arm (§3.5 existing behavior) applies unchanged: a `JulesCondition` alarm whose `timeout_at` already passed while the daemon was down resolves immediately to `Failed("timeout")` on re-arm; one whose `timeout_at` hasn't passed yet is re-armed and polls again immediately (not after a full fresh interval) so a restart doesn't cost the caller a full `poll_interval` of extra latency.

### 11.2 CLI surface and condition semantics

```text
snuze set --jules SESSION_ID[,SESSION_ID2,...] --condition all-terminal|any-terminal|any-completed
          --timeout DURATION --pane PANE_ID --prompt TEXT
          [--poll-interval DURATION] [--label TEXT] [--dry-run] [--json]
```

`--jules` is mutually exclusive with `--after` (exactly one trigger mode per `set` call, exit `2` otherwise — the same validate-before-any-work pattern already used for `--prompt`/`--command`). `--command` delivery is **not supported** in condition mode for v1 (a condition-driven wake-up is definitionally about resuming an *agent*, not running a plain shell command — no motivating use case yet; easy to add later if one shows up). `--timeout` is **required** with `--jules` — unlike `--after`, where the deadline itself is the bound, an unbounded condition poll is a real resource leak (a typo'd or permanently-stuck session ID would poll forever). `--poll-interval` defaults to `30s`; exposed but not required.

**Recommendation on Daniel's open question (dedicated `--jules` vs. generalized `--poll-cmd --until-match`): dedicated `--jules`, with the architecture built so a generic poll mode could be added later without touching the scheduler again.** Reasoning: Jules's real state model (`QUEUED`/`PLANNING`/`AWAITING_PLAN_APPROVAL`/`IN_PROGRESS`/`COMPLETED`/`AWAITING_USER_FEEDBACK`/`PAUSED`/`FAILED` — verified via the `jules-api` skill, not re-derived here) carries real, distinct meaning that a generic string-match-on-command-output would flatten. `AWAITING_USER_FEEDBACK` specifically needs to wake the pane *immediately* even though the underlying session isn't "done" — that's exactly the kind of semantic a boolean predicate loses. The `JulesAdapter` trait (Unit 08) is scoped narrowly on purpose; a future `--poll-cmd` mode would be a second implementation of a more general `ConditionChecker`-shaped trait, added when a second real use case exists — not spec'd speculatively now. This is the same "resist generality creep, solve the concrete case well" call made elsewhere in this codebase.

**Condition semantics, defined precisely** (`states: &[JulesSessionState]`, one per tracked session ID) — "wake-worthy" states are `Completed | Failed | AwaitingUserFeedback | Paused` (`Paused`'s inclusion is a judgment call, not verified against detailed Jules docs — flag for confirmation at implementation time, same honesty standard as the original herdr CLI syntax before it was verified):
- **`any-terminal`**: fires the instant *any one* tracked session reaches a wake-worthy state. For "wake me when the first of these needs me."
- **`all-terminal`**: fires only once *every* tracked session has reached a wake-worthy state. For "wake me when the whole batch is ready to reconcile" — directly the All-Hands Phase 3 pattern (2 parallel Jules sessions per unit).
- **`any-completed`**: fires the instant *any one* tracked session reaches specifically `Completed`. Narrower than `any-terminal` — a session that's `Failed` or `AwaitingUserFeedback` does not satisfy this on its own; only `Completed` does, or the timeout.

**Pre-flight validation, mirroring the existing `--pane` check:** at schedule time (including under `--dry-run`), `set --jules` calls `JulesAdapter::check_sessions` once against the live API and fails fast with exit `3` (`TargetUnavailable` — its scope now covers "Jules session not found" in addition to "Herdr pane not found," one exit code, two documented target kinds, no new number needed) if any session ID doesn't resolve. Never arm a poll against a session that's already known not to exist.

### 11.3 Template interpolation — an explicit allowlist, not a template engine

`--prompt` text may contain `{session_id}`, `{status}`, `{duration}`, `{pr_urls}` (`{pr_urls}` is an addition beyond what was asked for, directly serving the stated motivation — Jules exposes a completed session's PR at `session.outputs[].pullRequest.url`, and the entire point of waking up is usually to go pull and reconcile that PR, per the All-Hands Phase 4 runbook). Interpolation happens once, at fire time (values aren't known at schedule time), in the scheduler right before calling `HerdrAdapter::deliver()` — not inside the Herdr adapter itself, since this is a `snuze`-level concern.

- `{duration}`: `fired_at - created_at`, formatted human-readable (`"12m34s"`). Works for **both** trigger kinds — free consistency, no reason to scope it to Jules only.
- `{session_id}`: the tracked session ID (single) or comma-joined list (multiple).
- `{status}`: the bare state name for a single tracked session (`"COMPLETED"`); for multiple, comma-joined `id:state` pairs (`"sess1:COMPLETED,sess2:FAILED"`) — precise on purpose, an agent parsing this needs to know *which* session did what, not just a count.
- `{pr_urls}`: comma-joined PR URLs for any session that completed with one attached; omitted (literal empty string is fine here specifically, since absence-of-a-PR is itself informative) if none apply.

**This is a fixed allowlist, never a generic template engine, and this is a security boundary, not a style preference:** a referenced variable this list doesn't recognize is left as **literal, unsubstituted text** in the output (never silently blanked, which would look like a bug and hide a typo) — and there is no path, ever, by which arbitrary environment content (most importantly `JULES_API_KEY`) can end up interpolated into a prompt that then gets injected into a Herdr pane and potentially logged or displayed. The interpolation function takes a small, closed struct of known-safe values, never a generic key-value environment dump.

### 11.4 `JulesAdapter` — a new trait, parallel to `HerdrAdapter`

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum JulesState {
    Queued, Planning, AwaitingPlanApproval, InProgress,
    Completed, AwaitingUserFeedback, Paused, Failed,
    /// Escape hatch for any state string the alpha API returns that isn't modeled above yet.
    /// Never panics on an unrecognized value — the jules-api skill's own "re-verify, alpha API"
    /// caution applies here too.
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JulesSessionState {
    pub session_id: String,
    pub state: JulesState,
    pub pull_request_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JulesCondition { AllTerminal, AnyTerminal, AnyCompleted }

impl JulesCondition {
    #[must_use]
    pub fn is_satisfied(&self, states: &[JulesSessionState]) -> bool { /* see Unit 07 */ }
}

pub trait JulesAdapter: Send + Sync {
    /// Query current state for each session ID. A single network/auth failure fails the
    /// whole batch (`SnuzeError::DaemonUnavailable`) rather than partially succeeding —
    /// a poll tick that can't see all tracked sessions can't safely evaluate `all-terminal`.
    fn check_sessions(&self, session_ids: &[String]) -> Result<Vec<JulesSessionState>, SnuzeError>;
}
```

**Credentials:** `RealJulesAdapter` reads `JULES_API_KEY` from the process environment at construction — a local env var, exactly like `HERDR_PANE_ID`, **not** routed through Ciani's Hyperagent-side `jules-api` skill or its credential proxy. These are two independent integrations to the same external API, run from two different runtimes (a local daemon vs. a cloud agent); don't conflate them. Missing key at construction time is a new `SnuzeError::JulesCredentialMissing` variant, mapped to the **existing** exit code `4` (`BinaryMissing`'s slot, whose spirit — "a required external dependency isn't available" — already covers this; no new number, matching the exit-code-reuse discipline from `bplate/docs/units/10-exit-code-and-json-envelope-standard.md`).

### 11.5 Unit decomposition — five new units, extending the existing dependency-layer pattern

| Layer | Unit | Owns | Depends on |
|---|---|---|---|
| 1 | 07 — Kernel Extension (`Trigger`, `JulesCondition`, `JulesSessionState`, template interpolation) | `src/kernel/` (extends) | 01 |
| 2 | 08 — `JulesAdapter` Trait & Mock | `src/jules/` (trait + mock only) | 07 |
| 3 | 09 — `RealJulesAdapter` (HTTP impl) | `src/jules/adapter.rs` | 07, 08 (trait signature only) |
| 3 | 10 — Scheduler Condition Integration | `src/scheduler/` (extends `run()`/`arm()`) | 07, 08 |
| 4 | 11 — CLI Condition-Mode Surface | `src/cli/` (extends) | 07, 10 |

Units 09 and 10 are both Layer 3 — dispatchable in parallel, same interface-first discipline used for the original Units 02/03/04 (08's contract publishes the full trait signature inline, so 09 doesn't need to wait on 08's actual merge, only its text). Mirrors the original shape (foundation → parallel middle layer → integration → surface) deliberately, so pi's fan-out runs the same rhythm as last time.

### 11.6 Exit codes and JSON envelope: no new codes needed

Every new failure mode this feature introduces already fits the existing `0/2/3/4/5/6` taxonomy once two things are documented (not code-changed): `TargetUnavailable` (`3`) now covers "Jules session not found" alongside "Herdr pane not found," and a new `SnuzeError::JulesCredentialMissing` variant shares `BinaryMissing`'s code (`4`) rather than minting a seventh number. A condition timing out is not a process-level error at all — like a Herdr delivery failure, it's an *alarm status* (`Failed`, reason `"timeout"`), discovered later via `snuze status`/`list`, never a `set` exit code — the same "foreground exit code and eventual outcome are different facts" principle from §5, now proven out on a second, independent feature. JSON envelope is unchanged (`{"status":"success","data":...}` / `{"status":"error","code":<n>,"message":...}`) — `Alarm`'s new fields just appear in `data` like any other field.

### 11.7 Built against the post-fix delivery path — and one open question it raises

This spec assumes the current (branch-tip) `RealHerdrAdapter::deliver()`, which no longer passes `--wait` to `agent prompt` (removed in `a9c31502`, after an intermediate `--until working` attempt in `6fdb422` — both already on `feature/jules-condition-mode`, read directly rather than assumed). That fix is correct: `--wait` was almost certainly blocking on the *target agent's entire subsequent turn* rather than confirming injection, which would make every real wake-up look like a timeout against the 5-second call budget. `Delivered` now means "herdr accepted and injected the text," a weaker but more honest guarantee for a primitive whose job is to wake a dormant agent, not babysit what it does next.

**Open question this raises, not resolved here:** the herdr skill describes `agent_prompt_stalled` as tied to the *waiting* behavior ("a prompt sent from a non-working state must produce an observed lifecycle change within five seconds — otherwise `agent_prompt_stalled`"). Without `--wait`, it's unverified whether herdr still performs that stall-check at all — if it doesn't, `RealHerdrAdapter::deliver()`'s existing `is_prompt_stalled()` branch is now dead code, reachable only in theory. Not a defect and not blocking this feature (condition-mode fires through the same `deliver()` unchanged either way), but worth a quick check against the real binary next time someone's already in that file — flagging honestly now rather than asserting an answer I haven't verified.

### 11.8 Additional Done Criteria for this feature

- [ ] `Alarm`'s `schema_version`/`trigger` fields round-trip correctly for both old (`schema_version: 1`, implicit `Trigger::Deadline`) and new on-disk records — a mixed store (some alarms from before this feature, some after) must not break `list`/`scan_errors`.
- [ ] A condition alarm's `snuze status` shows live `last_known` session states while `pending`, not just the bare status enum.
- [ ] `any-terminal` / `all-terminal` / `any-completed` each have a test exercising the exact boundary that distinguishes them (e.g., one session `Failed` + one `InProgress` satisfies `any-terminal` but not `all-terminal` or `any-completed`).
- [ ] The `agent_prompt_stalled` open question (§11.7) gets a one-line verified answer before it's relied upon anywhere new.
