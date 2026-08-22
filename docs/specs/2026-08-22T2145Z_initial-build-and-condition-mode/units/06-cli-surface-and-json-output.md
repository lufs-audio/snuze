# Unit 06 — CLI Surface and JSON Output

## Objective
Implement the `snuze` binary's argument parsing, subcommand dispatch, human-readable and `--json` output, and exit code mapping, wired to a running daemon via Unit 05's `DaemonClient`.

## Context
See `SPEC.md` §7 (CLI Command Contract) for the full command surface and §5 for exit codes. Depends on Unit 01 (types, exit codes) and Unit 05 (`DaemonClient`, `Daemon::ensure_running`). This is the top-level integration and the only unit that owns `src/main.rs`.

## Acceptance criteria
- [ ] `clap` derive-based CLI matching `SPEC.md` §7 exactly: `set`, `list`, `status`, `cancel`, `daemon {start,stop,status,restart}`, `doctor`, and `--version`.
- [ ] `snuze set --after DURATION --pane PANE_ID (--prompt TEXT | --command CMD) [--label TEXT] [--dry-run] [--json]`: validates `--prompt`/`--command` are mutually exclusive and exactly one is present (exit `2` otherwise, before any daemon contact); parses `--after` via Unit 01's `parse_duration` (exit `2` on failure); reads `$HERDR_PANE_ID` from the process environment (absent if not running inside a Herdr-managed pane — that's a valid, non-error state, not a failure) and sets it as the new `Alarm`'s `created_by_pane`; calls `RealHerdrAdapter::pane_exists` (from Unit 04 — a direct `herdr` shell-out, independent of the `snuze` daemon) to fail fast with exit `3` if the pane is gone *before* arming anything. **If `--dry-run` is set, stop here**: print the fully-constructed `Alarm` (human or `--json`) exactly as it *would* be persisted, exit `0`, and do not call `Daemon::ensure_running` or `DaemonClient::set_alarm` at all — a dry run must not lazily spawn a daemon as a side effect; the ordering above (pane-existence check before the dry-run early-return, daemon contact after it) is deliberate and load-bearing, not incidental. Otherwise, call `Daemon::ensure_running` (lazy autostart — exit `6` if this fails), then `DaemonClient::set_alarm`; exit `5` if that returns `PersistFailed`. On success, prints the alarm ID (plain text) or the full `Alarm` (JSON, with `--json`).
- [ ] `snuze list [--status STATUS] [--json]`: calls `DaemonClient::list_alarms`; human output is a table (ID, pane, status, deadline, label); `--json` output is a JSON array of full `Alarm` objects, sorted by deadline ascending (matching Unit 02's store ordering).
- [ ] `snuze status ALARM_ID [--json]`: `DaemonClient::get_alarm`; exit `3` (`TargetUnavailable`) if not found, matching the "alarm ID not found" case in `SPEC.md` §5's exit code table.
- [ ] `snuze cancel (ALARM_ID | --all) [--json]`: `--all` cancels every currently-`Pending` alarm and reports a count; a single `ALARM_ID` cancels one and reports whether it was actually pending (canceling an already-fired or already-canceled alarm is not an error — it's a no-op that reports `false`/`already_terminal`, not exit code `3`, since the alarm *does* exist, it's just not cancelable anymore).
- [ ] `snuze daemon start|stop|status|restart [--json]`: thin wrapper over `Daemon::run_foreground`/`Daemon::stop`/`Daemon::is_running`. `start` without a foreground flag spawns detached (same code path as lazy autostart, per Unit 05's "no second divergent startup implementation" requirement — verify this by checking both call the same `Daemon` function, not just that they produce similar-looking behavior).
- [ ] `snuze doctor [--json]`: reports, per `SPEC.md` §7 — `herdr` binary present + version (or `BinaryMissing`); Herdr socket reachable (`herdr api schema` succeeds); `snuze` daemon running + responsive to `Ping`; state directory exists and is writable; counts of alarms by status, with `failed` and `missed` counts called out prominently in human output (these are the two statuses that mean "a wake-up silently didn't happen" — `doctor` exists specifically so a human or agent can catch that without having to know to ask). Exit `0` if all checks pass, non-zero (map to the most specific relevant code — e.g. `4` if `herdr` is missing, `6` if the daemon is unreachable) if any check fails, even though `doctor` is a read-only diagnostic — a failing health check should be visible in the exit code for scripting, not just in the printed output.
- [ ] Every subcommand's `--json` output is valid, parseable JSON on both success and failure paths, using `bplate`'s confirmed envelope shape (`SPEC.md` §7) — NOT the shape this unit's first draft specified, which was incompatible and has been corrected: `{"status": "success", "data": {...}}` on success, `{"status": "error", "code": <numeric exit code>, "message": "..."}` on failure. Never a plain-text panic message mixed into stdout — an agent parsing `--json` output must never receive a mix of JSON and human text on the same stream.
- [ ] Unit tests: argument-parsing tests for every subcommand's valid and invalid flag combinations (via `clap`'s testable `Command` construction, not by spawning the real binary) confirming the correct exit code is chosen *before* any daemon/network call would occur for usage errors; an integration test that runs the built binary against a real (test-temp-dir) daemon end-to-end for `set` → `list` → `status` → `cancel`, asserting both human and `--json` output shapes.

## Interface contract
```rust
// src/main.rs — thin: parse args, dispatch, map Result<_, SnuzeError> to
// ExitCode via the From impl from Unit 01, print, std::process::exit.

#[derive(clap::Parser)]
#[command(name = "snuze", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    Set(SetArgs),
    List(ListArgs),
    Status(StatusArgs),
    Cancel(CancelArgs),
    Daemon(DaemonArgs),
    Doctor(DoctorArgs),
}

#[derive(clap::Args)]
pub struct SetArgs {
    #[arg(long)]
    pub after: String,
    #[arg(long)]
    pub pane: String,
    #[arg(long)]
    pub prompt: Option<String>,
    #[arg(long)]
    pub command: Option<String>,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

// ListArgs, StatusArgs, CancelArgs, DaemonArgs, DoctorArgs: analogous,
// matching SPEC.md §7's flags exactly.

/// Every subcommand handler returns this; main() maps Ok to the
/// {"status":"success","data":...} envelope and Err to
/// {"status":"error","code":<exit code>,"message":...} — see SPEC.md §7.
/// The numeric code embedded in the JSON error body must always equal the
/// process's actual exit status; these are never allowed to diverge.
pub type CliResult = Result<serde_json::Value, snuze_kernel::SnuzeError>;
```

## Boundaries — do NOT touch
Do not implement daemon internals (`src/daemon/`), scheduling (`src/scheduler/`), storage (`src/store/`), or Herdr shell-out logic (`src/herdr/`) beyond calling `RealHerdrAdapter::pane_exists` for the pre-flight check in `set`. This unit is glue: parse, validate, call, format, exit.

## Output
- `src/main.rs`
- `src/cli/mod.rs`
- `src/cli/args.rs`
- `src/cli/output.rs`
- `tests/test_cli_args.rs`
- `tests/test_cli_integration.rs`

## Verification
```bash
cargo build --release
cargo test --test test_cli_args --test test_cli_integration
cargo clippy --all-targets -- -D warnings
```
