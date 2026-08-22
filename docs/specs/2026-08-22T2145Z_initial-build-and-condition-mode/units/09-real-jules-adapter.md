# Unit 09 — RealJulesAdapter (HTTP Implementation)

## Objective
Implement the real `JulesAdapter` (Unit 08) by calling the Jules v1alpha REST API directly over HTTPS, reading `JULES_API_KEY` from the local environment.

## Context
See `SPEC.md` §11.4. Depends on Unit 07 (`JulesSessionState`, `JulesState`, `SnuzeError`) and Unit 08's published `JulesAdapter` trait signature only.

**Canonical reference for the actual API shape: the `jules-api` Hyperagent skill's `jules.py` script (specifically its `status`/`watch` subcommands), not the official Jules docs directly.** The Ciani agent's own `jules-api` skill documents, from real verified use: base URL `https://jules.googleapis.com/v1alpha`, auth header `x-goog-api-key: <JULES_API_KEY>`, session states `QUEUED, PLANNING, AWAITING_PLAN_APPROVAL, IN_PROGRESS, COMPLETED, AWAITING_USER_FEEDBACK, PAUSED, FAILED`, and that a completed session's PR URL lives at `session.outputs[].pullRequest.url`. That same skill's notes explicitly warn: "the official docs contradict themselves" on resource-name construction, and this is an alpha API that should be re-verified rather than trusted from memory. `jules.py`'s `status`/`watch` subcommands are real, already-exercised-in-production code that resolves these ambiguities correctly — port their approach to the exact GET-by-session-ID endpoint and response parsing, rather than re-deriving it from the docs from scratch. If `jules.py` isn't directly available in this repo's build environment, its logic can be read from the Ciani Hyperagent skill of the same name; do not guess at the endpoint path and ship it unverified — that is exactly the failure mode `RealHerdrAdapter`'s own history in this repo (see Unit 04, and the `--wait` removal on `feature/jules-condition-mode`) warns against repeating.

This integration is **independent of and unrelated to** Ciani's cloud-side `jules-api` Hyperagent skill and its credential proxy — `snuze` is a local binary reading a local environment variable, calling the API directly. They happen to speak to the same external service; nothing else connects them.

## Acceptance criteria
- [ ] `RealJulesAdapter::new() -> Result<Self, SnuzeError>` reads `JULES_API_KEY` from the environment; returns `SnuzeError::JulesCredentialMissing` if absent or empty. Constructor-time check, matching `RealHerdrAdapter::new()`'s fail-fast pattern.
- [ ] Uses a minimal, **synchronous** HTTP client (e.g. `ureq`) — not `reqwest` or any client that pulls in an async runtime. `snuze` has exactly one architectural rule about concurrency (`SPEC.md` §3.1, §8: no `tokio`, one dedicated thread) and this unit must not be the one that quietly breaks it for the sake of an HTTP call.
- [ ] `check_sessions(&self, session_ids: &[String]) -> Result<Vec<JulesSessionState>, SnuzeError>` issues one GET request per session ID (batching into a single request is not supported by the API as currently understood; do not invent a batch endpoint) with a bounded per-request timeout (propose 10 seconds — generous relative to the 5-second local Herdr timeout, since this is a real network call to an external cloud service, not a local subprocess). All requests in the batch must complete (or the whole call fails) before returning — no partial results, per Unit 08's contract.
- [ ] Maps each session's JSON response into `JulesSessionState { session_id, state, pull_request_url }`. An HTTP 404 for a given session ID is a legitimate, non-error outcome only during the pre-flight existence check (`SPEC.md` §11.2) — for `check_sessions` calls made by the scheduler *after* an alarm is already armed, a 404 on a previously-valid session ID should surface as an error (`DaemonUnavailable`, reason mentioning the missing session), not be silently treated as some default state, since a session disappearing after being armed is itself noteworthy.
- [ ] Maps HTTP 401/403 (bad or revoked API key) to `SnuzeError::JulesCredentialMissing` (reusing the same variant/exit code as "key absent," since both mean "this adapter cannot authenticate," a distinction not worth a second exit code). Maps HTTP 429 to `SnuzeError::DaemonUnavailable("rate limited")`. Maps any other non-2xx or a response body that fails to parse into the expected shape to `SnuzeError::DaemonUnavailable` with a truncated (matching `RealHerdrAdapter`'s `MAX_REASON_LEN` convention) reason string — never panics on unexpected JSON shape, per this crate's zero-panic standard.
- [ ] An unrecognized `state` string in the response (this is an alpha API; new states may appear) deserializes to `JulesState::Unknown(String)` per Unit 07's contract, not a parse error.
- [ ] Unit tests use a local mock HTTP server (or a minimal hand-rolled one bound to `127.0.0.1` on an ephemeral port — do not make real network calls in `cargo test`) exercising: a successful multi-session response with mixed states; a 404 on a session; a 401; a 429; a response containing an unrecognized state string (asserts it becomes `Unknown`, not a failure); a request that never responds (asserts the timeout fires and maps to `DaemonUnavailable`, not an indefinite hang — this is directly analogous to `RealHerdrAdapter`'s own subprocess-timeout test and should not be skipped).

## Interface contract
```rust
use snuze_kernel::{JulesSessionState, SnuzeError};
use snuze_jules::JulesAdapter; // trait from Unit 08
use std::time::Duration;

pub struct RealJulesAdapter {
    api_key: String,
    base_url: String, // defaults to https://jules.googleapis.com/v1alpha; overridable for tests
    request_timeout: Duration,
}

impl RealJulesAdapter {
    /// # Errors
    /// Returns `SnuzeError::JulesCredentialMissing` if `JULES_API_KEY` is unset or empty.
    pub fn new() -> Result<Self, SnuzeError>;

    /// Test/advanced constructor allowing a custom base URL (for pointing at a local mock
    /// server in tests) and request timeout.
    pub fn with_base_url_and_timeout(
        api_key: String,
        base_url: String,
        request_timeout: Duration,
    ) -> Self;
}

impl JulesAdapter for RealJulesAdapter {
    fn check_sessions(&self, session_ids: &[String]) -> Result<Vec<JulesSessionState>, SnuzeError>;
}
```

## Boundaries — do NOT touch
Do not implement scheduling/polling-loop logic (`src/scheduler/`, beyond importing Unit 08's published trait) or CLI argument parsing (Unit 11). Do not modify Unit 07's types or Unit 08's trait signature — if the published interface is insufficient for a real HTTP response shape discovered during implementation, flag it back rather than changing it unilaterally, since Unit 10 may already be dispatched in parallel against the original signature.

## Output
- `src/jules/adapter.rs` (new)
- `tests/test_jules_adapter.rs` (new)
- `tests/fixtures/jules_mock_server.rs` or equivalent local-mock-server helper (new)

## Verification
```bash
cargo test --test test_jules_adapter
cargo clippy --all-targets -- -D warnings
```
