//! Multi-Layer Malware Detection Pipeline
//!
//! Replaces the old custom-ONNX approach with three production-ready layers
//! that require **zero training data**:
//!
//! 1. **YARA-X scanning** — community-maintained malware rules (VirusTotal).
//! 2. **OSV.dev lookups** — Google's open vulnerability database.
//! 3. **Content-hash threat intel** — SHA-256 matching against known-bad hashes.
//!
//! The legacy ONNX path is still available via `CREG_FORCE_ONNX=true` if a
//! real trained model exists, but the default pipeline no longer needs one.

use ndarray::Array2;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, warn};

use crate::tokenizer::CodeTokenizer;

/// Maximum wall-clock time allowed for a single deep-scan inference pass.
const SCAN_TIMEOUT: Duration = Duration::from_secs(30);

/// Errors that can occur during deep scanning.
#[derive(Debug, thiserror::Error)]
pub enum MlError {
    #[error("ONNX inference failed: {0}")]
    InferenceError(String),
    #[error("Tokenizer error: {0}")]
    TokenizerError(String),
    #[error("Tarball extraction failed: {0}")]
    ExtractionError(String),
    #[error("Model not found: {0}")]
    ModelNotFound(String),
}

/// A file flagged as suspicious by the deep-learning model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspiciousFile {
    /// Path of the file inside the package.
    pub path: String,
    /// Malicious probability assigned to this file (0.0 – 1.0).
    pub probability: f32,
    /// Short code snippet (first 200 chars) for reporting.
    pub snippet: String,
}

/// Result of a deep-learning scan over a package tarball.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepScanResult {
    /// Probability that the package is malicious (0.0 – 1.0).
    pub malicious_probability: f32,

    /// Model confidence in the prediction (0.0 – 1.0).
    pub confidence: f32,

    /// Human-readable classification based on probability thresholds.
    pub classification: ThreatClassification,

    /// Optional attention weights mapped to source-file regions.
    /// Keys are file paths; values are per-line suspiciousness scores.
    pub attention_regions: Option<HashMap<String, Vec<f32>>>,

    /// Files flagged as suspicious by the model.
    pub suspicious_files: Vec<SuspiciousFile>,

    /// Model version or artifact identifier used for the scan.
    pub model_version: String,

    /// Whether the result was produced by a real ONNX inference or a
    /// fallback/mock because the model is not present.
    pub is_mock: bool,
}

/// Classification buckets for deep-scan output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreatClassification {
    Safe,
    Suspicious,
    LikelyMalicious,
    ConfirmedMalicious,
}

impl ThreatClassification {
    /// Derive a classification from a probability score.
    pub fn from_probability(prob: f32) -> Self {
        match prob {
            p if p < 0.30 => ThreatClassification::Safe,
            p if p < 0.60 => ThreatClassification::Suspicious,
            p if p < 0.85 => ThreatClassification::LikelyMalicious,
            _ => ThreatClassification::ConfirmedMalicious,
        }
    }

    /// Whether this classification should contribute a blocking finding.
    pub fn should_block(&self) -> bool {
        matches!(self, ThreatClassification::ConfirmedMalicious)
    }
}

/// Deep-scan configuration.
pub struct DeepScanner {
    model_path: std::path::PathBuf,
    tokenizer_path: Option<std::path::PathBuf>,
    max_length: usize,
    /// Optional package info for OSV lookups.
    package_info: Option<crate::osv_client::PackageInfo>,
}

impl DeepScanner {
    /// Create a new scanner pointing at the given ONNX model.
    pub fn new<P: AsRef<Path>>(model_path: P) -> Self {
        Self {
            model_path: model_path.as_ref().to_path_buf(),
            tokenizer_path: None,
            max_length: 512,
            package_info: None,
        }
    }

    /// Check that the configured model file exists and is a valid size.
    /// Call at application startup to fail fast if the model is missing.
    ///
    /// Returns `Ok(())` if the default pipeline (YARA + OSV) will be used
    /// (i.e. `CREG_FORCE_ONNX` is not set).  When ONNX is forced, returns
    /// an error if the model file is missing or suspiciously small.
    pub fn validate_at_startup(&self) -> Result<(), MlError> {
        if std::env::var("CREG_FORCE_ONNX").unwrap_or_default() != "true" {
            // Default pipeline doesn't need the ONNX model — nothing to check.
            tracing::info!(
                "ML deep-scan: ONNX model not required (rule-based pipeline active)"
            );
            return Ok(());
        }

        if !self.model_path.exists() {
            let msg = format!(
                "ONNX model not found at '{}'. Set CREG_FORCE_ONNX=false or provide the model.",
                self.model_path.display()
            );
            tracing::error!("{}", msg);
            return Err(MlError::InferenceError(msg));
        }

        let meta = std::fs::metadata(&self.model_path).map_err(|e| {
            MlError::InferenceError(format!(
                "Cannot stat ONNX model at '{}': {e}",
                self.model_path.display()
            ))
        })?;

        if meta.len() < 1024 {
            let msg = format!(
                "ONNX model at '{}' is only {} bytes — likely a placeholder, not a trained model.",
                self.model_path.display(),
                meta.len()
            );
            tracing::warn!("{}", msg);
            return Err(MlError::InferenceError(msg));
        }

        tracing::info!(
            "ML deep-scan: ONNX model verified at '{}' ({} bytes)",
            self.model_path.display(),
            meta.len()
        );
        Ok(())
    }

    /// Attach a tokenizer JSON path.
    pub fn with_tokenizer<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.tokenizer_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Attach package metadata for OSV vulnerability lookups.
    pub fn with_package_info(mut self, info: crate::osv_client::PackageInfo) -> Self {
        self.package_info = Some(info);
        self
    }

    /// Return the model version string for inclusion in vote messages.
    pub fn model_version(&self) -> String {
        if std::env::var("CREG_FORCE_ONNX").unwrap_or_default() == "true"
            && self.model_path.exists()
        {
            let size = std::fs::metadata(&self.model_path)
                .map(|m| m.len())
                .unwrap_or(0);
            if size >= 1024 {
                return "codebert-v0.1.0".to_string();
            }
        }
        "creg-detect-v1.0.0".to_string()
    }

    /// Run the multi-layer scan (default) or legacy ONNX scan.
    pub fn scan(&self, tarball_bytes: &[u8]) -> Result<DeepScanResult, MlError> {
        // Legacy ONNX path — only used when explicitly forced AND a real model exists.
        if std::env::var("CREG_FORCE_ONNX").unwrap_or_default() == "true" {
            return self.scan_onnx(tarball_bytes);
        }

        // ── Multi-Layer Pipeline ──────────────────────────────────────
        let files = match extract_source_files(tarball_bytes) {
            Ok(f) => f,
            Err(_) => {
                // If tarball extraction fails, return mock rather than error.
                // This makes the pipeline resilient to corrupt/empty tarballs.
                return Ok(mock_result());
            }
        };

        if files.is_empty() {
            return Ok(mock_result());
        }

        // Layer 1: YARA pattern matching.
        let yara_matches = crate::yara_scanner::scan_files(&files);
        let yara_prob = crate::yara_scanner::matches_to_probability(&yara_matches);

        // Layer 2: OSV vulnerability lookup (optional).
        let osv_prob = if let Some(ref info) = self.package_info {
            let osv_result = crate::osv_client::query(info);
            crate::osv_client::vulns_to_probability(&osv_result)
        } else {
            0.0
        };

        // Layer 3: Content-hash threat intelligence.
        let threat_result = crate::threat_intel::check(tarball_bytes, &files);
        let hash_prob = threat_result.to_probability();

        // ── Combine scores: take the max of all three layers ─────────
        // A single confident layer is enough to flag a package.  This
        // avoids the averaging-dilution problem where one critical hit
        // gets watered down by two clean layers.
        let combined = yara_prob.max(osv_prob).max(hash_prob);

        // Build suspicious files list from YARA matches.
        let mut suspicious_files: Vec<SuspiciousFile> = yara_matches
            .iter()
            .map(|m| SuspiciousFile {
                path: m.matched_file.clone(),
                probability: match m.threat_level {
                    5 => 0.95,
                    4 => 0.80,
                    3 => 0.55,
                    2 => 0.35,
                    _ => 0.15,
                },
                snippet: format!("YARA rule '{}': {}", m.rule_name, m.description),
            })
            .collect();

        suspicious_files.sort_by(|a, b| {
            b.probability
                .partial_cmp(&a.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        suspicious_files.truncate(10);

        let classification = ThreatClassification::from_probability(combined);
        let confidence = if combined > 0.01 {
            (0.5 + (combined - 0.5).abs()).min(1.0)
        } else {
            0.5 // Moderate confidence in a clean result.
        };

        debug!(
            "Multi-layer scan: yara={:.4} osv={:.4} hash={:.4} combined={:.4} class={:?}",
            yara_prob, osv_prob, hash_prob, combined, classification
        );

        Ok(DeepScanResult {
            malicious_probability: combined,
            confidence,
            classification,
            attention_regions: None,
            suspicious_files,
            model_version: "creg-detect-v1.0.0".to_string(),
            is_mock: false,
        })
    }

    /// Legacy ONNX-based scan. Only called when `CREG_FORCE_ONNX=true`.
    fn scan_onnx(&self, tarball_bytes: &[u8]) -> Result<DeepScanResult, MlError> {
        if !self.model_path.exists() {
            warn!(
                "ONNX model not found at '{}'; returning degraded deep-scan result",
                self.model_path.display()
            );
            return Ok(mock_result());
        }

        let mut session = create_onnx_session(&self.model_path)?;

        // Extract source files from the tarball.
        let files = extract_source_files(tarball_bytes)
            .map_err(|e| MlError::ExtractionError(e.to_string()))?;

        if files.is_empty() {
            return Ok(mock_result());
        }

        // Load tokenizer — prefer the explicit path, otherwise fall back to a
        // default BPE tokenizer (which will work for shape validation but
        // produce nonsense vocabulary IDs if no real tokenizer.json is present).
        let tokenizer = if let Some(ref path) = self.tokenizer_path {
            CodeTokenizer::from_file(path, self.max_length)
                .map_err(|e| MlError::TokenizerError(e.to_string()))?
        } else {
            CodeTokenizer::new(self.max_length)
                .map_err(|e| MlError::TokenizerError(e.to_string()))?
        };

        let mut suspicious_files = Vec::new();
        let mut max_prob = 0.0f32;

        for (path, content) in &files {
            // Truncate very long files to avoid excessive memory use.
            let snippet = if content.len() > self.max_length * 4 {
                &content[..self.max_length * 4]
            } else {
                content
            };

            let (ids, mask) = tokenizer
                .encode_with_attention(snippet)
                .map_err(|e| MlError::TokenizerError(e.to_string()))?;

            let ids_i64: Vec<i64> = ids.iter().map(|&id| id as i64).collect();
            let mask_i64: Vec<i64> = mask.iter().map(|&m| m as i64).collect();

            let input_ids = Array2::from_shape_vec((1, ids_i64.len()), ids_i64)
                .map_err(|e| MlError::InferenceError(e.to_string()))?;
            let attention_mask = Array2::from_shape_vec((1, mask_i64.len()), mask_i64)
                .map_err(|e| MlError::InferenceError(e.to_string()))?;

            let prob = run_inference(&mut session, input_ids, attention_mask)?;

            if prob > 0.30 {
                suspicious_files.push(SuspiciousFile {
                    path: path.clone(),
                    probability: prob,
                    snippet: snippet.chars().take(200).collect(),
                });
            }

            if prob > max_prob {
                max_prob = prob;
            }
        }

        suspicious_files.sort_by(|a, b| {
            b.probability
                .partial_cmp(&a.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        suspicious_files.truncate(10);

        let classification = ThreatClassification::from_probability(max_prob);
        let confidence = (0.5 + (max_prob - 0.5).abs()).min(1.0);

        debug!(
            "Deep scan complete: prob={:.4}, classification={:?}, flagged_files={}",
            max_prob,
            classification,
            suspicious_files.len()
        );

        Ok(DeepScanResult {
            malicious_probability: max_prob,
            confidence,
            classification,
            attention_regions: None, // TODO: extract from ONNX attention outputs
            suspicious_files,
            model_version: "codebert-v0.1.0".to_string(),
            is_mock: false,
        })
    }
}

impl Default for DeepScanner {
    fn default() -> Self {
        Self::new("models/malware_classifier.onnx")
    }
}

/// Convenience free function that uses the default scanner.
///
/// Called from the validator pipeline after the light-weight `score()`
/// (rule-based) check.  Wraps the scan in a timeout to prevent hung
/// sessions from blocking the validator pipeline.
///
/// `package_info` is optional — when provided, OSV vulnerability lookups
/// are enabled.
pub fn deep_scan(
    tarball_bytes: &[u8],
    package_info: Option<crate::osv_client::PackageInfo>,
) -> Result<DeepScanResult, MlError> {
    let mut scanner = DeepScanner::default();
    scanner.package_info = package_info;

    // If we are inside a tokio runtime, use a timeout.  Otherwise fall
    // back to a synchronous call (e.g. in unit tests).
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let bytes = tarball_bytes.to_vec();
        let model_path = scanner.model_path.clone();
        let tokenizer_path = scanner.tokenizer_path.clone();
        let max_length = scanner.max_length;
        let pkg_info = scanner.package_info.clone();

        match handle.block_on(async move {
            tokio::time::timeout(SCAN_TIMEOUT, tokio::task::spawn_blocking(move || {
                let s = DeepScanner {
                    model_path,
                    tokenizer_path,
                    max_length,
                    package_info: pkg_info,
                };
                s.scan(&bytes)
            }))
            .await
        }) {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => Err(MlError::InferenceError(format!("Scan task panicked: {e}"))),
            Err(_) => {
                warn!("Deep-scan inference timed out after {}s", SCAN_TIMEOUT.as_secs());
                Ok(timeout_result())
            }
        }
    } else {
        scanner.scan(tarball_bytes)
    }
}

/// Produce a degraded result when no model is available or the model is a
/// placeholder. Carries `is_mock = true` so that the validator pipeline
/// emits a visible ML001 warning finding.
fn mock_result() -> DeepScanResult {
    warn!("ML deep-scan running in DEGRADED mode — no trained ONNX model loaded. Security coverage is limited to rule-based analysis only.");
    DeepScanResult {
        malicious_probability: 0.0, // Don't return fake 0.15 — be honest: no data
        confidence: 0.0,            // Zero confidence — no inference was performed
        classification: ThreatClassification::Safe,
        attention_regions: None,
        suspicious_files: Vec::new(),
        model_version: "degraded-no-model".to_string(),
        is_mock: true,
    }
}

/// Produce a degraded result when ONNX inference timed out.
fn timeout_result() -> DeepScanResult {
    warn!("ML deep-scan timed out — inference did not complete within {}s.", SCAN_TIMEOUT.as_secs());
    DeepScanResult {
        malicious_probability: 0.0,
        confidence: 0.0,
        classification: ThreatClassification::Safe,
        attention_regions: None,
        suspicious_files: Vec::new(),
        model_version: "degraded-timeout".to_string(),
        is_mock: true,
    }
}

/// Create an ONNX Runtime session from a file path.
fn create_onnx_session(path: &Path) -> Result<ort::session::Session, MlError> {
    let session = ort::session::Session::builder()
        .map_err(|e| MlError::InferenceError(format!("Failed to create session builder: {e}")))?
        .commit_from_file(path)
        .map_err(|e| {
            MlError::InferenceError(format!(
                "Failed to load ONNX model from {}: {e}",
                path.display()
            ))
        })?;

    Ok(session)
}

/// Softmax over a slice of logits.
fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits.iter().map(|&x| (x - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    exps.iter().map(|&x| x / sum).collect()
}

/// Run a single forward pass through the ONNX session and return the
/// scalar probability of the malicious class.
fn run_inference(
    session: &mut ort::session::Session,
    input_ids: Array2<i64>,
    attention_mask: Array2<i64>,
) -> Result<f32, MlError> {
    let input_ids_tensor = ort::value::Tensor::<i64>::from_array(input_ids)
        .map_err(|e| MlError::InferenceError(format!("Failed to create input_ids tensor: {e}")))?;
    let attention_mask_tensor =
        ort::value::Tensor::<i64>::from_array(attention_mask).map_err(|e| {
            MlError::InferenceError(format!("Failed to create attention_mask tensor: {e}"))
        })?;

    let outputs = session
        .run(ort::inputs![
            "input_ids" => input_ids_tensor,
            "attention_mask" => attention_mask_tensor
        ])
        .map_err(|e| MlError::InferenceError(format!("ONNX inference failed: {e}")))?;

    let output_value = &outputs["logits"];

    let array_view = output_value
        .try_extract_array::<f32>()
        .map_err(|e| MlError::InferenceError(format!("Failed to extract output array: {e}")))?;

    let shape = array_view.shape();
    if shape.len() != 2 || shape[0] != 1 {
        return Err(MlError::InferenceError(format!(
            "Unexpected ONNX output shape: expected [1, N], got {:?}",
            shape
        )));
    }

    let logits: Vec<f32> = array_view.iter().cloned().collect();
    let prob = if logits.len() == 1 {
        // Treat single output as probability directly.
        logits[0]
    } else {
        let probs = softmax(&logits);
        probs.get(1).copied().unwrap_or(0.0)
    };

    Ok(prob)
}

/// Extract text source files from a tar.gz byte slice.
fn extract_source_files(tarball: &[u8]) -> Result<Vec<(String, String)>, std::io::Error> {
    use std::io::Read;
    let gz = flate2::read::GzDecoder::new(tarball);
    let mut archive = tar::Archive::new(gz);
    let mut files = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().to_string();
        let mut content = String::new();
        if entry.read_to_string(&mut content).is_ok()
            && !content.is_empty()
            && is_source_file(&path)
        {
            files.push((path, content));
        }
    }
    Ok(files)
}

/// Check whether a path is a supported source file.
fn is_source_file(path: &str) -> bool {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    matches!(
        ext,
        "js" | "ts" | "mjs" | "cjs" | "py" | "rb" | "rs" | "java"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_result_when_no_source_files() {
        // With the multi-layer pipeline, passing invalid tarball data
        // returns a mock result because extract_source_files yields no files.
        let scanner = DeepScanner::new("/nonexistent/path/model.onnx");
        let result = scanner.scan(b"dummy tarball bytes").unwrap();

        // Invalid tarball → no files extracted → mock result.
        assert!(result.is_mock);
        assert_eq!(result.classification, ThreatClassification::Safe);
        assert_eq!(result.malicious_probability, 0.0);
    }

    #[test]
    fn test_onnx_fallback_mock_when_model_missing() {
        // Legacy ONNX path should mock when model does not exist.
        std::env::set_var("CREG_FORCE_ONNX", "true");
        let scanner = DeepScanner::new("/nonexistent/path/model.onnx");
        let result = scanner.scan(b"dummy tarball bytes").unwrap();
        std::env::remove_var("CREG_FORCE_ONNX");

        assert!(result.is_mock);
        assert_eq!(result.classification, ThreatClassification::Safe);
        assert!(result.model_version.starts_with("degraded"));
    }

    #[test]
    fn test_threat_classification_bounds() {
        assert_eq!(
            ThreatClassification::from_probability(0.0),
            ThreatClassification::Safe
        );
        assert_eq!(
            ThreatClassification::from_probability(0.29),
            ThreatClassification::Safe
        );
        assert_eq!(
            ThreatClassification::from_probability(0.30),
            ThreatClassification::Suspicious
        );
        assert_eq!(
            ThreatClassification::from_probability(0.59),
            ThreatClassification::Suspicious
        );
        assert_eq!(
            ThreatClassification::from_probability(0.60),
            ThreatClassification::LikelyMalicious
        );
        assert_eq!(
            ThreatClassification::from_probability(0.84),
            ThreatClassification::LikelyMalicious
        );
        assert_eq!(
            ThreatClassification::from_probability(0.85),
            ThreatClassification::ConfirmedMalicious
        );
        assert_eq!(
            ThreatClassification::from_probability(1.0),
            ThreatClassification::ConfirmedMalicious
        );
    }

    #[test]
    fn test_confirmed_malicious_blocks() {
        assert!(ThreatClassification::ConfirmedMalicious.should_block());
        assert!(!ThreatClassification::LikelyMalicious.should_block());
        assert!(!ThreatClassification::Suspicious.should_block());
        assert!(!ThreatClassification::Safe.should_block());
    }

    #[test]
    fn test_softmax() {
        let probs = softmax(&[1.0, 2.0, 3.0]);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(probs[2] > probs[1]);
        assert!(probs[1] > probs[0]);
    }

    #[test]
    fn test_is_source_file() {
        assert!(is_source_file("src/index.js"));
        assert!(is_source_file("lib/main.py"));
        assert!(is_source_file("foo.rs"));
        assert!(!is_source_file("README.md"));
        assert!(!is_source_file("package.json"));
    }
}
