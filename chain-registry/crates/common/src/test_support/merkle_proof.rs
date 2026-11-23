//! Binary Merkle tree and inclusion-proof helpers for simulations and stress
//! tests (moved from the retired `cross-chain` crate). Leaves and inner nodes
//! are lowercase hex SHA-256 strings; a parent is `sha256(left_hex || right_hex)`
//! and an odd node is paired with itself.

use crate::sha256_hex;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

/// A Merkle inclusion proof for one leaf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleInclusionProof {
    /// Canonical package identifier (e.g. "npm/@scope/foo@1.0.0").
    pub canonical: String,
    /// Leaf hash.
    pub leaf_hash: String,
    /// 0-based index of the leaf in the tree.
    pub leaf_index: usize,
    /// Total number of leaves in the tree.
    pub total_leaves: usize,
    /// Sibling hashes along the path from leaf to root.
    pub audit_path: Vec<String>,
    /// Expected root hash.
    pub root_hash: String,
}

impl MerkleInclusionProof {
    /// Whether the audit path reconstructs `root_hash` from `leaf_hash`.
    pub fn verify(&self) -> bool {
        verify_merkle_path(
            &self.leaf_hash,
            self.leaf_index,
            &self.audit_path,
            &self.root_hash,
        )
    }
}

/// Parent hash of two child nodes.
pub fn hash_pair(left: &str, right: &str) -> String {
    let mut combined = Vec::with_capacity(left.len() + right.len());
    combined.extend_from_slice(left.as_bytes());
    combined.extend_from_slice(right.as_bytes());
    sha256_hex(&combined)
}

/// Build every level of a binary Merkle tree, leaves first. The last level
/// holds exactly one node, the root. An empty leaf list yields the root of a
/// fixed `EMPTY_TREE` sentinel.
pub fn build_merkle_tree(leaves: &[String]) -> Vec<Vec<String>> {
    if leaves.is_empty() {
        return vec![vec![sha256_hex(b"EMPTY_TREE")]];
    }

    let mut levels = Vec::new();
    let mut current_level = leaves.to_vec();
    levels.push(current_level.clone());

    while current_level.len() > 1 {
        let next_level: Vec<String> = current_level
            .chunks(2)
            .map(|chunk| match chunk {
                [left, right] => hash_pair(left, right),
                [single] => hash_pair(single, single),
                _ => unreachable!("chunks(2) yields one or two items"),
            })
            .collect();
        levels.push(next_level.clone());
        current_level = next_level;
    }

    levels
}

/// Root of the tree over `leaves`.
pub fn merkle_root(leaves: &[String]) -> Result<String> {
    build_merkle_tree(leaves)
        .last()
        .and_then(|level| level.first())
        .cloned()
        .ok_or_else(|| anyhow!("Merkle tree has no root"))
}

/// Inclusion proof for the leaf at `leaf_index`.
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
    let root_hash = tree
        .last()
        .and_then(|level| level.first())
        .cloned()
        .ok_or_else(|| anyhow!("Merkle tree has no root"))?;
    let leaf_hash = leaves[leaf_index].clone();

    let mut audit_path = Vec::with_capacity(tree.len().saturating_sub(1));
    let mut current_idx = leaf_index;
    for level in &tree[..tree.len() - 1] {
        let sibling_idx = if current_idx.is_multiple_of(2) {
            if current_idx + 1 < level.len() {
                current_idx + 1
            } else {
                current_idx // odd node: paired with itself
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

/// Verify an audit path from leaf to root.
pub fn verify_merkle_path(
    leaf_hash: &str,
    leaf_index: usize,
    audit_path: &[String],
    expected_root: &str,
) -> bool {
    let mut current = leaf_hash.to_string();
    let mut idx = leaf_index;
    for sibling in audit_path {
        current = if idx.is_multiple_of(2) {
            hash_pair(&current, sibling)
        } else {
            hash_pair(sibling, &current)
        };
        idx /= 2;
    }
    current == expected_root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_leaf_proof_verifies() {
        let leaves = vec![sha256_hex(b"pkg-1")];
        let proof = generate_inclusion_proof(&leaves, "pkg-1", 0).expect("proof generation");
        assert!(proof.verify());
        assert_eq!(merkle_root(&leaves).expect("root"), proof.root_hash);
    }

    #[test]
    fn multi_leaf_proofs_verify_and_detect_tampering() {
        let leaves: Vec<String> = (0..8)
            .map(|i| sha256_hex(format!("package-{i}").as_bytes()))
            .collect();
        for i in 0..leaves.len() {
            let proof = generate_inclusion_proof(&leaves, &format!("pkg-{i}"), i).expect("proof");
            assert!(proof.verify(), "proof for leaf {i} failed");

            let mut tampered = proof.clone();
            tampered.leaf_hash = sha256_hex(b"malicious-tampered-content");
            assert!(!tampered.verify(), "tampered leaf must fail");

            let mut tampered_root = proof.clone();
            tampered_root.root_hash = sha256_hex(b"fake-root");
            assert!(!tampered_root.verify(), "tampered root must fail");
        }
    }

    #[test]
    fn odd_leaf_counts_verify() {
        let leaves: Vec<String> = (0..5)
            .map(|i| sha256_hex(format!("package-odd-{i}").as_bytes()))
            .collect();
        for i in 0..leaves.len() {
            let proof =
                generate_inclusion_proof(&leaves, &format!("pkg-odd-{i}"), i).expect("proof");
            assert!(proof.verify(), "proof for odd leaf {i} failed");
        }
    }

    #[test]
    fn out_of_range_and_empty_inputs_are_errors() {
        assert!(generate_inclusion_proof(&[], "x", 0).is_err());
        let leaves = vec![sha256_hex(b"a")];
        assert!(generate_inclusion_proof(&leaves, "x", 1).is_err());
        assert_eq!(
            merkle_root(&[]).expect("empty root"),
            sha256_hex(b"EMPTY_TREE")
        );
    }
}
