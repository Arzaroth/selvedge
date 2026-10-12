//! Several plans at one vendor, added up into one meter.
//!
//! TokenGauge draws the sum in its panels and remuda on its page, and the two
//! used to agree only because each had a copy of the same arithmetic in its
//! own language. The figures are numbers here; the words around them - a
//! label, a tone, "estimate" - are the caller's.
//!
//! Every plan is counted in units of the largest one, so a Max 20x and a Pro
//! both spent read "105% of 105%": each contributes `used × weight ÷ largest`,
//! and the capacity is the same sum at 100%. Counting every plan as 1 is the
//! same arithmetic with equal weights, so five plans read "409% of 500%".

use serde::Serialize;

/// One plan's usage of a window and what it weighs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Share {
    /// Percent of the window used, 0 to 100.
    pub used: f64,
    /// The plan's multiplier, in any unit the caller keeps the same for every
    /// share. Not positive or not finite, and the share is not counted.
    pub weight: f64,
}

/// The shares added up.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pooled {
    /// What is used, in percent of the largest plan: the "N" of "N% of M%".
    pub used: f64,
    /// Every plan at 100%, in the same unit: the "M".
    pub capacity: f64,
    /// `Σ used × weight ÷ Σ weight`, from 0 to 1: what a pooled bar fills to,
    /// so an idle small plan cannot make two busy large ones look free.
    pub fraction: f64,
    /// One per counted share, in the order given.
    pub segments: Vec<Segment>,
}

/// A share's stretch of a split bar.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Segment {
    /// Its share of the bar's width, from [`segment_widths`].
    pub width: f64,
    /// How full it is, from 0 to 1: the plan's own usage.
    pub fill: f64,
}

impl Pooled {
    /// "N% of M%", rounded as [`whole_percent`] rounds.
    pub fn label(&self) -> String {
        format!(
            "{}% of {}%",
            whole_percent(self.used),
            whole_percent(self.capacity)
        )
    }
}

fn counted(s: &Share) -> bool {
    s.weight.is_finite() && s.weight > 0.0 && s.used.is_finite()
}

/// Adds the shares up. `None` when no share counts.
pub fn pool(shares: &[Share]) -> Option<Pooled> {
    let counted: Vec<&Share> = shares.iter().filter(|s| counted(s)).collect();
    let largest = counted.iter().map(|s| s.weight).fold(0.0_f64, f64::max);
    if largest <= 0.0 {
        return None;
    }
    let total: f64 = counted.iter().map(|s| s.weight).sum();
    let weighted: f64 = counted.iter().map(|s| s.used * s.weight).sum();
    let weights: Vec<f64> = counted.iter().map(|s| s.weight).collect();
    Some(Pooled {
        used: weighted / largest,
        capacity: total / largest * 100.0,
        fraction: (weighted / total / 100.0).clamp(0.0, 1.0),
        segments: counted
            .iter()
            .zip(segment_widths(&weights))
            .map(|(s, width)| Segment {
                width,
                fill: (s.used / 100.0).clamp(0.0, 1.0),
            })
            .collect(),
    })
}

/// A percentage as a whole number, a half rounded to even: 12.5 is 12, 13.5 is
/// 14. What Rust's `{:.0}` prints, so a figure reads the same wherever it is
/// drawn.
pub fn whole_percent(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// A split bar's narrowest segment, as a share of the bar.
pub const MIN_SEGMENT_WIDTH: f64 = 0.1;

/// Each weight's share of a split bar, none under [`MIN_SEGMENT_WIDTH`]. A
/// Max 20x beside two Pro seats would otherwise draw them as slivers next to
/// one bar that reads as pooled. Segments held at the minimum are taken out
/// and the rest is shared again by weight, until none falls under it.
pub fn segment_widths(weights: &[f64]) -> Vec<f64> {
    let n = weights.len();
    if n == 0 {
        return Vec::new();
    }
    if MIN_SEGMENT_WIDTH * n as f64 >= 1.0 {
        return vec![1.0 / n as f64; n];
    }
    let mut pinned = vec![false; n];
    loop {
        let free = 1.0 - MIN_SEGMENT_WIDTH * pinned.iter().filter(|p| **p).count() as f64;
        let weight: f64 = weights
            .iter()
            .zip(&pinned)
            .filter(|(_, p)| !**p)
            .map(|(w, _)| w)
            .sum();
        let widths: Vec<f64> = weights
            .iter()
            .zip(&pinned)
            .map(|(w, p)| {
                if *p {
                    MIN_SEGMENT_WIDTH
                } else {
                    w / weight * free
                }
            })
            .collect();
        let mut moved = false;
        for (i, width) in widths.iter().enumerate() {
            if !pinned[i] && *width < MIN_SEGMENT_WIDTH {
                pinned[i] = true;
                moved = true;
            }
        }
        if !moved {
            return widths;
        }
    }
}

/// A Claude Team seat's weight against a Pro: a standard seat is sold at
/// 1.25x Pro's per-session allowance, a premium one at 6.25x
/// (support.claude.com, "What is the Team plan?").
pub const TEAM_STANDARD_WEIGHT: f64 = 1.25;
/// See [`TEAM_STANDARD_WEIGHT`].
pub const TEAM_PREMIUM_WEIGHT: f64 = 6.25;

/// The seat a Claude Team login holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    Standard,
    Premium,
}

impl Seat {
    /// What the seat weighs, in Pros.
    pub fn weight(self) -> f64 {
        match self {
            Seat::Standard => TEAM_STANDARD_WEIGHT,
            Seat::Premium => TEAM_PREMIUM_WEIGHT,
        }
    }
}

/// The `N` of a rate-limit tier that names an `Nx` (`default_claude_max_5x`
/// is 5). `None` for a tier that names none (`default_raven`).
pub fn tier_multiplier(tier: &str) -> Option<u32> {
    tier.split(|c: char| !c.is_ascii_alphanumeric())
        .filter_map(|word| word.strip_suffix(['x', 'X']))
        .filter(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .find_map(|digits| digits.parse::<u32>().ok().filter(|n| *n > 0))
}

/// The seat a Claude Team login holds, from what its profile and tokens say.
///
/// A rate-limit tier is the seat's own allowance, so one that names a
/// multiplier or says `premium` is a premium seat, whatever `seat_tier` is
/// called: a premium seat has been reported as `team_tier_1` beside a user
/// tier of `default_claude_max_5x`. The tiers to pass are every one the login
/// carries - `.claude.json`'s `userRateLimitTier`, the token's
/// `rateLimitTier` - since a token issued to a premium seat can still carry
/// the organization's `default_raven`. Without one, `seat_tier` decides when
/// it is `team_premium` or `team_standard`, and anything else is `None`.
pub fn team_seat(seat_tier: Option<&str>, rate_limit_tiers: &[Option<&str>]) -> Option<Seat> {
    let premium_tier = rate_limit_tiers.iter().flatten().any(|tier| {
        tier_multiplier(tier).is_some() || tier.to_ascii_lowercase().contains("premium")
    });
    if premium_tier {
        return Some(Seat::Premium);
    }
    match seat_tier?.trim().to_ascii_lowercase().as_str() {
        "team_premium" => Some(Seat::Premium),
        "team_standard" => Some(Seat::Standard),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tier_names_its_multiplier_or_none() {
        assert_eq!(tier_multiplier("default_claude_max_20x"), Some(20));
        assert_eq!(tier_multiplier("default_claude_max_5x"), Some(5));
        assert_eq!(tier_multiplier("default_claude_team_5X"), Some(5));
        assert_eq!(tier_multiplier("default_raven"), None);
        assert_eq!(tier_multiplier("default_claude_max_0x"), None);
        assert_eq!(tier_multiplier("0x_default_claude_max_5x"), Some(5));
        assert_eq!(tier_multiplier("x"), None);
        assert_eq!(tier_multiplier(""), None);
    }

    #[test]
    fn a_premium_seat_reads_from_its_tier_whatever_the_seat_is_called() {
        assert_eq!(
            team_seat(
                Some("team_tier_1"),
                &[Some("default_claude_max_5x"), Some("default_raven")]
            ),
            Some(Seat::Premium)
        );
        assert_eq!(
            team_seat(Some("team_tier_1"), &[None, Some("default_claude_max_5x")]),
            Some(Seat::Premium)
        );
        assert_eq!(
            team_seat(None, &[Some("default_raven_premium")]),
            Some(Seat::Premium)
        );
        assert_eq!(
            team_seat(Some("team_tier_1"), &[Some("default_raven")]),
            None
        );
    }

    #[test]
    fn without_a_tier_the_seat_tier_decides() {
        assert_eq!(
            team_seat(Some("team_standard"), &[Some("default_raven"), None]),
            Some(Seat::Standard)
        );
        assert_eq!(team_seat(Some(" TEAM_PREMIUM "), &[]), Some(Seat::Premium));
        assert_eq!(team_seat(Some("team_premium_trial"), &[]), None);
        assert_eq!(team_seat(Some("enterprise_premium"), &[]), None);
        assert_eq!(team_seat(None, &[None]), None);
        assert_eq!(Seat::Standard.weight(), 1.25);
        assert_eq!(Seat::Premium.weight(), 6.25);
    }

    fn close(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    fn share(used: f64, weight: f64) -> Share {
        Share { used, weight }
    }

    #[test]
    fn plans_count_in_units_of_the_largest() {
        let both_spent = pool(&[share(100.0, 20.0), share(100.0, 1.0)]).unwrap();
        assert_eq!(both_spent.label(), "105% of 105%");
        assert_eq!(both_spent.fraction, 1.0);

        let half_and_full = pool(&[share(50.0, 20.0), share(100.0, 5.0)]).unwrap();
        assert_eq!(half_and_full.label(), "75% of 125%");
        assert!((half_and_full.fraction - 0.6).abs() < 1e-9);
    }

    #[test]
    fn an_idle_small_plan_does_not_make_busy_ones_look_free() {
        let p = pool(&[share(90.0, 20.0), share(90.0, 20.0), share(0.0, 1.0)]).unwrap();
        assert!(p.fraction > 0.85, "{}", p.fraction);
    }

    #[test]
    fn equal_weights_count_every_plan_as_one() {
        let shares: Vec<Share> = [100.0, 100.0, 100.0, 100.0, 9.0]
            .into_iter()
            .map(|used| share(used, 1.0))
            .collect();
        assert_eq!(pool(&shares).unwrap().label(), "409% of 500%");
    }

    #[test]
    fn a_fractional_weight_counts_as_its_fraction() {
        let p = pool(&[share(0.0, 20.0), share(100.0, 1.25), share(100.0, 1.25)]).unwrap();
        assert_eq!(p.label(), "12% of 112%");
    }

    #[test]
    fn a_share_with_no_usable_weight_is_left_out() {
        let p = pool(&[share(40.0, 1.0), share(100.0, 0.0), share(100.0, f64::NAN)]).unwrap();
        assert_eq!(p.label(), "40% of 100%");
        assert_eq!(p.segments.len(), 1);
        assert_eq!(pool(&[share(10.0, 0.0)]), None);
        assert_eq!(pool(&[]), None);
    }

    #[test]
    fn a_half_rounds_to_even() {
        assert_eq!(whole_percent(12.5), 12);
        assert_eq!(whole_percent(13.5), 14);
        assert_eq!(whole_percent(105.0), 105);
        assert_eq!(format!("{:.0}", 12.5), whole_percent(12.5).to_string());
    }

    #[test]
    fn each_segment_fills_to_its_own_usage() {
        let p = pool(&[share(90.0, 4.0), share(100.0, 1.0)]).unwrap();
        assert!(close(
            &p.segments.iter().map(|s| s.width).collect::<Vec<_>>(),
            &[0.8, 0.2]
        ));
        assert!(close(
            &p.segments.iter().map(|s| s.fill).collect::<Vec<_>>(),
            &[0.9, 1.0]
        ));
    }

    #[test]
    fn a_small_plan_stays_wide_enough_to_read() {
        assert!(close(&segment_widths(&[20.0, 1.0, 1.0]), &[0.8, 0.1, 0.1]));
        assert!(close(
            &segment_widths(&[20.0, 5.0, 1.0]),
            &[0.72, 0.18, 0.1]
        ));
        assert!(close(
            &segment_widths(&[20.0, 20.0, 20.0, 5.0, 5.0, 1.0]),
            &[7.0 / 30.0, 7.0 / 30.0, 7.0 / 30.0, 0.1, 0.1, 0.1]
        ));
    }

    #[test]
    fn ten_or_more_plans_share_the_bar_evenly() {
        assert!(close(&segment_widths(&[1.0; 12]), &[1.0 / 12.0; 12]));
        let mut weights = vec![20.0];
        weights.extend([1.0; 9]);
        assert!(close(&segment_widths(&weights), &[0.1; 10]));
        assert!(close(&segment_widths(&[5.0]), &[1.0]));
        assert!(segment_widths(&[]).is_empty());
    }
}
