//! BN254 / BLS Aggregate Multi-Signature Module
//!
//! Provides cryptographic multi-signature aggregation and batch verification for
//! Layer 2 PBFT block consensus, compressing O(N) validator signatures into an
//! O(1) 32-byte / 64-byte aggregate point with a compact participation bitmask.

use anyhow::{bail, Context, Result};
use ark_bn254::{Bn254, Fr, G1Affine, G1Projective, G2Affine, G2Projective};
use ark_ec::pairing::Pairing;
use ark_ec::{AffineRepr, CurveGroup, Group};
use ark_ff::{Field, PrimeField, UniformRand};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Domain separation tag for consensus multi-signatures
const BLS_DST: &[u8] = b"CREG-BN254-BLS-SIG-V1";

/// A BN254 public key (point in G2)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlsPublicKey {
    /// Compressed bytes (hex encoded)
    pub pubkey_hex: String,
}

/// A single BN254 signature (point in G1)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlsSignature {
    /// Compressed bytes (hex encoded)
    pub sig_hex: String,
}

/// Aggregated multi-signature with participant bitmask
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlsAggregateSignature {
    /// Aggregate signature point (hex encoded compressed G1 point)
    pub aggregate_sig_hex: String,
    /// Signer participation bitmask (1 bit per validator index in set)
    pub participant_bitmask: Vec<u8>,
    /// Number of distinct validator signatures aggregated
    pub participant_count: usize,
}

/// Keypair for BN254 BLS signing
pub struct BlsKeypair {
    pub secret_key: Fr,
    pub public_key: G2Affine,
}

impl BlsKeypair {
    /// Generate a fresh random keypair using CSPRNG
    pub fn generate<R: RngCore>(rng: &mut R) -> Self {
        let secret_key = Fr::rand(rng);
        let public_key = (G2Projective::generator() * secret_key).into_affine();
        Self {
            secret_key,
            public_key,
        }
    }

    /// Reconstruct keypair from 32-byte secret key scalar
    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Result<Self> {
        let secret_key = Fr::from_be_bytes_mod_order(bytes);
        let public_key = (G2Projective::generator() * secret_key).into_affine();
        Ok(Self {
            secret_key,
            public_key,
        })
    }

    /// Sign a message hash
    pub fn sign(&self, message: &[u8]) -> BlsSignature {
        let hash_point = hash_to_g1(message);
        let sig_proj = hash_point * self.secret_key;
        let sig_affine: G1Affine = sig_proj.into_affine();

        let mut bytes = Vec::new();
        sig_affine
            .serialize_compressed(&mut bytes)
            .expect("serialization");

        BlsSignature {
            sig_hex: hex::encode(bytes),
        }
    }

    /// Export public key structure
    pub fn public_key(&self) -> BlsPublicKey {
        let mut bytes = Vec::new();
        self.public_key
            .serialize_compressed(&mut bytes)
            .expect("serialization");
        BlsPublicKey {
            pubkey_hex: hex::encode(bytes),
        }
    }
}

/// Hash arbitrary message bytes to a point in G1 using hash-to-field + generator multiplication
pub fn hash_to_g1(message: &[u8]) -> G1Affine {
    let mut hasher = Sha256::new();
    hasher.update(BLS_DST);
    hasher.update((message.len() as u64).to_be_bytes());
    hasher.update(message);
    let hash = hasher.finalize();

    let scalar = Fr::from_be_bytes_mod_order(&hash);
    (G1Projective::generator() * scalar).into_affine()
}

/// Aggregate multiple individual BLS signatures into one aggregate signature
pub fn aggregate_signatures(
    signatures: &[BlsSignature],
    signer_indices: &[usize],
    total_validator_count: usize,
) -> Result<BlsAggregateSignature> {
    if signatures.is_empty() {
        bail!("Cannot aggregate empty signature set");
    }
    if signatures.len() != signer_indices.len() {
        bail!("Signatures count does not match indices count");
    }

    let mut agg_proj = G1Projective::default();

    for sig in signatures {
        let raw_bytes = hex::decode(sig.sig_hex.trim_start_matches("0x"))
            .context("Invalid hex in signature")?;
        let affine = G1Affine::deserialize_compressed(&raw_bytes[..])
            .map_err(|e| anyhow::anyhow!("Failed to deserialize G1 point: {}", e))?;
        agg_proj += affine;
    }

    let agg_affine: G1Affine = agg_proj.into_affine();
    let mut agg_bytes = Vec::new();
    agg_affine
        .serialize_compressed(&mut agg_bytes)
        .map_err(|e| anyhow::anyhow!("Failed to serialize aggregate signature: {}", e))?;

    let bitmask = indices_to_bitmask(signer_indices, total_validator_count);

    Ok(BlsAggregateSignature {
        aggregate_sig_hex: hex::encode(agg_bytes),
        participant_bitmask: bitmask,
        participant_count: signatures.len(),
    })
}

/// Verify an aggregate BLS signature against a message and validator public keys
pub fn verify_aggregate_signature(
    agg_sig: &BlsAggregateSignature,
    message: &[u8],
    validator_pubkeys: &[BlsPublicKey],
) -> Result<bool> {
    let signer_indices = bitmask_to_indices(&agg_sig.participant_bitmask);
    if signer_indices.is_empty() {
        return Ok(false);
    }
    if signer_indices.len() != agg_sig.participant_count {
        return Ok(false);
    }

    // Deserialize aggregate signature point (G1)
    let raw_sig_bytes = hex::decode(agg_sig.aggregate_sig_hex.trim_start_matches("0x"))
        .context("Invalid aggregate signature hex")?;
    let agg_point = G1Affine::deserialize_compressed(&raw_sig_bytes[..])
        .map_err(|e| anyhow::anyhow!("Invalid aggregate G1 point: {}", e))?;

    // Sum participating public keys in G2
    let mut agg_pk_proj = G2Projective::default();
    for &idx in &signer_indices {
        let pk_struct = validator_pubkeys
            .get(idx)
            .context("Validator index out of bounds in bitmask")?;
        let raw_pk_bytes = hex::decode(pk_struct.pubkey_hex.trim_start_matches("0x"))
            .context("Invalid public key hex")?;
        let pk_affine = G2Affine::deserialize_compressed(&raw_pk_bytes[..])
            .map_err(|e| anyhow::anyhow!("Invalid G2 public key point: {}", e))?;
        agg_pk_proj += pk_affine;
    }
    let agg_pk_affine: G2Affine = agg_pk_proj.into_affine();

    // Hash message to G1
    let msg_g1 = hash_to_g1(message);

    // Pairing check: e(agg_sig, G2_generator) == e(H(m), agg_public_key)
    let g2_gen = G2Affine::generator();
    let left = Bn254::pairing(agg_point, g2_gen);
    let right = Bn254::pairing(msg_g1, agg_pk_affine);

    Ok(left == right)
}

/// Convert a list of indices into a compact byte bitmask
pub fn indices_to_bitmask(indices: &[usize], total_count: usize) -> Vec<u8> {
    let num_bytes = total_count.div_ceil(8);
    let mut bitmask = vec![0u8; num_bytes];
    for &idx in indices {
        if idx < total_count {
            let byte_idx = idx / 8;
            let bit_idx = idx % 8;
            bitmask[byte_idx] |= 1 << bit_idx;
        }
    }
    bitmask
}

/// Convert a byte bitmask back into participating validator indices
pub fn bitmask_to_indices(bitmask: &[u8]) -> Vec<usize> {
    let mut indices = Vec::new();
    for (byte_idx, &byte_val) in bitmask.iter().enumerate() {
        for bit_idx in 0..8 {
            if (byte_val & (1 << bit_idx)) != 0 {
                indices.push(byte_idx * 8 + bit_idx);
            }
        }
    }
    indices
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;

    #[test]
    fn test_bls_sign_and_verify_single() {
        let mut rng = OsRng;
        let keypair = BlsKeypair::generate(&mut rng);
        let message = b"creg-pbft-v1:prepare:0x1234567890abcdef";

        let sig = keypair.sign(message);
        let pk = keypair.public_key();

        let agg = aggregate_signatures(&[sig], &[0], 1).expect("aggregation");
        let valid = verify_aggregate_signature(&agg, message, &[pk]).expect("verification");
        assert!(valid, "Single signature verification should pass");
    }

    #[test]
    fn test_bls_aggregate_multi_signature_verification() {
        let mut rng = OsRng;
        let total_validators = 7;
        let signers_count = 5; // 5 of 7 quorum

        let mut keypairs = Vec::new();
        let mut pubkeys = Vec::new();
        for _ in 0..total_validators {
            let kp = BlsKeypair::generate(&mut rng);
            pubkeys.push(kp.public_key());
            keypairs.push(kp);
        }

        let message = b"creg-pbft-v1:commit:block_hash_9999";

        // Select first 5 validators to sign
        let participating_indices: Vec<usize> = (0..signers_count).collect();
        let mut signatures = Vec::new();
        for &idx in &participating_indices {
            signatures.push(keypairs[idx].sign(message));
        }

        let agg_sig = aggregate_signatures(&signatures, &participating_indices, total_validators)
            .expect("aggregation");

        assert_eq!(agg_sig.participant_count, 5);

        // Verification must succeed with correct message
        let valid = verify_aggregate_signature(&agg_sig, message, &pubkeys).expect("verification");
        assert!(valid, "Valid aggregate signature should verify");

        // Verification must fail with tampered message
        let invalid =
            verify_aggregate_signature(&agg_sig, b"wrong-message", &pubkeys).expect("verification");
        assert!(!invalid, "Tampered message should fail verification");
    }

    #[test]
    fn test_bitmask_roundtrip() {
        let total = 20;
        let indices = vec![0, 3, 7, 12, 19];
        let bitmask = indices_to_bitmask(&indices, total);
        let restored = bitmask_to_indices(&bitmask);
        assert_eq!(restored, indices);
    }
}
