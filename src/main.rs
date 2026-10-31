use clap::Parser;
use snuze::cli::args::{Cli, Command};
use snuze::cli::dispatch::dispatch;
use snuze::cli::output::{format_error, format_success};
use snuze::kernel::ExitCode;
use snuze::store::resolve_state_dir;

fn main() {
    let cli = Cli::parse();
    let state_dir = resolve_state_dir();

    let is_json = match &cli.command {
        Command::Set(args) => args.json,
        Command::List(args) => args.json,
        Command::Status(args) => args.json,
        Command::Cancel(args) => args.json,
        Command::Daemon(args) => args.json,
        Command::Events(args) => match &args.action {
            snuze::cli::args::EventsAction::Drain { json } => *json,
        },
        Command::Doctor(args) => args.json,
        Command::Signal(args) => args.json,
        Command::Wait(args) => args.json,
    };

    let result = dispatch(cli.command, &state_dir);

    match result {
        Ok(data) => {
            if is_json {
                println!("{}", format_success(data));
            } else if let Some(s) = data.as_str() {
                println!("{s}");
            } else if !data.is_null() {
                println!("{data}");
            }
            std::process::exit(ExitCode::Success.as_i32());
        }
        Err(e) => {
            let exit_code = ExitCode::from(&e).as_i32();
            if is_json {
                println!("{}", format_error(&e));
            } else {
                eprintln!("Error: {e}");
            }
            std::process::exit(exit_code);
        }
    }
}
