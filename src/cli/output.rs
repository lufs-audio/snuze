use crate::kernel::{ExitCode, SnuzeError};
use serde::Serialize;
use serde_json::Value;

/// Standard CLI result type returning a JSON value or domain error.
pub type CliResult = Result<Value, SnuzeError>;

#[derive(Serialize)]
pub struct JsonSuccess {
    pub status: String,
    pub data: Value,
}

#[derive(Serialize)]
pub struct JsonError {
    pub status: String,
    pub code: i32,
    pub message: String,
}

/// Wraps success data into the standard `{"status": "success", "data": ...}` envelope.
#[must_use]
pub fn format_success(data: Value) -> Value {
    serde_json::to_value(JsonSuccess {
        status: "success".to_string(),
        data,
    })
    .unwrap_or(Value::Null)
}

/// Wraps error details into the standard `{"status": "error", "code": ..., "message": ...}` envelope.
#[must_use]
pub fn format_error(error: &SnuzeError) -> Value {
    let exit_code = ExitCode::from(error).as_i32();
    serde_json::to_value(JsonError {
        status: "error".to_string(),
        code: exit_code,
        message: error.to_string(),
    })
    .unwrap_or(Value::Null)
}
