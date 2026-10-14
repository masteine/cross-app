//! Cover art for Steam games, fetched by appid from the Steam CDN and cached.

use std::fs;
use std::path::PathBuf;

use cross_core::paths::AppDirs;

use crate::{Error, Result};

const CDN: &str = "https://cdn.cloudflare.steamstatic.com/steam/apps";
const MAX_IMAGE: u64 = 5 * 1024 * 1024;

/// Candidate images, best first: the portrait library cover, then the wide header.
pub fn cover_urls(appid: u32) -> [String; 2] {
    [
        format!("{CDN}/{appid}/library_600x900.jpg"),
        format!("{CDN}/{appid}/header.jpg"),
    ]
}

/// Where a game's cover is cached.
pub fn cover_path(dirs: &AppDirs, appid: u32) -> PathBuf {
    dirs.root()
        .join("cache/covers")
        .join(format!("{appid}.jpg"))
}

/// The cached cover, downloading it if needed. `Ok(None)` when Steam has no image.
pub fn fetch_cover(dirs: &AppDirs, appid: u32, force: bool) -> Result<Option<PathBuf>> {
    fetch_cover_with(dirs, appid, force, |url| {
        cross_engine::download::fetch_bytes(url, MAX_IMAGE).map_err(Error::from)
    })
}

fn fetch_cover_with(
    dirs: &AppDirs,
    appid: u32,
    force: bool,
    fetch: impl Fn(&str) -> Result<Option<Vec<u8>>>,
) -> Result<Option<PathBuf>> {
    let path = cover_path(dirs, appid);
    if path.is_file() && !force {
        return Ok(Some(path));
    }
    for url in cover_urls(appid) {
        let Some(bytes) = fetch(&url)? else {
            continue;
        };
        // Refuse to cache an HTML error page or anything else that is not a JPEG.
        if !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            tracing::warn!(%url, "not a JPEG, skipping");
            continue;
        }
        let dir = path.parent().unwrap_or(dirs.root());
        fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        let tmp = path.with_extension("jpg.part");
        fs::write(&tmp, &bytes).map_err(|e| Error::io(&tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| Error::io(&path, e))?;
        return Ok(Some(path));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];

    #[test]
    fn urls_follow_the_cdn_layout() {
        assert_eq!(
            cover_urls(220),
            [
                "https://cdn.cloudflare.steamstatic.com/steam/apps/220/library_600x900.jpg",
                "https://cdn.cloudflare.steamstatic.com/steam/apps/220/header.jpg",
            ]
        );
    }

    #[test]
    fn caches_the_first_available_jpeg() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let calls = RefCell::new(Vec::new());
        // Portrait cover missing (404), header present.
        let fetch = |url: &str| {
            calls.borrow_mut().push(url.to_owned());
            Ok(url.ends_with("header.jpg").then(|| JPEG.to_vec()))
        };
        let path = fetch_cover_with(&dirs, 220, false, fetch).unwrap().unwrap();
        assert_eq!(fs::read(&path).unwrap(), JPEG);
        assert_eq!(calls.borrow().len(), 2);

        // Cached: no further requests unless forced.
        fetch_cover_with(&dirs, 220, false, fetch).unwrap();
        assert_eq!(calls.borrow().len(), 2);
        fetch_cover_with(&dirs, 220, true, fetch).unwrap();
        assert_eq!(calls.borrow().len(), 4);
    }

    #[test]
    fn rejects_non_images_and_reports_missing_art() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let html = |_: &str| Ok(Some(b"<html>blocked</html>".to_vec()));
        assert_eq!(fetch_cover_with(&dirs, 1, false, html).unwrap(), None);
        assert!(!cover_path(&dirs, 1).exists());
        assert_eq!(
            fetch_cover_with(&dirs, 1, false, |_| Ok(None)).unwrap(),
            None
        );
        let failing = |_: &str| Err(Error::InvalidInput("offline".into()));
        assert!(fetch_cover_with(&dirs, 1, false, failing).is_err());
    }
}
