# Unit 01 — pluggable-delivery-and-tmux-adapter

## Objective
Generalize alarm delivery by replacing the hardcoded `pane_id` with a pluggable `DeliveryTarget` enum and adding a native `TmuxAdapter` alongside `HerdrAdapter`.

## Context
- `src/kernel/types.rs` currently defines `pane_id: String` on `Alarm`.
- `src/herdr/adapter.rs` is currently the only delivery mechanism (`RealHerdrAdapter`).
- Terminal-native agents frequently run inside `tmux` rather than `herdr`.
- See phase spec: `docs/specs/2026-10-31T0045Z_generalized-triggers-and-control-planes/SPEC.md`.

## Acceptance criteria
- [ ] `Alarm` struct supports `DeliveryTarget` with variants: `Herdr { pane_id }`, `Tmux { target }`, `Command { command }`, and `Wait`.
- [ ] Implement `TmuxAdapter` invoking `tmux send-keys -t <target> "<prompt>" Enter` for prompts and commands.
- [ ] CLI flags allow explicit `--target tmux:<id>`, `--tmux <id>`, `--pane <id>` (Herdr), or `--herdr <id>`.
- [ ] If no target flag is supplied:
  - If `$HERDR_ENV=1`, defaults to `DeliveryTarget::Herdr` targeting current `$HERDR_PANE_ID`.
  - If `$TMUX` is set, defaults to `DeliveryTarget::Tmux` targeting current active tmux pane.
  - If neither is set, requires explicit `--target` or exits with `USAGE` (code 2).
- [ ] Backwards-compatible serde handles existing serialized alarms on disk.

## Interface contract
```rust
// In src/kernel/types.rs

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeliveryTarget {
    Herdr { pane_id: String },
    Tmux { target: String },
    Command { command: String },
    Wait,
}

// In src/scheduler/adapter.rs

pub trait DeliveryAdapter: Send + Sync {
    fn deliver(&self, target: &DeliveryTarget, delivery: &DeliveryMode) -> Result<DeliveryOutcome, SnuzeError>;
}
```

## Boundaries
- **Touches:**
  - `src/kernel/types.rs`
  - `src/scheduler/mod.rs`
  - `src/herdr/adapter.rs`
  - `src/tmux/` (new module for `TmuxAdapter`)
  - `src/cli/`
  - `tests/`
- **Does NOT touch:**
  - `src/store/` (on-disk store interface remains generic)
  - `src/daemon/` (uses scheduler trait abstraction)

## Verification
```bash
cargo test --test test_tmux_delivery
```
