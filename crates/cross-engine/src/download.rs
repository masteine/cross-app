//! Verified downloads with a content-addressed cache, plus archive extraction.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::catalog::Archive;
use crate::{Error, Progress, Result};

/// Hex SHA-256 of a file.
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|e| Error::io(path, e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| Error::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Return a verified local copy of `archive`, downloading it unless the cache
/// already holds a file with the right hash. Cache entries are named
/// `<sha256>-<file name>`, so a repeat install works offline.
pub fn fetch_verified(
    archive: &Archive,
    label: &str,
    cache_dir: &Path,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<PathBuf> {
    fs::create_dir_all(cache_dir).map_err(|e| Error::io(cache_dir, e))?;
    let cached = cache_dir.join(format!("{}-{}", archive.sha256, archive.file_name()));

    if cached.is_file() {
        progress(Progress::Verifying { label });
        if hash_file(&cached)? == archive.sha256 {
            tracing::info!(%label, "using cached download");
            progress(Progress::CacheHit { label });
            return Ok(cached);
        }
        tracing::warn!(path = %cached.display(), "cached download is corrupt, fetching again");
        fs::remove_file(&cached).map_err(|e| Error::io(&cached, e))?;
    }

    let part = cached.with_extension("part");
    let result = download_to(archive, label, &part, progress).and_then(|actual| {
        if actual == archive.sha256 {
            fs::rename(&part, &cached).map_err(|e| Error::io(&cached, e))
        } else {
            Err(Error::ChecksumMismatch {
                label: label.to_owned(),
                expected: archive.sha256.clone(),
                actual,
            })
        }
    });
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result.map(|()| cached)
}

/// Stream the URL into `dest`, returning the SHA-256 of what was written.
fn download_to(
    archive: &Archive,
    label: &str,
    dest: &Path,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<String> {
    let fail = |message: String| Error::Download {
        url: archive.url.clone(),
        message,
    };
    tracing::info!(%label, url = %archive.url, "downloading");

    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("cross-app/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(None)
        .build()
        .map_err(|e| fail(e.to_string()))?;
    let mut response = client
        .get(&archive.url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| fail(e.to_string()))?;
    let total = response.content_length().or(archive.size);

    let mut file = File::create(dest).map_err(|e| Error::io(dest, e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    progress(Progress::Downloading { label, done, total });
    loop {
        let n = response.read(&mut buf).map_err(|e| fail(e.to_string()))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| Error::io(dest, e))?;
        hasher.update(&buf[..n]);
        done += n as u64;
        progress(Progress::Downloading { label, done, total });
    }
    file.flush().map_err(|e| Error::io(dest, e))?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Fetch a small resource (a manifest, an image) into memory. `Ok(None)` when the
/// server says it does not exist (404/410); larger than `max_bytes` is an error.
pub fn fetch_bytes(url: &str, max_bytes: u64) -> Result<Option<Vec<u8>>> {
    let fail = |message: String| Error::Download {
        url: url.to_owned(),
        message,
    };
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("cross-app/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| fail(e.to_string()))?;
    let response = client.get(url).send().map_err(|e| fail(e.to_string()))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(fail(format!("HTTP status {status}")));
    }
    if response.content_length().is_some_and(|n| n > max_bytes) {
        return Err(fail(format!("larger than {max_bytes} bytes")));
    }
    let mut body = Vec::new();
    response
        .take(max_bytes + 1)
        .read_to_end(&mut body)
        .map_err(|e| fail(e.to_string()))?;
    if body.len() as u64 > max_bytes {
        return Err(fail(format!("larger than {max_bytes} bytes")));
    }
    Ok(Some(body))
}

/// Extract a `.tar`, `.tar.gz` or `.tar.xz` archive into `dest` using the
/// system `tar` (bsdtar on macOS), which refuses absolute and `..` member paths
/// by default. Archives are SHA-256-verified before they get here.
pub fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).map_err(|e| Error::io(dest, e))?;
    let output = Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .output()
        .map_err(|e| Error::Extract {
            archive: archive.to_owned(),
            message: format!("could not run tar: {e}"),
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::Extract {
            archive: archive.to_owned(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve `body` to the next `connections` HTTP GETs on a random local port.
    pub fn serve(body: Vec<u8>, connections: usize) -> String {
        serve_status("200 OK", body, connections)
    }

    /// Like [`serve`] with an explicit status line such as `404 Not Found`.
    pub fn serve_status(status: &'static str, body: Vec<u8>, connections: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for _ in 0..connections {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = Vec::new();
                let mut chunk = [0u8; 512];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&chunk[..n]),
                    }
                }
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    pub fn sha256_hex(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(data))
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{serve, serve_status, sha256_hex};
    use super::*;

    #[test]
    fn fetch_bytes_reads_small_bodies_and_maps_404_to_none() {
        let base = serve(b"hello".to_vec(), 1);
        assert_eq!(
            fetch_bytes(&format!("{base}/x"), 100).unwrap(),
            Some(b"hello".to_vec())
        );

        let base = serve_status("404 Not Found", b"nope".to_vec(), 1);
        assert_eq!(fetch_bytes(&format!("{base}/x"), 100).unwrap(), None);

        let base = serve_status("500 Internal Server Error", b"boom".to_vec(), 1);
        assert!(matches!(
            fetch_bytes(&format!("{base}/x"), 100).unwrap_err(),
            Error::Download { .. }
        ));

        let base = serve(vec![b'x'; 1000], 1);
        let err = fetch_bytes(&format!("{base}/x"), 10).unwrap_err();
        assert!(err.to_string().contains("larger than 10"), "{err}");
    }

    fn archive_for(base: &str, body: &[u8]) -> Archive {
        Archive {
            url: format!("{base}/engine.tar.gz"),
            sha256: sha256_hex(body),
            size: None,
        }
    }

    #[test]
    fn downloads_verifies_and_caches() {
        let body = b"payload".to_vec();
        let base = serve(body.clone(), 1);
        let archive = archive_for(&base, &body);
        let tmp = tempfile::tempdir().unwrap();

        let mut events = 0;
        let path = fetch_verified(&archive, "x", tmp.path(), &mut |_| events += 1).unwrap();
        assert_eq!(fs::read(&path).unwrap(), body);
        assert!(events >= 2);
        assert!(!path.with_extension("part").exists());

        // The server only answered once; a second fetch must come from the cache.
        let mut hit = false;
        let again = fetch_verified(&archive, "x", tmp.path(), &mut |p| {
            hit |= matches!(p, Progress::CacheHit { .. });
        })
        .unwrap();
        assert_eq!(again, path);
        assert!(hit);
    }

    #[test]
    fn corrupt_cache_entry_is_redownloaded() {
        let body = b"payload".to_vec();
        let base = serve(body.clone(), 1);
        let archive = archive_for(&base, &body);
        let tmp = tempfile::tempdir().unwrap();
        let cached = tmp.path().join(format!("{}-engine.tar.gz", archive.sha256));
        fs::write(&cached, b"corrupt").unwrap();

        let path = fetch_verified(&archive, "x", tmp.path(), &mut |_| {}).unwrap();
        assert_eq!(fs::read(path).unwrap(), body);
    }

    #[test]
    fn checksum_mismatch_leaves_nothing_behind() {
        let base = serve(b"tampered".to_vec(), 1);
        let archive = archive_for(&base, b"expected");
        let tmp = tempfile::tempdir().unwrap();

        let err = fetch_verified(&archive, "x", tmp.path(), &mut |_| {}).unwrap_err();
        assert!(matches!(err, Error::ChecksumMismatch { .. }), "{err}");
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn unreachable_server_is_a_download_error() {
        let archive = Archive {
            url: "http://127.0.0.1:1/x.tar.gz".into(),
            sha256: "0".repeat(64),
            size: None,
        };
        let tmp = tempfile::tempdir().unwrap();
        let err = fetch_verified(&archive, "x", tmp.path(), &mut |_| {}).unwrap_err();
        assert!(matches!(err, Error::Download { .. }), "{err}");
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn extracts_tar_gz() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(src.join("a/b")).unwrap();
        fs::write(src.join("a/b/file"), "hi").unwrap();
        let archive = tmp.path().join("x.tar.gz");
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&src)
            .arg(".")
            .status()
            .unwrap()
            .success());

        let dest = tmp.path().join("out");
        extract_archive(&archive, &dest).unwrap();
        assert_eq!(fs::read_to_string(dest.join("a/b/file")).unwrap(), "hi");

        let err = extract_archive(&tmp.path().join("missing.tar.gz"), &dest).unwrap_err();
        assert!(matches!(err, Error::Extract { .. }), "{err}");
    }
}
