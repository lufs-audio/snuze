# snuze

> Asynchronous Agent Alarm Clock Primitive for Herdr Panes in Rust

`snuze` is a contract-verified LUFS Primitive CLI in Rust (`lufs-audio/snuze`). It provides a non-blocking asynchronous alarm clock that allows an agent running in a Herdr terminal pane to schedule a future wake-up prompt (`snuze set --after 15m --pane <id> --prompt "<text>"`) and conclude its turn immediately without active polling loops.

## Commands

```bash
# Set an alarm
snuze set --after 15m --pane w1:p3 --prompt "Jules finished. Pull sessions, reconcile, verify, and open PR."

# List alarms
snuze list --json

# Check alarm status
snuze status <alarm-id> --json

# Cancel alarm
snuze cancel <alarm-id> --json
snuze cancel --all --json

# Daemon management
snuze daemon status --json
snuze daemon stop --json

# Diagnostics
snuze doctor --json
```

## Exit Codes

- `0`: Success
- `2`: Usage error (Invalid arguments, bad duration format, conflicting flags)
- `3`: Target unavailable (Herdr pane not found, alarm ID not found)
- `4`: Binary missing (`herdr` CLI not found on `PATH`)
- `5`: Contract violated (Failed to persist alarm file)
- `6`: Daemon unavailable (Daemon socket unreachable / failed to start)

## Build & Test

```bash
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
```
