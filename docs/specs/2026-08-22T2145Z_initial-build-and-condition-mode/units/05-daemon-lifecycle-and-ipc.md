# Unit 05 — Daemon Lifecycle and IPC

## Objective
Wire the Store (Unit 02), Scheduler (Unit 03), and Herdr Adapter (Unit 04) into a single long-running daemon process with a local socket server, lazy autostart, single-instance enforcement, and crash-recovery rebuild on boot. This is the first LUFS primitive to require a genuine background service, not a stateless CLI — treat lifecycle correctness as the primary deliverable of this unit, not a wrapper around the other three.

## Context
See `SPEC.md` §3.5 (Daemon Lifecycle), §3.4 (Missed-Alarm Semantics), §3.2 (state directory layout). Depends on Units 01, 02, 03, and 04 all being complete — this is the integration layer, dispatched only after Layer 2 (Units 02/03/04) lands.

## Acceptance criteria
- [ ] `Daemon::start(state_dir: &Path) -> Result<(), SnuzeError>` performs, in order: (1) acquire an advisory lock on `<state_dir>/daemon.pid` (fail with `SnuzeError::DaemonUnavailable` if another live process already holds it — check the PID in the file is actually running, not just that the file exists, so a stale PID file from a crashed daemon doesn't permanently block restart); (2) open `AlarmStore` at `state_dir`; (3) list all `Pending` alarms and `arm` each into a fresh `Scheduler`, applying the grace-window rule from Unit 03 (alarms already overdue past the grace window become `Missed` here, at boot, which is the actual trigger point for that rule even though Unit 03 owns the policy decision); (4) bind the Unix domain socket (named pipe on Windows) at `<state_dir>/daemon.sock`; (5) begin serving requests and run the scheduler's blocking loop on a dedicated thread.
- [ ] `Daemon::stop(state_dir: &Path) -> Result<(), SnuzeError>` sends a graceful shutdown request over the socket to a running daemon (not a raw `kill` from the CLI side — the daemon should close its socket and release its lock cleanly on receiving a stop request).
- [ ] Lazy autostart: when `snuze set` (Unit 06) cannot connect to `daemon.sock`, it spawns the daemon as a detached background process (double-fork or platform equivalent so it survives the CLI process exiting) and retries the connection with a short bounded backoff (propose: up to 2 seconds total) before giving up with `SnuzeError::DaemonUnavailable`. This lazy-spawn path and the explicit `snuze daemon start` path must converge on the same `Daemon::start` — no second, divergent startup implementation.
- [ ] Socket protocol: newline-delimited JSON request/response, one request per line, matching the shape Herdr itself uses (SPEC §8) for consistency of mental model. Define a small internal request enum (`SetAlarm`, `ListAlarms`, `GetAlarm`, `CancelAlarm`, `Shutdown`, `Ping`) and corresponding response types — this protocol is internal to `snuze` (CLI ↔ its own daemon), not a public contract, so it can evolve freely as long as the CLI and daemon in the same build agree; do not version it prematurely.
- [ ] Every daemon-side mutation (`SetAlarm`, `CancelAlarm`, and terminal-state updates coming from the scheduler's `on_terminal` callback) goes through `AlarmStore::put` before the daemon responds `Ok` to the client — this is where SPEC §2's "durability-before-ack" promise is actually implemented; get this ordering right or the whole contract in the SPEC's problem statement is false advertising.
- [ ] `Ping` request/response exists specifically so `snuze doctor` (Unit 06) can check "is the daemon alive and responsive" without invoking any alarm logic.
- [ ] Unit tests: start a daemon against a temp state dir, connect a test client, `SetAlarm` then `ListAlarms` and confirm the round trip; start a daemon, kill the process (`SIGKILL`, not a graceful stop) with a pending alarm on the books, start a **new** daemon against the same state dir, confirm the alarm is re-armed at its original deadline (this is the fault-injection test called out in `SPEC.md` §9 — it belongs here structurally, since Unit 05 owns the boot-time rebuild); attempt to start a second daemon while a first is running against the same state dir and confirm it fails cleanly with `DaemonUnavailable` rather than corrupting the lock or the store; simulate a stale PID file (write a PID that does not correspond to a running process) and confirm a new daemon can still start.

## Interface contract
```rust
use std::path::Path;
use snuze_kernel::{Alarm, AlarmStatus, SnuzeError};

pub struct Daemon;

impl Daemon {
    /// Blocks the calling thread/process for the daemon's lifetime.
    /// The CLI's `snuze daemon start --foreground` calls this directly;
    /// the detached-background spawn path execs a small wrapper binary
    /// (or itself with an internal flag) that calls this in a child process.
    pub fn run_foreground(state_dir: &Path) -> Result<(), SnuzeError>;

    /// Client-side: send a graceful shutdown request to a running daemon.
    pub fn stop(state_dir: &Path) -> Result<(), SnuzeError>;

    /// Client-side: true if a daemon is listening and responds to Ping.
    pub fn is_running(state_dir: &Path) -> bool;

    /// Client-side: ensure a daemon is running, spawning one if not,
    /// with bounded retry. Used by `snuze set`'s lazy-autostart path.
    pub fn ensure_running(state_dir: &Path) -> Result<(), SnuzeError>;
}

/// The client-side handle Unit 06's CLI uses to talk to a running daemon.
pub struct DaemonClient { /* opaque, holds the socket connection */ }

impl DaemonClient {
    pub fn connect(state_dir: &Path) -> Result<Self, SnuzeError>;
    pub fn set_alarm(&self, alarm: Alarm) -> Result<(), SnuzeError>;
    pub fn list_alarms(&self, status_filter: Option<AlarmStatus>) -> Result<Vec<Alarm>, SnuzeError>;
    pub fn get_alarm(&self, id: &str) -> Result<Alarm, SnuzeError>;
    pub fn cancel_alarm(&self, id: &str) -> Result<bool, SnuzeError>;
    pub fn ping(&self) -> Result<(), SnuzeError>;
}
```

## Boundaries — do NOT touch
Do not redefine `Alarm`/`AlarmStatus`/`SnuzeError` (Unit 01), the store's on-disk format (Unit 02), the scheduler's heap/clock internals (Unit 03), or the Herdr shell-out logic (Unit 04) — this unit composes those four, it does not reimplement any of them. Do not implement `clap` argument parsing or `--json` output shaping (that's Unit 06); `DaemonClient`'s return types are plain Rust structs, not CLI output.

## Output
- `src/daemon/mod.rs`
- `src/daemon/server.rs`
- `src/daemon/client.rs`
- `src/daemon/protocol.rs`
- `src/daemon/lock.rs`
- `tests/test_daemon.rs`

## Verification
```bash
cargo test --test test_daemon
cargo clippy --all-targets -- -D warnings
```
