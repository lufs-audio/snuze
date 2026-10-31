#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]
#![allow(clippy::too_many_lines)]

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
        assert_eq!(set.pane.as_deref(), Some("w1:p3"));
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
        assert_eq!(set.pane.as_deref(), Some("w1:p2"));
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
    // `--pane` is no longer a required *parse-level* flag: the delivery target is
    // resolved inside `handle_set` (explicit flag, else $HERDR_ENV/$TMUX
    // auto-detection, else a USAGE error). Parsing must succeed without it.
    let args = ["snuze", "set", "--after", "30s", "--prompt", "hi"];
    assert!(Cli::try_parse_from(args).is_ok());

    // Missing --after is likewise a handler-level validation, not a clap error.
    let args = ["snuze", "set", "--pane", "w1:p3", "--prompt", "hi"];
    assert!(Cli::try_parse_from(args).is_ok());
}

#[test]
fn test_set_args_delivery_target_flags() {
    // --tmux
    let args = [
        "snuze", "set", "--after", "1m", "--tmux", "%0", "--prompt", "hi",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Set(set) = cli.command {
        assert_eq!(set.tmux.as_deref(), Some("%0"));
        assert!(set.pane.is_none());
        assert!(set.herdr.is_none());
        assert!(set.target.is_none());
    } else {
        panic!("Expected Set command");
    }

    // --herdr
    let args = [
        "snuze", "set", "--after", "1m", "--herdr", "w1:p3", "--prompt", "hi",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Set(set) = cli.command {
        assert_eq!(set.herdr.as_deref(), Some("w1:p3"));
    } else {
        panic!("Expected Set command");
    }

    // --target
    let args = [
        "snuze", "set", "--after", "1m", "--target", "tmux:%2", "--prompt", "hi",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Set(set) = cli.command {
        assert_eq!(set.target.as_deref(), Some("tmux:%2"));
    } else {
        panic!("Expected Set command");
    }
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
fn test_signal_args_parsing() {
    let args = ["snuze", "signal", "deploy-complete", "--dry-run", "--json"];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Signal(sig) = cli.command {
        assert_eq!(sig.match_key, "deploy-complete");
        assert!(sig.dry_run);
        assert!(sig.json);
    } else {
        panic!("Expected Signal command");
    }
}

#[test]
fn test_set_args_signal_and_herdr_validation_errors() {
    use snuze::cli::handlers::handle_set;
    use snuze::kernel::ExitCode;

    let temp_dir = tempfile::tempdir().unwrap();
    let state_dir = temp_dir.path();

    // 1. --signal without --timeout
    let args = SetArgs {
        after: None,
        signal: Some("my_key".to_string()),
        herdr_pane: None,
        pid: None,
        watch_file: None,
        watch_mode: None,
        watch_for: None,
        timeout: None,
        pane: Some("w1:p1".to_string()),
        herdr: None,
        tmux: None,
        target: None,
        prompt: Some("Wake".to_string()),
        command: None,
        label: None,
        dry_run: true,
        json: false,
    };
    let err = handle_set(args, state_dir).unwrap_err();
    assert_eq!(ExitCode::from(&err), ExitCode::Usage);

    // 3. --herdr-pane without --watch-for
    let args = SetArgs {
        after: None,
        signal: None,
        herdr_pane: Some("w1:p2".to_string()),
        pid: None,
        watch_file: None,
        watch_mode: None,
        watch_for: None,
        timeout: Some("10m".to_string()),
        pane: Some("w1:p1".to_string()),
        herdr: None,
        tmux: None,
        target: None,
        prompt: Some("Wake".to_string()),
        command: None,
        label: None,
        dry_run: true,
        json: false,
    };
    let err = handle_set(args, state_dir).unwrap_err();
    assert_eq!(ExitCode::from(&err), ExitCode::Usage);

    // 4. --herdr-pane without --timeout
    let args = SetArgs {
        after: None,
        signal: None,
        herdr_pane: Some("w1:p2".to_string()),
        pid: None,
        watch_file: None,
        watch_mode: None,
        watch_for: Some("idle,done".to_string()),
        timeout: None,
        pane: Some("w1:p1".to_string()),
        herdr: None,
        tmux: None,
        target: None,
        prompt: Some("Wake".to_string()),
        command: None,
        label: None,
        dry_run: true,
        json: false,
    };
    let err = handle_set(args, state_dir).unwrap_err();
    assert_eq!(ExitCode::from(&err), ExitCode::Usage);

    // 5. --herdr-pane with invalid --watch-for status
    let args = SetArgs {
        after: None,
        signal: None,
        herdr_pane: Some("w1:p2".to_string()),
        pid: None,
        watch_file: None,
        watch_mode: None,
        watch_for: Some("idle,unknown_status".to_string()),
        timeout: Some("10m".to_string()),
        pane: Some("w1:p1".to_string()),
        herdr: None,
        tmux: None,
        target: None,
        prompt: Some("Wake".to_string()),
        command: None,
        label: None,
        dry_run: true,
        json: false,
    };
    let err = handle_set(args, state_dir).unwrap_err();
    assert_eq!(ExitCode::from(&err), ExitCode::Usage);

    // 6. --herdr-pane with empty status in list
    let args = SetArgs {
        after: None,
        signal: None,
        herdr_pane: Some("w1:p2".to_string()),
        pid: None,
        watch_file: None,
        watch_mode: None,
        watch_for: Some("idle,".to_string()),
        timeout: Some("10m".to_string()),
        pane: Some("w1:p1".to_string()),
        herdr: None,
        tmux: None,
        target: None,
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

#[test]
fn test_parse_target_spec() {
    use snuze::cli::target_parse::{parse_target_spec, validate_target};
    use snuze::kernel::{DeliveryTarget, ExitCode};

    assert_eq!(
        parse_target_spec("tmux:%0").unwrap(),
        DeliveryTarget::Tmux {
            target: "%0".to_string()
        }
    );
    assert_eq!(
        parse_target_spec("herdr:w1:p3").unwrap(),
        DeliveryTarget::Herdr {
            pane_id: "w1:p3".to_string()
        }
    );
    assert_eq!(
        parse_target_spec("pane:w1:p3").unwrap(),
        DeliveryTarget::Herdr {
            pane_id: "w1:p3".to_string()
        }
    );
    assert_eq!(
        parse_target_spec("command:cargo test --all").unwrap(),
        DeliveryTarget::Command {
            command: "cargo test --all".to_string()
        }
    );
    assert_eq!(parse_target_spec("wait").unwrap(), DeliveryTarget::Wait);
    assert_eq!(parse_target_spec("  WAIT  ").unwrap(), DeliveryTarget::Wait);

    // Malformed specs must be a USAGE (exit 2) error, never a silent default.
    for bad in ["", "   ", "tmux:", "nope:x", "wait:extra", "w1:p3"] {
        let err = parse_target_spec(bad).unwrap_err();
        assert_eq!(ExitCode::from(&err), ExitCode::Usage, "input {bad:?}");
    }

    // Blank destinations are rejected by validate_target.
    assert!(validate_target(&DeliveryTarget::Tmux {
        target: "   ".to_string()
    })
    .is_err());
    assert!(validate_target(&DeliveryTarget::Herdr {
        pane_id: String::new()
    })
    .is_err());
    assert!(validate_target(&DeliveryTarget::Command {
        command: " ".to_string()
    })
    .is_err());
    assert!(validate_target(&DeliveryTarget::Wait).is_ok());
    assert!(validate_target(&DeliveryTarget::Tmux {
        target: "%0".to_string()
    })
    .is_ok());
}

#[test]
fn test_set_args_pid_flag() {
    let args = [
        "snuze", "set", "--pid", "4242", "--tmux", "%0", "--prompt", "hi",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Set(set) = cli.command {
        assert_eq!(set.pid, Some(4242));
        assert!(set.after.is_none());
    } else {
        panic!("Expected Set command");
    }

    // --pid must be a valid u32 at parse time.
    let bad = [
        "snuze",
        "set",
        "--pid",
        "not-a-pid",
        "--tmux",
        "%0",
        "--prompt",
        "hi",
    ];
    assert!(Cli::try_parse_from(bad).is_err());
}

#[test]
fn test_set_args_watch_file_flags() {
    let args = [
        "snuze",
        "set",
        "--watch-file",
        "/tmp/take.json",
        "--watch-mode",
        "modified",
        "--tmux",
        "%0",
        "--prompt",
        "hi",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    if let Command::Set(set) = cli.command {
        assert_eq!(
            set.watch_file.as_deref(),
            Some(std::path::Path::new("/tmp/take.json"))
        );
        assert_eq!(set.watch_mode.as_deref(), Some("modified"));
        assert!(set.pid.is_none());
    } else {
        panic!("Expected Set command");
    }
}

#[test]
fn test_parse_file_watch_mode() {
    use snuze::cli::watch_parse::parse_file_watch_mode;
    use snuze::kernel::{ExitCode, FileWatchMode};

    assert_eq!(
        parse_file_watch_mode("exists").unwrap(),
        FileWatchMode::Exists
    );
    assert_eq!(
        parse_file_watch_mode("  MODIFIED ").unwrap(),
        FileWatchMode::Modified
    );
    let err = parse_file_watch_mode("sideways").unwrap_err();
    assert_eq!(ExitCode::from(&err), ExitCode::Usage);
}
