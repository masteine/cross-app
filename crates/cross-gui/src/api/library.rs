//! The library page: Steam games and program shortcuts from every bottle,
//! with cover art and compatibility badges.

use std::fs;

use cross_core::paths::AppDirs;
use cross_steam::library::{self, GameState};
use cross_steam::{covers, launch};
use serde::Serialize;

use crate::api::apps::{self, AppKind};
use crate::api::compat::{CompatInfo, CompatQuery, CompatSource};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemKind {
    SteamGame,
    SteamClient,
    Program,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryItem {
    pub kind: ItemKind,
    pub bottle: String,
    pub bottle_name: String,
    pub title: String,
    pub appid: Option<u32>,
    /// Program shortcuts: the path to start.
    pub path: Option<String>,
    pub state: Option<GameState>,
    pub size_on_disk: Option<u64>,
    /// The game has its own settings (`cross steam profile`).
    pub has_profile: bool,
    pub compat: CompatInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct Library {
    pub items: Vec<LibraryItem>,
    /// Bottles whose Steam library could not be read, with the reason.
    pub warnings: Vec<String>,
}

pub fn load(dirs: &AppDirs, compat: &dyn CompatSource) -> Result<Library> {
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    for bottle in cross_bottle::manager::BottleManager::new(dirs).list()? {
        let prefix = dirs.bottle_prefix(&bottle.id);
        for app in apps::views(&bottle) {
            let kind = match app.kind {
                AppKind::Program => ItemKind::Program,
                AppKind::SteamClient => ItemKind::SteamClient,
                AppKind::SteamProfile => continue,
            };
            let compat = match kind {
                ItemKind::Program => compat.lookup(&CompatQuery {
                    steam_appid: None,
                    program: Some(&app.path),
                    title: &app.name,
                }),
                _ => CompatInfo::unknown(),
            };
            items.push(LibraryItem {
                kind,
                bottle: bottle.id.clone(),
                bottle_name: bottle.name.clone(),
                title: app.name,
                appid: None,
                path: Some(app.path),
                state: None,
                size_on_disk: None,
                has_profile: false,
                compat,
            });
        }
        if !cross_steam::install::is_installed(&prefix) {
            continue;
        }
        match library::games(&prefix) {
            Ok(games) => {
                for game in games.into_iter().filter(|g| !g.is_tool) {
                    let has_profile = launch::game_profile(&bottle, game.appid)
                        .is_some_and(|p| *p != Default::default());
                    let compat = compat.lookup(&CompatQuery {
                        steam_appid: Some(game.appid),
                        program: None,
                        title: &game.name,
                    });
                    items.push(LibraryItem {
                        kind: ItemKind::SteamGame,
                        bottle: bottle.id.clone(),
                        bottle_name: bottle.name.clone(),
                        title: game.name,
                        appid: Some(game.appid),
                        path: None,
                        state: Some(game.state),
                        size_on_disk: Some(game.size_on_disk),
                        has_profile,
                        compat,
                    });
                }
            }
            Err(e) => warnings.push(format!("{}: {e}", bottle.id)),
        }
    }
    items.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.bottle.cmp(&b.bottle))
    });
    Ok(Library { items, warnings })
}

/// A game's cover as a `data:` URL for an `<img>`, downloading it into the
/// cover cache on first use (`fetch = false` only looks at the cache).
pub fn cover(dirs: &AppDirs, appid: u32, fetch: bool) -> Result<Option<String>> {
    let path = if fetch {
        covers::fetch_cover(dirs, appid, false)?
    } else {
        Some(covers::cover_path(dirs, appid)).filter(|p| p.is_file())
    };
    let Some(path) = path else {
        return Ok(None);
    };
    let bytes = fs::read(&path).map_err(|e| Error::io(&path, e))?;
    Ok(Some(format!("data:image/jpeg;base64,{}", base64(&bytes))))
}

/// Standard base64 with padding (RFC 4648), enough for cover images.
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameLaunched {
    pub steam_was_running: bool,
    pub has_profile: bool,
    pub log: String,
}

pub fn launch_game(dirs: &AppDirs, bottle: &str, appid: u32) -> Result<GameLaunched> {
    let launched = launch::launch_game(dirs, bottle, appid)?;
    Ok(GameLaunched {
        steam_was_running: launched.steam_was_running,
        has_profile: launched.has_profile,
        log: launched.log.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use cross_core::config::BottleStore;
    use cross_core::model::App;

    use super::*;
    use crate::api::compat::{NoCompatData, Rating};
    use crate::api::test_support::fake_bottle;

    /// A Steam install with one game and one redistributable in the prefix.
    fn fake_steam(dirs: &AppDirs, bottle: &str) {
        let steam = dirs
            .bottle_prefix(bottle)
            .join(cross_steam::STEAM_DIR_IN_PREFIX);
        let apps = steam.join("steamapps");
        fs::create_dir_all(&apps).unwrap();
        fs::write(steam.join("steam.exe"), "MZ").unwrap();
        let manifest = |appid: u32, name: &str| {
            format!(
                "\"AppState\"\n{{\n\t\"appid\"\t\t\"{appid}\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"{name}\"\n\t\"SizeOnDisk\"\t\t\"1024\"\n}}\n"
            )
        };
        fs::write(
            apps.join("appmanifest_220.acf"),
            manifest(220, "Half-Life 2"),
        )
        .unwrap();
        fs::write(
            apps.join("appmanifest_228980.acf"),
            manifest(228980, "Steamworks Common Redistributables"),
        )
        .unwrap();
    }

    #[test]
    fn library_merges_steam_games_and_shortcuts() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let mut bottle = fake_bottle(&dirs, "steam", "wine-a");
        bottle.apps.push(App {
            name: "Steam".into(),
            path: cross_steam::STEAM_EXE_WIN.into(),
            steam_appid: None,
            profile: Default::default(),
        });
        bottle.apps.push(App {
            name: "Notepad++".into(),
            path: r"C:\npp\notepad++.exe".into(),
            steam_appid: None,
            profile: Default::default(),
        });
        BottleStore::new(&dirs).save(&bottle).unwrap();
        fake_steam(&dirs, "steam");
        fake_bottle(&dirs, "empty", "wine-a");

        let lib = load(&dirs, &NoCompatData).unwrap();
        assert!(lib.warnings.is_empty(), "{:?}", lib.warnings);
        let titles: Vec<_> = lib.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Half-Life 2", "Notepad++", "Steam"]);

        let game = &lib.items[0];
        assert_eq!((game.kind, game.appid), (ItemKind::SteamGame, Some(220)));
        assert_eq!(game.state, Some(GameState::Installed));
        assert_eq!(game.compat.rating, Rating::Unknown);
        assert_eq!(lib.items[1].kind, ItemKind::Program);
        assert_eq!(lib.items[2].kind, ItemKind::SteamClient);
        let json = serde_json::to_value(&lib.items[0]).unwrap();
        assert_eq!(json["kind"], "steam-game");
        assert_eq!(json["compat"]["rating"], "unknown");
    }

    #[test]
    fn covers_come_from_the_cache_as_data_urls() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        assert_eq!(cover(&dirs, 220, false).unwrap(), None);
        let path = covers::cover_path(&dirs, 220);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, [0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        // Cached: no network even with fetch = true.
        assert_eq!(
            cover(&dirs, 220, true).unwrap().as_deref(),
            Some("data:image/jpeg;base64,/9j/4A==")
        );
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected, "{input:?}");
        }
    }
}
