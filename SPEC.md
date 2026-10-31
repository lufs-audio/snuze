# `snuze` — Spec Index

This repo's specs live under [`docs/specs/`](docs/specs/), one timestamped folder per development phase, each self-contained (its own `SPEC.md` plus `units/`). This file is a pointer, not a spec — see the `speccing` skill for the folder/timestamp convention itself.

**Phases, oldest first:**

- [`docs/specs/2026-08-22T2145Z_initial-build-and-condition-mode/`](docs/specs/2026-08-22T2145Z_initial-build-and-condition-mode/) — the original scheduling core (deadline alarms, the Herdr delivery adapter, the daemon, the CLI) plus the Jules condition-mode feature (poll-based wake-up on Google Jules session state). 11 units, both landed and merged.
- [`docs/specs/2026-08-22T2150Z_signal-triggers-and-ledger-emission/`](docs/specs/2026-08-22T2150Z_signal-triggers-and-ledger-emission/) — **Phase 3.** Two new push-based trigger kinds (Herdr pane-status events via `events.subscribe`, and an arbitrary external `snuze signal <key>` call driven by things like a `webhookd` route), plus a local, drainable event outbox that emits every terminal alarm event — the piece `snuze` owns of the work-ledger idea. 4 units, landed and merged.
- [`docs/specs/2026-10-31T0045Z_generalized-triggers-and-control-planes/`](docs/specs/2026-10-31T0045Z_generalized-triggers-and-control-planes/) — **Phase 4.** Pluggable delivery targets (`tmux`, `herdr`, command exec, and blocking `snuze wait`), process-exit (`--pid`) and file-artifact (`--watch-file`) triggers to coordinate with tools like `lrex` and `tscribe`, and full deprecation/removal of Jules-specific kernel polling. 4 units.

For the "why" behind this system and where it sits in the broader All-Hands picture, see `lufs-audio/kb`'s `docs/ecosystem/all-hands/` suite (especially page 09) and `docs/strategy/house-of-process-registries/instantiation-all-hands.md` — narrative context lives there, not here, per this house's usual KB-vs-in-repo-contract split.
