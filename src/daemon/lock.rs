use crate::kernel::SnuzeError;
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// RAII wrapper around exclusive advisory PID file lock for single-instance daemon.
pub struct DaemonLock {
    file: File,
    path: PathBuf,
}

impl DaemonLock {
    /// Acquires an advisory lock on `<state_dir>/daemon.pid`.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` if another live process already holds the lock
    /// or if the PID file cannot be opened/written.
    pub fn acquire(state_dir: &Path) -> Result<Self, SnuzeError> {
        if !state_dir.exists() {
            std::fs::create_dir_all(state_dir).map_err(|e| {
                SnuzeError::DaemonUnavailable(format!("failed to create state dir: {e}"))
            })?;
        }

        let path = state_dir.join("daemon.pid");

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("could not open pid file: {e}")))?;

        // Attempt exclusive advisory lock. If another live process holds it, this returns an error.
        // If a previous daemon crashed without cleaning up, the OS kernel released its lock,
        // so `try_lock_exclusive` succeeds and we overwrite the stale PID.
        if file.try_lock_exclusive().is_err() {
            return Err(SnuzeError::DaemonUnavailable(
                "daemon is already running".to_string(),
            ));
        }

        // Lock acquired; overwrite with current process PID
        file.set_len(0).map_err(|e| {
            SnuzeError::DaemonUnavailable(format!("could not truncate pid file: {e}"))
        })?;
        write!(file, "{}", std::process::id())
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("could not write pid file: {e}")))?;
        file.sync_all()
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("could not sync pid file: {e}")))?;

        Ok(Self { file, path })
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let _ = std::fs::remove_file(&self.path);
    }
}
