//! GitHub-release auto-updater.
//!
//! Who owns the copy on disk used to decide whether we may replace it: a
//! store-managed install has its own updater, and racing it leaves the store's
//! registry describing a version that is no longer there. TailGauge is in
//! neither store - not the KDE Store, not extensions.gnome.org - so every copy
//! is ours, and the refusal the shell helper carried is gone.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use self_update::backends::github::ReleaseList;

use crate::Project;
use crate::frontend::{self, Frontend};
use crate::state::{self, UpdateStatus};

/// Six hours: long enough that a panel polling this costs nothing, short
/// enough that a release lands the same day. The check itself is always live;
/// this is the caller's answer to how stale a banner may be.
pub const CACHE_TTL_MS: i64 = 6 * 60 * 60 * 1000;

/// Distinguishes the binary archive from the other assets a release carries.
///
/// Public because a consumer's own tests are what check that its release
/// workflow publishes an asset this will go looking for. Getting that wrong
/// produces a release nothing can update from, with green CI.
pub const ARCHIVE_SUFFIX: &str = ".tar.gz";

// ---------------------------------------------------------------------------
// checking
// ---------------------------------------------------------------------------

/// Query GitHub, recompute availability, and persist the cached status. The
/// `notified` guard is preserved across calls.
pub fn check(project: &Project, cache_file: &Path) -> Result<UpdateStatus> {
    let current = project.version.to_string();
    let mut status = state::read_update_status(cache_file).unwrap_or_default();
    status.current = current.clone();
    status.checked_ms = state::now_ms();

    let release = latest_release(project)?;
    let latest = release.version.clone();
    status.available = version_gt(&latest, &current);
    status.latest = Some(latest);

    state::write_update_status(cache_file, &status)?;
    Ok(status)
}

/// The cached answer while it is fresh, and a live [`check`] otherwise. A panel
/// polls this, so the round trip to GitHub is meant to be the exception.
pub fn check_cached(project: &Project, cache_file: &Path, force: bool) -> Result<UpdateStatus> {
    if !force
        && let Some(cached) = state::read_update_status(cache_file)
        && cached.latest.as_deref().is_some_and(|v| !v.is_empty())
        && state::now_ms().saturating_sub(cached.checked_ms) < CACHE_TTL_MS
    {
        return Ok(cached);
    }
    check(project, cache_file)
}

// ---------------------------------------------------------------------------
// releases
// ---------------------------------------------------------------------------

/// Substring the release asset name must contain for the running platform.
///
/// Public for the same reason as [`ARCHIVE_SUFFIX`].
/// Matches the release workflow's `<binary>-<tag>-<target>.tar.gz` naming.
pub fn arch_target() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("linux-x86_64"),
        "aarch64" | "arm64" => Ok("linux-aarch64"),
        other => bail!("unsupported arch: {other}"),
    }
}

/// True if dotted version `a` is greater than `b`. A leading `v` and any
/// pre-release suffix are ignored.
pub fn version_gt(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> [u64; 3] {
        let v = v.trim().trim_start_matches(['v', 'V']);
        let core = v.split(['-', '+']).next().unwrap_or(v);
        let mut out = [0u64; 3];
        for (i, seg) in core.split('.').take(3).enumerate() {
            out[i] = seg.parse().unwrap_or(0);
        }
        out
    }
    parts(a) > parts(b)
}

/// The newest release carrying an asset for the running platform.
fn latest_release(project: &Project) -> Result<self_update::update::Release> {
    let (owner, name) = project.owner_and_name();
    let target = arch_target()?;
    let releases = ReleaseList::configure()
        .repo_owner(&owner)
        .repo_name(&name)
        .build()?
        .fetch()
        .context("could not reach GitHub to check for updates")?;
    releases
        .into_iter()
        .find(|r| r.asset_for(target, Some(ARCHIVE_SUFFIX)).is_some())
        .ok_or_else(|| anyhow!("no release with a {target} asset found"))
}

fn release_named(project: &Project, version: &str) -> Result<self_update::update::Release> {
    let (owner, name) = project.owner_and_name();
    let releases = ReleaseList::configure()
        .repo_owner(&owner)
        .repo_name(&name)
        .build()?
        .fetch()
        .context("could not reach GitHub to fetch releases")?;
    let wanted = version.trim_start_matches('v');
    releases
        .into_iter()
        .find(|r| r.version.trim_start_matches('v') == wanted)
        .ok_or_else(|| anyhow!("no release v{wanted} to install frontends from"))
}

// ---------------------------------------------------------------------------
// applying
// ---------------------------------------------------------------------------

/// Exclusive lock, so an update fired from the panel and one fired from a
/// terminal cannot race on the shared staging directory.
///
/// The lock is the kernel's, held on an open descriptor, and not the mere
/// existence of a file. A lock that means "locked" while the file exists
/// outlives the process that took it: kill an update mid-download and every
/// later one is refused until somebody deletes the file by hand. Closing the
/// descriptor releases this one, and a process that dies closes its
/// descriptors.
struct UpdateLock(#[allow(dead_code)] std::fs::File);

impl UpdateLock {
    fn acquire(project: &Project, install_dir: &Path) -> Result<Self> {
        // Per project: two of these can share an install directory, and one
        // updating must not refuse the other.
        let path = install_dir.join(format!(".{}-update.lock", project.binary));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("cannot open the update lock {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(UpdateLock(file)),
            Err(std::fs::TryLockError::WouldBlock) => {
                bail!("an update is already running")
            }
            Err(std::fs::TryLockError::Error(e)) => {
                Err(e).context("failed to acquire the update lock")
            }
        }
    }
}

/// What an update did to a non-binary frontend, for the caller to report.
#[derive(Debug, Clone)]
pub struct FrontendOutcome {
    pub id: &'static str,
    pub label: &'static str,
    /// The version now on disk, read back from the installed copy rather than
    /// assumed from the release.
    pub version: Option<String>,
    pub restart_hint: &'static str,
    pub needs_session_restart: bool,
    /// Set when the install failed; the binary is already replaced by then, so
    /// this is reported rather than propagated.
    pub error: Option<String>,
}

/// Result of a successful [`apply`]: the version installed, plus what happened
/// to each non-binary frontend that was already present.
pub struct Applied {
    pub version: String,
    pub frontends: Vec<FrontendOutcome>,
}

/// Download the platform archive and replace the installed binary. Returns the
/// version installed - unchanged when already current, so a same-version run
/// never clobbers.
pub fn apply(project: &Project, cache_file: &Path) -> Result<Applied> {
    let target = arch_target()?;
    let release = latest_release(project)?;
    let current = project.version;
    if !version_gt(&release.version, current) {
        return Ok(Applied {
            version: current.to_string(),
            frontends: Vec::new(),
        });
    }

    let asset = release
        .asset_for(target, Some(ARCHIVE_SUFFIX))
        .ok_or_else(|| anyhow!("release {} has no {target} asset", release.version))?;

    let install_dir = install_dir()?;

    // Held for the whole download/extract/replace, so a second invocation
    // fails fast instead of corrupting the staging directory.
    let _lock = UpdateLock::acquire(project, &install_dir)?;

    // Staged inside the install directory so the final move is a rename on the
    // same filesystem.
    let tmp = install_dir.join(format!(".{}-update.tmp", project.binary));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)
        .with_context(|| format!("cannot create staging dir {}", tmp.display()))?;

    // Only what is already installed: this refreshes an existing frontend, it
    // does not decide that a machine should grow a GNOME extension.
    let present = frontend::installed(project);

    let result = (|| -> Result<Vec<FrontendOutcome>> {
        fetch_into(&tmp, &asset.name, &asset.download_url)?;

        // `is_file`, not `exists`: `Move::to_dest` renames without checking
        // what it is moving, so a directory of that name in the archive would
        // land on the installed binary's path and become what every alias
        // points at.
        let staged_binary = tmp.join(project.binary);
        if !staged_binary.is_file() {
            bail!(
                "release archive has no {} - refusing a partial update",
                project.binary
            );
        }

        let dest = install_dir.join(project.binary);
        // Move-with-temp so the running binary is replaced safely: the old
        // inode stays live for this process.
        self_update::Move::from_source(&staged_binary)
            .replace_using_temp(&tmp.join("tailgauge.old"))
            .to_dest(&dest)
            .with_context(|| format!("failed to replace {}", dest.display()))?;
        if let Ok(meta) = std::fs::metadata(&dest) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&dest, perms);
        }

        refresh_aliases(project, &install_dir);

        // An archive predating the frontend payloads carries none, and every
        // install would fail with the same "payload not found". Say nothing
        // rather than reporting a failure per frontend for an old release.
        Ok(if present.iter().any(|f| f.payload_in(&tmp).is_some()) {
            install_frontends_from(&tmp, &present)
        } else {
            Vec::new()
        })
    })();

    let _ = std::fs::remove_dir_all(&tmp);
    let frontends = result?;

    // Refresh the cached status so the panel drops the update banner.
    let mut status = state::read_update_status(cache_file).unwrap_or_default();
    status.current = release.version.clone();
    status.latest = Some(release.version.clone());
    status.available = false;
    status.notified = None;
    status.checked_ms = state::now_ms();
    let _ = state::write_update_status(cache_file, &status);

    Ok(Applied {
        version: release.version,
        frontends,
    })
}

/// Download the release matching `version` and install one frontend from it,
/// whether or not it is already present. This is the "switched desktops" path:
/// the payload always comes from the release the running binary belongs to, so
/// the frontend cannot land out of step with it.
pub fn install_frontends(
    project: &Project,
    targets: &[&'static Frontend],
    version: &str,
) -> Result<Vec<FrontendOutcome>> {
    let release = release_named(project, version)?;
    let asset = release
        .asset_for(arch_target()?, Some(ARCHIVE_SUFFIX))
        .ok_or_else(|| anyhow!("release {} has no asset for this platform", release.version))?;

    let install_dir = install_dir()?;
    // One lock, one download, one extraction for the whole set: installing
    // three frontends must not fetch the archive three times.
    let _lock = UpdateLock::acquire(project, &install_dir)?;

    let tmp = install_dir.join(format!(".{}-frontend.tmp", project.binary));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)
        .with_context(|| format!("cannot create staging dir {}", tmp.display()))?;

    let result = (|| -> Result<Vec<FrontendOutcome>> {
        fetch_into(&tmp, &asset.name, &asset.download_url)?;
        if !targets.iter().any(|t| t.payload_in(&tmp).is_some()) {
            bail!(
                "release v{} ships no frontend payloads",
                version.trim_start_matches('v')
            );
        }
        // Per-frontend failures are collected rather than propagated, so one
        // unwritable destination does not skip the rest of the set.
        Ok(install_frontends_from(&tmp, targets))
    })();

    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn install_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("cannot resolve current executable")?;
    Ok(exe
        .parent()
        .ok_or_else(|| anyhow!("cannot resolve install directory"))?
        .to_path_buf())
}

fn fetch_into(tmp: &Path, name: &str, url: &str) -> Result<()> {
    let archive = tmp.join(name);
    let file = std::fs::File::create(&archive)
        .with_context(|| format!("cannot create {}", archive.display()))?;
    // GitHub's asset `url` is the API endpoint, which streams the binary only
    // when `Accept: application/octet-stream` is set - otherwise it returns the
    // asset's JSON metadata.
    self_update::Download::from_url(url)
        .set_header(
            http::header::ACCEPT,
            http::HeaderValue::from_static("application/octet-stream"),
        )
        .show_progress(true)
        .download_to(file)
        .context("download failed")?;

    self_update::Extract::from_source(&archive)
        .archive(self_update::ArchiveKind::Tar(Some(
            self_update::Compression::Gz,
        )))
        .extract_into(tmp)
        .context("extract failed")
}

fn install_frontends_from(
    source_root: &Path,
    targets: &[&'static Frontend],
) -> Vec<FrontendOutcome> {
    targets
        .iter()
        .map(|f| match f.install_from(source_root) {
            Ok(_) => FrontendOutcome {
                id: f.id,
                label: f.label,
                version: f.installed_version(),
                restart_hint: f.restart.hint(),
                needs_session_restart: f.restart.needs_session_restart(),
                error: None,
            },
            Err(e) => FrontendOutcome {
                id: f.id,
                label: f.label,
                version: None,
                restart_hint: f.restart.hint(),
                needs_session_restart: f.restart.needs_session_restart(),
                error: Some(format!("{e:#}")),
            },
        })
        .collect()
}

/// Point every old helper name at the binary, replacing whatever is there - on
/// an upgrade from the shell helpers that is a real script, and leaving it
/// would let a stale copy answer for `tailgauge-ctl` forever.
pub fn refresh_aliases(project: &Project, install_dir: &Path) {
    // Never trade a working helper for a link to nothing.
    if !install_dir.join(project.binary).is_file() {
        return;
    }
    for alias in project.aliases {
        let path = install_dir.join(alias);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink())
            && std::fs::read_link(&path).is_ok_and(|t| t == Path::new(project.binary))
        {
            continue;
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::os::unix::fs::symlink(project.binary, &path);
    }
    // Names an older layout installed that this one does not write. Left in
    // place they answer for themselves forever: whatever a key binding or a
    // unit file points at is what runs, and the binary does not answer to
    // these, so the copy on disk is the only thing that can.
    for stale in project.legacy {
        let _ = std::fs::remove_file(install_dir.join(stale));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SAMPLE;

    const PROJECT: &Project = &SAMPLE;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tg-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_newer_version_is_newer_however_it_is_spelled() {
        assert!(version_gt("0.5.0", "0.4.9"));
        assert!(version_gt("v1.0.0", "0.31.0"));
        assert!(version_gt("0.10.0", "0.9.0"), "0.10 is not 0.1");
        assert!(!version_gt("0.4.0", "0.4.0"));
        assert!(!version_gt("0.4.0", "0.5.0"));
        assert!(
            !version_gt("0.5.0-rc1", "0.5.0"),
            "a pre-release is the release"
        );
    }

    #[test]
    fn every_old_helper_name_becomes_a_link_to_the_binary() {
        let dir = scratch("aliases");
        std::fs::write(dir.join(SAMPLE.binary), b"binary").unwrap();
        refresh_aliases(PROJECT, &dir);

        for alias in SAMPLE.aliases {
            let path = dir.join(alias);
            assert_eq!(
                std::fs::read_link(&path).unwrap(),
                Path::new(SAMPLE.binary),
                "{alias}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"binary", "{alias} resolves");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_upgrade_from_the_shell_helpers_replaces_the_scripts() {
        // What 0.4.0 leaves behind: real bash scripts. Left in place they would
        // answer for their names forever.
        let dir = scratch("stale");
        std::fs::write(dir.join(SAMPLE.binary), b"new").unwrap();
        std::fs::write(dir.join(SAMPLE.aliases[0]), b"#!/bin/bash\n").unwrap();
        refresh_aliases(PROJECT, &dir);

        let alias = dir.join(SAMPLE.aliases[0]);
        assert!(
            std::fs::symlink_metadata(&alias)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&alias).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_binary_leaves_the_helpers_alone() {
        // The half-extracted archive case: replacing a working helper with a
        // link to a file that is not there is worse than doing nothing.
        let dir = scratch("dangling");
        std::fs::write(dir.join(SAMPLE.aliases[0]), b"#!/bin/bash\n").unwrap();
        refresh_aliases(PROJECT, &dir);

        let alias = dir.join(SAMPLE.aliases[0]);
        assert!(
            !std::fs::symlink_metadata(&alias)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&alias).unwrap(), b"#!/bin/bash\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refreshing_an_existing_set_of_aliases_is_a_no_op() {
        let dir = scratch("idempotent");
        std::fs::write(dir.join(SAMPLE.binary), b"binary").unwrap();
        refresh_aliases(PROJECT, &dir);
        refresh_aliases(PROJECT, &dir);
        assert_eq!(
            std::fs::read_link(dir.join(SAMPLE.aliases[0])).unwrap(),
            Path::new(SAMPLE.binary)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_the_new_layout_does_not_write_is_taken_away() {
        let dir = scratch("legacy");
        std::fs::write(dir.join(SAMPLE.binary), b"binary").unwrap();
        let stale = dir.join(SAMPLE.legacy[0]);
        std::fs::write(&stale, b"#!/bin/bash\n").unwrap();

        refresh_aliases(PROJECT, &dir);
        assert!(
            !stale.exists(),
            "a helper nothing writes any more still answers for its own name"
        );
        // And the aliases it does write are still there.
        assert_eq!(
            std::fs::read_link(dir.join(SAMPLE.aliases[0])).unwrap(),
            Path::new(SAMPLE.binary)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_is_taken_away_when_there_is_no_binary_to_replace_it() {
        // Half an install is worse than the old one: without the binary these
        // names are all that works.
        let dir = scratch("legacy-no-binary");
        let stale = dir.join(SAMPLE.legacy[0]);
        std::fs::write(&stale, b"#!/bin/bash\n").unwrap();

        refresh_aliases(PROJECT, &dir);
        assert!(stale.exists(), "it was the only thing left that ran");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_lock_is_exclusive_and_released_on_the_way_out() {
        let dir = scratch("lock");
        {
            let _held = UpdateLock::acquire(PROJECT, &dir).expect("first");
            assert!(
                UpdateLock::acquire(PROJECT, &dir).is_err(),
                "two updates must not run"
            );
        }
        UpdateLock::acquire(PROJECT, &dir).expect("released on drop");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_lock_file_left_by_a_dead_process_does_not_block_the_next_update() {
        // The whole reason the lock is the kernel's. An update killed
        // mid-download leaves the file behind, and when existence was the lock
        // every later update was refused until somebody deleted it by hand.
        let dir = scratch("stale-lock");
        let path = dir.join(format!(".{}-update.lock", PROJECT.binary));
        std::fs::write(&path, b"").unwrap();

        UpdateLock::acquire(PROJECT, &dir).expect("a file nobody holds is not a lock");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_projects_sharing_an_install_directory_lock_separately() {
        // ~/.local/bin holds both of them, and one updating must not refuse
        // the other.
        let dir = scratch("two-projects");
        let other = Project {
            binary: "othergauge",
            ..SAMPLE
        };
        let _held = UpdateLock::acquire(PROJECT, &dir).expect("first");
        UpdateLock::acquire(&other, &dir).expect("a different project is not this one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fresh_cache_answers_without_reaching_github() {
        // The panel polls this. A check that always went out would hit the
        // unauthenticated rate limit on a machine that restarts its shell.
        let dir = scratch("cache");
        let cache = dir.join("update.json");
        state::write_update_status(
            &cache,
            &UpdateStatus {
                current: SAMPLE.version.into(),
                latest: Some("9.9.9".into()),
                available: true,
                checked_ms: state::now_ms(),
                notified: None,
            },
        )
        .unwrap();

        let status = check_cached(PROJECT, &cache, false).expect("served from cache");
        assert_eq!(status.latest.as_deref(), Some("9.9.9"));
        assert!(status.available);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_or_empty_cache_is_not_served() {
        let dir = scratch("stale-cache");
        let cache = dir.join("update.json");

        // Older than the TTL.
        state::write_update_status(
            &cache,
            &UpdateStatus {
                latest: Some("9.9.9".into()),
                checked_ms: state::now_ms() - CACHE_TTL_MS - 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            !cache_is_fresh(&cache),
            "a cache past the TTL must be refetched"
        );

        // Checked just now, but carrying no answer.
        state::write_update_status(
            &cache,
            &UpdateStatus {
                latest: None,
                checked_ms: state::now_ms(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            !cache_is_fresh(&cache),
            "a check that failed is not an answer"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The guard `check_cached` applies, without the network call behind it.
    fn cache_is_fresh(cache: &Path) -> bool {
        state::read_update_status(cache).is_some_and(|c| {
            c.latest.as_deref().is_some_and(|v| !v.is_empty())
                && state::now_ms().saturating_sub(c.checked_ms) < CACHE_TTL_MS
        })
    }
}
