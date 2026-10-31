# SPEC — `snuze` Phase 4: Generalized Ingress Triggers, Multiplexer Adapters & Jules Deprecation

**Status:** Ready for Review  
**Owner:** Daniel Ramirez  
**Phase folder:** `docs/specs/2026-10-31T0045Z_generalized-triggers-and-control-planes/`  
**Previous phases (archived, read-only references):**
- `docs/specs/2026-08-22T2145Z_initial-build-and-condition-mode/` (Phase 1 & 2: core scheduler, store, Herdr adapter, daemon, Jules condition mode)
- `docs/specs/2026-08-22T2150Z_signal-triggers-and-ledger-emission/` (Phase 3: push-based Herdr pane events, `snuze signal`, durable outbox)

---

## 1. Problem Statement

`snuze` was originally conceived and implemented as an asynchronous agent alarm clock within the Herdr and Jules All-Hands ecosystem. While the core scheduling engine, IPC daemon, and durability store have proven completely solid, two major architectural limitations and one maintenance liability have emerged in production:

1. **Vendor Coupling in the Core Kernel (The Jules Liability):**
   `Trigger::JulesCondition` baked Google Jules session polling directly into the scheduler kernel (`src/jules/`, `ureq` HTTP polling loops, `JulesSessionState`). With Google deprecating Jules and the workflow evolving, this third-party cloud logic has become dead weight. Embedding external SaaS APIs into a local scheduling primitive violates the LUFS Primitive standard—external state changes should enter `snuze` through generic signals or webhooks, never bespoke vendor pollers in the daemon.

2. **Control Plane Lock-in (Herdr Exclusivity):**
   Alarm delivery (`HerdrAdapter`) is hardcoded to `herdr agent prompt <pane_id>` and `herdr pane run <pane_id>`. When operators or autonomous agents run inside standard terminal multiplexers (`tmux`, `zellij`), background shell scripts, or headless pipelines, `snuze` cannot deliver prompts or commands, rendering it unusable outside of a running Herdr instance.

3. **Missing Local Tool Lifecycle Triggers (Process & File Ingress):**
   When coordinating with local CLI tools—such as `lrex` (recording audio takes) or `tscribe` (transcribing long-form audio)—an agent currently has to "guess" an alarm duration (`--after 30m`) or run wasteful polling loops. There is no first-class way to say *"wake me when this process exits"* (`--pid <PID>`) or *"wake me when this artifact is written"* (`--watch-file take.json`).

---

## 2. Goals & Constraints

### Goals
- **Pluggable Delivery Adapters (`tmux`, `herdr`, `command`, `wait`):** Generalize alarm delivery behind a modular `DeliveryAdapter` trait. Automatically select delivery target:
  - If `$HERDR_ENV=1`, defaults to Herdr.
  - If `$TMUX` is set, defaults to `tmux send-keys` / prompt injection.
  - `--target <type>:<id>` explicitly selects destination.
  - `snuze wait <alarm_id>` allows synchronous scripts to block cleanly without polling.
- **Process Exit Ingress (`Trigger::ProcessExit`):** Add `--pid <PID>` to fire an alarm as soon as a monitored process terminates (e.g. `snuze set --pid $(pgrep lrex) --prompt "Recording done"`).
- **File Artifact Ingress (`Trigger::FileArtifact`):** Add `--watch-file <PATH>` to fire when a file or directory contract is satisfied (e.g. `take.json` or `transcript.md`).
- **Clean Deprecation of Jules:** Deprecate/remove `src/jules/` and `Trigger::JulesCondition`. Route external event notifications through the existing, highly reliable `snuze signal <KEY>` mechanism.

### Constraints
- **Zero Breakage for Existing Herdr Workflows:** Existing `snuze set --pane <id> --after ...` syntax continues to work unchanged.
- **Zero Network Code in `snuze` Core:** Removing Jules HTTP code returns `snuze` to a 100% local-first, zero-network daemon.
- **Fast and Lightweight:** Process and file watching must not spin aggressive loops; use `/proc`, `kill(pid, 0)`, and event-driven backoff.

---

## 3. Ecosystem References

- **`lufs-audio/lrex` (Audio Recorder):** Long-running process emitting contract files (`take.json`). Snuze will coordinate take completion without active agent polling.
- **`lufs-audio/tscribe` (Transcription Primitive):** Asynchronous transcription engine producing `transcript.md`. Snuze will alert agents upon job completion.
- **`tmux` (Terminal Multiplexer):** Standard multiplexer for terminal agents (`tmux send-keys -t <target> "<prompt>" Enter`).
- **`herdr` (`herdrdev/herdr`):** Agent terminal multiplexer (`herdr agent prompt`, `herdr pane run`).

---

## 4. Architectural Overview

```
                        ┌───────────────────────────────────────────┐
                        │              snuze daemon                 │
                        │                                           │
  INGRESS TRIGGERS      │  Scheduler & Event Loop                   │  EGRESS ADAPTERS
 ──────────────────     │   ├── Deadline (Timer)                    │ ─────────────────
 • --after <dur>   ────►│   ├── ProcessExit (--pid <PID>)      ─────┼► • HerdrAdapter
 • --pid <PID>     ────►│   ├── FileArtifact (--watch-file)    ─────┼► • TmuxAdapter
 • --watch-file    ────►│   ├── ExternalSignal (--signal)      ─────┼► • CommandAdapter
 • --signal <KEY>  ────►│   └── HerdrStatus (--herdr-pane)     ─────┼► • Waiter (CLI)
                        │                                           │
                        └───────────────────────────────────────────┘
```

### 4.1 Pluggable Delivery Targets
The domain model `Alarm` generalizes `pane_id: String` to:
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeliveryTarget {
    Herdr { pane_id: String },
    Tmux { target: String },
    Command { command: String },
    Wait,
}
```

### 4.2 Ingress Triggers
`Trigger` is updated to remove `JulesCondition` and add:
```rust
pub enum Trigger {
    Deadline,
    ProcessExit {
        pid: u32,
        timeout_at: SystemTime,
    },
    FileArtifact {
        path: PathBuf,
        timeout_at: SystemTime,
    },
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
```

---

## 5. Done Criteria

- [ ] `snuze set --tmux <pane> --after 1m --prompt "hello"` wakes the agent in a `tmux` session.
- [ ] `snuze set --pid <PID> --prompt "Process exited"` fires immediately upon process termination.
- [ ] `snuze set --watch-file <PATH> --prompt "File ready"` fires upon file creation.
- [ ] `snuze wait <alarm_id>` blocks synchronously until the alarm fires, exiting with code 0.
- [ ] `jules/` module removed, eliminating external `ureq` HTTP network dependencies.
- [ ] All existing test suites pass; new unit and integration tests verify tmux delivery and process/file triggers.

---

## 6. Units of Work

- **Unit 01 (`units/01-pluggable-delivery-and-tmux-adapter.md`):** `DeliveryTarget` enum, `DeliveryAdapter` trait, `TmuxAdapter`, and target auto-detection.
- **Unit 02 (`units/02-process-exit-trigger.md`):** `Trigger::ProcessExit`, PID monitoring in daemon, and `--pid` CLI option.
- **Unit 03 (`units/03-file-artifact-trigger.md`):** `Trigger::FileArtifact`, path existence/modification watcher in daemon, and `--watch-file` CLI option.
- **Unit 04 (`units/04-jules-deprecation-and-wait-command.md`):** Remove Jules module/code, add `snuze wait <id>` command, and update CLI/docs.
