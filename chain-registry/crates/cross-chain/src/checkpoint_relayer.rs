//! Cross-Chain State Root Checkpoint Relayer
//!
//! Packages finalized PBFT block state roots and Merkle transparency roots
//! into verifiable L1 checkpoint batches for periodic anchoring on Ethereum Sepolia / Mainnet.

use anyhow::{bail, Result};
use common::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// An L1 checkpoint batch containing state root commitments and multi-signatures
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L1CheckpointBatch {
    /// Finalized block height anchored
    pub block_height: u64,
    /// Block hash of the finalized block
    pub block_hash: String,
    /// State root of the package namespace ledger
    pub state_root: String,
    /// Merkle transparency tree root representing package publication history
    pub transparency_root: String,
    /// Number of package updates/verifications in this batch
    pub package_count: usize,
    /// Aggregate BLS multi-signature over the state root
    pub bls_aggregate_signature: Option<String>,
    /// Bitmask of participating validators
    pub participant_bitmask: Option<Vec<u8>>,
    /// UNIX timestamp of batch creation
    pub timestamp: u64,
}

impl L1CheckpointBatch {
    /// Compute the deterministic digest of this checkpoint batch
    pub fn batch_digest(&self) -> String {
        let payload = format!(
            "creg-l1-checkpoint-v1:{}:{}:{}:{}",
            self.block_height, self.block_hash, self.state_root, self.transparency_root
        );
        sha256_hex(payload.as_bytes())
    }

    /// Format Ethereum ABI calldata string for `CrossChainRegistry::anchorStateRoot`
    pub fn format_l1_calldata(&self) -> String {
        format!(
            "anchorStateRoot(uint64 blockHeight={}, bytes32 stateRoot=0x{}, bytes32 transparencyRoot=0x{})",
            self.block_height,
            self.state_root.trim_start_matches("0x"),
            self.transparency_root.trim_start_matches("0x")
        )
    }
}

/// In-memory bounded queue for checkpoint relaying to L1
pub struct CheckpointRelayQueue {
    max_capacity: usize,
    queue: VecDeque<L1CheckpointBatch>,
    last_anchored_height: u64,
}

impl CheckpointRelayQueue {
    pub fn new(max_capacity: usize) -> Self {
        Self {
            max_capacity: max_capacity.max(10),
            queue: VecDeque::new(),
            last_anchored_height: 0,
        }
    }

    /// Enqueue a newly finalized checkpoint batch
    pub fn enqueue(&mut self, batch: L1CheckpointBatch) -> Result<()> {
        if batch.block_height <= self.last_anchored_height {
            bail!(
                "Batch height {} already anchored (current height {})",
                batch.block_height,
                self.last_anchored_height
            );
        }

        if self.queue.len() >= self.max_capacity {
            self.queue.pop_front(); // Evict oldest unanchored batch to prevent unbounded growth
        }

        self.queue.push_back(batch);
        Ok(())
    }

    /// Pop the next batch to submit to Ethereum L1
    pub fn pop_next(&mut self) -> Option<L1CheckpointBatch> {
        self.queue.pop_front()
    }

    /// Mark a block height as successfully confirmed on L1
    pub fn mark_confirmed(&mut self, height: u64) {
        if height > self.last_anchored_height {
            self.last_anchored_height = height;
        }
        // Remove any queued items with height <= confirmed height
        self.queue.retain(|b| b.block_height > height);
    }

    pub fn last_anchored_height(&self) -> u64 {
        self.last_anchored_height
    }

    pub fn pending_count(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_batch_digest_and_calldata() {
        let batch = L1CheckpointBatch {
            block_height: 1000,
            block_hash: "0x1111222233334444555566667777888899990000111122223333444455556666".into(),
            state_root: "0xabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdef"
                .into(),
            transparency_root: "0x9876543210987654321098765432109876543210987654321098765432109876"
                .into(),
            package_count: 42,
            bls_aggregate_signature: Some("0xfeedbeef".into()),
            participant_bitmask: Some(vec![0xFF, 0x0F]),
            timestamp: 1724760000,
        };

        let digest = batch.batch_digest();
        assert_eq!(digest.len(), 64);

        let calldata = batch.format_l1_calldata();
        assert!(calldata.contains("blockHeight=1000"));
        assert!(calldata.contains("stateRoot=0xabcdef"));
    }

    #[test]
    fn test_relay_queue_lifecycle() {
        let mut queue = CheckpointRelayQueue::new(5);

        let batch1 = L1CheckpointBatch {
            block_height: 10,
            block_hash: "0x10".into(),
            state_root: "0xroot10".into(),
            transparency_root: "0xtrans10".into(),
            package_count: 5,
            bls_aggregate_signature: None,
            participant_bitmask: None,
            timestamp: 100,
        };

        let batch2 = L1CheckpointBatch {
            block_height: 20,
            block_hash: "0x20".into(),
            state_root: "0xroot20".into(),
            transparency_root: "0xtrans20".into(),
            package_count: 8,
            bls_aggregate_signature: None,
            participant_bitmask: None,
            timestamp: 200,
        };

        queue.enqueue(batch1).expect("enqueue 1");
        queue.enqueue(batch2).expect("enqueue 2");
        assert_eq!(queue.pending_count(), 2);

        // Mark height 10 confirmed
        queue.mark_confirmed(10);
        assert_eq!(queue.last_anchored_height(), 10);
        assert_eq!(queue.pending_count(), 1);

        let next = queue.pop_next().expect("should have batch 2");
        assert_eq!(next.block_height, 20);
        assert_eq!(queue.pending_count(), 0);
    }
}
