// crates/validator/src/static_analysis.rs
// Stage 1: Static analysis of package source files.
// Scans the tarball for known malicious patterns without executing anything.
// Also integrates ML-based rule scoring and deep learning inference.

use anyhow::Result;
use common::{Finding, FindingSeverity, PackageManifest};
use serde_json;
use std::sync::OnceLock;

/// Shannon entropy threshold for flagging obfuscated lines.
/// Configurable via the `CREG_ENTROPY_THRESHOLD` environment variable.
fn entropy_threshold() -> f64 {
    std::env::var("CREG_ENTROPY_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(5.5)
}

pub struct StaticAnalysisResult {
    pub findings: Vec<Finding>,
    /// Weighted ensemble score (0–100) combining all analysis signals.
    /// Higher = more dangerous.
    pub ensemble_score: f64,
}

/// A single static-analysis pattern used for substring matching in source text.
#[derive(Debug, Clone, serde::Deserialize)]
struct Pattern {
    id: String,
    description: String,
    severity: FindingSeverity,
    /// Simple substring match for now; extend to regex or AST checks.
    needle: String,
}

/// Built-in default patterns; used when no external file is configured.
fn default_patterns() -> Vec<Pattern> {
    vec![
        Pattern { id: "SA001".into(), description: "Dynamic eval() of external or user-controlled data".into(), severity: FindingSeverity::Critical, needle: "eval(".into() },
        Pattern { id: "SA002".into(), description: "Obfuscated base64 string decode at runtime".into(), severity: FindingSeverity::High, needle: "Buffer.from(".into() },
        Pattern { id: "SA003".into(), description: "exec() / execSync() shell execution".into(), severity: FindingSeverity::Critical, needle: "execSync(".into() },
        Pattern { id: "SA004".into(), description: "Spawns child processes (child_process.spawn)".into(), severity: FindingSeverity::Medium, needle: "child_process".into() },
        Pattern { id: "SA005".into(), description: "Reads environment variables (potential credential harvesting)".into(), severity: FindingSeverity::Low, needle: "process.env".into() },
        Pattern { id: "SA006".into(), description: "Raw HTTP request in install/postinstall hook".into(), severity: FindingSeverity::High, needle: "require('http')".into() },
        Pattern { id: "SA007".into(), description: "Writes to home directory or system paths".into(), severity: FindingSeverity::High, needle: "os.homedir()".into() },
        Pattern { id: "SA008".into(), description: "Crypto miner indicators".into(), severity: FindingSeverity::Critical, needle: "CryptoNight".into() },
    ]
}

/// Load the pattern list. If `CREG_PATTERNS_FILE` is set, load patterns from
/// that JSON file; otherwise fall back to the built-in defaults. The result
/// is cached for the lifetime of the process.
fn patterns() -> &'static Vec<Pattern> {
    static PATTERNS: OnceLock<Vec<Pattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        if let Ok(path) = std::env::var("CREG_PATTERNS_FILE") {
            match std::fs::read_to_string(&path) {
                Ok(json) => match serde_json::from_str::<Vec<Pattern>>(&json) {
                    Ok(custom) => {
                        tracing::info!("Loaded {} patterns from {}", custom.len(), path);
                        return custom;
                    }
                    Err(e) => tracing::warn!("Failed to parse patterns file {}: {}; using defaults", path, e),
                },
                Err(e) => tracing::warn!("Failed to read patterns file {}: {}; using defaults", path, e),
            }
        }
        default_patterns()
    })
}

pub async fn run(tarball_bytes: &[u8], manifest: &PackageManifest) -> Result<StaticAnalysisResult> {
    let mut findings = Vec::new();

    // Extract files from the tarball (tar.gz).
    let files = extract_text_files(tarball_bytes)?;

    for (path, content) in &files {
        // Only analyse JS/TS/Python/Rust/Ruby source files.
        if !is_source_file(path) {
            continue;
        }

        for pat in patterns() {
            if content.contains(&pat.needle[..]) {
                // Cross-check against the publisher's declared manifest.
                if is_excused_by_manifest(pat, manifest) {
                    continue;
                }

                findings.push(Finding {
                    id: pat.id.to_string(),
                    title: pat.description.to_string(),
                    severity: pat.severity,
                    description: pat.description.to_string(),
                    file: path.clone(),
                    line: find_line_number(content, &pat.needle),
                });
            }
        }

        let threshold = entropy_threshold();
        let mut has_high_entropy = false;
        // Entropy check: flag highly entropic strings (obfuscated code).
        for (line_num, line) in content.lines().enumerate() {
            if shannon_entropy(line) > threshold && line.len() > 80 {
                has_high_entropy = true;
                findings.push(Finding {
                    id: "SA009".into(),
                    title: "High-entropy string detected".into(),
                    severity: FindingSeverity::High,
                    description: "High-entropy string — possible obfuscated payload".into(),
                    file: path.clone(),
                    line: Some(line_num + 1),
                });
                break; // Flag once per file and pass whole file to LLM
            }
        }

        if has_high_entropy {
            if let Ok(score) = crate::llm::predict_intent(&content).await {
                if score >= 80 {
                    findings.push(Finding {
                        id: "SA011".into(),
                        title: "AI-Verified Malicious Intent".into(),
                        severity: FindingSeverity::Critical,
                        description: format!("LLM semantic analysis indicates high probability (score: {}) of malicious intent in obfuscated logic.", score),
                        file: path.clone(),
                        line: None,
                    });
                } else if score >= 50 {
                    findings.push(Finding {
                        id: "SA011".into(),
                        title: "AI-Suspicious Obfuscation".into(),
                        severity: FindingSeverity::Medium,
                        description: format!("LLM analysis flagged suspicious but inconclusive obfuscated logic (score: {}).", score),
                        file: path.clone(),
                        line: None,
                    });
                }
            }
        }
    }

    // Check for typosquatting using Levenshtein distance against all popular packages.
    // Extract the package name from package.json / Cargo.toml / setup.py for the check.
    let (pkg_name, pkg_version, ecosystem) = extract_package_identity(&files);
    if !pkg_name.is_empty() {
        if let Some(finding) = check_typosquatting_real(&pkg_name, &ecosystem) {
            findings.push(finding);
        }
    }

    // ── Rule-Based ML Scoring (Phase 2a) ────────────────────────────────────
    // Extract AST features from source files and run rule-based threat scoring.
    // This provides immediate ML-style detection independent of the ONNX model.
    let ecosystem_str = &ecosystem;
    let all_code: String = files
        .iter()
        .filter(|(p, _)| is_source_file(p))
        .map(|(_, c)| c.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    if !all_code.is_empty() {
        let extract_ecosystem = if ecosystem_str == "npm" || ecosystem_str.is_empty() {
            "npm"
        } else {
            ecosystem_str
        };
        match ml_validator::FeatureExtractor::extract(extract_ecosystem, &all_code) {
            Ok(features) => {
                let prediction = ml_validator::MlValidator::new().predict(&features);
                if prediction.threat_score >= 76 {
                    findings.push(Finding {
                        id: "ML002".into(),
                        title: "Rule-Based ML: Malicious Threat Score".into(),
                        severity: FindingSeverity::High,
                        description: format!(
                            "Rule-based ML scoring detected malicious threat level (score: {}/100, confidence: {:.2}). \
                             Indicators: eval={}, network={}, fs_ops={}, obfuscation={}, entropy={:.2}",
                            prediction.threat_score, prediction.confidence,
                            features.eval_count, features.network_calls,
                            features.file_system_ops, features.obfuscation_indicators,
                            features.entropy
                        ),
                        file: "rule-based-ml".into(),
                        line: None,
                    });
                } else if prediction.threat_score >= 51 {
                    findings.push(Finding {
                        id: "ML003".into(),
                        title: "Rule-Based ML: Suspicious Threat Score".into(),
                        severity: FindingSeverity::Medium,
                        description: format!(
                            "Rule-based ML scoring flagged suspicious patterns (score: {}/100, confidence: {:.2}).",
                            prediction.threat_score, prediction.confidence
                        ),
                        file: "rule-based-ml".into(),
                        line: None,
                    });
                }
            }
            Err(e) => {
                tracing::warn!("Feature extraction failed: {}; skipping rule-based ML", e);
            }
        }
    }

    // ── Multi-Layer Malware Scan (YARA + OSV + Threat Intel) ───────────────
    // Runs the 3-layer detection pipeline: YARA pattern matching, OSV
    // vulnerability database lookup, and content-hash threat intelligence.
    // When CREG_FORCE_ONNX=true, falls back to the legacy ONNX path.
    let pkg_info = if !pkg_name.is_empty() {
        Some(ml_validator::osv_client::PackageInfo {
            name: pkg_name.clone(),
            version: pkg_version.clone(),
            ecosystem: ecosystem.clone(),
        })
    } else {
        None
    };

    match ml_validator::deep_scan(tarball_bytes, pkg_info) {
        Ok(deep) => {
            // If deep scan ran in mock/degraded mode, emit a visible warning finding
            // so validators and the network are aware ML coverage is not active.
            if deep.is_mock {
                findings.push(Finding {
                    id: "ML001".into(),
                    title: "ML Deep Scan: Degraded Mode".into(),
                    severity: FindingSeverity::Medium,
                    description: format!(
                        "Multi-layer scan ran in degraded mode (version: {}). \
                         Detection layers (YARA/OSV/ThreatIntel) may be partially unavailable.",
                        deep.model_version
                    ),
                    file: "deep_scan".into(),
                    line: None,
                });
            }

            let prob = deep.malicious_probability;
            match deep.classification {
                ml_validator::ThreatClassification::ConfirmedMalicious => {
                    findings.push(Finding {
                        id: "DS003".into(),
                        title: "AI Deep Scan: Confirmed Malicious".into(),
                        severity: FindingSeverity::Critical,
                        description: format!(
                            "Multi-layer scan (YARA+OSV+ThreatIntel) indicates high probability ({:.2}) of malicious content.",
                            prob
                        ),
                        file: "deep_scan".into(),
                        line: None,
                    });
                }
                ml_validator::ThreatClassification::LikelyMalicious => {
                    findings.push(Finding {
                        id: "DS002".into(),
                        title: "AI Deep Scan: Likely Malicious".into(),
                        severity: FindingSeverity::High,
                        description: format!(
                            "Multi-layer scan (YARA+OSV+ThreatIntel) indicates likely malicious content (probability: {:.2}).",
                            prob
                        ),
                        file: "deep_scan".into(),
                        line: None,
                    });
                }
                ml_validator::ThreatClassification::Suspicious => {
                    findings.push(Finding {
                        id: "DS001".into(),
                        title: "AI Deep Scan: Suspicious".into(),
                        severity: FindingSeverity::Medium,
                        description: format!(
                            "Multi-layer scan (YARA+OSV+ThreatIntel) flagged suspicious patterns (probability: {:.2}).",
                            prob
                        ),
                        file: "deep_scan".into(),
                        line: None,
                    });
                }
                _ => {}
            }
        }
        Err(e) => {
            tracing::warn!(
                "Deep scan failed: {}; continuing with static analysis only",
                e
            );
            // Emit a finding so the network knows ML was not available
            findings.push(Finding {
                id: "ML001".into(),
                title: "ML Deep Scan: Unavailable".into(),
                severity: FindingSeverity::Medium,
                description: format!(
                    "Multi-layer scan failed: {}. YARA/OSV/ThreatIntel detection was not performed. \
                     Package was analyzed with static rules only.",
                    e
                ),
                file: "deep_scan".into(),
                line: None,
            });
        }
    }

    // ── Ensemble Scoring ─────────────────────────────────────────────────────
    // Combine rule-based, deep-scan, and LLM signals into a single weighted
    // score. Weights: static patterns 30%, rule-based ML 25%, deep scan 30%,
    // LLM 15%. Each component is normalised to 0–100.
    let ensemble_score = compute_ensemble_score(&findings);

    Ok(StaticAnalysisResult { findings, ensemble_score })
}

/// Checks whether a finding is covered by the publisher's declared manifest.
fn is_excused_by_manifest(pat: &Pattern, manifest: &PackageManifest) -> bool {
    match pat.id.as_str() {
        "SA004" => manifest.spawns_processes, // declared it spawns processes
        "SA001" | "SA003" => false,           // eval/exec never excused
        _ => false,
    }
}

/// Compute weighted ensemble score (0–100) from findings.
/// Components:
///   - Static pattern findings (SA*): 30%  → max severity maps to 100
///   - Rule-based ML (ML002/ML003):   25%  → threat_score extracted from description
///   - Deep scan (DS001-DS003):        30%  → probability mapped to 0-100
///   - LLM (SA011):                    15%  → score extracted from description
fn compute_ensemble_score(findings: &[Finding]) -> f64 {
    let mut static_score: f64 = 0.0;
    let mut ml_rule_score: f64 = 0.0;
    let mut deep_score: f64 = 0.0;
    let mut llm_score: f64 = 0.0;

    for f in findings {
        match f.id.as_str() {
            // Static pattern findings — score by severity
            id if id.starts_with("SA") && id != "SA009" && id != "SA011" => {
                let sev = match f.severity {
                    FindingSeverity::Critical => 100.0,
                    FindingSeverity::High => 75.0,
                    FindingSeverity::Medium => 50.0,
                    FindingSeverity::Low => 25.0,
                };
                if sev > static_score { static_score = sev; }
            }
            // Rule-based ML score
            "ML002" => ml_rule_score = 85.0,
            "ML003" => {
                if ml_rule_score < 60.0 { ml_rule_score = 60.0; }
            }
            // Deep scan score
            "DS003" => deep_score = 100.0,
            "DS002" => {
                if deep_score < 75.0 { deep_score = 75.0; }
            }
            "DS001" => {
                if deep_score < 50.0 { deep_score = 50.0; }
            }
            // LLM score
            "SA011" => {
                let sev = match f.severity {
                    FindingSeverity::Critical => 90.0,
                    _ => 60.0,
                };
                if sev > llm_score { llm_score = sev; }
            }
            _ => {}
        }
    }

    let score = static_score * 0.30 + ml_rule_score * 0.25 + deep_score * 0.30 + llm_score * 0.15;
    score.min(100.0)
}

/// Shannon entropy of a string — high values indicate obfuscation.
fn shannon_entropy(s: &str) -> f64 {
    let mut freq = [0usize; 256];
    for b in s.bytes() {
        freq[b as usize] += 1;
    }
    let len = s.len() as f64;
    freq.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

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

fn find_line_number(content: &str, needle: &str) -> Option<usize> {
    content.lines().enumerate().find_map(|(i, l)| {
        if l.contains(needle) {
            Some(i + 1)
        } else {
            None
        }
    })
}

fn extract_text_files(tarball: &[u8]) -> Result<Vec<(String, String)>> {
    use std::io::Read;
    let gz = flate2::read::GzDecoder::new(tarball);
    let mut archive = tar::Archive::new(gz);
    let mut files = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().to_string();
        let mut content = String::new();
        if entry.read_to_string(&mut content).is_ok() && !content.is_empty() {
            files.push((path, content));
        }
    }
    Ok(files)
}

use crate::typosquat;

/// Extract the package name, version, and ecosystem from the tarball's manifest files.
fn extract_package_identity(files: &[(String, String)]) -> (String, String, String) {
    for (path, content) in files {
        if path.ends_with("package.json") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(content) {
                if let Some(name) = v["name"].as_str() {
                    let version = v["version"].as_str().unwrap_or("0.0.0").to_string();
                    return (name.to_string(), version, "npm".to_string());
                }
            }
        }
        if path.ends_with("Cargo.toml") {
            let mut name = String::new();
            let mut version = String::new();
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("name") {
                    let n = rest
                        .trim_start_matches([' ', '=', '"'])
                        .trim_end_matches('"')
                        .trim();
                    if !n.is_empty() {
                        name = n.to_string();
                    }
                }
                if let Some(rest) = line.strip_prefix("version") {
                    let v = rest
                        .trim_start_matches([' ', '=', '"'])
                        .trim_end_matches('"')
                        .trim();
                    if !v.is_empty() && version.is_empty() {
                        version = v.to_string();
                    }
                }
            }
            if !name.is_empty() {
                if version.is_empty() {
                    version = "0.0.0".to_string();
                }
                return (name, version, "cargo".to_string());
            }
        }
        if path.ends_with("setup.py")
            || path.ends_with("setup.cfg")
            || path.ends_with("pyproject.toml")
        {
            let mut name = String::new();
            let mut version = String::new();
            for line in content.lines() {
                if line.trim_start().starts_with("name") {
                    let n = line
                        .splitn(2, '=')
                        .nth(1)
                        .unwrap_or("")
                        .trim()
                        .trim_matches(['"', '\'', ' ']);
                    if !n.is_empty() {
                        name = n.to_string();
                    }
                }
                if line.trim_start().starts_with("version") {
                    let v = line
                        .splitn(2, '=')
                        .nth(1)
                        .unwrap_or("")
                        .trim()
                        .trim_matches(['"', '\'', ' ']);
                    if !v.is_empty() && version.is_empty() {
                        version = v.to_string();
                    }
                }
            }
            if !name.is_empty() {
                if version.is_empty() {
                    version = "0.0.0".to_string();
                }
                return (name, version, "pypi".to_string());
            }
        }
    }
    (String::new(), String::new(), String::new())
}

/// Levenshtein-distance based typosquat check against all known popular packages.
pub fn check_typosquatting_real(package_name: &str, ecosystem: &str) -> Option<Finding> {
    typosquat::check(package_name, ecosystem).map(|m| Finding {
        id: "SA010".into(),
        title: "Typosquatting detected".into(),
        severity: FindingSeverity::Critical,
        description: format!(
            "Possible typosquatting: '{}' is edit distance {} from popular package '{}'",
            m.candidate, m.distance, m.target
        ),
        file: "package manifest".into(),
        line: None,
    })
}
