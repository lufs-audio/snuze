#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use clap::Parser;
use snuze::cli::args::{Cli, Command};

#[test]
fn test_set_args_valid_prompt() {
    let args = [
        "snuze", "set", "--after", "30s", "--pane", "w1:p3", "--prompt", "Wake up",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Set(set) = cli.command {
        assert_eq!(set.after, "30s");
        assert_eq!(set.pane, "w1:p3");
        assert_eq!(set.prompt.as_deref(), Some("Wake up"));
        assert!(set.command.is_none());
        assert!(!set.dry_run);
        assert!(!set.json);
    } else {
        panic!("Expected Set command");
    }
}

#[test]
fn test_set_args_valid_command() {
    let args = [
        "snuze",
        "set",
        "--after",
        "1h30m",
        "--pane",
        "w1:p2",
        "--command",
        "cargo test",
        "--label",
        "CI",
        "--dry-run",
        "--json",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Set(set) = cli.command {
        assert_eq!(set.after, "1h30m");
        assert_eq!(set.pane, "w1:p2");
        assert_eq!(set.command.as_deref(), Some("cargo test"));
        assert!(set.prompt.is_none());
        assert_eq!(set.label.as_deref(), Some("CI"));
        assert!(set.dry_run);
        assert!(set.json);
    } else {
        panic!("Expected Set command");
    }
}

#[test]
fn test_set_args_missing_required() {
    // Missing --pane
    let args = ["snuze", "set", "--after", "30s", "--prompt", "hi"];
    assert!(Cli::try_parse_from(args).is_err());

    // Missing --after
    let args = ["snuze", "set", "--pane", "w1:p3", "--prompt", "hi"];
    assert!(Cli::try_parse_from(args).is_err());
}

#[test]
fn test_list_args() {
    let args = ["snuze", "list"];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::List(list) = cli.command {
        assert!(list.status.is_none());
        assert!(!list.json);
    } else {
        panic!("Expected List command");
    }

    let args_filter = ["snuze", "list", "--status", "pending", "--json"];
    let cli_filter = Cli::try_parse_from(args_filter).unwrap();
    if let Command::List(list) = cli_filter.command {
        assert_eq!(list.status.as_deref(), Some("pending"));
        assert!(list.json);
    } else {
        panic!("Expected List command");
    }
}

#[test]
fn test_status_args() {
    let args = ["snuze", "status", "01J5X9M8X2S9C5M94K60M8X2S9", "--json"];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Status(status) = cli.command {
        assert_eq!(status.alarm_id, "01J5X9M8X2S9C5M94K60M8X2S9");
        assert!(status.json);
    } else {
        panic!("Expected Status command");
    }

    // Missing positional ALARM_ID
    let args_missing = ["snuze", "status"];
    assert!(Cli::try_parse_from(args_missing).is_err());
}

#[test]
fn test_cancel_args() {
    // Single ID
    let args = ["snuze", "cancel", "01J5X9M8X2S9C5M94K60M8X2S9"];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Cancel(cancel) = cli.command {
        assert_eq!(
            cancel.alarm_id.as_deref(),
            Some("01J5X9M8X2S9C5M94K60M8X2S9")
        );
        assert!(!cancel.all);
    } else {
        panic!("Expected Cancel command");
    }

    // --all
    let args_all = ["snuze", "cancel", "--all", "--json"];
    let cli_all = Cli::try_parse_from(args_all).unwrap();
    if let Command::Cancel(cancel) = cli_all.command {
        assert!(cancel.alarm_id.is_none());
        assert!(cancel.all);
        assert!(cancel.json);
    } else {
        panic!("Expected Cancel command");
    }

    // Both ID and --all conflict
    let args_conflict = ["snuze", "cancel", "01J5X9M8X2S9C5M94K60M8X2S9", "--all"];
    assert!(Cli::try_parse_from(args_conflict).is_err());

    // Neither ID nor --all
    let args_neither = ["snuze", "cancel"];
    assert!(Cli::try_parse_from(args_neither).is_err());
}

#[test]
fn test_daemon_args() {
    let args_start = ["snuze", "daemon", "start", "--foreground", "--json"];
    let cli = Cli::try_parse_from(args_start).unwrap();
    if let Command::Daemon(daemon) = cli.command {
        assert!(daemon.json);
        match daemon.action {
            snuze::cli::args::DaemonAction::Start { foreground } => assert!(foreground),
            _ => panic!("Expected Start action"),
        }
    } else {
        panic!("Expected Daemon command");
    }

    let args_stop = ["snuze", "daemon", "stop"];
    let cli_stop = Cli::try_parse_from(args_stop).unwrap();
    assert!(matches!(
        cli_stop.command,
        Command::Daemon(snuze::cli::args::DaemonArgs {
            action: snuze::cli::args::DaemonAction::Stop,
            ..
        })
    ));
}

#[test]
fn test_doctor_args() {
    let args = ["snuze", "doctor", "--json"];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Doctor(doctor) = cli.command {
        assert!(doctor.json);
    } else {
        panic!("Expected Doctor command");
    }
}
