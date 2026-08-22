# Agent Integration & Architecture Guide — `snuze`

> Operational guidelines, architectural contracts, and verification commands for AI coding agents.

---

## 1. Project Purpose & Architecture

`snuze` is the asynchronous agent alarm clock primitive for the LUFS ecosystem, authored by Daniel Ramirez <dan@lufs.org>.

### Core Rules for Agents
1. **Contract-First:** Never break public interfaces without updating contracts and tests.
2. **Deterministic Outputs:** CLI commands support `--json` output with standard `{ "status": "success", "data": ... }` and `{ "status": "error", "code": <exit_code>, "message": ... }` envelopes.
3. **Verification Before Hand-off:** Run the 3-tier verification pipeline (`cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`) before declaring any unit complete.
4. **Zero Panics:** Library and kernel code must not panic or unwrap in production paths; use monadic error handling (`?`, `ok_or_else`, `map_err`).

---

## 2. Essential Commands

```bash
# Fast check
cargo check --message-format=json

# Clippy lints
cargo clippy --all-targets -- -D warnings

# Unit & integration tests
cargo test
```

---

## 3. Architecture & Unit Boundaries

- `src/kernel/`: Domain types (`Alarm`, `AlarmStatus`, `DeliveryMode`), duration parsing, `SnuzeError`, `ExitCode` mapping.
- `src/store/`: Crash-safe disk persistence for alarms (`$XDG_STATE_HOME/snuze/alarms/<id>.json`).
- `src/scheduler/`: Min-heap priority queue and condition-variable sleeping thread; publishes `HerdrAdapter` trait and `Clock` abstraction.
- `src/herdr/`: Real `HerdrAdapter` implementation shelling out to `herdr agent prompt` and `herdr pane run`.
- `src/daemon/`: Background daemon process, PID lock, Unix domain socket server, lazy autostart, crash-recovery re-arm.
- `src/cli/`: Clap CLI parser, subcommand dispatch, `--json` formatting, `src/main.rs`.
