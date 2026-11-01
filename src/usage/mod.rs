//! The usage records remuda writes and TokenGauge reads.
//!
//! remuda switches logins and runs the proxy, so only it knows which login a
//! request spent; TokenGauge reads the transcripts and prices them, so only it
//! knows what a request cost. Each needs the other's half, and the files that
//! carry it between them are a contract: a field one side renames is a field
//! the other silently stops seeing. So each format is declared here once, with
//! its writer and its reader, and neither caller spells a key of its own.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Tokens billed at each rate. `input` is *fresh* input: Codex reports cached
/// tokens inside its input count and Anthropic reports them beside them, and
/// every reader normalises onto Anthropic's shape.
///
/// The field names are TokenGauge's sync wire format, which repeats them once
/// per bucket on every upload, hence one or three letters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCounts {
    #[serde(rename = "i", default, skip_serializing_if = "is_zero")]
    pub input: u64,
    #[serde(rename = "o", default, skip_serializing_if = "is_zero")]
    pub output: u64,
    #[serde(rename = "cw5", default, skip_serializing_if = "is_zero")]
    pub cache_write_5m: u64,
    #[serde(rename = "cw1h", default, skip_serializing_if = "is_zero")]
    pub cache_write_1h: u64,
    #[serde(rename = "cr", default, skip_serializing_if = "is_zero")]
    pub cache_read: u64,
}

pub(crate) fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl TokenCounts {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }

    pub fn cache_creation(&self) -> u64 {
        self.cache_write_5m + self.cache_write_1h
    }

    pub fn add(&mut self, other: &TokenCounts) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write_5m += other.cache_write_5m;
        self.cache_write_1h += other.cache_write_1h;
        self.cache_read += other.cache_read;
    }
}

/// How a login is named anywhere its account id must not be written down: the
/// first 16 hex digits of the id's SHA-256. TokenGauge's synced store carries
/// this, and remuda maps it back through the `accountId` in its sidecars.
pub fn account_digest(account_id: &str) -> String {
    Sha256::digest(account_id.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_counts_keep_their_wire_names_and_drop_zeros() {
        let tokens = TokenCounts {
            input: 1,
            output: 2,
            cache_write_5m: 0,
            cache_write_1h: 4,
            cache_read: 5,
        };
        assert_eq!(
            serde_json::to_string(&tokens).unwrap(),
            r#"{"i":1,"o":2,"cw1h":4,"cr":5}"#
        );
        assert_eq!(
            serde_json::from_str::<TokenCounts>("{}").unwrap(),
            TokenCounts::default()
        );
        assert_eq!(tokens.total(), 12);
        assert_eq!(tokens.cache_creation(), 4);
    }

    #[test]
    fn a_digest_is_sixteen_hex_digits_of_the_sha256() {
        assert_eq!(account_digest("abc"), "ba7816bf8f01cfea");
    }
}
