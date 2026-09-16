//! The one file a project writes for itself: what the last update check
//! found, so a panel drawing itself never waits on GitHub.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::Project;

/// What the last update check found, so a panel opened a minute later does not
/// pay for another round trip to GitHub.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateStatus {
    /// Currently-installed version (no leading `v`).
    #[serde(default)]
    pub current: String,
    /// Latest release seen on GitHub, if a check succeeded.
    #[serde(default)]
    pub latest: Option<String>,
    /// True when `latest` is newer than `current`.
    #[serde(default)]
    pub available: bool,
    /// Unix ms of the last successful check.
    #[serde(default)]
    pub checked_ms: i64,
    /// Version a desktop notification was last fired for, so an available
    /// update is announced once rather than on every check.
    #[serde(default)]
    pub notified: Option<String>,
}

pub fn cache_dir(project: &Project) -> PathBuf {
    match std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v).join(project.binary),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join(".cache")
            .join(project.binary),
    }
}

pub fn update_cache_file(project: &Project) -> PathBuf {
    cache_dir(project).join("update.json")
}

pub fn read_update_status(path: &Path) -> Option<UpdateStatus> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Written through a neighbour and renamed: a panel reading this file while it
/// is rewritten must never see half of it.
pub fn write_update_status(path: &Path, status: &UpdateStatus) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let staged = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&staged, serde_json::to_vec_pretty(status)?)
        .with_context(|| format!("cannot write {}", staged.display()))?;
    std::fs::rename(&staged, path).with_context(|| format!("cannot replace {}", path.display()))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_survives_a_round_trip_and_a_missing_file() {
        let dir = std::env::temp_dir().join(format!("tg-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("update.json");

        assert!(read_update_status(&path).is_none());

        let status = UpdateStatus {
            current: "0.5.0".into(),
            latest: Some("0.6.0".into()),
            available: true,
            checked_ms: 1_700_000_000_000,
            notified: None,
        };
        write_update_status(&path, &status).unwrap();

        let back = read_update_status(&path).unwrap();
        assert_eq!(back.current, "0.5.0");
        assert_eq!(back.latest.as_deref(), Some("0.6.0"));
        assert!(back.available);

        // Nothing is left beside it: a stray staging file in the cache
        // directory is one a later read could pick up.
        let strays: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "update.json")
            .collect();
        assert!(strays.is_empty(), "left behind: {strays:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_written_by_an_older_build_still_reads() {
        let dir = std::env::temp_dir().join(format!("tg-state-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("update.json");
        std::fs::write(&path, r#"{"latest":"0.4.0"}"#).unwrap();

        let back = read_update_status(&path).unwrap();
        assert_eq!(back.latest.as_deref(), Some("0.4.0"));
        assert_eq!(back.current, "");
        assert!(!back.available);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
