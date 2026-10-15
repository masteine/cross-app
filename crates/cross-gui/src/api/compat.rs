//! Compatibility badges (green / yellow / red) on library cards.
//!
//! The ratings come from `cross-compat` ([`ProfileDb`]); [`NoCompatData`] is the
//! fallback when its profiles cannot be loaded, so a card then shows "unknown"
//! instead of a made-up rating.

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

/// Ratings from the `cross-compat` profiles (built-in, community, local).
pub struct ProfileDb(cross_compat::CompatDb);

impl ProfileDb {
    pub fn new(db: cross_compat::CompatDb) -> Self {
        Self(db)
    }

    pub fn load(dirs: &cross_core::paths::AppDirs) -> cross_compat::Result<Self> {
        cross_compat::CompatDb::load(dirs).map(Self)
    }
}

impl CompatSource for ProfileDb {
    /// By Steam appid, else by executable file name. Executables are not hashed
    /// here: a card is drawn for every game, and games can be gigabytes.
    fn lookup(&self, query: &CompatQuery<'_>) -> CompatInfo {
        let by_appid = query.steam_appid.and_then(|id| self.0.by_appid(id));
        let by_name = || {
            let name = std::path::Path::new(query.program?).file_name()?.to_str()?;
            self.0.entries().find(|e| e.profile.matches_exe_name(name))
        };
        let Some(entry) = by_appid.or_else(by_name) else {
            return CompatInfo::unknown();
        };
        let profile = &entry.profile;
        CompatInfo {
            rating: match profile.rating {
                cross_compat::Rating::Green => Rating::Green,
                cross_compat::Rating::Yellow => Rating::Yellow,
                cross_compat::Rating::Red => Rating::Red,
            },
            // Says "(unverified)" for profiles nobody has confirmed yet.
            note: Some(format!("{}: {}", profile.name, profile.badge_text())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_db_rates_by_appid_and_marks_unverified() {
        let db = ProfileDb::new(cross_compat::CompatDb::builtin());
        let appid =
            db.0.entries()
                .find_map(|e| e.profile.steam_appid)
                .expect("built-in profiles include Steam games");
        let info = db.lookup(&CompatQuery {
            steam_appid: Some(appid),
            program: None,
            title: "",
        });
        assert_ne!(info.rating, Rating::Unknown);
        assert!(info.note.unwrap().contains("unverified"));

        let none = db.lookup(&CompatQuery {
            steam_appid: Some(1),
            program: None,
            title: "",
        });
        assert_eq!(none, CompatInfo::unknown());
    }

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
