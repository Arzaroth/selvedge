//! Finding and running the things a project does not ship.
//!
//! Every caller of this crate shells out: to a schema compiler, to a shell
//! restart, to whatever draws a desktop notification. The rules are the same
//! each time - resolve on `PATH` and answer for what is actually runnable,
//! never inherit stdin, and treat a missing program as an answer rather than
//! an error.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The first executable named `name` on `PATH`, or `None`.
///
/// Small enough to keep here rather than take a dependency on, and `which` as
/// a subprocess is a spawn to do what reading `PATH` does.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

pub fn has(name: &str) -> bool {
    which(name).is_some()
}

/// A file is not a program. A `PATH` walk that stops at `is_file` answers with
/// a data file that happens to share the name, and the caller spawns it and
/// fails somewhere less obvious.
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// `name` on `PATH`, else `~/.local/bin/<name>`, where the installers put it:
/// a GUI or a service started outside a login shell often has no
/// `~/.local/bin` on its `PATH`.
pub fn installed(name: &str) -> Option<PathBuf> {
    installed_in(
        name,
        which(name),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn installed_in(name: &str, on_path: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    on_path.or_else(|| {
        let local = home?.join(".local/bin").join(name);
        is_executable(&local).then_some(local)
    })
}

/// When a process started, in clock ticks since boot, which tells it apart
/// from a later one given the same pid. `None` off Linux or once it is gone.
pub fn start_time(pid: u32) -> Option<u64> {
    let stat =
        std::fs::read_to_string(Path::new("/proc").join(pid.to_string()).join("stat")).ok()?;
    stat_start_time(&stat)
}

/// Field 22 of a `/proc/<pid>/stat` line. The command name in field 2 may hold
/// spaces and parentheses, so the fields are counted from after its last `)`.
pub fn stat_start_time(stat: &str) -> Option<u64> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

/// Run a command to completion, capturing both streams.
pub fn run<I, S>(program: &str, args: I) -> std::io::Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
}

/// Run a command for its exit status alone, with both streams discarded.
pub fn run_quiet<I, S>(program: &str, args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Trimmed stdout of a command that succeeded, or `None`.
pub fn output<I, S>(program: &str, args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let out = run(program, args).ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_is_not_on_path_resolves_to_nothing() {
        assert!(which("selvedge-no-such-binary").is_none());
        assert!(!has("selvedge-no-such-binary"));
        #[cfg(unix)]
        assert!(has("sh"), "sh is on PATH on every unix");
    }

    /// The reason this is not `is_file`: a readable file with the right name
    /// and no execute bit is not something to hand to `Command::new`.
    #[cfg(unix)]
    #[test]
    fn a_file_that_is_not_executable_is_not_a_program() {
        let dir = std::env::temp_dir().join(format!("selvedge-proc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let name = "selvedge-not-a-program";
        std::fs::write(dir.join(name), "not a program").unwrap();

        let _env = crate::env_lock();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let joined =
            std::env::join_paths(std::iter::once(dir.clone()).chain(std::env::split_paths(&path)))
                .unwrap();
        // SAFETY: guarded above, and the value is restored below.
        unsafe { std::env::set_var("PATH", &joined) };
        let found = which(name);
        unsafe { std::env::set_var("PATH", &path) };

        assert!(found.is_none(), "a non-executable file answered: {found:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_start_time_is_the_twenty_second_stat_field() {
        let stat = "4242 (a (b) c) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 987654 20 21";
        assert_eq!(stat_start_time(stat), Some(987654));
        assert_eq!(stat_start_time("4242 (short) S 1 2"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_has_a_start_time() {
        assert!(start_time(std::process::id()).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_program_off_path_is_found_where_the_installer_puts_it() {
        use std::os::unix::fs::PermissionsExt;
        let home = std::env::temp_dir().join(format!("selvedge-installed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let program = bin.join("samplegauge");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();

        assert_eq!(installed_in("samplegauge", None, Some(home.clone())), None);
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            installed_in("samplegauge", None, Some(home.clone())),
            Some(program)
        );
        let on_path = PathBuf::from("/usr/bin/samplegauge");
        assert_eq!(
            installed_in("samplegauge", Some(on_path.clone()), Some(home.clone())),
            Some(on_path)
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn output_is_trimmed_and_a_failure_is_not_an_answer() {
        assert_eq!(
            output("sh", ["-c", "printf 'hi\n\n'"]).as_deref(),
            Some("hi")
        );
        assert_eq!(output("sh", ["-c", "exit 1"]), None);
        assert!(run_quiet("sh", ["-c", "exit 0"]));
        assert!(!run_quiet("sh", ["-c", "exit 1"]));
        assert!(!run_quiet("selvedge-no-such-binary", ["x"]));
    }
}
