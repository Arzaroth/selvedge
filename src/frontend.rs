//! Desktop frontends that are not binaries.
//!
//! The Plasma applet, the GNOME extension and the Omarchy bar widget are QML
//! and JavaScript installed outside `~/.local/bin`, so replacing the binary
//! leaves them untouched. A newer binary feeding older QML looks like a missing
//! feature rather than a stale install.
//!
//! Everything here is path-and-copy work with no network. [`crate::update`]
//! owns fetching a release; this owns knowing where each artifact belongs, what
//! version is sitting there, and how to replace it.

use std::path::{Path, PathBuf};

use crate::Project;
use anyhow::{Context, Result, bail};

/// Directory inside the release archive holding the frontend payloads.
pub const ARCHIVE_ROOT: &str = "frontends";

/// Directory a checkout assembles its payloads into, in the archive's own
/// layout. The project's build script writes it; it is not in the repository.
pub const BUILD_ROOT: &str = "build";

/// What has to happen before a freshly installed frontend is actually running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    /// Picks itself up, or needs a cheap restart the user already knows about.
    Cheap(&'static str),
    /// Needs the whole session restarted - GNOME Shell cannot reload an
    /// extension on Wayland without logging out.
    Session(&'static str),
}

impl Restart {
    pub fn hint(&self) -> &'static str {
        match self {
            Restart::Cheap(h) | Restart::Session(h) => h,
        }
    }

    pub fn needs_session_restart(&self) -> bool {
        matches!(self, Restart::Session(_))
    }
}

/// How a frontend records its own version, so skew against the binary is
/// visible rather than silent.
#[derive(Debug, Clone, Copy)]
pub enum VersionSource {
    /// `metadata.json` -> `KPlugin.Version` (Plasma).
    PlasmaMetadata,
    /// `metadata.json` -> `version-name` (GNOME).
    GnomeMetadata,
    /// `manifest.json` -> `version` (Omarchy plugin schema).
    ManifestVersion,
}

#[derive(Debug, Clone, Copy)]
pub struct Frontend {
    pub id: &'static str,
    pub label: &'static str,
    /// Path of the payload inside the archive, below [`ARCHIVE_ROOT`], and the
    /// same path below the repository root in a checkout.
    pub payload: &'static str,
    /// Directory name the payload lands in; the id its desktop expects.
    pub artifact: &'static str,
    pub version_source: VersionSource,
    /// Ships GSettings schemas, which are XML in the payload and a compiled
    /// blob at runtime. See [`compile_schemas`].
    pub gsettings_schemas: bool,
    /// Its sources are compiled rather than installed as they are, so in a
    /// checkout `payload` is the compiler's input and not a payload.
    /// [`payload_in`](Frontend::payload_in) looks under [`BUILD_ROOT`] for
    /// these: installing the source directory would land an extension the
    /// shell refuses to load.
    pub compiled: bool,
    pub restart: Restart,
}

pub fn find(project: &Project, id: &str) -> Option<&'static Frontend> {
    let id = id.trim().to_lowercase();
    project.frontends.iter().find(|f| f.id == id)
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn data_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => home().map(|h| h.join(".local/share")),
    }
}

fn config_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => home().map(|h| h.join(".config")),
    }
}

impl Frontend {
    /// Where this frontend's desktop expects to find it.
    pub fn dest_dir(&self) -> Option<PathBuf> {
        match self.id {
            "plasma" => data_home().map(|d| d.join("plasma/plasmoids").join(self.artifact)),
            "gnome" => data_home().map(|d| d.join("gnome-shell/extensions").join(self.artifact)),
            "omarchy" => config_home().map(|c| c.join("omarchy/plugins").join(self.artifact)),
            _ => None,
        }
    }

    pub fn is_installed(&self) -> bool {
        self.dest_dir().is_some_and(|d| d.is_dir())
    }

    /// The version recorded in the installed copy, which is the one actually
    /// running - not the version of the binary asking the question.
    pub fn installed_version(&self) -> Option<String> {
        self.version_in(&self.dest_dir()?)
    }

    fn version_in(&self, dir: &Path) -> Option<String> {
        let (file, pointer) = match self.version_source {
            VersionSource::PlasmaMetadata => ("metadata.json", "/KPlugin/Version"),
            VersionSource::GnomeMetadata => ("metadata.json", "/version-name"),
            VersionSource::ManifestVersion => ("manifest.json", "/version"),
        };
        let raw = std::fs::read_to_string(dir.join(file)).ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let value = parsed.pointer(pointer)?;
        // Plasma writes it as a string; be forgiving about a number.
        let text = match value {
            serde_json::Value::String(s) => s.clone(),
            other => other
                .as_str()
                .map(str::to_string)
                .unwrap_or(other.to_string()),
        };
        let text = text.trim().trim_start_matches('v').to_string();
        (!text.is_empty()).then_some(text)
    }

    /// Copy the payload out of an extracted archive (or a checkout) into place,
    /// replacing whatever is there.
    pub fn install_from(&self, source_root: &Path) -> Result<PathBuf> {
        let dest = self.dest_dir().ok_or_else(|| {
            anyhow::anyhow!("cannot resolve an install directory for {}", self.id)
        })?;
        self.install_into(source_root, &dest)
    }

    /// The body of [`Frontend::install_from`] with the destination supplied, so
    /// a test can exercise the real staging and replacement without moving
    /// `$XDG_DATA_HOME` out from under the process.
    pub fn install_into(&self, source_root: &Path, dest: &Path) -> Result<PathBuf> {
        let src = self.payload_in(source_root).ok_or_else(|| {
            anyhow::anyhow!(
                "{} payload not found under {}",
                self.label,
                source_root.display()
            )
        })?;

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }

        // Staged beside the destination so the final move is a rename on the
        // same filesystem, and hidden so a half-written copy is not picked up
        // by the scanners that walk these directories. The name is built from
        // the whole directory name: every artifact id contains dots, so
        // `with_extension` would truncate `org.tailgauge.plasmoid` to
        // `org.tailgauge.tg-new` and could shadow an unrelated directory.
        let name = dest
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow::anyhow!("cannot name a staging dir for {}", dest.display()))?;
        let staged = dest.with_file_name(format!(".{name}.tg-new"));
        let _ = std::fs::remove_dir_all(&staged);

        // Replace rather than merge: a file dropped upstream has to disappear
        // here too, or a stale QML file keeps being loaded alongside the new one.
        if let Err(e) = copy_dir(&src, &staged) {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(e)
                .with_context(|| format!("cannot stage {} into {}", self.label, staged.display()));
        }

        if let Err(e) = compile_schemas(self, &staged) {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(e);
        }

        // Move the old install aside rather than deleting it, so a rename that
        // fails leaves something on disk.
        let retired = dest.with_file_name(format!(".{name}.tg-old"));
        let _ = std::fs::remove_dir_all(&retired);
        let had_old = dest.exists() && std::fs::rename(dest, &retired).is_ok();

        if let Err(e) = std::fs::rename(&staged, dest) {
            if had_old {
                let _ = std::fs::rename(&retired, dest);
            }
            let _ = std::fs::remove_dir_all(&staged);
            return Err(e).with_context(|| format!("cannot move {} into place", self.label));
        }
        let _ = std::fs::remove_dir_all(&retired);
        Ok(dest.to_path_buf())
    }

    /// Whether the installed copy is in a state its desktop can load. Only
    /// GSettings schemas can be half-installed this way; everything else is
    /// covered by the directory being there at all.
    pub fn schemas_ready(&self) -> bool {
        self.dest_dir().is_none_or(|d| self.schemas_ready_in(&d))
    }

    fn schemas_ready_in(&self, dir: &Path) -> bool {
        !self.gsettings_schemas || dir.join("schemas/gschemas.compiled").is_file()
    }

    /// Locate this frontend's payload under an archive root or a checkout.
    pub fn payload_in(&self, source_root: &Path) -> Option<PathBuf> {
        let archived = source_root.join(ARCHIVE_ROOT).join(self.payload);
        if archived.is_dir() {
            return Some(archived);
        }
        let in_checkout = self.checkout_payload(source_root);
        in_checkout.is_dir().then_some(in_checkout)
    }

    /// Where a checkout keeps this payload. For a compiled frontend that is
    /// what the build script assembled, never the sources it assembled it
    /// from.
    fn checkout_payload(&self, source_root: &Path) -> PathBuf {
        if self.compiled {
            source_root
                .join(BUILD_ROOT)
                .join(ARCHIVE_ROOT)
                .join(self.payload)
        } else {
            source_root.join(self.payload)
        }
    }
}

/// Every frontend already present on this machine.
pub fn installed(project: &Project) -> Vec<&'static Frontend> {
    project
        .frontends
        .iter()
        .filter(|f| f.is_installed())
        .collect()
}

/// Compile the GSettings schemas the payload ships as XML.
///
/// The compiled blob is built on the machine it runs on and so is neither in
/// the repository nor in the archive, and `install_into` replaces the whole
/// directory - so an install that skipped this would leave the schema directory
/// with the XML and nothing else. That is worse than shipping no schemas at
/// all: GNOME's `Extension.getSettings()` switches to
/// `Gio.SettingsSchemaSource.new_from_directory` the moment `schemas/` exists,
/// and then fails on the missing `gschemas.compiled` at every enable.
///
/// Run on the staged copy, so a machine without the compiler keeps the install
/// it already had instead of gaining a broken one.
fn compile_schemas(frontend: &Frontend, staged: &Path) -> Result<()> {
    if !frontend.gsettings_schemas {
        return Ok(());
    }
    let schemas = staged.join("schemas");
    if !schemas.is_dir() {
        bail!(
            "{} ships GSettings schemas but {} has none",
            frontend.label,
            staged.display()
        );
    }
    let out = std::process::Command::new("glib-compile-schemas")
        .arg(&schemas)
        .output()
        .with_context(|| {
            format!(
                "cannot run glib-compile-schemas, which {} needs - install the glib2 tools \
                 (Debian/Ubuntu: libglib2.0-bin)",
                frontend.label
            )
        })?;
    if !out.status.success() {
        bail!(
            "glib-compile-schemas failed on {}: {}",
            schemas.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dest.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &target)?;
        } else if file_type.is_symlink() {
            // The Omarchy plugin registry refuses a plugin folder containing
            // one, and none of the payloads ship any; refusing beats copying
            // something that resolves differently on the target machine.
            bail!("refusing to copy symlink {}", entry.path().display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SAMPLE;

    #[test]
    fn ids_resolve_case_insensitively() {
        assert_eq!(find(&SAMPLE, "plasma").unwrap().id, "plasma");
        assert_eq!(find(&SAMPLE, "  GNOME ").unwrap().id, "gnome");
        assert!(find(&SAMPLE, "aqua").is_none());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tg-frontend-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn every_frontend_resolves_a_destination() {
        // A frontend whose id is not handled in dest_dir() would silently be
        // uninstallable, and an update would skip it without a word.
        for f in crate::SAMPLE.frontends {
            assert!(f.dest_dir().is_some(), "{} has no destination", f.id);
        }
    }

    #[test]
    fn version_is_read_from_each_metadata_shape() {
        let dir = scratch("version");
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(
            dir.join("metadata.json"),
            r#"{"KPlugin":{"Version":"0.5.0"}}"#,
        )
        .unwrap();
        assert_eq!(
            find(&SAMPLE, "plasma").unwrap().version_in(&dir).as_deref(),
            Some("0.5.0")
        );

        std::fs::write(dir.join("metadata.json"), r#"{"version-name":"v0.5.0"}"#).unwrap();
        assert_eq!(
            find(&SAMPLE, "gnome").unwrap().version_in(&dir).as_deref(),
            Some("0.5.0"),
            "a leading v must not read as a different version"
        );

        std::fs::write(dir.join("manifest.json"), r#"{"version":"0.5.0"}"#).unwrap();
        assert_eq!(
            find(&SAMPLE, "omarchy")
                .unwrap()
                .version_in(&dir)
                .as_deref(),
            Some("0.5.0")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn payload_is_found_in_an_archive_or_a_checkout() {
        let dir = scratch("layout");
        let plasma = find(&SAMPLE, "plasma").unwrap();

        std::fs::create_dir_all(dir.join(ARCHIVE_ROOT).join(plasma.payload)).unwrap();
        assert_eq!(
            plasma.payload_in(&dir).unwrap(),
            dir.join(ARCHIVE_ROOT).join(plasma.payload)
        );

        let checkout = dir.join("checkout");
        std::fs::create_dir_all(checkout.join(plasma.payload)).unwrap();
        assert_eq!(
            plasma.payload_in(&checkout).unwrap(),
            checkout.join(plasma.payload)
        );

        assert!(plasma.payload_in(&dir.join("empty")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_layout_resolves_every_frontend() {
        let dir = scratch("archive");
        for f in crate::SAMPLE.frontends {
            std::fs::create_dir_all(dir.join(ARCHIVE_ROOT).join(f.payload)).unwrap();
        }
        for f in crate::SAMPLE.frontends {
            assert!(
                f.payload_in(&dir).is_some(),
                "{} does not resolve in the archive layout release.yml builds",
                f.id
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_replaces_rather_than_merges() {
        let plasma = find(&SAMPLE, "plasma").unwrap();
        let dir = scratch("install");

        let archive = dir.join("archive");
        let payload = archive.join(ARCHIVE_ROOT).join(plasma.payload);
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::write(payload.join("kept.qml"), "new").unwrap();
        std::fs::write(
            payload.join("metadata.json"),
            r#"{"KPlugin":{"Version":"9.9.9"}}"#,
        )
        .unwrap();

        let dest = dir.join("plasmoids").join(plasma.artifact);
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("gone.qml"), "old").unwrap();

        assert_eq!(plasma.install_into(&archive, &dest).unwrap(), dest);
        assert!(dest.join("kept.qml").exists());
        assert!(
            !dest.join("gone.qml").exists(),
            "a file dropped upstream must not survive the install"
        );
        assert_eq!(plasma.version_in(&dest).as_deref(), Some("9.9.9"));

        assert!(
            !dest
                .with_file_name(format!(".{}.tg-new", plasma.artifact))
                .exists(),
            "staging dir survived a successful install"
        );
        assert!(
            !dest.with_extension("tg-new").exists(),
            "staging must not truncate the dotted artifact name"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The old install must survive a failed replacement.
    #[test]
    fn a_failed_replacement_leaves_the_old_install_where_it_was() {
        let plasma = find(&SAMPLE, "plasma").unwrap();
        let dir = scratch("rollback");

        let src = dir.join(ARCHIVE_ROOT).join(plasma.payload);
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("new.txt"), "new").unwrap();

        let dest = dir.join("installed").join(plasma.artifact);
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("old.txt"), "old").unwrap();

        plasma.install_into(&dir, &dest).expect("install");
        assert!(dest.join("new.txt").exists());
        assert!(!dest.join("old.txt").exists());
        let leftovers: Vec<_> = std::fs::read_dir(dest.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "staging left behind: {leftovers:?}");

        // Fail *after* staging, which is the half that matters: a file sitting
        // where the retired copy wants to go makes the move-aside fail, so the
        // old install is still at `dest` when the promotion rename runs - and
        // that rename fails too, because `dest` is a non-empty directory.
        #[cfg(unix)]
        {
            std::fs::write(
                dest.with_file_name(format!(".{}.tg-old", plasma.artifact)),
                "x",
            )
            .unwrap();
            assert!(plasma.install_into(&dir, &dest).is_err());
            assert!(
                dest.join("new.txt").exists(),
                "the working install has to still be there after a failure"
            );
            assert!(
                !dest
                    .with_file_name(format!(".{}.tg-new", plasma.artifact))
                    .exists(),
                "the staged copy has to be cleaned up on the way out"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_install_leaves_nothing_staged() {
        let plasma = find(&SAMPLE, "plasma").unwrap();
        let dir = scratch("install-fail");
        let dest = dir.join("plasmoids").join(plasma.artifact);
        std::fs::create_dir_all(&dest).unwrap();

        assert!(plasma.install_into(&dir.join("empty"), &dest).is_err());
        assert!(
            !dest
                .with_file_name(format!(".{}.tg-new", plasma.artifact))
                .exists(),
            "a failed install must not leave a staged directory behind"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The payload ships the schema XML and nothing else, because the compiled
    /// blob is built on the machine that runs it. An install that left it that
    /// way produces an extension GNOME refuses to enable.
    #[test]
    fn a_gnome_install_compiles_its_schemas() {
        let dir = scratch("schemas");
        let gnome = find(&SAMPLE, "gnome").unwrap();

        // A payload of this crate's own making, in the archive's layout: the
        // machinery has to work for a project this crate has never seen.
        let source = dir.join("archive");
        let payload = source.join(ARCHIVE_ROOT).join(gnome.payload);
        std::fs::create_dir_all(payload.join("schemas")).unwrap();
        std::fs::write(
            payload.join("metadata.json"),
            br#"{"uuid":"x","version-name":"1.2.3"}"#,
        )
        .unwrap();
        std::fs::write(
            payload.join("schemas/org.gnome.shell.extensions.sample.gschema.xml"),
            br#"<?xml version="1.0" encoding="UTF-8"?>
<schemalist>
  <schema id="org.gnome.shell.extensions.sample" path="/org/gnome/shell/extensions/sample/">
    <key name="refresh-interval" type="i"><default>30</default></key>
  </schema>
</schemalist>
"#,
        )
        .unwrap();

        let dest = dir.join("extensions").join(gnome.artifact);
        let installed = gnome.install_into(&source, &dest);
        if crate::which("glib-compile-schemas").is_none() {
            // Nothing to compile with: refusing is the whole point, so the
            // working install a user already had is still there afterwards.
            assert!(
                installed.is_err(),
                "an install that cannot compile must fail"
            );
            assert!(!dest.exists(), "and must not leave a broken copy behind");
        } else {
            installed.expect("install");
            assert!(
                gnome.schemas_ready_in(&dest),
                "the installed extension has no compiled schemas"
            );
            assert_eq!(gnome.version_in(&dest).as_deref(), Some("1.2.3"));
        }

        // A frontend with no schemas is ready wherever it is - the check must
        // not turn into a file the Plasma applet is now expected to grow.
        assert!(find(&SAMPLE, "plasma").unwrap().schemas_ready_in(&dest));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
