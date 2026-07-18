// crates/node/src/pending_pool.rs
// Pending pool for packages submitted but not yet finalized through PBFT.
// Persisted to `<data_dir>/pending_pool.json` so restarts do not drop submissions.

use chrono::{DateTime, Duration, Utc};
use common::PublishRequest;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct PendingEntry {
    pub request: PublishRequest,
    pub received_at: DateTime<Utc>,
    /// How many times the validator pipeline has attempted this package.
    pub attempt_count: u32,
    /// Set to true once the validator pipeline has picked this up.
    pub in_progress: bool,

    // === HYBRID ANALYSIS COMMITTEE (mandatory + voluntary support) ===
    /// Populated at admission. Contains the deterministically selected mandatory committee.
    pub analysis_committee: Option<common::AnalysisCommittee>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedEntry {
    request: PublishRequest,
    received_at: DateTime<Utc>,
    attempt_count: u32,
    in_progress: bool,
    #[serde(default)]
    analysis_committee: Option<common::AnalysisCommittee>,
}

pub struct PendingPool {
    entries: HashMap<String, PendingEntry>,
    path: Option<PathBuf>,
}

impl PendingPool {
    /// In-memory pool (tests only).
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            path: None,
        }
    }

    /// Load from disk when present; new submissions are persisted after each mutation.
    pub fn open(data_dir: &Path) -> Self {
        let path = data_dir.join("pending_pool.json");
        let mut pool = Self {
            entries: HashMap::new(),
            path: Some(path.clone()),
        };

        if !path.exists() {
            return pool;
        }

        match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<HashMap<String, PersistedEntry>>(&raw) {
                Ok(map) => {
                    let count = map.len();
                    for (key, persisted) in map {
                        pool.entries.insert(
                            key,
                            PendingEntry {
                                request: persisted.request,
                                received_at: persisted.received_at,
                                attempt_count: persisted.attempt_count,
                                // Stuck in_progress entries become eligible again after restart.
                                in_progress: false,
                                analysis_committee: persisted.analysis_committee,
                            },
                        );
                    }
                    tracing::info!(
                        "[PendingPool] Restored {} pending package(s) from {}",
                        count,
                        path.display()
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        "[PendingPool] Ignoring corrupt pending pool at {}: {}",
                        path.display(),
                        error
                    );
                }
            },
            Err(error) => {
                tracing::warn!(
                    "[PendingPool] Could not read pending pool at {}: {}",
                    path.display(),
                    error
                );
            }
        }

        pool
    }

    fn persist_result(&self) -> anyhow::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };

        let snapshot: HashMap<String, PersistedEntry> = self
            .entries
            .iter()
            .map(|(key, entry)| {
                (
                    key.clone(),
                    PersistedEntry {
                        request: entry.request.clone(),
                        received_at: entry.received_at,
                        attempt_count: entry.attempt_count,
                        in_progress: entry.in_progress,
                        analysis_committee: entry.analysis_committee.clone(),
                    },
                )
            })
            .collect();

        let encoded = serde_json::to_string_pretty(&snapshot)?;

        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &encoded)?;
        if let Err(error) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(error.into());
        }
        Ok(())
    }

    fn persist(&self) {
        if let Err(error) = self.persist_result() {
            tracing::warn!("[PendingPool] Failed to persist pending pool: {}", error);
        }
    }

    /// Add a new submission.
    ///
    /// Returns `false` (and logs a warning) if the exact same content hash
    /// is already pending — prevents duplicate work and resubmission spam.
    /// If the same canonical exists but with a *different* content hash
    /// (i.e. a resubmission with changed content), it replaces the old entry.
    pub fn insert(&mut self, request: PublishRequest) -> bool {
        match self.insert_durable(request) {
            Ok(inserted) => inserted,
            Err(error) => {
                tracing::warn!("[PendingPool] Durable insert failed: {}", error);
                false
            }
        }
    }

    pub fn insert_durable(&mut self, request: PublishRequest) -> anyhow::Result<bool> {
        let key = request.id.canonical();

        if let Some(existing) = self.entries.get(&key) {
            if existing.request.content_hash == request.content_hash {
                tracing::warn!(
                    "[PendingPool] Duplicate submission ignored for {} (same content hash)",
                    key
                );
                return Ok(false);
            }
            tracing::info!(
                "[PendingPool] Replacing pending entry for {} (new content hash)",
                key
            );
        }

        tracing::info!("[PendingPool] Inserting package: {}", key);
        let previous = self.entries.insert(
            key.clone(),
            PendingEntry {
                request,
                received_at: Utc::now(),
                attempt_count: 0,
                in_progress: false,
                analysis_committee: None, // set later by admission or first processor
            },
        );
        if let Err(error) = self.persist_result() {
            match previous {
                Some(previous) => {
                    self.entries.insert(key, previous);
                }
                None => {
                    self.entries.remove(&key);
                }
            }
            return Err(error);
        }
        Ok(true)
    }

    /// Attach or update the analysis committee for a package (called at admission or first processing).
    pub fn set_analysis_committee(
        &mut self,
        canonical: &str,
        committee: common::AnalysisCommittee,
    ) {
        if let Some(entry) = self.entries.get_mut(canonical) {
            entry.analysis_committee = Some(committee);
            self.persist();
        }
    }

    pub fn set_analysis_committee_durable(
        &mut self,
        canonical: &str,
        committee: common::AnalysisCommittee,
    ) -> anyhow::Result<()> {
        let previous = self
            .entries
            .get(canonical)
            .and_then(|entry| entry.analysis_committee.clone());
        let Some(entry) = self.entries.get_mut(canonical) else {
            anyhow::bail!("pending package disappeared before committee persistence");
        };
        entry.analysis_committee = Some(committee);
        if let Err(error) = self.persist_result() {
            if let Some(entry) = self.entries.get_mut(canonical) {
                entry.analysis_committee = previous;
            }
            return Err(error);
        }
        Ok(())
    }

    pub fn get_committee(&self, canonical: &str) -> Option<&common::AnalysisCommittee> {
        self.entries
            .get(canonical)
            .and_then(|e| e.analysis_committee.as_ref())
    }

    pub fn contains(&self, canonical: &str) -> bool {
        self.entries.contains_key(canonical)
    }

    pub fn get(&self, canonical: &str) -> Option<&PendingEntry> {
        self.entries.get(canonical)
    }

    /// Return the Ed25519 publisher key that currently controls a package
    /// namespace in the pending pool, if any.  Namespace ownership is a
    /// package-level property, not a per-version race; admission uses this
    /// helper before accepting a first version that has not reached the
    /// canonical chain yet.
    pub fn publisher_for_prefix(&self, prefix: &str) -> Option<String> {
        self.entries
            .values()
            .filter(|entry| entry.request.id.canonical().starts_with(prefix))
            .map(|entry| entry.request.publisher_pubkey.clone())
            .find(|publisher| !publisher.trim().is_empty())
    }

    /// Remove a package from the pool (after it's been verified or rejected).
    pub fn remove(&mut self, canonical: &str) -> Option<PendingEntry> {
        let removed = self.entries.remove(canonical);
        if removed.is_some() {
            self.persist();
        }
        removed
    }

    /// Returns entries ready for validation (not in progress, or stuck > 5 min).
    pub fn ready_for_validation(&mut self) -> Vec<PublishRequest> {
        let cutoff = Utc::now() - Duration::minutes(5);
        let eligible: Vec<_> = self
            .entries
            .values_mut()
            .filter(|e| !e.in_progress || e.received_at < cutoff)
            .collect();

        if !eligible.is_empty() {
            tracing::info!(
                "[PendingPool] Found {} eligible packages for validation",
                eligible.len()
            );
        }

        let requests: Vec<PublishRequest> = eligible
            .into_iter()
            .map(|e| {
                e.in_progress = true;
                e.attempt_count += 1;
                e.request.clone()
            })
            .collect();

        if !requests.is_empty() {
            self.persist();
        }

        requests
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn all_canonicals(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::package::{PackageId, PackageManifest};
    use tempfile::tempdir;

    fn sample_request(name: &str, hash: &str) -> PublishRequest {
        PublishRequest {
            id: PackageId::new("npm", name, "1.0.0"),
            content_hash: hash.into(),
            ipfs_cid: "QmTest".into(),
            publisher_address: "0x38371A715Bd36142766EB026e61de061b45C9b00".into(),
            publisher_pubkey: "0437e4adac481519cd6ae66907294c40cfcbf0bdeadd47806f6233be4bd5f82d"
                .into(),
            signature: "sig".into(),
            manifest: PackageManifest::default(),
            submitted_at: Utc::now(),
            shielded: false,
            key_bundle: None,
            pgp_signature: None,
            pgp_public_key: None,
            threshold: 0,
            publisher_pubkeys: vec![],
            signatures: vec![],
            ..Default::default()
        }
    }

    #[test]
    fn open_persists_and_reloads_after_restart() {
        let dir = tempdir().unwrap();
        let canonical = "npm:pkg@1.0.0";

        {
            let mut pool = PendingPool::open(dir.path());
            assert!(pool.insert(sample_request("pkg", "hash-a")));
            assert_eq!(pool.len(), 1);
        }

        let pool = PendingPool::open(dir.path());
        assert!(pool.contains(canonical));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn remove_clears_persisted_entry() {
        let dir = tempdir().unwrap();
        let canonical = "npm:pkg@1.0.0";

        let mut pool = PendingPool::open(dir.path());
        pool.insert(sample_request("pkg", "hash-a"));
        pool.remove(canonical);

        let pool = PendingPool::open(dir.path());
        assert!(!pool.contains(canonical));
    }
}
