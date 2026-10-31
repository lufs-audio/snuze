//! Unit tests for `Trigger::FileArtifact` (Unit 03).
//!
//! Covers the artifact predicate (existence, modification, and wildcard matching),
//! firing when an independent thread writes the artifact, timeout expiry,
//! cancellation, and the non-busy-loop poll backoff.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::indexing_slicing)]
#![allow(clippy::panic)]
#![allow(clippy::duration_suboptimal_units)]

use snuze::kernel::{
    Alarm, AlarmStatus, DeliveryMode, DeliveryTarget, FileWatchMode, SnuzeError, Trigger,
};
use snuze::scheduler::{
    file_artifact_satisfied, Clock, DeliveryAdapter, DeliveryOutcome, FakeClock, FiredEvent,
    Scheduler, SystemClock,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

#[derive(Clone, Default)]
struct RecordingAdapter {
    deliveries: Arc<Mutex<Vec<(DeliveryTarget, DeliveryMode)>>>,
}

impl RecordingAdapter {
    fn delivered_count(&self) -> usize {
        self.deliveries.lock().unwrap().len()
    }
}

impl DeliveryAdapter for RecordingAdapter {
    fn deliver(
        &self,
        target: &DeliveryTarget,
        delivery: &DeliveryMode,
    ) -> Result<DeliveryOutcome, SnuzeError> {
        self.deliveries
            .lock()
            .unwrap()
            .push((target.clone(), delivery.clone()));
        Ok(DeliveryOutcome::Delivered)
    }
}

fn artifact_alarm(
    id: &str,
    path: &Path,
    mode: FileWatchMode,
    created_at: SystemTime,
    timeout_at: SystemTime,
) -> Alarm {
    Alarm {
        id: id.to_string(),
        target: DeliveryTarget::Wait,
        created_by_pane: None,
        delivery: DeliveryMode::Prompt {
            text: format!("{} ready", path.display()),
        },
        label: None,
        created_at,
        deadline: timeout_at,
        status: AlarmStatus::Pending,
        fired_at: None,
        failure_reason: None,
        trigger: Trigger::FileArtifact {
            path: path.to_path_buf(),
            mode,
            timeout_at,
        },
        schema_version: 5,
    }
}

fn spawn_scheduler(
    clock: Box<dyn Clock>,
    adapter: RecordingAdapter,
) -> (Arc<Scheduler>, Receiver<FiredEvent>) {
    let (tx, rx): (Sender<FiredEvent>, Receiver<FiredEvent>) = channel();
    let scheduler = Arc::new(Scheduler::new(
        Box::new(adapter),
        clock,
        Duration::from_mins(10),
        Box::new(move |event| {
            let _ = tx.send(event);
        }),
    ));
    let bg = Arc::clone(&scheduler);
    thread::spawn(move || bg.run());
    (scheduler, rx)
}

// ---------------------------------------------------------------------------
// Artifact predicate
// ---------------------------------------------------------------------------

#[test]
fn test_file_artifact_exists_predicate() {
    let dir = TempDir::new().unwrap();
    let present = dir.path().join("take.json");
    let absent = dir.path().join("nope.json");
    std::fs::write(&present, b"{}").unwrap();

    assert!(file_artifact_satisfied(
        &present,
        FileWatchMode::Exists,
        UNIX_EPOCH
    ));
    assert!(!file_artifact_satisfied(
        &absent,
        FileWatchMode::Exists,
        UNIX_EPOCH
    ));
    // A directory is a valid artifact contract too.
    assert!(file_artifact_satisfied(
        dir.path(),
        FileWatchMode::Exists,
        UNIX_EPOCH
    ));
}

#[test]
fn test_file_artifact_modified_predicate() {
    let dir = TempDir::new().unwrap();
    let present = dir.path().join("transcript.md");
    std::fs::write(&present, b"hello").unwrap();

    // Anything written after the epoch counts as modified since the epoch.
    assert!(file_artifact_satisfied(
        &present,
        FileWatchMode::Modified,
        UNIX_EPOCH
    ));
    // Nothing can have been modified after a far-future baseline.
    assert!(!file_artifact_satisfied(
        &present,
        FileWatchMode::Modified,
        UNIX_EPOCH + Duration::from_hours(1_000_000)
    ));
    // A missing file is never "modified".
    assert!(!file_artifact_satisfied(
        &dir.path().join("missing.md"),
        FileWatchMode::Modified,
        UNIX_EPOCH
    ));
}

#[test]
fn test_file_artifact_wildcard_matching() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("take-001.json"), b"{}").unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();

    let star = dir.path().join("*.json");
    assert!(file_artifact_satisfied(
        &star,
        FileWatchMode::Exists,
        UNIX_EPOCH
    ));

    let question = dir.path().join("take-???.json");
    assert!(file_artifact_satisfied(
        &question,
        FileWatchMode::Exists,
        UNIX_EPOCH
    ));

    let no_match = dir.path().join("*.wav");
    assert!(!file_artifact_satisfied(
        &no_match,
        FileWatchMode::Exists,
        UNIX_EPOCH
    ));

    let wrong_length = dir.path().join("take-?.json");
    assert!(!file_artifact_satisfied(
        &wrong_length,
        FileWatchMode::Exists,
        UNIX_EPOCH
    ));

    // Wildcards must not leak across directory separators: an exact-prefix
    // non-match in the parent stays a non-match.
    let other_dir = dir.path().join("sub");
    std::fs::create_dir_all(&other_dir).unwrap();
    assert!(!file_artifact_satisfied(
        &dir.path().join("sub/*.json"),
        FileWatchMode::Exists,
        UNIX_EPOCH
    ));
    // ...but the same wildcard inside the subdirectory matches nothing because
    // that directory is empty.
    assert!(!file_artifact_satisfied(
        &other_dir.join("*.json"),
        FileWatchMode::Exists,
        UNIX_EPOCH
    ));
}

// ---------------------------------------------------------------------------
// Firing when an independent thread writes the artifact
// ---------------------------------------------------------------------------

#[test]
fn test_file_artifact_alarm_fires_when_thread_writes_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("take.json");

    let writer_path = path.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        std::fs::write(&writer_path, b"{\"ok\":true}").unwrap();
    });

    let adapter = RecordingAdapter::default();
    let (scheduler, rx) = spawn_scheduler(Box::new(SystemClock::default()), adapter.clone());

    let now = SystemTime::now();
    scheduler.arm(artifact_alarm(
        "artifact-fire",
        &path,
        FileWatchMode::Exists,
        now,
        now + Duration::from_secs(30),
    ));

    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "artifact alarm did not fire after the file appeared"
        );
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(FiredEvent::Fired { id, .. }) => {
                assert_eq!(id, "artifact-fire");
                break;
            }
            Ok(FiredEvent::Progress { .. }) => continue,
            Ok(other) => panic!("unexpected event: {other:?}"),
            Err(_) => continue,
        }
    }

    assert_eq!(adapter.delivered_count(), 1);
}

#[test]
fn test_file_artifact_modified_fires_only_after_a_later_write() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("take.json");
    std::fs::write(&path, b"stale").unwrap();

    let adapter = RecordingAdapter::default();
    let (scheduler, rx) = spawn_scheduler(Box::new(SystemClock::default()), adapter.clone());

    // Baseline is "now": the pre-existing file was written strictly before it.
    let baseline = SystemTime::now();
    scheduler.arm(artifact_alarm(
        "artifact-modified",
        &path,
        FileWatchMode::Modified,
        baseline,
        baseline + Duration::from_secs(30),
    ));

    // First poll must not fire: the stale file predates the baseline.
    match rx.recv_timeout(Duration::from_secs(2)).expect("first poll") {
        FiredEvent::Progress { .. } => {}
        other => panic!("expected Progress before rewrite, got {other:?}"),
    }
    assert_eq!(adapter.delivered_count(), 0);

    // Rewrite the file strictly after the baseline.
    thread::sleep(Duration::from_millis(1100));
    std::fs::write(&path, b"fresh").unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "modified-mode alarm did not fire after the rewrite"
        );
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(FiredEvent::Fired { id, .. }) => {
                assert_eq!(id, "artifact-modified");
                break;
            }
            Ok(FiredEvent::Progress { .. }) => continue,
            Ok(other) => panic!("unexpected event: {other:?}"),
            Err(_) => continue,
        }
    }
    assert_eq!(adapter.delivered_count(), 1);
}

// ---------------------------------------------------------------------------
// Timeout expiry and cancellation
// ---------------------------------------------------------------------------

#[test]
fn test_file_artifact_alarm_times_out_when_never_written() {
    let dir = TempDir::new().unwrap();
    let path: PathBuf = dir.path().join("never.json");

    let base = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let clock = FakeClock::new(base);
    let adapter = RecordingAdapter::default();
    let (scheduler, rx) = spawn_scheduler(Box::new(clock.clone()), adapter.clone());

    scheduler.arm(artifact_alarm(
        "artifact-timeout",
        &path,
        FileWatchMode::Exists,
        base,
        base + Duration::from_hours(1),
    ));

    match rx.recv_timeout(Duration::from_millis(500)).expect("poll") {
        FiredEvent::Progress { .. } => {}
        other => panic!("expected Progress, got {other:?}"),
    }

    clock.advance(Duration::from_hours(1) + Duration::from_secs(1));

    match rx.recv_timeout(Duration::from_millis(500)).expect("event") {
        FiredEvent::Failed { id, reason } => {
            assert_eq!(id, "artifact-timeout");
            assert_eq!(reason, "timeout");
        }
        other => panic!("expected Failed(timeout), got {other:?}"),
    }
    assert_eq!(adapter.delivered_count(), 0);
}

#[test]
fn test_file_artifact_alarm_armed_past_timeout_fails_immediately() {
    let dir = TempDir::new().unwrap();
    let base = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let clock = FakeClock::new(base);
    let adapter = RecordingAdapter::default();
    let (scheduler, rx) = spawn_scheduler(Box::new(clock), adapter);

    scheduler.arm(artifact_alarm(
        "artifact-expired",
        &dir.path().join("x.json"),
        FileWatchMode::Exists,
        base - Duration::from_hours(2),
        base - Duration::from_secs(1),
    ));

    match rx.recv_timeout(Duration::from_millis(500)).expect("event") {
        FiredEvent::Failed { id, reason } => {
            assert_eq!(id, "artifact-expired");
            assert_eq!(reason, "timeout");
        }
        other => panic!("expected Failed(timeout), got {other:?}"),
    }
}

#[test]
fn test_file_artifact_alarm_cancelled_does_not_fire() {
    let dir = TempDir::new().unwrap();
    let base = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let clock = FakeClock::new(base);
    let adapter = RecordingAdapter::default();
    let (scheduler, rx) = spawn_scheduler(Box::new(clock.clone()), adapter.clone());

    let path = dir.path().join("cancel.json");
    scheduler.arm(artifact_alarm(
        "artifact-cancel",
        &path,
        FileWatchMode::Exists,
        base,
        base + Duration::from_hours(1),
    ));
    let _ = rx.recv_timeout(Duration::from_millis(500));

    assert!(scheduler.cancel("artifact-cancel"));

    // Even once the artifact appears, a cancelled alarm must stay silent.
    std::fs::write(&path, b"{}").unwrap();
    clock.advance(Duration::from_hours(2));

    while let Ok(event) = rx.recv_timeout(Duration::from_millis(200)) {
        assert!(
            matches!(event, FiredEvent::Progress { .. }),
            "cancelled alarm produced {event:?}"
        );
    }
    assert_eq!(adapter.delivered_count(), 0);
}

// ---------------------------------------------------------------------------
// Backoff: no busy loop
// ---------------------------------------------------------------------------

#[test]
fn test_file_artifact_poll_interval_backs_off_and_is_capped() {
    let dir = TempDir::new().unwrap();
    let base = UNIX_EPOCH + Duration::from_secs(1_000_000);
    let clock = FakeClock::new(base);
    let adapter = RecordingAdapter::default();
    let (scheduler, rx) = spawn_scheduler(Box::new(clock.clone()), adapter);

    scheduler.arm(artifact_alarm(
        "artifact-backoff",
        &dir.path().join("never.json"),
        FileWatchMode::Exists,
        base,
        base + Duration::from_hours(24),
    ));

    let mut intervals = Vec::new();
    for _ in 0..45 {
        let event = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("expected a Progress poll");
        let FiredEvent::Progress { next_check_at, .. } = event else {
            panic!("expected Progress, got {event:?}");
        };
        let interval = next_check_at
            .duration_since(clock.now())
            .expect("next check must be in the future");
        intervals.push(interval);
        clock.advance(interval);
    }

    assert_eq!(intervals[0], Duration::from_secs(1));
    assert!(
        intervals.last().unwrap() > intervals.first().unwrap(),
        "poll interval must grow: {intervals:?}"
    );
    assert!(
        intervals.iter().all(|d| *d <= Duration::from_secs(30)),
        "poll interval must be capped at 30s: {intervals:?}"
    );
}
