//! Each provider's mark and colour, so TokenGauge and remuda draw it the same.
//!
//! A mark is an SVG in `currentColor`, for the caller to tint; the only fill it
//! fixes is a part of the brand that is never drawn any other way, Kimi's blue
//! dot. A brand that is black and white has no colour: the caller draws it in
//! its own foreground, which is what keeps it visible on a dark panel and a
//! light page alike.

/// One provider's mark and colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brand {
    /// The provider's id, as TokenGauge's snapshot and remuda's store name it.
    pub id: &'static str,
    /// A complete SVG document.
    pub svg: &'static str,
    /// `#RRGGBB`, or `None` for a brand drawn in the caller's foreground.
    pub color: Option<&'static str>,
    /// The mark is drawn in the caller's foreground even though the brand has
    /// a colour: Kimi's K is black beside its blue dot.
    pub foreground_mark: bool,
}

impl Brand {
    /// The colour to draw the mark's `currentColor` in, or `None` for the
    /// caller's foreground.
    pub fn mark_color(&self) -> Option<&'static str> {
        if self.foreground_mark {
            None
        } else {
            self.color
        }
    }
}

/// Every provider with a mark, ordered by id.
pub const BRANDS: &[Brand] = &[
    Brand {
        id: "claude",
        svg: include_str!("../assets/providers/claude.svg"),
        color: Some("#DE7356"),
        foreground_mark: false,
    },
    Brand {
        id: "codex",
        svg: include_str!("../assets/providers/codex.svg"),
        color: Some("#74AA9C"),
        foreground_mark: false,
    },
    Brand {
        id: "cursor",
        svg: include_str!("../assets/providers/cursor.svg"),
        color: Some("#6C7086"),
        foreground_mark: false,
    },
    Brand {
        id: "glm",
        svg: include_str!("../assets/providers/glm.svg"),
        color: None,
        foreground_mark: false,
    },
    Brand {
        id: "grok",
        svg: include_str!("../assets/providers/grok.svg"),
        color: None,
        foreground_mark: false,
    },
    Brand {
        id: "kimi",
        svg: include_str!("../assets/providers/kimi.svg"),
        color: Some("#1783FF"),
        foreground_mark: true,
    },
    Brand {
        id: "opencode",
        svg: include_str!("../assets/providers/opencode.svg"),
        color: None,
        foreground_mark: false,
    },
    Brand {
        id: "openrouter",
        svg: include_str!("../assets/providers/openrouter.svg"),
        color: Some("#6467F2"),
        foreground_mark: false,
    },
];

/// The brand of a provider id, or `None` for one with no mark.
pub fn brand(id: &str) -> Option<&'static Brand> {
    BRANDS.iter().find(|b| b.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_sorted_and_unique() {
        assert!(BRANDS.windows(2).all(|w| w[0].id < w[1].id));
    }

    #[test]
    fn every_mark_is_an_svg_with_a_view_box_the_caller_can_tint() {
        for b in BRANDS {
            assert!(
                b.svg.trim_start().starts_with("<svg") || b.svg.contains("\n<svg"),
                "{}",
                b.id
            );
            assert!(b.svg.contains("viewBox=\""), "{}", b.id);
            assert!(b.svg.contains("currentColor"), "{}", b.id);
        }
    }

    #[test]
    fn colours_are_hex() {
        for c in BRANDS.iter().filter_map(|b| b.color) {
            assert!(c.len() == 7 && c.starts_with('#'), "{c}");
            assert!(c[1..].chars().all(|d| d.is_ascii_hexdigit()), "{c}");
        }
    }

    #[test]
    fn brand_finds_a_provider_and_not_a_stranger() {
        assert_eq!(brand("kimi").and_then(|b| b.color), Some("#1783FF"));
        assert_eq!(brand("grok").map(|b| b.color), Some(None));
        assert!(brand("nobody").is_none());
    }

    #[test]
    fn kimi_draws_its_k_in_the_foreground_and_claude_in_its_colour() {
        assert_eq!(brand("kimi").unwrap().mark_color(), None);
        assert_eq!(brand("claude").unwrap().mark_color(), Some("#DE7356"));
    }
}
