// crates/validator/src/static_analysis.rs
// Stage 1: Static analysis of package source files.
// Scans the tarball for known malicious patterns without executing anything.

use anyhow::Result;
use common::{PackageManifest, Finding, FindingSeverity};

pub struct StaticAnalysisResult {
    pub findings: Vec<Finding>,
}

/// Dangerous patterns that are checked in source text.
struct Pattern {
    id: &'static str,
    description: &'static str,
    severity: FindingSeverity,
    /// Simple substring match for now; extend to regex or AST checks.
    needle: &'static str,
}

const PATTERNS: &[Pattern] = &[
    Pattern {
        id: "SA001",
        description: "Dynamic eval() of external or user-controlled data",
        severity: FindingSeverity::Critical,
        needle: "eval(",
    },
    Pattern {
        id: "SA002",
        description: "Obfuscated base64 string decode at runtime",
        severity: FindingSeverity::High,
        needle: "Buffer.from(",
    },
    Pattern {
        id: "SA003",
        description: "exec() / execSync() shell execution",
        severity: FindingSeverity::Critical,
        needle: "execSync(",
    },
    Pattern {
        id: "SA004",
        description: "Spawns child processes (child_process.spawn)",
        severity: FindingSeverity::Medium,
        needle: "child_process",
    },
    Pattern {
        id: "SA005",
        description: "Reads environment variables (potential credential harvesting)",
        severity: FindingSeverity::Low,
        needle: "process.env",
    },
    Pattern {
        id: "SA006",
        description: "Raw HTTP request in install/postinstall hook",
        severity: FindingSeverity::High,
        needle: "require('http')",
    },
    Pattern {
        id: "SA007",
        description: "Writes to home directory or system paths",
        severity: FindingSeverity::High,
        needle: "os.homedir()",
    },
    Pattern {
        id: "SA008",
        description: "Crypto miner indicators",
        severity: FindingSeverity::Critical,
        needle: "CryptoNight",
    },
];

pub async fn run(
    tarball_bytes: &[u8],
    manifest: &PackageManifest,
) -> Result<StaticAnalysisResult> {
    let mut findings = Vec::new();

    // Extract files from the tarball (tar.gz).
    let files = extract_text_files(tarball_bytes)?;

    for (path, content) in &files {
        // Only analyse JS/TS/Python/Rust/Ruby source files.
        if !is_source_file(path) { continue; }

        for pat in PATTERNS {
            if content.contains(pat.needle) {
                // Cross-check against the publisher's declared manifest.
                if is_excused_by_manifest(pat, manifest) { continue; }

                findings.push(Finding {
                    id: pat.id.to_string(),
                    title: pat.description.to_string(),
                    severity: pat.severity,
                    description: pat.description.to_string(),
                    file: path.clone(),
                    line: find_line_number(content, pat.needle),
                });
            }
        }

        let mut has_high_entropy = false;
        // Entropy check: flag highly entropic strings (obfuscated code).
        for (line_num, line) in content.lines().enumerate() {
            if shannon_entropy(line) > 5.5 && line.len() > 80 {
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

    // Check for typosquatting indicators vs known popular packages.
    if let Some(finding) = check_typosquatting(&files) {
        findings.push(finding);
    }

    Ok(StaticAnalysisResult { findings })
}

/// Checks whether a finding is covered by the publisher's declared manifest.
fn is_excused_by_manifest(pat: &Pattern, manifest: &PackageManifest) -> bool {
    match pat.id {
        "SA004" => manifest.spawns_processes, // declared it spawns processes
        "SA001" | "SA003" => false,           // eval/exec never excused
        _ => false,
    }
}

/// Shannon entropy of a string — high values indicate obfuscation.
fn shannon_entropy(s: &str) -> f64 {
    let mut freq = [0usize; 256];
    for b in s.bytes() { freq[b as usize] += 1; }
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
    matches!(ext, "js" | "ts" | "mjs" | "cjs" | "py" | "rb" | "rs" | "java")
}

fn find_line_number(content: &str, needle: &str) -> Option<usize> {
    content.lines().enumerate().find_map(|(i, l)| {
        if l.contains(needle) { Some(i + 1) } else { None }
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

fn check_typosquatting(files: &[(String, String)]) -> Option<Finding> {
    // Simplified: check if package.json name closely matches a known package.
    let known = ["express", "lodash", "react", "axios", "moment", "chalk"];
    for (path, content) in files {
        if path.ends_with("package.json") {
            for popular in &known {
                // Levenshtein distance check omitted for brevity.
                // A real implementation would use a proper edit-distance library.
                if content.contains(&format!("\"{}\"", popular.replace('e', "3"))) {
                    return Some(Finding {
                        id: "SA010".into(),
                        title: "Typosquatting mismatch".into(),
                        severity: FindingSeverity::Critical,
                        description: format!("Possible typosquatting of '{}'", popular),
                        file: path.clone(),
                        line: None,
                    });
                }
            }
        }
    }
    None
}

// ── Real typosquatting check using Levenshtein distance ──────────────────────

use crate::typosquat;

/// Replace the placeholder typosquat check with the real Levenshtein implementation.
pub fn check_typosquatting_real(
    package_name: &str,
    ecosystem: &str,
) -> Option<Finding> {
    typosquat::check(package_name, ecosystem).map(|m| Finding {
        id:          "SA010".into(),
        title:       "Typosquatting detected".into(),
        severity:    FindingSeverity::Critical,
        description: format!(
            "Possible typosquatting: '{}' is edit distance {} from popular package '{}'",
            m.candidate, m.distance, m.target
        ),
        file:        "package manifest".into(),
        line:        None,
    })
}
