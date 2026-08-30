//! Merkle Proof-of-Inclusion Module
//!
//! Generates and verifies cryptographic Merkle inclusion proofs connecting
//! individual package publications to the root anchored on Ethereum L1/L2.

use anyhow::{bail, Result};
use common::sha256_hex;
use serde::{Deserialize, Serialize};

/// A cryptographic Merkle inclusion proof for a package publication
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleInclusionProof {
    /// Canonical package identifier (e.g. "npm/@scope/foo@1.0.0")
    pub canonical: String,
    /// Leaf hash representing the package publication state
    pub leaf_hash: String,
    /// 0-based index of the leaf in the tree
    pub leaf_index: usize,
    /// Total number of leaves in this tree checkpoint
    pub total_leaves: usize,
    /// Sibling hashes along the path from leaf to root
    pub audit_path: Vec<String>,
    /// Expected Merkle root hash anchored on L1
    pub root_hash: String,
}

impl MerkleInclusionProof {
    /// Verify that this inclusion proof cryptographically reconstructs the expected root hash
    pub fn verify(&self) -> bool {
        verify_merkle_path(
            &self.leaf_hash,
            self.leaf_index,
            &self.audit_path,
            &self.root_hash,
        )
    }
}

/// Compute parent hash of two child nodes in the Merkle tree
pub fn hash_pair(left: &str, right: &str) -> String {
    let mut combined = Vec::with_capacity(64);
    combined.extend_from_slice(left.as_bytes());
    combined.extend_from_slice(right.as_bytes());
    sha256_hex(&combined)
}

/// Build a complete binary Merkle tree over a list of leaf hashes, returning all tree levels
pub fn build_merkle_tree(leaves: &[String]) -> Vec<Vec<String>> {
    if leaves.is_empty() {
        return vec![vec![sha256_hex(b"EMPTY_TREE")]];
    }

    let mut levels = Vec::new();
    let mut current_level = leaves.to_vec();
    levels.push(current_level.clone());

    while current_level.len() > 1 {
        let mut next_level = Vec::new();
        for chunk in current_level.chunks(2) {
            if chunk.len() == 2 {
                next_level.push(hash_pair(&chunk[0], &chunk[1]));
            } else {
                // Odd leaf: pair with itself
                next_level.push(hash_pair(&chunk[0], &chunk[0]));
            }
        }
        levels.push(next_level.clone());
        current_level = next_level;
    }

    levels
}

/// Generate a Merkle inclusion proof for the leaf at `leaf_index`
pub fn generate_inclusion_proof(
    leaves: &[String],
    canonical: &str,
    leaf_index: usize,
) -> Result<MerkleInclusionProof> {
    if leaves.is_empty() {
        bail!("Cannot generate proof for empty leaf list");
    }
    if leaf_index >= leaves.len() {
        bail!(
            "Leaf index {} out of bounds (total {})",
            leaf_index,
            leaves.len()
        );
    }

    let tree = build_merkle_tree(leaves);
    let root_hash = tree.last().unwrap()[0].clone();
    let leaf_hash = leaves[leaf_index].clone();

    let mut audit_path = Vec::new();
    let mut current_idx = leaf_index;

    for level in &tree[..tree.len() - 1] {
        let sibling_idx = if current_idx % 2 == 0 {
            if current_idx + 1 < level.len() {
                current_idx + 1
            } else {
                current_idx // paired with self
            }
        } else {
            current_idx - 1
        };

        audit_path.push(level[sibling_idx].clone());
        current_idx /= 2;
    }

    Ok(MerkleInclusionProof {
        canonical: canonical.to_string(),
        leaf_hash,
        leaf_index,
        total_leaves: leaves.len(),
        audit_path,
        root_hash,
    })
}

/// Verify a Merkle audit path from leaf to root
pub fn verify_merkle_path(
    leaf_hash: &str,
    leaf_index: usize,
    audit_path: &[String],
    expected_root: &str,
) -> bool {
    let mut current = leaf_hash.to_string();
    let mut idx = leaf_index;

    for sibling in audit_path {
        current = if idx % 2 == 0 {
            hash_pair(&current, sibling)
        } else {
            hash_pair(sibling, &current)
        };
        idx /= 2;
    }

    &current == expected_root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_leaf_merkle_proof() {
        let leaves = vec![sha256_hex(b"pkg-1")];
        let proof = generate_inclusion_proof(&leaves, "pkg-1", 0).expect("proof generation");
        assert!(proof.verify());
    }

    #[test]
    fn test_multi_leaf_merkle_proofs() {
        let leaves: Vec<String> = (0..8)
            .map(|i| sha256_hex(format!("package-{}", i).as_bytes()))
            .collect();

        for i in 0..leaves.len() {
            let canonical = format!("pkg-{}", i);
            let proof = generate_inclusion_proof(&leaves, &canonical, i).expect("proof generation");
            assert!(proof.verify(), "Proof for leaf {} failed verification", i);

            // Tampered leaf hash must fail
            let mut tampered = proof.clone();
            tampered.leaf_hash = sha256_hex(b"malicious-tampered-content");
            assert!(!tampered.verify(), "Tampered leaf should fail");

            // Tampered root hash must fail
            let mut tampered_root = proof.clone();
            tampered_root.root_hash = sha256_hex(b"fake-root");
            assert!(!tampered_root.verify(), "Tampered root should fail");
        }
    }

    #[test]
    fn test_odd_leaves_merkle_proofs() {
        let leaves: Vec<String> = (0..5)
            .map(|i| sha256_hex(format!("package-odd-{}", i).as_bytes()))
            .collect();

        for i in 0..leaves.len() {
            let canonical = format!("pkg-odd-{}", i);
            let proof = generate_inclusion_proof(&leaves, &canonical, i).expect("proof generation");
            assert!(proof.verify(), "Proof for odd leaf {} failed verification", i);
        }
    }
}
