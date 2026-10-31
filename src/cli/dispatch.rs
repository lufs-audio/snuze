use crate::cli::args::Command;
use crate::cli::output::CliResult;
use std::path::Path;

/// Dispatches a parsed subcommand to its handler.
///
/// # Errors
/// Returns `SnuzeError` based on command execution failure.
pub fn dispatch(command: Command, state_dir: &Path) -> CliResult {
    match command {
        Command::Set(args) => crate::cli::handlers::handle_set(args, state_dir),
        Command::List(args) => crate::cli::handlers::handle_list(&args, state_dir),
        Command::Status(args) => crate::cli::handlers::handle_status(&args, state_dir),
        Command::Cancel(args) => crate::cli::handlers::handle_cancel(args, state_dir),
        Command::Daemon(args) => crate::cli::handlers::handle_daemon(&args, state_dir),
        Command::Events(args) => crate::cli::handlers::handle_events(&args, state_dir),
        Command::Doctor(args) => crate::cli::handlers::handle_doctor(&args, state_dir),
        Command::Signal(args) => crate::cli::handlers::handle_signal(&args, state_dir),
        Command::Wait(args) => crate::cli::handlers::handle_wait(&args, state_dir),
    }
}
