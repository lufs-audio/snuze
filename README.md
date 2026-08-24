# snuze

> Asynchronous Agent Alarm Clock Primitive for Herdr Panes in Rust

`snuze` is a contract-verified LUFS Primitive CLI in Rust (`lufs-audio/snuze`). It provides an asynchronous alarm clock that allows an agent running in a Herdr terminal pane to schedule a future wake-up prompt and conclude its turn immediately without active polling loops.

## Trigger Modes

1. **Fixed Duration:** Wake up after a duration (`--after 15m`).
2. **Jules Condition Mode:** Wake up when Google Jules sessions satisfy a condition (`--jules <IDs> --condition any-completed --timeout 1h`).
3. **Herdr Status Push:** Wake up immediately when a watched Herdr pane reaches target lifecycle states (`--herdr-pane <ID> --watch-for done,idle --timeout 1h`).
4. **External Signal:** Wake up on arbitrary fleet signals/webhooks (`--signal <KEY> --timeout 1h`).

## Commands

```bash
# 1. Schedule alarms
snuze set --after 15m --pane w1:p3 --prompt "Wake up after 15m"
snuze set --jules <session_1>,<session_2> --condition any-completed --timeout 1h --pane w1:p3 --prompt "Jules finished ({pr_urls})"
snuze set --herdr-pane w1:p2 --watch-for done,idle --timeout 30m --pane w1:p3 --prompt "Pane w1:p2 is done"
snuze set --signal ci-build-123 --timeout 2h --pane w1:p3 --prompt "CI build completed"

# 2. Trigger external signals
snuze signal ci-build-123 --json

# 3. List and inspect alarms
snuze list --json
snuze status <alarm-id> --json

# 4. Drain local event outbox (work ledger emission)
snuze events drain --json

# 5. Cancel alarms
snuze cancel <alarm-id> --json
snuze cancel --all --json

# 6. Daemon management & diagnostics
snuze daemon status --json
snuze daemon stop --json
snuze doctor --json
```

## Exit Codes

- `0`: Success
- `2`: Usage error (Invalid arguments, bad duration format, conflicting flags)
- `3`: Target unavailable (Herdr pane, watched pane, Jules session, or alarm not found)
- `4`: Binary or credential missing (`herdr` CLI not found on `PATH`, `JULES_API_KEY` missing)
- `5`: Contract violated (Failed to persist alarm or outbox file)
- `6`: Daemon unavailable (Daemon socket unreachable / failed to start, Herdr socket unreachable)

## Build & Test

```bash
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
```
