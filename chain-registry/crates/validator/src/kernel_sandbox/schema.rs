//! Canonical kernel behavior graph (`KernelBehaviorGraphV1`) and replayable
//! trigger specification (`TriggerSpecV1`) shared across Tier 1 (Firecracker +
//! eBPF), Tier 2 (hardened `nsjail`), and Tier 3 (standalone WASM) execution.

use common::{Finding, FindingSeverity, PackageManifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::net::IpAddr;
use std::path::Path;

pub const KERNEL_GRAPH_SCHEMA_VERSION: u16 = 1;
pub const TRIGGER_SPEC_SCHEMA_VERSION: u16 = 1;
pub const MAX_TRIGGER_ENV_VARS: usize = 64;
pub const MAX_TRIGGER_ARGS: usize = 32;
pub const MAX_KERNEL_EVENTS: usize = 4_096;

/// Execution isolation tier that produced a sandbox result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTier {
    Tier1Firecracker,
    Tier2Nsjail,
    Tier3Wasm,
    DevBypass,
}

impl ExecutionTier {
    pub fn as_engine_name(self) -> &'static str {
        match self {
            Self::Tier1Firecracker => "firecracker",
            Self::Tier2Nsjail => "nsjail",
            Self::Tier3Wasm => "wasm",
            Self::DevBypass => "dev-bypass",
        }
    }
}

impl fmt::Display for ExecutionTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_engine_name())
    }
}

/// Deterministic input vector used for multi-trigger or replayable sandbox runs
/// (`VNA-17`). Part of `SandboxCacheKey` so changing triggers invalidates cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerSpecV1 {
    pub schema_version: u16,
    #[serde(default)]
    pub env_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub clock_offset_secs: i64,
    #[serde(default)]
    pub entry_function: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
}

impl Default for TriggerSpecV1 {
    fn default() -> Self {
        Self {
            schema_version: TRIGGER_SPEC_SCHEMA_VERSION,
            env_overrides: BTreeMap::new(),
            clock_offset_secs: 0,
            entry_function: None,
            args: Vec::new(),
        }
    }
}

impl TriggerSpecV1 {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.schema_version != TRIGGER_SPEC_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported TriggerSpecV1 schema_version {} (expected {})",
                self.schema_version,
                TRIGGER_SPEC_SCHEMA_VERSION
            );
        }
        if self.env_overrides.len() > MAX_TRIGGER_ENV_VARS {
            anyhow::bail!(
                "TriggerSpecV1 env_overrides count {} exceeds max {}",
                self.env_overrides.len(),
                MAX_TRIGGER_ENV_VARS
            );
        }
        for (k, v) in &self.env_overrides {
            if k.is_empty()
                || k.len() > 128
                || k.contains('=')
                || k.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f)
            {
                anyhow::bail!("invalid TriggerSpecV1 env key '{}'", k);
            }
            if v.len() > 4096 || v.bytes().any(|b| b == 0) {
                anyhow::bail!("invalid TriggerSpecV1 env value for '{}'", k);
            }
        }
        if self.args.len() > MAX_TRIGGER_ARGS {
            anyhow::bail!(
                "TriggerSpecV1 args count {} exceeds max {}",
                self.args.len(),
                MAX_TRIGGER_ARGS
            );
        }
        for arg in &self.args {
            if arg.len() > 1024 || arg.bytes().any(|b| b == 0) {
                anyhow::bail!("invalid TriggerSpecV1 arg");
            }
        }
        if let Some(ref entry) = self.entry_function {
            if entry.is_empty()
                || entry.len() > 256
                || entry.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f)
            {
                anyhow::bail!("invalid TriggerSpecV1 entry_function");
            }
        }
        Ok(())
    }

    pub fn canonical_sha256(&self) -> String {
        let encoded = serde_json::to_vec(self).unwrap_or_default();
        hex::encode(Sha256::digest(&encoded))
    }
}

/// Deterministic kernel-level violation code emitted from sandbox telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum KernelViolationCode {
    #[serde(rename = "KB-CANARY-EXFIL")]
    KbCanaryExfil,
    #[serde(rename = "KB-MEMFD-EXEC")]
    KbMemfdExec,
    #[serde(rename = "KB-PTRACE-ESCAPE")]
    KbPtraceEscape,
    #[serde(rename = "KB-IOURING-EVASION")]
    KbIouringEvasion,
    #[serde(rename = "KB-NET-UNDECLARED")]
    KbNetUndeclared,
    #[serde(rename = "KB-FS-PERSIST")]
    KbFsPersist,
    #[serde(rename = "KB-SECCOMP-VIOLATION")]
    KbSeccompViolation,
    #[serde(rename = "KB-ROOTFS-DIGEST-MISMATCH")]
    KbRootfsDigestMismatch,
    #[serde(rename = "KB-PROC-UNDECLARED")]
    KbProcUndeclared,
}

impl KernelViolationCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KbCanaryExfil => "KB-CANARY-EXFIL",
            Self::KbMemfdExec => "KB-MEMFD-EXEC",
            Self::KbPtraceEscape => "KB-PTRACE-ESCAPE",
            Self::KbIouringEvasion => "KB-IOURING-EVASION",
            Self::KbNetUndeclared => "KB-NET-UNDECLARED",
            Self::KbFsPersist => "KB-FS-PERSIST",
            Self::KbSeccompViolation => "KB-SECCOMP-VIOLATION",
            Self::KbRootfsDigestMismatch => "KB-ROOTFS-DIGEST-MISMATCH",
            Self::KbProcUndeclared => "KB-PROC-UNDECLARED",
        }
    }

    pub fn default_severity(self) -> FindingSeverity {
        match self {
            Self::KbCanaryExfil
            | Self::KbMemfdExec
            | Self::KbPtraceEscape
            | Self::KbIouringEvasion
            | Self::KbSeccompViolation
            | Self::KbRootfsDigestMismatch
            | Self::KbProcUndeclared => FindingSeverity::Critical,
            Self::KbNetUndeclared | Self::KbFsPersist => FindingSeverity::High,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::KbCanaryExfil => "Canary secret exfiltration detected",
            Self::KbMemfdExec => "Anonymous memory / RWX execution detected",
            Self::KbPtraceEscape => "Kernel namespace or process debugging escape attempt",
            Self::KbIouringEvasion => "Blocked io_uring / eBPF syscall evasion attempt",
            Self::KbNetUndeclared => "Undeclared network egress attempted",
            Self::KbFsPersist => "Undeclared filesystem persistence write detected",
            Self::KbSeccompViolation => "Seccomp policy violation detected",
            Self::KbRootfsDigestMismatch => "Sandbox rootfs integrity verification failed",
            Self::KbProcUndeclared => "Undeclared child process execution",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelViolation {
    pub code: KernelViolationCode,
    pub severity: FindingSeverity,
    pub subject: String,
    pub detail: String,
}

impl KernelViolation {
    fn sort_key(&self) -> (KernelViolationCode, u8, &str, &str) {
        let sev = match self.severity {
            FindingSeverity::Critical => 4,
            FindingSeverity::High => 3,
            FindingSeverity::Medium => 2,
            FindingSeverity::Low => 1,
        };
        (self.code, sev, self.subject.as_str(), self.detail.as_str())
    }
}

impl Ord for KernelViolation {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl PartialOrd for KernelViolation {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalProcessNode {
    pub executable: String,
    pub argv: Vec<String>,
    pub parent_executable: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalNetworkEvent {
    pub protocol: String,
    pub destination: String,
    pub port: u16,
    pub sni: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MemoryAnomaly {
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct KernelEscapeEvent {
    pub syscall: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanaryEncoding {
    Raw,
    Base64,
    Base64Url,
    HexLower,
    HexUpper,
    UrlEncoded,
}

impl fmt::Display for CanaryEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Raw => "raw",
            Self::Base64 => "base64",
            Self::Base64Url => "base64url",
            Self::HexLower => "hex_lower",
            Self::HexUpper => "hex_upper",
            Self::UrlEncoded => "url_encoded",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanaryChannel {
    Stdout,
    Stderr,
    NetworkPayload,
    OverlayFile,
    DnsQuery,
    TlsSni,
}

impl fmt::Display for CanaryChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::NetworkPayload => "network_payload",
            Self::OverlayFile => "overlay_file",
            Self::DnsQuery => "dns_query",
            Self::TlsSni => "tls_sni",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanaryExfiltration {
    pub canary_name: String,
    pub channel: CanaryChannel,
    pub encoding: CanaryEncoding,
    pub context: String,
}

/// Engine-normalized raw kernel event captured by eBPF CO-RE (Tier 1) or
/// `nsjail` seccomp/audit + host sinkhole logs (Tier 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "hook", rename_all = "snake_case")]
pub enum RawKernelEvent {
    Exec {
        executable: String,
        #[serde(default)]
        argv: Vec<String>,
        #[serde(default)]
        parent: Option<String>,
    },
    OpenWrite {
        path: String,
    },
    Network {
        protocol: String,
        destination: String,
        #[serde(default)]
        port: u16,
        #[serde(default)]
        sni: Option<String>,
    },
    MemfdCreate {
        #[serde(default)]
        name: String,
    },
    MemoryRwx {
        kind: String,
        #[serde(default)]
        detail: String,
    },
    KernelEscape {
        syscall: String,
        #[serde(default)]
        detail: String,
    },
    SeccompViolation {
        syscall: String,
        #[serde(default)]
        detail: String,
    },
}

/// Normalized deterministic execution behavior graph emitted by Tier 1 / Tier 2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelBehaviorGraphV1 {
    pub schema_version: u16,
    pub tier: ExecutionTier,
    pub trigger_spec_sha256: String,
    pub processes: Vec<CanonicalProcessNode>,
    pub files_written: Vec<String>,
    pub overlay_added: Vec<String>,
    pub overlay_modified: Vec<String>,
    pub network_events: Vec<CanonicalNetworkEvent>,
    pub memory_anomalies: Vec<MemoryAnomaly>,
    pub kernel_escapes: Vec<KernelEscapeEvent>,
    pub canary_exfiltrations: Vec<CanaryExfiltration>,
    pub violations: Vec<KernelViolation>,
}

#[derive(Serialize)]
struct CanonicalDigestView<'a> {
    schema_version: u16,
    trigger_spec_sha256: &'a str,
    processes: &'a [CanonicalProcessNode],
    files_written: &'a [String],
    overlay_added: &'a [String],
    overlay_modified: &'a [String],
    network_events: &'a [CanonicalNetworkEvent],
    memory_anomalies: &'a [MemoryAnomaly],
    kernel_escapes: &'a [KernelEscapeEvent],
    canary_exfiltrations: &'a [CanaryExfiltration],
    violations: &'a [KernelViolation],
}

impl KernelBehaviorGraphV1 {
    /// Deterministic SHA-256 over the canonicalized behavior graph with runtime
    /// PIDs, ASLR addresses, timestamps, and per-run directory UUIDs stripped.
    pub fn canonical_digest(&self) -> String {
        let view = CanonicalDigestView {
            schema_version: self.schema_version,
            trigger_spec_sha256: &self.trigger_spec_sha256,
            processes: &self.processes,
            files_written: &self.files_written,
            overlay_added: &self.overlay_added,
            overlay_modified: &self.overlay_modified,
            network_events: &self.network_events,
            memory_anomalies: &self.memory_anomalies,
            kernel_escapes: &self.kernel_escapes,
            canary_exfiltrations: &self.canary_exfiltrations,
            violations: &self.violations,
        };
        let bytes = serde_json::to_vec(&view).unwrap_or_default();
        hex::encode(Sha256::digest(&bytes))
    }

    pub fn into_findings(&self) -> Vec<Finding> {
        self.violations
            .iter()
            .map(|v| Finding {
                id: v.code.as_str().to_string(),
                title: v.code.title().to_string(),
                severity: v.severity,
                description: format!("{}: {}", v.subject, v.detail),
                file: "kernel-sandbox".to_string(),
                line: None,
            })
            .collect()
    }
}

/// Normalize volatile paths, PIDs, ASLR hex pointers, and temp run UUIDs so
/// `KernelBehaviorGraphV1::canonical_digest` is deterministic across runs.
pub fn normalize_volatile_text(input: &str) -> String {
    let mut s = input.replace('\\', "/");
    for prefix in ["creg-sandbox-", "creg-fc-", "creg-decoy-", "creg-tmp-"] {
        while let Some(idx) = s.find(prefix) {
            let after = &s[idx + prefix.len()..];
            let token_len = after
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
                .count();
            let mut replacement = String::with_capacity(s.len());
            replacement.push_str(&s[..idx]);
            replacement.push_str("<run>");
            replacement.push_str(&after[token_len..]);
            s = replacement;
        }
    }
    s = strip_hex_addresses(&s);
    s = strip_id_kv(&s, "pid=");
    s = strip_id_kv(&s, "tid=");
    strip_id_kv(&s, "ts=")
}

fn strip_hex_addresses(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 8 <= bytes.len()
            && bytes[i] == b'0'
            && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
        {
            let hex_len = bytes[i + 2..]
                .iter()
                .take_while(|b| b.is_ascii_hexdigit())
                .count();
            if (6..=16).contains(&hex_len) {
                out.push_str("<addr>");
                i += 2 + hex_len;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn strip_id_kv(input: &str, key: &str) -> String {
    let mut out = input.to_string();
    let mut search_from = 0;
    while let Some(rel) = out[search_from..].find(key) {
        let start = search_from + rel;
        let digits_start = start + key.len();
        let digits_len = out.as_bytes()[digits_start..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits_len > 0 {
            out.replace_range(digits_start..digits_start + digits_len, "<id>");
            search_from = digits_start + "<id>".len();
        } else {
            search_from = digits_start;
        }
    }
    out
}

fn host_allowed_by_manifest(
    dest: &str,
    sni: Option<&str>,
    manifest: &PackageManifest,
    resolved_ips: &HashSet<String>,
) -> bool {
    let check_single = |candidate: &str| {
        let norm = candidate.trim().trim_end_matches('.').to_ascii_lowercase();
        if norm.is_empty() {
            return true;
        }
        if norm.parse::<IpAddr>().is_ok() {
            return resolved_ips.contains(&norm);
        }
        manifest.allowed_network_hosts.iter().any(|decl| {
            let decl_norm = decl.trim().trim_end_matches('.').to_ascii_lowercase();
            !decl_norm.is_empty() && (norm == decl_norm || norm.ends_with(&format!(".{decl_norm}")))
        })
    };
    if let Some(sni_host) = sni {
        if !check_single(sni_host) {
            return false;
        }
    }
    check_single(dest)
}

fn path_allowed_by_manifest(path: &str, manifest: &PackageManifest) -> bool {
    manifest.allowed_fs_writes.iter().any(|declared| {
        let d = normalize_volatile_text(declared);
        path == d || path.starts_with(&format!("{}/", d.trim_end_matches('/')))
    })
}

fn process_allowed_by_manifest(executable: &str, manifest: &PackageManifest) -> (bool, bool) {
    if !manifest.spawns_processes {
        return (false, false);
    }
    if manifest.allowed_process_spawns.is_empty() {
        return (true, false);
    }
    let binary_name = Path::new(executable)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let allowed = manifest
        .allowed_process_spawns
        .iter()
        .any(|decl| executable == decl || binary_name == *decl);
    (allowed, true)
}

/// Deterministic normalizer turning raw kernel probe telemetry, overlayfs diffs,
/// and canary detections into a canonical `KernelBehaviorGraphV1`.
#[allow(clippy::too_many_arguments)]
pub fn normalize_telemetry(
    tier: ExecutionTier,
    trigger_spec: &TriggerSpecV1,
    manifest: &PackageManifest,
    resolved_ips: &HashSet<String>,
    raw_events: &[RawKernelEvent],
    overlay_added: &[String],
    overlay_modified: &[String],
    canary_exfiltrations: &[CanaryExfiltration],
) -> KernelBehaviorGraphV1 {
    let mut processes = BTreeSet::new();
    let mut files_written = BTreeSet::new();
    let mut network_events = BTreeSet::new();
    let mut memory_anomalies = BTreeSet::new();
    let mut kernel_escapes = BTreeSet::new();
    let mut violations = BTreeSet::new();

    for event in raw_events.iter().take(MAX_KERNEL_EVENTS) {
        match event {
            RawKernelEvent::Exec {
                executable,
                argv,
                parent,
            } => {
                let norm_exe = normalize_volatile_text(executable);
                let norm_argv = argv
                    .iter()
                    .map(|a| normalize_volatile_text(a))
                    .collect::<Vec<_>>();
                let norm_parent = parent.as_deref().map(normalize_volatile_text);
                let (allowed, had_allowlist) = process_allowed_by_manifest(&norm_exe, manifest);
                if !allowed {
                    violations.insert(KernelViolation {
                        code: KernelViolationCode::KbProcUndeclared,
                        severity: if had_allowlist {
                            FindingSeverity::High
                        } else {
                            FindingSeverity::Critical
                        },
                        subject: norm_exe.clone(),
                        detail: if had_allowlist {
                            "process binary not declared in allowed_process_spawns".into()
                        } else {
                            "undeclared child process execution while spawns_processes=false".into()
                        },
                    });
                }
                processes.insert(CanonicalProcessNode {
                    executable: norm_exe,
                    argv: norm_argv,
                    parent_executable: norm_parent,
                });
            }
            RawKernelEvent::OpenWrite { path } => {
                let norm_path = normalize_volatile_text(path);
                if !path_allowed_by_manifest(&norm_path, manifest) {
                    violations.insert(KernelViolation {
                        code: KernelViolationCode::KbFsPersist,
                        severity: FindingSeverity::High,
                        subject: norm_path.clone(),
                        detail: "undeclared filesystem write outside allowed_fs_writes".into(),
                    });
                }
                files_written.insert(norm_path);
            }
            RawKernelEvent::Network {
                protocol,
                destination,
                port,
                sni,
            } => {
                let norm_dest = normalize_volatile_text(destination)
                    .trim()
                    .trim_end_matches('.')
                    .to_ascii_lowercase();
                let norm_sni = sni.as_deref().map(|s| {
                    normalize_volatile_text(s)
                        .trim_end_matches('.')
                        .to_ascii_lowercase()
                });
                if !host_allowed_by_manifest(
                    &norm_dest,
                    norm_sni.as_deref(),
                    manifest,
                    resolved_ips,
                ) {
                    let display = norm_sni.as_deref().unwrap_or(&norm_dest).to_string();
                    violations.insert(KernelViolation {
                        code: KernelViolationCode::KbNetUndeclared,
                        severity: FindingSeverity::High,
                        subject: display,
                        detail: format!("undeclared {} egress to port {}", protocol, port),
                    });
                }
                network_events.insert(CanonicalNetworkEvent {
                    protocol: protocol.to_ascii_lowercase(),
                    destination: norm_dest,
                    port: *port,
                    sni: norm_sni,
                });
            }
            RawKernelEvent::MemfdCreate { name } => {
                let norm = normalize_volatile_text(name);
                memory_anomalies.insert(MemoryAnomaly {
                    kind: "memfd_create".into(),
                    detail: norm.clone(),
                });
                violations.insert(KernelViolation {
                    code: KernelViolationCode::KbMemfdExec,
                    severity: FindingSeverity::Critical,
                    subject: if norm.is_empty() {
                        "<anonymous-memfd>".into()
                    } else {
                        norm
                    },
                    detail: "memfd_create anonymous executable memory creation attempted".into(),
                });
            }
            RawKernelEvent::MemoryRwx { kind, detail } => {
                let norm = normalize_volatile_text(detail);
                memory_anomalies.insert(MemoryAnomaly {
                    kind: kind.to_ascii_lowercase(),
                    detail: norm.clone(),
                });
                violations.insert(KernelViolation {
                    code: KernelViolationCode::KbMemfdExec,
                    severity: FindingSeverity::Critical,
                    subject: kind.clone(),
                    detail: if norm.is_empty() {
                        "PROT_WRITE | PROT_EXEC page mapping attempted".into()
                    } else {
                        norm
                    },
                });
            }
            RawKernelEvent::KernelEscape { syscall, detail } => {
                let norm_sys = syscall.trim().to_ascii_lowercase();
                let norm_det = normalize_volatile_text(detail);
                kernel_escapes.insert(KernelEscapeEvent {
                    syscall: norm_sys.clone(),
                    detail: norm_det.clone(),
                });
                let code = if matches!(norm_sys.as_str(), "io_uring_setup" | "bpf") {
                    KernelViolationCode::KbIouringEvasion
                } else {
                    KernelViolationCode::KbPtraceEscape
                };
                violations.insert(KernelViolation {
                    code,
                    severity: FindingSeverity::Critical,
                    subject: norm_sys,
                    detail: if norm_det.is_empty() {
                        "privileged/escape syscall attempted".into()
                    } else {
                        norm_det
                    },
                });
            }
            RawKernelEvent::SeccompViolation { syscall, detail } => {
                let norm_sys = syscall.trim().to_ascii_lowercase();
                let norm_det = normalize_volatile_text(detail);
                kernel_escapes.insert(KernelEscapeEvent {
                    syscall: norm_sys.clone(),
                    detail: norm_det.clone(),
                });
                violations.insert(KernelViolation {
                    code: KernelViolationCode::KbSeccompViolation,
                    severity: FindingSeverity::Critical,
                    subject: norm_sys,
                    detail: if norm_det.is_empty() {
                        "syscall blocked by seccomp policy".into()
                    } else {
                        norm_det
                    },
                });
            }
        }
    }

    let overlay_added_norm: BTreeSet<String> = overlay_added
        .iter()
        .map(|p| normalize_volatile_text(p))
        .collect();
    let overlay_modified_norm: BTreeSet<String> = overlay_modified
        .iter()
        .map(|p| normalize_volatile_text(p))
        .collect();

    for path in overlay_added_norm
        .iter()
        .chain(overlay_modified_norm.iter())
    {
        if !path_allowed_by_manifest(path, manifest) {
            violations.insert(KernelViolation {
                code: KernelViolationCode::KbFsPersist,
                severity: FindingSeverity::High,
                subject: path.clone(),
                detail: "overlayfs persistence file write outside allowed_fs_writes".into(),
            });
        }
    }

    let mut exfils = BTreeSet::new();
    for leak in canary_exfiltrations {
        let norm_leak = CanaryExfiltration {
            canary_name: leak.canary_name.clone(),
            channel: leak.channel,
            encoding: leak.encoding,
            context: normalize_volatile_text(&leak.context),
        };
        violations.insert(KernelViolation {
            code: KernelViolationCode::KbCanaryExfil,
            severity: FindingSeverity::Critical,
            subject: norm_leak.canary_name.clone(),
            detail: format!(
                "canary exfiltrated via {} ({}) [{}]",
                norm_leak.channel, norm_leak.encoding, norm_leak.context
            ),
        });
        exfils.insert(norm_leak);
    }

    KernelBehaviorGraphV1 {
        schema_version: KERNEL_GRAPH_SCHEMA_VERSION,
        tier,
        trigger_spec_sha256: trigger_spec.canonical_sha256(),
        processes: processes.into_iter().collect(),
        files_written: files_written.into_iter().collect(),
        overlay_added: overlay_added_norm.into_iter().collect(),
        overlay_modified: overlay_modified_norm.into_iter().collect(),
        network_events: network_events.into_iter().collect(),
        memory_anomalies: memory_anomalies.into_iter().collect(),
        kernel_escapes: kernel_escapes.into_iter().collect(),
        canary_exfiltrations: exfils.into_iter().collect(),
        violations: violations.into_iter().collect(),
    }
}
