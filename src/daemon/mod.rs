pub mod client;
pub mod lock;
pub mod protocol;
pub mod server;

use crate::kernel::SnuzeError;
use std::path::Path;

pub use client::DaemonClient;
pub use server::Daemon;

impl Daemon {
    /// Client-side: send a graceful shutdown request to a running daemon.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` if the shutdown request fails.
    pub fn stop(state_dir: &Path) -> Result<(), SnuzeError> {
        if let Ok(client) = DaemonClient::connect(state_dir) {
            let _ = client.send_request(&protocol::Request::Shutdown);
        }
        Ok(())
    }

    /// Client-side: true if a daemon is listening and responds to Ping.
    #[must_use]
    pub fn is_running(state_dir: &Path) -> bool {
        if let Ok(client) = DaemonClient::connect(state_dir) {
            client.ping().is_ok()
        } else {
            false
        }
    }

    /// Client-side: ensure a daemon is running, spawning one if not,
    /// with bounded retry. Used by `snuze set`'s lazy-autostart path.
    ///
    /// # Errors
    /// Returns `SnuzeError::DaemonUnavailable` if daemon cannot be spawned or reached within 2 seconds.
    pub fn ensure_running(state_dir: &Path) -> Result<(), SnuzeError> {
        if Self::is_running(state_dir) {
            return Ok(());
        }

        // Spawn detached daemon process
        let exe = std::env::current_exe()
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("could not find current exe: {e}")))?;

        let mut cmd = std::process::Command::new(exe);
        cmd.arg("daemon")
            .arg("start")
            .arg("--foreground")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null());

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }

        cmd.spawn()
            .map_err(|e| SnuzeError::DaemonUnavailable(format!("failed to spawn daemon: {e}")))?;

        // Bounded retry up to 2 seconds
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(2) {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if Self::is_running(state_dir) {
                return Ok(());
            }
        }

        Err(SnuzeError::DaemonUnavailable(
            "daemon failed to start or respond within 2s timeout".to_string(),
        ))
    }
}
