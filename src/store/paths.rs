use std::env;
use std::path::PathBuf;

/// Single source of truth for the state directory; called by the daemon (Unit 05)
/// and the CLI (Unit 06). Do not reimplement this resolution elsewhere.
///
/// Resolves to `$XDG_STATE_HOME/snuze` if `XDG_STATE_HOME` is set and non-empty,
/// otherwise falls back to `~/.local/state/snuze`.
#[must_use]
pub fn resolve_state_dir() -> PathBuf {
    if let Ok(val) = env::var("XDG_STATE_HOME") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("snuze");
        }
    }

    if let Some(home) = dirs::home_dir() {
        return home.join(".local").join("state").join("snuze");
    }

    if let Ok(home) = env::var("HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed)
                .join(".local")
                .join("state")
                .join("snuze");
        }
    }

    PathBuf::from(".local").join("state").join("snuze")
}
