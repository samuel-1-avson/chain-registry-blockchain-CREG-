// crates/node/src/pending_pool.rs
// In-memory pending pool for packages that have been submitted but not yet
// gone through PBFT consensus. Packages here are installable with --unverified.

use chrono::{DateTime, Duration, Utc};
use common::PublishRequest;
use std::collections::HashMap;

pub struct PendingEntry {
    pub request: PublishRequest,
    pub received_at: DateTime<Utc>,
    /// How many times the validator pipeline has attempted this package.
    pub attempt_count: u32,
    /// Set to true once the validator pipeline has picked this up.
    pub in_progress: bool,
}

pub struct PendingPool {
    entries: HashMap<String, PendingEntry>, // canonical → entry
}

impl PendingPool {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Add a new submission.
    ///
    /// Returns `false` (and logs a warning) if the exact same content hash
    /// is already pending — prevents duplicate work and resubmission spam.
    /// If the same canonical exists but with a *different* content hash
    /// (i.e. a resubmission with changed content), it replaces the old entry.
    pub fn insert(&mut self, request: PublishRequest) -> bool {
        let key = request.id.canonical();

        if let Some(existing) = self.entries.get(&key) {
            if existing.request.content_hash == request.content_hash {
                tracing::warn!(
                    "[PendingPool] Duplicate submission ignored for {} (same content hash)",
                    key
                );
                return false;
            }
            tracing::info!(
                "[PendingPool] Replacing pending entry for {} (new content hash)",
                key
            );
        }

        tracing::info!("[PendingPool] Inserting package: {}", key);
        self.entries.insert(
            key,
            PendingEntry {
                request,
                received_at: Utc::now(),
                attempt_count: 0,
                in_progress: false,
            },
        );
        true
    }

    pub fn contains(&self, canonical: &str) -> bool {
        self.entries.contains_key(canonical)
    }

    pub fn get(&self, canonical: &str) -> Option<&PendingEntry> {
        self.entries.get(canonical)
    }

    /// Remove a package from the pool (after it's been verified or rejected).
    pub fn remove(&mut self, canonical: &str) -> Option<PendingEntry> {
        self.entries.remove(canonical)
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

        eligible
            .into_iter()
            .map(|e| {
                e.in_progress = true;
                e.attempt_count += 1;
                e.request.clone()
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn all_canonicals(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }
}
