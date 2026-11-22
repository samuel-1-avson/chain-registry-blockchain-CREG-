//! Tier 2 Hardened `nsjail` Sandbox + Digest-Pinned Alpine Rootfs (`VNA-14`, `VNA-15`).
//!
//! Remediates all VNA-14 vulnerabilities:
//! - NEVER falls back to host `/` rootfs; verifies per-ecosystem rootfs
//!   SHA-256 against `rootfs-digests.json` before execution.
//! - NEVER executes without an existing seccomp config file (`nsjail-seccomp.cfg`).
//! - Mounts the untrusted tarball read-only at `/pkg/package.tar.gz` and runs
//!   ecosystem install commands against `/pkg/package.tar.gz` (never raw host
//!   temp paths) with NO `>/dev/null 2>&1` suppression.
//! - Enforces `--cgroup_pids_max 64`, rlimits, decoy home bind-mount with
//!   canary tokens, and normalized `KernelBehaviorGraphV1` output.

use super::canary::CanaryProfile;
use super::schema::{
    normalize_telemetry, CanaryChannel, ExecutionTier, KernelBehaviorGraphV1, RawKernelEvent,
    TriggerSpecV1, MAX_KERNEL_EVENTS,
};
use anyhow::{Context, Result};
use common::{PackageId, PackageManifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const ROOTFS_DIGEST_MANIFEST_FILENAME: &str = "rootfs-digests.json";
pub const GUEST_PKG_PATH: &str = "/pkg/package.tar.gz";
pub const GUEST_HOME_PATH: &str = "/home/sandbox";
pub const MAX_NSJAIL_STDIO_BYTES: usize = 256 * 1024;
pub const NSJAIL_PIDS_MAX: u32 = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootfsDigestEntry {
    pub sha256: String,
    pub file_count: usize,
    #[serde(default)]
    pub base_image: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootfsDigestManifest {
    pub schema_version: u16,
    pub ecosystems: BTreeMap<String, RootfsDigestEntry>,
}

#[derive(Debug, Clone)]
pub struct VerifiedRootfs {
    pub ecosystem: String,
    pub rootfs_dir: PathBuf,
    pub sha256: String,
    pub file_count: usize,
}

#[derive(Debug, Clone)]
pub struct NsjailInvocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub chroot_dir: PathBuf,
    pub rootfs_sha256: String,
}

pub fn normalize_sandbox_ecosystem(ecosystem: &str) -> Result<&'static str> {
    match ecosystem.trim().to_ascii_lowercase().as_str() {
        "npm" => Ok("npm"),
        "pip" | "pypi" | "python" => Ok("pip"),
        "cargo" | "rust" => Ok("cargo"),
        "rubygems" | "gem" | "ruby" => Ok("rubygems"),
        "maven" | "java" => Ok("maven"),
        other => anyhow::bail!("unsupported sandbox ecosystem '{}'", other),
    }
}

/// Deterministic SHA-256 over a rootfs directory tree without following
/// symlinks (immune to symlink escape loops and cross-host inode differences).
pub fn compute_directory_tree_sha256(dir: &Path) -> Result<(String, usize)> {
    if !dir.is_dir() {
        anyhow::bail!("rootfs path '{}' is not a directory", dir.display());
    }
    let mut rel_entries: Vec<(String, PathBuf, std::fs::FileType)> = Vec::new();
    let mut stack = vec![(String::new(), dir.to_path_buf())];

    while let Some((rel_prefix, current)) = stack.pop() {
        let rd = std::fs::read_dir(&current)
            .with_context(|| format!("failed to read rootfs dir {}", current.display()))?;
        for entry in rd {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains('\n') || name.contains('\0') {
                anyhow::bail!("invalid control character in rootfs filename");
            }
            let rel = if rel_prefix.is_empty() {
                name
            } else {
                format!("{}/{}", rel_prefix, name)
            };
            let path = entry.path();
            let ft = entry.file_type()?;
            if ft.is_dir() {
                stack.push((rel.clone(), path.clone()));
            }
            rel_entries.push((rel, path, ft));
        }
    }

    rel_entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut tree_hasher = Sha256::new();
    tree_hasher.update(b"CREG-ROOTFS-TREE-V1\n");
    let mut regular_files = 0usize;

    for (rel, full_path, ft) in rel_entries {
        if ft.is_symlink() {
            let target = std::fs::read_link(&full_path)?;
            let target_str = target.to_string_lossy().replace('\\', "/");
            tree_hasher.update(format!("l {} {}\n", rel, target_str).as_bytes());
        } else if ft.is_dir() {
            tree_hasher.update(format!("d {}\n", rel).as_bytes());
        } else if ft.is_file() {
            regular_files += 1;
            let mut f = std::fs::File::open(&full_path)?;
            let mut file_hasher = Sha256::new();
            let mut buf = [0u8; 16 * 1024];
            let mut size = 0u64;
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                file_hasher.update(&buf[..n]);
            }
            let digest = hex::encode(file_hasher.finalize());
            tree_hasher.update(format!("f {} {} {}\n", rel, size, digest).as_bytes());
        }
    }

    Ok((hex::encode(tree_hasher.finalize()), regular_files))
}

/// Strictly resolve and verify an ecosystem rootfs directory against
/// `<rootfs_base>/rootfs-digests.json`. NEVER falls back to `/`.
pub fn resolve_rootfs_strict(rootfs_base: &Path, ecosystem: &str) -> Result<VerifiedRootfs> {
    let eco = normalize_sandbox_ecosystem(ecosystem)?;
    if !rootfs_base.is_dir() {
        anyhow::bail!(
            "KB-ROOTFS-DIGEST-MISMATCH: sandbox rootfs base directory missing at '{}'",
            rootfs_base.display()
        );
    }
    let manifest_path = rootfs_base.join(ROOTFS_DIGEST_MANIFEST_FILENAME);
    let raw_manifest = std::fs::read_to_string(&manifest_path).with_context(|| {
        format!(
            "KB-ROOTFS-DIGEST-MISMATCH: missing or unreadable rootfs manifest at '{}'",
            manifest_path.display()
        )
    })?;
    let manifest: RootfsDigestManifest =
        serde_json::from_str(&raw_manifest).with_context(|| {
            format!(
                "KB-ROOTFS-DIGEST-MISMATCH: corrupt rootfs manifest '{}'",
                manifest_path.display()
            )
        })?;
    if manifest.schema_version != 1 {
        anyhow::bail!(
            "KB-ROOTFS-DIGEST-MISMATCH: unsupported rootfs manifest schema {}",
            manifest.schema_version
        );
    }
    let pinned = manifest.ecosystems.get(eco).with_context(|| {
        format!(
            "KB-ROOTFS-DIGEST-MISMATCH: no pinned digest for ecosystem '{}'",
            eco
        )
    })?;

    let eco_dir = rootfs_base.join(eco);
    let (actual_sha256, actual_files) =
        compute_directory_tree_sha256(&eco_dir).with_context(|| {
            format!(
                "KB-ROOTFS-DIGEST-MISMATCH: ecosystem rootfs '{}' invalid or missing",
                eco_dir.display()
            )
        })?;

    if actual_sha256 != pinned.sha256 || actual_files != pinned.file_count {
        anyhow::bail!(
            "KB-ROOTFS-DIGEST-MISMATCH: rootfs '{}' digest/count mismatch (expected {} / {}, got {} / {})",
            eco_dir.display(),
            pinned.sha256,
            pinned.file_count,
            actual_sha256,
            actual_files
        );
    }

    Ok(VerifiedRootfs {
        ecosystem: eco.to_string(),
        rootfs_dir: eco_dir,
        sha256: actual_sha256,
        file_count: actual_files,
    })
}

/// Write a `rootfs-digests.json` manifest for testing or rootfs build scripts.
pub fn write_rootfs_digest_manifest(
    rootfs_base: &Path,
    ecosystems: &[&str],
) -> Result<RootfsDigestManifest> {
    let mut map = BTreeMap::new();
    for eco in ecosystems {
        let dir = rootfs_base.join(eco);
        let (sha256, file_count) = compute_directory_tree_sha256(&dir)?;
        map.insert(
            (*eco).to_string(),
            RootfsDigestEntry {
                sha256,
                file_count,
                base_image: format!("creg-rootfs-{}:pinned", eco),
            },
        );
    }
    let manifest = RootfsDigestManifest {
        schema_version: 1,
        ecosystems: map,
    };
    let path = rootfs_base.join(ROOTFS_DIGEST_MANIFEST_FILENAME);
    std::fs::write(path, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(manifest)
}

/// Select clean ecosystem install command referencing `/pkg/package.tar.gz` with
/// ZERO output suppression (`>/dev/null 2>&1`).
pub fn nsjail_install_command(ecosystem: &str) -> Result<Vec<String>> {
    let eco = normalize_sandbox_ecosystem(ecosystem)?;
    let cmd = match eco {
        "npm" => format!(
            "/usr/bin/node /usr/lib/node_modules/npm/bin/npm-cli.js install {} --offline --ignore-scripts=false",
            GUEST_PKG_PATH
        ),
        "cargo" => format!(
            "/usr/bin/cargo install --path {} --offline --no-default-features",
            GUEST_PKG_PATH
        ),
        "rubygems" => format!("/usr/bin/gem install --local {}", GUEST_PKG_PATH),
        "maven" => format!(
            "/usr/bin/mvn -B -o install:install-file -Dfile={}",
            GUEST_PKG_PATH
        ),
        _ => format!(
            "/usr/bin/python3 -m pip install --no-index --no-build-isolation {}",
            GUEST_PKG_PATH
        ),
    };
    Ok(vec!["/bin/sh".into(), "-c".into(), cmd])
}

/// Build a fail-closed Tier 2 `nsjail` argument vector.
#[allow(clippy::too_many_arguments)]
pub fn build_nsjail_args(
    pkg_id: &PackageId,
    host_tarball_path: &Path,
    decoy_home_dir: &Path,
    seccomp_cfg_path: Option<&Path>,
    rootfs_base_dir: Option<&Path>,
    memory_mb: u32,
    timeout_secs: u64,
    canary: &CanaryProfile,
    trigger_spec: &TriggerSpecV1,
) -> Result<NsjailInvocation> {
    trigger_spec.validate()?;

    let cfg_path = seccomp_cfg_path
        .filter(|p| p.is_file())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "nsjail seccomp configuration file is missing or invalid; refusing to run without seccomp"
            )
        })?;

    let rootfs_base = rootfs_base_dir.ok_or_else(|| {
        anyhow::anyhow!("KB-ROOTFS-DIGEST-MISMATCH: sandbox rootfs_base_dir is required")
    })?;
    let verified = resolve_rootfs_strict(rootfs_base, &pkg_id.ecosystem)?;
    let install_args = nsjail_install_command(&pkg_id.ecosystem)?;

    let mut args = vec![
        "-Mo".to_string(),
        "--config".to_string(),
        cfg_path.to_string_lossy().into_owned(),
        "--log_fd".to_string(),
        "2".to_string(),
        "--chroot".to_string(),
        verified.rootfs_dir.to_string_lossy().into_owned(),
        "--bindmount_ro".to_string(),
        format!("{}:{}", host_tarball_path.to_string_lossy(), GUEST_PKG_PATH),
        "--bindmount".to_string(),
        format!("{}:{}", decoy_home_dir.to_string_lossy(), GUEST_HOME_PATH),
        "--user".to_string(),
        "99999".to_string(),
        "--group".to_string(),
        "99999".to_string(),
        "--time_limit".to_string(),
        timeout_secs.max(1).to_string(),
        "--max_cpus".to_string(),
        "1".to_string(),
        "--rlimit_as".to_string(),
        memory_mb.max(64).to_string(),
        "--rlimit_fsize".to_string(),
        "256".to_string(),
        "--rlimit_nofile".to_string(),
        "256".to_string(),
        "--cgroup_pids_max".to_string(),
        NSJAIL_PIDS_MAX.to_string(),
        "--env".to_string(),
        format!("HOME={}", GUEST_HOME_PATH),
        "--env".to_string(),
        format!("CREG_TRIGGER_SHA256={}", trigger_spec.canonical_sha256()),
    ];

    for (k, v) in canary.env_vars() {
        args.push("--env".to_string());
        args.push(format!("{}={}", k, v));
    }
    for (k, v) in &trigger_spec.env_overrides {
        args.push("--env".to_string());
        args.push(format!("{}={}", k, v));
    }

    args.push("--".to_string());
    args.extend(install_args);

    Ok(NsjailInvocation {
        program: PathBuf::from("nsjail"),
        args,
        chroot_dir: verified.rootfs_dir,
        rootfs_sha256: verified.sha256,
    })
}

/// Extract canonical `RawKernelEvent` records from `nsjail` seccomp/audit logs.
pub fn parse_nsjail_telemetry(stderr: &[u8]) -> Vec<RawKernelEvent> {
    let slice = &stderr[..stderr.len().min(MAX_NSJAIL_STDIO_BYTES)];
    let text = String::from_utf8_lossy(slice);
    let mut events = Vec::new();

    for raw_line in text.lines() {
        if events.len() >= MAX_KERNEL_EVENTS {
            break;
        }
        let line: String = raw_line
            .chars()
            .map(|c| {
                if c.is_ascii_control() && c != '\t' {
                    '?'
                } else {
                    c
                }
            })
            .collect();

        if line.contains("memfd_create") {
            let name = extract_quoted(&line).unwrap_or_default();
            events.push(RawKernelEvent::MemfdCreate { name });
            continue;
        }
        for esc in [
            "ptrace",
            "process_vm_writev",
            "io_uring_setup",
            "bpf",
            "unshare",
            "setns",
            "mount",
        ] {
            let call_match = line.contains(&format!("{esc}("));
            let seccomp_tok_match = line.contains("SECCOMP")
                && line
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .any(|tok| tok == esc);
            if call_match || seccomp_tok_match {
                events.push(RawKernelEvent::KernelEscape {
                    syscall: esc.to_string(),
                    detail: line.trim().to_string(),
                });
            }
        }
        if line.contains("seccomp violation") || line.contains("SECCOMP") && line.contains("KILL") {
            let syscall = extract_quoted(&line).unwrap_or_else(|| "blocked".into());
            events.push(RawKernelEvent::SeccompViolation {
                syscall,
                detail: line.trim().to_string(),
            });
            continue;
        }
        if line.contains("connect(") {
            if let Some(addr_start) = line.find("addr=") {
                let rest = &line[addr_start + 5..];
                let addr: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ':')
                    .collect();
                if !addr.is_empty() {
                    events.push(RawKernelEvent::Network {
                        protocol: "tcp".into(),
                        destination: addr,
                        port: extract_port(&line).unwrap_or(0),
                        sni: None,
                    });
                    continue;
                }
            }
            let host =
                extract_quoted(&line).unwrap_or_else(|| format!("unknown-egress:{}", line.trim()));
            events.push(RawKernelEvent::Network {
                protocol: "socket".into(),
                destination: host,
                port: extract_port(&line).unwrap_or(0),
                sni: None,
            });
            continue;
        }
        if (line.contains("open(") || line.contains("openat("))
            && (line.contains("O_WRONLY") || line.contains("O_RDWR") || line.contains("O_CREAT"))
        {
            let path =
                extract_quoted(&line).unwrap_or_else(|| format!("unknown-write:{}", line.trim()));
            events.push(RawKernelEvent::OpenWrite { path });
            continue;
        }
        if line.contains("execve(") || line.contains("execveat(") {
            let executable =
                extract_quoted(&line).unwrap_or_else(|| format!("unknown-exec:{}", line.trim()));
            events.push(RawKernelEvent::Exec {
                executable,
                argv: vec![],
                parent: None,
            });
        }
    }

    events
}

fn extract_quoted(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    let val = &rest[..end];
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

fn extract_port(line: &str) -> Option<u16> {
    let idx = line.find("port=")?;
    let digits: String = line[idx + 5..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

pub struct NsjailOutcome {
    pub graph: KernelBehaviorGraphV1,
    pub exit_code: i32,
    pub wall_time_ms: u64,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_nsjail_sandbox_strict(
    tarball_path: &Path,
    pkg_id: &PackageId,
    seccomp_cfg_path: Option<&Path>,
    rootfs_base_dir: Option<&Path>,
    memory_mb: u32,
    timeout_secs: u64,
    manifest: &PackageManifest,
    resolved_ips: &HashSet<String>,
    trigger_spec: &TriggerSpecV1,
) -> Result<NsjailOutcome> {
    let started = Instant::now();
    let decoy_dir = tempfile::tempdir().context("create ephemeral nsjail decoy home")?;
    let canary = CanaryProfile::from_env_run();
    canary.stage_decoy_home(decoy_dir.path())?;

    let invocation = build_nsjail_args(
        pkg_id,
        tarball_path,
        decoy_dir.path(),
        seccomp_cfg_path,
        rootfs_base_dir,
        memory_mb,
        timeout_secs,
        &canary,
        trigger_spec,
    )?;

    let mut cmd = tokio::process::Command::new(&invocation.program);
    cmd.args(&invocation.args).kill_on_drop(true);
    let output = tokio::time::timeout(
        Duration::from_secs(timeout_secs.saturating_add(5)),
        cmd.output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("nsjail sandbox timed out after {}s", timeout_secs))?
    .context("failed to execute nsjail")?;

    let stdout_cap = &output.stdout[..output.stdout.len().min(MAX_NSJAIL_STDIO_BYTES)];
    let stderr_cap = &output.stderr[..output.stderr.len().min(MAX_NSJAIL_STDIO_BYTES)];

    let events = parse_nsjail_telemetry(stderr_cap);
    let mut canary_leaks = canary.scan_payload(CanaryChannel::Stdout, stdout_cap);
    canary_leaks.extend(canary.scan_payload(CanaryChannel::Stderr, stderr_cap));

    let graph = normalize_telemetry(
        ExecutionTier::Tier2Nsjail,
        trigger_spec,
        manifest,
        resolved_ips,
        &events,
        &[],
        &[],
        &canary_leaks,
    );

    Ok(NsjailOutcome {
        graph,
        exit_code: output.status.code().unwrap_or(-1),
        wall_time_ms: started.elapsed().as_millis() as u64,
    })
}

/// Check if Tier 2 (`nsjail` binary + seccomp cfg + rootfs base dir) is configured.
pub async fn detect_nsjail_ready(
    seccomp_cfg_path: Option<&Path>,
    rootfs_base_dir: Option<&Path>,
) -> bool {
    let Some(cfg) = seccomp_cfg_path else {
        return false;
    };
    let Some(base) = rootfs_base_dir else {
        return false;
    };
    if !cfg.is_file() || !base.join(ROOTFS_DIGEST_MANIFEST_FILENAME).is_file() {
        return false;
    }
    tokio::process::Command::new("nsjail")
        .arg("--help")
        .output()
        .await
        .is_ok_and(|o| o.status.success())
}
