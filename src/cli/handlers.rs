use crate::cli::args::{
    CancelArgs, DaemonAction, DaemonArgs, DoctorArgs, EventsAction, EventsArgs, ListArgs, SetArgs,
    SignalArgs, StatusArgs,
};
use crate::cli::jules_parse::{parse_jules_condition, parse_jules_session_ids};
use crate::cli::output::CliResult;
use crate::cli::watch_parse::parse_watch_statuses;
use crate::daemon::{Daemon, DaemonClient};
use crate::herdr::RealHerdrAdapter;
use crate::jules::adapter::RealJulesAdapter;
use crate::jules::JulesAdapter;
use crate::kernel::duration::parse_duration;
use crate::kernel::{Alarm, AlarmStatus, DeliveryMode, JulesCondition, SnuzeError, Trigger};
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

    let mut status_str = format!("{:?}", alarm.status);
    if alarm.status == AlarmStatus::Pending {
        match &alarm.trigger {
            Trigger::JulesCondition { last_known, .. } => {
                for s in last_known {
                    let _ = write!(status_str, "\n  {}: {}", s.session_id, s.state.as_str());
                }
            }
            Trigger::HerdrStatus {
                watched_pane_id,
                watch_for,
                last_known,
                ..
            } => {
                let watch_str = watch_for
                    .iter()
                    .map(|s| {
                        serde_json::to_string(s)
                            .unwrap_or_default()
                            .trim_matches('"')
                            .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let last_known_str = last_known.map_or_else(
                    || "none".to_string(),
                    |s| {
                        serde_json::to_string(&s)
                            .unwrap_or_default()
                            .trim_matches('"')
                            .to_string()
                    },
                );
                let _ = write!(
                    status_str,
                    "\n  watching {watched_pane_id} for [{watch_str}] (last: {last_known_str})"
                );
            }
            Trigger::ExternalSignal { match_key, .. } => {
                let _ = write!(status_str, "\n  waiting for signal '{match_key}'");
            }
            Trigger::Deadline => {}
        }
    }

    format!(
        "ID: {}\n\
         Pane: {}\n\
         Created By Pane: {}\n\
         Status: {}\n\
         Delivery: {}\n\
         Label: {}\n\
         Failure Reason: {}",
        alarm.id, alarm.pane_id, caller_str, status_str, mode_str, label_str, failure_str
    )
}

/// Handles scheduling a new alarm.
///
/// # Errors
/// Returns `SnuzeError` if argument validation, storage persistence, or daemon communication fails.
#[allow(clippy::too_many_lines)]
pub fn handle_set(args: SetArgs, state_dir: &Path) -> CliResult {
    let is_after = args.after.is_some();
    let is_jules = args.jules.is_some();
    let is_signal = args.signal.is_some();
    let is_herdr_pane = args.herdr_pane.is_some();

    // 1. Exactly one trigger mode must be present
    let mode_count = [is_after, is_jules, is_signal, is_herdr_pane]
        .into_iter()
        .filter(|&x| x)
        .count();

    if mode_count != 1 {
        return Err(SnuzeError::ConflictingFlags(
            "exactly one of --after, --jules, --signal, or --herdr-pane is required".to_string(),
        ));
    }

    // 2. Delivery mode validation
    if is_jules {
        if args.command.is_some() {
            return Err(SnuzeError::ConflictingFlags(
                "--command is not supported with --jules (condition mode does not support command delivery)".to_string(),
            ));
        }
        if args.prompt.is_none() {
            return Err(SnuzeError::ConflictingFlags(
                "--prompt is required with --jules".to_string(),
            ));
        }
    } else if (args.prompt.is_some() && args.command.is_some())
        || (args.prompt.is_none() && args.command.is_none())
    {
        return Err(SnuzeError::ConflictingFlags(
            "exactly one of --prompt or --command is required".to_string(),
        ));
    }

    // 3. Required flags per mode
    if is_jules {
        if args.condition.is_none() {
            return Err(SnuzeError::ConflictingFlags(
                "--condition is required with --jules".to_string(),
            ));
        }
        if args.timeout.is_none() {
            return Err(SnuzeError::ConflictingFlags(
                "--timeout is required with --jules".to_string(),
            ));
        }
    } else if is_signal {
        if args.timeout.is_none() {
            return Err(SnuzeError::ConflictingFlags(
                "--timeout is required with --signal".to_string(),
            ));
        }
    } else if is_herdr_pane {
        if args.watch_for.is_none() {
            return Err(SnuzeError::ConflictingFlags(
                "--watch-for is required with --herdr-pane; valid options are: idle, done, blocked, working".to_string(),
            ));
        }
        if args.timeout.is_none() {
            return Err(SnuzeError::ConflictingFlags(
                "--timeout is required with --herdr-pane".to_string(),
            ));
        }
    }

    // 4. Parse mode-specific sub-options
    let parsed_condition = if is_jules {
        if let Some(ref cond_str) = args.condition {
            Some(parse_jules_condition(cond_str)?)
        } else {
            None
        }
    } else {
        None
    };

    let parsed_watch_statuses = if is_herdr_pane {
        if let Some(ref watch_str) = args.watch_for {
            Some(parse_watch_statuses(watch_str)?)
        } else {
            None
        }
    } else {
        None
    };

    // 5. Parse duration / timeout and poll-interval
    let (after_duration, timeout_duration, poll_interval_duration) = if is_jules {
        let timeout_str = args.timeout.as_deref().unwrap_or_default();
        let timeout_dur = parse_duration(timeout_str)?;
        let poll_interval_dur = if let Some(ref pi) = args.poll_interval {
            parse_duration(pi)?
        } else {
            std::time::Duration::from_secs(30)
        };
        (None, Some(timeout_dur), Some(poll_interval_dur))
    } else if is_signal || is_herdr_pane {
        let timeout_str = args.timeout.as_deref().unwrap_or_default();
        let timeout_dur = parse_duration(timeout_str)?;
        (None, Some(timeout_dur), None)
    } else {
        let after_str = args.after.as_deref().unwrap_or_default();
        let after_dur = parse_duration(after_str)?;
        (Some(after_dur), None, None)
    };

    // 6. Split --jules session IDs
    let session_ids = if is_jules {
        let jules_str = args.jules.as_deref().unwrap_or_default();
        Some(parse_jules_session_ids(jules_str)?)
    } else {
        None
    };

    // Pre-flight watched pane check
    let adapter = RealHerdrAdapter::new()?;
    if is_herdr_pane {
        let watched_pane_id = args.herdr_pane.as_deref().unwrap_or_default();
        if !adapter.pane_exists(watched_pane_id)? {
            return Err(SnuzeError::WatchedPaneNotFound(watched_pane_id.to_string()));
        }
    }

    // Pre-flight delivery pane check
    if !adapter.pane_exists(&args.pane)? {
        return Err(SnuzeError::PaneNotFound(args.pane));
    }

    // Pre-flight Jules check
    if is_jules {
        let jules_adapter = RealJulesAdapter::new()?;
        if let Some(ref ids) = session_ids {
            jules_adapter.check_sessions(ids)?;
        }
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
    let (deadline, trigger, schema_version) = if is_jules {
        let timeout_dur = timeout_duration.unwrap_or(std::time::Duration::from_mins(5));
        let poll_dur = poll_interval_duration.unwrap_or(std::time::Duration::from_secs(30));
        let timeout_at = now + timeout_dur;
        let next_check = now + poll_dur;
        let trigger = Trigger::JulesCondition {
            session_ids: session_ids.unwrap_or_default(),
            condition: parsed_condition.unwrap_or(JulesCondition::AllTerminal),
            poll_interval_secs: poll_dur.as_secs(),
            timeout_at,
            last_known: Vec::new(),
        };
        (next_check, trigger, 2)
    } else if is_signal {
        let timeout_dur = timeout_duration.unwrap_or(std::time::Duration::from_mins(5));
        let timeout_at = now + timeout_dur;
        let trigger = Trigger::ExternalSignal {
            match_key: args.signal.unwrap_or_default(),
            timeout_at,
        };
        (timeout_at, trigger, 3)
    } else if is_herdr_pane {
        let timeout_dur = timeout_duration.unwrap_or(std::time::Duration::from_mins(5));
        let timeout_at = now + timeout_dur;
        let trigger = Trigger::HerdrStatus {
            watched_pane_id: args.herdr_pane.unwrap_or_default(),
            watch_for: parsed_watch_statuses.unwrap_or_default(),
            timeout_at,
            last_known: None,
        };
        (timeout_at, trigger, 3)
    } else {
        let dur = after_duration.unwrap_or(std::time::Duration::from_secs(0));
        (now + dur, Trigger::Deadline, 1)
    };

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
        trigger,
        schema_version,
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

/// Handles signaling an external alarm.
///
/// # Errors
/// Returns `SnuzeError` if daemon is unreachable.
pub fn handle_signal(args: &SignalArgs, state_dir: &Path) -> CliResult {
    if args.dry_run {
        let client = DaemonClient::connect(state_dir)?;
        let pending = client.list_alarms(Some(AlarmStatus::Pending))?;
        let mut matched_alarms = Vec::new();
        for alarm in pending {
            if let Trigger::ExternalSignal { match_key, .. } = &alarm.trigger {
                if match_key == &args.match_key {
                    matched_alarms.push(alarm.id);
                }
            }
        }
        if args.json {
            Ok(serde_json::json!({
                "matched": matched_alarms.len(),
                "alarm_ids": matched_alarms
            }))
        } else {
            Ok(serde_json::Value::String(format!(
                "Would signal {} alarms: {:?}",
                matched_alarms.len(),
                matched_alarms
            )))
        }
    } else {
        Daemon::ensure_running(state_dir)?;
        let client = DaemonClient::connect(state_dir)?;
        let matched_alarms = client.signal_alarm(&args.match_key)?;
        if args.json {
            Ok(serde_json::json!({
                "matched": matched_alarms.len(),
                "alarm_ids": matched_alarms
            }))
        } else {
            Ok(serde_json::Value::String(format!(
                "Signaled {} alarms: {:?}",
                matched_alarms.len(),
                matched_alarms
            )))
        }
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
                Ok(serde_json::Value::String(
                    "Daemon exited cleanly".to_string(),
                ))
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
#[allow(clippy::too_many_lines)]
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
    check_results.insert("state_dir_writable", serde_json::json!(state_dir_writable));

    let mut counts = HashMap::new();
    let mut corrupt_alarms = Vec::new();
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
        corrupt_alarms = store.scan_errors();
    }
    check_results.insert("alarm_counts", serde_json::json!(counts));

    let scan_errors_json: Vec<serde_json::Value> = corrupt_alarms
        .iter()
        .map(|(file, reason)| {
            serde_json::json!({
                "file": file,
                "reason": reason,
            })
        })
        .collect();
    check_results.insert("corrupt_alarms", serde_json::json!(scan_errors_json));

    let failed_count = counts.get("failed").copied().unwrap_or(0);
    let missed_count = counts.get("missed").copied().unwrap_or(0);
    let pending_count = counts.get("pending").copied().unwrap_or(0);

    let herdr_ok = herdr_binary != "missing";

    if !herdr_ok {
        return Err(SnuzeError::HerdrBinaryMissing);
    }

    if !herdr_socket_reachable {
        return Err(SnuzeError::DaemonUnavailable(
            "herdr socket is unreachable".to_string(),
        ));
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
        let mut output = format!(
            "Herdr Binary: {}\n\
             Herdr Socket: {}\n\
             Daemon Running: {}\n\
             State Dir Writable: {}\n\
             Pending Alarms: {}\n\
             Failed Alarms: {}\n\
             Missed Alarms: {}\n\
             Corrupt Alarms: {}",
            herdr_binary,
            if herdr_socket_reachable {
                "reachable"
            } else {
                "unreachable"
            },
            if daemon_running { "running" } else { "stopped" },
            if state_dir_writable { "yes" } else { "no" },
            pending_count,
            failed_count,
            missed_count,
            corrupt_alarms.len()
        );
        if !corrupt_alarms.is_empty() {
            output.push_str("\nCorrupt Alarm Files:");
            for (file, reason) in &corrupt_alarms {
                let _ = write!(output, "\n  - {file}: {reason}");
            }
        }
        Ok(serde_json::Value::String(output))
    }
}

/// Handles event outbox commands.
///
/// # Errors
/// Returns `SnuzeError` if daemon communication fails.
pub fn handle_events(args: &EventsArgs, state_dir: &Path) -> CliResult {
    match args.action {
        EventsAction::Drain { json } => {
            let client = DaemonClient::connect(state_dir)?;
            let events = client.drain_events()?;

            if json {
                Ok(serde_json::to_value(&events).unwrap_or(serde_json::Value::Null))
            } else {
                let mut output = String::new();
                let _ = writeln!(output, "Drained {} events", events.len());
                for event in &events {
                    let _ = writeln!(
                        output,
                        "- [{}] {} {} ({})",
                        event.alarm_id,
                        event.trigger_kind,
                        event.status,
                        event.delivery_target_pane
                    );
                }
                Ok(serde_json::Value::String(output.trim_end().to_string()))
            }
        }
    }
}
