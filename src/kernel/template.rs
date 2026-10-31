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
