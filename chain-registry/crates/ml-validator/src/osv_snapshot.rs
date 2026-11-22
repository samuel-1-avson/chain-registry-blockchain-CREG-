//! Pinned OSV advisory snapshot for consensus-safe vulnerability lookups.
//!
//! Validators must not call live OSV HTTP during `validate_package`. When
//! `CREG_OSV_CONSENSUS=true`, lookups read only from a local JSON snapshot
//! whose epoch is recorded in `analysis_bundles.osv_snapshot_epoch`.
//!
//! Off-chain callers should use [`lookup_advisory`] (pinned first, optional live
//! fallback). Consensus hot path must use [`lookup_pinned`] only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::osv_client::{self, OsvResult, OsvVulnerability, PackageInfo};

const DEFAULT_SNAPSHOT_PATH: &str = "data/osv_snapshot.json";
pub const SCHEMA_V1: &str = "creg-osv-snapshot-v1";

/// On-disk snapshot format shipped with the validator image or mounted by operators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsvSnapshot {
    pub epoch: String,
    pub schema: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub built_at: String,
    #[serde(default)]
    pub entries: HashMap<String, Vec<OsvVulnerability>>,
}

#[derive(Debug, Clone)]
pub struct OsvSnapshotStatus {
    pub enabled: bool,
    pub available: bool,
    pub digest: String,
    pub epoch: String,
    pub source: String,
    pub built_at: String,
    pub lookups: usize,
    pub records: usize,
    pub error: Option<String>,
}

impl OsvSnapshot {
    /// Load a snapshot from JSON bytes (tests and tooling).
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| format!("parse OSV snapshot: {e}"))
    }

    /// Cache key for a package version — must match snapshot builder tooling.
    pub fn entry_key(info: &PackageInfo) -> String {
        osv_client::cache_key(info)
    }
}

#[derive(Debug, Default)]
struct SnapshotCache {
    path: PathBuf,
    epoch: String,
    digest: String,
    snapshot: Option<Arc<OsvSnapshot>>,
}

static SNAPSHOT_CACHE: OnceLock<Mutex<SnapshotCache>> = OnceLock::new();

fn snapshot_cache() -> &'static Mutex<SnapshotCache> {
    SNAPSHOT_CACHE.get_or_init(|| Mutex::new(SnapshotCache::default()))
}

/// True when validators should use the pinned snapshot on the consensus hot path.
pub fn osv_consensus_enabled() -> bool {
    env_truthy("CREG_OSV_CONSENSUS")
}

/// When set with consensus OSV, OSV002 (critical CVE) findings become deterministic blockers.
pub fn osv_block_critical_enabled() -> bool {
    env_truthy("CREG_OSV_BLOCK_CRITICAL") && osv_consensus_enabled()
}

/// Off-chain: try live OSV when pinned snapshot misses or is unavailable.
pub fn osv_live_fallback_enabled() -> bool {
    env_truthy("CREG_OSV_LIVE_FALLBACK")
}

/// Epoch string recorded on validator votes (`analysis_bundles.osv_snapshot_epoch`).
pub fn bundle_epoch() -> String {
    if !osv_consensus_enabled() {
        return "osv-off".to_string();
    }
    std::env::var("CREG_OSV_SNAPSHOT_EPOCH")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "osv-epoch-0".to_string())
}

/// Path to the pinned snapshot file.
pub fn snapshot_path() -> PathBuf {
    std::env::var("CREG_OSV_SNAPSHOT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_SNAPSHOT_PATH))
}

/// Whether consensus OSV is enabled and a snapshot file loaded for the configured epoch.
pub fn snapshot_available() -> bool {
    if !osv_consensus_enabled() {
        return false;
    }
    loaded_snapshot().is_some()
}

/// Inspect the pinned OSV data and its source provenance without using live HTTP.
pub fn snapshot_status() -> OsvSnapshotStatus {
    if !osv_consensus_enabled() {
        return OsvSnapshotStatus {
            enabled: false,
            available: false,
            digest: String::new(),
            epoch: bundle_epoch(),
            source: String::new(),
            built_at: String::new(),
            lookups: 0,
            records: 0,
            error: Some("consensus OSV snapshot is disabled".to_string()),
        };
    }
    let path = snapshot_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return OsvSnapshotStatus {
                enabled: true,
                available: false,
                digest: String::new(),
                epoch: bundle_epoch(),
                source: String::new(),
                built_at: String::new(),
                lookups: 0,
                records: 0,
                error: Some(format!("cannot read OSV snapshot: {error}")),
            };
        }
    };
    let digest = crate::threat_intel::sha256_hex(&bytes);
    let snapshot = match OsvSnapshot::from_json_bytes(&bytes) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return OsvSnapshotStatus {
                enabled: true,
                available: false,
                digest,
                epoch: bundle_epoch(),
                source: String::new(),
                built_at: String::new(),
                lookups: 0,
                records: 0,
                error: Some(error),
            };
        }
    };
    let records = snapshot.entries.values().map(Vec::len).sum();
    let invalid_keys = snapshot
        .entries
        .keys()
        .filter(|key| {
            let Some((ecosystem, package_version)) = key.split_once(':') else {
                return true;
            };
            let Some((package, version)) = package_version.rsplit_once('@') else {
                return true;
            };
            ecosystem.trim().is_empty() || package.trim().is_empty() || version.trim().is_empty()
        })
        .count();
    let invalid_vulnerabilities = snapshot
        .entries
        .values()
        .flatten()
        .filter(|vulnerability| vulnerability.id.trim().is_empty())
        .count();
    let expected_epoch = bundle_epoch();
    let error = if snapshot.schema != SCHEMA_V1 {
        Some(format!(
            "unsupported OSV snapshot schema '{}'",
            snapshot.schema
        ))
    } else if snapshot.epoch != expected_epoch {
        Some(format!(
            "OSV snapshot epoch '{}' does not match configured epoch '{expected_epoch}'",
            snapshot.epoch
        ))
    } else if snapshot.source != "https://api.osv.dev/v1/query"
        || snapshot.built_at.trim().is_empty()
    {
        Some("OSV snapshot must identify the official OSV query API and its build time".to_string())
    } else if snapshot.entries.is_empty() {
        Some("OSV snapshot contains no successfully queried package versions".to_string())
    } else if invalid_keys != 0 || invalid_vulnerabilities != 0 {
        Some(format!(
            "OSV snapshot has {invalid_keys} malformed package keys and {invalid_vulnerabilities} advisories without IDs"
        ))
    } else {
        None
    };
    OsvSnapshotStatus {
        enabled: true,
        available: error.is_none(),
        digest,
        epoch: snapshot.epoch,
        source: snapshot.source,
        built_at: snapshot.built_at,
        lookups: snapshot.entries.len(),
        records,
        error,
    }
}

/// Clear cached snapshot (tests).
#[cfg(test)]
pub fn reset_snapshot_cache_for_tests() {
    if let Ok(mut cache) = snapshot_cache().lock() {
        *cache = SnapshotCache::default();
    }
}

fn file_digest(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| crate::threat_intel::sha256_hex(&bytes))
}

fn load_snapshot_from_disk(path: &Path, expected_epoch: &str) -> Option<Arc<OsvSnapshot>> {
    if !path.exists() {
        warn!(
            path = %path.display(),
            "OSV consensus enabled but snapshot file not found"
        );
        return None;
    }

    match std::fs::read_to_string(path) {
        Ok(json) => match OsvSnapshot::from_json_bytes(json.as_bytes()) {
            Ok(snapshot) => {
                if snapshot.epoch != expected_epoch {
                    warn!(
                        path = %path.display(),
                        file_epoch = %snapshot.epoch,
                        expected_epoch = %expected_epoch,
                        "OSV snapshot epoch does not match CREG_OSV_SNAPSHOT_EPOCH"
                    );
                    return None;
                }
                if snapshot.schema != SCHEMA_V1 {
                    warn!(
                        path = %path.display(),
                        schema = %snapshot.schema,
                        "Unsupported OSV snapshot schema (want {})",
                        SCHEMA_V1
                    );
                    return None;
                }
                info!(
                    path = %path.display(),
                    epoch = %snapshot.epoch,
                    entries = snapshot.entries.len(),
                    "Loaded pinned OSV snapshot"
                );
                Some(Arc::new(snapshot))
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Failed to parse OSV snapshot");
                None
            }
        },
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Failed to read OSV snapshot");
            None
        }
    }
}

fn loaded_snapshot() -> Option<Arc<OsvSnapshot>> {
    if !osv_consensus_enabled() {
        return None;
    }

    let path = snapshot_path();
    let epoch = bundle_epoch();
    let digest = file_digest(&path)?;

    let mut cache = snapshot_cache().lock().ok()?;

    let cache_valid = cache.snapshot.is_some()
        && cache.path == path
        && cache.epoch == epoch
        && cache.digest == digest;

    if cache_valid {
        return cache.snapshot.clone();
    }

    let snapshot = load_snapshot_from_disk(&path, &epoch);
    cache.path = path;
    cache.epoch = epoch;
    cache.digest = digest;
    cache.snapshot = snapshot.clone();
    snapshot
}

/// Lookup vulnerabilities for a package inside an already-loaded snapshot.
pub fn lookup_in_snapshot(snapshot: &OsvSnapshot, info: &PackageInfo) -> OsvResult {
    let key = OsvSnapshot::entry_key(info);
    match snapshot.entries.get(&key) {
        Some(vulnerabilities) => OsvResult {
            queried: true,
            vulnerabilities: vulnerabilities.clone(),
        },
        None => OsvResult::unavailable(),
    }
}

/// Local-only OSV lookup for consensus. Never performs HTTP.
pub fn lookup_pinned(info: &PackageInfo) -> OsvResult {
    if !osv_consensus_enabled() {
        return OsvResult::unavailable();
    }

    let Some(snapshot) = loaded_snapshot() else {
        return OsvResult::unavailable();
    };

    lookup_in_snapshot(snapshot.as_ref(), info)
}

/// Off-chain advisory lookup: pinned snapshot first, optional live OSV fallback.
pub fn lookup_advisory(info: &PackageInfo) -> OsvResult {
    if osv_consensus_enabled() {
        let pinned = lookup_pinned(info);
        if pinned.queried {
            if !pinned.vulnerabilities.is_empty() || !osv_live_fallback_enabled() {
                return pinned;
            }
        } else if !osv_live_fallback_enabled() {
            return pinned;
        }
    }

    if osv_live_fallback_enabled() || !osv_consensus_enabled() {
        return osv_client::query(info);
    }

    OsvResult::unavailable()
}

fn env_truthy(key: &str) -> bool {
    common::parse_env_bool(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::MutexGuard;

    fn env_test_lock() -> MutexGuard<'static, ()> {
        crate::env_test_lock()
    }

    fn sample_info() -> PackageInfo {
        PackageInfo {
            name: "lodash".into(),
            version: "4.17.20".into(),
            ecosystem: "npm".into(),
        }
    }

    #[test]
    fn snapshot_entry_key_uses_osv_ecosystem() {
        assert_eq!(OsvSnapshot::entry_key(&sample_info()), "npm:lodash@4.17.20");
    }

    #[test]
    fn parses_minimal_snapshot() {
        let json = r#"{
            "epoch": "test-epoch",
            "schema": "creg-osv-snapshot-v1",
            "entries": {
                "npm:lodash@4.17.20": [
                    { "id": "GHSA-test", "summary": "test vuln", "severity": "HIGH" }
                ]
            }
        }"#;
        let snapshot = OsvSnapshot::from_json_bytes(json.as_bytes()).unwrap();
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries["npm:lodash@4.17.20"][0].id, "GHSA-test");
    }

    #[test]
    fn bundle_epoch_off_when_consensus_disabled() {
        let _lock = env_test_lock();
        std::env::remove_var("CREG_OSV_CONSENSUS");
        assert_eq!(bundle_epoch(), "osv-off");
    }

    #[test]
    fn lookup_in_snapshot_returns_matching_vulns() {
        let json = r#"{
            "epoch": "test-epoch",
            "schema": "creg-osv-snapshot-v1",
            "entries": {
                "npm:lodash@4.17.20": [
                    { "id": "GHSA-test", "summary": "prototype pollution", "severity": "HIGH" }
                ]
            }
        }"#;
        let snapshot = OsvSnapshot::from_json_bytes(json.as_bytes()).unwrap();
        let result = lookup_in_snapshot(&snapshot, &sample_info());
        assert!(result.queried);
        assert_eq!(result.vulnerabilities.len(), 1);
        assert_eq!(result.vulnerabilities[0].id, "GHSA-test");
    }

    #[test]
    fn lookup_in_snapshot_empty_when_key_missing() {
        let snapshot = OsvSnapshot::from_json_bytes(
            br#"{"epoch":"e","schema":"creg-osv-snapshot-v1","entries":{}}"#,
        )
        .unwrap();
        let result = lookup_in_snapshot(&snapshot, &sample_info());
        assert!(!result.queried);
        assert!(result.vulnerabilities.is_empty());
    }

    #[test]
    fn lookup_pinned_unavailable_when_consensus_disabled() {
        let _lock = env_test_lock();
        std::env::remove_var("CREG_OSV_CONSENSUS");
        let result = lookup_pinned(&sample_info());
        assert!(!result.queried);
        assert!(result.vulnerabilities.is_empty());
    }

    #[test]
    fn lookup_pinned_reloads_when_file_mtime_changes() {
        let _lock = env_test_lock();
        reset_snapshot_cache_for_tests();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("osv.json");
        let epoch = "reload-test-epoch";

        let write_snapshot = |vuln_id: &str| {
            let json = format!(
                r#"{{
                    "epoch": "{epoch}",
                    "schema": "creg-osv-snapshot-v1",
                    "entries": {{
                        "npm:lodash@4.17.20": [
                            {{ "id": "{vuln_id}", "summary": "test", "severity": "HIGH" }}
                        ]
                    }}
                }}"#
            );
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(json.as_bytes()).unwrap();
            file.sync_all().unwrap();
        };

        std::env::set_var("CREG_OSV_CONSENSUS", "true");
        std::env::set_var("CREG_OSV_SNAPSHOT_PATH", path.to_string_lossy().as_ref());
        std::env::set_var("CREG_OSV_SNAPSHOT_EPOCH", epoch);
        std::env::remove_var("CREG_OSV_LIVE_FALLBACK");

        write_snapshot("GHSA-first");
        let first = lookup_pinned(&sample_info());
        assert!(first.queried, "expected pinned snapshot to load");
        assert_eq!(first.vulnerabilities[0].id, "GHSA-first");

        // File mtimes are often 1s granular on Windows.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write_snapshot("GHSA-second");
        let second = lookup_pinned(&sample_info());
        assert!(second.queried);
        assert_eq!(second.vulnerabilities[0].id, "GHSA-second");

        std::env::remove_var("CREG_OSV_CONSENSUS");
        std::env::remove_var("CREG_OSV_SNAPSHOT_PATH");
        std::env::remove_var("CREG_OSV_SNAPSHOT_EPOCH");
        reset_snapshot_cache_for_tests();
    }

    #[test]
    fn lookup_advisory_uses_pinned_without_live_fallback() {
        let _lock = env_test_lock();
        reset_snapshot_cache_for_tests();
        std::env::set_var("CREG_OSV_CONSENSUS", "true");
        std::env::remove_var("CREG_OSV_LIVE_FALLBACK");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("osv.json");
        let json = r#"{
            "epoch": "osv-epoch-0",
            "schema": "creg-osv-snapshot-v1",
            "entries": {
                "npm:lodash@4.17.20": [
                    { "id": "GHSA-pinned", "summary": "pinned only", "severity": "MEDIUM" }
                ]
            }
        }"#;
        std::fs::write(&path, json).unwrap();
        std::env::set_var("CREG_OSV_SNAPSHOT_PATH", path.to_string_lossy().as_ref());
        std::env::set_var("CREG_OSV_SNAPSHOT_EPOCH", "osv-epoch-0");

        let result = lookup_advisory(&sample_info());
        assert!(result.queried);
        assert_eq!(result.vulnerabilities[0].id, "GHSA-pinned");

        std::env::remove_var("CREG_OSV_CONSENSUS");
        std::env::remove_var("CREG_OSV_SNAPSHOT_PATH");
        std::env::remove_var("CREG_OSV_SNAPSHOT_EPOCH");
        reset_snapshot_cache_for_tests();
    }
}
