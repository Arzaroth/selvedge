//! `tokengauge --usage-json [--since YYYY-MM-DD]`: what was spent, priced.
//!
//! TokenGauge prints one [`UsageExport`] on stdout, a row per local day,
//! provider, model, login and source. remuda draws it and never prices a
//! token itself, so the two cannot disagree on a figure.
//!
//! A field is only ever added; anything else is a new [`SCHEMA_VERSION`],
//! which an older reader refuses rather than misreads.

use serde::{Deserialize, Serialize};

use super::TokenCounts;

pub const SCHEMA_VERSION: u32 = 1;
pub const FLAG: &str = "--usage-json";
pub const SINCE: &str = "--since";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageExport {
    pub schema_version: u32,
    /// When the export was made, in ms.
    pub generated_at: i64,
    pub currency: String,
    /// The first day the record holds, when it holds any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covers_from: Option<String>,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Read from a CLI's transcripts.
    Cli,
    /// Seen only by remuda's proxy: no transcript holds it.
    Proxy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// Local day, `YYYY-MM-DD`.
    pub date: String,
    pub provider: String,
    pub model: String,
    /// The login's [`account_digest`](super::account_digest), or `None` for
    /// usage from before the ledger or that no record attributes.
    #[serde(default)]
    pub login: Option<String>,
    pub source: Source,
    /// Sessions that started on this row.
    #[serde(default, skip_serializing_if = "super::is_zero")]
    pub sessions: u64,
    pub tokens: TokenCounts,
    pub usd: TypeCosts,
}

/// A row's cost split by the rate it was billed at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeCosts {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
}

impl TypeCosts {
    pub fn total(&self) -> f64 {
        self.input + self.output + self.cache_write + self.cache_read
    }

    pub fn add(&mut self, other: &TypeCosts) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
    }
}

#[derive(Debug)]
pub enum ParseError {
    Json(serde_json::Error),
    /// Written by a TokenGauge newer than this reader understands.
    Newer(u32),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Json(err) => write!(f, "the usage export did not parse: {err}"),
            ParseError::Newer(version) => write!(
                f,
                "the usage export is schema {version}, newer than the {SCHEMA_VERSION} understood here"
            ),
        }
    }
}

impl std::error::Error for ParseError {}

impl UsageExport {
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Version {
            schema_version: u32,
        }
        let version: Version = serde_json::from_slice(bytes).map_err(ParseError::Json)?;
        if version.schema_version > SCHEMA_VERSION {
            return Err(ParseError::Newer(version.schema_version));
        }
        serde_json::from_slice(bytes).map_err(ParseError::Json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export() -> UsageExport {
        UsageExport {
            schema_version: SCHEMA_VERSION,
            generated_at: 1_000,
            currency: "USD".into(),
            covers_from: Some("2026-10-01".into()),
            rows: vec![Row {
                date: "2026-11-01".into(),
                provider: "claude".into(),
                model: "claude-opus-5-5".into(),
                login: Some("ba7816bf8f01cfea".into()),
                source: Source::Proxy,
                sessions: 2,
                tokens: TokenCounts {
                    input: 1,
                    cache_read: 9,
                    ..TokenCounts::default()
                },
                usd: TypeCosts {
                    input: 0.5,
                    output: 0.0,
                    cache_write: 0.0,
                    cache_read: 0.25,
                },
            }],
        }
    }

    #[test]
    fn the_document_is_spelled_as_the_module_says() {
        assert_eq!(
            serde_json::to_string(&export()).unwrap(),
            concat!(
                r#"{"schemaVersion":1,"generatedAt":1000,"currency":"USD","coversFrom":"2026-10-01","#,
                r#""rows":[{"date":"2026-11-01","provider":"claude","model":"claude-opus-5-5","#,
                r#""login":"ba7816bf8f01cfea","source":"proxy","sessions":2,"tokens":{"i":1,"cr":9},"#,
                r#""usd":{"input":0.5,"output":0.0,"cacheWrite":0.0,"cacheRead":0.25}}]}"#
            )
        );
    }

    #[test]
    fn an_export_reads_back_and_a_newer_schema_is_refused() {
        let bytes = serde_json::to_vec(&export()).unwrap();
        assert_eq!(UsageExport::parse(&bytes).unwrap(), export());
        assert_eq!(export().rows[0].usd.total(), 0.75);

        let newer = br#"{"schemaVersion":2,"rows":"anything"}"#;
        assert!(matches!(
            UsageExport::parse(newer),
            Err(ParseError::Newer(2))
        ));
        assert!(matches!(
            UsageExport::parse(b"nonsense"),
            Err(ParseError::Json(_))
        ));
    }

    #[test]
    fn an_unattributed_row_has_a_null_login() {
        let mut export = export();
        export.rows[0].login = None;
        export.rows[0].sessions = 0;
        let text = serde_json::to_string(&export).unwrap();
        assert!(text.contains(r#""login":null,"source""#), "{text}");
        assert!(!text.contains("sessions"), "{text}");
    }
}
