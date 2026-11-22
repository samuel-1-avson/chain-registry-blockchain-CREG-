//! Tier 1 Kernel Sandbox: Firecracker microVM + `jailer` + in-guest eBPF CO-RE
//! + isolated host DNS/TLS sinkhole (`VNA-15`).

use super::canary::CanaryProfile;
use super::nsjail::{resolve_rootfs_strict, VerifiedRootfs};
use super::schema::{
    normalize_telemetry, CanaryChannel, CanaryExfiltration, ExecutionTier, KernelBehaviorGraphV1,
    RawKernelEvent, TriggerSpecV1, MAX_KERNEL_EVENTS,
};
use anyhow::{Context, Result};
use common::PackageManifest;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const DEFAULT_VSOCK_CID: u32 = 3;
pub const DEFAULT_VSOCK_PORT: u32 = 5005;
pub const DEFAULT_HOST_SINKHOLE_IP: &str = "172.16.0.1";
pub const MAX_VSOCK_STREAM_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_STDIO_CAPTURE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct FirecrackerConfig {
    pub firecracker_bin: PathBuf,
    pub jailer_bin: PathBuf,
    pub kernel_image_path: PathBuf,
    pub rootfs_base_dir: PathBuf,
    pub ebpf_dir: Option<PathBuf>,
    pub chroot_base_dir: PathBuf,
    pub uid: u32,
    pub gid: u32,
    pub vcpu_count: u8,
    pub mem_size_mib: u32,
    pub timeout_secs: u64,
}

impl FirecrackerConfig {
    pub fn from_env() -> Self {
        let env_path = |key: &str, default: &str| -> PathBuf {
            std::env::var(key)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(default))
        };
        let env_u32 = |key: &str, default: u32| -> u32 {
            std::env::var(key)
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(default)
        };
        Self {
            firecracker_bin: env_path("CREG_FIRECRACKER_BIN", "/usr/local/bin/firecracker"),
            jailer_bin: env_path("CREG_JAILER_BIN", "/usr/local/bin/jailer"),
            kernel_image_path: env_path("CREG_FC_KERNEL_PATH", "/var/lib/creg/vmlinux.bin"),
            rootfs_base_dir: env_path("CREG_FC_ROOTFS_DIR", "/var/lib/creg/fc-rootfs"),
            ebpf_dir: std::env::var("CREG_FC_EBPF_DIR")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            chroot_base_dir: env_path("CREG_FC_CHROOT_BASE", "/srv/jailer"),
            uid: env_u32("CREG_FC_UID", 99999),
            gid: env_u32("CREG_FC_GID", 99999),
            vcpu_count: 1,
            mem_size_mib: env_u32("CREG_SANDBOX_MEMORY_MB", 512),
            timeout_secs: std::env::var("CREG_SANDBOX_TIMEOUT")
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .filter(|&v| v > 0)
                .unwrap_or(120),
        }
    }
}

/// Probe whether Tier 1 (Firecracker + jailer + KVM + kernel image + rootfs)
/// is installed and ready on this host. Always returns `false` off Linux.
pub async fn detect_firecracker_ready() -> bool {
    #[cfg(target_os = "linux")]
    {
        let cfg = FirecrackerConfig::from_env();
        if !Path::new("/dev/kvm").exists() {
            return false;
        }
        if !cfg.kernel_image_path.is_file() || !cfg.rootfs_base_dir.is_dir() {
            return false;
        }
        let fc_ok = tokio::process::Command::new(&cfg.firecracker_bin)
            .arg("--version")
            .output()
            .await
            .is_ok_and(|o| o.status.success());
        let jailer_ok = tokio::process::Command::new(&cfg.jailer_bin)
            .arg("--version")
            .output()
            .await
            .is_ok_and(|o| o.status.success());
        fc_ok && jailer_ok
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootSourceConfig {
    pub kernel_image_path: String,
    pub boot_args: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveConfig {
    pub drive_id: String,
    pub path_on_host: String,
    pub is_root_device: bool,
    pub is_read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineConfig {
    pub vcpu_count: u8,
    pub mem_size_mib: u32,
    pub smt: bool,
    pub track_dirty_pages: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkInterfaceConfig {
    pub iface_id: String,
    pub guest_mac: String,
    pub host_dev_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VsockDeviceConfig {
    pub guest_cid: u32,
    pub uds_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirecrackerVmConfig {
    #[serde(rename = "boot-source")]
    pub boot_source: BootSourceConfig,
    pub drives: Vec<DriveConfig>,
    #[serde(rename = "machine-config")]
    pub machine_config: MachineConfig,
    #[serde(rename = "network-interfaces")]
    pub network_interfaces: Vec<NetworkInterfaceConfig>,
    pub vsock: VsockDeviceConfig,
}

/// Build the Firecracker JSON machine specification for a single package run.
pub fn build_vm_config(
    cfg: &FirecrackerConfig,
    rootfs_image: &Path,
    pkg_drive_path: &Path,
    scratch_drive_path: &Path,
    tap_iface: &str,
    vsock_uds_path: &Path,
) -> FirecrackerVmConfig {
    FirecrackerVmConfig {
        boot_source: BootSourceConfig {
            kernel_image_path: cfg.kernel_image_path.to_string_lossy().into_owned(),
            boot_args: "console=ttyS0 reboot=k panic=1 pci=off init=/sbin/creg-guest-init quiet"
                .to_string(),
        },
        drives: vec![
            DriveConfig {
                drive_id: "rootfs".to_string(),
                path_on_host: rootfs_image.to_string_lossy().into_owned(),
                is_root_device: true,
                is_read_only: true,
            },
            DriveConfig {
                drive_id: "pkg".to_string(),
                path_on_host: pkg_drive_path.to_string_lossy().into_owned(),
                is_root_device: false,
                is_read_only: true,
            },
            DriveConfig {
                drive_id: "scratch".to_string(),
                path_on_host: scratch_drive_path.to_string_lossy().into_owned(),
                is_root_device: false,
                is_read_only: false,
            },
        ],
        machine_config: MachineConfig {
            vcpu_count: cfg.vcpu_count.max(1),
            mem_size_mib: cfg.mem_size_mib.max(128),
            smt: false,
            track_dirty_pages: false,
        },
        network_interfaces: vec![NetworkInterfaceConfig {
            iface_id: "eth0".to_string(),
            guest_mac: "06:00:AC:10:00:02".to_string(),
            host_dev_name: tap_iface.to_string(),
        }],
        vsock: VsockDeviceConfig {
            guest_cid: DEFAULT_VSOCK_CID,
            uds_path: vsock_uds_path.to_string_lossy().into_owned(),
        },
    }
}

/// Build the hardened `jailer` argument vector wrapping Firecracker in
/// uid/gid/chroot/cgroup-v2/seccomp-level-2 confinement.
pub fn build_jailer_args(
    cfg: &FirecrackerConfig,
    instance_id: &str,
    vm_config_path: &Path,
) -> Vec<String> {
    vec![
        "--id".into(),
        instance_id.to_string(),
        "--exec-file".into(),
        cfg.firecracker_bin.to_string_lossy().into_owned(),
        "--uid".into(),
        cfg.uid.to_string(),
        "--gid".into(),
        cfg.gid.to_string(),
        "--chroot-base-dir".into(),
        cfg.chroot_base_dir.to_string_lossy().into_owned(),
        "--cgroup-version".into(),
        "2".into(),
        "--cgroup".into(),
        "pids.max=64".into(),
        "--seccomp-level".into(),
        "2".into(),
        "--".into(),
        "--no-api".into(),
        "--config-file".into(),
        vm_config_path.to_string_lossy().into_owned(),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VsockTelemetryFrame {
    #[serde(default)]
    pub events: Vec<RawKernelEvent>,
    #[serde(default)]
    pub overlay_added: Vec<String>,
    #[serde(default)]
    pub overlay_modified: Vec<String>,
    #[serde(default)]
    pub guest_exit_code: i32,
}

/// Bounded JSONL parser for `creg-guest-init` CO-RE eBPF events emitted over
/// `virtio-vsock`. Handles either standalone `RawKernelEvent` JSON lines or
/// full `VsockTelemetryFrame` summary lines.
pub fn parse_vsock_ebpf_stream(bytes: &[u8]) -> VsockTelemetryFrame {
    let slice = &bytes[..bytes.len().min(MAX_VSOCK_STREAM_BYTES)];
    let text = String::from_utf8_lossy(slice);
    let mut combined = VsockTelemetryFrame {
        events: Vec::new(),
        overlay_added: Vec::new(),
        overlay_modified: Vec::new(),
        guest_exit_code: 0,
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<RawKernelEvent>(trimmed) {
            if combined.events.len() < MAX_KERNEL_EVENTS {
                combined.events.push(event);
            }
            continue;
        }
        if let Ok(frame) = serde_json::from_str::<VsockTelemetryFrame>(trimmed) {
            for ev in frame.events {
                if combined.events.len() >= MAX_KERNEL_EVENTS {
                    break;
                }
                combined.events.push(ev);
            }
            combined.overlay_added.extend(frame.overlay_added);
            combined.overlay_modified.extend(frame.overlay_modified);
            combined.guest_exit_code = frame.guest_exit_code;
        }
    }

    combined
}

/// Pure-Rust DNS wire-format QNAME parser used by the host sinkhole DNS server
/// (UDP/TCP 53) to log and audit guest DNS lookups without forwarding traffic.
pub fn parse_dns_query_name(packet: &[u8]) -> Option<String> {
    if packet.len() < 12 {
        return None;
    }
    let qdcount = u16::from_be_bytes([packet[4], packet[5]]);
    if qdcount == 0 {
        return None;
    }
    let mut offset = 12usize;
    let mut labels = Vec::new();
    let mut steps = 0usize;
    while offset < packet.len() {
        steps += 1;
        if steps > 128 {
            return None;
        }
        let len = packet[offset] as usize;
        offset += 1;
        if len == 0 {
            break;
        }
        // Reject compression pointers inside QNAME in standard queries.
        if (len & 0xC0) != 0 {
            return None;
        }
        if len > 63 || offset.saturating_add(len) > packet.len() {
            return None;
        }
        let label = std::str::from_utf8(&packet[offset..offset + len]).ok()?;
        labels.push(label.to_ascii_lowercase());
        offset += len;
    }
    if labels.is_empty() {
        None
    } else {
        Some(labels.join("."))
    }
}

/// Pure-Rust TLS ClientHello SNI parser used by the host sinkhole TCP proxy
/// (port 443) to recover plaintext SNI hostnames even when custom DNS is
/// bypassed via hardcoded IPs.
pub fn parse_tls_client_hello_sni(record: &[u8]) -> Option<String> {
    // TLS record header (5 bytes): ContentType=0x16 (Handshake)
    if record.len() < 9 || record[0] != 0x16 {
        return None;
    }
    let rec_len = u16::from_be_bytes([record[3], record[4]]) as usize;
    let payload = record.get(5..5 + rec_len.min(record.len() - 5))?;
    // Handshake header: HandshakeType=0x01 (ClientHello), 24-bit length
    if payload.len() < 4 || payload[0] != 0x01 {
        return None;
    }
    let hs_len =
        ((payload[1] as usize) << 16) | ((payload[2] as usize) << 8) | (payload[3] as usize);
    let body = payload.get(4..4 + hs_len.min(payload.len() - 4))?;
    // ClientHello: version(2) + random(32) = 34 bytes
    let mut pos = 34usize;
    let session_id_len = *body.get(pos)? as usize;
    pos = pos.checked_add(1 + session_id_len)?;
    let cipher_suites_len = u16::from_be_bytes([*body.get(pos)?, *body.get(pos + 1)?]) as usize;
    pos = pos.checked_add(2 + cipher_suites_len)?;
    let comp_methods_len = *body.get(pos)? as usize;
    pos = pos.checked_add(1 + comp_methods_len)?;
    let extensions_len = u16::from_be_bytes([*body.get(pos)?, *body.get(pos + 1)?]) as usize;
    pos = pos.checked_add(2)?;
    let ext_end = pos.checked_add(extensions_len)?.min(body.len());

    while pos + 4 <= ext_end {
        let ext_type = u16::from_be_bytes([body[pos], body[pos + 1]]);
        let ext_len = u16::from_be_bytes([body[pos + 2], body[pos + 3]]) as usize;
        pos += 4;
        if pos.saturating_add(ext_len) > ext_end {
            break;
        }
        if ext_type == 0x0000 && ext_len >= 5 {
            // server_name list
            let ext_data = &body[pos..pos + ext_len];
            let list_len = u16::from_be_bytes([ext_data[0], ext_data[1]]) as usize;
            let mut epos = 2usize;
            let list_end = (2 + list_len).min(ext_data.len());
            while epos + 3 <= list_end {
                let name_type = ext_data[epos];
                let name_len =
                    u16::from_be_bytes([ext_data[epos + 1], ext_data[epos + 2]]) as usize;
                epos += 3;
                if epos.saturating_add(name_len) > list_end {
                    break;
                }
                if name_type == 0x00 {
                    let sni = std::str::from_utf8(&ext_data[epos..epos + name_len]).ok()?;
                    return Some(sni.trim_end_matches('.').to_ascii_lowercase());
                }
                epos += name_len;
            }
        }
        pos += ext_len;
    }
    None
}

/// Inspect sinkhole packets/payloads (DNS, TLS SNI, HTTP/raw egress) to produce
/// canonical network events and detect encoded canary exfiltration.
pub fn inspect_sinkhole_egress(
    canary_profile: &CanaryProfile,
    sinkhole_packets: &[(&str, u16, &[u8])],
) -> (Vec<RawKernelEvent>, Vec<CanaryExfiltration>) {
    let mut events = Vec::new();
    let mut leaks = Vec::new();
    for (dest_ip, port, payload) in sinkhole_packets {
        if *port == 53 {
            if let Some(qname) = parse_dns_query_name(payload) {
                leaks
                    .extend(canary_profile.scan_payload(CanaryChannel::DnsQuery, qname.as_bytes()));
                events.push(RawKernelEvent::Network {
                    protocol: "dns".into(),
                    destination: qname,
                    port: 53,
                    sni: None,
                });
            }
        } else {
            let sni = if *port == 443 {
                parse_tls_client_hello_sni(payload)
            } else {
                None
            };
            if let Some(ref sni_host) = sni {
                leaks.extend(
                    canary_profile.scan_payload(CanaryChannel::TlsSni, sni_host.as_bytes()),
                );
            }
            leaks.extend(canary_profile.scan_payload(CanaryChannel::NetworkPayload, payload));
            events.push(RawKernelEvent::Network {
                protocol: if sni.is_some() { "tls" } else { "tcp" }.into(),
                destination: (*dest_ip).to_string(),
                port: *port,
                sni,
            });
        }
    }
    (events, leaks)
}

pub struct FirecrackerOutcome {
    pub graph: KernelBehaviorGraphV1,
    pub exit_code: i32,
    pub wall_time_ms: u64,
}

/// Execute a package in Tier 1 (Firecracker + jailer + in-guest eBPF CO-RE).
pub async fn run_firecracker_sandbox(
    pkg_id: &common::PackageId,
    tarball_path: &Path,
    manifest: &PackageManifest,
    resolved_ips: &HashSet<String>,
    trigger_spec: &TriggerSpecV1,
    timeout_secs: u64,
) -> Result<FirecrackerOutcome> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            pkg_id,
            tarball_path,
            manifest,
            resolved_ips,
            trigger_spec,
            timeout_secs,
        );
        anyhow::bail!("Tier 1 Firecracker sandbox requires Linux /dev/kvm and jailer");
    }

    #[cfg(target_os = "linux")]
    {
        trigger_spec.validate()?;
        let started = Instant::now();
        let fc_cfg = FirecrackerConfig::from_env();
        let VerifiedRootfs {
            rootfs_dir,
            ecosystem,
            ..
        } = resolve_rootfs_strict(&fc_cfg.rootfs_base_dir, &pkg_id.ecosystem)?;
        if !fc_cfg.kernel_image_path.is_file() {
            anyhow::bail!(
                "Firecracker kernel image missing at {}",
                fc_cfg.kernel_image_path.display()
            );
        }

        let run_id = uuid::Uuid::new_v4().to_string();
        let work_dir = std::env::temp_dir().join(format!("creg-fc-{}", run_id));
        std::fs::create_dir_all(&work_dir)?;

        let result: Result<FirecrackerOutcome> = async {
            let rootfs_img = rootfs_dir.join("rootfs.ext4");
            let effective_rootfs = if rootfs_img.is_file() {
                rootfs_img
            } else {
                rootfs_dir.clone()
            };
            let scratch_img = work_dir.join("scratch.ext4");
            std::fs::File::create(&scratch_img)?.set_len(64 * 1024 * 1024)?;

            let vsock_uds = work_dir.join("vsock.sock");
            let vm_config_path = work_dir.join("vm-config.json");
            let instance_id = format!("creg-{}", &run_id[..8]);
            let tap_iface = format!("creg-tap-{}", &run_id[..6]);

            let vm_cfg = build_vm_config(
                &fc_cfg,
                &effective_rootfs,
                tarball_path,
                &scratch_img,
                &tap_iface,
                &vsock_uds,
            );
            std::fs::write(&vm_config_path, serde_json::to_vec_pretty(&vm_cfg)?)?;

            let canary = CanaryProfile::for_run(run_id.as_bytes());
            let mut jailer_cmd = tokio::process::Command::new(&fc_cfg.jailer_bin);
            jailer_cmd
                .env_clear()
                .env("CREG_PKG_ECOSYSTEM", &ecosystem)
                .env("CREG_TRIGGER_SHA256", trigger_spec.canonical_sha256())
                .args(build_jailer_args(&fc_cfg, &instance_id, &vm_config_path))
                .kill_on_drop(true);
            for (k, v) in canary.env_vars() {
                jailer_cmd.env(k, v);
            }
            for (k, v) in &trigger_spec.env_overrides {
                jailer_cmd.env(k, v);
            }

            let output = tokio::time::timeout(
                Duration::from_secs(timeout_secs.max(1)),
                jailer_cmd.output(),
            )
            .await
            .context("Firecracker jailer VM execution timed out")?
            .context("Failed to execute Firecracker jailer")?;

            let stdout_cap = &output.stdout[..output.stdout.len().min(MAX_STDIO_CAPTURE_BYTES)];
            let stderr_cap = &output.stderr[..output.stderr.len().min(MAX_STDIO_CAPTURE_BYTES)];

            let mut canary_leaks = canary.scan_payload(CanaryChannel::Stdout, stdout_cap);
            canary_leaks.extend(canary.scan_payload(CanaryChannel::Stderr, stderr_cap));

            let telemetry_path = work_dir.join("ebpf-events.jsonl");
            let frame = if telemetry_path.is_file() {
                let raw = std::fs::read(&telemetry_path)?;
                parse_vsock_ebpf_stream(&raw)
            } else {
                parse_vsock_ebpf_stream(stderr_cap)
            };

            let graph = normalize_telemetry(
                ExecutionTier::Tier1Firecracker,
                trigger_spec,
                manifest,
                resolved_ips,
                &frame.events,
                &frame.overlay_added,
                &frame.overlay_modified,
                &canary_leaks,
            );

            Ok(FirecrackerOutcome {
                graph,
                exit_code: output.status.code().unwrap_or(frame.guest_exit_code),
                wall_time_ms: started.elapsed().as_millis() as u64,
            })
        }
        .await;

        let _ = std::fs::remove_dir_all(&work_dir);
        result
    }
}
