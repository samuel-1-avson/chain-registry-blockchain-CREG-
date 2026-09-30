// crates/node/src/proof.rs
// Builds and serves Merkle inclusion proofs for the light-client endpoint.
// GET /v1/packages/:canonical/proof → LightClientResponse

use crate::chain_store::ChainStore;
use anyhow::{bail, Context, Result};
use common::{BlockHeader, PackageStatus};
use resolver::light_client::{build_merkle_proof, LightClientResponse};

/// Build a full light-client proof response for a given package canonical ID.
/// Returns None if the package is not found or is still pending.
pub fn build_proof(
    canonical: &str,
    chain: &ChainStore,
    bootstrap: &common::ValidatorSet,
    data_dir: &std::path::Path,
) -> Result<Option<LightClientResponse>> {
    // Find the ChainRecord.
    let record = match chain.get_package(canonical)? {
        Some(r) => r,
        None => return Ok(None),
    };
    // A revocation changes the package's current lifecycle state, but it does
    // not erase the publication transaction from canonical history. Keep its
    // inclusion proof available for audit while returning the current status.
    if !matches!(
        &record.status,
        PackageStatus::Verified | PackageStatus::Revoked { .. }
    ) {
        return Ok(None);
    }
    let package_status = match &record.status {
        PackageStatus::Verified => "verified",
        PackageStatus::Revoked { .. } => "revoked",
        PackageStatus::Pending => return Ok(None),
    }
    .to_string();

    // Find the block that contains the Publish transaction for this package.
    let block_hash = &record.block_hash;
    let block = match chain.get_block_by_hash(block_hash)? {
        Some(b) => b,
        None => {
            tracing::warn!(
                "Block {} not found for package {}",
                &block_hash[..12],
                canonical
            );
            return Ok(None);
        }
    };

    let canonical_block = chain
        .get_block_by_height(block.header.height)?
        .context("proof block is absent from canonical history")?;
    if canonical_block.hash() != block.hash() {
        bail!("proof package points to a noncanonical block");
    }

    // Locate the transaction index within the block.
    let tx_index = block.transactions.iter().position(|tx| {
        tx.published_record()
            .is_some_and(|record| record.id.canonical() == canonical)
    });

    let tx_index = match tx_index {
        Some(i) => i,
        None => bail!(
            "Package {} not found in block {}",
            canonical,
            &block_hash[..12]
        ),
    };

    // Build the Merkle proof.
    let proof = build_merkle_proof(&block, tx_index)?;

    // Build the header chain from genesis to this block for light-client verification.
    // In production this would only return headers from the last known checkpoint.
    // For now we return the full chain (efficient for a registry with few blocks).
    let mut header_chain: Vec<(BlockHeader, String)> = Vec::new();
    for h in 1..block.header.height {
        let b = chain
            .get_block_by_height(h)?
            .with_context(|| format!("proof ancestry is missing block {h}"))?;
        header_chain.push((b.header.clone(), b.hash()));
    }
    // Include the package block itself.
    header_chain.push((block.header.clone(), block.hash()));
    let authority =
        crate::validator_set_history::verification_set_at(data_dir, block.header.height, bootstrap)
            .map_err(anyhow::Error::msg)?;
    let validator_set_history = crate::validator_set_history::load_checked(data_dir)?
        .into_iter()
        .filter(|entry| entry.effective_from_height <= block.header.height)
        .filter_map(|entry| entry.certificate)
        .collect();
    let proven_transaction = Some(block.transactions[tx_index].clone());

    Ok(Some(LightClientResponse {
        status: "verified".into(),
        package_status,
        block_hash: block.hash(),
        block_header: block.header,
        proof,
        header_chain,
        pbft_signatures: block.pbft_signatures,
        active_validators: authority.validators,
        proven_transaction,
        validator_set_history,
    }))
}
