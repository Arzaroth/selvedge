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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_parses_and_every_plan_has_a_term() {
        for (provider, p) in &catalog().providers {
            assert!(!p.plans.is_empty(), "{provider}");
            for (id, plan) in &p.plans {
                assert!(!plan.terms.is_empty(), "{provider}/{id}");
            }
        }
    }

    #[test]
    fn terms_are_dated_in_order_and_only_the_first_is_open_ended() {
        for (provider, p) in &catalog().providers {
            for (id, plan) in &p.plans {
                let dated: Vec<&str> = plan
                    .terms
                    .iter()
                    .filter_map(|t| t.from.as_deref())
                    .collect();
                let open = plan.terms.len() - dated.len();
                assert!(
                    open <= 1 && plan.terms.iter().skip(open).all(|t| t.from.is_some()),
                    "{provider}/{id}"
                );
                assert!(dated.windows(2).all(|w| w[0] < w[1]), "{provider}/{id}");
                assert!(
                    dated.iter().all(|d| d.len() == 10 && day_parts(d)),
                    "{provider}/{id}"
                );
            }
        }
    }

    fn day_parts(day: &str) -> bool {
        let parts: Vec<&str> = day.split('-').collect();
        parts.len() == 3 && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()))
    }

    #[test]
    fn a_sourced_term_says_when_it_was_read() {
        for p in catalog().providers.values() {
            for plan in p.plans.values() {
                for t in &plan.terms {
                    assert_eq!(t.source.is_some(), t.checked.is_some(), "{}", plan.name);
                    assert!(
                        t.usd_month.is_none_or(|u| u > 0.0) && t.weight.is_none_or(|w| w > 0.0)
                    );
                }
            }
        }
    }

    #[test]
    fn a_week_reads_the_term_in_force_that_day() {
        let pro = plan("codex", "pro").unwrap();
        assert_eq!(pro.on("2026-09-01").unwrap().weight, Some(20.0));
        assert_eq!(pro.on("2026-10-15").unwrap().weight, Some(10.0));
        assert_eq!(pro.on("2026-10-29").unwrap().weight, Some(20.0));
        assert_eq!(pro.current().unwrap().usd_month, Some(200.0));
        let promax = plan("codex", "promax").unwrap();
        assert_eq!(promax.on("2026-09-28"), None);
        assert_eq!(weight("codex", "promax"), Some(50.0));
        assert_eq!(weight("codex", "nope"), None);
        assert_eq!(weight("nope", "pro"), None);
    }

    #[test]
    fn the_codex_weights_are_what_tokengauge_weighs_now() {
        for (id, w) in [
            ("plus", 1.0),
            ("team", 1.0),
            ("prolite", 10.0),
            ("self_serve_business_prolite", 10.0),
            ("pro", 20.0),
            ("promax", 50.0),
        ] {
            assert_eq!(weight("codex", id), Some(w), "{id}");
        }
    }

    #[test]
    fn a_millisecond_falls_on_its_utc_day() {
        assert_eq!(day(0), "1970-01-01");
        assert_eq!(day(-1), "1969-12-31");
        assert_eq!(day(1_793_750_400_000 - 1), "2026-11-03");
        assert_eq!(day(1_793_750_400_000), "2026-11-04");
        assert_eq!(day(951_782_400_000), "2000-02-29");
    }
}
