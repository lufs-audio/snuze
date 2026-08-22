use crate::kernel::{JulesCondition, SnuzeError};

/// Parses and validates `--jules`'s comma-separated value into individual session IDs.
///
/// # Errors
/// Returns `SnuzeError::ConflictingFlags` on an empty list or any empty entry (e.g. trailing comma).
pub fn parse_jules_session_ids(raw: &str) -> Result<Vec<String>, SnuzeError> {
    if raw.trim().is_empty() {
        return Err(SnuzeError::ConflictingFlags(
            "empty --jules session list".to_string(),
        ));
    }

    let raw_parts: Vec<&str> = raw.split(',').collect();
    let mut ids = Vec::with_capacity(raw_parts.len());

    for part in raw_parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            return Err(SnuzeError::ConflictingFlags(
                "empty session ID in --jules list".to_string(),
            ));
        }
        ids.push(trimmed.to_string());
    }

    if ids.is_empty() {
        return Err(SnuzeError::ConflictingFlags(
            "empty --jules session list".to_string(),
        ));
    }

    Ok(ids)
}

/// Parses `--condition`'s string value.
///
/// # Errors
/// Returns `SnuzeError::ConflictingFlags` on any value other than the three recognized ones,
/// listing the valid options in the error message.
pub fn parse_jules_condition(raw: &str) -> Result<JulesCondition, SnuzeError> {
    match raw {
        "all-terminal" => Ok(JulesCondition::AllTerminal),
        "any-terminal" => Ok(JulesCondition::AnyTerminal),
        "any-completed" => Ok(JulesCondition::AnyCompleted),
        _ => Err(SnuzeError::ConflictingFlags(format!(
            "unrecognized --condition '{raw}'; valid options are: all-terminal, any-terminal, any-completed"
        ))),
    }
}
