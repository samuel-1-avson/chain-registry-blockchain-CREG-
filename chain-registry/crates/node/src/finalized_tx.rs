// crates/node/src/finalized_tx.rs
// A bounded async channel that the validator_pipeline writes verified
// Transaction objects into, and the block_producer buffers until it is
// the VRF-selected proposer for the current epoch.

use common::Transaction;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{mpsc, Mutex};

/// Capacity of the finalized-tx channel.
/// At 5-second block intervals with ~100 tx/block this is comfortable.
const CHANNEL_CAPACITY: usize = 512;

/// In-memory buffer cap for finalized transactions awaiting block production.
/// Txs are never dropped on a failed proposer gate; only this cap evicts.
pub const PENDING_BLOCK_TX_BUFFER_CAP: usize = 1000;

static PENDING_BUFFER_DEPTH: AtomicUsize = AtomicUsize::new(0);

/// Current depth of the block producer's pending finalized-tx buffer (for metrics).
pub fn pending_buffer_depth() -> usize {
    PENDING_BUFFER_DEPTH.load(Ordering::Relaxed)
}

fn set_pending_buffer_depth(depth: usize) {
    PENDING_BUFFER_DEPTH.store(depth, Ordering::Relaxed);
}

/// Update the exported pending-buffer depth gauge (used after local drains).
pub fn sync_pending_buffer_depth(depth: usize) {
    set_pending_buffer_depth(depth);
}

/// Sender half — held by the validator_pipeline.
pub type FinalizedTxSender = mpsc::Sender<Transaction>;

/// Receiver half — held (behind a Mutex) by the block_producer so it
/// can drain without needing to clone the receiver.
pub type FinalizedTxReceiver = Arc<Mutex<mpsc::Receiver<Transaction>>>;

/// Create a matched sender/receiver pair.
pub fn channel() -> (FinalizedTxSender, FinalizedTxReceiver) {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
    (tx, Arc::new(Mutex::new(rx)))
}

/// Non-blocking recv from the channel into `buffer`, respecting [`PENDING_BLOCK_TX_BUFFER_CAP`].
pub async fn recv_into_buffer(rx: &FinalizedTxReceiver, buffer: &mut Vec<Transaction>) {
    let mut guard = rx.lock().await;
    loop {
        match guard.try_recv() {
            Ok(tx) => {
                if buffer.len() >= PENDING_BLOCK_TX_BUFFER_CAP {
                    tracing::warn!(
                        pending = buffer.len(),
                        cap = PENDING_BLOCK_TX_BUFFER_CAP,
                        "Pending block tx buffer at cap; dropping finalized transaction"
                    );
                    continue;
                }
                buffer.push(tx);
            }
            Err(mpsc::error::TryRecvError::Empty) => break,
            Err(mpsc::error::TryRecvError::Disconnected) => break,
        }
    }
    set_pending_buffer_depth(buffer.len());
}

/// Put failed or deferred transactions back at the front of `buffer`.
pub fn requeue_front(buffer: &mut Vec<Transaction>, txs: Vec<Transaction>) {
    if txs.is_empty() {
        return;
    }
    let mut merged = txs;
    merged.append(buffer);
    if merged.len() > PENDING_BLOCK_TX_BUFFER_CAP {
        let dropped = merged.len() - PENDING_BLOCK_TX_BUFFER_CAP;
        tracing::warn!(
            dropped,
            cap = PENDING_BLOCK_TX_BUFFER_CAP,
            "Pending block tx buffer overflow on requeue; dropping oldest transactions"
        );
        merged.drain(..dropped);
    }
    *buffer = merged;
    set_pending_buffer_depth(buffer.len());
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{ChainRecord, PackageId, PackageStatus, Transaction};
    use chrono::Utc;

    fn sample_publish_tx(id: &str) -> Transaction {
        Transaction::Publish(ChainRecord {
            id: PackageId::new("npm", id, "1.0.0"),
            content_hash: "abc".into(),
            ipfs_cid: "bafy".into(),
            publisher_pubkey: "pk".into(),
            block_hash: "0".repeat(64),
            published_at: Utc::now(),
            validator_signatures: vec![],
            status: PackageStatus::Verified,
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn recv_into_buffer_respects_cap() {
        let (tx, rx) = channel();
        let send_count = PENDING_BLOCK_TX_BUFFER_CAP + 5;
        let sender = tokio::spawn(async move {
            for i in 0..send_count {
                tx.send(sample_publish_tx(&format!("pkg-{}", i)))
                    .await
                    .unwrap();
            }
        });

        let mut buffer = Vec::new();
        while buffer.len() < PENDING_BLOCK_TX_BUFFER_CAP {
            recv_into_buffer(&rx, &mut buffer).await;
            if sender.is_finished() && buffer.len() >= PENDING_BLOCK_TX_BUFFER_CAP {
                break;
            }
            tokio::task::yield_now().await;
        }
        sender.abort();

        assert_eq!(buffer.len(), PENDING_BLOCK_TX_BUFFER_CAP);
        assert_eq!(pending_buffer_depth(), PENDING_BLOCK_TX_BUFFER_CAP);
    }

    #[test]
    fn requeue_front_preserves_order_and_cap() {
        let mut buffer = vec![sample_publish_tx("a")];
        let failed = vec![sample_publish_tx("b"), sample_publish_tx("c")];
        requeue_front(&mut buffer, failed);
        assert_eq!(buffer.len(), 3);
        if let Transaction::Publish(r) = &buffer[0] {
            assert_eq!(r.id.name, "b");
        }
    }
}
