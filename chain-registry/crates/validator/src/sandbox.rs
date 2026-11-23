//! Stage 2: 3-Tier Behavioral Kernel Sandbox (`VNA-14` – `VNA-17`).
//!
//! Engine waterfall:
//!   1. Tier 1 — Firecracker microVM + `jailer` + in-guest eBPF CO-RE + host sinkhole (Linux `/dev/kvm`)
//!   2. Tier 2 — Hardened `nsjail` + SHA-256 digest-pinned Alpine rootfs + required seccomp-BPF
//!   3. Tier 3 — Standalone WASM/WASI only (`wasmtime >= 40`, 64 KiB stdio cap, fuel + epoch timeout)
//!   4. Dev bypass — explicit `CREG_DEV_SANDBOX=true` + `CREG_TESTNET=true` (never public/production)
//!   5. Fail closed — refuse to vote without isolated behavioral verification

use crate::bundle::AnalysisBundleSet;
use crate::kernel_sandbox::{
    detect_firecracker_ready, detect_nsjail_ready, run_firecracker_sandbox,
    run_nsjail_sandbox_strict, KernelBehaviorGraphV1, TriggerSpecV1,
};
use anyhow::Result;
use common::{Finding, FindingSeverity, PackageManifest};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Instant;

async fn command_ready(program: &str, args: &[&str]) -> bool {
    match tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
    {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                "Sandbox engine '{}' unavailable: exit={} stderr={}",
                program,
                output.status,
                stderr.trim()
            );
            false
        }
        Err(err) => {
            tracing::debug!("Sandbox engine '{}' not found: {}", program, err);
            false
        }
    }
}

// ── Public Types ────────────────────────────────────────────────────────────

/// Reported sandbox engine status for health/runtime-config surfaces (`MAL-001`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SandboxStatus {
    /// Engine that will be used for behavioural analysis:
    /// "firecracker", "nsjail", "dev-bypass", or "none".
    pub engine: String,
    /// True when `CREG_DEV_SANDBOX=true` is active for local testnet development.
    pub dev_bypass: bool,
    /// True when the active engine provides hardware/kernel isolation (`firecracker` or `nsjail`).
    pub isolated: bool,
    /// True when isolated native execution is unavailable.
    pub degraded: bool,
    /// Human-readable status note for operators.
    pub note: String,
}

static SANDBOX_STATUS: tokio::sync::OnceCell<SandboxStatus> = tokio::sync::OnceCell::const_new();

/// Detect (once per process) which sandbox engine this node will use.
pub async fn engine_status() -> SandboxStatus {
    SANDBOX_STATUS
        .get_or_init(detect_engine_status)
        .await
        .clone()
}

/// For read-only observer nodes, probe validator HTTP peers for fleet sandbox posture.
pub async fn fleet_sandbox_from_peers(peer_urls: &[String]) -> Option<SandboxStatus> {
    if peer_urls.is_empty() {
        return None;
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return None,
    };

    #[derive(serde::Deserialize)]
    struct PeerHealth {
        sandbox: Option<SandboxStatus>,
    }

    for url in peer_urls {
        let health_url = format!("{}/v1/health", url.trim_end_matches('/'));
        let Ok(resp) = client.get(&health_url).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(body) = resp.json::<PeerHealth>().await else {
            continue;
        };
        if let Some(sandbox) = body.sandbox.filter(|s| s.engine != "none") {
            return Some(sandbox);
        }
    }

    None
}

fn env_flag_true(key: &str) -> bool {
    matches!(
        std::env::var(key).as_deref(),
        Ok("true") | Ok("1") | Ok("TRUE")
    )
}

/// `CREG_DEV_SANDBOX` is permitted only on explicit non-public, non-production
/// testnet/dev profiles (`ISSUE-013`).
fn dev_sandbox_bypass_allowed(
    dev_sandbox: bool,
    testnet: bool,
    public_validator: bool,
    production: bool,
) -> bool {
    dev_sandbox && testnet && !public_validator && !production
}

fn dev_sandbox_bypass_active() -> bool {
    dev_sandbox_bypass_allowed(
        env_flag_true("CREG_DEV_SANDBOX"),
        env_flag_true("CREG_TESTNET"),
        env_flag_true("CREG_PUBLIC_VALIDATOR"),
        env_flag_true("CREG_PRODUCTION"),
    )
}

async fn detect_engine_status() -> SandboxStatus {
    let dev_bypass = dev_sandbox_bypass_active();
    let cfg = SandboxConfig::default();

    if detect_firecracker_ready().await {
        return SandboxStatus {
            engine: "firecracker".into(),
            dev_bypass,
            isolated: true,
            degraded: false,
            note: "Tier 1 Firecracker microVM + jailer + in-guest eBPF CO-RE".into(),
        };
    }

    if detect_nsjail_ready(
        cfg.nsjail_config_path.as_deref(),
        cfg.rootfs_base_dir.as_deref(),
    )
    .await
    {
        return SandboxStatus {
            engine: "nsjail".into(),
            dev_bypass,
            isolated: true,
            degraded: false,
            note: "Tier 2 hardened nsjail + seccomp-BPF + digest-pinned Alpine rootfs".into(),
        };
    }

    if dev_bypass {
        return SandboxStatus {
            engine: "dev-bypass".into(),
            dev_bypass,
            isolated: false,
            degraded: true,
            note: "CREG_DEV_SANDBOX=true — behavioural analysis skipped; every package receives SB012 (High). Local development only.".into(),
        };
    }

    SandboxStatus {
        engine: "none".into(),
        dev_bypass,
        isolated: false,
        degraded: true,
        note: "no isolated sandbox engine available (Firecracker/hardened nsjail missing) — non-WASM packages fail closed and this validator abstains".into(),
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SandboxResult {
    pub findings: Vec<Finding>,
    pub observed_network_hosts: Vec<String>,
    pub observed_fs_writes: Vec<String>,
    pub observed_process_spawns: Vec<String>,
    /// Canonical normalized kernel behavior graph (Tier 1 / Tier 2).
    #[serde(default)]
    pub kernel_graph: Option<KernelBehaviorGraphV1>,
    /// Execution metrics for observability.
    pub metrics: SandboxMetrics,
}

/// Execution metrics collected during sandbox run.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SandboxMetrics {
    /// Engine used: "firecracker", "nsjail", "wasm", "dev-bypass", or "none".
    pub engine_used: String,
    /// Wall-clock time in milliseconds.
    pub wall_time_ms: u64,
    /// Process exit code (0 = success).
    pub exit_code: i32,
    /// Total number of behavioural observations recorded.
    pub observations_count: usize,
    /// Total number of findings produced.
    pub findings_count: usize,
}

/// Sandbox configuration limits & deterministic cache inputs.
pub struct SandboxConfig {
    /// Wall-clock timeout for install + hooks (seconds).
    pub timeout_secs: u64,
    /// Max memory for the sandbox (megabytes).
    pub memory_mb: u32,
    /// Network containment mode.
    pub network_mode: NetworkMode,
    /// Path to the required `nsjail` seccomp config file (protobuf text format).
    pub nsjail_config_path: Option<PathBuf>,
    /// Base directory containing digest-pinned per-ecosystem rootfs trees and `rootfs-digests.json`.
    pub rootfs_base_dir: Option<PathBuf>,
    /// Cached tarball + policy + manifest + trigger digest key.
    pub tarball_hash: Option<String>,
    /// Active policy bundle identifier (`VNA-17`).
    pub policy_bundle_id: String,
    /// Deterministic trigger input vector (`VNA-17`).
    pub trigger_spec: TriggerSpecV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkMode {
    /// No outbound connections at all.
    Isolated,
    /// Allow only hosts declared in the manifest (captured via sinkhole).
    ManifestOnly,
    /// Full network (local testing only — never in production validators).
    Full,
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(default)
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(default)
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

impl Default for SandboxConfig {
    fn default() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));

        let config_base = exe_dir.map(|d| d.join("config").join("sandbox"));

        Self {
            timeout_secs: env_u64("CREG_SANDBOX_TIMEOUT", 120),
            memory_mb: env_u32("CREG_SANDBOX_MEMORY_MB", 512),
            network_mode: NetworkMode::ManifestOnly,
            nsjail_config_path: env_path("CREG_SANDBOX_NSJAIL_CONFIG")
                .or_else(|| config_base.as_ref().map(|d| d.join("nsjail-seccomp.cfg"))),
            rootfs_base_dir: env_path("CREG_SANDBOX_ROOTFS_DIR")
                .or_else(|| config_base.as_ref().map(|d| d.join("rootfs"))),
            tarball_hash: None,
            policy_bundle_id: AnalysisBundleSet::current().policy_bundle_id,
            trigger_spec: TriggerSpecV1::default(),
        }
    }
}

/// In-memory cache for sandbox results keyed by manifest/policy/trigger hash.
static RESULT_CACHE: std::sync::LazyLock<std::sync::Mutex<HashMap<String, SandboxResult>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Canonical-keyed store for the most-recent sandbox result per package (Stage 3 diffing).
static CANONICAL_STORE: std::sync::LazyLock<std::sync::Mutex<HashMap<String, SandboxResult>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

fn sandbox_store_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CREG_SANDBOX_STORE_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Ok(data_dir) = std::env::var("CREG_DATA_DIR") {
        if !data_dir.trim().is_empty() {
            return PathBuf::from(data_dir).join("sandbox_history");
        }
    }
    PathBuf::from("data/sandbox_history")
}

fn canonical_to_filename(canonical: &str) -> String {
    let sanitized: String = canonical
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '@' | ' ' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    format!("{}.json", sanitized)
}

/// Persist a sandbox result by package canonical for future Stage 3 differential checks.
pub fn store_result(canonical: &str, result: &SandboxResult) {
    if let Ok(mut store) = CANONICAL_STORE.lock() {
        store.insert(canonical.to_string(), result.clone());
    }

    let dir = sandbox_store_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::debug!("Could not create sandbox store dir {:?}: {}", dir, e);
        return;
    }
    let file_path = dir.join(canonical_to_filename(canonical));
    if let Ok(json) = serde_json::to_string_pretty(result) {
        if let Err(e) = std::fs::write(&file_path, json) {
            tracing::warn!("Failed to persist sandbox result to {:?}: {}", file_path, e);
        } else {
            tracing::debug!("Persisted sandbox result durably for {}", canonical);
        }
    }
}

/// Retrieve a previously stored sandbox result by canonical.
pub fn get_result(canonical: &str) -> Option<SandboxResult> {
    if let Ok(store) = CANONICAL_STORE.lock() {
        if let Some(res) = store.get(canonical) {
            return Some(res.clone());
        }
    }

    let dir = sandbox_store_dir();
    let file_path = dir.join(canonical_to_filename(canonical));
    if file_path.exists() {
        if let Ok(data) = std::fs::read_to_string(&file_path) {
            if let Ok(res) = serde_json::from_str::<SandboxResult>(&data) {
                if let Ok(mut store) = CANONICAL_STORE.lock() {
                    store.insert(canonical.to_string(), res.clone());
                }
                return Some(res);
            }
        }
    }

    None
}

// ── Cache key helper (VNA-17) ───────────────────────────────────────────────

#[derive(serde::Serialize)]
struct CanonicalManifestDigestInput {
    allowed_network_hosts: Vec<String>,
    allowed_fs_writes: Vec<String>,
    spawns_processes: bool,
    allowed_process_spawns: Vec<String>,
}

fn canonical_manifest_sha256(manifest: &PackageManifest) -> String {
    let mut hosts = manifest
        .allowed_network_hosts
        .iter()
        .map(|h| h.trim().trim_end_matches('.').to_ascii_lowercase())
        .collect::<Vec<_>>();
    hosts.sort();
    hosts.dedup();

    let mut writes = manifest
        .allowed_fs_writes
        .iter()
        .map(|w| w.trim().replace('\\', "/"))
        .collect::<Vec<_>>();
    writes.sort();
    writes.dedup();

    let mut spawns = manifest
        .allowed_process_spawns
        .iter()
        .map(|p| p.trim().to_string())
        .collect::<Vec<_>>();
    spawns.sort();
    spawns.dedup();

    let canonical = CanonicalManifestDigestInput {
        allowed_network_hosts: hosts,
        allowed_fs_writes: writes,
        spawns_processes: manifest.spawns_processes,
        allowed_process_spawns: spawns,
    };
    let encoded = serde_json::to_vec(&canonical).unwrap_or_default();
    hex::encode(Sha256::digest(&encoded))
}

/// Deterministic sandbox result cache key (`VNA-17`).
///
/// Binds:
///   `tarball_sha256 || ecosystem || canonical_manifest_sha256 || policy_bundle_id || trigger_spec_sha256 || timeout || memory || network_mode`
/// so modifying any declared manifest permission, policy bundle, or trigger spec
/// invalidates stale cached verdicts.
#[allow(clippy::too_many_arguments)]
pub fn compute_sandbox_cache_key(
    tarball_bytes: &[u8],
    ecosystem: &str,
    manifest: &PackageManifest,
    policy_bundle_id: &str,
    trigger_spec: &TriggerSpecV1,
    timeout_secs: u64,
    memory_mb: u32,
    network_mode: &NetworkMode,
) -> String {
    let tarball_sha256 = hex::encode(Sha256::digest(tarball_bytes));
    let manifest_sha256 = canonical_manifest_sha256(manifest);
    let trigger_sha256 = trigger_spec.canonical_sha256();
    let network_mode_byte: u8 = match network_mode {
        NetworkMode::Isolated => 0,
        NetworkMode::ManifestOnly => 1,
        NetworkMode::Full => 2,
    };

    let mut hasher = Sha256::new();
    hasher.update(b"CREG-SANDBOX-KEY-V2:");
    hasher.update(tarball_sha256.as_bytes());
    hasher.update(b":");
    hasher.update(ecosystem.trim().to_ascii_lowercase().as_bytes());
    hasher.update(b":");
    hasher.update(manifest_sha256.as_bytes());
    hasher.update(b":");
    hasher.update(policy_bundle_id.as_bytes());
    hasher.update(b":");
    hasher.update(trigger_sha256.as_bytes());
    hasher.update(timeout_secs.to_le_bytes());
    hasher.update(memory_mb.to_le_bytes());
    hasher.update([network_mode_byte]);
    hex::encode(hasher.finalize())
}

async fn resolve_manifest_domains(hosts: &[String]) -> HashSet<String> {
    // Pure deterministic literal-IP parsing (zero live DNS queries on consensus path; VNA-20).
    let mut resolved = HashSet::new();
    for host in hosts {
        let normalized = host.trim().trim_end_matches('.');
        if let Ok(ip) = normalized.parse::<IpAddr>() {
            resolved.insert(ip.to_string());
        }
    }
    resolved
}

fn result_from_graph(
    graph: KernelBehaviorGraphV1,
    exit_code: i32,
    wall_time_ms: u64,
) -> SandboxResult {
    let engine_used = graph.tier.as_engine_name().to_string();
    let findings = graph.into_findings();
    let observed_network_hosts = graph
        .network_events
        .iter()
        .map(|ev| ev.sni.as_deref().unwrap_or(&ev.destination).to_string())
        .collect::<Vec<_>>();
    let observed_fs_writes = graph
        .files_written
        .iter()
        .chain(graph.overlay_added.iter())
        .chain(graph.overlay_modified.iter())
        .cloned()
        .collect::<Vec<_>>();
    let observed_process_spawns = graph
        .processes
        .iter()
        .map(|p| p.executable.clone())
        .collect::<Vec<_>>();
    let observations_count =
        observed_network_hosts.len() + observed_fs_writes.len() + observed_process_spawns.len();
    let findings_count = findings.len();

    SandboxResult {
        findings,
        observed_network_hosts,
        observed_fs_writes,
        observed_process_spawns,
        kernel_graph: Some(graph),
        metrics: SandboxMetrics {
            engine_used,
            wall_time_ms,
            exit_code,
            observations_count,
            findings_count,
        },
    }
}

// ── Main Entry Point ────────────────────────────────────────────────────────

pub async fn run(
    pkg_id: &common::PackageId,
    tarball_bytes: &[u8],
    manifest: &PackageManifest,
) -> Result<SandboxResult> {
    run_with_trigger(pkg_id, tarball_bytes, manifest, &TriggerSpecV1::default()).await
}

pub async fn run_with_trigger(
    pkg_id: &common::PackageId,
    tarball_bytes: &[u8],
    manifest: &PackageManifest,
    trigger_spec: &TriggerSpecV1,
) -> Result<SandboxResult> {
    trigger_spec.validate()?;

    let mut config = SandboxConfig {
        trigger_spec: trigger_spec.clone(),
        ..SandboxConfig::default()
    };
    let start_time = Instant::now();

    let cache_key = compute_sandbox_cache_key(
        tarball_bytes,
        &pkg_id.ecosystem,
        manifest,
        &config.policy_bundle_id,
        &config.trigger_spec,
        config.timeout_secs,
        config.memory_mb,
        &config.network_mode,
    );
    config.tarball_hash = Some(cache_key.clone());

    if let Ok(cache) = RESULT_CACHE.lock() {
        if let Some(cached) = cache.get(&cache_key) {
            tracing::info!("Sandbox result cache HIT for {}", pkg_id);
            return Ok(cached.clone());
        }
    }

    let tmp_dir = std::env::temp_dir().join(format!("creg-sandbox-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp_dir)?;
    let tarball_path = tmp_dir.join("package.tar.gz");
    std::fs::write(&tarball_path, tarball_bytes)?;

    let resolved_ips = resolve_manifest_domains(&manifest.allowed_network_hosts).await;

    // ── Tier 1: Firecracker microVM + jailer + in-guest eBPF CO-RE ───────────
    if detect_firecracker_ready().await {
        tracing::info!(
            "Tier 1 Firecracker detected — launching microVM sandbox for {}",
            pkg_id
        );
        let outcome = run_firecracker_sandbox(
            pkg_id,
            &tarball_path,
            manifest,
            &resolved_ips,
            trigger_spec,
            config.timeout_secs,
        )
        .await;
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let outcome = outcome?;
        let result = result_from_graph(outcome.graph, outcome.exit_code, outcome.wall_time_ms);
        cache_result(&cache_key, &result);
        return Ok(result);
    }

    // ── Tier 2: Hardened nsjail + digest-pinned Alpine rootfs + seccomp ─────
    if command_ready("nsjail", &["--help"]).await {
        tracing::info!(
            "Tier 2 nsjail detected — launching hardened namespace sandbox for {}",
            pkg_id
        );
        let outcome = run_nsjail_sandbox_strict(
            &tarball_path,
            pkg_id,
            config.nsjail_config_path.as_deref(),
            config.rootfs_base_dir.as_deref(),
            config.memory_mb,
            config.timeout_secs,
            manifest,
            &resolved_ips,
            trigger_spec,
        )
        .await;
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let outcome = outcome?;
        let result = result_from_graph(outcome.graph, outcome.exit_code, outcome.wall_time_ms);
        cache_result(&cache_key, &result);
        return Ok(result);
    }

    // ── Tier 3: Standalone WASM/WASI only (`VNA-16`) ────────────────────────
    // Strictly gated to genuine standalone WASM packages so npm/pypi/cargo
    // tarballs cannot smuggle a dummy `.wasm` file to bypass fail-closed abstention.
    if crate::wasm_sandbox::is_standalone_wasm_package(pkg_id, tarball_bytes) {
        tracing::info!("Tier 3 standalone WASM package detected for {}", pkg_id);
        let wasm_res =
            crate::wasm_sandbox::run_in_wasm(pkg_id, &tarball_path, &config, manifest).await;
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let wasm_result = wasm_res?;
        let observations_count = wasm_result.observed_network_hosts.len()
            + wasm_result.observed_fs_writes.len()
            + wasm_result.observed_process_spawns.len();
        let result = SandboxResult {
            metrics: SandboxMetrics {
                engine_used: "wasm".into(),
                wall_time_ms: start_time.elapsed().as_millis() as u64,
                exit_code: wasm_result.metrics.exit_code,
                observations_count,
                findings_count: wasm_result.findings.len(),
            },
            ..wasm_result
        };
        cache_result(&cache_key, &result);
        return Ok(result);
    }

    // ── Dev bypass (local testnet/dev only) ─────────────────────────────────
    if dev_sandbox_bypass_active() {
        tracing::warn!(
            "CREG_DEV_SANDBOX=true — behavioural analysis is SKIPPED for {}. \
             Local testnet/dev only; NEVER enabled on public/production validators.",
            pkg_id
        );
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Ok(SandboxResult {
            findings: vec![Finding {
                id: "SB012".into(),
                title: "Dev sandbox bypass active".into(),
                severity: FindingSeverity::High,
                description: "CREG_DEV_SANDBOX=true: Behavioural analysis was skipped. Operator opted in to an unsafe local-development bypass.".into(),
                file: "sandbox-engine".into(),
                line: None,
            }],
            observed_network_hosts: vec![],
            observed_fs_writes: vec![],
            observed_process_spawns: vec![],
            kernel_graph: None,
            metrics: SandboxMetrics {
                engine_used: "dev-bypass".into(),
                wall_time_ms: start_time.elapsed().as_millis() as u64,
                exit_code: 0,
                observations_count: 0,
                findings_count: 1,
            },
        });
    }

    // ── Fail Closed ─────────────────────────────────────────────────────────
    let _ = std::fs::remove_dir_all(&tmp_dir);
    tracing::error!(
        "No isolated kernel sandbox engine (Firecracker or hardened nsjail) is available for {}, \
         and package is not standalone WASM. Refusing to vote without behavioural isolation.",
        pkg_id
    );
    anyhow::bail!(
        "no isolated sandbox engine available for {} and CREG_DEV_SANDBOX bypass is not allowed; \
         refusing to vote without behavioural analysis",
        pkg_id
    )
}

fn sandbox_cache_max() -> usize {
    std::env::var("CREG_SANDBOX_CACHE_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000)
}

fn cache_result(key: &str, result: &SandboxResult) {
    if let Ok(mut cache) = RESULT_CACHE.lock() {
        let max = sandbox_cache_max();
        if cache.len() >= max {
            cache.clear();
            tracing::debug!("Sandbox RESULT_CACHE cleared at {} entries cap", max);
        }
        cache.insert(key.to_string(), result.clone());
    }
}

/// Multi-phase sandbox execution hook using distinct `TriggerSpecV1` profiles.
#[allow(dead_code)]
pub async fn run_multiphase(
    pkg_id: &common::PackageId,
    tarball_bytes: &[u8],
    manifest: &PackageManifest,
) -> Result<SandboxResult> {
    let phase1_manifest = PackageManifest {
        allowed_network_hosts: manifest.allowed_network_hosts.clone(),
        allowed_fs_writes: manifest.allowed_fs_writes.clone(),
        spawns_processes: manifest.spawns_processes,
        allowed_process_spawns: manifest.allowed_process_spawns.clone(),
        description: manifest.description.clone(),
    };

    let phase1 = run_with_trigger(
        pkg_id,
        tarball_bytes,
        &phase1_manifest,
        &TriggerSpecV1 {
            env_overrides: [("CREG_PHASE".into(), "install".into())].into(),
            ..TriggerSpecV1::default()
        },
    )
    .await?;
    if phase1
        .findings
        .iter()
        .any(|f| matches!(f.severity, FindingSeverity::Critical))
    {
        return Ok(phase1);
    }

    let phase2_manifest = PackageManifest {
        allowed_network_hosts: vec![],
        ..phase1_manifest
    };
    let phase2 = run_with_trigger(
        pkg_id,
        tarball_bytes,
        &phase2_manifest,
        &TriggerSpecV1 {
            clock_offset_secs: 86_400,
            env_overrides: [("CREG_PHASE".into(), "postinstall".into())].into(),
            ..TriggerSpecV1::default()
        },
    )
    .await?;

    let mut combined = phase1;
    combined.findings.extend(phase2.findings);
    for host in phase2.observed_network_hosts {
        if !combined.observed_network_hosts.contains(&host) {
            combined.observed_network_hosts.push(host);
        }
    }
    for path in phase2.observed_fs_writes {
        if !combined.observed_fs_writes.contains(&path) {
            combined.observed_fs_writes.push(path);
        }
    }
    for spawn in phase2.observed_process_spawns {
        if !combined.observed_process_spawns.contains(&spawn) {
            combined.observed_process_spawns.push(spawn);
        }
    }

    Ok(combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dev_sandbox_bypass_requires_testnet_and_blocks_public() {
        assert!(dev_sandbox_bypass_allowed(true, true, false, false));
        assert!(!dev_sandbox_bypass_allowed(true, false, false, false));
        assert!(!dev_sandbox_bypass_allowed(true, true, true, false));
        assert!(!dev_sandbox_bypass_allowed(true, true, false, true));
        assert!(!dev_sandbox_bypass_allowed(false, true, false, false));
    }

    #[tokio::test]
    async fn test_resolve_manifest_domains() {
        let hosts = vec![
            "127.0.0.1".to_string(),
            "8.8.8.8.".to_string(),
            " ::1 ".to_string(),
        ];
        let resolved = resolve_manifest_domains(&hosts).await;
        assert!(resolved.contains("127.0.0.1"));
        assert!(resolved.contains("8.8.8.8"));
        assert!(resolved.contains("::1"));

        let local_hosts = vec!["localhost".to_string(), "registry.example.com".to_string()];
        let resolved_local = resolve_manifest_domains(&local_hosts).await;
        assert!(resolved_local.is_empty());
    }

    #[test]
    fn test_durable_sandbox_store_and_retrieve() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let prev_dir = std::env::var("CREG_SANDBOX_STORE_DIR").ok();
        std::env::set_var("CREG_SANDBOX_STORE_DIR", temp_dir.path().to_str().unwrap());

        let canonical = "npm:test-pkg@1.2.3";
        let mock_result = SandboxResult {
            findings: vec![],
            observed_network_hosts: vec!["registry.npmjs.org".into()],
            observed_fs_writes: vec!["/tmp/test.txt".into()],
            observed_process_spawns: vec!["node".into()],
            kernel_graph: None,
            metrics: SandboxMetrics {
                engine_used: "wasm".into(),
                wall_time_ms: 42,
                exit_code: 0,
                observations_count: 3,
                findings_count: 0,
            },
        };

        store_result(canonical, &mock_result);

        let loaded = get_result(canonical).expect("should retrieve stored result");
        assert_eq!(loaded.observed_network_hosts, vec!["registry.npmjs.org"]);
        assert_eq!(loaded.observed_fs_writes, vec!["/tmp/test.txt"]);
        assert_eq!(loaded.metrics.engine_used, "wasm");

        if let Ok(mut store) = CANONICAL_STORE.lock() {
            store.clear();
        }

        let disk_loaded =
            get_result(canonical).expect("should retrieve from disk after memory clear");
        assert_eq!(disk_loaded.observed_process_spawns, vec!["node"]);

        restore_env("CREG_SANDBOX_STORE_DIR", prev_dir);
    }

    #[test]
    fn test_cache_key_sensitive_to_manifest_policy_and_trigger() {
        let tarball = b"deterministic-tarball-bytes";
        let trigger = TriggerSpecV1::default();
        let base_manifest = PackageManifest::default();

        let base_key = compute_sandbox_cache_key(
            tarball,
            "npm",
            &base_manifest,
            "policy-v1",
            &trigger,
            120,
            512,
            &NetworkMode::ManifestOnly,
        );

        let mut net_manifest = base_manifest.clone();
        net_manifest
            .allowed_network_hosts
            .push("api.example.com".into());
        assert_ne!(
            base_key,
            compute_sandbox_cache_key(
                tarball,
                "npm",
                &net_manifest,
                "policy-v1",
                &trigger,
                120,
                512,
                &NetworkMode::ManifestOnly,
            )
        );

        let mut proc_manifest = base_manifest.clone();
        proc_manifest.spawns_processes = true;
        assert_ne!(
            base_key,
            compute_sandbox_cache_key(
                tarball,
                "npm",
                &proc_manifest,
                "policy-v1",
                &trigger,
                120,
                512,
                &NetworkMode::ManifestOnly,
            )
        );

        assert_ne!(
            base_key,
            compute_sandbox_cache_key(
                tarball,
                "npm",
                &base_manifest,
                "policy-v2",
                &trigger,
                120,
                512,
                &NetworkMode::ManifestOnly,
            )
        );

        let mut alt_trigger = trigger.clone();
        alt_trigger.clock_offset_secs = 86_400;
        assert_ne!(
            base_key,
            compute_sandbox_cache_key(
                tarball,
                "npm",
                &base_manifest,
                "policy-v1",
                &alt_trigger,
                120,
                512,
                &NetworkMode::ManifestOnly,
            )
        );
    }

    fn restore_env(name: &str, previous: Option<String>) {
        match previous {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}
