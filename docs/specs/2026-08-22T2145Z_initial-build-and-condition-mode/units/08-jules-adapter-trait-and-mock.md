# Unit 08 — JulesAdapter Trait and Mock

## Objective
Publish the `JulesAdapter` trait — the interface boundary between the scheduler and the real Jules HTTP API — plus a fully-featured test mock. This unit's trait signature is what Units 09 and 10 both build against in parallel; treat it as frozen once this unit lands, exactly as `HerdrAdapter`'s signature was treated in the original Unit 03.

## Context
See `SPEC.md` §11.4. Depends on Unit 07's `JulesSessionState`, `JulesState`, `SnuzeError`. Reference pattern: `src/scheduler/mod.rs`'s existing `HerdrAdapter` trait + the mock adapters used in `tests/test_scheduler.rs` — same shape, applied to a query-only (not delivery) interface.

## Acceptance criteria
- [ ] Define `JulesAdapter` trait with one method, `check_sessions`, taking a session ID slice and returning `Result<Vec<JulesSessionState>, SnuzeError>` for the *whole batch* — a single auth/network failure fails the entire call rather than returning partial results, because a poll that can't see every tracked session can't safely evaluate `AllTerminal`.
- [ ] Implement `MockJulesAdapter` (test-only, but not gated behind `#[cfg(test)]` in this crate the way `FakeClock` is — Unit 10 needs to import it from its own test files in a different module, so it must be a normal `pub` item, e.g. behind a `testing` Cargo feature or simply public with a doc comment marking it test-only, matching whichever convention `FakeClock` actually used in Unit 03; do not invent a second convention). `MockJulesAdapter` must support, via a builder or constructor arguments: returning a fixed, configurable `Vec<JulesSessionState>` for any call; returning `Err(SnuzeError::DaemonUnavailable(..))` to simulate an API failure; and recording every `check_sessions` call it received (session IDs + call count) so a test can assert *how many times* polling happened, not just the final outcome — this is what lets Unit 10's tests prove re-arm/poll-interval timing without a real Jules API.
- [ ] `MockJulesAdapter` must be swappable mid-test (e.g. `set_response(states)` or equivalent) so a single test can simulate a session transitioning from `InProgress` on the first poll to `Completed` on a later one — this is the realistic shape of what Unit 10 needs to test the re-arm loop.
- [ ] Unit tests for this unit itself: `MockJulesAdapter` returns exactly what it was configured to return; call count increments correctly across multiple calls; `set_response` correctly changes subsequent call results without affecting already-recorded call history.

## Interface contract
```rust
use snuze_kernel::{JulesSessionState, SnuzeError};
use std::sync::Mutex;

pub trait JulesAdapter: Send + Sync {
    /// Query current state for each listed session ID in one batch call.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` (reused: "the remote service this
    /// adapter depends on is unreachable") on any network, auth, or malformed-response
    /// failure. Never returns a partial `Vec` — the whole batch succeeds or the whole
    /// call fails, since `AllTerminal` cannot be safely evaluated from partial data.
    fn check_sessions(&self, session_ids: &[String]) -> Result<Vec<JulesSessionState>, SnuzeError>;
}

/// Test double recording every call it receives.
pub struct MockJulesAdapter {
    response: Mutex<Result<Vec<JulesSessionState>, String>>, // String stand-in for a cloneable error reason
    calls: Mutex<Vec<Vec<String>>>, // one entry per call, the session_ids it was asked about
}

impl MockJulesAdapter {
    #[must_use]
    pub fn new(initial_response: Vec<JulesSessionState>) -> Self;

    /// Replace the response returned by subsequent calls without touching call history.
    pub fn set_response(&self, response: Vec<JulesSessionState>);

    /// Make subsequent calls fail with `SnuzeError::DaemonUnavailable(reason)`.
    pub fn set_failure(&self, reason: &str);

    /// Number of times `check_sessions` has been called so far.
    #[must_use]
    pub fn call_count(&self) -> usize;

    /// The session ID list passed on the Nth call (0-indexed), if it happened.
    #[must_use]
    pub fn call_args(&self, n: usize) -> Option<Vec<String>>;
}

impl JulesAdapter for MockJulesAdapter {
    fn check_sessions(&self, session_ids: &[String]) -> Result<Vec<JulesSessionState>, SnuzeError>;
}
```

## Boundaries — do NOT touch
Do not implement any real HTTP client, `JULES_API_KEY` reading, or network code — that is entirely Unit 09's job, built against the trait above without needing this unit's own code to exist first (the signature here is the contract). Do not touch `src/scheduler/mod.rs`'s `run()`/`arm()` (Unit 10) or any CLI file (Unit 11).

## Output
- `src/jules/mod.rs` (new)
- `src/jules/mock.rs` (new)
- `tests/test_jules_mock.rs` (new)

## Verification
```bash
cargo test --test test_jules_mock
cargo clippy --all-targets -- -D warnings
```
