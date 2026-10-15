//! `cross compat ...`: game compatibility profiles.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, Result};
use clap::Subcommand;
use cross_compat::db::{self, Layer};
use cross_compat::{CompatDb, Entry, GameProfile, MatchKind};
use cross_core::config::BottleStore;
use cross_core::paths::AppDirs;

#[derive(Subcommand)]
pub enum CompatCommand {
    /// List compatibility profiles (built-in, community, local)
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show one profile, by id or Steam appid
    Show {
        profile: String,
        #[arg(long)]
        json: bool,
    },
    /// Find the profile for a Steam game or a Windows program on this Mac
    Match {
        #[arg(long, conflicts_with = "exe", required_unless_present = "exe")]
        appid: Option<u32>,
        /// Path of the .exe on this Mac (matched by SHA-256, then by name)
        #[arg(long)]
        exe: Option<PathBuf>,
    },
    /// Write a profile's settings into a bottle's game entry
    Apply {
        bottle: String,
        /// Profile id or Steam appid
        profile: String,
        /// The program's path (Windows path or a path on this Mac), for games
        /// not started through Steam
        #[arg(long)]
        exe: Option<PathBuf>,
    },
    /// Fetch a community bundle.toml over https (only a newer revision is taken)
    Sync {
        #[arg(long)]
        url: String,
        /// Accept the bundle even if its revision is not newer
        #[arg(long)]
        force: bool,
    },
    /// Check a profile file and keep it as a local override
    Import { file: PathBuf },
    /// Print a bundle.toml built from a repository's profiles/ directory
    Bundle {
        dir: PathBuf,
        #[arg(long)]
        revision: u32,
    },
}

/// Bottle that should receive this command's log output.
pub fn log_bottle(command: &CompatCommand) -> Option<&str> {
    match command {
        CompatCommand::Apply { bottle, .. } => Some(bottle),
        _ => None,
    }
}

pub fn run(command: CompatCommand, dirs: &AppDirs) -> Result<ExitCode> {
    match command {
        CompatCommand::List { json } => {
            let db = load(dirs)?;
            if json {
                let rows: Vec<_> = db.entries().collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                println!(
                    "{:22}  {:>8}  {:20}  {:9}  NAME",
                    "ID", "APPID", "RATING", "FROM"
                );
                for e in db.entries() {
                    let p = &e.profile;
                    println!(
                        "{:22}  {:>8}  {:20}  {:9}  {}",
                        p.id,
                        p.steam_appid
                            .map_or_else(|| "-".to_owned(), |a| a.to_string()),
                        p.badge_text(),
                        layer_name(&e.layer),
                        p.name
                    );
                }
            }
        }
        CompatCommand::Show { profile, json } => {
            let db = load(dirs)?;
            let entry = find(&db, &profile)?;
            if json {
                println!("{}", serde_json::to_string_pretty(entry)?);
            } else {
                print!("{}", entry.profile.to_toml()?);
                println!("# from: {}", layer_name(&entry.layer));
            }
        }
        CompatCommand::Match { appid, exe } => {
            let db = load(dirs)?;
            let found = match (appid, exe) {
                (Some(appid), _) => db.by_appid(appid).map(|e| (e, MatchKind::SteamAppid)),
                (None, Some(exe)) => db.by_exe(&exe)?.map(|m| (m.entry, m.kind)),
                (None, None) => None,
            };
            match found {
                Some((entry, kind)) => {
                    let how = match kind {
                        MatchKind::SteamAppid => "Steam appid",
                        MatchKind::ExeSha256 => "exact executable (SHA-256)",
                        MatchKind::ExeName => "executable name only",
                    };
                    println!(
                        "{} ({}): {} [matched by {how}]",
                        entry.profile.name,
                        entry.profile.id,
                        entry.profile.badge_text()
                    );
                }
                None => {
                    println!("No compatibility profile matches.");
                    return Ok(ExitCode::from(1));
                }
            }
        }
        CompatCommand::Apply {
            bottle,
            profile,
            exe,
        } => {
            let db = load(dirs)?;
            let entry = find(&db, &profile)?;
            let store = BottleStore::new(dirs);
            let mut b = store.load(&bottle)?;
            let applied = cross_compat::apply_to_bottle(&mut b, &entry.profile, exe.as_deref())?;
            store.save(&b)?;
            println!(
                "Applied profile {} to {} in bottle {bottle}.",
                entry.profile.id, applied.app
            );
            print_cautions(&entry.profile);
            if !applied.missing_deps.is_empty() {
                println!(
                    "Install its dependencies: cross deps install {bottle} {}",
                    applied.missing_deps.join(" ")
                );
            }
        }
        CompatCommand::Sync { url, force } => {
            let out = db::sync_from(dirs, &url, force)?;
            let from = out
                .from
                .map_or_else(|| "none".to_owned(), |r| r.to_string());
            if out.updated {
                println!(
                    "Community profiles updated: revision {from} -> {} ({} profiles).",
                    out.to, out.profiles
                );
            } else {
                println!(
                    "Already up to date (have revision {from}, fetched {}).",
                    out.to
                );
            }
        }
        CompatCommand::Import { file } => {
            let p = db::import_local(dirs, &file)?;
            println!("Imported {} as a local profile ({}).", p.id, p.badge_text());
        }
        CompatCommand::Bundle { dir, revision } => {
            print!("{}", db::make_bundle(&dir, revision)?);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn load(dirs: &AppDirs) -> Result<CompatDb> {
    let db = CompatDb::load(dirs)?;
    for problem in &db.problems {
        eprintln!("warning: skipped {problem}");
    }
    Ok(db)
}

fn find<'a>(db: &'a CompatDb, key: &str) -> Result<&'a Entry> {
    db.get(key)
        .or_else(|| key.parse().ok().and_then(|appid| db.by_appid(appid)))
        .ok_or_else(|| anyhow!("no compatibility profile {key:?}; see `cross compat list`"))
}

fn layer_name(layer: &Layer) -> &'static str {
    match layer {
        Layer::Builtin => "built-in",
        Layer::Community => "community",
        Layer::Local => "local",
    }
}

fn print_cautions(p: &GameProfile) {
    if !p.is_verified() {
        println!("note: this profile is unverified (from: {})", p.source.name);
    }
    if let Some(ac) = &p.anticheat {
        println!("note: uses {ac}, which does not run under Wine on macOS");
    }
    for bug in &p.known_bugs {
        println!("known issue: {bug}");
    }
}
