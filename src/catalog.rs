//! What each subscription costs and weighs, and since when.
//!
//! A plan's weight is its allowance as a multiple of the provider's base plan
//! (`unit`), the figure [`crate::plans::pool`] adds plans up by. Both it and
//! the price change on a date - OpenAI doubled every Pro tier on 2026-10-29 -
//! so a plan is a list of terms, each in force from its `from` day until the
//! next one's, and a figure about a past week reads the term of that week.
//!
//! The table is `catalog.json`, compiled in. A term with no `from` held
//! before anything recorded; a missing price or weight is one nobody has
//! verified, never a guess. `pricing` names the page the canary workflow
//! watches and the dollar figures it last saw there.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

const TABLE: &str = include_str!("catalog.json");

#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub providers: BTreeMap<String, Provider>,
}

#[derive(Debug, Deserialize)]
pub struct Provider {
    /// The plan a weight of 1 means, or what the weights count when the
    /// provider sells no multiple.
    pub unit: String,
    pub pricing: Pricing,
    pub plans: BTreeMap<String, Plan>,
}

#[derive(Debug, Deserialize)]
pub struct Pricing {
    pub url: String,
    /// `server` when the page's HTML carries its prices, `client` when a
    /// script fills them in and a plain fetch sees none.
    pub rendered: String,
    /// Every dollar figure the page showed when last checked.
    #[serde(default)]
    pub figures: Vec<f64>,
}

#[derive(Debug, Deserialize)]
pub struct Plan {
    pub name: String,
    /// Oldest first.
    pub terms: Vec<Term>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    /// The first day it held, `YYYY-MM-DD` (UTC). `None` for a term that held
    /// before anything recorded.
    pub from: Option<String>,
    /// The monthly list price in US dollars, billed monthly.
    pub usd_month: Option<f64>,
    pub weight: Option<f64>,
    pub note: Option<String>,
    pub source: Option<String>,
    /// When `source` was last read.
    pub checked: Option<String>,
}

impl Plan {
    /// The term in force on `day` (`YYYY-MM-DD`): the last one whose `from`
    /// is on or before it.
    pub fn on(&self, day: &str) -> Option<&Term> {
        self.terms
            .iter()
            .rev()
            .find(|t| t.from.as_deref().is_none_or(|from| from <= day))
    }

    /// The term in force now.
    pub fn current(&self) -> Option<&Term> {
        self.terms.last()
    }
}

/// The compiled-in table.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| serde_json::from_str(TABLE).expect("catalog.json parses"))
}

/// A provider's plan by the id its API or credentials name it with.
pub fn plan(provider: &str, id: &str) -> Option<&'static Plan> {
    catalog().providers.get(provider)?.plans.get(id)
}

/// What a plan weighs now.
pub fn weight(provider: &str, id: &str) -> Option<f64> {
    plan(provider, id)?.current()?.weight
}

/// The day `ms` (since the epoch) falls on in UTC, `YYYY-MM-DD`, for [`Plan::on`].
pub fn day(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}
