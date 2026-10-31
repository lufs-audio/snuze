# snuze

> Asynchronous Agent Alarm Clock Primitive for Terminal Agents in Rust

`snuze` is a contract-verified LUFS Primitive CLI in Rust (`lufs-audio/snuze`). It provides an asynchronous alarm clock that allows an agent running in a terminal pane (Herdr or `tmux`) to schedule a future wake-up prompt and conclude its turn immediately without active polling loops.

`snuze` is 100% local-first: it contains no network code.

## Trigger Modes

1. **Fixed Duration:** Wake up after a duration (`--after 15m`).
2. **Process Exit:** Wake up as soon as a local process terminates (`--pid <PID>`).
3. **File Artifact:** Wake up when a contract file appears or is rewritten (`--watch-file take.json`).
4. **Herdr Status Push:** Wake up immediately when a watched Herdr pane reaches target lifecycle states (`--herdr-pane <ID> --watch-for done,idle --timeout 1h`).
5. **External Signal:** Wake up on arbitrary fleet signals/webhooks (`--signal <KEY> --timeout 1h`).

> **Migrating from Jules condition mode:** Jules-specific polling has been removed. External state changes — including anything that used to be a Jules session notification — now enter `snuze` through the generic `snuze signal <KEY>` mechanism, driven by whatever produced the event (a webhook, a CI job, a local script).

## Delivery Targets

Alarm delivery is pluggable. Pick a target explicitly, or let `snuze` infer one from the environment (`$HERDR_ENV=1` → Herdr; `$TMUX` → tmux):

| Target | Flag | Delivered with |
| --- | --- | --- |
| Herdr pane | `--pane <id>` / `--herdr <id>` | `herdr agent prompt` / `herdr pane run` |
| tmux pane | `--tmux <target>` | `tmux send-keys -t <target> "<text>" Enter` |
| Shell command | `--target command:<cmd>` | `sh -c <cmd>` |
| Blocking wait | `--target wait` | nothing — observed via `snuze wait` |

The explicit form is `--target herdr:<pane>`, `--target tmux:<target>`, `--target command:<cmd>`, or `--target wait`.

## Commands

```bash
# 1. Schedule alarms
snuze set --after 15m --pane w1:p3 --prompt "Wake up after 15m"
snuze set --after 30m --tmux %0 --prompt "Wake up in tmux"
snuze set --pid $(pgrep -f lrex) --prompt "Recording finished" --tmux %0
snuze set --watch-file take.json --prompt "Take artifact ready" --tmux %0
snuze set --watch-file 'transcript-*.md' --watch-mode modified --prompt "Transcript ready" --tmux %0
snuze set --herdr-pane w1:p2 --watch-for done,idle --timeout 30m --pane w1:p3 --prompt "Pane w1:p2 is done"
snuze set --signal ci-build-123 --timeout 2h --pane w1:p3 --prompt "CI build completed"

# 2. Block synchronously until an alarm is terminal
snuze wait <alarm-id> --timeout 1h --json

# 3. Trigger external signals
snuze signal ci-build-123 --json

# 4. List and inspect alarms
snuze list --json
snuze status <alarm-id> --json

# 5. Drain local event outbox (work ledger emission)
snuze events drain --json

# 6. Cancel alarms
snuze cancel <alarm-id> --json
snuze cancel --all --json

# 7. Daemon management & diagnostics
snuze daemon status --json
snuze daemon stop --json
snuze doctor --json
```

## Exit Codes

- `0`: Success
- `2`: Usage error (Invalid arguments, bad duration format, conflicting flags, unresolvable delivery target)
- `3`: Target unavailable (Herdr pane, watched pane, process, or alarm not found)
- `4`: Binary missing (`herdr` or `tmux` CLI not found on `PATH`)
- `5`: Contract violated (failed to persist an alarm/outbox file; `snuze wait` timed out or the alarm did not fire)
- `6`: Daemon unavailable (Daemon socket unreachable / failed to start, Herdr socket unreachable)

## Alarm File Compatibility

Alarms persist as one JSON file per alarm under `$XDG_STATE_HOME/snuze/alarms/`. Two older on-disk shapes are still read transparently:

- a bare `pane_id` string (pre-`DeliveryTarget`) is upgraded to `{"target":{"type":"herdr","pane_id":...}}`;
- a removed trigger kind (e.g. `jules_condition`) degrades to a plain `deadline` alarm rather than being discarded.

New records are always written in the current shape.

## Build & Test

```bash
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
```
