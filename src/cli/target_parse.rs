use crate::kernel::{DeliveryTarget, SnuzeError};

/// Parses a `--target <kind>:<value>` specification.
///
/// Accepted forms:
/// - `herdr:<pane_id>` (alias `pane:<pane_id>`)
/// - `tmux:<target>`
/// - `command:<shell command>`
/// - `wait` (no value)
///
/// # Errors
/// Returns `SnuzeError::ConflictingFlags` for an empty spec, a missing value, or
/// an unrecognized kind. `ConflictingFlags` maps to the `USAGE` exit code (2).
pub fn parse_target_spec(raw: &str) -> Result<DeliveryTarget, SnuzeError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(SnuzeError::ConflictingFlags(
            "empty --target specification".to_string(),
        ));
    }

    if trimmed.eq_ignore_ascii_case("wait") {
        return Ok(DeliveryTarget::Wait);
    }

    let Some((kind, value)) = trimmed.split_once(':') else {
        return Err(SnuzeError::ConflictingFlags(format!(
            "unrecognized --target '{trimmed}'; expected herdr:<pane>, tmux:<target>, command:<cmd>, or wait"
        )));
    };

    if value.trim().is_empty() {
        return Err(SnuzeError::ConflictingFlags(format!(
            "--target '{kind}:' is missing a value"
        )));
    }

    match kind.trim() {
        "herdr" | "pane" => Ok(DeliveryTarget::Herdr {
            pane_id: value.to_string(),
        }),
        "tmux" => Ok(DeliveryTarget::Tmux {
            target: value.to_string(),
        }),
        "command" => Ok(DeliveryTarget::Command {
            command: value.to_string(),
        }),
        other => Err(SnuzeError::ConflictingFlags(format!(
            "unrecognized --target kind '{other}'; expected herdr, tmux, command, or wait"
        ))),
    }
}

/// Rejects a target whose destination is present but blank.
///
/// # Errors
/// Returns `SnuzeError::ConflictingFlags` for an empty destination.
pub fn validate_target(target: &DeliveryTarget) -> Result<(), SnuzeError> {
    let blank = match target {
        DeliveryTarget::Herdr { pane_id } => pane_id.trim().is_empty(),
        DeliveryTarget::Tmux { target } => target.trim().is_empty(),
        DeliveryTarget::Command { command } => command.trim().is_empty(),
        DeliveryTarget::Wait => false,
    };

    if blank {
        return Err(SnuzeError::ConflictingFlags(format!(
            "delivery target `{}` requires a non-empty destination",
            target.kind()
        )));
    }

    Ok(())
}
