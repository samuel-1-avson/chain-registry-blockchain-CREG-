// crates/cli/src/install.rs
// Resolves trust verdict, then either proceeds or blocks the install.
//
// TODO(C-19): Use retry::with_retry for network calls for resilience
// TODO(C-21): Call lockfile::write_receipt after successful install to record trust verdict
// TODO(C-22): Call policy::evaluate() to check org-level policy before install
// TODO(C-23): Load config_file settings and pass through to control behavior

use crate::output;
use anyhow::{bail, Result};
use colored::Colorize;
use common::{PackageId, VerdictStatus};
use dialoguer::Confirm;

pub async fn run(
    raw_package: &str,
    ecosystem_hint: Option<&str>,
    allow_unverified: bool,
    node_url: Option<&str>,
) -> Result<()> {
    // ── 1. Parse "name@version" or plain "name" ───────────────────────────────
    let (name, version) = parse_package_arg(raw_package);
    let ecosystem = ecosystem_hint
        .map(String::from)
        .unwrap_or_else(detect_ecosystem);

    let pkg_id = PackageId::new(&ecosystem, &name, version.as_deref().unwrap_or("latest"));

    // ── 2. Query the chain (cache-first, then live node) ─────────────────────
    println!("{} Resolving {} ...", "→".cyan(), pkg_id.canonical().bold());
    let verdict = resolver::resolve_id(&pkg_id, node_url).await?;

    // ── 3. Trust decision ─────────────────────────────────────────────────────
    match &verdict.status {
        VerdictStatus::Verified {
            block_hash,
            findings,
            ipfs_cid: _,
            content_hash: _,
        } => {
            output::print_verdict(&verdict);
            if !block_hash.is_empty() {
                println!(
                    "  {} chain record: block {}",
                    "✓".green(),
                    &block_hash[..std::cmp::min(12, block_hash.len())]
                );
            }

            // Defense-in-depth: check if findings are severe despite verification
            let has_severe = findings.iter().any(|f| {
                matches!(
                    f.severity,
                    common::FindingSeverity::Critical | common::FindingSeverity::High
                )
            });
            if has_severe && !allow_unverified {
                let proceed = Confirm::new()
                    .with_prompt(format!(
                        "{} Package '{}' has high-severity security findings. Install anyway?",
                        "⚠".yellow().bold(),
                        pkg_id.canonical()
                    ))
                    .default(false)
                    .interact()?;
                if !proceed {
                    bail!("Install cancelled due to security findings.");
                }
            }
        }

        VerdictStatus::Unverified => {
            output::print_verdict(&verdict);
            if !allow_unverified {
                let proceed = Confirm::new()
                    .with_prompt(format!(
                        "{} Package '{}' is not yet chain-verified. Install anyway?",
                        "⚠".yellow(),
                        pkg_id.canonical()
                    ))
                    .default(false)
                    .interact()?;

                if !proceed {
                    bail!("Install cancelled — package not chain-verified.");
                }
            } else {
                println!(
                    "{} installing unverified package (--unverified flag set)",
                    "⚠".yellow()
                );
            }
        }

        VerdictStatus::Revoked { reason, findings } => {
            output::print_verdict(&verdict);
            bail!(
                "{} Package '{}' is REVOKED and cannot be installed.\n  Reason: {}\n  Findings: {} record(s)",
                "✗".red().bold(),
                pkg_id.canonical(),
                reason,
                findings.len()
            );
        }

        VerdictStatus::Unknown => {
            output::print_verdict(&verdict);
            if !allow_unverified {
                bail!(
                    "{} Package '{}' is unknown to the chain registry.\n  Use --unverified to install from the original registry.",
                    "✗".red(),
                    pkg_id.canonical()
                );
            }
            println!(
                "{} unknown to chain registry — falling through to original registry",
                "⚠".yellow()
            );
        }
    }

    // ── 4. Swarm Download (Decentralised Distribution) ───────────────────────
    let mut local_tarball: Option<std::path::PathBuf> = None;
    match &verdict.status {
        VerdictStatus::Verified {
            content_hash,
            ipfs_cid,
            ..
        } => {
            if ipfs_cid.is_empty() {
                bail!(
                    "{} Verified package '{}' has no IPFS CID. Cannot install without verified content.",
                    "X".red(),
                    pkg_id.canonical()
                );
            }
            println!("{} Fetching from P2P swarm...", "->".cyan());
            let nodes = vec![node_url.unwrap_or("http://localhost:8080").to_string()];
            let downloader = resolver::downloader::P2PDownloader::new(nodes);
            let temp_file =
                std::env::temp_dir().join(format!("{}.tgz", pkg_id.name.replace('/', "_")));

            match downloader
                .download(ipfs_cid, content_hash, &temp_file)
                .await
            {
                Ok(_) => {
                    local_tarball = Some(temp_file);
                }
                Err(e) => {
                    bail!(
                        "{} P2P download failed for verified package '{}': {}. \
                         Refusing to fall back to unverified original registry.",
                        "X".red(),
                        pkg_id.canonical(),
                        e
                    );
                }
            }
        }
        VerdictStatus::Unverified | VerdictStatus::Unknown => {
            // For unverified/unknown packages, falling back to the original registry is acceptable.
        }
        VerdictStatus::Revoked { .. } => {
            // Already bailed above; unreachable.
        }
    }

    // ── 5. Delegate to the real package manager ───────────────────────────────
    let install_target = local_tarball
        .as_ref()
        .and_then(|p| p.to_str())
        .unwrap_or(raw_package);

    delegate_to_real_pm(&ecosystem, install_target)?;

    Ok(())
}

/// Calls the real package manager (the one on PATH *after* our shim dir).
fn delegate_to_real_pm(ecosystem: &str, raw_package: &str) -> Result<()> {
    let pm_name = match ecosystem {
        "npm" => "npm",
        "pypi" => "pip",
        "cargo" => "cargo",
        "rubygems" => "gem",
        "maven" => "mvn",
        _ => bail!("Unknown ecosystem: {}", ecosystem),
    };

    // Find the real package manager binary, skipping our own shim.
    // We compare canonical paths to filter out our shim directory.
    let shim_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".local")
        .join("bin");
    let real_bin = which::which_all(pm_name)?
        .find(|p| {
            // Skip binaries in our shim directory
            p.parent().map_or(true, |parent| {
                parent.canonicalize().ok() != shim_dir.canonicalize().ok()
            })
        })
        .ok_or_else(|| anyhow::anyhow!("Real '{}' not found in PATH (only our shim exists)", pm_name))?;

    let mut args: Vec<&str> = match ecosystem {
        "npm" => vec!["install", raw_package],
        "pypi" => vec!["install", raw_package],
        "cargo" => vec!["add", raw_package],
        "rubygems" => vec!["install", raw_package],
        "maven" => vec!["dependency:resolve"],
        _ => unreachable!(),
    };

    // Pass through any extra args from CREG_PM_ARGS env var
    let extra_args = std::env::var("CREG_PM_ARGS").unwrap_or_default();
    let extra: Vec<&str> = extra_args.split_whitespace().collect();
    args.extend(&extra);

    let status = std::process::Command::new(&real_bin).args(&args).status()?;

    if !status.success() {
        bail!("Package manager exited with status {}", status);
    }
    Ok(())
}

/// Splits "express@4.18.0" → ("express", Some("4.18.0"))
fn parse_package_arg(raw: &str) -> (String, Option<String>) {
    // Handle scoped npm packages: @scope/pkg@version
    if raw.starts_with('@') {
        let rest = &raw[1..];
        if let Some(idx) = rest.rfind('@') {
            let name = format!("@{}", &rest[..idx]);
            let version = rest[idx + 1..].to_string();
            return (name, Some(version));
        }
        return (raw.to_string(), None);
    }
    match raw.rfind('@') {
        Some(idx) => (raw[..idx].to_string(), Some(raw[idx + 1..].to_string())),
        None => (raw.to_string(), None),
    }
}

/// Detects the current project's ecosystem from files in the working directory.
fn detect_ecosystem() -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    if cwd.join("package.json").exists() {
        return "npm".into();
    }
    if cwd.join("Cargo.toml").exists() {
        return "cargo".into();
    }
    if cwd.join("requirements.txt").exists() || cwd.join("pyproject.toml").exists() {
        return "pypi".into();
    }
    if cwd.join("Gemfile").exists() {
        return "rubygems".into();
    }
    if cwd.join("pom.xml").exists() {
        return "maven".into();
    }
    "unknown".into()
}
