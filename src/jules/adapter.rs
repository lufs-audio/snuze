use super::mock::JulesAdapter;
use crate::kernel::{JulesSessionState, JulesState, SnuzeError};
use serde::Deserialize;
use std::time::Duration;

const MAX_REASON_LEN: usize = 256;

/// Production adapter that queries the Google Jules v1alpha REST API.
pub struct RealJulesAdapter {
    api_key: String,
    base_url: String,
    request_timeout: Duration,
}

#[derive(Deserialize)]
struct JulesSessionResponse {
    state: Option<JulesState>,
    outputs: Option<Vec<JulesOutputResponse>>,
}

#[derive(Deserialize)]
struct JulesOutputResponse {
    #[serde(rename = "pullRequest")]
    pull_request: Option<JulesPullRequestResponse>,
}

#[derive(Deserialize)]
struct JulesPullRequestResponse {
    url: Option<String>,
}

impl RealJulesAdapter {
    /// Creates a new `RealJulesAdapter` using `JULES_API_KEY` from the environment.
    ///
    /// # Errors
    /// Returns `SnuzeError::JulesCredentialMissing` if `JULES_API_KEY` is not set or empty.
    pub fn new() -> Result<Self, SnuzeError> {
        let api_key =
            std::env::var("JULES_API_KEY").map_err(|_| SnuzeError::JulesCredentialMissing)?;
        if api_key.trim().is_empty() {
            return Err(SnuzeError::JulesCredentialMissing);
        }
        let base_url = std::env::var("JULES_BASE_URL")
            .unwrap_or_else(|_| "https://jules.googleapis.com/v1alpha".to_string());
        Ok(Self {
            api_key,
            base_url,
            request_timeout: Duration::from_secs(10),
        })
    }

    /// Test and advanced constructor allowing custom base URL and request timeout.
    #[must_use]
    pub fn with_base_url_and_timeout(
        api_key: String,
        base_url: String,
        request_timeout: Duration,
    ) -> Self {
        Self {
            api_key,
            base_url,
            request_timeout,
        }
    }
}

impl JulesAdapter for RealJulesAdapter {
    fn check_sessions(&self, session_ids: &[String]) -> Result<Vec<JulesSessionState>, SnuzeError> {
        let mut results = Vec::with_capacity(session_ids.len());
        let agent = ureq::AgentBuilder::new()
            .timeout(self.request_timeout)
            .build();

        for session_id in session_ids {
            let url = format!(
                "{}/sessions/{}",
                self.base_url.trim_end_matches('/'),
                session_id
            );
            let resp_result = agent.get(&url).set("x-goog-api-key", &self.api_key).call();

            match resp_result {
                Ok(resp) => {
                    let body: JulesSessionResponse = resp.into_json().map_err(|e| {
                        SnuzeError::DaemonUnavailable(format!("malformed Jules response: {e}"))
                    })?;
                    let state = body
                        .state
                        .unwrap_or(JulesState::Unknown("MISSING".to_string()));
                    let pull_request_url = body.outputs.and_then(|outputs| {
                        outputs
                            .into_iter()
                            .find_map(|o| o.pull_request.and_then(|pr| pr.url))
                    });
                    results.push(JulesSessionState {
                        session_id: session_id.clone(),
                        state,
                        pull_request_url,
                    });
                }
                Err(ureq::Error::Status(401 | 403, _)) => {
                    return Err(SnuzeError::JulesCredentialMissing);
                }
                Err(ureq::Error::Status(404, _)) => {
                    return Err(SnuzeError::JulesSessionNotFound(session_id.clone()));
                }
                Err(ureq::Error::Status(429, _)) => {
                    return Err(SnuzeError::DaemonUnavailable("rate limited".to_string()));
                }
                Err(ureq::Error::Status(code, resp)) => {
                    let msg = resp.into_string().unwrap_or_default();
                    let truncated = if msg.len() > MAX_REASON_LEN {
                        match msg.char_indices().nth(MAX_REASON_LEN) {
                            Some((idx, _)) => &msg[..idx],
                            None => &msg,
                        }
                    } else {
                        &msg
                    };
                    return Err(SnuzeError::DaemonUnavailable(format!(
                        "Jules API returned status {code}: {truncated}"
                    )));
                }
                Err(ureq::Error::Transport(e)) => {
                    return Err(SnuzeError::DaemonUnavailable(format!(
                        "Jules API transport error: {e}"
                    )));
                }
            }
        }

        Ok(results)
    }
}
