//! Durable propagation of actual publisher/validator revocation proofs.
use anyhow::{Context, Result};
use common::{PackageStatus, SignedRevocationRequest, Transaction};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

pub const TOPIC: &str = "creg/v1/revocations";
const MAX_RETAINED_PROOFS: usize = 1000;

fn directory(data_dir: &Path) -> PathBuf {
    data_dir.join("revocation-outbox")
}

pub fn transaction(request: &SignedRevocationRequest, content_hash: &str) -> Transaction {
    Transaction::Revoke {
        package_canonical: request.package_canonical.clone(),
        reason: request.reason.clone(),
        revoked_by: request.revoker_pubkey.clone(),
        evidence_hash: content_hash.to_owned(),
    }
}

fn verify_signature(request: &SignedRevocationRequest) -> Result<()> {
    anyhow::ensure!(
        request.signature_version <= 1,
        "unsupported revocation signature version"
    );
    anyhow::ensure!(
        !request.reason.is_empty()
            && request.reason.len() <= 4096
            && request.package_canonical.len() <= 256
            && request.revoker_pubkey.len() == 64
            && request.signature.len() == 128,
        "invalid revocation proof bounds"
    );
    let key = hex::decode(&request.revoker_pubkey)?;
    let key = VerifyingKey::try_from(key.as_slice())?;
    let signature = hex::decode(&request.signature)?;
    key.verify(
        request.signing_message().as_bytes(),
        &Signature::try_from(signature.as_slice())?,
    )?;
    Ok(())
}

fn persist(data_dir: &Path, id: &str, request: &SignedRevocationRequest) -> Result<()> {
    let dir = directory(data_dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{id}.json"));
    if path.exists() {
        let prior: SignedRevocationRequest = serde_json::from_slice(&std::fs::read(&path)?)?;
        // Upgrading a legacy proof to a chain-bound signature is safe only for
        // the exact same owner, package and reason; the durable work ID binds it.
        anyhow::ensure!(
            prior.package_canonical == request.package_canonical
                && prior.reason == request.reason
                && prior.revoker_pubkey == request.revoker_pubkey
                && prior.chain_id == request.chain_id
                && prior.genesis_hash == request.genesis_hash,
            "retained revocation proof conflicts with its work ID"
        );
        if prior.signature_version >= request.signature_version {
            return Ok(());
        }
    } else {
        let count = std::fs::read_dir(&dir)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .count();
        anyhow::ensure!(
            count < MAX_RETAINED_PROOFS,
            "revocation outbox is full; retained proofs preserved"
        );
    }
    let temp = dir.join(format!("{id}.tmp"));
    let mut file = std::fs::File::create(&temp)?;
    file.write_all(&serde_json::to_vec(request)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temp, &path)?;
    #[cfg(unix)]
    std::fs::File::open(&dir)?.sync_all()?;
    Ok(())
}

/// Serialize persistence with canonical adoption. Persist the original proof
/// first so a crash before work enqueue can be repaired by the periodic worker.
pub async fn admit(state: &crate::SharedState, request: &SignedRevocationRequest) -> Result<bool> {
    verify_signature(request)?;
    let s = state.write().await;
    anyhow::ensure!(
        request.chain_id == s.config.chain_id,
        "revocation chain mismatch"
    );
    let genesis = s
        .chain
        .get_block_by_height(0)?
        .context("canonical genesis missing")?;
    anyhow::ensure!(
        request.genesis_hash == genesis.hash(),
        "revocation genesis mismatch"
    );
    let record = s
        .chain
        .get_package(&request.package_canonical)?
        .context("revocation package missing")?;
    anyhow::ensure!(
        record.publisher_pubkey == request.revoker_pubkey
            || s.validator_set
                .validators
                .iter()
                .any(|v| v.pubkey == request.revoker_pubkey
                    && common::validator_status_is_active(&v.status)),
        "revoker is not the owner or an active validator"
    );
    if matches!(record.status, PackageStatus::Revoked { .. }) {
        return Ok(false);
    }
    anyhow::ensure!(
        record.status == PackageStatus::Verified,
        "revocation package is not finalized as verified"
    );
    let tx = transaction(request, &record.content_hash);
    persist(&s.config.data_dir, &common::transaction_hash(&tx), request)?;
    // Observers relay the proof and follow canonical blocks; they do not own
    // a producer queue or vote in PBFT.
    if s.config.is_validator {
        s.chain.enqueue_finalized_work(&tx)?;
    }
    Ok(true)
}

pub async fn broadcast(
    handle: &crate::p2p::P2PHandle,
    request: SignedRevocationRequest,
) -> Result<()> {
    let delivery_id = chrono::Utc::now()
        .timestamp_nanos_opt()
        .context("delivery timestamp overflow")?
        .to_string();
    let data = serde_json::to_vec(&common::GossipMessage::RevocationRequest {
        request,
        delivery_id,
    })?;
    handle
        .sender
        .send(crate::p2p::P2PCommand::Broadcast {
            topic: TOPIC.into(),
            data,
        })
        .await
        .map_err(|_| anyhow::anyhow!("revocation gossip channel closed"))
}

pub async fn run(state: crate::SharedState, handle: crate::p2p::P2PHandle) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(10));
    loop {
        ticker.tick().await;
        let dir = { directory(&state.read().await.config.data_dir) };
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                tracing::error!(%error, "Revocation outbox unreadable; preserving work");
                continue;
            }
        };
        for entry in entries {
            let result = async {
                let path = entry?.path();
                if path.extension().is_none_or(|ext| ext != "json") {
                    return Ok::<(), anyhow::Error>(());
                }
                let request: SignedRevocationRequest =
                    serde_json::from_slice(&std::fs::read(&path)?)?;
                if admit(&state, &request).await? {
                    broadcast(&handle, request).await?;
                } else {
                    // Canonical state is terminal. Keep the public signed proof
                    // as retained evidence; stop replaying it rather than erase it.
                    let s = state.read().await;
                    if s.chain
                        .get_package(&request.package_canonical)?
                        .is_some_and(|record| {
                            matches!(record.status, PackageStatus::Revoked { .. })
                        })
                    {
                        let done = path.with_extension("committed");
                        std::fs::rename(&path, &done)?;
                        #[cfg(unix)]
                        std::fs::File::open(&dir)?.sync_all()?;
                    }
                }
                Ok(())
            }
            .await;
            if let Err(error) = result {
                tracing::warn!(%error, "Revocation propagation deferred; proof retained");
            }
        }
    }
}
