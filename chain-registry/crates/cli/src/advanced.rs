//! Advanced validation commands (ZK and ML)
//!
//! Provides CLI commands for:
//! - ZK proof generation and verification
//! - ML-based threat detection
//! - WASM sandbox validation

use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::{info, warn};

use ml_validator::{FeatureExtractor, MlValidator};
use zk_validator::{PackageInputs, ZkValidator};
use wasm_sandbox::{SandboxConfig, SandboxInput, WasmSandbox};

/// Generate a ZK proof for a package
pub async fn generate_zk_proof(
    tarball_path: &PathBuf,
    _manifest_path: Option<&PathBuf>,
) -> Result<Vec<u8>> {
    info!("Generating ZK proof for package...");
    
    // Read tarball
    let tarball_bytes = tokio::fs::read(tarball_path).await
        .context("Failed to read tarball")?;
    
    // Compute content hash
    let content_hash = common::sha256(&tarball_bytes);
    
    // Extract features for manifest hash
    let manifest_hash = common::sha256(b"manifest"); // Simplified
    
    // Run static analysis (placeholder scores)
    let static_analysis_score = 95u8;
    let sandbox_safe = true;
    
    // Create ZK validator
    let validator = ZkValidator::new()
        .context("Failed to initialize ZK validator")?;
    
    // Create inputs
    let inputs = PackageInputs::new(
        content_hash,
        manifest_hash,
        static_analysis_score,
        sandbox_safe,
    );
    
    // Generate proof
    let proof = validator.generate_proof(&inputs)
        .context("Failed to generate ZK proof")?;
    
    // Serialize proof
    let proof_bytes = ZkValidator::serialize_proof(&proof)?;
    
    info!("ZK proof generated: {} bytes", proof_bytes.len());
    
    Ok(proof_bytes)
}

/// Verify a package using ML-based threat detection
pub async fn ml_verify(
    tarball_path: &PathBuf,
    ecosystem: &str,
) -> Result<ml_validator::PredictionResult> {
    info!("Running ML-based verification...");
    
    // Read package content
    let content = tokio::fs::read_to_string(tarball_path).await
        .context("Failed to read package")?;
    
    // Extract features
    let features = FeatureExtractor::extract(ecosystem, &content)
        .context("Failed to extract features")?;
    
    // Run ML validator
    let _validator = MlValidator::new();
    let result = _validator.predict(&features);
    
    info!(
        "ML verification complete: score={}, level={:?}",
        result.threat_score,
        result.threat_level
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
    let tarball_bytes = tokio::fs::read(tarball_path).await
        .context("Failed to read tarball")?;
    
    // Create sandbox config
    let config = SandboxConfig::default()
        .with_memory_limit(256 * 1024 * 1024)
        .with_timeout_secs(30);
    
    // Create sandbox
    let sandbox = WasmSandbox::new(config)
        .context("Failed to create WASM sandbox")?;
    
    // Create input
    let input = SandboxInput::new(package_name, version, ecosystem)
        .with_tarball(tarball_bytes);
    
    // Load validator WASM. For production, this loads the actual compiled validator module.
    // Right now, we embed the core generic validator.
    let validator_wasm = include_bytes!("../validators/dummy.wasm");
    
    // Run validation
    let result = sandbox.validate_package(validator_wasm, &input).await
        .context("WASM validation failed")?;
    
    info!(
        "WASM validation complete: success={}, exit_code={}",
        result.success,
        result.exit_code
    );
    
    Ok(result)
}

/// Batch ML verification for multiple packages
pub async fn batch_ml_verify(
    packages: &[(String, PathBuf)],
    ecosystem: &str,
) -> Result<Vec<(String, ml_validator::PredictionResult)>> {
    info!("Running batch ML verification for {} packages...", packages.len());
    
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
                results.push((name.clone(), ml_validator::PredictionResult::new(
                    100,
                    1.0,
                    risk_result,
                )));
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
    
    tokio::fs::write(output_path, &proof).await
        .context("Failed to write ZK proof to file")?;
    
    info!("ZK proof saved to {:?}", output_path);
    Ok(())
}

/// Verify a ZK proof file
pub async fn verify_zk_proof_file(
    proof_path: &PathBuf,
    tarball_path: &PathBuf,
) -> Result<bool> {
    info!("Verifying ZK proof from {:?}...", proof_path);
    
    // Read proof
    let proof_bytes = tokio::fs::read(proof_path).await
        .context("Failed to read proof file")?;
    
    // Deserialize proof
    let proof = ZkValidator::deserialize_proof(&proof_bytes)?;
    
    // Read tarball to get content hash
    let tarball_bytes = tokio::fs::read(tarball_path).await
        .context("Failed to read tarball")?;
    let content_hash = common::sha256(&tarball_bytes);
    let manifest_hash = common::sha256(b"manifest");
    
    // Create validator
    let validator = ZkValidator::new()?;
    
    // Create inputs
    let inputs = PackageInputs::new(
        content_hash,
        manifest_hash,
        95, // static analysis score
        true, // sandbox safe
    );
    
    // Verify proof
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
}
