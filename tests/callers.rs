//! Both callers, declared the way they declare themselves.
//!
//! The crate came out of one of them, so its shape fits that one by
//! construction. This is the other half of the claim: TokenGauge ships two
//! binaries where TailGauge ships one, keeps an old name working where
//! TailGauge takes one away, and is installed by an MSI on Windows where
//! TailGauge has no Windows at all.
//!
//! Most of what this file checks, it checks by compiling.

#[cfg(feature = "self-update")]
use selvedge::update;
use selvedge::{Frontend, Project, Restart, VersionSource, frontend, state};

const TAILGAUGE_FRONTENDS: &[Frontend] = &[
    Frontend {
        id: "plasma",
        label: "KDE Plasma applet",
        payload: "plasma/org.tailgauge.plasmoid",
        artifact: "org.tailgauge.plasmoid",
        version_source: VersionSource::PlasmaMetadata,
        gsettings_schemas: false,
        compiled: false,
        restart: Restart::Cheap("kquitapp6 plasmashell && kstart plasmashell"),
    },
    Frontend {
        id: "gnome",
        label: "GNOME Shell extension",
        payload: "gnome/tailgauge@arzaroth.github.io",
        artifact: "tailgauge@arzaroth.github.io",
        version_source: VersionSource::GnomeMetadata,
        gsettings_schemas: true,
        compiled: true,
        restart: Restart::Session("log out and back in"),
    },
    Frontend {
        id: "omarchy",
        label: "Omarchy bar widget",
        payload: "omarchy/arzaroth.tailgauge",
        artifact: "arzaroth.tailgauge",
        version_source: VersionSource::ManifestVersion,
        gsettings_schemas: false,
        compiled: false,
        restart: Restart::Cheap("omarchy-shell -q shell rescanPlugins"),
    },
];

const TAILGAUGE: Project = Project {
    binaries: &["tailgauge"],
    repo: "Arzaroth/TailGauge",
    repo_env: "TAILGAUGE_REPO",
    version: "0.5.1",
    frontends: TAILGAUGE_FRONTENDS,
    aliases: &[
        "tailgauge-ctl",
        "tailgauge-watch",
        "tailgauge-notify",
        "tailgauge-send",
        "tailgauge-receive",
        "tailgauge-file-select",
        "tailgauge-copy",
    ],
    legacy: &["tailgauge-update"],
    msi_marker_key: None,
};

const TOKENGAUGE_FRONTENDS: &[Frontend] = &[Frontend {
    id: "gnome",
    label: "GNOME Shell extension",
    payload: "gnome/tokengauge@arzaroth.github.io",
    artifact: "tokengauge@arzaroth.github.io",
    version_source: VersionSource::GnomeMetadata,
    gsettings_schemas: true,
    compiled: true,
    restart: Restart::Session("log out and back in"),
}];

#[cfg(not(target_os = "windows"))]
const TOKENGAUGE_BINARIES: &[&str] = &["tokengauge", "tokengauge-tui"];
#[cfg(target_os = "windows")]
const TOKENGAUGE_BINARIES: &[&str] = &["tokengauge-tui.exe", "tokengauge-tray.exe"];

const TOKENGAUGE: Project = Project {
    binaries: TOKENGAUGE_BINARIES,
    repo: "Arzaroth/TokenGauge",
    repo_env: "TOKENGAUGE_REPO",
    version: "0.23.0",
    frontends: TOKENGAUGE_FRONTENDS,
    // The name it shipped under before 0.23.0. It still answers to it, so it
    // is an alias and not something to take away.
    aliases: &["tokengauge-waybar"],
    legacy: &[],
    msi_marker_key: Some(r"HKCU\Software\TokenGauge"),
};

#[test]
fn a_project_shipping_several_binaries_names_the_one_the_rest_follow() {
    assert_eq!(TOKENGAUGE.binaries.len(), 2);
    assert_eq!(TAILGAUGE.binaries.len(), 1);
    // The primary names the assets and is what every alias points at, so it
    // has to be the one the release is built around.
    assert!(TOKENGAUGE.primary().starts_with("tokengauge"));
    assert_eq!(TAILGAUGE.primary(), "tailgauge");
}

#[test]
fn an_old_name_is_either_kept_working_or_taken_away_and_never_both() {
    for project in [TAILGAUGE, TOKENGAUGE] {
        for stale in project.legacy {
            assert!(
                !project.aliases.contains(stale),
                "{stale} would be written and removed by the same update"
            );
        }
    }
    // The two callers resolve it in opposite directions, which is the reason
    // both fields exist: TokenGauge's old name still runs, TailGauge's is a
    // shell script the binary answers with a usage error.
    assert!(TOKENGAUGE.aliases.contains(&"tokengauge-waybar"));
    assert!(TAILGAUGE.legacy.contains(&"tailgauge-update"));
}

#[test]
fn the_repository_is_overridable_per_project() {
    // A fork updates from its own releases, and the two must not share a
    // variable or one would redirect the other.
    assert_ne!(TAILGAUGE.repo_env, TOKENGAUGE.repo_env);
    let (owner, name) = TOKENGAUGE.owner_and_name();
    assert_eq!((owner.as_str(), name.as_str()), ("Arzaroth", "TokenGauge"));
}

/// The half a GUI takes: it installs payloads and reads a cached check, and
/// never links the network stack that produced it.
#[test]
fn a_caller_can_take_the_payload_installer_without_the_updater() {
    let present = frontend::installed(&TOKENGAUGE);
    let _ = present.len();
    assert!(frontend::find(&TOKENGAUGE, "gnome").is_some());
    let _ = state::update_cache_file(&TAILGAUGE);
}

#[test]
fn each_project_caches_under_its_own_name() {
    let a = state::update_cache_file(&TAILGAUGE);
    let b = state::update_cache_file(&TOKENGAUGE);
    assert_ne!(a, b, "one project's check would answer for the other");
    assert!(b.to_string_lossy().contains("tokengauge"));
}

#[test]
fn every_frontend_id_resolves_for_the_project_that_ships_it() {
    assert!(frontend::find(&TOKENGAUGE, "gnome").is_some());
    assert!(
        frontend::find(&TOKENGAUGE, "omarchy").is_none(),
        "a frontend one project ships is not a frontend the other has"
    );
    assert!(frontend::find(&TAILGAUGE, "omarchy").is_some());
}

/// Nothing here runs: it exists so the call sites both projects have today
/// fail to compile if this crate's surface moves under them.
#[cfg(feature = "self-update")]
#[allow(dead_code)]
fn the_api_both_callers_use(project: &Project) -> anyhow::Result<()> {
    let cache = state::update_cache_file(project);
    let status = update::check(project, &cache)?;
    let _ = update::check_cached(project, &cache, true)?;
    let _ = update::version_gt(&status.current, project.version);

    let applied = update::apply(project, &cache)?;
    // TokenGauge returns early on this; TailGauge never sets it.
    if applied.installer_launched {
        return Ok(());
    }
    for outcome in &applied.frontends {
        let _ = (outcome.id, outcome.label, &outcome.error);
        let _ = (outcome.needs_session_restart, outcome.restart_hint);
    }

    let present = frontend::installed(project);
    let _ = update::install_frontends(project, &present, &applied.version)?;
    if let Some(f) = frontend::find(project, "gnome") {
        let _ = (f.installed_version(), f.restart.needs_session_restart());
    }
    let _ = (update::ARCHIVE_SUFFIX, update::arch_target()?);
    Ok(())
}
