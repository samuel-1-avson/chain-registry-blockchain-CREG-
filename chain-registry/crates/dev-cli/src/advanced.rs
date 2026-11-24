//! Advanced local validation commands.
//!
//! Provides CLI commands for:
//! - ML-based threat detection
//! - WASM sandbox validation
//!
//! The local research Groth16 proof commands were removed with the retired
//! `zk-validator` crate (ADR-009, ADR-019 Increment 6 / `ECA-33`).

use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing::{info, warn};

use ml_validator::{FeatureExtractor, MlValidator};
use wasm_sandbox::{SandboxConfig, SandboxInput, WasmSandbox};

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
    let validator = MlValidator::new();
    let result = validator.predict(&features);

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
