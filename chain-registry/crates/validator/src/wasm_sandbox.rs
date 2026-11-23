use crate::sandbox::{SandboxConfig, SandboxMetrics, SandboxResult};
use anyhow::{Context, Result};
use common::{Finding, FindingSeverity, PackageId, PackageManifest};
use std::time::Instant;

/// Returns `true` ONLY for genuine standalone WASM packages (`VNA-16`).
///
/// Prevents non-WASM `npm` / `pypi` / `cargo` packages from smuggling a tiny
/// dummy `.wasm` file to bypass Tier 1/Tier 2 fail-closed abstention and run in
/// the Tier 3 WASI-only fallback.
pub fn is_standalone_wasm_package(pkg_id: &PackageId, tarball_bytes: &[u8]) -> bool {
    let Ok(unpacked) = creg_unpack::unpack_tarball(tarball_bytes) else {
        return false;
    };
    if unpacked.files.is_empty() {
        return false;
    }

    let mut has_valid_wasm = false;
    let is_wasm_ecosystem = pkg_id.ecosystem.trim().eq_ignore_ascii_case("wasm");

    for file in &unpacked.files {
        let lower_path = file.path.to_ascii_lowercase();
        let bytes = file.raw_bytes.as_ref();

        if lower_path.ends_with(".wasm") {
            if !bytes.starts_with(b"\x00asm") {
                return false;
            }
            has_valid_wasm = true;
            continue;
        }

        // Disallow any native binary headers disguised without extension
        if bytes.starts_with(b"\x7fELF")
            || bytes.starts_with(b"MZ")
            || matches!(
                bytes.get(0..4),
                Some([0xfe, 0xed, 0xfa, 0xce])
                    | Some([0xfe, 0xed, 0xfa, 0xcf])
                    | Some([0xcf, 0xfa, 0xed, 0xfe])
                    | Some([0xca, 0xfe, 0xba, 0xbe])
            )
        {
            return false;
        }

        // Disallow script, compiled, or build-hook payloads in ANY ecosystem
        let file_name = lower_path.rsplit('/').next().unwrap_or(&lower_path);
        const DISALLOWED_EXTS: &[&str] = &[
            ".js", ".mjs", ".cjs", ".ts", ".mts", ".cts", ".jsx", ".tsx", ".py", ".pyc", ".pyo",
            ".pyw", ".rb", ".rs", ".sh", ".bash", ".zsh", ".ps1", ".bat", ".cmd", ".c", ".cc",
            ".cpp", ".h", ".go", ".java", ".class", ".jar", ".so", ".dll", ".dylib", ".node",
        ];
        if DISALLOWED_EXTS.iter().any(|ext| lower_path.ends_with(ext)) {
            return false;
        }
        const DISALLOWED_NAMES: &[&str] = &[
            "setup.py",
            "setup.cfg",
            "pyproject.toml",
            "build.rs",
            "rakefile",
            "gemfile",
            "makefile",
            "cmakelists.txt",
            "install.sh",
            "preinstall.sh",
            "postinstall.sh",
        ];
        if DISALLOWED_NAMES.contains(&file_name) {
            return false;
        }

        if file_name == "package.json" {
            let Ok(val) = serde_json::from_slice::<serde_json::Value>(bytes) else {
                return false;
            };
            if let Some(scripts) = val.get("scripts").and_then(|s| s.as_object()) {
                if !scripts.is_empty() {
                    return false;
                }
            }
            for field in ["main", "module", "bin", "browser"] {
                if let Some(target) = val.get(field).and_then(|v| v.as_str()) {
                    if !target.to_ascii_lowercase().ends_with(".wasm") {
                        return false;
                    }
                }
            }
            continue;
        }

        if is_wasm_ecosystem {
            continue;
        }

        // Non-wasm ecosystem (e.g. npm `-wasm` wrapper): only passive docs/metadata permitted.
        let is_passive_meta = lower_path.ends_with(".md")
            || lower_path.ends_with(".txt")
            || lower_path.ends_with(".rst")
            || lower_path.ends_with(".wit")
            || lower_path.ends_with(".d.ts")
            || lower_path.ends_with(".map")
            || file_name.starts_with("license")
            || file_name.starts_with("copying")
            || file_name.starts_with("notice")
            || file_name == "readme"
            || file_name == "changelog";
        if !is_passive_meta {
            return false;
        }
    }

    has_valid_wasm
}

fn extract_wasm_files(tarball_path: &std::path::Path) -> Result<Vec<(String, Vec<u8>)>> {
    let tarball_bytes = std::fs::read(tarball_path)
        .with_context(|| format!("Failed to read tarball at {:?}", tarball_path))?;

    let unpacked = creg_unpack::unpack_tarball(&tarball_bytes)
        .map_err(|e| anyhow::anyhow!("Bounded archive extraction failed: {e}"))?;

    let wasm_files = unpacked
        .files
        .into_iter()
        .filter(|file| {
            file.path.to_ascii_lowercase().ends_with(".wasm")
                || file.raw_bytes.starts_with(b"\x00asm")
        })
        .map(|file| (file.path, file.raw_bytes.to_vec()))
        .collect();
    Ok(wasm_files)
}

/// Cross-platform fallback sandbox using wasmtime.
/// Executes package payload within a WebAssembly sandbox.
pub async fn run_in_wasm(
    pkg_id: &common::PackageId,
    tarball_path: &std::path::Path,
    config: &SandboxConfig,
    _manifest: &PackageManifest,
) -> Result<SandboxResult> {
    tracing::info!(
        "[WASM] Initializing WASM sandbox execution for {}...",
        pkg_id
    );
    let start_time = Instant::now();

    let mut findings = Vec::new();

    // ── Extract WASM payloads from tarball ──
    let wasm_files = match extract_wasm_files(tarball_path) {
        Ok(files) => files,
        Err(e) => {
            tracing::error!("[WASM] Tarball extraction failed: {}", e);
            findings.push(Finding {
                id: "SB006".into(),
                title: "WASM Extraction Failure".into(),
                severity: FindingSeverity::Critical,
                description: format!("Failed to extract WASM payload from tarball: {}", e),
                file: "wasm_sandbox".into(),
                line: None,
            });
            return Ok(SandboxResult {
                findings,
                observed_network_hosts: vec![],
                observed_fs_writes: vec![],
                observed_process_spawns: vec![],
                kernel_graph: None,
                metrics: SandboxMetrics {
                    engine_used: "wasm".into(),
                    wall_time_ms: start_time.elapsed().as_millis() as u64,
                    exit_code: 1,
                    observations_count: 0,
                    findings_count: 1,
                },
            });
        }
    };

    if wasm_files.is_empty() {
        tracing::warn!("[WASM] Package does not contain any .wasm files.");
        findings.push(Finding {
            id: "SB006".into(),
            title: "Missing WASM Content".into(),
            severity: FindingSeverity::Critical,
            description:
                "Package was flagged as a WASM candidate but contains no valid .wasm modules."
                    .into(),
            file: "wasm_sandbox".into(),
            line: None,
        });
        return Ok(SandboxResult {
            findings,
            observed_network_hosts: vec![],
            observed_fs_writes: vec![],
            observed_process_spawns: vec![],
            kernel_graph: None,
            metrics: SandboxMetrics {
                engine_used: "wasm".into(),
                wall_time_ms: start_time.elapsed().as_millis() as u64,
                exit_code: 1,
                observations_count: 0,
                findings_count: 1,
            },
        });
    }

    // ── Configure and Instantiate WasmSandbox ──
    let memory_limit = (config.memory_mb as usize) * 1024 * 1024;
    let mut limits = wasm_sandbox_crate::ResourceLimits::strict();
    limits.max_memory = memory_limit;
    limits.max_time_ms = config.timeout_secs.saturating_mul(1000);
    // Allow generous per-second instruction budget so epoch interruption cleanly
    // enforces wall-clock timeout on infinite loops while still capping CPU.
    limits.max_fuel = config
        .timeout_secs
        .saturating_mul(25_000_000_000)
        .max(500_000_000);
    let wasm_config = wasm_sandbox_crate::SandboxConfig::default()
        .with_resource_limits(limits)
        .with_capabilities(wasm_sandbox_crate::CapabilitySet::validator());

    let sandbox = match wasm_sandbox_crate::WasmSandbox::new(wasm_config) {
        Ok(sb) => sb,
        Err(e) => {
            tracing::error!("[WASM] Failed to create WASM sandbox: {}", e);
            findings.push(Finding {
                id: "SB006".into(),
                title: "WASM Sandbox Initialization Error".into(),
                severity: FindingSeverity::Critical,
                description: format!("Failed to create WASM sandbox: {}", e),
                file: "wasm_sandbox".into(),
                line: None,
            });
            return Ok(SandboxResult {
                findings,
                observed_network_hosts: vec![],
                observed_fs_writes: vec![],
                observed_process_spawns: vec![],
                kernel_graph: None,
                metrics: SandboxMetrics {
                    engine_used: "wasm".into(),
                    wall_time_ms: start_time.elapsed().as_millis() as u64,
                    exit_code: 1,
                    observations_count: 0,
                    findings_count: 1,
                },
            });
        }
    };

    let sandbox_input =
        wasm_sandbox_crate::SandboxInput::new(&pkg_id.name, &pkg_id.version, &pkg_id.ecosystem);

    let mut execution_failed = false;

    for (path, wasm_bytes) in wasm_files {
        tracing::info!("[WASM] Executing WASM module: {} ...", path);
        match sandbox.run(&wasm_bytes, &sandbox_input).await {
            Ok(res) => {
                if res.stdout_truncated || res.stderr_truncated {
                    findings.push(Finding {
                        id: "SB015".into(),
                        title: "WASM sandbox stdio output truncated".into(),
                        severity: FindingSeverity::Medium,
                        description: format!(
                            "WASM module {} exceeded the {} byte stdio cap and was truncated.",
                            path,
                            wasm_sandbox_crate::MAX_WASM_STDIO_BYTES
                        ),
                        file: path.clone(),
                        line: None,
                    });
                }
                for f in res.findings {
                    findings.push(Finding {
                        id: f.category.clone(),
                        title: f.description.clone(),
                        severity: match f.severity {
                            wasm_sandbox_crate::Severity::Info => FindingSeverity::Low,
                            wasm_sandbox_crate::Severity::Low => FindingSeverity::Low,
                            wasm_sandbox_crate::Severity::Medium => FindingSeverity::Medium,
                            wasm_sandbox_crate::Severity::High => FindingSeverity::High,
                            wasm_sandbox_crate::Severity::Critical => FindingSeverity::Critical,
                        },
                        description: f.description,
                        file: path.clone(),
                        line: None,
                    });
                }
                if res.exit_code != 0 {
                    findings.push(Finding {
                        id: "SB006".into(),
                        title: "WASM execution failed".into(),
                        severity: FindingSeverity::Critical,
                        description: format!(
                            "WASM module {} exited with non-zero code {}.",
                            path, res.exit_code
                        ),
                        file: path,
                        line: None,
                    });
                    execution_failed = true;
                    break;
                }
            }
            Err(e) => {
                tracing::warn!("[WASM] Sandbox execution error for {}: {}", path, e);
                let description = match e {
                    wasm_sandbox_crate::SandboxError::Timeout(d) => {
                        format!("WASM module {} execution timed out after {:?}", path, d)
                    }
                    wasm_sandbox_crate::SandboxError::ResourceLimitExceeded(msg) => {
                        format!("WASM module {} exceeded resource limit: {}", path, msg)
                    }
                    wasm_sandbox_crate::SandboxError::CompilationError(msg) => {
                        format!("WASM module {} failed to compile: {}", path, msg)
                    }
                    other => {
                        format!("WASM module {} execution failed: {}", path, other)
                    }
                };
                findings.push(Finding {
                    id: "SB006".into(),
                    title: "WASM sandbox execution failed".into(),
                    severity: FindingSeverity::Critical,
                    description,
                    file: path,
                    line: None,
                });
                execution_failed = true;
                break;
            }
        }
    }

    if !execution_failed {
        findings.push(Finding {
            id: "SB005".into(),
            title: "WASM Execution Succeeded".into(),
            severity: FindingSeverity::Low,
            description:
                "Package was securely executed within WebAssembly strict architecture boundaries."
                    .into(),
            file: "wasm_sandbox".into(),
            line: None,
        });
    }

    let findings_count = findings.len();

    Ok(SandboxResult {
        findings,
        observed_network_hosts: vec![],
        observed_fs_writes: vec![],
        observed_process_spawns: vec![],
        kernel_graph: None,
        metrics: SandboxMetrics {
            engine_used: "wasm".into(),
            wall_time_ms: start_time.elapsed().as_millis() as u64,
            exit_code: if execution_failed { 1 } else { 0 },
            observations_count: 0,
            findings_count,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::NetworkMode;
    use common::{PackageId, PackageManifest};
    use flate2::write::GzEncoder;
    use flate2::Compression;

    use tar::Builder;
    use tempfile::NamedTempFile;

    fn create_test_tarball(wasm_filename: &str, wasm_bytes: &[u8]) -> NamedTempFile {
        let temp_file = NamedTempFile::new().unwrap();
        {
            let gz = GzEncoder::new(temp_file.as_file(), Compression::default());
            let mut tar = Builder::new(gz);

            let mut header = tar::Header::new_gnu();
            header.set_size(wasm_bytes.len() as u64);
            header.set_mode(0o755);
            tar.append_data(&mut header, wasm_filename, wasm_bytes)
                .unwrap();
            tar.finish().unwrap();
        }
        temp_file
    }

    #[tokio::test]
    async fn test_wasm_sandbox_success() {
        // Hand-crafted bytecode omitted memory/import sections and traps on wasmtime 17+.
        let wasm_bytes = wat::parse_str(
            r#"(module
                (func (export "main") (result i32)
                  i32.const 0)
            )"#,
        )
        .expect("valid WAT");

        let temp_tarball = create_test_tarball("payload.wasm", &wasm_bytes);
        let pkg_id = PackageId {
            name: "test-package-wasm".into(),
            version: "1.0.0".into(),
            ecosystem: "npm".into(),
        };

        let config = SandboxConfig {
            timeout_secs: 2,
            memory_mb: 64,
            network_mode: NetworkMode::Isolated,
            nsjail_config_path: None,
            rootfs_base_dir: None,
            tarball_hash: None,
            ..SandboxConfig::default()
        };
        let manifest = PackageManifest::default();

        let result = run_in_wasm(&pkg_id, temp_tarball.path(), &config, &manifest)
            .await
            .unwrap();

        assert_eq!(result.metrics.engine_used, "wasm");
        assert_eq!(result.metrics.exit_code, 0);

        // Should contain SB005 indicating success
        let has_success = result.findings.iter().any(|f| f.id == "SB005");
        assert!(
            has_success,
            "Expected SB005 success finding, got {:?}",
            result.findings
        );
    }

    #[tokio::test]
    async fn test_wasm_sandbox_timeout_disruption() {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            test_wasm_sandbox_timeout_disruption_inner(),
        )
        .await;
        match result {
            Ok(inner) => inner,
            Err(_) => panic!("WASM timeout test exceeded 8s — sandbox may be hung"),
        }
    }

    async fn test_wasm_sandbox_timeout_disruption_inner() {
        // WASM bytecode that executes an infinite loop: loop { br 0 }
        // Signature must be () -> i32 to match WasmSandbox::run()'s get_typed_func call.
        // The i32.const 0 after the loop is unreachable but satisfies the type checker.
        let loop_wasm_bytes = &[
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // Magic & Version
            0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f, // Type: () -> i32
            0x03, 0x02, 0x01, 0x00, // Function index 0 uses type index 0
            0x07, 0x08, 0x01, 0x04, 0x6d, 0x61, 0x69, 0x6e, 0x00,
            0x00, // Export "main" as function index 0
            0x0a, 0x0b, 0x01, 0x09, 0x00, 0x03, 0x40, 0x0c, 0x00, 0x0b, 0x41, 0x00,
            0x0b, // Code: loop { br 0 }; i32.const 0; end
        ];

        let temp_tarball = create_test_tarball("payload.wasm", loop_wasm_bytes);
        let pkg_id = PackageId {
            name: "test-loop-wasm".into(),
            version: "1.0.0".into(),
            ecosystem: "npm".into(),
        };

        // Use a very short 1 second timeout to ensure it triggers quickly
        let config = SandboxConfig {
            timeout_secs: 1,
            memory_mb: 64,
            network_mode: NetworkMode::Isolated,
            nsjail_config_path: None,
            rootfs_base_dir: None,
            tarball_hash: None,
            ..SandboxConfig::default()
        };
        let manifest = PackageManifest::default();

        let result = run_in_wasm(&pkg_id, temp_tarball.path(), &config, &manifest)
            .await
            .unwrap();

        assert_eq!(result.metrics.exit_code, 1);

        // Should contain SB006 indicating execution failure/timeout
        let failure_finding = result.findings.iter().find(|f| f.id == "SB006");
        assert!(failure_finding.is_some(), "Expected SB006 failure finding");
        let desc = &failure_finding.unwrap().description;
        assert!(
            desc.contains("timed out"),
            "Expected description to mention timeout, got: {}",
            desc
        );
    }

    #[tokio::test]
    async fn test_wasm_sandbox_no_wasm_files() {
        let temp_tarball = create_test_tarball("not_wasm.txt", b"some text content");
        let pkg_id = PackageId {
            name: "test-package-wasm".into(),
            version: "1.0.0".into(),
            ecosystem: "npm".into(),
        };

        let config = SandboxConfig {
            timeout_secs: 5,
            memory_mb: 64,
            network_mode: NetworkMode::Isolated,
            nsjail_config_path: None,
            rootfs_base_dir: None,
            tarball_hash: None,
            ..SandboxConfig::default()
        };
        let manifest = PackageManifest::default();

        let result = run_in_wasm(&pkg_id, temp_tarball.path(), &config, &manifest)
            .await
            .unwrap();

        assert_eq!(result.metrics.exit_code, 1);
        let failure_finding = result.findings.iter().find(|f| f.id == "SB006");
        assert!(failure_finding.is_some());
        assert!(failure_finding
            .unwrap()
            .description
            .contains("no valid .wasm modules"));
    }
}
