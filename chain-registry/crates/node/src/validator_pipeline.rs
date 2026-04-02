// crates/node/src/validator_pipeline.rs
// Drives packages from pending pool through VRF → 3-stage validation →
// PBFT consensus → writes finalised Transaction to the channel.

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use common::{
    ChainRecord, PackageStatus, PublishRequest,
    Transaction, ValidatorVote, Finding, FindingSeverity,
};
use chrono::Utc;
use crate::{NodeState, finalized_tx::FinalizedTxSender, gossip::Gossip};

const POLL_INTERVAL_SECS: u64 = 2;

pub async fn run(
    state:  Arc<RwLock<NodeState>>,
    tx_out: FinalizedTxSender,
) {
    let mut ticker = interval(Duration::from_secs(POLL_INTERVAL_SECS));
    tracing::info!("Validator pipeline started");

    loop {
        ticker.tick().await;
        // tracing::debug!("Validator heartbeat"); // Keep it quiet for production but useful for debug
        if let Err(e) = tick(Arc::clone(&state), &tx_out).await {
            tracing::error!("Validator pipeline error: {}", e);
        }
    }
}

async fn tick(
    state:  Arc<RwLock<NodeState>>,
    tx_out: &FinalizedTxSender,
) -> anyhow::Result<()> {
    let pending: Vec<PublishRequest> = {
        let mut s = state.write().await;
        s.pending_pool.ready_for_validation()
    };

    if pending.is_empty() { return Ok(()); }
    tracing::info!("Pipeline processing {} package(s)", pending.len());

    let handles: Vec<_> = pending.into_iter().map(|req| {
        let state  = Arc::clone(&state);
        let sender = tx_out.clone();
        tokio::spawn(async move {
            process_package(state, req, sender).await;
        })
    }).collect();

    for h in handles {
        if let Err(e) = h.await {
            tracing::error!("Package task panicked: {}", e);
        }
    }
    Ok(())
}

async fn process_package(
    state:  Arc<RwLock<NodeState>>,
    req:    PublishRequest,
    tx_out: FinalizedTxSender,
) {
    let canonical = req.id.canonical();
    tracing::info!("Processing {}", canonical);

    let ipfs_url = {
        let s = state.read().await;
        s.config.ipfs_url.clone()
    };

    // ── Fetch tarball from IPFS ───────────────────────────────────────────────
    let mut tarball = match fetch_from_ipfs(&req.ipfs_cid, &ipfs_url).await {
        Ok(b)  => b,
        Err(e) => {
            tracing::error!("IPFS fetch failed for {}: {}", canonical, e);
            cleanup(&state, &canonical).await;
            return;
        }
    };

    if tarball.is_empty() {
        tracing::error!("Empty tarball received for {} — rejecting", canonical);
        cleanup(&state, &canonical).await;
        return;
    }

    // ── 2.5. Decrypt if shielded ──────────────────────────────────────────────
    if req.shielded {
        if let Some(bundle) = &req.key_bundle {
            tracing::info!("Decrypting shielded package: {}", canonical);
            match decrypt_shielded(&tarball, bundle) {
                Ok(decrypted) => {
                    tarball = decrypted;
                }
                Err(e) => {
                    tracing::error!("Decryption failed for {}: {}", canonical, e);
                    cleanup(&state, &canonical).await;
                    return;
                }
            }
        }
    }

    // ── Verify content hash ───────────────────────────────────────────────────
    let actual = common::sha256_hex(&tarball);
    if actual != req.content_hash {
        tracing::error!("Content hash mismatch for {}", canonical);
        let node_id = state.read().await.config.node_id.clone();
        let tx = common::Transaction::Revoke {
            package_canonical: canonical.clone(),
            reason:            "Content hash mismatch — possible tampering".into(),
            revoked_by:        node_id,
            evidence_hash:     "".into(),
        };
        let _ = tx_out.send(tx).await;
        cleanup(&state, &canonical).await;
        return;
    }


    let (is_validator, node_id, privkey_opt, prev_manifest) = {
        let s = state.read().await;
        let prev = s.chain.get_latest_version(&req.id.ecosystem, &req.id.name).ok().flatten();
        (s.config.is_validator, s.config.node_id.clone(), s.config.validator_privkey.clone(), prev.map(|r| req.manifest.clone()))
    };

    let (vote, pgp_fingerprint, findings) = if is_validator {
        if let Some(privkey) = privkey_opt.as_ref() {
            tracing::info!("[Consensus] Node is a validator — running full analysis for {}", canonical);
            match validator::validate_package(&req, &tarball, privkey, prev_manifest.as_ref()).await {
                Ok(res)  => (res.vote, res.pgp_fingerprint, res.findings),
                Err(e) => {
                    tracing::error!("Validation error for {}: {}", canonical, e);
                    cleanup(&state, &canonical).await;
                    return;
                }
            }
        } else {
            tracing::error!("[Consensus] Validator node missing private key — cannot analyze {}", canonical);
            cleanup(&state, &canonical).await;
            return;
        }
    } else {
        tracing::warn!("[Consensus] Node is NOT a validator — skipping analysis for {}", canonical);
        (ValidatorVote::Approve, None, Vec::new()) // Non-validators trust the consensus result.
    };


    // ── Generate our own signature (validators only) ──────────────────────────
    // Non-validators skipped consensus steps already; guard here defensively.
    let privkey_str = match privkey_opt.as_ref() {
        Some(k) => k,
        None => {
            tracing::warn!("No validator key — skipping signing for {}", canonical);
            cleanup(&state, &canonical).await;
            return;
        }
    };

    let our_sig = {
        use ed25519_dalek::{SigningKey, Signer};
        let key_bytes = match hex::decode(privkey_str) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!("Invalid validator key hex for {}: {}", canonical, e);
                cleanup(&state, &canonical).await;
                return;
            }
        };
        let key_arr: [u8; 32] = match key_bytes.try_into() {
            Ok(a) => a,
            Err(_) => {
                tracing::error!("Validator key must be 32 bytes for {}", canonical);
                cleanup(&state, &canonical).await;
                return;
            }
        };
        let signing_key = SigningKey::from_bytes(&key_arr);

        // Sign canonical || content_hash to bind the verdict to this exact version.
        let msg = format!("{}-{}", canonical, req.content_hash);
        let signature = signing_key.sign(msg.as_bytes());

        common::ValidatorSignature {
            validator_id:     node_id.clone(),
            validator_pubkey: hex::encode(signing_key.verifying_key().as_bytes()),
            signature:        hex::encode(signature.to_bytes()),
            vote:             vote.clone(),
            signed_at:        Utc::now(),
        }
    };

    // Store our own vote locally
    {
        let mut sw = state.write().await;
        sw.votes.entry(canonical.clone()).or_insert_with(Vec::new).push(our_sig.clone());
    }

    // Gossip our vote to peers via P2P Gossipsub
    let (approved, reject_reason) = match &vote {
        ValidatorVote::Approve => (true, None),
        ValidatorVote::Reject { reason } => (false, Some(reason.clone())),
    };

    // Sign the gossip vote with the message format the receive_vote handler verifies:
    // "<block_hash>:<approved>"
    let gossip_sig = {
        use ed25519_dalek::{SigningKey, Signer};
        let key_bytes = hex::decode(privkey_str).unwrap_or_default();
        if let Ok(key_arr) = key_bytes.try_into() as Result<[u8; 32], _> {
            let sk = SigningKey::from_bytes(&key_arr);
            let msg = format!("{}:{}", canonical, approved);
            hex::encode(sk.sign(msg.as_bytes()).to_bytes())
        } else {
            our_sig.signature.clone()
        }
    };

    let gossip_vote = crate::gossip::VoteGossip {
        block_hash:       canonical.clone(),
        validator_id:     node_id.clone(),
        validator_pubkey: our_sig.validator_pubkey.clone(),
        phase:            "commit".into(),
        approved,
        reject_reason,
        signature:        gossip_sig,
    };
    
    let p2p_handle = state.read().await.p2p.clone();
    let _ = p2p_handle.sender.send(crate::p2p::P2PCommand::Broadcast {
        topic: "creg/v1/votes".into(),
        data: serde_json::to_vec(&gossip_vote).unwrap_or_default(),
    }).await;

    // ── WAIT FOR QUORUM ───────────────────────────────────────────────────────
    let quorum_size = {
        let s = state.read().await;
        (s.validator_set.validators.len() * 2 / 3) + 1
    };
    let mut final_sigs = Vec::new();

    for _ in 0..60 { // Wait up to 30 seconds
        {
            let sr = state.read().await;
            if let Some(sigs) = sr.votes.get(&canonical) {
                if sigs.len() >= quorum_size {
                    final_sigs = sigs.clone();
                    break;
                }
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    if final_sigs.is_empty() {
        tracing::error!("Consensus timeout for package {}", canonical);
        return;
    }

    // ── Write finalised transaction ───────────────────────────────────────────
    let tx = match &vote {
        ValidatorVote::Approve => {
            let record = ChainRecord {
                id:                   req.id.clone(),
                content_hash:         req.content_hash.clone(),
                ipfs_cid:             req.ipfs_cid.clone(),
                publisher_pubkey:     req.publisher_pubkey.clone(),
                publisher_pubkeys:    req.publisher_pubkeys.clone(),
                block_hash:           "pending".into(),
                published_at:         Utc::now(),
                validator_signatures: final_sigs,
                status:               PackageStatus::Verified,
                shielded:             req.shielded,
                key_bundle:           req.key_bundle.clone(),
                pgp_fingerprint,
                findings,
                access_count:         0,
                last_accessed:        None,
                ..Default::default()
            };
            Transaction::Publish(record)
        }
        ValidatorVote::Reject { reason } => {
            common::Transaction::Revoke {
                package_canonical: canonical.clone(),
                reason:            reason.clone(),
                revoked_by:        node_id.clone(),
                evidence_hash:     "".into(),
            }
        }
    };

    if tx_out.send(tx).await.is_err() {
        tracing::error!("Finalized-tx channel closed — dropping result for {}", canonical);
    } else {
        tracing::info!(
            "{} → {}",
            canonical,
            if matches!(vote, ValidatorVote::Approve) { "VERIFIED" } else { "REJECTED" }
        );
    }

    cleanup(&state, &canonical).await;
}

async fn cleanup(state: &Arc<RwLock<NodeState>>, canonical: &str) {
    let mut s = state.write().await;
    s.pending_pool.remove(canonical);
}

async fn fetch_from_ipfs(cid: &str, ipfs_url: &str) -> anyhow::Result<Vec<u8>> {
    let url = format!("{}/api/v0/cat?arg={}", ipfs_url.trim_end_matches('/'), cid);
    let bytes = reqwest::Client::new()
        .post(&url).send().await?.bytes().await?.to_vec();
    Ok(bytes)
}

fn decrypt_shielded(data: &[u8], bundle: &str) -> anyhow::Result<Vec<u8>> {
    use aes_gcm::{Aes256Gcm, Key, Nonce, aead::{Aead, KeyInit}};
    
    // Parse bundle: "hex_key:hex_nonce"
    let parts: Vec<&str> = bundle.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid key bundle format");
    }
    
    let key_bytes = hex::decode(parts[0])?;
    let nonce_bytes = hex::decode(parts[1])?;
    
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let cipher = Aes256Gcm::new(key);
    
    // The CLI prepends the 12-byte nonce, so we skip it to get the ciphertext
    // Or we just use the provided nonce_bytes. 
    // In our CLI implementation, final_payload = nonce_bytes + ciphertext.
    let ciphertext = if data.len() > 12 && &data[..12] == &nonce_bytes[..] {
        &data[12..]
    } else {
        data
    };

    let decrypted = cipher.decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("AES decryption failed: {}", e))?;
        
    Ok(decrypted)
}
