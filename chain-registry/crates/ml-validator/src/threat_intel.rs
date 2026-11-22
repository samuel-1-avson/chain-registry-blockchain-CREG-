//! Content-hash–based threat intelligence matching.
//!
//! Maintains a JSON-backed database of SHA-256 hashes of known-malicious
//! package contents.  Validators share newly discovered malicious hashes
//! via network consensus; this module looks up the per-file content hashes
//! against that database.
//!
//! No ML training required — the database grows organically as the
//! validator network flags packages.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;
use tracing::{debug, info, warn};

/// Minimum useful DB size below which we emit a persistent warning.
const MIN_USEFUL_DB_SIZE: usize = 100;

pub const THREAT_DB_SCHEMA_V1: &str = "creg-threat-db-v1";

/// Default path for the known-malicious hash database.
const DEFAULT_DB_PATH: &str = "data/known_malicious_hashes.json";

/// A single entry in the threat intel database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatEntry {
    /// Human-readable label (e.g. "ua-parser-js@0.7.29 postinstall backdoor").
    pub label: String,
    /// Threat level (1–5).
    pub threat_level: u8,
    /// Source of the intelligence (e.g. "socket.dev", "validator-consensus").
    pub source: String,
}

/// The in-memory threat intel database.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThreatDatabase {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub source_response_sha256: String,
    #[serde(default)]
    pub built_at: String,
    /// Map from SHA-256 hex hash → threat entry.
    #[serde(default)]
    pub entries: HashMap<String, ThreatEntry>,
}

#[derive(Debug, Clone)]
pub struct ThreatDatabaseStatus {
    pub available: bool,
    pub digest: String,
    pub source: String,
    pub source_response_sha256: String,
    pub built_at: String,
    pub entries: usize,
    pub error: Option<String>,
}

/// Result of a threat-intel lookup.
#[derive(Debug, Clone)]
pub struct ThreatIntelResult {
    /// Matching entries: hash → entry.
    pub matches: Vec<(String, ThreatEntry)>,
}

impl ThreatIntelResult {
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// Convert to a malicious probability (0.0 – 1.0).
    pub fn to_probability(&self) -> f32 {
        if self.matches.is_empty() {
            return 0.0;
        }
        let max_threat: u8 = self
            .matches
            .iter()
            .map(|(_, e)| e.threat_level)
            .max()
            .unwrap_or(0);
        match max_threat {
            5 => 0.99,
            4 => 0.90,
            3 => 0.70,
            2 => 0.40,
            _ => 0.20,
        }
    }
}

/// Database path, overridable via `CREG_THREAT_DB_PATH`.
fn db_path() -> PathBuf {
    std::env::var("CREG_THREAT_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_DB_PATH))
}

/// Loaded database singleton.
static THREAT_DB: OnceLock<ThreatDatabase> = OnceLock::new();

fn load_database() -> ThreatDatabase {
    let path = db_path();

    if !path.exists() {
        warn!(
            path = %path.display(),
            "Operator threat-intelligence snapshot is missing; no built-in sample hashes are used"
        );
        return ThreatDatabase::default();
    }

    let db = match std::fs::read_to_string(&path) {
        Ok(json) => match serde_json::from_str::<ThreatDatabase>(&json) {
            Ok(db) => {
                info!(path = %path.display(), entries = db.entries.len(), "Operator threat-intel snapshot loaded");
                db
            }
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to parse operator threat-intel snapshot; using an empty database"
                );
                ThreatDatabase::default()
            }
        },
        Err(e) => {
            warn!(
                path = %path.display(),
                error = %e,
                "Failed to read operator threat-intel snapshot; using an empty database"
            );
            ThreatDatabase::default()
        }
    };
    warn_if_small(&db);
    db
}

/// Inspect the configured snapshot without changing its contents.
pub fn database_status() -> ThreatDatabaseStatus {
    let path = db_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return ThreatDatabaseStatus {
                available: false,
                digest: String::new(),
                source: String::new(),
                source_response_sha256: String::new(),
                built_at: String::new(),
                entries: 0,
                error: Some(format!("cannot read configured threat database: {error}")),
            };
        }
    };
    let digest = sha256_hex(&bytes);
    let db = match serde_json::from_slice::<ThreatDatabase>(&bytes) {
        Ok(db) => db,
        Err(error) => {
            return ThreatDatabaseStatus {
                available: false,
                digest,
                source: String::new(),
                source_response_sha256: String::new(),
                built_at: String::new(),
                entries: 0,
                error: Some(format!("invalid threat database JSON: {error}")),
            };
        }
    };
    let invalid_hashes = db
        .entries
        .keys()
        .filter(|hash| {
            hash.len() != 64
                || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                || hash.bytes().any(|byte| byte.is_ascii_uppercase())
        })
        .count();
    let invalid_entries = db
        .entries
        .values()
        .filter(|entry| {
            entry.label.trim().is_empty()
                || !(1..=5).contains(&entry.threat_level)
                || entry.source.trim().is_empty()
        })
        .count();
    let error = if db.schema != THREAT_DB_SCHEMA_V1 {
        Some(format!(
            "unsupported or missing threat database schema '{}'",
            db.schema
        ))
    } else if db.built_at.trim().is_empty() {
        Some("threat database build-time provenance is required".to_string())
    } else if !db.source.starts_with("https://mb-api.abuse.ch/api/v1/") {
        Some("threat database source is not the Abuse.ch MalwareBazaar API".to_string())
    } else if db.source_response_sha256.len() != 64
        || !db
            .source_response_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Some("threat database has no valid digest for its captured source response".to_string())
    } else if invalid_hashes != 0 || invalid_entries != 0 {
        Some(format!(
            "threat database has {invalid_hashes} malformed SHA-256 keys and {invalid_entries} invalid entries"
        ))
    } else if db.entries.len() < MIN_USEFUL_DB_SIZE {
        Some(format!(
            "threat database has {} entries; at least {MIN_USEFUL_DB_SIZE} are required",
            db.entries.len()
        ))
    } else {
        None
    };
    ThreatDatabaseStatus {
        available: error.is_none(),
        digest,
        source: db.source,
        source_response_sha256: db.source_response_sha256,
        built_at: db.built_at,
        entries: db.entries.len(),
        error,
    }
}

fn warn_if_small(db: &ThreatDatabase) {
    if db.entries.len() < MIN_USEFUL_DB_SIZE {
        warn!(
            entries = db.entries.len(),
            minimum = MIN_USEFUL_DB_SIZE,
            "Threat intel database is below minimum useful size"
        );
    }
}

fn get_db() -> &'static ThreatDatabase {
    THREAT_DB.get_or_init(load_database)
}

/// Compute the SHA-256 hash of content.
pub fn sha256_hex(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

/// Check extracted package files against the threat intel database.
///
/// `files` is a list of `(relative_path, file_content)` pairs.
/// Also checks the hash of the entire tarball.
pub fn check(tarball_bytes: &[u8], files: &[(String, String)]) -> ThreatIntelResult {
    let db = get_db();
    let mut matches = Vec::new();

    // Check whole-tarball hash.
    let tarball_hash = sha256_hex(tarball_bytes);
    if let Some(entry) = db.entries.get(&tarball_hash) {
        debug!(
            "Threat intel: tarball hash {} matches '{}'",
            &tarball_hash[..16],
            entry.label
        );
        matches.push((tarball_hash, entry.clone()));
    }

    // Check individual file hashes.
    let mut seen = HashSet::new();
    for (path, content) in files {
        let hash = sha256_hex(content.as_bytes());
        if seen.contains(&hash) {
            continue;
        }
        seen.insert(hash.clone());

        if let Some(entry) = db.entries.get(&hash) {
            debug!(
                "Threat intel: file '{}' hash {} matches '{}'",
                path,
                &hash[..16],
                entry.label
            );
            matches.push((hash, entry.clone()));
        }
    }

    ThreatIntelResult { matches }
}

/// Checks a pre-unpacked archive against the threat-intelligence database using
/// precomputed raw-byte SHA-256 hashes (zero per-scan disk or full re-hashing, `VNA-12`).
pub fn check_unpacked(archive: &creg_unpack::UnpackedArchive) -> ThreatIntelResult {
    let db = get_db();
    let mut matches = Vec::new();

    if let Some(entry) = db.entries.get(&archive.tarball_sha256) {
        debug!(
            "Threat intel: tarball hash {} matches '{}'",
            &archive.tarball_sha256[..archive.tarball_sha256.len().min(16)],
            entry.label
        );
        matches.push((archive.tarball_sha256.clone(), entry.clone()));
    }

    let mut seen = HashSet::with_capacity(archive.files.len());
    for file in &archive.files {
        if !seen.insert(&file.sha256) {
            continue;
        }
        if let Some(entry) = db.entries.get(&file.sha256) {
            debug!(
                "Threat intel: file '{}' hash {} matches '{}'",
                file.path,
                &file.sha256[..file.sha256.len().min(16)],
                entry.label
            );
            matches.push((file.sha256.clone(), entry.clone()));
        }
    }

    ThreatIntelResult { matches }
}

/// Add a new entry to the on-disk database.
///
/// Called when the network reaches consensus that a package is malicious.
pub fn record_threat(hash: &str, entry: ThreatEntry) -> Result<(), String> {
    let path = db_path();

    // Load current database (may differ from cached singleton).
    let mut db: ThreatDatabase = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    } else {
        ThreatDatabase::default()
    };

    db.entries.insert(hash.to_string(), entry);

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }

    let json = serde_json::to_string_pretty(&db).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("write: {e}"))?;

    info!(
        "Recorded threat intel entry: hash={}",
        &hash[..16.min(hash.len())]
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hex_deterministic() {
        let h1 = sha256_hex(b"hello world");
        let h2 = sha256_hex(b"hello world");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // 256 bits = 64 hex chars
    }

    #[test]
    fn test_sha256_hex_known_digest() {
        // SHA-256 of empty input is well-known
        let h = sha256_hex(b"");
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_hex_different_inputs() {
        assert_ne!(sha256_hex(b"aaa"), sha256_hex(b"bbb"));
    }

    #[test]
    fn test_threat_intel_result_empty() {
        let r = ThreatIntelResult { matches: vec![] };
        assert!(r.is_empty());
        assert_eq!(r.to_probability(), 0.0);
    }

    #[test]
    fn test_threat_intel_result_probability_level_5() {
        let r = ThreatIntelResult {
            matches: vec![(
                "abc".into(),
                ThreatEntry {
                    label: "test".into(),
                    threat_level: 5,
                    source: "test".into(),
                },
            )],
        };
        assert!(!r.is_empty());
        assert!((r.to_probability() - 0.99).abs() < 0.01);
    }

    #[test]
    fn test_threat_intel_result_probability_level_3() {
        let r = ThreatIntelResult {
            matches: vec![(
                "abc".into(),
                ThreatEntry {
                    label: "test".into(),
                    threat_level: 3,
                    source: "test".into(),
                },
            )],
        };
        assert!((r.to_probability() - 0.70).abs() < 0.01);
    }

    #[test]
    fn test_check_no_match_empty_db() {
        // With default empty database, nothing should match.
        let tarball = b"some tarball bytes";
        let files = vec![("index.js".to_string(), "console.log('safe')".to_string())];
        let result = check(tarball, &files);
        assert!(result.is_empty());
    }

    #[test]
    fn test_check_deduplicates_files() {
        // If two files have identical content, only one hash lookup occurs.
        let tarball = b"tarball";
        let files = vec![
            ("a.js".to_string(), "same content".to_string()),
            ("b.js".to_string(), "same content".to_string()),
        ];
        let result = check(tarball, &files);
        // No match expected (empty default DB), but no panic either.
        assert!(result.is_empty());
    }

    #[test]
    fn test_record_and_read_threat() {
        // Use a temp file to test record_threat round-trip.
        let dir = std::env::temp_dir().join("creg_test_threat_intel");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("test_db.json");
        // Clean up any previous run.
        let _ = std::fs::remove_file(&db_path);

        std::env::set_var("CREG_THREAT_DB_PATH", db_path.to_str().unwrap());

        let entry = ThreatEntry {
            label: "evil-pkg@1.0.0".into(),
            threat_level: 5,
            source: "unit-test".into(),
        };
        record_threat(
            "deadbeef00112233deadbeef00112233deadbeef00112233deadbeef00112233",
            entry,
        )
        .unwrap();

        // Read back and verify.
        let json = std::fs::read_to_string(&db_path).unwrap();
        let db: ThreatDatabase = serde_json::from_str(&json).unwrap();
        assert_eq!(db.entries.len(), 1);
        assert!(db
            .entries
            .contains_key("deadbeef00112233deadbeef00112233deadbeef00112233deadbeef00112233"));

        // Cleanup.
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&dir);
        std::env::remove_var("CREG_THREAT_DB_PATH");
    }
}
