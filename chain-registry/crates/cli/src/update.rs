// crates/cli/src/update.rs
// Verified self-update for the `creg` CLI.
//
// Updates are explicit (the caller must pass --yes), use GitHub release
// metadata, require a SHA-256 digest from the release, and stage the binary
// before replacing the currently running executable. No private keys or node
// data are read by this module.

use anyhow::{bail, Context, Result};
use colored::Colorize;
use futures::StreamExt;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Cursor, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const DEFAULT_GITHUB_REPO: &str = "samuel-1-avson/chain-registry-blockchain-CREG-";
const GITHUB_API: &str = "https://api.github.com";

#[derive(Debug, Clone)]
struct ReleaseCandidate {
    tag: String,
    version: Version,
    asset_name: String,
    download_url: String,
    expected_sha256: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    /// GitHub may expose a digest directly; SHA256SUMS remains the fallback
    /// for older releases and is required when this field is absent.
    digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NodeReleaseRecord {
    version: String,
    download_url: String,
    sha256: String,
}

pub async fn run(
    node_url: Option<&str>,
    check_only: bool,
    confirm: bool,
    requested_version: Option<&str>,
) -> Result<()> {
    let current = compiled_version();
    let requested_tag = requested_version.map(|value| value.trim().to_string());
    let requested = requested_tag.as_deref().map(normalize_tag).transpose()?;
    let repo =
        std::env::var("CREG_GITHUB_REPO").unwrap_or_else(|_| DEFAULT_GITHUB_REPO.to_string());

    println!(
        "{} Checking for creg updates (current: v{})...",
        "→".cyan(),
        current
    );

    // Preserve support for nodes that publish a release record, but use the
    // GitHub release API when the node is offline or does not expose one.
    let configured_node_url = node_url
        .map(str::to_owned)
        .or_else(|| std::env::var("CREG_NODE_URL").ok());
    let candidate = if requested.is_none() {
        if let Some(url) = configured_node_url.as_deref() {
            match fetch_node_release(url, &current).await {
                Ok(candidate) if candidate.version > current => Some(candidate),
                Ok(candidate) => {
                    println!(
                        "{} Node advertises {} or older; checking GitHub for newer releases...",
                        "ℹ".blue(),
                        candidate.tag
                    );
                    None
                }
                Err(error) => {
                    println!(
                        "{} Node release metadata unavailable ({}); checking GitHub...",
                        "ℹ".blue(),
                        redact_error(&error)
                    );
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    let candidate = match candidate {
        Some(candidate) => candidate,
        None => fetch_github_release(&repo, requested_tag.as_deref()).await?,
    };

    if requested.is_none() && candidate.version <= current {
        println!(
            "{} Already up to date (v{}; release {})",
            "✓".green(),
            current,
            candidate.tag
        );
        return Ok(());
    }

    println!(
        "{} New release available: {}",
        "⬆".yellow().bold(),
        candidate.tag
    );
    println!("  Current: v{}", current);
    println!("  Target:  {}", candidate.tag);
    println!("  Asset:   {}", candidate.asset_name);

    if check_only {
        println!("\n  Check complete; no files were changed.");
        return Ok(());
    }
    if !confirm {
        println!(
            "\n  {} Update is staged only with explicit confirmation.",
            "ℹ".blue()
        );
        println!("  Re-run: creg update --yes");
        return Ok(());
    }

    install_update(&candidate).await
}

fn compiled_version() -> Version {
    let embedded = env!("CREG_EFFECTIVE_VERSION");
    Version::parse(embedded.trim().trim_start_matches('v'))
        .unwrap_or_else(|_| Version::new(0, 0, 0))
}

fn normalize_tag(value: &str) -> Result<Version> {
    let normalized = value.trim().trim_start_matches('v');
    Version::parse(normalized)
        .with_context(|| format!("release version is not valid semver: {value}"))
}

fn redact_error(error: &anyhow::Error) -> String {
    let text = error.to_string();
    text.split_once("?")
        .map(|(prefix, _)| format!("{prefix}?<redacted>"))
        .unwrap_or(text)
}

async fn fetch_node_release(node_url: &str, current: &Version) -> Result<ReleaseCandidate> {
    let url = format!("{}/v1/releases/cli/latest", node_url.trim_end_matches('/'));
    let client = http_client()?;
    let record = client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .context("node release request failed")?
        .error_for_status()
        .context("node did not publish a usable release record")?
        .json::<NodeReleaseRecord>()
        .await
        .context("node release record was not valid JSON")?;
    let version = normalize_tag(&record.version)?;
    if version <= *current {
        return Ok(ReleaseCandidate {
            tag: format!("v{}", version),
            version,
            asset_name: "node-published-artifact".into(),
            download_url: record.download_url,
            expected_sha256: canonical_sha256(&record.sha256)?,
        });
    }
    Ok(ReleaseCandidate {
        tag: record.version,
        version,
        asset_name: "node-published-artifact".into(),
        download_url: record.download_url,
        expected_sha256: canonical_sha256(&record.sha256)?,
    })
}

async fn fetch_github_release(repo: &str, requested: Option<&str>) -> Result<ReleaseCandidate> {
    let client = http_client()?;
    let releases = if let Some(tag) = requested {
        let url = format!("{GITHUB_API}/repos/{repo}/releases/tags/{tag}");
        vec![client
            .get(url)
            .send()
            .await
            .context("GitHub release request failed")?
            .error_for_status()
            .context("requested GitHub release was not found")?
            .json::<GithubRelease>()
            .await
            .context("GitHub release metadata was not valid JSON")?]
    } else {
        let url = format!("{GITHUB_API}/repos/{repo}/releases?per_page=30");
        client
            .get(url)
            .send()
            .await
            .context("GitHub release list request failed")?
            .error_for_status()
            .context("GitHub release list was unavailable")?
            .json::<Vec<GithubRelease>>()
            .await
            .context("GitHub release list was not valid JSON")?
    };

    let mut parsed = releases
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = normalize_tag(&release.tag_name).ok()?;
            Some((version, release))
        })
        .collect::<Vec<_>>();
    parsed.sort_by(|left, right| left.0.cmp(&right.0));
    let (_, release) = parsed
        .pop()
        .ok_or_else(|| anyhow::anyhow!("no semver GitHub releases were found"))?;
    let version = normalize_tag(&release.tag_name)?;
    let asset = select_platform_asset(&release.assets, &release.tag_name)?;
    let expected_sha256 = match asset.digest.as_deref() {
        Some(digest) => canonical_sha256(digest.strip_prefix("sha256:").unwrap_or(digest))?,
        None => {
            let manifest = release
                .assets
                .iter()
                .find(|asset| asset.name.eq_ignore_ascii_case("SHA256SUMS"))
                .ok_or_else(|| anyhow::anyhow!("release has no SHA256SUMS asset"))?;
            let manifest_bytes = client
                .get(&manifest.browser_download_url)
                .send()
                .await
                .context("SHA256SUMS download failed")?
                .error_for_status()
                .context("SHA256SUMS asset was not available")?
                .bytes()
                .await
                .context("SHA256SUMS asset could not be read")?;
            checksum_for_asset(&manifest_bytes, &asset.name)?
        }
    };

    Ok(ReleaseCandidate {
        tag: release.tag_name,
        version,
        asset_name: asset.name,
        download_url: asset.browser_download_url,
        expected_sha256,
    })
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!(
            "creg/{} self-updater",
            env!("CREG_EFFECTIVE_VERSION")
        ))
        .build()
        .context("could not create update HTTP client")
}

fn select_platform_asset(assets: &[GithubAsset], tag: &str) -> Result<GithubAsset> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let arch_names: &[&str] = match arch {
        "x86_64" => &["amd64", "x86_64"],
        "aarch64" => &["arm64", "aarch64"],
        other => &[other],
    };
    let os_names: &[&str] = match os {
        "windows" => &["windows"],
        "macos" => &["macos", "darwin"],
        "linux" => &["linux"],
        other => &[other],
    };
    let tag = tag.trim_start_matches('v');
    let mut candidates = Vec::new();
    for os_name in os_names {
        for arch_name in arch_names {
            candidates.push(format!("chain-registry-v{tag}-{os_name}-{arch_name}.zip"));
            candidates.push(format!(
                "chain-registry-v{tag}-{os_name}-{arch_name}.tar.gz"
            ));
            candidates.push(format!("chain-registry-{tag}-{os_name}-{arch_name}.zip"));
            candidates.push(format!("chain-registry-{tag}-{os_name}-{arch_name}.tar.gz"));
        }
    }
    assets
        .iter()
        .find(|asset| candidates.iter().any(|name| name == &asset.name))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("release {} has no {}-{} creg archive", tag, os, arch))
}

fn canonical_sha256(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("release checksum is not a canonical SHA-256 digest")
    }
    Ok(normalized)
}

fn checksum_for_asset(manifest: &[u8], asset_name: &str) -> Result<String> {
    let text = std::str::from_utf8(manifest).context("SHA256SUMS is not UTF-8")?;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let Some(digest) = fields.next() else {
            continue;
        };
        let Some(name) = fields.next() else { continue };
        if name.trim_start_matches('*') == asset_name {
            return canonical_sha256(digest);
        }
    }
    bail!("SHA256SUMS does not contain an entry for {asset_name}")
}

async fn install_update(candidate: &ReleaseCandidate) -> Result<()> {
    println!("{} Downloading {}...", "→".cyan(), candidate.asset_name);
    let client = http_client()?;
    let response = client
        .get(&candidate.download_url)
        .timeout(Duration::from_secs(180))
        .send()
        .await
        .context("failed to download update artifact")?
        .error_for_status()
        .context("update artifact was not available")?;
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.context("error reading update artifact")?);
    }
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != candidate.expected_sha256 {
        bail!(
            "update checksum mismatch (expected {}, got {})",
            candidate.expected_sha256,
            actual
        );
    }
    println!("  {} SHA-256 verified: {}", "✓".green(), &actual[..16]);

    let current_exe = std::env::current_exe().context("cannot determine current executable")?;
    let staging = unique_staging_dir(&current_exe)?;
    fs::create_dir_all(&staging)?;
    let result = (|| -> Result<()> {
        let staged_binary = extract_binary(&candidate.asset_name, &bytes, &staging)?;
        install_staged_binary(&current_exe, &staged_binary)?;
        #[cfg(not(windows))]
        fs::remove_dir_all(&staging).context("could not remove update staging directory")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn unique_staging_dir(current_exe: &Path) -> Result<PathBuf> {
    let parent = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("current executable has no parent directory"))?;
    let name = format!(
        ".creg-update-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    );
    Ok(parent.join(name))
}

fn extract_binary(asset_name: &str, bytes: &[u8], staging: &Path) -> Result<PathBuf> {
    let target_name = if cfg!(windows) { "creg.exe" } else { "creg" };
    let destination = staging.join(target_name);
    if asset_name.ends_with(".zip") {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
            .context("update archive is not a valid ZIP")?;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .context("could not read ZIP entry")?;
            let Some(name) = Path::new(entry.name()).file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name == target_name {
                let mut output = File::create(&destination)?;
                io::copy(&mut entry, &mut output)?;
                output.flush()?;
                set_executable(&destination)?;
                return Ok(destination);
            }
        }
        bail!("update ZIP does not contain {target_name}")
    } else if asset_name.ends_with(".tar.gz") || asset_name.ends_with(".tgz") {
        let decoder = flate2::read::GzDecoder::new(Cursor::new(bytes));
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries().context("could not read tar archive")? {
            let mut entry = entry.context("could not read tar entry")?;
            let Some(name) = entry
                .path()
                .ok()
                .and_then(|path| path.file_name().map(|name| name.to_owned()))
                .and_then(|name| name.to_str().map(str::to_owned))
            else {
                continue;
            };
            if name == target_name {
                let mut output = File::create(&destination)?;
                io::copy(&mut entry, &mut output)?;
                output.flush()?;
                set_executable(&destination)?;
                return Ok(destination);
            }
        }
        bail!("update tarball does not contain {target_name}")
    } else {
        let mut output = File::create(&destination)?;
        output.write_all(bytes)?;
        output.flush()?;
        set_executable(&destination)?;
        Ok(destination)
    }
}

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn install_staged_binary(current: &Path, staged: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        install_windows(current, staged)
    }
    #[cfg(not(windows))]
    {
        let backup = current.with_extension("previous");
        let _ = fs::remove_file(&backup);
        fs::rename(current, &backup).context("could not stage the current creg binary")?;
        if let Err(error) = fs::rename(staged, current) {
            let _ = fs::rename(&backup, current);
            return Err(error).context("could not install the updated creg binary");
        }
        let _ = fs::remove_file(backup);
        println!(
            "{} creg updated; restart the command to use {}",
            "✓".green().bold(),
            current.display()
        );
        Ok(())
    }
}

#[cfg(windows)]
fn install_windows(current: &Path, staged: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let script = staged.with_extension("ps1");
    fs::write(
        &script,
        r#"param([int]$ParentPid,[string]$Current,[string]$Staged,[string]$Script)
while (Get-Process -Id $ParentPid -ErrorAction SilentlyContinue) { Start-Sleep -Milliseconds 200 }
$Backup = "$Current.previous"
try {
  if (Test-Path -LiteralPath $Backup) { Remove-Item -LiteralPath $Backup -Force }
  Move-Item -LiteralPath $Current -Destination $Backup -Force
  Move-Item -LiteralPath $Staged -Destination $Current -Force
  Remove-Item -LiteralPath $Backup -Force -ErrorAction SilentlyContinue
} catch {
  if ((Test-Path -LiteralPath $Backup) -and !(Test-Path -LiteralPath $Current)) {
    Move-Item -LiteralPath $Backup -Destination $Current -Force
  }
  exit 1
}
Remove-Item -LiteralPath $Script -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath (Split-Path -Parent $Staged) -Recurse -Force -ErrorAction SilentlyContinue
"#,
    )?;
    let parent_pid = std::process::id().to_string();
    let current_arg = current.to_string_lossy().into_owned();
    let staged_arg = staged.to_string_lossy().into_owned();
    let script_arg = script.to_string_lossy().into_owned();
    std::process::Command::new("powershell.exe")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script_arg)
        .arg(&parent_pid)
        .arg(&current_arg)
        .arg(&staged_arg)
        .arg(&script_arg)
        .spawn()
        .context("could not start the Windows updater helper")?;
    println!(
        "{} Update verified and staged; the helper will replace the binary after this process exits.",
        "✓".green().bold()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_manifest_accepts_unix_and_windows_forms() {
        let digest = "ab".repeat(32);
        let unix = format!("{digest}  chain-registry-v0.4.2-linux-amd64.tar.gz\n");
        let windows = format!("{digest} *chain-registry-v0.4.2-windows-amd64.zip\n");
        assert_eq!(
            checksum_for_asset(unix.as_bytes(), "chain-registry-v0.4.2-linux-amd64.tar.gz")
                .unwrap(),
            digest
        );
        assert_eq!(
            checksum_for_asset(
                windows.as_bytes(),
                "chain-registry-v0.4.2-windows-amd64.zip"
            )
            .unwrap(),
            digest
        );
    }

    #[test]
    fn invalid_checksums_are_rejected() {
        assert!(canonical_sha256("not-a-digest").is_err());
        assert!(checksum_for_asset(b"deadbeef  other.zip\n", "creg.zip").is_err());
    }

    #[test]
    fn tags_normalize_to_semver() {
        assert_eq!(
            normalize_tag("v0.4.2-testnet").unwrap().to_string(),
            "0.4.2-testnet"
        );
    }
}
