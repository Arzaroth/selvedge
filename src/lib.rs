//! What a self-updating desktop widget needs and its panel does not.
//!
//! Two projects shell out to a binary that draws a panel on three desktops,
//! and both have to answer the same question: how does a user who installed
//! this from a release archive get the next one, without the binary and the
//! QML it feeds ending up a release apart? That question has nothing to do
//! with either panel, and the answer was 1,400 lines long in both of them.
//!
//! Everything here is machinery. What differs between the two - the binary's
//! name, the repository its releases come from, the frontends it ships - is
//! declared by the caller in a [`Project`] and handed back in.

pub mod frontend;
pub mod state;
pub mod update;

pub use frontend::{Frontend, Restart, VersionSource};
pub use state::UpdateStatus;

/// Everything the machinery cannot know about the project driving it.
///
/// A `&'static Project` is threaded through rather than read from a global,
/// so a test can describe a project that does not exist and drive the whole
/// install against it.
#[derive(Debug, Clone, Copy)]
pub struct Project {
    /// The installed executable's name, which also names its release assets:
    /// `<binary>-<tag>-<target>.tar.gz`.
    pub binary: &'static str,
    /// `owner/repo` releases are pulled from.
    pub repo: &'static str,
    /// Environment variable that overrides `repo`, so a fork can update from
    /// its own releases without a rebuild.
    pub repo_env: &'static str,
    /// The running binary's version. It has to come from the caller:
    /// `CARGO_PKG_VERSION` here would be this crate's version, not theirs.
    pub version: &'static str,
    /// Desktop payloads shipped in the same archive as the binary.
    pub frontends: &'static [Frontend],
    /// Symlinks installed beside the binary, which an update has to keep
    /// pointing at the copy it just wrote.
    pub aliases: &'static [&'static str],
    /// Executables an older layout installed that nothing writes now. An
    /// update removes them rather than leaving a stale copy to answer.
    pub legacy: &'static [&'static str],
}

impl Project {
    /// The repository to pull releases from, after the environment has had its
    /// say.
    pub fn repo(&self) -> String {
        std::env::var(self.repo_env).unwrap_or_else(|_| self.repo.to_string())
    }

    pub fn owner_and_name(&self) -> (String, String) {
        let repo = self.repo();
        match repo.split_once('/') {
            Some((owner, name)) => (owner.to_string(), name.to_string()),
            None => match self.repo.split_once('/') {
                Some((owner, name)) => (owner.to_string(), name.to_string()),
                None => (String::new(), repo),
            },
        }
    }
}

/// Resolve a program on PATH. Small enough to keep here rather than take a
/// dependency on, and `which` as a subprocess is a spawn to do what reading
/// PATH does.
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

/// A project that does not exist, so the machinery can be driven without
/// standing up a real one. Its shape is the shape both real callers have.
#[cfg(test)]
pub(crate) const SAMPLE: Project = Project {
    binary: "samplegauge",
    repo: "Arzaroth/SampleGauge",
    repo_env: "SAMPLEGAUGE_REPO",
    version: "1.2.3",
    frontends: SAMPLE_FRONTENDS,
    aliases: &["samplegauge-ctl", "samplegauge-watch"],
    legacy: &["samplegauge-update"],
};

#[cfg(test)]
const SAMPLE_FRONTENDS: &[Frontend] = &[
    Frontend {
        id: "plasma",
        label: "KDE Plasma applet",
        payload: "plasma/org.samplegauge.plasmoid",
        artifact: "org.samplegauge.plasmoid",
        version_source: VersionSource::PlasmaMetadata,
        gsettings_schemas: false,
        compiled: false,
        restart: Restart::Cheap("kquitapp6 plasmashell && kstart plasmashell"),
    },
    Frontend {
        id: "gnome",
        label: "GNOME Shell extension",
        payload: "gnome/samplegauge@arzaroth.github.io",
        artifact: "samplegauge@arzaroth.github.io",
        version_source: VersionSource::GnomeMetadata,
        gsettings_schemas: true,
        compiled: true,
        restart: Restart::Session("log out and back in"),
    },
    Frontend {
        id: "omarchy",
        label: "Omarchy bar widget",
        payload: "omarchy/arzaroth.samplegauge",
        artifact: "arzaroth.samplegauge",
        version_source: VersionSource::ManifestVersion,
        gsettings_schemas: false,
        compiled: false,
        restart: Restart::Cheap("omarchy-shell -q shell rescanPlugins"),
    },
];
