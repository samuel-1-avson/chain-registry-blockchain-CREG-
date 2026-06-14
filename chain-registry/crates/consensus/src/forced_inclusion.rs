// crates/consensus/src/forced_inclusion.rs
// Censorship-resistance mechanism: tracks pending transactions and
// flags proposers that repeatedly omit eligible transactions.

use common::Transaction;
use std::collections::HashMap;
use std::time::Instant;

/// After this many blocks without inclusion, a submitted transaction
/// becomes "forced" — proposers who omit it are flagged for GRIEFING.
/// Overridable via `CREG_FORCED_INCLUSION_DELAY` environment variable.
fn forced_inclusion_block_delay() -> u64 {
    std::env::var("CREG_FORCED_INCLUSION_DELAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5)
}

/// A submitted transaction awaiting inclusion in a block.
#[derive(Debug, Clone)]
pub struct PendingTransaction {
    /// Hash of the serialised transaction.
    pub tx_hash: String,
    /// Full transaction payload awaiting inclusion.
    pub transaction: Transaction,
    /// Block height at which the transaction was first seen by the tracker.
    pub submitted_at_height: u64,
    /// Wall-clock time added (for metrics/observability only).
    pub submitted_at: Instant,
}

/// Tracks pending transactions and detects censorship.
pub struct ForcedInclusionTracker {
    /// tx_hash → pending tx metadata
    pending: HashMap<String, PendingTransaction>,
}

/// Evidence that a proposer censored a forced-inclusion transaction.
#[derive(Debug, Clone)]
pub struct CensorshipEvidence {
    pub proposer_id: String,
    pub block_height: u64,
    pub omitted_tx_hashes: Vec<String>,
}

impl ForcedInclusionTracker {
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }

    /// Record a new transaction that should eventually be included.
    pub fn submit(&mut self, tx: Transaction, current_height: u64) {
        let tx_hash = common::transaction_hash(&tx);
        self.pending
            .entry(tx_hash.clone())
            .or_insert(PendingTransaction {
                tx_hash,
                transaction: tx,
                submitted_at_height: current_height,
                submitted_at: Instant::now(),
            });
    }

    /// Return forced-inclusion transaction payloads at `current_height`.
    pub fn forced_transaction_payloads(&self, current_height: u64) -> Vec<&Transaction> {
        self.forced_transactions(current_height)
            .into_iter()
            .map(|pending| &pending.transaction)
            .collect()
    }

    /// Mark transactions as included after a block is finalised.
    pub fn mark_included(&mut self, tx_hashes: &[String]) {
        for hash in tx_hashes {
            self.pending.remove(hash);
        }
    }

    /// Return all transactions that have exceeded the forced-inclusion
    /// deadline at the given block height.
    pub fn forced_transactions(&self, current_height: u64) -> Vec<&PendingTransaction> {
        self.pending
            .values()
            .filter(|tx| {
                current_height.saturating_sub(tx.submitted_at_height)
                    >= forced_inclusion_block_delay()
            })
            .collect()
    }

    /// Check a newly-proposed block for censorship of forced-inclusion
    /// transactions.  Returns evidence if any were omitted.
    pub fn check_block(
        &self,
        proposer_id: &str,
        block_height: u64,
        included_tx_hashes: &[String],
    ) -> Option<CensorshipEvidence> {
        let forced = self.forced_transactions(block_height);
        if forced.is_empty() {
            return None;
        }

        let included_set: std::collections::HashSet<&String> = included_tx_hashes.iter().collect();

        let omitted: Vec<String> = forced
            .iter()
            .filter(|tx| !included_set.contains(&tx.tx_hash))
            .map(|tx| tx.tx_hash.clone())
            .collect();

        if omitted.is_empty() {
            return None;
        }

        tracing::warn!(
            "[CENSORSHIP] Proposer {} at height {} omitted {} forced-inclusion txs",
            proposer_id,
            block_height,
            omitted.len()
        );

        Some(CensorshipEvidence {
            proposer_id: proposer_id.to_string(),
            block_height,
            omitted_tx_hashes: omitted,
        })
    }

    /// Number of currently pending (un-included) transactions.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tx(hash_seed: &str) -> Transaction {
        use chrono::Utc;
        use common::{ChainRecord, PackageId, PackageStatus};

        Transaction::Publish(ChainRecord {
            id: PackageId::new("npm", hash_seed, "1.0.0"),
            content_hash: hash_seed.into(),
            ipfs_cid: "bafy".into(),
            publisher_pubkey: "pk".into(),
            block_hash: "0".repeat(64),
            published_at: Utc::now(),
            validator_signatures: vec![],
            status: PackageStatus::Verified,
            ..Default::default()
        })
    }

    #[test]
    fn forced_after_delay() {
        let mut tracker = ForcedInclusionTracker::new();
        tracker.submit(sample_tx("tx1"), 10);
        assert!(tracker.forced_transactions(12).is_empty());
        assert_eq!(tracker.forced_transactions(15).len(), 1);
    }

    #[test]
    fn included_transactions_removed() {
        let mut tracker = ForcedInclusionTracker::new();
        let tx1 = sample_tx("tx1");
        let tx2 = sample_tx("tx2");
        let tx1_hash = common::transaction_hash(&tx1);
        tracker.submit(tx1, 10);
        tracker.submit(tx2, 10);
        tracker.mark_included(&[tx1_hash]);
        assert_eq!(tracker.pending_count(), 1);
    }

    #[test]
    fn censorship_detected() {
        let mut tracker = ForcedInclusionTracker::new();
        let tx1 = sample_tx("tx1");
        let tx2 = sample_tx("tx2");
        let tx1_hash = common::transaction_hash(&tx1);
        let tx2_hash = common::transaction_hash(&tx2);
        tracker.submit(tx1, 10);
        tracker.submit(tx2, 10);

        let evidence = tracker.check_block("proposer_a", 16, &[tx1_hash]);
        assert!(evidence.is_some());
        let ev = evidence.unwrap();
        assert_eq!(ev.omitted_tx_hashes, vec![tx2_hash]);
    }

    #[test]
    fn no_censorship_when_all_included() {
        let mut tracker = ForcedInclusionTracker::new();
        let tx1 = sample_tx("tx1");
        let tx1_hash = common::transaction_hash(&tx1);
        tracker.submit(tx1, 10);
        let evidence = tracker.check_block("proposer_a", 16, &[tx1_hash]);
        assert!(evidence.is_none());
    }
}
