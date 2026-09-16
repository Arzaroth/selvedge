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

        let path = std::env::var_os("PATH").unwrap_or_default();
        let joined =
            std::env::join_paths(std::iter::once(dir.clone()).chain(std::env::split_paths(&path)))
                .unwrap();
        // SAFETY: single-threaded test, and the value is restored below.
        unsafe { std::env::set_var("PATH", &joined) };
        let found = which(name);
        unsafe { std::env::set_var("PATH", &path) };

        assert!(found.is_none(), "a non-executable file answered: {found:?}");
        let _ = std::fs::remove_dir_all(&dir);
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
