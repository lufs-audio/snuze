# Unit 07 — Jules Kernel Extension

## Objective
Extend the kernel with the `Trigger` type, Jules session-state types, condition-satisfaction logic, and template interpolation needed for condition-driven wake-ups. Every downstream Jules unit (08–11) depends on this one; it depends only on Unit 01's existing types.

## Context
See `SPEC.md` §11.1 (Trigger generalization), §11.2 (condition semantics), §11.3 (template interpolation). Existing reference: `src/kernel/types.rs` (the `Alarm` struct this unit extends — add fields, do not remove or rename any existing field or change its type) and `src/kernel/errors.rs` (the `SnuzeError`/`ExitCode` this unit adds one new error variant to, reusing an existing exit code).

## Acceptance criteria
- [ ] Add `Trigger` enum (`Deadline`, `JulesCondition{session_ids, condition, poll_interval_secs, timeout_at, last_known}`) with `#[serde(tag = "kind", rename_all = "snake_case")]` and a `Default` impl returning `Trigger::Deadline`.
- [ ] Add `Alarm.trigger: Trigger` with `#[serde(default)]` and `Alarm.schema_version: u32` with `#[serde(default)]` returning `1`. Do not change the type or meaning-breaking behavior of any existing `Alarm` field; `deadline`'s doc comment must be updated to describe its dual meaning (exact fire time for `Deadline`, next-check-time for `JulesCondition`) but its type (`SystemTime`) is unchanged.
- [ ] Add `JulesState` enum: `Queued, Planning, AwaitingPlanApproval, InProgress, Completed, AwaitingUserFeedback, Paused, Failed, Unknown(String)`. Deserialize any state string not matching a named variant into `Unknown(String)` rather than failing — this is an alpha external API; unrecognized values must never cause a panic or a deserialization error.
- [ ] Add `JulesSessionState { session_id: String, state: JulesState, pull_request_url: Option<String> }`.
- [ ] Add `JulesCondition` enum: `AllTerminal, AnyTerminal, AnyCompleted` with `#[serde(rename_all = "kebab-case")]` (so CLI strings `all-terminal`/`any-terminal`/`any-completed` map directly).
- [ ] Implement `JulesCondition::is_satisfied(&self, states: &[JulesSessionState]) -> bool` exactly per `SPEC.md` §11.2: define a private `fn is_wake_worthy(state: &JulesState) -> bool` returning true for `Completed | Failed | AwaitingUserFeedback | Paused` only, then: `AllTerminal` = every state in the slice is wake-worthy; `AnyTerminal` = at least one is; `AnyCompleted` = at least one is exactly `JulesState::Completed`. An empty `states` slice must return `false` for all three variants (never vacuously true — a poll that returned zero results is not a satisfied condition, it is at best inconclusive).
- [ ] Add `SnuzeError::JulesCredentialMissing` variant; extend the existing `From<&SnuzeError> for ExitCode` match arm so it maps to `ExitCode::BinaryMissing` (reuse, not a new code) alongside the existing `HerdrBinaryMissing` arm.
- [ ] Implement template interpolation: `pub fn interpolate(template: &str, vars: &TemplateVars) -> String` where `TemplateVars` is a plain struct (not a map) with exactly the fields `duration: Option<String>`, `session_id: Option<String>`, `status: Option<String>`, `pr_urls: Option<String>`. Substitutes `{duration}`, `{session_id}`, `{status}`, `{pr_urls}` when the corresponding field is `Some`; leaves the literal `{name}` text untouched when the field is `None` or the token isn't one of these four names. This function must never accept or expose a generic key-value map — the closed set of fields on `TemplateVars` is the whole security boundary described in `SPEC.md` §11.3, and it only holds if nothing ever routes arbitrary strings (especially environment content) into it.
- [ ] Unit tests: `is_satisfied` for all three conditions across representative state combinations, explicitly including the boundary case in `SPEC.md` §11.8 (one `Failed` + one `InProgress` satisfies `AnyTerminal` but not `AllTerminal` or `AnyCompleted`) and the empty-slice case; `interpolate` substitutes all four known variables correctly, leaves an unknown `{foo}` token untouched, and leaves a known-but-`None` variable's token untouched (not blanked); `Alarm` with `schema_version` and `trigger` omitted from the input JSON (simulating an old on-disk record) deserializes to `schema_version: 1, trigger: Trigger::Deadline`; a `Trigger::JulesCondition` value round-trips through `serde_json` losslessly including `last_known`.

## Interface contract
```rust
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum JulesState {
    Queued, Planning, AwaitingPlanApproval, InProgress,
    Completed, AwaitingUserFeedback, Paused, Failed,
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
    pub fn is_satisfied(&self, states: &[JulesSessionState]) -> bool;
}

#[derive(Debug, Clone, Default)]
pub struct TemplateVars {
    pub duration: Option<String>,
    pub session_id: Option<String>,
    pub status: Option<String>,
    pub pr_urls: Option<String>,
}

#[must_use]
pub fn interpolate(template: &str, vars: &TemplateVars) -> String;

// Additive to the existing Alarm struct (Unit 01) -- do not restate the whole
// struct here; add exactly these two fields with #[serde(default)]:
//   pub trigger: Trigger,
//   pub schema_version: u32,   // default 1

// Additive to the existing SnuzeError enum (Unit 01) -- add exactly this variant:
//   #[error("JULES_API_KEY not set in environment")]
//   JulesCredentialMissing,
// and extend the existing ExitCode mapping match with:
//   SnuzeError::JulesCredentialMissing => ExitCode::BinaryMissing,
```

## Boundaries — do NOT touch
Do not implement the `JulesAdapter` trait or any HTTP/network code (Units 08/09). Do not touch the scheduler's `run()`/`arm()` logic (Unit 10) or any CLI file (Unit 11). Do not modify any existing `Alarm` field's type, name, or serialization tag — this unit is purely additive to Unit 01's types.

## Output
- `src/kernel/trigger.rs` (new: `Trigger`, `JulesCondition`, `is_satisfied`)
- `src/kernel/jules_state.rs` (new: `JulesState`, `JulesSessionState`)
- `src/kernel/template.rs` (new: `TemplateVars`, `interpolate`)
- `src/kernel/types.rs` (modified: add `trigger`/`schema_version` fields to `Alarm`)
- `src/kernel/errors.rs` (modified: add `JulesCredentialMissing` variant + exit code mapping)
- `src/kernel/mod.rs` (modified: export new types)
- `tests/test_kernel_jules.rs` (new)

## Verification
```bash
cargo test --test test_kernel --test test_kernel_jules
cargo clippy --all-targets -- -D warnings
```
