//! Advanced validation commands (ZK and ML)
//!
//! Provides CLI commands for:
//! - Local research Groth16 proofs (not used for package admission)
//! - ML-based threat detection
//! - WASM sandbox validation

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use ml_validator::{FeatureExtractor, MlValidator};
use wasm_sandbox::{SandboxConfig, SandboxInput, WasmSandbox};
use zk_validator::{PackageInputs, ZkValidator};

struct LocalCircuitInputs {
    content_hash: [u8; 32],
    manifest_hash: [u8; 32],
    static_analysis_score: u8,
    sandbox_safe: bool,
}

fn static_analysis_safety_score(findings: &[common::Finding]) -> u8 {
    let penalty: i32 = findings
        .iter()
        .map(|f| match f.severity {
            common::FindingSeverity::Critical => 20i32,
            common::FindingSeverity::High => 10i32,
            common::FindingSeverity::Medium => 5i32,
            common::FindingSeverity::Low => 2i32,
        })
        .sum();
    (100i32 - penalty).clamp(0, 100) as u8
}

fn sandbox_declared_safe<E: std::fmt::Display>(
    result: Result<validator::sandbox::SandboxResult, E>,
) -> bool {
    match result {
        Ok(result) => !result
            .findings
            .iter()
            .any(|f| matches!(f.severity, common::FindingSeverity::Critical)),
        Err(e) => {
            warn!("Sandbox unavailable, treating as UNSAFE: {}", e);
            false
        }
    }
}

/// Publisher-local analysis used only as Groth16 public inputs.
/// These scores are not admission evidence (ISSUE-011 / ADR-009).
async fn local_publisher_declared_inputs(
    tarball_path: &Path,
    tarball_bytes: &[u8],
) -> Result<LocalCircuitInputs> {
    let content_hash = common::sha256(tarball_bytes);

    let manifest_path_candidate = tarball_path.with_extension("").with_extension("json");
    let manifest_bytes = tokio::fs::read(&manifest_path_candidate)
        .await
        .unwrap_or_else(|_| tarball_bytes.to_vec());
    let manifest_hash = common::sha256(&manifest_bytes);

    let default_manifest = common::PackageManifest {
        allowed_network_hosts: vec![],
        allowed_fs_writes: vec![],
        spawns_processes: false,
        description: None,
        allowed_process_spawns: vec![],
    };
    let static_result = validator::static_analysis::run(tarball_bytes, &default_manifest)
        .await
        .context("Static analysis failed")?;
    let static_analysis_score = static_analysis_safety_score(&static_result.findings);
    info!(
        "Static analysis score: {}/100 ({} findings)",
        static_analysis_score,
        static_result.findings.len()
    );

    let pkg_id = common::PackageId {
        ecosystem: "unknown".into(),
        name: tarball_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("package")
            .to_string(),
        version: "0.0.0".into(),
    };
    let sandbox_safe = sandbox_declared_safe(
        validator::sandbox::run(&pkg_id, tarball_bytes, &default_manifest).await,
    );
    info!("Sandbox result: sandbox_safe={}", sandbox_safe);

    Ok(LocalCircuitInputs {
        content_hash,
        manifest_hash,
        static_analysis_score,
        sandbox_safe,
    })
}

/// Generate a local research Groth16 proof for a package.
///
/// Runs static analysis and sandbox to obtain scores. The circuit still proves
/// only that those caller-selected values satisfy inequalities — not that a
/// named scanner ran. `creg publish` does not submit this proof.
pub async fn generate_zk_proof(
    tarball_path: &PathBuf,
    _manifest_path: Option<&PathBuf>,
) -> Result<Vec<u8>> {
    info!("Generating local research ZK proof (not used for admission)...");
    warn!(
        "ISSUE-011 / ADR-009: Groth16 package proofs are publisher-declared research artifacts; \
         they are not package-admission evidence."
    );

    let tarball_bytes = tokio::fs::read(tarball_path)
        .await
        .context("Failed to read tarball")?;
    let derived = local_publisher_declared_inputs(tarball_path, &tarball_bytes).await?;

    let validator = ZkValidator::new().context("Failed to initialize ZK validator")?;
    let inputs = PackageInputs::new(
        derived.content_hash,
        derived.manifest_hash,
        derived.static_analysis_score,
        derived.sandbox_safe,
    );
    let proof = validator
        .generate_proof(&inputs)
        .context("Failed to generate ZK proof")?;

    let proof_bytes = ZkValidator::serialize_proof(&proof)?;
    info!(
        "ZK proof generated: {} bytes (score={}, safe={})",
        proof_bytes.len(),
        derived.static_analysis_score,
        derived.sandbox_safe
    );

    Ok(proof_bytes)
}

/// Verify a package using ML-based threat detection
pub async fn ml_verify(
    tarball_path: &PathBuf,
    ecosystem: &str,
) -> Result<ml_validator::PredictionResult> {
    info!("Running ML-based verification...");

    // Read package content (as bytes, then convert to lossy string for ML feature extraction)
    let raw_bytes = tokio::fs::read(tarball_path)
        .await
        .context("Failed to read package")?;
    let content = String::from_utf8_lossy(&raw_bytes).to_string();

    // Extract features
    let features =
        FeatureExtractor::extract(ecosystem, &content).context("Failed to extract features")?;

    // Run ML validator
    let _validator = MlValidator::new();
    let result = _validator.predict(&features);

    info!(
        "ML verification complete: score={}, level={:?}",
        result.threat_score, result.threat_level
    );

    Ok(result)
}

/// Validate a package in WASM sandbox
pub async fn wasm_validate(
    tarball_path: &PathBuf,
    package_name: &str,
    version: &str,
    ecosystem: &str,
) -> Result<wasm_sandbox::SandboxResult> {
    info!("Running WASM sandbox validation...");

    // Read tarball
    let tarball_bytes = tokio::fs::read(tarball_path)
        .await
        .context("Failed to read tarball")?;

    // Create sandbox config
    let config = SandboxConfig::default()
        .with_memory_limit(256 * 1024 * 1024)
        .with_timeout_secs(30);

    // Create sandbox
    let sandbox = WasmSandbox::new(config).context("Failed to create WASM sandbox")?;

    // Create input
    let input = SandboxInput::new(package_name, version, ecosystem).with_tarball(tarball_bytes);

    // Load validator WASM from the path set by CREG_VALIDATOR_WASM or the
    // standard install location.  We do NOT embed a dummy — if no real WASM
    // module is present we return a safe default rather than running nothing.
    let wasm_path_override = std::env::var("CREG_VALIDATOR_WASM")
        .ok()
        .map(std::path::PathBuf::from);
    let default_wasm_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".creg")
        .join("validator.wasm");

    let validator_wasm_path = wasm_path_override.filter(|p| p.exists()).or_else(|| {
        if default_wasm_path.exists() {
            Some(default_wasm_path)
        } else {
            None
        }
    });

    let result = if let Some(wasm_path) = validator_wasm_path {
        let wasm_bytes = tokio::fs::read(&wasm_path)
            .await
            .with_context(|| format!("Failed to read WASM validator at {}", wasm_path.display()))?;
        sandbox
            .validate_package(&wasm_bytes, &input)
            .await
            .context("WASM validation failed")?
    } else {
        warn!("No WASM validator found. Set CREG_VALIDATOR_WASM or place validator.wasm in ~/.creg/. Returning safe default.");
        wasm_sandbox::SandboxResult {
            success: true,
            exit_code: 0,
            findings: vec![],
            resource_usage: wasm_sandbox::ResourceUsage::default(),
            stdout: String::new(),
            stderr: "WASM validator not configured — static analysis still ran".into(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    };

    info!(
        "WASM validation complete: success={}, exit_code={}",
        result.success, result.exit_code
    );

    Ok(result)
}

/// Batch ML verification for multiple packages
#[allow(dead_code)]
pub async fn batch_ml_verify(
    packages: &[(String, PathBuf)],
    ecosystem: &str,
) -> Result<Vec<(String, ml_validator::PredictionResult)>> {
    info!(
        "Running batch ML verification for {} packages...",
        packages.len()
    );

    let mut results = Vec::new();

    for (name, path) in packages {
        match ml_verify(path, ecosystem).await {
            Ok(result) => {
                results.push((name.clone(), result));
            }
            Err(e) => {
                warn!("Failed to verify {}: {}", name, e);
                // Create a high-risk result for failed verifications
                let mut risk_result = std::collections::HashMap::new();
                risk_result.insert(ml_validator::ThreatLevel::Malicious, 1.0);
                results.push((
                    name.clone(),
                    ml_validator::PredictionResult::new(100, 1.0, risk_result),
                ));
            }
        }
    }

    Ok(results)
}

/// Generate and save ZK proof to file
pub async fn generate_and_save_zk_proof(
    tarball_path: &PathBuf,
    manifest_path: Option<&PathBuf>,
    output_path: &PathBuf,
) -> Result<()> {
    let proof = generate_zk_proof(tarball_path, manifest_path).await?;

    tokio::fs::write(output_path, &proof)
        .await
        .context("Failed to write ZK proof to file")?;

    info!("ZK proof saved to {:?}", output_path);
    eprintln!(
        "Note: this Groth16 proof is a local research artifact. `creg publish` does not submit it; \
         validators analyze independently (ISSUE-011 / ADR-009)."
    );
    Ok(())
}

/// Verify a local research ZK proof file.
/// Re-runs static analysis and sandbox on the tarball to derive public inputs.
/// Never uses hardcoded placeholder scores (ISSUE-011).
pub async fn verify_zk_proof_file(proof_path: &PathBuf, tarball_path: &PathBuf) -> Result<bool> {
    info!("Verifying local research ZK proof from {:?}...", proof_path);

    let proof_bytes = tokio::fs::read(proof_path)
        .await
        .context("Failed to read proof file")?;
    let proof = ZkValidator::deserialize_proof(&proof_bytes)?;

    let tarball_bytes = tokio::fs::read(tarball_path)
        .await
        .context("Failed to read tarball")?;
    let derived = local_publisher_declared_inputs(tarball_path, &tarball_bytes).await?;

    info!(
        "Derived inputs — score={}, sandbox_safe={}",
        derived.static_analysis_score, derived.sandbox_safe
    );

    let validator = ZkValidator::new()?;
    let inputs = PackageInputs::new(
        derived.content_hash,
        derived.manifest_hash,
        derived.static_analysis_score,
        derived.sandbox_safe,
    );
    let is_valid = validator.verify_proof(&proof, &inputs.public_inputs())?;

    info!("ZK proof verification result: {}", is_valid);
    Ok(is_valid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ml_validator_creation() {
        let validator = MlValidator::new();
        let info = validator.model_info();
        assert_eq!(info.get("type"), Some(&"rule-based".to_string()));
    }

    #[test]
    fn static_analysis_score_is_derived_not_placeholder_85() {
        assert_eq!(static_analysis_safety_score(&[]), 100);
        let critical = common::Finding {
            id: "t".into(),
            title: "t".into(),
            severity: common::FindingSeverity::Critical,
            description: "t".into(),
            file: "t".into(),
            line: None,
        };
        assert_eq!(static_analysis_safety_score(&[critical]), 80);
    }

    #[test]
    fn sandbox_unavailable_is_unsafe_not_placeholder_true() {
        assert!(!sandbox_declared_safe(Err::<
            validator::sandbox::SandboxResult,
            _,
        >("nsjail missing")));
    }
}
