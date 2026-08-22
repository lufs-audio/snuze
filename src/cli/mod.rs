pub mod args;
pub mod dispatch;
pub mod handlers;
pub mod output;

pub use args::{Cli, Command};
pub use dispatch::dispatch;
pub use output::{format_error, format_success, CliResult, JsonError, JsonSuccess};
