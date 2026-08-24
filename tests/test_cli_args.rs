#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use clap::Parser;
use snuze::cli::args::{Cli, Command, SetArgs};

#[test]
fn test_set_args_valid_prompt() {
    let args = [
        "snuze", "set", "--after", "30s", "--pane", "w1:p3", "--prompt", "Wake up",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Set(set) = cli.command {
        assert_eq!(set.after.as_deref(), Some("30s"));
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
        assert_eq!(set.after.as_deref(), Some("1h30m"));
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
    // Missing --pane (required field)
    let args = ["snuze", "set", "--after", "30s", "--prompt", "hi"];
    assert!(Cli::try_parse_from(args).is_err());

    // Missing --after is no longer a parse-level error: --after is optional when --jules is used.
    // The "exactly one of --after or --jules" validation moves to the handler (handle_set).
    // Here we just verify that parsing succeeds without --after.
    let args = ["snuze", "set", "--pane", "w1:p3", "--prompt", "hi"];
    assert!(Cli::try_parse_from(args).is_ok());
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
fn test_set_args_valid_jules() {
    let args = [
        "snuze",
        "set",
        "--jules",
        "sess-1,sess-2",
        "--condition",
        "all-terminal",
        "--timeout",
        "15m",
        "--poll-interval",
        "45s",
        "--pane",
        "w1:p3",
        "--prompt",
        "Jules done",
        "--label",
        "jules-wake",
        "--dry-run",
        "--json",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Set(set) = cli.command {
        assert_eq!(set.jules.as_deref(), Some("sess-1,sess-2"));
        assert_eq!(set.condition.as_deref(), Some("all-terminal"));
        assert_eq!(set.timeout.as_deref(), Some("15m"));
        assert_eq!(set.poll_interval.as_deref(), Some("45s"));
        assert_eq!(set.pane, "w1:p3");
        assert_eq!(set.prompt.as_deref(), Some("Jules done"));
        assert!(set.command.is_none());
        assert_eq!(set.label.as_deref(), Some("jules-wake"));
        assert!(set.dry_run);
        assert!(set.json);
    } else {
        panic!("Expected Set command");
    }
}

#[test]
fn test_set_jules_validation_errors() {
    use snuze::cli::handlers::handle_set;
    use snuze::kernel::ExitCode;

    let temp_dir = tempfile::tempdir().unwrap();
    let state_dir = temp_dir.path();

    // 1. Missing --condition with --jules
    let args = SetArgs {
        after: None,
        jules: Some("sess-1".to_string()),
        condition: None,
        timeout: Some("10m".to_string()),
        poll_interval: None,
        pane: "w1:p1".to_string(),
        prompt: Some("Wake".to_string()),
        command: None,
        label: None,
        dry_run: true,
        json: false,
    };
    let err = handle_set(args, state_dir).unwrap_err();
    assert_eq!(ExitCode::from(&err), ExitCode::Usage);

    // 2. Missing --timeout with --jules
    let args = SetArgs {
        after: None,
        jules: Some("sess-1".to_string()),
        condition: Some("all-terminal".to_string()),
        timeout: None,
        poll_interval: None,
        pane: "w1:p1".to_string(),
        prompt: Some("Wake".to_string()),
        command: None,
        label: None,
        dry_run: true,
        json: false,
    };
    let err = handle_set(args, state_dir).unwrap_err();
    assert_eq!(ExitCode::from(&err), ExitCode::Usage);

    // 3. --command given with --jules
    let args = SetArgs {
        after: None,
        jules: Some("sess-1".to_string()),
        condition: Some("all-terminal".to_string()),
        timeout: Some("10m".to_string()),
        poll_interval: None,
        pane: "w1:p1".to_string(),
        prompt: None,
        command: Some("cargo check".to_string()),
        label: None,
        dry_run: true,
        json: false,
    };
    let err = handle_set(args, state_dir).unwrap_err();
    assert_eq!(ExitCode::from(&err), ExitCode::Usage);

    // 4. Both --after and --jules provided
    let args = SetArgs {
        after: Some("10s".to_string()),
        jules: Some("sess-1".to_string()),
        condition: Some("all-terminal".to_string()),
        timeout: Some("10m".to_string()),
        poll_interval: None,
        pane: "w1:p1".to_string(),
        prompt: Some("Wake".to_string()),
        command: None,
        label: None,
        dry_run: true,
        json: false,
    };
    let err = handle_set(args, state_dir).unwrap_err();
    assert_eq!(ExitCode::from(&err), ExitCode::Usage);

    // 5. Malformed --jules (trailing comma)
    let args = SetArgs {
        after: None,
        jules: Some("sess-1,".to_string()),
        condition: Some("all-terminal".to_string()),
        timeout: Some("10m".to_string()),
        poll_interval: None,
        pane: "w1:p1".to_string(),
        prompt: Some("Wake".to_string()),
        command: None,
        label: None,
        dry_run: true,
        json: false,
    };
    let err = handle_set(args, state_dir).unwrap_err();
    assert_eq!(ExitCode::from(&err), ExitCode::Usage);

    // 6. Unrecognized --condition
    let args = SetArgs {
        after: None,
        jules: Some("sess-1".to_string()),
        condition: Some("invalid-condition".to_string()),
        timeout: Some("10m".to_string()),
        poll_interval: None,
        pane: "w1:p1".to_string(),
        prompt: Some("Wake".to_string()),
        command: None,
        label: None,
        dry_run: true,
        json: false,
    };
    let err = handle_set(args, state_dir).unwrap_err();
    assert_eq!(ExitCode::from(&err), ExitCode::Usage);
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
