//! Compatibility badges (green / yellow / red) on library cards.
//!
//! The ratings themselves come from `cross-compat`, which is built in Sprint 7.
//! Until then the GUI talks to this small trait and ships [`NoCompatData`], so
//! every card shows "unknown" instead of a made-up rating. Plugging in the real
//! database means implementing [`CompatSource`] and passing it to the app state.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Rating {
    /// Runs out of the box.
    Green,
    /// Runs with known issues or manual steps.
    Yellow,
    /// Does not run (anti-cheat, missing API, ...).
    Red,
    /// No data.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompatInfo {
    pub rating: Rating,
    /// Short explanation shown as the badge tooltip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl CompatInfo {
    pub fn unknown() -> Self {
        Self {
            rating: Rating::Unknown,
            note: None,
        }
    }
}

/// What a rating can be looked up by: a Steam appid, or the program path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompatQuery<'a> {
    pub steam_appid: Option<u32>,
    pub program: Option<&'a str>,
    pub title: &'a str,
}

pub trait CompatSource: Send + Sync {
    fn lookup(&self, query: &CompatQuery<'_>) -> CompatInfo;
}

/// Placeholder until `cross-compat` exists: knows nothing about any game.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoCompatData;

impl CompatSource for NoCompatData {
    fn lookup(&self, _query: &CompatQuery<'_>) -> CompatInfo {
        CompatInfo::unknown()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_never_invents_ratings() {
        let info = NoCompatData.lookup(&CompatQuery {
            steam_appid: Some(220),
            program: None,
            title: "Half-Life 2",
        });
        assert_eq!(info, CompatInfo::unknown());
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            serde_json::json!({ "rating": "unknown" })
        );
    }
}
