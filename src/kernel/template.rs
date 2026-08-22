use super::jules_state::JulesSessionState;

/// Closed allowlist of variables available for prompt template interpolation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateVars {
    pub duration: Option<String>,
    pub session_id: Option<String>,
    pub status: Option<String>,
    pub pr_urls: Option<String>,
}

/// Interpolates `{duration}`, `{session_id}`, `{status}`, and `{pr_urls}` tokens in the template.
///
/// Unrecognized tokens and variables with `None` value are left untouched.
#[must_use]
pub fn interpolate(template: &str, vars: &TemplateVars) -> String {
    let mut result = template.to_string();

    if let Some(ref duration) = vars.duration {
        result = result.replace("{duration}", duration);
    }
    if let Some(ref session_id) = vars.session_id {
        result = result.replace("{session_id}", session_id);
    }
    if let Some(ref status) = vars.status {
        result = result.replace("{status}", status);
    }
    if let Some(ref pr_urls) = vars.pr_urls {
        result = result.replace("{pr_urls}", pr_urls);
    }

    result
}

/// Formats template variables (`session_id`, `status`, `pr_urls`) from observed session states.
#[must_use]
pub fn format_jules_template_vars(states: &[JulesSessionState]) -> (String, String, String) {
    if states.is_empty() {
        return (String::new(), String::new(), String::new());
    }

    if states.len() == 1 {
        if let Some(state_entry) = states.first() {
            let session_id = state_entry.session_id.clone();
            let status_name = state_entry.state.as_str().to_string();
            let pr_urls = state_entry.pull_request_url.clone().unwrap_or_default();
            return (session_id, status_name, pr_urls);
        }
    }

    let session_ids: String = states
        .iter()
        .map(|s| s.session_id.as_str())
        .collect::<Vec<_>>()
        .join(",");

    let status_summary: String = states
        .iter()
        .map(|s| format!("{}:{}", s.session_id, s.state.as_str()))
        .collect::<Vec<_>>()
        .join(",");

    let pr_urls: String = states
        .iter()
        .filter_map(|s| {
            s.pull_request_url
                .as_ref()
                .filter(|url| !url.trim().is_empty())
        })
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(",");

    (session_ids, status_summary, pr_urls)
}
