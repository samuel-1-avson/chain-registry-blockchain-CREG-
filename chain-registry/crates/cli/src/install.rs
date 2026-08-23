// crates/cli/src/install.rs
// Resolves trust verdict, then either proceeds or blocks the install.

use crate::output;
use crate::retry;
use anyhow::{bail, Result};
use colored::Colorize;
use common::{PackageId, VerdictStatus};
use std::time::Duration;

pub async fn run(
    raw_package: &str,
    ecosystem_hint: Option<&str>,
    allow_unverified: bool,
    node_url: Option<&str>,
) -> Result<()> {
    // ── 1. Parse "name@version" or plain "name" ───────────────────────────────
    let (name, version) = parse_package_arg(raw_package);

    // ── C-23: Load config file settings ─────────────────────────────────────
    let cfg = crate::config_file::Config::load().unwrap_or_default();
    let ecosystem = ecosystem_hint
        .map(String::from)
        .or_else(|| cfg.defaults.ecosystem.clone())
        .unwrap_or_else(detect_ecosystem);

    let pkg_id = PackageId::new(&ecosystem, &name, version.as_deref().unwrap_or("latest"));

    // ── 2. Query the chain (cache-first, then live node) — with retry ────────
    // Effective node URL: CLI arg > env var > config file default
    let effective_node_url = node_url
        .map(String::from)
        .or_else(|| std::env::var("CREG_NODE_URL").ok())
        .or(Some(cfg.node.url.clone()));
    let effective_node_ref = effective_node_url.as_deref();

    println!("{} Resolving {} ...", "→".cyan(), pkg_id.canonical().bold());
    let verdict = match retry::with_retry("resolve package", 3, Duration::from_millis(500), || {
        resolver::install_gate::resolve_id_for_install(&pkg_id, effective_node_ref)
    })
    .await
    {
        Ok(verdict) => verdict,
        Err(err) if allow_unverified => {
            println!(
                "{} chain registry unreachable ({err}); continuing because --unverified / {ALLOW} is set",
                "⚠".yellow(),
                ALLOW = resolver::install_gate::ALLOW_UNVERIFIED_ENV
            );
            common::TrustVerdict {
                package: pkg_id.clone(),
                status: VerdictStatus::Unknown,
                resolved_at: chrono::Utc::now(),
                source: common::VerdictSource::Chain {
                    node_url: effective_node_url
                        .clone()
                        .unwrap_or_else(|| "unconfigured".into()),
                },
                deterministic_risk: None,
                evidence_phase: None,
                block_phase: None,
                chain_status: None,
                publisher: None,
            }
        }
        Err(err) => {
            bail!(
                "{} chain registry unreachable ({err}). Refusing installation; use --unverified or {ALLOW}=true only as an explicit break-glass exception.",
                "✗".red().bold(),
                ALLOW = resolver::install_gate::ALLOW_UNVERIFIED_ENV
            );
        }
    };

    // ── C-22: Org-level policy check ─────────────────────────────────────────
    // Load ~/.creg/policy.toml (if it exists) and evaluate the package against
    // all configured rules before applying individual trust decisions.
    let policy_path = dirs::home_dir()
        .map(|h| h.join(".creg").join("policy.toml"))
        .filter(|p| p.exists());

    if let Some(ref path) = policy_path {
        match crate::policy::PolicyFile::load(path) {
            Ok(policy) => {
                // enforce_ecosystems: empty list means all ecosystems are enforced.
                let ecosystem_enforced = policy.enforce_ecosystems.is_empty()
                    || policy.enforce_ecosystems.iter().any(|e| e == &ecosystem);

                if ecosystem_enforced {
                    let status_str = verdict.status.label().to_lowercase();
                    let publisher = verdict.publisher.as_deref().unwrap_or("");
                    let mut violations =
                        policy.evaluate(&pkg_id.canonical(), &status_str, publisher);

                    // block_on_findings: check verified packages for matching severities.
                    if let VerdictStatus::Verified { findings, .. } = &verdict.status {
                        for finding in findings {
                            let sev = format!("{:?}", finding.severity);
                            if policy
                                .block_on_findings
                                .iter()
                                .any(|b| b.eq_ignore_ascii_case(&sev))
                            {
                                violations.push(format!(
                                    "Package has a {} finding: {}",
                                    sev, finding.description
                                ));
                            }
                        }
                    }

                    if !violations.is_empty() {
                        for v in &violations {
                            eprintln!("{} Policy violation: {}", "✗".red().bold(), v);
                        }
                        bail!(
                            "Install blocked by org policy ({} violation(s)). \
                             Run `creg policy show` to review active rules.",
                            violations.len()
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Could not load policy file {}: {} — skipping policy check",
                    path.display(),
                    e
                );
            }
        }
    }

    // ── 3. Trust decision (shared with PATH shims, SHIM-301) ─────────────────
    output::print_verdict(&verdict);
    match resolver::install_gate::admit_install_live(&verdict, allow_unverified, effective_node_ref)
        .await?
    {
        resolver::install_gate::InstallDecision::Allow => {
            if let VerdictStatus::Verified { block_hash, .. } = &verdict.status {
                if !block_hash.is_empty() {
                    println!(
                        "  {} chain record: block {}",
                        "✓".green(),
                        &block_hash[..std::cmp::min(12, block_hash.len())]
                    );
                }
            }
        }
        resolver::install_gate::InstallDecision::AllowUnverified => {
            println!(
                "{} installing unverified package (--unverified / {} break-glass)",
                "⚠".yellow(),
                resolver::install_gate::ALLOW_UNVERIFIED_ENV
            );
        }
        resolver::install_gate::InstallDecision::Block { message, .. } => {
            bail!("{} {message}", "✗".red().bold());
        }
    }

    // ── 4. Swarm Download (Decentralised Distribution) ───────────────────────
    // Write audit receipt to the lockfile (C-21).
    let cwd = std::env::current_dir().unwrap_or_default();
    if let Err(e) = crate::lockfile::write_receipt(&cwd, &verdict) {
        eprintln!("{} Failed to write audit receipt: {}", "⚠".yellow(), e);
    }

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
            let fallback_node = effective_node_url
                .as_deref()
                .unwrap_or("http://localhost:8080")
                .to_string();
            let temp_file =
                std::env::temp_dir().join(format!("{}.tgz", pkg_id.name.replace('/', "_")));

            let ipfs_cid_owned = ipfs_cid.clone();
            let content_hash_owned = content_hash.clone();
            let temp_file_owned = temp_file.clone();
            match retry::with_retry("P2P download", 3, Duration::from_millis(500), || {
                let dl = resolver::downloader::P2PDownloader::new(vec![fallback_node.clone()]);
                let cid = ipfs_cid_owned.clone();
                let hash = content_hash_owned.clone();
                let path = temp_file_owned.clone();
                async move { dl.download(&cid, &hash, &path).await }
            })
            .await
            {
                Ok(_) => {
                    local_tarball = Some(temp_file);
                }
                Err(e) => {
                    bail!(
                        "{} P2P download failed for verified package '{}' after 3 attempts: {}. \
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

    // Find the real package manager binary, skipping our own shim directory
    // (Unix ~/.local/bin or Windows %LOCALAPPDATA%\creg\bin).
    let real_bin = which::which_all(pm_name)?
        .find(|p| !crate::intercept::path_is_in_shim_dir(p))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Real '{}' not found in PATH (only our shim exists)",
                pm_name
            )
        })?;

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
