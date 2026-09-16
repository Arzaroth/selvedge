//! GitHub-release auto-updater.
//!
//! Who owns the copy on disk used to decide whether we may replace it: a
//! store-managed install has its own updater, and racing it leaves the store's
//! registry describing a version that is no longer there. TailGauge is in
//! neither store - not the KDE Store, not extensions.gnome.org - so every copy
//! is ours, and the refusal the shell helper carried is gone.

#[cfg(unix)]
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
///
/// Windows releases ship a zip, because that is what a Windows user can open
/// without installing anything. It has to agree with [`archive_kind`], or the
/// asset that is downloaded is handed to the wrong extractor.
#[cfg(windows)]
pub const ARCHIVE_SUFFIX: &str = ".zip";
#[cfg(not(windows))]
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
/// Matches the release workflow's `<binary>-<tag>-<target><suffix>` naming.
///
/// The operating system is part of it, not just the architecture: an x86_64
/// answer that named only the architecture matched the Linux asset on Windows,
/// and the update downloaded a tarball of ELF binaries.
#[cfg(windows)]
pub fn arch_target() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("windows-x86_64"),
        other => bail!("unsupported arch: {other}"),
    }
}

#[cfg(not(windows))]
pub fn arch_target() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("linux-x86_64"),
        "aarch64" | "arm64" => Ok("linux-aarch64"),
        other => bail!("unsupported arch: {other}"),
    }
}

/// The extractor for the asset [`ARCHIVE_SUFFIX`] asks for.
fn archive_kind() -> self_update::ArchiveKind {
    #[cfg(windows)]
    {
        self_update::ArchiveKind::Zip
    }
    #[cfg(not(windows))]
    {
        self_update::ArchiveKind::Tar(Some(self_update::Compression::Gz))
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
        let path = install_dir.join(format!(".{}-update.lock", project.primary()));
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
    /// Windows, MSI installs only: the upgrade was handed to `msiexec` and is
    /// running now. Nothing has been replaced yet, and the caller must exit
    /// promptly, because one of the files the installer is about to replace is
    /// the executable running this code.
    pub installer_launched: bool,
}

/// Download the platform archive and replace the installed binary. Returns the
/// version installed - unchanged when already current, so a same-version run
/// never clobbers.
pub fn apply(project: &Project, cache_file: &Path) -> Result<Applied> {
    let release = latest_release(project)?;
    let current = project.version;
    if !version_gt(&release.version, current) {
        return Ok(Applied {
            installer_launched: false,
            version: current.to_string(),
            frontends: Vec::new(),
        });
    }

    // Where an MSI owns what is on disk, it is the thing that upgrades it.
    // Replacing the files underneath would leave Windows describing a version
    // that is no longer installed, and a later package comparing against it.
    #[cfg(windows)]
    if project
        .msi_marker_key
        .is_some_and(|key| msi_product_code(key).is_some())
    {
        return msi_upgrade(&release);
    }

    let target = arch_target()?;

    let asset = release
        .asset_for(target, Some(ARCHIVE_SUFFIX))
        .ok_or_else(|| anyhow!("release {} has no {target} asset", release.version))?;

    let install_dir = install_dir()?;

    // Held for the whole download/extract/replace, so a second invocation
    // fails fast instead of corrupting the staging directory.
    let _lock = UpdateLock::acquire(project, &install_dir)?;

    // Staged inside the install directory so the final move is a rename on the
    // same filesystem.
    let tmp = install_dir.join(format!(".{}-update.tmp", project.primary()));
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
        // `is_file`, not `exists`: the move renames without looking at what it
        // is moving, so a directory of that name in the archive would pass,
        // land on the installed binary's path, and become what the aliases
        // point at.
        let staged_primary = tmp.join(project.primary());
        if !staged_primary.is_file() {
            bail!(
                "release archive has no {} - refusing a partial update",
                project.primary()
            );
        }

        for binary in project.binaries {
            let src = tmp.join(binary);
            // An archive from before a binary existed carries none of it.
            if !src.is_file() {
                continue;
            }
            let dest = install_dir.join(binary);
            // Move-with-temp so a running binary is replaced safely: on unix
            // the old inode stays live for this process, and on Windows the
            // locked file is renamed aside rather than deleted in place.
            self_update::Move::from_source(&src)
                .replace_using_temp(&tmp.join(format!("{binary}.old")))
                .to_dest(&dest)
                .with_context(|| format!("failed to replace {}", dest.display()))?;
            #[cfg(unix)]
            if let Ok(meta) = std::fs::metadata(&dest) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(&dest, perms);
            }
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
        installer_launched: false,
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

    let tmp = install_dir.join(format!(".{}-frontend.tmp", project.primary()));
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
        .archive(archive_kind())
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
    if !install_dir.join(project.primary()).is_file() {
        return;
    }
    // An alias is a symlink, so there are none to refresh where there are no
    // symlinks. A Windows project ships its extra names as real executables in
    // the archive, which the replace loop above has already written.
    #[cfg(unix)]
    for alias in project.aliases {
        let path = install_dir.join(alias);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink())
            && std::fs::read_link(&path).is_ok_and(|t| t == Path::new(project.primary()))
        {
            continue;
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::os::unix::fs::symlink(project.primary(), &path);
    }
    // Names an older layout installed that this one does not write. Left in
    // place they answer for themselves forever: whatever a key binding or a
    // unit file points at is what runs, and the binary does not answer to
    // these, so the copy on disk is the only thing that can.
    for stale in project.legacy {
        let _ = std::fs::remove_file(install_dir.join(stale));
    }
}

// ---------------------------------------------------------------------------
// Windows: an install the MSI owns is upgraded by the MSI
// ---------------------------------------------------------------------------

/// The ProductCode of the MSI that installed this copy, if one did.
///
/// MSI names the Add/Remove Programs entry by a code that changes with every
/// release, so the installer records the current one at a path that does not.
/// Its presence is also the only reliable answer to "was this installed by the
/// MSI", which decides how an upgrade is applied.
#[cfg(windows)]
fn msi_product_code(marker_key: &str) -> Option<String> {
    read_registry_value(marker_key, "ProductCode").filter(|code| is_product_code(code))
}

/// Hand the upgrade to `msiexec`, so Windows keeps describing what is on disk.
///
/// This returns while the installer is still running, and it has to: the
/// package replaces the executable this process is running from. MSI cannot
/// replace a locked file without scheduling a reboot, so the caller exits and
/// the installer proceeds into a directory nobody holds open.
#[cfg(windows)]
fn msi_upgrade(release: &self_update::update::Release) -> Result<Applied> {
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.ends_with(".msi"))
        .ok_or_else(|| {
            anyhow!(
                "release {} has no .msi asset; download it from the releases page",
                release.version
            )
        })?;

    // Not the install directory: this process exits before the installer is
    // finished, so nothing here can clean up after it, and debris in the
    // install directory is what a self-updating MSI install must not leave.
    let tmp = std::env::temp_dir().join("selvedge-update");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)
        .with_context(|| format!("cannot create staging dir {}", tmp.display()))?;
    let package = tmp.join(&asset.name);

    let f = std::fs::File::create(&package)
        .with_context(|| format!("cannot create {}", package.display()))?;
    self_update::Download::from_url(&asset.download_url)
        .set_header(
            http::header::ACCEPT,
            http::HeaderValue::from_static("application/octet-stream"),
        )
        .show_progress(true)
        .download_to(f)
        .context("download failed")?;

    // `/qb` rather than `/qn`: a per-user install needs no elevation, but it
    // does need somewhere to say so when a file is in use, and a silent
    // install that hit that would schedule a reboot without telling anyone.
    std::process::Command::new("msiexec")
        .args(["/i", &package.to_string_lossy(), "/qb"])
        .spawn()
        .context("failed to launch msiexec")?;

    // Deliberately not touching the cached update status: msiexec runs
    // asynchronously and may fail or be cancelled, so recording "now on the
    // new version, no update available" would hide a still-pending upgrade
    // until something else re-checked. The next check settles it honestly.
    Ok(Applied {
        version: release.version.clone(),
        frontends: Vec::new(),
        installer_launched: true,
    })
}

#[cfg(windows)]
fn read_registry_value(key: &str, name: &str) -> Option<String> {
    let out = no_window(std::process::Command::new("reg"))
        .args(["query", key, "/v", name])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_reg_value(&String::from_utf8_lossy(&out.stdout), name)
}

/// `reg.exe` from a GUI process would flash a console window.
#[cfg(windows)]
fn no_window(mut cmd: std::process::Command) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Pull one value out of `reg query` output, whose shape is
/// `    <name>    REG_SZ    <value>` under a key heading.
///
/// Not `#[cfg(windows)]`: the parsing is where the mistakes live, and a test
/// that only runs on the release runner is a test nobody sees fail.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_reg_value(output: &str, name: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| {
            // The name must end at whitespace, or `ProductCodeOther` answers
            // for `ProductCode`.
            let rest = line.trim_start().strip_prefix(name)?;
            let rest = rest.strip_prefix(char::is_whitespace)?.trim_start();
            let (kind, value) = rest.split_once(char::is_whitespace)?;
            kind.starts_with("REG_").then(|| value.trim().to_string())
        })
        .filter(|value| !value.is_empty())
}

/// A registry-shaped GUID, `{8-4-4-4-12}`. The value is user-writable, and it
/// is about to name a registry key.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_product_code(value: &str) -> bool {
    let Some(inner) = value
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return false;
    };
    inner.split('-').map(str::len).eq([8usize, 4, 4, 4, 12])
        && inner.chars().all(|c| c == '-' || c.is_ascii_hexdigit())
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

    // Aliases are symlinks, so this is about a platform that has them.
    #[test]
    #[cfg(unix)]
    fn every_old_helper_name_becomes_a_link_to_the_binary() {
        let dir = scratch("aliases");
        std::fs::write(dir.join(SAMPLE.primary()), b"binary").unwrap();
        refresh_aliases(PROJECT, &dir);

        for alias in SAMPLE.aliases {
            let path = dir.join(alias);
            assert_eq!(
                std::fs::read_link(&path).unwrap(),
                Path::new(SAMPLE.primary()),
                "{alias}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"binary", "{alias} resolves");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Aliases are symlinks, so this is about a platform that has them.
    #[test]
    #[cfg(unix)]
    fn an_upgrade_from_the_shell_helpers_replaces_the_scripts() {
        // What 0.4.0 leaves behind: real bash scripts. Left in place they would
        // answer for their names forever.
        let dir = scratch("stale");
        std::fs::write(dir.join(SAMPLE.primary()), b"new").unwrap();
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

    // Aliases are symlinks, so this is about a platform that has them.
    #[test]
    #[cfg(unix)]
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

    // Aliases are symlinks, so this is about a platform that has them.
    #[test]
    #[cfg(unix)]
    fn refreshing_an_existing_set_of_aliases_is_a_no_op() {
        let dir = scratch("idempotent");
        std::fs::write(dir.join(SAMPLE.primary()), b"binary").unwrap();
        refresh_aliases(PROJECT, &dir);
        refresh_aliases(PROJECT, &dir);
        assert_eq!(
            std::fs::read_link(dir.join(SAMPLE.aliases[0])).unwrap(),
            Path::new(SAMPLE.primary())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_the_new_layout_does_not_write_is_taken_away() {
        let dir = scratch("legacy");
        std::fs::write(dir.join(SAMPLE.primary()), b"binary").unwrap();
        let stale = dir.join(SAMPLE.legacy[0]);
        std::fs::write(&stale, b"#!/bin/bash\n").unwrap();

        refresh_aliases(PROJECT, &dir);
        assert!(
            !stale.exists(),
            "a helper nothing writes any more still answers for its own name"
        );
        // And the aliases it does write are still there.
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_link(dir.join(SAMPLE.aliases[0])).unwrap(),
            Path::new(SAMPLE.primary())
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
    fn a_value_is_read_from_the_registry_line_that_names_it() {
        let out = "\r\nHKEY_CURRENT_USER\\Software\\SampleGauge\r\n    \
                   ProductCode    REG_SZ    {D2B4E6A1-1111-4C3E-9F0A-ABCDEF012345}\r\n";
        assert_eq!(
            parse_reg_value(out, "ProductCode").as_deref(),
            Some("{D2B4E6A1-1111-4C3E-9F0A-ABCDEF012345}")
        );
        // A longer name starting with the one asked for must not answer.
        let other = "    ProductCodeOther    REG_SZ    {D2B4E6A1-1111-4C3E-9F0A-ABCDEF012345}\r\n";
        assert_eq!(parse_reg_value(other, "ProductCode"), None);
        assert_eq!(parse_reg_value("", "ProductCode"), None);
        assert_eq!(
            parse_reg_value("    ProductCode    REG_SZ    \r\n", "ProductCode"),
            None
        );
    }

    #[test]
    fn only_a_registry_shaped_guid_is_taken_for_one() {
        // It is user-writable, and it is about to name a registry key.
        assert!(is_product_code("{D2B4E6A1-1111-4C3E-9F0A-ABCDEF012345}"));
        assert!(!is_product_code("D2B4E6A1-1111-4C3E-9F0A-ABCDEF012345"));
        assert!(!is_product_code("{D2B4E6A1-1111-4C3E-9F0A-ABCDEF01234}"));
        assert!(!is_product_code("{D2B4E6A1-1111-4C3E-9F0A-ABCDEFZZ2345}"));
        assert!(!is_product_code("{}"));
        assert!(!is_product_code(
            r"{../../etc/passwd-1111-4C3E-9F0A-ABCDEF012345}"
        ));
    }

    // Aliases are symlinks, so this is about a platform that has them.
    #[test]
    #[cfg(unix)]
    fn the_primary_binary_is_the_one_the_rest_are_named_after() {
        assert_eq!(SAMPLE.primary(), "samplegauge");
        // And the aliases point at it, not at a later one.
        let dir = scratch("primary");
        std::fs::write(dir.join(SAMPLE.primary()), b"binary").unwrap();
        refresh_aliases(PROJECT, &dir);
        assert_eq!(
            std::fs::read_link(dir.join(SAMPLE.aliases[0])).unwrap(),
            Path::new("samplegauge")
        );
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
        let path = dir.join(format!(".{}-update.lock", PROJECT.primary()));
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
            binaries: &["othergauge"],
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

    /// The extractor and the asset asked for have to describe the same file.
    /// Nothing downstream notices the disagreement: the download succeeds and
    /// the extraction fails on a machine no one is building on.
    #[test]
    fn the_archive_kind_matches_the_suffix_the_updater_asks_for() {
        match archive_kind() {
            self_update::ArchiveKind::Zip => assert_eq!(ARCHIVE_SUFFIX, ".zip"),
            self_update::ArchiveKind::Tar(_) => assert_eq!(ARCHIVE_SUFFIX, ".tar.gz"),
            other => panic!("no suffix declared for {other:?}"),
        }
    }

    /// An asset name carries the operating system as well as the architecture.
    /// Naming only the architecture is how a Windows x86_64 build asked for
    /// the Linux asset and got one.
    #[test]
    fn the_platform_substring_names_the_running_os() {
        let target = arch_target().expect("this test builds on supported arches only");
        let os = if cfg!(windows) { "windows" } else { "linux" };
        assert!(
            target.starts_with(os),
            "{target} does not name {os}, so it matches another platform's asset"
        );
    }
}
