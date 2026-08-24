use crate::kernel::{HerdrWatchStatus, SnuzeError};

/// Parses `--watch-for`'s comma-separated value.
///
/// # Errors
/// Returns `SnuzeError::ConflictingFlags` on an empty list, an empty entry,
/// or any value other than idle/done/blocked/working.
pub fn parse_watch_statuses(raw: &str) -> Result<Vec<HerdrWatchStatus>, SnuzeError> {
    if raw.trim().is_empty() {
        return Err(SnuzeError::ConflictingFlags(
            "empty --watch-for list".to_string(),
        ));
    }

    let raw_parts: Vec<&str> = raw.split(',').collect();
    let mut statuses = Vec::with_capacity(raw_parts.len());

    for part in raw_parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            return Err(SnuzeError::ConflictingFlags(
                "empty status in --watch-for list".to_string(),
            ));
        }
        let status = match trimmed {
            "idle" => HerdrWatchStatus::Idle,
            "done" => HerdrWatchStatus::Done,
            "blocked" => HerdrWatchStatus::Blocked,
            "working" => HerdrWatchStatus::Working,
            _ => {
                return Err(SnuzeError::ConflictingFlags(format!(
                    "unrecognized --watch-for status '{trimmed}'; valid options are: idle, done, blocked, working"
                )))
            }
        };
        statuses.push(status);
    }

    if statuses.is_empty() {
        return Err(SnuzeError::ConflictingFlags(
            "empty --watch-for list".to_string(),
        ));
    }

    Ok(statuses)
}
