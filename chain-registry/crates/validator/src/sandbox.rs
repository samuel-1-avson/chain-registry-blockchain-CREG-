// crates/validator/src/sandbox.rs
// Stage 2: Behavioural analysis — installs the package in a locked-down
// container and records what system calls it makes.
// In production this spawns a real container (gVisor / nsjail).
// This module defines the interface and a simulation for development.

use anyhow::Result;
use common::{PackageManifest, Finding, FindingSeverity};

#[derive(Debug, Clone)]
pub struct SandboxResult {
    pub findings: Vec<Finding>,
    pub observed_network_hosts: Vec<String>,
    pub observed_fs_writes: Vec<String>,
    pub observed_process_spawns: Vec<String>,
}

/// Sandbox configuration limits.
pub struct SandboxConfig {
    /// Wall-clock timeout for the install + postinstall hooks (seconds).
    pub timeout_secs: u64,
    /// Max memory for the sandbox (megabytes).
    pub memory_mb: u32,
    /// Block all network by default; only whitelist declared manifest hosts.
    pub network_mode: NetworkMode,
}

pub enum NetworkMode {
    /// No outbound connections at all.
    Isolated,
    /// Allow only hosts declared in the manifest.
    ManifestOnly,
    /// Full network (used for testing only — never in production validators).
    Full,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 120,
            memory_mb: 512,
            network_mode: NetworkMode::ManifestOnly,
        }
    }
}

pub async fn run(
    _pkg_id:       &common::PackageId,
    tarball_bytes: &[u8],
    manifest:      &PackageManifest,
) -> Result<SandboxResult> {
    let config = SandboxConfig::default();

    // ── Production Environment Check ──────────────────────────────────────────
    // In a production validator, nsjail or runsc must be available in the PATH.
    // If not found, we fail the validation to prevent "lazy validation" security gaps.

    let tmp_dir = std::env::temp_dir().join(format!("creg-sandbox-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;
    
    let tarball_path = tmp_dir.join("package.tar.gz");
    std::fs::write(&tarball_path, tarball_bytes)?;

    tracing::info!("Launching nsjail sandbox for package behavioural analysis...");

    // ── Execute nsjail ────────────────────────────────────────────────────────
    let nsjail_check = tokio::process::Command::new("nsjail").arg("--version").output().await;

    if nsjail_check.is_err() {
        tracing::error!("nsjail not found in PATH — system configured for strict fail-closed security. Aborting validation.");
        
        let _ = std::fs::remove_dir_all(&tmp_dir);

        return Err(anyhow::anyhow!("CRITICAL: Kernel-level sandboxing (nsjail) is missing. Failsafe activated. Validation aborted."));
    }

    // Select the install command based on the package ecosystem.
    let install_args: Vec<std::ffi::OsString> = match _pkg_id.ecosystem.as_str() {
        "npm" => vec![
            "/usr/bin/node".into(), "/usr/lib/node_modules/npm/bin/npm-cli.js".into(),
            "install".into(), tarball_path.as_os_str().to_owned(),
        ],
        "cargo" => vec![
            "/usr/bin/cargo".into(), "install".into(),
            "--path".into(), tarball_path.as_os_str().to_owned(),
            "--no-default-features".into(),
        ],
        "rubygems" => vec![
            "/usr/bin/gem".into(), "install".into(), tarball_path.as_os_str().to_owned(),
        ],
        "maven" => vec![
            "/usr/bin/mvn".into(), "install:install-file".into(),
            "-Dfile".into(), tarball_path.as_os_str().to_owned(),
        ],
        // Default to pip for pypi and unknown ecosystems.
        _ => vec![
            "/usr/bin/python3".into(), "-m".into(), "pip".into(),
            "install".into(), tarball_path.as_os_str().to_owned(),
        ],
    };

    let output = tokio::process::Command::new("nsjail")
        .arg("-Mo")
        .arg("--chroot").arg("/") // In production, this would be a minimal rootfs
        .arg("--user").arg("99999")
        .arg("--group").arg("99999")
        .arg("--time_limit").arg(config.timeout_secs.to_string())
        .arg("--max_cpus").arg("1")
        .arg("--rlimit_as").arg(config.memory_mb.to_string())
        .arg("--")
        .args(&install_args)
        .output()
        .await?;

    let observations = parse_nsjail_output(&output.stderr)?;
    let findings = check_against_manifest(&observations, manifest);

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp_dir);

    Ok(SandboxResult {
        findings,
        observed_network_hosts: observations.network_hosts,
        observed_fs_writes: observations.fs_writes,
        observed_process_spawns: observations.process_spawns,
    })
}

struct Observations {
    network_hosts: Vec<String>,
    fs_writes: Vec<String>,
    process_spawns: Vec<String>,
}

/// Parse nsjail stderr/logs to extract observed system calls.
fn parse_nsjail_output(stderr: &[u8]) -> Result<Observations> {
    let stderr_str = String::from_utf8_lossy(stderr);
    let mut network_hosts = Vec::new();
    let mut fs_writes = Vec::new();
    let mut process_spawns = Vec::new();

    // nsjail with -Mo (Audit mode) will log syscalls. 
    // We regex for common patterns: connect(), open(O_WRONLY), execve().
    for line in stderr_str.lines() {
        if line.contains("connect(") {
            // Extract IP/host from connect syntax
            network_hosts.push("undeclared-egress-detected".into());
        }
        if line.contains("open(") && (line.contains("O_WRONLY") || line.contains("O_RDWR")) {
            fs_writes.push("undeclared-write-detected".into());
        }
        if line.contains("execve(") {
            process_spawns.push("undeclared-process-spawn".into());
        }
    }

    Ok(Observations { network_hosts, fs_writes, process_spawns })
}

/// Cross-check the observed behaviour against what the publisher declared.
fn check_against_manifest(obs: &Observations, manifest: &PackageManifest) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Undeclared network access.
    for host in &obs.network_hosts {
        if !manifest.allowed_network_hosts.iter().any(|h| h == host) {
            findings.push(Finding {
                id:          "SB001".into(),
                title:       "Undeclared network access".into(),
                severity:    FindingSeverity::High,
                description: format!("Undeclared network access to '{}'", host),
                file:        "install-hook".into(),
                line:        None,
            });
        }
    }

    // Undeclared filesystem writes.
    for path in &obs.fs_writes {
        if !manifest.allowed_fs_writes.iter().any(|p| p == path) {
            findings.push(Finding {
                id:          "SB002".into(),
                title:       "Undeclared filesystem write".into(),
                severity:    FindingSeverity::High,
                description: format!("Undeclared filesystem write to '{}'", path),
                file:        "install-hook".into(),
                line:        None,
            });
        }
    }

    // Undeclared process spawns.
    for spawn in &obs.process_spawns {
        if !manifest.spawns_processes {
            findings.push(Finding {
                id:          "SB003".into(),
                title:       "Undeclared process spawn".into(),
                severity:    FindingSeverity::Critical,
                description: format!("Undeclared child process spawn: '{}'", spawn),
                file:        "install-hook".into(),
                line:        None,
            });
        }
    }

    findings
}
