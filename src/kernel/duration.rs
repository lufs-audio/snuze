use std::time::Duration;
use crate::kernel::errors::SnuzeError;

/// Parses a duration string (e.g. `30s`, `15m`, `1h`, `90m`, `1h30m`) into a `Duration`.
///
/// # Errors
///
/// Returns `SnuzeError::InvalidDuration` if the input is empty, contains invalid characters,
/// is missing unit suffixes or numeric amounts, or overflows `u64` seconds.
pub fn parse_duration(s: &str) -> Result<Duration, SnuzeError> {
    if s.is_empty() {
        return Err(SnuzeError::InvalidDuration("duration string is empty".to_string()));
    }

    let mut total_secs: u64 = 0;
    let mut num_buf = String::new();
    let mut unit_buf = String::new();

    let mut chars = s.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            if !unit_buf.is_empty() {
                let mult = match unit_buf.as_str() {
                    "s" => 1u64,
                    "m" => 60u64,
                    "h" => 3600u64,
                    "d" => 86400u64,
                    _ => return Err(SnuzeError::InvalidDuration(format!("unknown unit '{unit_buf}'"))),
                };
                let val: u64 = num_buf.parse().map_err(|_| {
                    SnuzeError::InvalidDuration(format!("invalid number '{num_buf}'"))
                })?;
                let part = val.checked_mul(mult).ok_or_else(|| {
                    SnuzeError::InvalidDuration("duration overflow".to_string())
                })?;
                total_secs = total_secs.checked_add(part).ok_or_else(|| {
                    SnuzeError::InvalidDuration("duration overflow".to_string())
                })?;
                num_buf.clear();
                unit_buf.clear();
            }
            num_buf.push(c);
            chars.next();
        } else if c.is_ascii_alphabetic() {
            if num_buf.is_empty() {
                return Err(SnuzeError::InvalidDuration(format!("missing number before unit '{c}'")));
            }
            unit_buf.push(c);
            chars.next();
        } else {
            return Err(SnuzeError::InvalidDuration(format!("invalid character '{c}'")));
        }
    }

    if num_buf.is_empty() {
        return Err(SnuzeError::InvalidDuration("missing number in duration".to_string()));
    }

    if unit_buf.is_empty() {
        return Err(SnuzeError::InvalidDuration(format!("missing unit for number '{num_buf}'")));
    }

    let mult = match unit_buf.as_str() {
        "s" => 1u64,
        "m" => 60u64,
        "h" => 3600u64,
        "d" => 86400u64,
        _ => return Err(SnuzeError::InvalidDuration(format!("unknown unit '{unit_buf}'"))),
    };
    let val: u64 = num_buf.parse().map_err(|_| {
        SnuzeError::InvalidDuration(format!("invalid number '{num_buf}'"))
    })?;
    let part = val.checked_mul(mult).ok_or_else(|| {
        SnuzeError::InvalidDuration("duration overflow".to_string())
    })?;
    total_secs = total_secs.checked_add(part).ok_or_else(|| {
        SnuzeError::InvalidDuration("duration overflow".to_string())
    })?;

    Ok(Duration::from_secs(total_secs))
}
