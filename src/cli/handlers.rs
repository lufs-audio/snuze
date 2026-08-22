use crate::cli::args::{
    CancelArgs, DaemonAction, DaemonArgs, DoctorArgs, ListArgs, SetArgs, StatusArgs,
};
use crate::cli::output::CliResult;
use crate::daemon::{Daemon, DaemonClient};
use crate::herdr::RealHerdrAdapter;
use crate::kernel::duration::parse_duration;
use crate::kernel::{Alarm, AlarmStatus, DeliveryMode, SnuzeError, Trigger};
use crate::store::AlarmStore;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::time::SystemTime;
use uuid::Uuid;

fn format_alarm_human(alarm: &Alarm) -> String {
    let mode_str = match &alarm.delivery {
        DeliveryMode::Prompt { text } => format!("Prompt: {text}"),
        DeliveryMode::Command { text } => format!("Command: {text}"),
    };

    let label_str = alarm.label.as_deref().unwrap_or("-");
    let failure_str = alarm.failure_reason.as_deref().unwrap_or("-");
    let caller_str = alarm.created_by_pane.as_deref().unwrap_or("-");

    format!(
        "ID: {}\n\
         Pane: {}\n\
         Created By Pane: {}\n\
         Status: {:?}\n\
         Delivery: {}\n\
         Label: {}\n\
         Failure Reason: {}",
        alarm.id, alarm.pane_id, caller_str, alarm.status, mode_str, label_str, failure_str
    )
}

/// Handles scheduling a new alarm.
///
/// # Errors
/// Returns `SnuzeError` if argument validation, storage persistence, or daemon communication fails.
pub fn handle_set(args: SetArgs, state_dir: &Path) -> CliResult {
    if (args.prompt.is_some() && args.command.is_some())
        || (args.prompt.is_none() && args.command.is_none())
    {
        return Err(SnuzeError::ConflictingFlags(
            "exactly one of --prompt or --command is required".to_string(),
        ));
    }

    let duration = parse_duration(&args.after)?;

    let adapter = RealHerdrAdapter::new()?;
    if !adapter.pane_exists(&args.pane)? {
        return Err(SnuzeError::PaneNotFound(args.pane));
    }

    let created_by_pane = std::env::var("HERDR_PANE_ID").ok();

    let delivery = if let Some(prompt) = args.prompt {
        DeliveryMode::Prompt { text: prompt }
    } else if let Some(command) = args.command {
        DeliveryMode::Command { text: command }
    } else {
        return Err(SnuzeError::ConflictingFlags(
            "missing delivery payload".to_string(),
        ));
    };

    let now = SystemTime::now();
    let deadline = now + duration;

    let alarm = Alarm {
        id: Uuid::now_v7().to_string(),
        pane_id: args.pane,
        created_by_pane,
        delivery,
        label: args.label,
        created_at: now,
        deadline,
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::default(),
        schema_version: 1,
    };

    if args.dry_run {
        if args.json {
            return Ok(serde_json::to_value(&alarm).unwrap_or(serde_json::Value::Null));
        }
        return Ok(serde_json::Value::String(format_alarm_human(&alarm)));
    }

    Daemon::ensure_running(state_dir)?;

    let client = DaemonClient::connect(state_dir)?;
    client.set_alarm(alarm.clone())?;

    if args.json {
        Ok(serde_json::to_value(&alarm).unwrap_or(serde_json::Value::Null))
    } else {
        Ok(serde_json::Value::String(alarm.id))
    }
}

/// Handles listing alarms.
///
/// # Errors
/// Returns `SnuzeError` if daemon communication fails or if filter is invalid.
pub fn handle_list(args: &ListArgs, state_dir: &Path) -> CliResult {
    let status_filter = match args.status.as_deref() {
        Some("pending") => Some(AlarmStatus::Pending),
        Some("fired") => Some(AlarmStatus::Fired),
        Some("failed") => Some(AlarmStatus::Failed),
        Some("missed") => Some(AlarmStatus::Missed),
        Some("canceled") => Some(AlarmStatus::Canceled),
        Some(other) => {
            return Err(SnuzeError::ConflictingFlags(format!(
                "invalid status filter: {other}"
            )))
        }
        None => None,
    };

    let client = DaemonClient::connect(state_dir)?;
    let alarms = client.list_alarms(status_filter)?;

    if args.json {
        Ok(serde_json::to_value(&alarms).unwrap_or(serde_json::Value::Null))
    } else {
        let mut output = String::new();
        let _ = writeln!(output, "{:-<80}", "");
        let _ = writeln!(
            output,
            "{:<38} {:<12} {:<10} {:<15}",
            "ID", "PANE", "STATUS", "LABEL"
        );
        let _ = writeln!(output, "{:-<80}", "");
        for alarm in alarms {
            let label = alarm.label.as_deref().unwrap_or("-");
            let status = match alarm.status {
                AlarmStatus::Pending => "pending",
                AlarmStatus::Fired => "fired",
                AlarmStatus::Failed => "failed",
                AlarmStatus::Missed => "missed",
                AlarmStatus::Canceled => "canceled",
            };
            let _ = writeln!(
                output,
                "{:<38} {:<12} {:<10} {:<15}",
                alarm.id, alarm.pane_id, status, label
            );
        }
        Ok(serde_json::Value::String(output))
    }
}

/// Handles querying a specific alarm status.
///
/// # Errors
/// Returns `SnuzeError` if the alarm is not found or daemon is unreachable.
pub fn handle_status(args: &StatusArgs, state_dir: &Path) -> CliResult {
    let client = DaemonClient::connect(state_dir)?;
    let alarm = client.get_alarm(&args.alarm_id)?;
    if args.json {
        Ok(serde_json::to_value(&alarm).unwrap_or(serde_json::Value::Null))
    } else {
        Ok(serde_json::Value::String(format_alarm_human(&alarm)))
    }
}

/// Handles canceling alarms.
///
/// # Errors
/// Returns `SnuzeError` if daemon is unreachable or argument flags are malformed.
pub fn handle_cancel(args: CancelArgs, state_dir: &Path) -> CliResult {
    let client = DaemonClient::connect(state_dir)?;

    if args.all {
        let pending = client.list_alarms(Some(AlarmStatus::Pending))?;
        let mut canceled_count = 0;
        for alarm in &pending {
            if client.cancel_alarm(&alarm.id)? {
                canceled_count += 1;
            }
        }
        if args.json {
            Ok(serde_json::json!({ "canceled_count": canceled_count }))
        } else {
            Ok(serde_json::Value::String(format!(
                "Canceled {canceled_count} alarms"
            )))
        }
    } else if let Some(id) = args.alarm_id {
        let canceled = client.cancel_alarm(&id)?;
        if args.json {
            Ok(serde_json::json!({ "canceled": canceled }))
        } else {
            Ok(serde_json::Value::String(if canceled {
                "Canceled".to_string()
            } else {
                "Not pending/already terminal".to_string()
            }))
        }
    } else {
        Err(SnuzeError::ConflictingFlags(
            "either --all or an ALARM_ID is required".to_string(),
        ))
    }
}

/// Handles daemon control requests.
///
/// # Errors
/// Returns `SnuzeError` if daemon cannot be started or reached.
pub fn handle_daemon(args: &DaemonArgs, state_dir: &Path) -> CliResult {
    match args.action {
        DaemonAction::Start { foreground } => {
            if foreground {
                Daemon::run_foreground(state_dir)?;
                Ok(serde_json::Value::String("Daemon exited cleanly".to_string()))
            } else {
                Daemon::ensure_running(state_dir)?;
                Ok(serde_json::Value::String("Daemon started".to_string()))
            }
        }
        DaemonAction::Stop => {
            Daemon::stop(state_dir)?;
            Ok(serde_json::Value::String("Daemon stopped".to_string()))
        }
        DaemonAction::Status => {
            let running = Daemon::is_running(state_dir);
            if args.json {
                Ok(serde_json::json!({ "running": running }))
            } else {
                Ok(serde_json::Value::String(if running {
                    "Daemon is running".to_string()
                } else {
                    "Daemon is stopped".to_string()
                }))
            }
        }
        DaemonAction::Restart => {
            let _ = Daemon::stop(state_dir);
            Daemon::ensure_running(state_dir)?;
            Ok(serde_json::Value::String("Daemon restarted".to_string()))
        }
    }
}

/// Runs self-diagnostic checks.
///
/// # Errors
/// Returns `SnuzeError` if diagnostics determine an unrecoverable component failure.
pub fn handle_doctor(args: &DoctorArgs, state_dir: &Path) -> CliResult {
    let mut check_results = HashMap::new();

    let exe = std::env::var("FAKE_HERDR_PATH").unwrap_or_else(|_| "herdr".to_string());
    let mut herdr_version = std::process::Command::new(&exe);
    herdr_version.arg("--version");

    let herdr_binary = match herdr_version.output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => "missing".to_string(),
    };
    check_results.insert("herdr_binary", serde_json::json!(herdr_binary));

    let mut herdr_api = std::process::Command::new(&exe);
    herdr_api.arg("api").arg("schema");
    let herdr_socket_reachable = match herdr_api.output() {
        Ok(out) => out.status.success(),
        _ => false,
    };
    check_results.insert(
        "herdr_socket_reachable",
        serde_json::json!(herdr_socket_reachable),
    );

    let daemon_running = Daemon::is_running(state_dir);
    check_results.insert("daemon_running", serde_json::json!(daemon_running));

    let test_file = state_dir.join(".snuze_doctor_probe");
    let state_dir_writable = (|| {
        std::fs::create_dir_all(state_dir).ok()?;
        std::fs::write(&test_file, b"ok").ok()?;
        std::fs::remove_file(&test_file).ok()?;
        Some(true)
    })()
    .unwrap_or(false);
    check_results.insert(
        "state_dir_writable",
        serde_json::json!(state_dir_writable),
    );

    let mut counts = HashMap::new();
    if let Ok(store) = AlarmStore::open(state_dir) {
        if let Ok(alarms) = store.list(None) {
            for a in alarms {
                let key = match a.status {
                    AlarmStatus::Pending => "pending",
                    AlarmStatus::Fired => "fired",
                    AlarmStatus::Failed => "failed",
                    AlarmStatus::Missed => "missed",
                    AlarmStatus::Canceled => "canceled",
                };
                *counts.entry(key).or_insert(0) += 1;
            }
        }
    }
    check_results.insert("alarm_counts", serde_json::json!(counts));

    let failed_count = counts.get("failed").copied().unwrap_or(0);
    let missed_count = counts.get("missed").copied().unwrap_or(0);
    let pending_count = counts.get("pending").copied().unwrap_or(0);

    let herdr_ok = herdr_binary != "missing";

    if !herdr_ok {
        if args.json {
            return Err(SnuzeError::HerdrBinaryMissing);
        }
        return Err(SnuzeError::HerdrBinaryMissing);
    }

    if !state_dir_writable {
        return Err(SnuzeError::PersistFailed(
            "state directory is not writable".to_string(),
        ));
    }

    if !daemon_running {
        return Err(SnuzeError::DaemonUnavailable(
            "snuze daemon is not running".to_string(),
        ));
    }

    if args.json {
        Ok(serde_json::to_value(&check_results).unwrap_or(serde_json::Value::Null))
    } else {
        let output = format!(
            "Herdr Binary: {}\n\
             Herdr Socket: {}\n\
             Daemon Running: {}\n\
             State Dir Writable: {}\n\
             Pending Alarms: {}\n\
             Failed Alarms: {}\n\
             Missed Alarms: {}",
            herdr_binary,
            if herdr_socket_reachable { "reachable" } else { "unreachable" },
            if daemon_running { "running" } else { "stopped" },
            if state_dir_writable { "yes" } else { "no" },
            pending_count,
            failed_count,
            missed_count
        );
        Ok(serde_json::Value::String(output))
    }
}
