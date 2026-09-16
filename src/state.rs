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
        Some(v) => PathBuf::from(v).join(project.primary()),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join(".cache")
            .join(project.primary()),
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

/// Held across a read-modify-write of the cache, and nothing else. Short
/// enough that blocking is right: the alternative is two of this file's
/// writers interleaving on it.
///
/// It lives here rather than beside the updater because the file is this
/// module's, and announcing an update reads and writes it without ever
/// fetching one.
pub(crate) struct CacheGuard(#[allow(dead_code)] Option<std::fs::File>);

impl CacheGuard {
    pub(crate) fn acquire(cache_file: &Path) -> Self {
        let path = cache_file.with_extension("lock");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .ok();
        // Best effort: a cache that cannot be guarded is still a cache worth
        // writing, and the failure it guards against is a stale banner.
        if let Some(f) = &file {
            let _ = f.lock();
        }
        CacheGuard(file)
    }
}

/// Announce a version that has not been announced yet, once.
///
/// `announce` is handed `(latest, current)` and is called only when the cache
/// says there is something new to say. It returns whether the announcement
/// actually happened; `false` puts the version back on offer so the next check
/// tries again rather than marking it announced forever.
///
/// The guard, the read and the write-back are this crate's because the file is
/// this crate's. A caller doing it for itself is a third writer of a file a
/// check and an update already coordinate over, and the one that takes no
/// lock: its write-back carries the `current` it was handed, so an update
/// finishing in between is undone on paper and the panel goes on offering an
/// update to the version already installed.
pub fn announce_if_new(
    cache_file: &Path,
    announce: impl FnOnce(&str, &str) -> bool,
) -> Result<bool> {
    let Some((latest, current)) = claim_announcement(cache_file)? else {
        return Ok(false);
    };
    // Deliberately outside the lock: a caller may draw something that waits on
    // a person, and the file should not be held for that.
    if announce(&latest, &current) {
        return Ok(true);
    }
    release_announcement(cache_file, &latest)?;
    Ok(false)
}

/// Take the announcement under the lock, so two processes checking at once
/// cannot both decide it is theirs to make.
fn claim_announcement(cache_file: &Path) -> Result<Option<(String, String)>> {
    let _guard = CacheGuard::acquire(cache_file);
    let mut status = read_update_status(cache_file).unwrap_or_default();
    let Some(latest) = status.latest.clone().filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if !status.available || status.notified.as_deref() == Some(latest.as_str()) {
        return Ok(None);
    }
    let current = status.current.clone();
    status.notified = Some(latest.clone());
    write_update_status(cache_file, &status)?;
    Ok(Some((latest, current)))
}

fn release_announcement(cache_file: &Path, latest: &str) -> Result<()> {
    let _guard = CacheGuard::acquire(cache_file);
    let mut status = read_update_status(cache_file).unwrap_or_default();
    // Only if it is still ours: a check may have moved on to a newer version.
    if status.notified.as_deref() == Some(latest) {
        status.notified = None;
        write_update_status(cache_file, &status)?;
    }
    Ok(())
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

    fn announce_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sv-ann-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_version_is_announced_once_and_the_guard_is_written_back() {
        let dir = announce_dir("once");
        let path = dir.join("update.json");
        write_update_status(
            &path,
            &UpdateStatus {
                current: "1.0.0".into(),
                latest: Some("1.1.0".into()),
                available: true,
                ..Default::default()
            },
        )
        .unwrap();

        let mut seen = Vec::new();
        assert!(
            announce_if_new(&path, |latest, current| {
                seen.push(format!("{current}->{latest}"));
                true
            })
            .unwrap()
        );
        assert_eq!(seen, ["1.0.0->1.1.0"]);
        assert_eq!(
            read_update_status(&path).unwrap().notified.as_deref(),
            Some("1.1.0")
        );

        // Second time round there is nothing new to say.
        assert!(!announce_if_new(&path, |_, _| panic!("announced twice")).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The whole reason this is not a caller's write-back: it must not carry a
    /// `current` from before an update that landed meanwhile.
    #[test]
    fn announcing_leaves_every_other_field_as_it_found_them() {
        let dir = announce_dir("fields");
        let path = dir.join("update.json");
        write_update_status(
            &path,
            &UpdateStatus {
                current: "1.0.0".into(),
                latest: Some("1.1.0".into()),
                available: true,
                checked_ms: 1_700_000_000_000,
                notified: None,
            },
        )
        .unwrap();

        // An update finishes while the announcement is being drawn.
        assert!(
            announce_if_new(&path, |_, _| {
                let mut after = read_update_status(&path).unwrap();
                after.current = "1.1.0".into();
                after.available = false;
                write_update_status(&path, &after).unwrap();
                true
            })
            .unwrap()
        );

        let end = read_update_status(&path).unwrap();
        assert_eq!(end.current, "1.1.0", "the update was undone on paper");
        assert!(!end.available, "the panel is still offering an update");
        assert_eq!(end.checked_ms, 1_700_000_000_000);
    }

    /// An announcement that did not happen is not an announcement.
    #[test]
    fn a_refused_announcement_is_offered_again() {
        let dir = announce_dir("refused");
        let path = dir.join("update.json");
        write_update_status(
            &path,
            &UpdateStatus {
                current: "1.0.0".into(),
                latest: Some("1.1.0".into()),
                available: true,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(!announce_if_new(&path, |_, _| false).unwrap());
        assert_eq!(read_update_status(&path).unwrap().notified, None);
        assert!(announce_if_new(&path, |_, _| true).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_is_announced_when_there_is_no_update() {
        let dir = announce_dir("none");
        let path = dir.join("update.json");
        assert!(!announce_if_new(&path, |_, _| panic!("no cache at all")).unwrap());

        write_update_status(
            &path,
            &UpdateStatus {
                current: "1.1.0".into(),
                latest: Some("1.1.0".into()),
                available: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!announce_if_new(&path, |_, _| panic!("already current")).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `XDG_CACHE_HOME` is frequently unset, so the fallback is the usual
    /// path rather than the exception.
    #[test]
    fn the_cache_directory_falls_back_to_the_home_one() {
        let _env = crate::env_lock();
        let before = std::env::var_os("XDG_CACHE_HOME");
        // SAFETY: guarded above, and restored below.
        unsafe { std::env::set_var("XDG_CACHE_HOME", "/xdg") };
        let with = cache_dir(&crate::SAMPLE);
        unsafe { std::env::remove_var("XDG_CACHE_HOME") };
        let without = cache_dir(&crate::SAMPLE);
        // An empty value is not a directory, and must read as unset.
        unsafe { std::env::set_var("XDG_CACHE_HOME", "") };
        let empty = cache_dir(&crate::SAMPLE);
        match before {
            Some(v) => unsafe { std::env::set_var("XDG_CACHE_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CACHE_HOME") },
        }

        assert_eq!(with, std::path::Path::new("/xdg/samplegauge"));
        assert!(
            without.ends_with(".cache/samplegauge"),
            "{}",
            without.display()
        );
        assert_eq!(empty, without, "an empty value is not a directory");
        assert_eq!(
            update_cache_file(&crate::SAMPLE),
            without.join("update.json")
        );
    }

    /// A check that moved on to a newer version owns the guard now, and a
    /// refused announcement for the older one must not clear it.
    #[test]
    fn a_refusal_does_not_clear_a_guard_that_moved_on() {
        let dir = announce_dir("moved-on");
        let path = dir.join("update.json");
        write_update_status(
            &path,
            &UpdateStatus {
                current: "1.0.0".into(),
                latest: Some("1.1.0".into()),
                available: true,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(
            !announce_if_new(&path, |_, _| {
                // A check lands mid-announcement and finds something newer.
                let mut moved = read_update_status(&path).unwrap();
                moved.latest = Some("1.2.0".into());
                moved.notified = Some("1.2.0".into());
                write_update_status(&path, &moved).unwrap();
                false
            })
            .unwrap()
        );

        assert_eq!(
            read_update_status(&path).unwrap().notified.as_deref(),
            Some("1.2.0"),
            "a refusal for 1.1.0 cleared the guard for 1.2.0"
        );
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
