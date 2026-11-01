//! `<store>/.attribution.jsonl`: which login each CLI spent, and when.
//!
//! A transcript names no login, so remuda writes down what it knows as it
//! happens, one JSON object per line:
//!
//! - `{"kind":"live","at":<ms>,"provider":"claude","accountId":"…","name":"work"}`:
//!   from `at` on, the CLI's own files hold this login. Written on every
//!   switch, and when remuda finds the files changed under it (a `claude
//!   /login`), stamped with the files' mtime. No `accountId` means a login
//!   remuda does not store, or none: usage from then on is nobody's it knows.
//! - `{"kind":"run","at":<ms>,"provider":"claude","session":"<id>","accountId":"…","name":"work"}`:
//!   the CLI session `session` ran under `remuda run` on this login, whatever
//!   the CLI's own files held meanwhile.
//!
//! [`Ledger::login_at`] is the rule a reader applies. Nothing in the file is
//! a guess: before its first line, usage is unattributed.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const FILE: &str = ".attribution.jsonl";
const LOCK: &str = ".attribution.lock";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Entry {
    Live {
        at: i64,
        provider: String,
        #[serde(rename = "accountId", default, skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Run {
        at: i64,
        provider: String,
        session: String,
        #[serde(rename = "accountId")]
        account_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl Entry {
    pub fn at(&self) -> i64 {
        match self {
            Entry::Live { at, .. } | Entry::Run { at, .. } => *at,
        }
    }

    pub fn provider(&self) -> &str {
        match self {
            Entry::Live { provider, .. } | Entry::Run { provider, .. } => provider,
        }
    }
}

pub fn append(store: &Path, entry: &Entry) -> io::Result<()> {
    let _lock = super::lock(store, LOCK)?;
    super::append_line(&store.join(FILE), entry)
}

pub fn read(store: &Path) -> io::Result<Vec<Entry>> {
    super::read_lines(&store.join(FILE))
}

/// Drops what is older than `before`, except each CLI's last `live` line from
/// then, which is still in force and is what everything after it is read
/// against. A line this build cannot read, which a newer writer may have
/// added, is kept as it is.
pub fn prune(store: &Path, before: i64) -> io::Result<()> {
    let _lock = super::lock(store, LOCK)?;
    let path = store.join(FILE);
    let lines = super::raw_lines(&path)?;
    let entries: Vec<Option<Entry>> = lines
        .iter()
        .map(|line| serde_json::from_str(line).ok())
        .collect();
    let mut in_force: HashMap<&str, (i64, usize)> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        if let Some(Entry::Live { at, provider, .. }) = entry
            && *at < before
            && in_force
                .get(provider.as_str())
                .is_none_or(|&(kept, _)| kept <= *at)
        {
            in_force.insert(provider, (*at, index));
        }
    }
    let kept: Vec<&str> = lines
        .iter()
        .zip(&entries)
        .enumerate()
        .filter(|(index, (_, entry))| match entry {
            Some(entry) => entry.at() >= before || in_force.values().any(|&(_, k)| k == *index),
            None => true,
        })
        .map(|(_, (line, _))| line.as_str())
        .collect();
    if kept.len() == lines.len() {
        return Ok(());
    }
    super::replace_lines(&path, &kept)
}

/// The ledger read once, to answer for many events.
#[derive(Debug, Default)]
pub struct Ledger {
    live: HashMap<String, Vec<(i64, Option<String>)>>,
    runs: HashMap<(String, String), String>,
}

impl Ledger {
    pub fn new(entries: impl IntoIterator<Item = Entry>) -> Self {
        let mut ledger = Ledger::default();
        for entry in entries {
            match entry {
                Entry::Live {
                    at,
                    provider,
                    account_id,
                    ..
                } => ledger
                    .live
                    .entry(provider)
                    .or_default()
                    .push((at, account_id)),
                Entry::Run {
                    provider,
                    session,
                    account_id,
                    ..
                } => {
                    ledger.runs.insert((provider, session), account_id);
                }
            }
        }
        for changes in ledger.live.values_mut() {
            changes.sort_by_key(|(at, _)| *at);
        }
        ledger
    }

    pub fn load(store: &Path) -> io::Result<Self> {
        Ok(Ledger::new(read(store)?))
    }

    /// The account a request `provider` billed at `at` spent: the login its
    /// session ran under if a `remuda run` claimed it, else the one the CLI's
    /// files held then, else `None`.
    pub fn login_at(&self, provider: &str, session: Option<&str>, at: i64) -> Option<&str> {
        if let Some(account) =
            session.and_then(|s| self.runs.get(&(provider.to_string(), s.to_string())))
        {
            return Some(account);
        }
        let changes = self.live.get(provider)?;
        let after = changes.partition_point(|(changed, _)| *changed <= at);
        changes[..after].last()?.1.as_deref()
    }

    /// The last `live` line for `provider`: `None` when there is none, and
    /// `Some(None)` when it named no login remuda stores.
    pub fn live_now(&self, provider: &str) -> Option<Option<&str>> {
        self.live
            .get(provider)?
            .last()
            .map(|(_, account)| account.as_deref())
    }

    /// Whether a `run` line already claims `session`.
    pub fn claims(&self, provider: &str, session: &str) -> bool {
        self.runs
            .contains_key(&(provider.to_string(), session.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn live(at: i64, account: Option<&str>) -> Entry {
        Entry::Live {
            at,
            provider: "claude".into(),
            account_id: account.map(str::to_string),
            name: account.map(|a| format!("{a}-name")),
        }
    }

    fn run(at: i64, session: &str, account: &str) -> Entry {
        Entry::Run {
            at,
            provider: "claude".into(),
            session: session.into(),
            account_id: account.into(),
            name: None,
        }
    }

    #[test]
    fn the_lines_are_spelled_as_the_module_says() {
        assert_eq!(
            serde_json::to_string(&live(5, Some("acc"))).unwrap(),
            r#"{"kind":"live","at":5,"provider":"claude","accountId":"acc","name":"acc-name"}"#
        );
        assert_eq!(
            serde_json::to_string(&run(6, "s1", "acc")).unwrap(),
            r#"{"kind":"run","at":6,"provider":"claude","session":"s1","accountId":"acc"}"#
        );
        assert_eq!(
            serde_json::to_string(&live(7, None)).unwrap(),
            r#"{"kind":"live","at":7,"provider":"claude"}"#
        );
    }

    #[test]
    fn a_request_spends_the_login_live_when_it_was_billed() {
        let ledger = Ledger::new([live(200, Some("b")), live(100, Some("a")), live(300, None)]);
        assert_eq!(ledger.login_at("claude", None, 99), None);
        assert_eq!(ledger.login_at("claude", None, 100), Some("a"));
        assert_eq!(ledger.login_at("claude", None, 250), Some("b"));
        assert_eq!(ledger.login_at("claude", None, 300), None);
        assert_eq!(ledger.login_at("codex", None, 250), None);
        assert_eq!(ledger.live_now("claude"), Some(None));
        assert_eq!(ledger.live_now("codex"), None);
    }

    #[test]
    fn a_run_session_beats_whatever_was_live() {
        let ledger = Ledger::new([live(100, Some("a")), run(150, "s1", "r")]);
        assert_eq!(ledger.login_at("claude", Some("s1"), 120), Some("r"));
        assert_eq!(ledger.login_at("claude", Some("s2"), 120), Some("a"));
        assert_eq!(ledger.login_at("codex", Some("s1"), 120), None);
        assert!(ledger.claims("claude", "s1"));
        assert!(!ledger.claims("codex", "s1"));
    }

    #[test]
    fn appended_lines_read_back_and_a_bad_line_costs_only_itself() {
        let store = crate::usage::temp_store("ledger-append");
        append(&store, &live(1, Some("a"))).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(store.join(FILE))
            .unwrap()
            .write_all(b"{\"kind\":\"later\",\"at\":2}\nnot json\n")
            .unwrap();
        append(&store, &run(3, "s", "b")).unwrap();
        assert_eq!(
            read(&store).unwrap(),
            vec![live(1, Some("a")), run(3, "s", "b")]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.join(FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&store);
    }

    #[test]
    fn no_file_is_no_ledger() {
        let store = crate::usage::temp_store("ledger-none");
        assert!(read(&store).unwrap().is_empty());
        assert_eq!(
            Ledger::load(&store).unwrap().login_at("claude", None, 1),
            None
        );
        let _ = std::fs::remove_dir_all(&store);
    }

    #[test]
    fn pruning_keeps_the_live_line_still_in_force() {
        let store = crate::usage::temp_store("ledger-prune");
        for entry in [
            live(10, Some("old")),
            run(15, "s", "r"),
            live(20, Some("a")),
            live(150, Some("b")),
        ] {
            append(&store, &entry).unwrap();
        }
        prune(&store, 100).unwrap();
        assert_eq!(
            read(&store).unwrap(),
            vec![live(20, Some("a")), live(150, Some("b"))]
        );
        let ledger = Ledger::load(&store).unwrap();
        assert_eq!(ledger.login_at("claude", None, 120), Some("a"));
        let _ = std::fs::remove_dir_all(&store);
    }
}
