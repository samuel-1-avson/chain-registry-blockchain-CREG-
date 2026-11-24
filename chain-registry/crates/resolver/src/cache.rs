// crates/resolver/src/cache.rs
// Bounded JSON proof/verdict cache replacing unmaintained `sled` (ECA-13,
// ECA-15, ECA-33, ADR-019 Increment 3).
//
// Positive and pending verdicts cache for `install_gate::INSTALL_CACHE_MAX_AGE`
// (5 minutes); revocations cache for 48 hours. `Unknown` is never cached.
// The anti-rollback journal in `freshness.rs`, not this cache, is what keeps
// certified revocations and quarantines sticky.

use crate::file_store::{atomic_write, quarantine_corrupt_file, FileLockGuard};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use common::{DisplayText, PackageId, TrustVerdict, VerdictSource, VerdictStatus};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub const CACHE_PATH_ENV: &str = "CREG_CACHE_PATH";
pub const CACHE_SCHEMA_VERSION: u16 = 2;
pub const MAX_CACHE_ENTRIES: usize = 5_000;
pub const MAX_CACHE_FILE_BYTES: u64 = 32 * 1024 * 1024;

static PROCESS_CACHE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn process_cache_guard() -> std::sync::MutexGuard<'static, ()> {
    PROCESS_CACHE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn default_schema_version() -> u16 {
    CACHE_SCHEMA_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    verdict: TrustVerdict,
    expires_at: DateTime<Utc>,
    #[serde(default)]
    last_accessed_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    #[serde(default = "default_schema_version")]
    schema_version: u16,
    #[serde(default)]
    entries: BTreeMap<String, CacheEntry>,
}

impl Default for CacheFile {
    fn default() -> Self {
        Self {
            schema_version: CACHE_SCHEMA_VERSION,
            entries: BTreeMap::new(),
        }
    }
}

/// Bounded, crash-safe JSON store for cached [`TrustVerdict`] entries.
pub struct CacheStore {
    path: PathBuf,
    max_entries: usize,
}

impl CacheStore {
    pub fn open_path(path: PathBuf) -> Self {
        Self::open_path_with_limit(path, MAX_CACHE_ENTRIES)
    }

    pub fn open_path_with_limit(path: PathBuf, max_entries: usize) -> Self {
        Self {
            path,
            max_entries: max_entries.max(1),
        }
    }

    pub fn open_default() -> Result<Self> {
        if let Ok(path) = std::env::var(CACHE_PATH_ENV) {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                let target = PathBuf::from(trimmed);
                if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("create cache dir {}", parent.display()))?;
                }
                return Ok(Self::open_path(target));
            }
        }
        let dir = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("chain-registry");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create cache dir {}", dir.display()))?;
        Ok(Self::open_path(dir.join("verdict-cache.json")))
    }

    fn load_locked(&self) -> Result<CacheFile> {
        let meta = match std::fs::metadata(&self.path) {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CacheFile::default());
            }
            Err(err) => {
                return Err(err).with_context(|| format!("stat cache {}", self.path.display()));
            }
        };
        if meta.len() > MAX_CACHE_FILE_BYTES {
            quarantine_corrupt_file(&self.path);
            bail!(
                "verdict cache {} exceeded max size ({} bytes); quarantined",
                self.path.display(),
                MAX_CACHE_FILE_BYTES
            );
        }
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CacheFile::default());
            }
            Err(err) => {
                return Err(err).with_context(|| format!("read cache {}", self.path.display()));
            }
        };
        let file: CacheFile = match serde_json::from_slice(&bytes) {
            Ok(file) => file,
            Err(err) => {
                quarantine_corrupt_file(&self.path);
                return Err(err).context("decode verdict cache; quarantined corrupt file");
            }
        };
        if file.schema_version == 0 || file.schema_version > CACHE_SCHEMA_VERSION {
            quarantine_corrupt_file(&self.path);
            bail!(
                "unsupported verdict cache schema version {}; quarantined",
                file.schema_version
            );
        }
        Ok(file)
    }

    fn save_locked(&self, file: &CacheFile) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(file).context("encode verdict cache")?;
        atomic_write(&self.path, &bytes)
    }

    /// Retrieve a cached verdict. Returns `None` if missing or expired.
    pub fn get(&self, id: &PackageId) -> Result<Option<TrustVerdict>> {
        self.get_at(id, Utc::now())
    }

    pub fn get_at(&self, id: &PackageId, now: DateTime<Utc>) -> Result<Option<TrustVerdict>> {
        let _process_guard = process_cache_guard();
        let _file_guard = FileLockGuard::acquire(&self.path)?;
        let mut file = self.load_locked()?;
        let key = id.canonical();
        let Some(entry) = file.entries.get_mut(&key) else {
            return Ok(None);
        };
        if now > entry.expires_at {
            file.entries.remove(&key);
            self.save_locked(&file)?;
            return Ok(None);
        }
        entry.last_accessed_ms = now.timestamp_millis();
        let mut verdict = entry.verdict.clone();
        verdict.source = VerdictSource::Cache {
            expires_at: entry.expires_at,
        };
        self.save_locked(&file)?;
        Ok(Some(verdict))
    }

    /// Store a verdict with an appropriate TTL, enforcing the LRU entry limit.
    pub fn set(&self, id: &PackageId, verdict: &TrustVerdict) -> Result<()> {
        self.set_at(id, verdict, Utc::now())
    }

    pub fn set_at(&self, id: &PackageId, verdict: &TrustVerdict, now: DateTime<Utc>) -> Result<()> {
        let _process_guard = process_cache_guard();
        let _file_guard = FileLockGuard::acquire(&self.path)?;
        let mut file = self.load_locked()?;
        let key = id.canonical();

        let ttl = match &verdict.status {
            VerdictStatus::Verified { .. }
            | VerdictStatus::Quarantined { .. }
            | VerdictStatus::Unverified => crate::install_gate::INSTALL_CACHE_MAX_AGE,
            VerdictStatus::Revoked { .. } => Duration::hours(48),
            // Never cache "unknown": a transient miss must not mask a later
            // publication or revocation. Drop any older entry instead.
            VerdictStatus::Unknown => {
                if file.entries.remove(&key).is_some() {
                    self.save_locked(&file)?;
                }
                return Ok(());
            }
        };

        file.entries
            .retain(|_, existing| now <= existing.expires_at);
        file.entries.insert(
            key,
            CacheEntry {
                verdict: verdict.clone(),
                expires_at: now + ttl,
                last_accessed_ms: now.timestamp_millis(),
            },
        );
        while file.entries.len() > self.max_entries {
            let Some(oldest_key) = file
                .entries
                .iter()
                .min_by(|(key_a, entry_a), (key_b, entry_b)| {
                    entry_a
                        .last_accessed_ms
                        .cmp(&entry_b.last_accessed_ms)
                        .then_with(|| key_a.cmp(key_b))
                })
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            file.entries.remove(&oldest_key);
        }
        self.save_locked(&file)
    }

    pub fn clear(&self) -> Result<()> {
        let _process_guard = process_cache_guard();
        let _file_guard = FileLockGuard::acquire(&self.path)?;
        self.save_locked(&CacheFile::default())
    }

    pub fn entries(&self) -> Result<Vec<(String, TrustVerdict, DateTime<Utc>)>> {
        let _process_guard = process_cache_guard();
        let _file_guard = FileLockGuard::acquire(&self.path)?;
        let file = self.load_locked()?;
        Ok(file
            .entries
            .into_iter()
            .map(|(key, entry)| (key, entry.verdict, entry.expires_at))
            .collect())
    }
}

/// Retrieve a cached verdict from the default store. Returns `None` if missing or expired.
pub fn get(id: &PackageId) -> Result<Option<TrustVerdict>> {
    CacheStore::open_default()?.get(id)
}

/// Store a verdict in the default store with an appropriate TTL.
pub fn set(id: &PackageId, verdict: &TrustVerdict) -> Result<()> {
    CacheStore::open_default()?.set(id, verdict)
}

/// Clear all entries from the local cache and the install freshness journal.
pub fn clear() -> Result<()> {
    CacheStore::open_default()?.clear()?;
    crate::freshness::clear_default_store()?;
    Ok(())
}

/// Print a summary of all cached entries.
pub fn print_entries() -> Result<()> {
    let entries = CacheStore::open_default()?.entries()?;
    println!("{:<50} {:<12} Expires", "Package", "Status");
    println!("{}", "-".repeat(90));
    for (_, verdict, expires_at) in entries {
        println!(
            "{:<50} {:<12} {}",
            DisplayText::for_terminal(&verdict.package.canonical()),
            verdict.status.label(),
            expires_at.format("%Y-%m-%d %H:%M UTC")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn pkg(name: &str) -> PackageId {
        PackageId::new("npm", name, "1.0.0")
    }

    fn make_verdict(name: &str, status: VerdictStatus, resolved_at: DateTime<Utc>) -> TrustVerdict {
        TrustVerdict {
            package: pkg(name),
            status,
            resolved_at,
            source: VerdictSource::Chain {
                node_url: "http://127.0.0.1:8080".into(),
            },
            deterministic_risk: None,
            evidence_phase: None,
            block_phase: None,
            chain_status: None,
            publisher: None,
        }
    }

    fn verified_status() -> VerdictStatus {
        VerdictStatus::Verified {
            block_hash: "ab".repeat(32),
            content_hash: "cd".repeat(32),
            ipfs_cid: "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".into(),
            findings: vec![],
        }
    }

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 23, 12, 0, 0).unwrap()
    }

    #[test]
    fn json_cache_round_trips_and_evicts_expired() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open_path(dir.path().join("verdict-cache.json"));
        let now = t0();
        let id = pkg("left-pad");
        store
            .set_at(&id, &make_verdict("left-pad", verified_status(), now), now)
            .unwrap();

        let hit = store
            .get_at(&id, now + Duration::minutes(4))
            .unwrap()
            .expect("cache hit");
        assert!(matches!(hit.source, VerdictSource::Cache { .. }));
        assert!(hit.status.is_safe());

        // After 5-minute TTL expires, get_at evicts and returns None.
        assert!(store
            .get_at(&id, now + Duration::minutes(6))
            .unwrap()
            .is_none());
        assert!(store.entries().unwrap().is_empty());
    }

    #[test]
    fn json_cache_never_caches_unknown_and_removes_prior() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open_path(dir.path().join("verdict-cache.json"));
        let now = t0();
        let id = pkg("express");
        store
            .set_at(&id, &make_verdict("express", verified_status(), now), now)
            .unwrap();
        assert!(store.get_at(&id, now).unwrap().is_some());

        store
            .set_at(
                &id,
                &make_verdict("express", VerdictStatus::Unknown, now),
                now + Duration::seconds(1),
            )
            .unwrap();
        assert!(store
            .get_at(&id, now + Duration::seconds(1))
            .unwrap()
            .is_none());
        assert!(store.entries().unwrap().is_empty());
    }

    #[test]
    fn json_cache_enforces_lru_bound() {
        let dir = tempfile::tempdir().unwrap();
        let store = CacheStore::open_path_with_limit(dir.path().join("verdict-cache.json"), 3);
        let base = t0();

        for (offset, name) in [(0, "pkg-a"), (1, "pkg-b"), (2, "pkg-c")] {
            let at = base + Duration::seconds(offset);
            store
                .set_at(&pkg(name), &make_verdict(name, verified_status(), at), at)
                .unwrap();
        }
        // Touch pkg-a so pkg-b becomes least recently used.
        assert!(store
            .get_at(&pkg("pkg-a"), base + Duration::seconds(3))
            .unwrap()
            .is_some());
        let at_d = base + Duration::seconds(4);
        store
            .set_at(
                &pkg("pkg-d"),
                &make_verdict("pkg-d", verified_status(), at_d),
                at_d,
            )
            .unwrap();

        let keys: Vec<String> = store
            .entries()
            .unwrap()
            .into_iter()
            .map(|(k, _, _)| k)
            .collect();
        assert_eq!(
            keys,
            vec!["npm:pkg-a@1.0.0", "npm:pkg-c@1.0.0", "npm:pkg-d@1.0.0"]
        );
    }

    #[test]
    fn json_cache_quarantines_corruption_and_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("verdict-cache.json");
        std::fs::write(&path, b"{not valid json").unwrap();
        let store = CacheStore::open_path(path.clone());

        assert!(store.get(&pkg("left-pad")).is_err());
        assert!(!path.exists(), "corrupt file must be moved aside");
        let quarantined: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("verdict-cache.json.corrupt-")
            })
            .collect();
        assert_eq!(quarantined.len(), 1);

        // Unsupported future schema version is also quarantined and fails closed.
        std::fs::write(
            &path,
            format!(
                r#"{{"schema_version":{},"entries":{{}}}}"#,
                CACHE_SCHEMA_VERSION + 1
            ),
        )
        .unwrap();
        assert!(store.get(&pkg("left-pad")).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn json_cache_handles_concurrent_writers_under_file_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("verdict-cache.json");
        let now = t0();
        let handles: Vec<_> = (0..8)
            .map(|worker| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let store = CacheStore::open_path(path);
                    for i in 0..5 {
                        let name = format!("pkg-{worker}-{i}");
                        let id = pkg(&name);
                        let verdict = make_verdict(&name, verified_status(), now);
                        store.set_at(&id, &verdict, now).unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let store = CacheStore::open_path(path);
        assert_eq!(store.entries().unwrap().len(), 40);
    }
}
