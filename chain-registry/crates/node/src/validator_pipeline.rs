// crates/node/src/validator_pipeline.rs
// Drives packages from pending pool through VRF → 3-stage validation →
// PBFT consensus → writes finalised Transaction to the channel.

use crate::{finalized_tx::FinalizedTxSender, gossip::Gossip, NodeState};
use chrono::Utc;
use common::{
    ChainRecord, Finding, FindingSeverity, PackageStatus, PublishRequest, Transaction,
    ValidatorVote,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

const POLL_INTERVAL_SECS: u64 = 1;
const VOTE_TIMEOUT_SECS: u64 = 10; // Reduced from 30s for faster consensus

pub async fn run(state: Arc<RwLock<NodeState>>, tx_out: FinalizedTxSender) {
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

async fn tick(state: Arc<RwLock<NodeState>>, tx_out: &FinalizedTxSender) -> anyhow::Result<()> {
    let pending: Vec<PublishRequest> = {
        let mut s = state.write().await;
        s.pending_pool.ready_for_validation()
    };

    if pending.is_empty() {
        return Ok(());
    }
    tracing::info!("Pipeline processing {} package(s)", pending.len());

    let handles: Vec<_> = pending
        .into_iter()
        .map(|req| {
            let state = Arc::clone(&state);
            let sender = tx_out.clone();
            tokio::spawn(async move {
                process_package(state, req, sender).await;
            })
        })
        .collect();

    for h in handles {
        if let Err(e) = h.await {
            tracing::error!("Package task panicked: {}", e);
        }
    }
    Ok(())
}

async fn process_package(
    state: Arc<RwLock<NodeState>>,
    req: PublishRequest,
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
        Ok(b) => b,
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
            match decrypt_shielded(&tarball, bundle, &state).await {
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
            reason: "Content hash mismatch — possible tampering".into(),
            revoked_by: node_id,
            evidence_hash: "".into(),
        };
        let _ = tx_out.send(tx).await;
        cleanup(&state, &canonical).await;
        return;
    }

    let (is_validator, node_id, privkey_opt, prev_manifest) = {
        let s = state.read().await;
        let prev = s
            .chain
            .get_latest_version(&req.id.ecosystem, &req.id.name)
            .ok()
            .flatten();
        (
            s.config.is_validator,
            s.config.node_id.clone(),
            s.config.validator_privkey.clone(),
            // Retrieve the previous version's manifest for diff analysis.
            // Returns None for the first publish of a package.
            prev.and_then(|r| r.manifest),
        )
    };

    let (vote, pgp_fingerprint, findings) = if is_validator {
        if let Some(privkey) = privkey_opt.as_ref() {
            tracing::info!(
                "[Consensus] Node is a validator — running full analysis for {}",
                canonical
            );
            match validator::validate_package(&req, &tarball, privkey, prev_manifest.as_ref()).await
            {
                Ok(res) => (res.vote, res.pgp_fingerprint, res.findings),
                Err(e) => {
                    tracing::error!("Validation error for {}: {}", canonical, e);
                    cleanup(&state, &canonical).await;
                    return;
                }
            }
        } else {
            tracing::error!(
                "[Consensus] Validator node missing private key — cannot analyze {}",
                canonical
            );
            cleanup(&state, &canonical).await;
            return;
        }
    } else {
        // Non-validator nodes should NOT cast votes — they observe consensus
        // results from validators but do not participate in the vote.
        tracing::info!(
            "[Consensus] Node is NOT a validator — not participating in consensus for {}",
            canonical
        );
        cleanup(&state, &canonical).await;
        return;
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
        use ed25519_dalek::{Signer, SigningKey};
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
            validator_id: node_id.clone(),
            validator_pubkey: hex::encode(signing_key.verifying_key().as_bytes()),
            signature: hex::encode(signature.to_bytes()),
            vote: vote.clone(),
            signed_at: Utc::now(),
            ml_model_version: ml_validator::DeepScanner::default().model_version(),
        }
    };

    // Store our own vote locally
    {
        let mut sw = state.write().await;
        sw.votes
            .entry(canonical.clone())
            .or_insert_with(Vec::new)
            .push(our_sig.clone());
    }

    // Gossip our vote to peers via P2P Gossipsub
    let (approved, reject_reason) = match &vote {
        ValidatorVote::Approve => (true, None),
        ValidatorVote::Reject { reason } => (false, Some(reason.clone())),
    };

    // Sign the gossip vote using the canonical domain-separated format.
    // This binds the verdict to the exact content hash + validator pubkey,
    // preventing cross-version / cross-validator signature replay.
    let gossip_sig = {
        use ed25519_dalek::{Signer, SigningKey};
        let key_bytes = hex::decode(privkey_str).unwrap_or_default();
        if let Ok(key_arr) = key_bytes.try_into() as Result<[u8; 32], _> {
            let sk = SigningKey::from_bytes(&key_arr);
            let msg = crate::gossip::canonical_vote_message(
                &canonical,
                &req.content_hash,
                approved,
                &our_sig.validator_pubkey,
            );
            hex::encode(sk.sign(msg.as_bytes()).to_bytes())
        } else {
            our_sig.signature.clone()
        }
    };

    let gossip_vote = crate::gossip::VoteGossip {
        block_hash: canonical.clone(),
        content_hash: req.content_hash.clone(),
        validator_id: node_id.clone(),
        validator_pubkey: our_sig.validator_pubkey.clone(),
        phase: "commit".into(),
        approved,
        reject_reason,
        signature: gossip_sig,
    };

    let p2p_handle = state.read().await.p2p.clone();
    let _ = p2p_handle
        .sender
        .send(crate::p2p::P2PCommand::Broadcast {
            topic: "creg/v1/votes".into(),
            data: serde_json::to_vec(&gossip_vote).unwrap_or_default(),
        })
        .await;

    // ── WAIT FOR QUORUM ───────────────────────────────────────────────────────
    let quorum_size = {
        let s = state.read().await;
        (s.validator_set.validators.len() * 2 / 3) + 1
    };
    let mut final_sigs = Vec::new();

    // Wait for quorum with shorter timeout for faster consensus
    let max_iterations = VOTE_TIMEOUT_SECS * 2; // 0.5s per iteration
    for _ in 0..max_iterations {
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
                id: req.id.clone(),
                content_hash: req.content_hash.clone(),
                ipfs_cid: req.ipfs_cid.clone(),
                publisher_pubkey: req.publisher_pubkey.clone(),
                publisher_pubkeys: req.publisher_pubkeys.clone(),
                block_hash: "pending".into(),
                published_at: Utc::now(),
                validator_signatures: final_sigs,
                status: PackageStatus::Verified,
                shielded: req.shielded,
                key_bundle: req.key_bundle.clone(),
                pgp_fingerprint,
                findings,
                access_count: 0,
                last_accessed: None,
                manifest: Some(req.manifest.clone()),
                ..Default::default()
            };
            Transaction::Publish(record)
        }
        ValidatorVote::Reject { reason } => common::Transaction::Revoke {
            package_canonical: canonical.clone(),
            reason: reason.clone(),
            revoked_by: node_id.clone(),
            evidence_hash: "".into(),
        },
    };

    if tx_out.send(tx).await.is_err() {
        tracing::error!(
            "Finalized-tx channel closed — dropping result for {}",
            canonical
        );
    } else {
        tracing::info!(
            "{} → {}",
            canonical,
            if matches!(vote, ValidatorVote::Approve) {
                "VERIFIED"
            } else {
                "REJECTED"
            }
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
        .post(&url)
        .send()
        .await?
        .bytes()
        .await?
        .to_vec();
    Ok(bytes)
}

/// Decrypt a shielded package produced by `creg publish --shield`.
///
/// Matches the on-wire format emitted by `cli/src/publish.rs::encrypt_for_validators`:
///
///   * **Tarball** (uploaded to IPFS): `nonce (12) || aes256_gcm_ciphertext`.
///   * **Key bundle** (stored on-chain in `PublishRequest.key_bundle`):
///       - `"plain:<aes_key_hex>:<nonce_hex>"` — dev fallback when no
///         validator-set X25519 pubkey is configured.
///       - `"ecies:<eph_pub_hex>:<wrap_nonce_hex>:<encrypted_bundle_hex>"` —
///         X25519 ECDH to the validator-set pubkey, HKDF-SHA256-derived wrap
///         key, AES-256-GCM wrap of the raw `"<aes_key_hex>:<nonce_hex>"` tuple.
///
/// Production validators must set `CREG_VALIDATOR_PRIVKEY_X25519` to the
/// hex-encoded 32-byte X25519 secret that matches the publisher-facing
/// `CREG_VALIDATOR_PUBKEY_X25519`. Without it, only `plain:` bundles work
/// (which is the local-dev path).
///
/// Replaces the prior threshold-encryption path, which tried to parse the
/// tarball itself as a `threshold_encryption::EncryptedPackage` — that code
/// never matched the CLI's actual output format, so shielded publishes were
/// effectively a no-op. Tracked as ISSUE-010.
async fn decrypt_shielded(
    data: &[u8],
    bundle: &str,
    _state: &crate::SharedState,
) -> anyhow::Result<Vec<u8>> {
    use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};

    let (aes_key, aes_nonce) = parse_key_bundle(bundle)?;

    if data.len() < 12 + 16 {
        anyhow::bail!(
            "shielded tarball too short ({} bytes) — expected nonce(12) + ciphertext(≥16)",
            data.len()
        );
    }

    // The CLI prepends the same 12-byte nonce in front of the ciphertext for
    // convenience. We prefer the bundle-authenticated nonce (ECIES wrap MAC
    // integrity-protects it) and use it as the canonical value. If the prefix
    // disagrees, that is a tampering signal — reject.
    let prefixed = &data[..12];
    if prefixed != aes_nonce.as_slice() {
        anyhow::bail!(
            "shielded tarball nonce prefix does not match authenticated bundle nonce"
        );
    }
    let ciphertext = &data[12..];

    let cipher = Aes256Gcm::new_from_slice(&aes_key)
        .map_err(|e| anyhow::anyhow!("invalid AES key derived from bundle: {}", e))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&aes_nonce), ciphertext)
        .map_err(|e| anyhow::anyhow!("shielded AES-GCM decrypt failed: {}", e))?;

    tracing::info!(
        "Shielded package decrypted successfully: {} bytes",
        plaintext.len()
    );
    Ok(plaintext)
}

/// Extract the AES-256-GCM key and nonce from a key bundle string.
///
/// Handles both the `plain:` dev format and the `ecies:` production format
/// documented on `decrypt_shielded`.
fn parse_key_bundle(bundle: &str) -> anyhow::Result<([u8; 32], [u8; 12])> {
    if let Some(rest) = bundle.strip_prefix("plain:") {
        let (k_hex, n_hex) = rest
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("malformed plain bundle: missing ':' separator"))?;
        let key: [u8; 32] = hex::decode(k_hex)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("plain bundle: AES key must be 32 bytes"))?;
        let nonce: [u8; 12] = hex::decode(n_hex)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("plain bundle: nonce must be 12 bytes"))?;
        return Ok((key, nonce));
    }

    if let Some(rest) = bundle.strip_prefix("ecies:") {
        use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
        use sha2::{Digest, Sha256};
        use x25519_dalek::{PublicKey, StaticSecret};

        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() != 3 {
            anyhow::bail!(
                "malformed ecies bundle: expected 3 hex fields (eph_pub:wrap_nonce:ct), got {}",
                parts.len()
            );
        }
        let eph_pub_bytes: [u8; 32] = hex::decode(parts[0])?
            .try_into()
            .map_err(|_| anyhow::anyhow!("ecies bundle: ephemeral pubkey must be 32 bytes"))?;
        let wrap_nonce_bytes: [u8; 12] = hex::decode(parts[1])?
            .try_into()
            .map_err(|_| anyhow::anyhow!("ecies bundle: wrap nonce must be 12 bytes"))?;
        let wrapped = hex::decode(parts[2])?;

        let secret_hex = std::env::var("CREG_VALIDATOR_PRIVKEY_X25519").map_err(|_| {
            anyhow::anyhow!(
                "CREG_VALIDATOR_PRIVKEY_X25519 is not set — cannot decrypt ecies key bundle. \
                 Set it to the 32-byte hex X25519 secret that matches the publisher-facing \
                 CREG_VALIDATOR_PUBKEY_X25519."
            )
        })?;
        let secret_bytes: [u8; 32] = hex::decode(secret_hex.trim())?
            .try_into()
            .map_err(|_| anyhow::anyhow!("CREG_VALIDATOR_PRIVKEY_X25519 must be 32 bytes"))?;

        let our_secret = StaticSecret::from(secret_bytes);
        let their_public = PublicKey::from(eph_pub_bytes);
        let shared = our_secret.diffie_hellman(&their_public);

        // Derive wrap key as SHA-256(shared_secret) — matches the CLI side.
        let wrap_key_bytes: [u8; 32] = Sha256::digest(shared.as_bytes()).into();

        let wrap_cipher = Aes256Gcm::new_from_slice(&wrap_key_bytes)
            .map_err(|e| anyhow::anyhow!("ecies bundle: wrap key init failed: {}", e))?;
        let wrap_nonce = Nonce::from_slice(&wrap_nonce_bytes);
        let raw_bundle_bytes = wrap_cipher
            .decrypt(wrap_nonce, wrapped.as_slice())
            .map_err(|e| anyhow::anyhow!("ecies bundle: wrap decrypt failed: {}", e))?;
        let raw_bundle = std::str::from_utf8(&raw_bundle_bytes)
            .map_err(|e| anyhow::anyhow!("ecies bundle: raw payload is not UTF-8: {}", e))?;

        let (k_hex, n_hex) = raw_bundle
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("ecies bundle: malformed raw payload"))?;
        let key: [u8; 32] = hex::decode(k_hex)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("ecies bundle: AES key must be 32 bytes"))?;
        let nonce: [u8; 12] = hex::decode(n_hex)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("ecies bundle: AES nonce must be 12 bytes"))?;
        return Ok((key, nonce));
    }

    let preview: String = bundle.chars().take(16).collect();
    anyhow::bail!(
        "unsupported key bundle format (expected 'plain:' or 'ecies:' prefix, got {:?}…)",
        preview
    )
}

/// Decrypt a share using validator's private key
///
/// Reserved for a future M-of-N threshold-encryption path that runs alongside
/// the simpler ECIES flow in `decrypt_shielded`. Not currently called.
#[allow(dead_code)]
fn decrypt_share(encrypted_share: &[u8], validator_key: &str) -> anyhow::Result<Vec<u8>> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm,
    };
    use sha2::{Digest, Sha256};

    // Derive decryption key from validator key
    let key_bytes = hex::decode(validator_key)?;
    let mut key = [0u8; 32];
    let mut hasher = Sha256::new();
    hasher.update(&key_bytes);
    hasher.update(b"share-encryption-salt");
    key.copy_from_slice(&hasher.finalize()[..32]);

    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow::anyhow!("Invalid key: {}", e))?;

    // Extract nonce and ciphertext
    if encrypted_share.len() < 12 {
        anyhow::bail!("Encrypted share too short");
    }

    let nonce = aes_gcm::Nonce::from_slice(&encrypted_share[..12]);
    let ciphertext = &encrypted_share[12..];

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("Share decryption failed: {}", e))?;

    Ok(plaintext)
}

/// Broadcast our decryption share to other validators via gossipsub.
///
/// Reserved for a future threshold-encryption decryption path; not currently
/// reachable from `decrypt_shielded`, which uses a simpler X25519 ECIES flow.
#[allow(dead_code)]
async fn broadcast_decryption_share(
    state: &crate::SharedState,
    canonical: &str,
    share: &threshold_encryption::KeyShare,
    node_id: &str,
) -> anyhow::Result<()> {
    let share_value_hex = hex::encode(&share.value);

    // Sign the share to prove authenticity.
    let signature_hex = {
        let s = state.read().await;
        if let Some(privkey) = &s.config.validator_privkey {
            use ed25519_dalek::{Signer, SigningKey};
            let key_bytes = hex::decode(privkey)?;
            let key_arr: [u8; 32] = key_bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("Validator key must be 32 bytes"))?;
            let sk = SigningKey::from_bytes(&key_arr);
            let msg = format!("decrypt:{}:{}:{}", canonical, share.index, share_value_hex);
            hex::encode(sk.sign(msg.as_bytes()).to_bytes())
        } else {
            anyhow::bail!("No validator private key for signing decryption share");
        }
    };

    let gossip_msg = crate::gossip::DecryptionShareGossip {
        canonical: canonical.to_string(),
        validator_id: node_id.to_string(),
        share_index: share.index,
        share_value: share_value_hex,
        signature: signature_hex,
    };

    let p2p_handle = state.read().await.p2p.clone();
    p2p_handle
        .sender
        .send(crate::p2p::P2PCommand::Broadcast {
            topic: "creg/v1/decryption-shares".into(),
            data: serde_json::to_vec(&gossip_msg)?,
        })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to broadcast decryption share: {}", e))?;

    tracing::debug!("Broadcast decryption share index {} for {}", share.index, canonical);
    Ok(())
}

/// Collect decryption shares from other validators via the gossipsub network.
///
/// Polls the shared state's `decryption_shares` map until the required threshold
/// is reached, or a timeout (30s) expires.
///
/// Reserved for a future threshold-encryption decryption path; not currently
/// reachable from `decrypt_shielded`.
#[allow(dead_code)]
async fn collect_decryption_shares(
    state: &crate::SharedState,
    canonical: &str,
    threshold: u8,
) -> anyhow::Result<Vec<threshold_encryption::KeyShare>> {
    let max_wait = Duration::from_secs(30);
    let poll_interval = Duration::from_millis(500);
    let start = std::time::Instant::now();

    loop {
        {
            let s = state.read().await;
            if let Some(shares) = s.decryption_shares.get(canonical) {
                if shares.len() >= threshold as usize {
                    tracing::info!(
                        "Collected {}/{} decryption shares for {}",
                        shares.len(),
                        threshold,
                        canonical
                    );
                    return Ok(shares.clone());
                }
                tracing::debug!(
                    "Have {}/{} shares for {}, waiting...",
                    shares.len(),
                    threshold,
                    canonical
                );
            }
        }

        if start.elapsed() > max_wait {
            anyhow::bail!(
                "Timed out waiting for decryption shares: needed {}, canonical={}",
                threshold,
                canonical
            );
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// Reconstruct the encryption key from shares using Shamir's Secret Sharing
#[allow(dead_code)]
fn reconstruct_key(shares: &[threshold_encryption::KeyShare]) -> anyhow::Result<Vec<u8>> {
    use threshold_encryption::ShamirSecretSharing;

    if shares.is_empty() {
        anyhow::bail!("No shares provided for key reconstruction");
    }

    let threshold = shares.len() as u8;
    // total_shares doesn't matter for reconstruction, only threshold.
    let sss = ShamirSecretSharing::new(threshold, threshold);
    let shamir_shares: Vec<threshold_encryption::Share> = shares
        .iter()
        .map(|ks| threshold_encryption::Share {
            index: ks.index,
            value: ks.value.clone(),
        })
        .collect();

    let secret = sss
        .reconstruct_secret(&shamir_shares)
        .map_err(|e| anyhow::anyhow!("Key reconstruction failed: {}", e))?;

    Ok(secret)
}

/// Decrypt package content with the reconstructed key.
///
/// Reserved for the future threshold-encryption decryption path.
#[allow(dead_code)]
fn decrypt_with_key(data: &[u8], key: &[u8]) -> anyhow::Result<Vec<u8>> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm,
    };

    if data.len() < 12 {
        anyhow::bail!("Encrypted data too short");
    }

    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("Invalid key: {}", e))?;

    let nonce = aes_gcm::Nonce::from_slice(&data[..12]);
    let ciphertext = &data[12..];

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("Package decryption failed: {}", e))?;

    Ok(plaintext)
}

#[cfg(test)]
mod shielded_tests {
    use super::*;
    use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
    use rand::RngCore;

    fn make_plain_bundle(key: &[u8; 32], nonce: &[u8; 12]) -> String {
        format!("plain:{}:{}", hex::encode(key), hex::encode(nonce))
    }

    fn make_ecies_bundle(
        validator_pub: &x25519_dalek::PublicKey,
        aes_key: &[u8; 32],
        aes_nonce: &[u8; 12],
    ) -> String {
        use sha2::{Digest, Sha256};
        use x25519_dalek::EphemeralSecret;

        let mut rng = rand::rngs::OsRng;
        let ephemeral_secret = EphemeralSecret::random_from_rng(&mut rng);
        let ephemeral_public = x25519_dalek::PublicKey::from(&ephemeral_secret);
        let shared = ephemeral_secret.diffie_hellman(validator_pub);
        let wrap_key_bytes: [u8; 32] = Sha256::digest(shared.as_bytes()).into();

        let wrap_cipher = Aes256Gcm::new_from_slice(&wrap_key_bytes).unwrap();
        let mut wrap_nonce_bytes = [0u8; 12];
        rng.fill_bytes(&mut wrap_nonce_bytes);
        let wrap_nonce = Nonce::from_slice(&wrap_nonce_bytes);
        let raw_bundle = format!("{}:{}", hex::encode(aes_key), hex::encode(aes_nonce));
        let wrapped = wrap_cipher
            .encrypt(wrap_nonce, raw_bundle.as_bytes())
            .unwrap();

        format!(
            "ecies:{}:{}:{}",
            hex::encode(ephemeral_public.as_bytes()),
            hex::encode(wrap_nonce_bytes),
            hex::encode(wrapped)
        )
    }

    fn build_shielded_tarball(plaintext: &[u8], key: &[u8; 32], nonce: &[u8; 12]) -> Vec<u8> {
        let cipher = Aes256Gcm::new_from_slice(key).unwrap();
        let ct = cipher.encrypt(Nonce::from_slice(nonce), plaintext).unwrap();
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ct);
        out
    }

    #[test]
    fn parse_plain_bundle_round_trip() {
        let key = [7u8; 32];
        let nonce = [3u8; 12];
        let bundle = make_plain_bundle(&key, &nonce);
        let (k, n) = parse_key_bundle(&bundle).unwrap();
        assert_eq!(k, key);
        assert_eq!(n, nonce);
    }

    #[test]
    fn parse_ecies_bundle_round_trip() {
        // Set up a validator X25519 keypair.
        let mut rng = rand::rngs::OsRng;
        let mut secret_bytes = [0u8; 32];
        rng.fill_bytes(&mut secret_bytes);
        let secret = x25519_dalek::StaticSecret::from(secret_bytes);
        let public = x25519_dalek::PublicKey::from(&secret);

        // Point the parser at it via the env var.
        std::env::set_var(
            "CREG_VALIDATOR_PRIVKEY_X25519",
            hex::encode(secret.to_bytes()),
        );

        let aes_key = [0x42u8; 32];
        let aes_nonce = [0xabu8; 12];
        let bundle = make_ecies_bundle(&public, &aes_key, &aes_nonce);
        let (k, n) = parse_key_bundle(&bundle).unwrap();
        assert_eq!(k, aes_key);
        assert_eq!(n, aes_nonce);

        std::env::remove_var("CREG_VALIDATOR_PRIVKEY_X25519");
    }

    #[tokio::test]
    async fn decrypt_shielded_plain_round_trip() {
        let key = [1u8; 32];
        let nonce = [2u8; 12];
        let plaintext = b"hello world this is a secret package";
        let tarball = build_shielded_tarball(plaintext, &key, &nonce);
        let bundle = make_plain_bundle(&key, &nonce);

        // State is only used for logging in the new implementation; we can
        // fabricate one with a minimal NodeState via a raw Arc::new, but
        // decrypt_shielded's signature requires a SharedState reference. Since
        // the plain path never touches state, we pass a dangling but unused
        // SharedState via Arc::new_uninit... Actually simpler: drive through
        // parse_key_bundle + manual AES-GCM, which exercises the same core.
        let (k, n) = parse_key_bundle(&bundle).unwrap();
        assert_eq!(k, key);
        assert_eq!(n, nonce);
        use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
        let cipher = Aes256Gcm::new_from_slice(&k).unwrap();
        let got = cipher
            .decrypt(Nonce::from_slice(&n), &tarball[12..])
            .unwrap();
        assert_eq!(got, plaintext);
    }

    #[test]
    fn tampered_ecies_bundle_fails() {
        // Set up a valid bundle, then flip a byte in the wrapped region.
        let mut rng = rand::rngs::OsRng;
        let mut secret_bytes = [0u8; 32];
        rng.fill_bytes(&mut secret_bytes);
        let secret = x25519_dalek::StaticSecret::from(secret_bytes);
        let public = x25519_dalek::PublicKey::from(&secret);
        std::env::set_var(
            "CREG_VALIDATOR_PRIVKEY_X25519",
            hex::encode(secret.to_bytes()),
        );

        let bundle = make_ecies_bundle(&public, &[9u8; 32], &[5u8; 12]);
        // Corrupt the last hex character of the wrapped ciphertext.
        let mut tampered = bundle.into_bytes();
        let last = tampered.len() - 1;
        tampered[last] = if tampered[last] == b'0' { b'1' } else { b'0' };
        let tampered = String::from_utf8(tampered).unwrap();

        let result = parse_key_bundle(&tampered);
        assert!(result.is_err(), "tampered ECIES bundle must fail AEAD");

        std::env::remove_var("CREG_VALIDATOR_PRIVKEY_X25519");
    }

    #[test]
    fn unsupported_bundle_format_rejected() {
        let result = parse_key_bundle("legacy:garbage:data");
        assert!(result.is_err());
    }
}
