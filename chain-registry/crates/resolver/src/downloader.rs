// crates/resolver/src/downloader.rs
// Bounded, verifying package downloader (ECA-14, ADR-019 Increment 1).
//
// Every attempt:
// - validates the CID grammar before it is interpolated into a URL path;
// - streams the body through an incremental SHA-256 into a file created with
//   `create_new` inside a private per-download temp directory (0700 on Unix,
//   the per-user %TEMP% on Windows), never a predictable shared path;
// - aborts as soon as the body exceeds `max_bytes` (default 64 MiB, matching
//   the validator-side `INV-09` budget) or the request exceeds its timeout;
// - deletes its partial file on failure or cancellation.
//
// Attempts race across the configured nodes; the first verified body wins,
// the losing futures are dropped (cancelled) and their files removed. The
// returned `VerifiedDownload` owns the temp directory, so the verified file
// exists exactly as long as the handle does.

use anyhow::{bail, Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default upper bound for one package body.
pub const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;
/// Operator override for the body cap, in bytes.
pub const MAX_DOWNLOAD_BYTES_ENV: &str = "CREG_MAX_DOWNLOAD_BYTES";
/// Default whole-request timeout for one attempt.
pub const DEFAULT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A downloaded file whose SHA-256 matched the expected content hash.
/// Dropping the handle deletes the file and its private directory.
#[derive(Debug)]
pub struct VerifiedDownload {
    dir: tempfile::TempDir,
    path: PathBuf,
    len: u64,
}

impl VerifiedDownload {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Private directory that holds the verified file.
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }
}

pub struct P2PDownloader {
    pub nodes: Vec<String>,
    pub max_bytes: u64,
    pub timeout: Duration,
    temp_root: Option<PathBuf>,
}

impl P2PDownloader {
    /// Downloader with the default limits (`CREG_MAX_DOWNLOAD_BYTES` may
    /// lower or raise the cap; invalid or zero values fall back to the default).
    pub fn new(nodes: Vec<String>) -> Self {
        let max_bytes = std::env::var(MAX_DOWNLOAD_BYTES_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MAX_DOWNLOAD_BYTES);
        Self::with_limits(nodes, max_bytes, DEFAULT_DOWNLOAD_TIMEOUT)
    }

    pub fn with_limits(nodes: Vec<String>, max_bytes: u64, timeout: Duration) -> Self {
        Self {
            nodes,
            max_bytes: max_bytes.max(1),
            timeout,
            temp_root: None,
        }
    }

    /// Create the private download directories under `root` instead of the
    /// system temp directory (tests, or operators with a dedicated volume).
    pub fn with_temp_root(mut self, root: PathBuf) -> Self {
        self.temp_root = Some(root);
        self
    }

    /// Download `ipfs_cid` from the configured nodes, verify it against
    /// `expected_hash` (lowercase or uppercase hex SHA-256) and return a
    /// handle to the verified file named `file_name` (sanitised).
    pub async fn download_verified(
        &self,
        ipfs_cid: &str,
        expected_hash: &str,
        file_name: &str,
    ) -> Result<VerifiedDownload> {
        if self.nodes.is_empty() {
            bail!("No P2P nodes available for download");
        }
        common::validate_ipfs_cid(ipfs_cid)
            .map_err(|reason| anyhow::anyhow!("refusing to download invalid IPFS CID: {reason}"))?;
        let expected = expected_hash.trim().to_ascii_lowercase();
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("expected content hash is not a 64-character SHA-256 hex digest");
        }

        let mut builder = tempfile::Builder::new();
        builder.prefix("creg-download-");
        let dir = match &self.temp_root {
            Some(root) => builder.tempdir_in(root),
            None => builder.tempdir(),
        }
        .context("create private download directory")?;

        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .context("build download client")?;

        tracing::info!(
            "downloading {} from {} node(s) (cap {} bytes)",
            ipfs_cid,
            self.nodes.len(),
            self.max_bytes
        );

        let parts: Vec<PathBuf> = (0..self.nodes.len())
            .map(|index| dir.path().join(format!("part-{index}")))
            .collect();
        let mut attempts = FuturesUnordered::new();
        for (index, node) in self.nodes.iter().enumerate() {
            let url = format!("{}/v1/ipfs/{}", node.trim_end_matches('/'), ipfs_cid);
            let client = client.clone();
            let part = parts[index].clone();
            let expected = expected.clone();
            let max_bytes = self.max_bytes;
            attempts.push(async move {
                let result = fetch_to_file(&client, &url, &part, max_bytes, &expected).await;
                (index, url, result)
            });
        }

        let mut winner = None;
        let mut failures = Vec::new();
        while let Some((index, url, result)) = attempts.next().await {
            match result {
                Ok(len) => {
                    winner = Some((index, len));
                    break;
                }
                Err(err) => failures.push(format!("{url}: {err:#}")),
            }
        }
        // Cancel losing attempts; each removes its own partial file on drop.
        drop(attempts);

        let Some((index, len)) = winner else {
            bail!(
                "failed to download or verify package from any node: {}",
                failures.join("; ")
            );
        };
        for (other, part) in parts.iter().enumerate() {
            if other != index {
                let _ = std::fs::remove_file(part);
            }
        }
        let path = dir.path().join(safe_file_name(file_name));
        std::fs::rename(&parts[index], &path)
            .with_context(|| format!("finalise verified download {}", path.display()))?;
        tracing::info!("verified download ready at {}", path.display());
        Ok(VerifiedDownload { dir, path, len })
    }
}

/// Removes a partial download file unless disarmed. Declared before the file
/// handle in `fetch_to_file`, so the handle is closed first on every exit
/// path, including cancellation.
struct PartialFile<'a> {
    path: &'a Path,
    armed: bool,
}

impl Drop for PartialFile<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(self.path);
        }
    }
}

async fn fetch_to_file(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    max_bytes: u64,
    expected_hash: &str,
) -> Result<u64> {
    let mut response = client
        .get(url)
        .send()
        .await
        .context("request failed")?
        .error_for_status()
        .context("node returned an error status")?;
    if let Some(declared) = response.content_length() {
        if declared > max_bytes {
            bail!("declared body of {declared} bytes exceeds the {max_bytes}-byte download cap");
        }
    }

    let mut guard = PartialFile { path, armed: true };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    while let Some(chunk) = response.chunk().await.context("read body")? {
        total = total.saturating_add(chunk.len() as u64);
        if total > max_bytes {
            bail!("body exceeds the {max_bytes}-byte download cap");
        }
        hasher.update(&chunk);
        file.write_all(&chunk).context("write body")?;
    }
    file.sync_all().context("sync body")?;
    drop(file);

    let actual = hex::encode(hasher.finalize());
    if actual != expected_hash {
        bail!("content hash mismatch (got {actual}, expected {expected_hash})");
    }
    guard.armed = false;
    Ok(total)
}

/// File name for the verified artifact: path separators and anything outside
/// `[A-Za-z0-9._@+-]` become `_`, leading dots are stripped, and the name
/// always ends in `.tgz` (npm installs a local tarball only by that suffix).
fn safe_file_name(raw: &str) -> String {
    let mut name: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '@' | '+' | '-') {
                ch
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    while name.starts_with('.') {
        name.remove(0);
    }
    if name.is_empty() {
        name.push_str("package");
    }
    if !name.ends_with(".tgz") {
        name.push_str(".tgz");
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const CID: &str = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";

    /// Serves `body` once per connection; `None` streams forever.
    async fn serve(body: Option<Vec<u8>>, declared_len: Option<u64>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut request = [0u8; 2048];
                    let _ = socket.read(&mut request).await;
                    match body {
                        Some(body) => {
                            let len = declared_len.unwrap_or(body.len() as u64);
                            let head = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
                            );
                            let _ = socket.write_all(head.as_bytes()).await;
                            let _ = socket.write_all(&body).await;
                        }
                        None => {
                            let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
                            if socket.write_all(head.as_bytes()).await.is_err() {
                                return;
                            }
                            let chunk = vec![b'a'; 64 * 1024];
                            let frame = format!("{:x}\r\n", chunk.len());
                            loop {
                                if socket.write_all(frame.as_bytes()).await.is_err()
                                    || socket.write_all(&chunk).await.is_err()
                                    || socket.write_all(b"\r\n").await.is_err()
                                {
                                    return;
                                }
                            }
                        }
                    }
                });
            }
        });
        format!("http://{addr}")
    }

    fn sha(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    fn entries(dir: &Path) -> usize {
        std::fs::read_dir(dir).unwrap().count()
    }

    #[tokio::test]
    async fn eco_adv_13_endless_body_aborts_at_cap_and_leaves_no_temp_file() {
        let root = tempfile::tempdir().unwrap();
        let node = serve(None, None).await;
        let downloader =
            P2PDownloader::with_limits(vec![node], 1024 * 1024, Duration::from_secs(30))
                .with_temp_root(root.path().to_path_buf());
        let err = downloader
            .download_verified(CID, &sha(b"irrelevant"), "left-pad")
            .await
            .expect_err("endless body must be refused");
        assert!(format!("{err:#}").contains("download cap"), "{err:#}");
        assert_eq!(entries(root.path()), 0, "temp directory left behind");
    }

    #[tokio::test]
    async fn declared_oversize_body_is_refused_before_reading() {
        let root = tempfile::tempdir().unwrap();
        let node = serve(Some(vec![b'x'; 16]), Some(10 * 1024 * 1024)).await;
        let downloader = P2PDownloader::with_limits(vec![node], 1024, Duration::from_secs(10))
            .with_temp_root(root.path().to_path_buf());
        let err = downloader
            .download_verified(CID, &sha(b"x"), "pkg")
            .await
            .expect_err("oversize declaration must be refused");
        assert!(format!("{err:#}").contains("download cap"), "{err:#}");
        assert_eq!(entries(root.path()), 0);
    }

    #[tokio::test]
    async fn hash_mismatch_is_refused_and_cleaned_up() {
        let root = tempfile::tempdir().unwrap();
        let node = serve(Some(b"tampered".to_vec()), None).await;
        let downloader = P2PDownloader::with_limits(vec![node], 1024, Duration::from_secs(10))
            .with_temp_root(root.path().to_path_buf());
        let err = downloader
            .download_verified(CID, &sha(b"original"), "pkg")
            .await
            .expect_err("mismatch must be refused");
        assert!(format!("{err:#}").contains("hash mismatch"), "{err:#}");
        assert_eq!(entries(root.path()), 0);
    }

    #[tokio::test]
    async fn verified_body_is_returned_in_private_dir_and_removed_on_drop() {
        let root = tempfile::tempdir().unwrap();
        let body = b"package tarball bytes".to_vec();
        let good = serve(Some(body.clone()), None).await;
        let bad = serve(Some(b"evil".to_vec()), None).await;
        let downloader = P2PDownloader::with_limits(vec![bad, good], 1024, Duration::from_secs(10))
            .with_temp_root(root.path().to_path_buf());
        let download = downloader
            .download_verified(CID, &sha(&body).to_ascii_uppercase(), "@scope/pkg")
            .await
            .expect("verified download");
        assert_eq!(std::fs::read(download.path()).unwrap(), body);
        assert_eq!(download.len(), body.len() as u64);
        assert!(download.path().starts_with(download.dir()));
        assert_eq!(
            download.path().file_name().unwrap().to_str().unwrap(),
            "@scope_pkg.tgz"
        );
        assert_eq!(entries(download.dir()), 1, "loser partial file left behind");
        drop(download);
        assert_eq!(entries(root.path()), 0);
    }

    #[tokio::test]
    async fn invalid_cid_and_hash_are_refused_without_network() {
        let downloader = P2PDownloader::with_limits(
            vec!["http://127.0.0.1:9".into()],
            1024,
            Duration::from_secs(1),
        );
        for cid in ["../../etc/passwd", "bafy?x=1", "Qm/../x", ""] {
            assert!(downloader
                .download_verified(cid, &sha(b"x"), "pkg")
                .await
                .is_err());
        }
        assert!(downloader
            .download_verified(CID, "not-a-hash", "pkg")
            .await
            .is_err());
    }

    #[test]
    fn file_names_are_sanitised() {
        assert_eq!(safe_file_name("left-pad"), "left-pad.tgz");
        assert_eq!(safe_file_name("../../x"), "_.._x.tgz");
        assert_eq!(safe_file_name("..."), "package.tgz");
        assert_eq!(safe_file_name("a b\\c"), "a_b_c.tgz");
        assert_eq!(safe_file_name("pkg.tgz"), "pkg.tgz");
    }
}
