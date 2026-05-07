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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EvidenceDeterminism {
    Deterministic,
    Advisory,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvidenceGroup {
    pub id: String,
    pub label: String,
    pub determinism: EvidenceDeterminism,
    pub score: f64,
    pub findings: Vec<Finding>,
}

pub struct StaticAnalysisResult {
    pub evidence_groups: Vec<EvidenceGroup>,
    pub findings: Vec<Finding>,
    /// Deterministic score (0-100) derived from static, rule-based ML, and deep scan evidence.
    pub deterministic_score: f64,
    /// Advisory score (0-100) derived from snippet-level semantic analysis.
    pub advisory_score: f64,
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
    /// Simple substring match. Extend to regex or AST checks via CREG_PATTERNS_FILE.
    needle: String,
    /// Optional ecosystem filter (e.g. `"npm"`, `"pypi"`, `"cargo"`).
    /// When `None` the pattern applies to all ecosystems.
    #[serde(default)]
    ecosystem: Option<String>,
}

/// Built-in default patterns; used when no external file is configured.
/// Patterns are ecosystem-aware: `ecosystem: None` applies to every package,
/// while a specific ecosystem string restricts the pattern to that registry.
fn default_patterns() -> Vec<Pattern> {
    vec![
        // ── npm / Node.js ─────────────────────────────────────────────────────
        Pattern { id: "SA001".into(), description: "Dynamic eval() of external or user-controlled data".into(), severity: FindingSeverity::Critical, needle: "eval(".into(), ecosystem: Some("npm".into()) },
        Pattern { id: "SA002".into(), description: "Obfuscated base64 string decode at runtime".into(), severity: FindingSeverity::High, needle: "Buffer.from(".into(), ecosystem: Some("npm".into()) },
        Pattern { id: "SA003".into(), description: "exec() / execSync() shell execution".into(), severity: FindingSeverity::Critical, needle: "execSync(".into(), ecosystem: Some("npm".into()) },
        Pattern { id: "SA004".into(), description: "Spawns child processes (child_process.spawn)".into(), severity: FindingSeverity::Medium, needle: "child_process".into(), ecosystem: Some("npm".into()) },
        Pattern { id: "SA005".into(), description: "Reads environment variables (potential credential harvesting)".into(), severity: FindingSeverity::Low, needle: "process.env".into(), ecosystem: Some("npm".into()) },
        Pattern { id: "SA006".into(), description: "Raw HTTP request in install/postinstall hook".into(), severity: FindingSeverity::High, needle: "require('http')".into(), ecosystem: Some("npm".into()) },
        Pattern { id: "SA007".into(), description: "Writes to home directory or system paths".into(), severity: FindingSeverity::High, needle: "os.homedir()".into(), ecosystem: Some("npm".into()) },
        Pattern { id: "SA008".into(), description: "Crypto miner indicators".into(), severity: FindingSeverity::Critical, needle: "CryptoNight".into(), ecosystem: None },

        // ── Python / PyPI ─────────────────────────────────────────────────────
        Pattern { id: "SA020".into(), description: "os.system() shell execution".into(), severity: FindingSeverity::Critical, needle: "os.system(".into(), ecosystem: Some("pypi".into()) },
        Pattern { id: "SA021".into(), description: "subprocess.Popen() shell execution".into(), severity: FindingSeverity::High, needle: "subprocess.Popen(".into(), ecosystem: Some("pypi".into()) },
        Pattern { id: "SA022".into(), description: "eval() in Python code".into(), severity: FindingSeverity::Critical, needle: "eval(".into(), ecosystem: Some("pypi".into()) },
        Pattern { id: "SA023".into(), description: "exec() built-in usage".into(), severity: FindingSeverity::High, needle: "exec(".into(), ecosystem: Some("pypi".into()) },
        Pattern { id: "SA024".into(), description: "Dynamic __import__ call".into(), severity: FindingSeverity::High, needle: "__import__(".into(), ecosystem: Some("pypi".into()) },
        Pattern { id: "SA025".into(), description: "urllib.request or urllib2 HTTP outbound".into(), severity: FindingSeverity::Medium, needle: "urllib.request".into(), ecosystem: Some("pypi".into()) },
        Pattern { id: "SA026".into(), description: "socket.connect() raw TCP".into(), severity: FindingSeverity::Medium, needle: "socket.connect(".into(), ecosystem: Some("pypi".into()) },
        Pattern { id: "SA027".into(), description: "os.environ credential access".into(), severity: FindingSeverity::Low, needle: "os.environ".into(), ecosystem: Some("pypi".into()) },

        // ── Rust / crates.io ──────────────────────────────────────────────────
        Pattern { id: "SA030".into(), description: "std::process::Command shell execution".into(), severity: FindingSeverity::Medium, needle: "std::process::Command".into(), ecosystem: Some("cargo".into()) },
        Pattern { id: "SA031".into(), description: "std::fs::write to suspicious path".into(), severity: FindingSeverity::Medium, needle: "std::fs::write".into(), ecosystem: Some("cargo".into()) },
        Pattern { id: "SA032".into(), description: "unsafe block — may bypass memory safety".into(), severity: FindingSeverity::Low, needle: "unsafe {".into(), ecosystem: Some("cargo".into()) },

        // ── Shell scripts (any ecosystem) ─────────────────────────────────────
        Pattern { id: "SA040".into(), description: "curl pipe to bash — remote code execution".into(), severity: FindingSeverity::Critical, needle: "curl ".into(), ecosystem: None },
        Pattern { id: "SA041".into(), description: "wget pipe to shell — remote code execution".into(), severity: FindingSeverity::Critical, needle: "wget ".into(), ecosystem: None },
        Pattern { id: "SA042".into(), description: "base64 decode then execute shell pattern".into(), severity: FindingSeverity::High, needle: "base64 -d".into(), ecosystem: None },

        // ── Cross-ecosystem indicators ────────────────────────────────────────
        Pattern { id: "SA050".into(), description: "Crypto miner stratum protocol connection".into(), severity: FindingSeverity::Critical, needle: "stratum+tcp://".into(), ecosystem: None },
        Pattern { id: "SA051".into(), description: "Reverse shell netcat pattern".into(), severity: FindingSeverity::Critical, needle: "nc -e".into(), ecosystem: None },
        Pattern { id: "SA052".into(), description: "Python eval-based reverse shell".into(), severity: FindingSeverity::Critical, needle: "pty.spawn".into(), ecosystem: None },
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
    let mut deterministic_findings = Vec::new();
    let mut snippet_llm_findings = Vec::new();
    let mut rule_ml_findings = Vec::new();
    let mut deep_scan_findings = Vec::new();

    // Extract files from the tarball (tar.gz).
    // Oversized files are not scanned but produce an SA013 finding so the
    // limitation is visible in the report and consensus logs.
    let (files, oversized_paths) = extract_text_files(tarball_bytes)?;
    for path in oversized_paths {
        deterministic_findings.push(Finding {
            id: "SA013".into(),
            title: "File too large for static analysis".into(),
            severity: FindingSeverity::Medium,
            description: format!(
                "'{}' exceeds the static analysis file size limit ({:.0} MiB). \
                 The file was not pattern-scanned or entropy-checked. \
                 Increase CREG_STATIC_MAX_FILE_BYTES or investigate manually.",
                path,
                max_file_bytes() as f64 / (1024.0 * 1024.0)
            ),
            file: path.clone(),
            line: None,
        });
    }

    // Determine identity and ecosystem once for pattern filtering, typosquat,
    // and OSV lookups. Falls back to empty strings when no manifest is found
    // (patterns with `ecosystem: None` still apply; ecosystem-specific patterns
    // are skipped).
    let (pkg_name, pkg_version, ecosystem) = extract_package_identity(&files);

    for (path, content) in &files {
        // Only analyse source files; skip binaries, images, lock files, etc.
        if !is_source_file(path) {
            continue;
        }

        for pat in patterns() {
            // Skip patterns scoped to a different ecosystem.
            if let Some(ref pat_eco) = pat.ecosystem {
                if !pat_eco.eq_ignore_ascii_case(&ecosystem) {
                    continue;
                }
            }

            if content.contains(&pat.needle[..]) {
                // Cross-check against the publisher's declared manifest.
                if is_excused_by_manifest(pat, manifest) {
                    continue;
                }

                deterministic_findings.push(Finding {
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
                deterministic_findings.push(Finding {
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
            match crate::llm::predict_intent(&content).await {
                Ok(Some(score)) if score >= 80 => {
                    snippet_llm_findings.push(Finding {
                        id: "SA011".into(),
                        title: "AI-Verified Malicious Intent".into(),
                        severity: FindingSeverity::Critical,
                        description: format!("LLM semantic analysis indicates high probability (score: {}) of malicious intent in obfuscated logic.", score),
                        file: path.clone(),
                        line: None,
                    });
                }
                Ok(Some(score)) if score >= 50 => {
                    snippet_llm_findings.push(Finding {
                        id: "SA011".into(),
                        title: "AI-Suspicious Obfuscation".into(),
                        severity: FindingSeverity::Medium,
                        description: format!("LLM analysis flagged suspicious but inconclusive obfuscated logic (score: {}).", score),
                        file: path.clone(),
                        line: None,
                    });
                }
                Ok(Some(_)) => {
                    // Score below 50 — LLM ran and considered it benign; no finding.
                }
                Ok(None) => {
                    // LLM unavailable — emit a visible degraded-mode finding so
                    // consensus can see that obfuscated code was NOT semantically
                    // verified. Treated as High (not Critical) because static
                    // analysis still flagged SA009 on the same file.
                    snippet_llm_findings.push(Finding {
                        id: "SA012".into(),
                        title: "LLM unavailable for obfuscated code".into(),
                        severity: FindingSeverity::High,
                        description: "High-entropy code was flagged but the LLM semantic analyser was unavailable. Treating as unverified.".into(),
                        file: path.clone(),
                        line: None,
                    });
                }
                Err(e) => {
                    // Treat LLM errors the same as unavailability: emit a
                    // degraded-mode finding so consensus can see that this
                    // high-entropy file was NOT semantically verified.
                    // Silently scoring it 0 (benign) here would allow
                    // obfuscated malware to pass whenever the LLM endpoint
                    // is slow, rate-limited, or misconfigured.
                    tracing::warn!(error = %e, "LLM predict_intent failed — treating high-entropy file as unverified");
                    snippet_llm_findings.push(Finding {
                        id: "SA012".into(),
                        title: "LLM error for obfuscated code".into(),
                        severity: FindingSeverity::High,
                        description: format!(
                            "High-entropy code was flagged but the LLM semantic analyser returned an error: {}. \
                             Treating as unverified.",
                            e
                        ),
                        file: path.clone(),
                        line: None,
                    });
                }
            }
        }
    }

    // Check for typosquatting using Levenshtein distance against all popular packages.
    // pkg_name / pkg_version / ecosystem were extracted at the top of run().
    if !pkg_name.is_empty() {
        if let Some(finding) = check_typosquatting_real(&pkg_name, &ecosystem) {
            deterministic_findings.push(finding);
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
                    rule_ml_findings.push(Finding {
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
                    rule_ml_findings.push(Finding {
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

    let deep_scan_result = {
        let tarball_bytes = tarball_bytes.to_vec();
        let pkg_info = pkg_info.clone();
        let eco = ecosystem.clone();
        tokio::task::spawn_blocking(move || ml_validator::deep_scan(&tarball_bytes, pkg_info, &eco)).await
    };

    match deep_scan_result {
        Ok(Ok(deep)) => {
            // If deep scan ran in mock/degraded mode, emit a visible warning finding
            // so validators and the network are aware ML coverage is not active.
            if deep.is_mock {
                deep_scan_findings.push(Finding {
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
                    deep_scan_findings.push(Finding {
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
                    deep_scan_findings.push(Finding {
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
                    deep_scan_findings.push(Finding {
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
        Ok(Err(e)) => {
            tracing::warn!(
                "Deep scan failed: {}; continuing with static analysis only",
                e
            );
            // Emit a finding so the network knows ML was not available
            deep_scan_findings.push(Finding {
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
        Err(e) => {
            tracing::warn!(
                "Deep scan task failed: {}; continuing with static analysis only",
                e
            );
            deep_scan_findings.push(Finding {
                id: "ML001".into(),
                title: "ML Deep Scan: Unavailable".into(),
                severity: FindingSeverity::Medium,
                description: format!(
                    "Multi-layer scan task failed: {}. YARA/OSV/ThreatIntel detection was not performed. \
                     Package was analyzed with static rules only.",
                    e
                ),
                file: "deep_scan".into(),
                line: None,
            });
        }
    }

    let evidence_groups = [
        build_evidence_group(
            "static-patterns",
            "Static pattern and manifest evidence",
            EvidenceDeterminism::Deterministic,
            deterministic_findings,
        ),
        build_evidence_group(
            "snippet-llm",
            "Snippet-level semantic review",
            EvidenceDeterminism::Advisory,
            snippet_llm_findings,
        ),
        build_evidence_group(
            "rule-ml",
            "Rule-based ML scoring",
            EvidenceDeterminism::Deterministic,
            rule_ml_findings,
        ),
        build_evidence_group(
            "deep-scan",
            "Deep scan threat classification",
            EvidenceDeterminism::Deterministic,
            deep_scan_findings,
        ),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    let deterministic_score = compute_deterministic_score(&evidence_groups);
    let advisory_score = compute_advisory_score(&evidence_groups);
    let ensemble_score = compute_ensemble_score(deterministic_score, advisory_score);
    let findings = evidence_groups
        .iter()
        .flat_map(|group| group.findings.clone())
        .collect();

    Ok(StaticAnalysisResult {
        evidence_groups,
        findings,
        deterministic_score,
        advisory_score,
        ensemble_score,
    })
}

/// Checks whether a finding is covered by the publisher's declared manifest.
///
/// When a package legitimately declares a capability (e.g. `allowed_network_hosts`
/// for an HTTP client), the corresponding finding is suppressed so that
/// legitimate packages are not penalised for declared, expected behaviour.
/// Patterns whose IDs are not listed here are never excused regardless of the
/// manifest (e.g. `eval()` execution, `execSync()` shell injection).
fn is_excused_by_manifest(pat: &Pattern, manifest: &PackageManifest) -> bool {
    match pat.id.as_str() {
        // eval() and exec-family are never excused — no legitimate package needs them.
        "SA001" | "SA003" | "SA022" | "SA023" => false,

        // Child-process spawning: excused only when explicitly declared.
        "SA004" | "SA030" => manifest.spawns_processes,

        // Network access: excused when at least one allowed host is declared.
        "SA006" | "SA025" | "SA026" => !manifest.allowed_network_hosts.is_empty(),

        // Home/system directory writes: excused when at least one write path is declared.
        "SA007" | "SA031" => !manifest.allowed_fs_writes.is_empty(),

        // Env-var reads are low-severity and commonly needed; excuse if spawns_processes
        // is true (a common pattern: spawn a subprocess with env vars forwarded).
        // The crypto miner needle (SA008/SA050/SA051/SA052) is never excused.
        _ => false,
    }
}

fn build_evidence_group(
    id: &str,
    label: &str,
    determinism: EvidenceDeterminism,
    findings: Vec<Finding>,
) -> Option<EvidenceGroup> {
    if findings.is_empty() {
        return None;
    }

    Some(EvidenceGroup {
        id: id.to_string(),
        label: label.to_string(),
        determinism,
        score: group_score(id, &findings),
        findings,
    })
}

fn group_score(id: &str, findings: &[Finding]) -> f64 {
    match id {
        "static-patterns" => findings
            .iter()
            .filter(|finding| finding.id != "SA011" && finding.id != "SA012")
            .map(|finding| severity_score(finding.severity))
            .fold(0.0, f64::max),
        "rule-ml" => {
            if findings.iter().any(|finding| finding.id == "ML002") {
                85.0
            } else if findings.iter().any(|finding| finding.id == "ML003") {
                60.0
            } else {
                0.0
            }
        }
        "deep-scan" => {
            if findings.iter().any(|finding| finding.id == "DS003") {
                100.0
            } else if findings.iter().any(|finding| finding.id == "DS002") {
                75.0
            } else if findings.iter().any(|finding| finding.id == "DS001") {
                50.0
            } else if findings.iter().any(|finding| finding.id == "ML001") {
                40.0
            } else {
                0.0
            }
        }
        "snippet-llm" => findings
            .iter()
            .map(|finding| match finding.id.as_str() {
                "SA011" if finding.severity == FindingSeverity::Critical => 90.0,
                "SA011" => 60.0,
                "SA012" => 55.0,
                _ => 0.0,
            })
            .fold(0.0, f64::max),
        _ => 0.0,
    }
}

fn compute_deterministic_score(groups: &[EvidenceGroup]) -> f64 {
    let static_score = score_for_group(groups, "static-patterns");
    let rule_ml_score = score_for_group(groups, "rule-ml");
    let deep_score = score_for_group(groups, "deep-scan");

    let weighted = static_score * 0.30 + rule_ml_score * 0.25 + deep_score * 0.30;
    if weighted == 0.0 {
        0.0
    } else {
        (weighted / 0.85).min(100.0)
    }
}

fn compute_advisory_score(groups: &[EvidenceGroup]) -> f64 {
    score_for_group(groups, "snippet-llm")
}

fn compute_ensemble_score(deterministic_score: f64, advisory_score: f64) -> f64 {
    (deterministic_score * 0.85 + advisory_score * 0.15).min(100.0)
}

fn score_for_group(groups: &[EvidenceGroup], id: &str) -> f64 {
    groups
        .iter()
        .find(|group| group.id == id)
        .map(|group| group.score)
        .unwrap_or(0.0)
}

fn severity_score(severity: FindingSeverity) -> f64 {
    match severity {
        FindingSeverity::Critical => 100.0,
        FindingSeverity::High => 75.0,
        FindingSeverity::Medium => 50.0,
        FindingSeverity::Low => 25.0,
    }
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
        // JavaScript / TypeScript
        "js" | "ts" | "mjs" | "cjs" | "jsx" | "tsx"
        // Python
        | "py" | "pyw"
        // Ruby
        | "rb"
        // Rust
        | "rs"
        // JVM
        | "java" | "kt" | "groovy" | "scala"
        // Shell scripts — critical: postinstall hooks are often .sh
        | "sh" | "bash" | "zsh" | "fish" | "ksh"
        // Web / server-side
        | "php" | "php5" | "phtml"
        // Go
        | "go"
        // C / C++ — native add-ons common in npm packages
        | "c" | "cpp" | "cc" | "cxx" | "h" | "hpp"
        // Swift / Objective-C
        | "swift" | "m"
        // Lua
        | "lua"
        // PowerShell — relevant for Windows packages
        | "ps1" | "psm1"
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

/// Maximum bytes read from a single source file.
/// Configurable via `CREG_STATIC_MAX_FILE_BYTES` (default: 5 MiB).
/// Files exceeding this limit are skipped with an SA013 finding so the
/// limitation is visible in the report — silently dropping them would let
/// attackers hide malicious code past the size threshold.
fn max_file_bytes() -> u64 {
    std::env::var("CREG_STATIC_MAX_FILE_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5 * 1024 * 1024) // 5 MiB
}

/// Returns `(readable_files, oversized_paths)`.
/// `oversized_paths` are paths whose size exceeds `max_file_bytes()`.
/// Callers emit SA013 findings for each oversized path.
fn extract_text_files(tarball: &[u8]) -> Result<(Vec<(String, String)>, Vec<String>)> {
    use std::io::Read;
    let limit = max_file_bytes();
    let gz = flate2::read::GzDecoder::new(tarball);
    let mut archive = tar::Archive::new(gz);
    let mut files = Vec::new();
    let mut oversized = Vec::new();
    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?.to_string_lossy().to_string();
        // Check declared size from tar header before reading. A malicious
        // tarball may lie about size, so we also enforce the limit during read.
        let declared_size = entry.header().size().unwrap_or(0);
        if declared_size > limit {
            oversized.push(path);
            continue;
        }
        // Read with a hard byte cap to guard against tarballs that under-report size.
        let mut content = String::new();
        let mut limited = entry.take(limit + 1);
        if limited.read_to_string(&mut content).is_ok() && !content.is_empty() {
            if content.len() as u64 > limit {
                oversized.push(path);
            } else {
                files.push((path, content));
            }
        }
    }
    Ok((files, oversized))
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
