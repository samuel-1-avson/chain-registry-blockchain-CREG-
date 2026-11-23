// crates/resolver/src/cache.rs
// Local sled-backed KV store for TrustVerdicts.
// Positive and pending verdicts cache for the install freshness window
// (ECA-13: no 24h unauthenticated reuse); revocations cache for 48h. The
// anti-rollback journal in `freshness.rs`, not this cache, is what keeps a
// revocation or quarantine sticky. ADR-019 Inc 3 replaces `sled` (ECA-15).

use anyhow::Result;
use chrono::{Duration, Utc};
use common::{PackageId, TrustVerdict, VerdictSource, VerdictStatus};
use sled::Db;
use std::sync::OnceLock;

static DB: OnceLock<Db> = OnceLock::new();

fn db() -> Result<&'static Db> {
    DB.get_or_init(|| {
        let path = dirs::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("chain-registry")
            .join("verdict-cache");
        sled::open(path).expect("Failed to open verdict cache")
    });
    DB.get()
        .ok_or_else(|| anyhow::anyhow!("Verdict cache not initialized"))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    verdict: TrustVerdict,
    expires_at: chrono::DateTime<Utc>,
}

/// Retrieve a cached verdict. Returns None if missing or expired.
pub fn get(id: &PackageId) -> Result<Option<TrustVerdict>> {
    let db = db()?;
    let key = id.canonical();
    match db.get(key.as_bytes())? {
        None => Ok(None),
        Some(bytes) => {
            let entry: CacheEntry = serde_json::from_slice(&bytes)?;
            if Utc::now() > entry.expires_at {
                // Stale — evict and signal a miss.
                db.remove(key.as_bytes())?;
                return Ok(None);
            }
            // Patch source to indicate this is a cache hit.
            let mut v = entry.verdict;
            v.source = VerdictSource::Cache {
                expires_at: entry.expires_at,
            };
            Ok(Some(v))
        }
    }
}

/// Store a verdict with an appropriate TTL.
pub fn set(id: &PackageId, verdict: &TrustVerdict) -> Result<()> {
    let ttl = match &verdict.status {
        VerdictStatus::Verified { .. } => crate::install_gate::INSTALL_CACHE_MAX_AGE,
        VerdictStatus::Revoked { .. } => Duration::hours(48), // cache revocations longer
        VerdictStatus::Quarantined { .. } => crate::install_gate::INSTALL_CACHE_MAX_AGE,
        VerdictStatus::Unverified => crate::install_gate::INSTALL_CACHE_MAX_AGE,
        // Never cache "unknown": a transient miss must not mask a later
        // publication or revocation. Drop any older entry instead.
        VerdictStatus::Unknown => {
            db()?.remove(id.canonical().as_bytes())?;
            return Ok(());
        }
    };
    let expires_at = Utc::now() + ttl;
    let entry = CacheEntry {
        verdict: verdict.clone(),
        expires_at,
    };
    let bytes = serde_json::to_vec(&entry)?;
    db()?.insert(id.canonical().as_bytes(), bytes)?;
    Ok(())
}

/// Clear all entries from the local cache and the install freshness journal.
pub fn clear() -> Result<()> {
    db()?.clear()?;
    crate::freshness::clear_default_store()?;
    Ok(())
}

/// Print a summary of all cached entries.
pub fn print_entries() -> Result<()> {
    let db = db()?;
    println!("{:<50} {:<12} Expires", "Package", "Status");
    println!("{}", "-".repeat(90));
    for item in db.iter() {
        let (_, v) = item?;
        if let Ok(entry) = serde_json::from_slice::<CacheEntry>(&v) {
            println!(
                "{:<50} {:<12} {}",
                entry.verdict.package.canonical(),
                entry.verdict.status.label(),
                entry.expires_at.format("%Y-%m-%d %H:%M UTC")
            );
        }
    }
    Ok(())
}
