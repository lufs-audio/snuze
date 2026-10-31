pub mod args;
pub mod dispatch;
pub mod handlers;
pub mod output;
pub mod target_parse;
pub mod watch_parse;

pub use args::{Cli, Command};
pub use dispatch::dispatch;
pub use output::{format_error, format_success, CliResult, JsonError, JsonSuccess};
pub use target_parse::{parse_target_spec, validate_target};
pub use watch_parse::{parse_file_watch_mode, parse_watch_statuses};
