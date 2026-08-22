pub mod paths;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::kernel::{Alarm, AlarmStatus, SnuzeError};
pub use paths::resolve_state_dir;

/// Durable alarm store backed by folder-per-entry flat JSON files.
#[derive(Debug, Clone)]
pub struct AlarmStore {
    alarms_dir: PathBuf,
}

impl AlarmStore {
    /// Opens the alarm store at `state_dir/alarms/`, creating the directory if absent.
    ///
    /// # Errors
    ///
    /// Returns `SnuzeError::PersistFailed` if creating the directory fails.
    pub fn open(state_dir: &Path) -> Result<Self, SnuzeError> {
        let alarms_dir = state_dir.join("alarms");
        if !alarms_dir.exists() {
            fs::create_dir_all(&alarms_dir).map_err(|e| {
                SnuzeError::PersistFailed(format!("failed to create alarms directory: {e}"))
            })?;
        }

        Ok(Self { alarms_dir })
    }

    /// Persists an alarm record atomically via write-to-temp-file-then-rename and fsync.
    ///
    /// # Errors
    ///
    /// Returns `SnuzeError::PersistFailed` on any I/O or serialization error.
    pub fn put(&self, alarm: &Alarm) -> Result<(), SnuzeError> {
        let target_path = self.alarms_dir.join(format!("{}.json", alarm.id));
        let temp_path = self
            .alarms_dir
            .join(format!(".{}.tmp.{}", alarm.id, std::process::id()));

        let json_data = serde_json::to_string_pretty(alarm)
            .map_err(|e| SnuzeError::PersistFailed(format!("failed to serialize alarm: {e}")))?;

        let mut file = fs::File::create(&temp_path)
            .map_err(|e| SnuzeError::PersistFailed(format!("failed to create temp file: {e}")))?;

        if let Err(e) = file.write_all(json_data.as_bytes()) {
            let _ = fs::remove_file(&temp_path);
            return Err(SnuzeError::PersistFailed(format!(
                "failed to write temp file: {e}"
            )));
        }

        if let Err(e) = file.sync_all() {
            let _ = fs::remove_file(&temp_path);
            return Err(SnuzeError::PersistFailed(format!(
                "failed to sync temp file: {e}"
            )));
        }

        drop(file);

        if let Err(e) = fs::rename(&temp_path, &target_path) {
            let _ = fs::remove_file(&temp_path);
            return Err(SnuzeError::PersistFailed(format!(
                "failed to rename temp file to target file: {e}"
            )));
        }

        if let Ok(dir) = fs::File::open(&self.alarms_dir) {
            let _ = dir.sync_all();
        }

        Ok(())
    }

    /// Reads and deserializes a single alarm by ID.
    ///
    /// # Errors
    ///
    /// Returns `SnuzeError::AlarmNotFound` if the alarm file does not exist,
    /// or `SnuzeError::PersistFailed` if reading or deserialization fails.
    pub fn get(&self, id: &str) -> Result<Alarm, SnuzeError> {
        let target_path = self.alarms_dir.join(format!("{id}.json"));
        if !target_path.exists() {
            return Err(SnuzeError::AlarmNotFound(id.to_string()));
        }

        let content = fs::read_to_string(&target_path).map_err(|e| {
            SnuzeError::PersistFailed(format!("failed to read alarm file '{id}.json': {e}"))
        })?;

        serde_json::from_str(&content).map_err(|e| {
            SnuzeError::PersistFailed(format!("failed to deserialize alarm '{id}.json': {e}"))
        })
    }

    /// Scans the store and returns all alarms matching `status_filter`, sorted by `deadline` ascending.
    ///
    /// Individual corrupted or unreadable alarm files are skipped without failing the batch.
    ///
    /// # Errors
    ///
    /// Returns `SnuzeError::PersistFailed` if reading the directory fails.
    pub fn list(&self, status_filter: Option<AlarmStatus>) -> Result<Vec<Alarm>, SnuzeError> {
        let entries = fs::read_dir(&self.alarms_dir).map_err(|e| {
            SnuzeError::PersistFailed(format!("failed to read alarms directory: {e}"))
        })?;

        let mut alarms = Vec::new();
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            // Only parse legitimate .json files, skip hidden or temp files
            if file_name.starts_with('.')
                || !path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            {
                continue;
            }

            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };

            let Ok(alarm) = serde_json::from_str::<Alarm>(&content) else {
                continue;
            };

            if let Some(filter) = status_filter {
                if alarm.status != filter {
                    continue;
                }
            }

            alarms.push(alarm);
        }

        alarms.sort_by_key(|a| a.deadline);
        Ok(alarms)
    }

    /// Deletes an alarm file from the store.
    ///
    /// # Errors
    ///
    /// Returns `SnuzeError::AlarmNotFound` if the alarm file is absent,
    /// or `SnuzeError::PersistFailed` if deletion fails.
    pub fn delete(&self, id: &str) -> Result<(), SnuzeError> {
        let target_path = self.alarms_dir.join(format!("{id}.json"));
        if !target_path.exists() {
            return Err(SnuzeError::AlarmNotFound(id.to_string()));
        }

        fs::remove_file(&target_path).map_err(|e| {
            SnuzeError::PersistFailed(format!("failed to delete alarm '{id}.json': {e}"))
        })?;

        if let Ok(dir) = fs::File::open(&self.alarms_dir) {
            let _ = dir.sync_all();
        }

        Ok(())
    }

    /// Scans the store directory for corrupted or unparseable `.json` alarm files.
    /// Returns a list of `(filename, reason)` pairs sorted by filename.
    #[must_use]
    pub fn scan_errors(&self) -> Vec<(String, String)> {
        let mut errors = Vec::new();
        let Ok(entries) = fs::read_dir(&self.alarms_dir) else {
            return errors;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            if file_name.starts_with('.')
                || !path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            {
                continue;
            }

            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    errors.push((file_name.to_string(), format!("failed to read file: {e}")));
                    continue;
                }
            };

            if let Err(e) = serde_json::from_str::<Alarm>(&content) {
                errors.push((file_name.to_string(), format!("failed to deserialize: {e}")));
            }
        }

        errors.sort_by(|a, b| a.0.cmp(&b.0));
        errors
    }
}
