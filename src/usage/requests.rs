//! `<store>/.proxy-requests.jsonl`: one line per request remuda's proxy answered.
//!
//! The proxy is the one place a request is seen whole: which login and which
//! key it spent, how it ended and how long it took. The CLI's transcript has
//! the tokens too when the client was Claude Code, so a reader adding these up
//! beside the transcripts joins the two on [`Request::dedup_parts`], the ids
//! the *client* received, which are what it wrote down.
//!
//! The file moves to `.proxy-requests.1.jsonl` at [`ROTATE_AT`] bytes, and
//! [`read`] reads both.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::TokenCounts;

pub const FILE: &str = ".proxy-requests.jsonl";
pub const ROTATED: &str = ".proxy-requests.1.jsonl";
pub const ROTATE_AT: u64 = 8 << 20;
const LOCK: &str = ".proxy-requests.lock";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    Failed,
    /// The client went away before the answer was written out.
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    /// When the request arrived, in ms.
    pub at: i64,
    pub provider: String,
    /// The model the client asked for, as the provider was sent it.
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// The login's store name, for a reader that only shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The id of the proxy key the client sent, never the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The client's format: `messages`, `chat` or `responses`.
    pub format: String,
    pub stream: bool,
    pub outcome: Outcome,
    /// The HTTP status the client got, 0 when none was written.
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_byte_ms: Option<u64>,
    pub latency_ms: u64,
    #[serde(default)]
    pub tokens: TokenCounts,
    #[serde(default, skip_serializing_if = "super::is_zero")]
    pub reasoning: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,
}

impl Request {
    /// The message id and request id as a transcript would record them, or
    /// `None` when the client got neither and nothing could match it.
    pub fn dedup_parts(&self) -> Option<(&str, &str)> {
        let message = self.message_id.as_deref().unwrap_or("");
        let request = self.request_id.as_deref().unwrap_or("");
        (!message.is_empty() || !request.is_empty()).then_some((message, request))
    }
}

pub fn append(store: &Path, request: &Request) -> io::Result<()> {
    let _lock = super::lock(store, LOCK)?;
    let path = store.join(FILE);
    if fs::metadata(&path).is_ok_and(|m| m.len() >= ROTATE_AT) {
        fs::rename(&path, store.join(ROTATED))?;
    }
    super::append_line(&path, request)
}

/// Every request both files hold, oldest first.
pub fn read(store: &Path) -> io::Result<Vec<Request>> {
    let mut requests = super::read_lines(&store.join(ROTATED))?;
    requests.extend(super::read_lines(&store.join(FILE))?);
    Ok(requests)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(at: i64) -> Request {
        Request {
            at,
            provider: "claude".into(),
            model: "claude-opus-5-5".into(),
            account_id: Some("acc".into()),
            name: Some("work".into()),
            key: Some("k1".into()),
            format: "messages".into(),
            stream: true,
            outcome: Outcome::Ok,
            status: 200,
            first_byte_ms: Some(120),
            latency_ms: 900,
            tokens: TokenCounts {
                input: 3,
                output: 7,
                ..TokenCounts::default()
            },
            reasoning: 0,
            message_id: Some("msg_1".into()),
            request_id: Some("req_1".into()),
            speed: None,
        }
    }

    #[test]
    fn a_line_is_spelled_as_the_module_says() {
        assert_eq!(
            serde_json::to_string(&request(5)).unwrap(),
            concat!(
                r#"{"at":5,"provider":"claude","model":"claude-opus-5-5","accountId":"acc","#,
                r#""name":"work","key":"k1","format":"messages","stream":true,"outcome":"ok","#,
                r#""status":200,"firstByteMs":120,"latencyMs":900,"tokens":{"i":3,"o":7},"#,
                r#""messageId":"msg_1","requestId":"req_1"}"#
            )
        );
    }

    #[test]
    fn the_dedup_parts_are_the_ids_the_client_got() {
        let mut req = request(1);
        assert_eq!(req.dedup_parts(), Some(("msg_1", "req_1")));
        req.request_id = None;
        assert_eq!(req.dedup_parts(), Some(("msg_1", "")));
        req.message_id = Some(String::new());
        assert_eq!(req.dedup_parts(), None);
    }

    #[test]
    fn a_full_file_rotates_and_both_halves_are_read() {
        let store = crate::usage::temp_store("requests-rotate");
        append(&store, &request(1)).unwrap();
        let file = fs::OpenOptions::new()
            .append(true)
            .open(store.join(FILE))
            .unwrap();
        file.set_len(ROTATE_AT).unwrap();
        append(&store, &request(2)).unwrap();
        assert!(store.join(ROTATED).exists());
        let ats: Vec<i64> = read(&store).unwrap().iter().map(|r| r.at).collect();
        assert_eq!(ats, vec![1, 2]);
        let _ = fs::remove_dir_all(&store);
    }
}
