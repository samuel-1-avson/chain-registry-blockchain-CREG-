//! Multi-Layer Malware Detection Pipeline
//!
//! Replaces legacy ONNX theater with three deterministic verification layers:
//!
//! 1. **YARA-X raw-byte scanning** — lock-free `Arc<yara_x::Rules>` matching with timeouts.
//! 2. **Pinned OSV / off-chain lookups** — deterministic vulnerability matching.
//! 3. **Content-hash threat intel** — SHA-256 matching against known-bad hashes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, warn};

use crate::yara_scanner::{YaraMatch, YaraScanError};

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
    #[error("Deep scan timed out")]
    Timeout,
}

/// A file flagged as suspicious by deep scanning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspiciousFile {
    /// Path of the file inside the package.
    pub path: String,
    /// Malicious probability assigned to this file (0.0 – 1.0).
    pub probability: f32,
    /// Short human-readable match detail.
    pub snippet: String,
}

/// Result of a multi-layer deep scan over a package tarball.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepScanResult {
    /// Probability that the package is malicious (0.0 – 1.0).
    pub malicious_probability: f32,
    /// Confidence in the prediction (0.0 – 1.0).
    pub confidence: f32,
    /// Classification bucket based on probability thresholds.
    pub classification: ThreatClassification,
    /// Optional per-region scores.
    pub attention_regions: Option<HashMap<String, Vec<f32>>>,
    /// Flagged files from YARA / hash intel.
    pub suspicious_files: Vec<SuspiciousFile>,
    /// Scanner profile/version identifier used for the scan.
    pub model_version: String,
    /// Legacy compatibility flag (`false` for real deterministic scans).
    pub is_mock: bool,
}

/// Classification buckets for deep-scan output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreatClassification {
    Safe,
    Suspicious,
    LikelyMalicious,
    ConfirmedMalicious,
    /// Inference did not complete (timeout, missing rules, etc.).
    /// Validators abstain or fail closed rather than approving.
    Degraded,
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

    /// Whether the validator should abstain from voting due to degraded analysis.
    pub fn should_abstain(&self) -> bool {
        matches!(self, ThreatClassification::Degraded)
    }
}

fn is_yara_manifest_or_hook(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with("/package.json")
        || lower == "package.json"
        || lower.ends_with("/cargo.toml")
        || lower == "cargo.toml"
        || lower.ends_with("/setup.py")
        || lower == "setup.py"
        || lower.ends_with("/pyproject.toml")
        || lower == "pyproject.toml"
        || lower.ends_with("/setup.cfg")
        || lower == "setup.cfg"
        || lower.ends_with("/pom.xml")
        || lower == "pom.xml"
        || lower.ends_with(".gemspec")
        || lower.ends_with(".pth")
        || lower.ends_with("/.npmrc")
        || lower == ".npmrc"
        || lower.ends_with("/.pypirc")
        || lower == ".pypirc"
}

fn has_binary_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(b"\x00asm")
        || bytes.starts_with(b"MZ")
        || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xce])
        || bytes.starts_with(&[0xce, 0xfa, 0xed, 0xfe])
        || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xcf])
        || bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
}

/// Selects all raw-byte scan targets from a bounded [`creg_unpack::UnpackedArchive`]:
/// ecosystem scripts/sources, declared manifests & hooks, non-UTF-8 source/config
/// files (`VNA-09`), and embedded ELF/PE/Mach-O/WASM binaries.
pub fn select_yara_targets<'a>(
    archive: &'a creg_unpack::UnpackedArchive,
    ecosystem: &str,
) -> Vec<(&'a str, &'a [u8])> {
    archive
        .files
        .iter()
        .filter(|file| {
            creg_unpack::is_source_file_for_ecosystem(&file.path, ecosystem)
                || is_yara_manifest_or_hook(&file.path)
                || (!file.is_valid_utf8 && (file.is_declared_source || file.is_declared_config))
                || has_binary_magic(&file.raw_bytes)
        })
        .map(|file| (file.path.as_str(), file.raw_bytes.as_ref()))
        .collect()
}

/// Extracts files via `creg-unpack` and runs lock-free raw-byte YARA scanning.
pub fn scan_tarball_with_yara(
    tarball_bytes: &[u8],
    ecosystem: &str,
) -> Result<Vec<YaraMatch>, MlError> {
    let archive = creg_unpack::unpack_tarball(tarball_bytes)
        .map_err(|e| MlError::ExtractionError(e.to_string()))?;
    let targets = select_yara_targets(&archive, ecosystem);
    crate::yara_scanner::scan_raw_files(&targets).map_err(map_yara_err)
}

fn map_yara_err(err: YaraScanError) -> MlError {
    match err {
        YaraScanError::RulesUnavailable => {
            MlError::ModelNotFound("YARA rule bundle unavailable".to_string())
        }
        YaraScanError::Timeout { .. } => MlError::Timeout,
        YaraScanError::ScanFailed { reason, .. } => MlError::InferenceError(reason),
    }
}

/// Deep-scan orchestrator.
pub struct DeepScanner {
    _model_path: std::path::PathBuf,
    _tokenizer_path: Option<std::path::PathBuf>,
    _max_length: usize,
    package_info: Option<crate::osv_client::PackageInfo>,
    pub ecosystem: String,
}

impl DeepScanner {
    /// Create a new scanner. The ONNX model path is kept only for API compatibility.
    pub fn new<P: AsRef<Path>>(model_path: P) -> Self {
        Self {
            _model_path: model_path.as_ref().to_path_buf(),
            _tokenizer_path: None,
            _max_length: 512,
            package_info: None,
            ecosystem: String::new(),
        }
    }

    /// Inspect scanner profile readiness at startup.
    pub fn validate_at_startup(&self) -> Result<(), MlError> {
        if std::env::var("CREG_FORCE_ONNX").unwrap_or_default() == "true" {
            tracing::warn!(
                "CREG_FORCE_ONNX=true is DEPRECATED and ignored. The legacy ONNX path has been retired."
            );
        }
        let profile = crate::scanner_profile::current();
        if profile.strict_phase_c && !profile.ready {
            return Err(MlError::ModelNotFound(format!(
                "Phase C scanner profile {} is incomplete: {}",
                profile.digest,
                profile.errors.join("; ")
            )));
        }
        tracing::info!(
            scanner_profile_digest = %profile.digest,
            yara_digest = %profile.yara_digest,
            threat_intel_digest = %profile.threat_intel_digest,
            osv_digest = %profile.osv_digest,
            osv_epoch = %profile.osv_epoch,
            scanner_version = %profile.scanner_version,
            "Deterministic package-analysis profile inspected"
        );
        Ok(())
    }

    pub fn with_tokenizer<P: AsRef<Path>>(mut self, path: P) -> Self {
        self._tokenizer_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_package_info(mut self, info: crate::osv_client::PackageInfo) -> Self {
        self.package_info = Some(info);
        self
    }

    pub const SCANNER_VERSION: &str = crate::scanner_profile::SCANNER_VERSION;
    pub const DEGRADED_YARA_MISSING: &str = "degraded-yara-missing";

    /// Return the active scanner version string for consensus votes.
    pub fn model_version(&self) -> String {
        crate::scanner_profile::current().scanner_version
    }

    /// Runs deep scanning directly over a pre-unpacked archive without redundant
    /// decompression or re-hashing (`VNA-11`, `VNA-12`).
    pub fn scan_unpacked(
        &self,
        archive: &creg_unpack::UnpackedArchive,
    ) -> Result<DeepScanResult, MlError> {
        let profile = crate::scanner_profile::current();
        if profile.strict_phase_c && !profile.ready {
            return Ok(DeepScanResult {
                malicious_probability: 0.0,
                confidence: 0.0,
                classification: ThreatClassification::Degraded,
                attention_regions: None,
                suspicious_files: Vec::new(),
                model_version: profile.scanner_version,
                is_mock: false,
            });
        }
        if std::env::var("CREG_FORCE_ONNX").unwrap_or_default() == "true" {
            tracing::warn!(
                "CREG_FORCE_ONNX=true is DEPRECATED and ignored. Running lock-free YARA-X scan."
            );
        }

        let targets = select_yara_targets(archive, &self.ecosystem);
        let yara_matches = match crate::yara_scanner::scan_raw_files(&targets) {
            Ok(matches) => matches,
            Err(YaraScanError::RulesUnavailable) => {
                warn!(
                    "YARA rules unavailable — vote will be degraded-yara-missing and excluded from quorum"
                );
                return Ok(DeepScanResult {
                    malicious_probability: 0.0,
                    confidence: 0.0,
                    classification: ThreatClassification::Degraded,
                    attention_regions: None,
                    suspicious_files: Vec::new(),
                    model_version: Self::DEGRADED_YARA_MISSING.to_string(),
                    is_mock: false,
                });
            }
            Err(YaraScanError::Timeout { .. }) => return Err(MlError::Timeout),
            Err(YaraScanError::ScanFailed { reason, .. }) => {
                return Err(MlError::InferenceError(reason));
            }
        };
        let yara_prob = crate::yara_scanner::matches_to_probability(&yara_matches);

        let osv_prob = if let Some(ref info) = self.package_info {
            let osv_result = crate::osv_client::query(info);
            crate::osv_client::vulns_to_probability(&osv_result)
        } else {
            0.0
        };

        let threat_result = crate::threat_intel::check_unpacked(archive);
        let hash_prob = threat_result.to_probability();

        let combined = yara_prob.max(osv_prob).max(hash_prob);

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
                .then_with(|| a.path.cmp(&b.path))
        });
        suspicious_files.truncate(10);

        let classification = ThreatClassification::from_probability(combined);
        let confidence = if combined > 0.01 {
            (0.5 + (combined - 0.5).abs()).min(1.0)
        } else {
            0.5
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
            model_version: profile.scanner_version,
            is_mock: false,
        })
    }

    /// Run the multi-layer scan on compressed `.tar.gz` bytes via `creg-unpack`.
    pub fn scan(&self, tarball_bytes: &[u8]) -> Result<DeepScanResult, MlError> {
        let archive = creg_unpack::unpack_tarball(tarball_bytes)
            .map_err(|e| MlError::ExtractionError(e.to_string()))?;
        self.scan_unpacked(&archive)
    }
}

impl Default for DeepScanner {
    fn default() -> Self {
        Self::new("models/malware_classifier.onnx")
    }
}

/// Executes deep scanning directly in the caller's thread/blocking task, relying on
/// `creg-unpack` memory bounds and `yara_x::Scanner::set_timeout` rather than
/// spawning detached OS threads (`VNA-11`, `VNA-12`).
pub fn deep_scan(
    tarball_bytes: &[u8],
    package_info: Option<crate::osv_client::PackageInfo>,
    ecosystem: &str,
) -> Result<DeepScanResult, MlError> {
    let mut scanner = DeepScanner::default();
    scanner.package_info = package_info;
    scanner.ecosystem = ecosystem.to_string();
    scanner.scan(tarball_bytes)
}

/// Executes deep scanning over an already-unpacked `creg_unpack::UnpackedArchive`.
pub fn deep_scan_unpacked(
    archive: &creg_unpack::UnpackedArchive,
    package_info: Option<crate::osv_client::PackageInfo>,
    ecosystem: &str,
) -> Result<DeepScanResult, MlError> {
    let mut scanner = DeepScanner::default();
    scanner.package_info = package_info;
    scanner.ecosystem = ecosystem.to_string();
    scanner.scan_unpacked(archive)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gzip_tar(files: &[(&str, &[u8])]) -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, *name, *bytes).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn temp_rules_dir(suffix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "creg-yara-{suffix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp rules dir");
        dir
    }

    struct YaraDirGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        previous: Option<String>,
        dir: std::path::PathBuf,
    }

    impl YaraDirGuard {
        fn set(dir: std::path::PathBuf) -> Self {
            let lock = crate::env_test_lock();
            let previous = std::env::var("CREG_YARA_RULES_DIR").ok();
            std::env::set_var("CREG_YARA_RULES_DIR", dir.as_os_str());
            Self {
                _lock: lock,
                previous,
                dir,
            }
        }
    }

    impl Drop for YaraDirGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("CREG_YARA_RULES_DIR", value),
                None => std::env::remove_var("CREG_YARA_RULES_DIR"),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn test_invalid_tarball_fails_closed() {
        let scanner = DeepScanner::new("/nonexistent/path/model.onnx");
        let err = scanner
            .scan(b"dummy tarball bytes")
            .expect_err("corrupt archives must not return a mock-clean result");
        assert!(matches!(err, MlError::ExtractionError(_)));
    }

    #[test]
    fn test_onnx_fallback_still_fails_closed_on_invalid_tarball() {
        let _lock = crate::env_test_lock();
        std::env::set_var("CREG_FORCE_ONNX", "true");
        let scanner = DeepScanner::new("/nonexistent/path/model.onnx");
        let err = scanner.scan(b"dummy tarball bytes");
        std::env::remove_var("CREG_FORCE_ONNX");
        assert!(matches!(err, Err(MlError::ExtractionError(_))));
    }

    #[test]
    fn test_valid_archive_without_source_files_is_not_mock() {
        let dir = temp_rules_dir("present");
        std::fs::write(
            dir.join("dummy.yar"),
            "rule creg_dummy { condition: false }\n",
        )
        .expect("dummy yara rule");
        let _guard = YaraDirGuard::set(dir);

        let scanner = DeepScanner::new("/nonexistent/path/model.onnx");
        let tarball = gzip_tar(&[("README.md", b"# no source files")]);
        let result = scanner.scan(&tarball).expect("valid gzip tar must scan");
        assert!(!result.is_mock);
        assert_eq!(result.model_version, scanner.model_version());
        assert!(result
            .model_version
            .starts_with(DeepScanner::SCANNER_VERSION));
        assert_ne!(result.classification, ThreatClassification::Degraded);
    }

    #[test]
    fn missing_yara_rules_degrades_the_vote_profile() {
        let dir = temp_rules_dir("missing");
        let _guard = YaraDirGuard::set(dir);

        let scanner = DeepScanner::new("/nonexistent/path/model.onnx");
        let tarball = gzip_tar(&[("src/lib.rs", b"fn main() {}")]);
        let result = scanner.scan(&tarball).expect("valid gzip tar must scan");
        assert_eq!(result.model_version, DeepScanner::DEGRADED_YARA_MISSING);
        assert_eq!(result.classification, ThreatClassification::Degraded);
        assert!(result.classification.should_abstain());
        assert_eq!(scanner.model_version(), DeepScanner::DEGRADED_YARA_MISSING);
    }

    #[test]
    fn non_utf8_source_file_still_matches_yara_rule() {
        let dir = temp_rules_dir("rawbyte");
        std::fs::write(
            dir.join("raw.yar"),
            "rule RawByteHit { meta: threat_level = 5 strings: $a = \"eval(\" $b = \"process.env\" condition: $a and $b }\n",
        )
        .expect("write rule");
        let _guard = YaraDirGuard::set(dir);

        let mut evil_js = b"// \xff\xfe invalid utf8 prefix\n".to_vec();
        evil_js.extend_from_slice(b"eval(process.env.AWS_SECRET_ACCESS_KEY);");
        let tarball = gzip_tar(&[("package/index.js", &evil_js)]);
        let matches = scan_tarball_with_yara(&tarball, "npm").expect("raw byte scan");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].rule_name, "RawByteHit");
    }

    #[test]
    fn test_timeout_error_is_not_mock_ok() {
        assert_eq!(MlError::Timeout.to_string(), "Deep scan timed out");
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
        assert!(!ThreatClassification::Degraded.should_block());
        assert!(ThreatClassification::Degraded.should_abstain());
        assert!(!ThreatClassification::Safe.should_abstain());
    }

    #[test]
    fn test_extract_source_files_rejects_bomb_via_creg_unpack() {
        let zero_bomb = vec![0u8; 6 * 1024 * 1024];
        let tarball = gzip_tar(&[("index.js", &zero_bomb)]);
        let err = scan_tarball_with_yara(&tarball, "npm").expect_err("must reject 6 MiB file");
        assert!(matches!(err, MlError::ExtractionError(_)));
    }
}
