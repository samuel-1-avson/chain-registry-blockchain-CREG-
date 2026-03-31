// crates/consensus/src/vrf.rs
// Verifiable Random Function (VRF) — randomly assigns validator subsets
// to each package submission so colluders can't predict their assignments.

use anyhow::Result;
use common::sha256_hex;

/// Uses a deterministic VRF seed (block height + package canonical ID)
/// to select N validators from the active set without repetition.
/// A real deployment would use a cryptographic VRF (e.g. ECVRF),
/// but SHA-256-based selection is sufficient for this implementation.
pub fn select_validators(
    active_validators: &[String],
    package_canonical: &str,
    block_height: u64,
    n: usize,
) -> Result<Vec<String>> {
    if active_validators.len() < n {
        anyhow::bail!(
            "Need {} validators but only {} are active",
            n,
            active_validators.len()
        );
    }

    // Deterministic seed from block height + package id.
    let seed = sha256_hex(
        format!("{}:{}", block_height, package_canonical).as_bytes()
    );

    // Fisher-Yates shuffle seeded from the VRF output.
    let mut indices: Vec<usize> = (0..active_validators.len()).collect();
    let seed_bytes = hex::decode(&seed)?;

    for i in (1..indices.len()).rev() {
        // Derive a position from successive bytes of the seed.
        let byte_idx = i % seed_bytes.len();
        let j = (seed_bytes[byte_idx] as usize + i) % (i + 1);
        indices.swap(i, j);
    }

    let selected = indices[..n]
        .iter()
        .map(|&i| active_validators[i].clone())
        .collect();

    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_is_deterministic() {
        let validators: Vec<String> = (0..10).map(|i| format!("val_{}", i)).collect();
        let a = select_validators(&validators, "npm:express@4.0.0", 100, 5).unwrap();
        let b = select_validators(&validators, "npm:express@4.0.0", 100, 5).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_packages_get_different_sets() {
        let validators: Vec<String> = (0..20).map(|i| format!("val_{}", i)).collect();
        let a = select_validators(&validators, "npm:express@4.0.0", 100, 5).unwrap();
        let b = select_validators(&validators, "npm:lodash@4.0.0", 100, 5).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn no_duplicate_selections() {
        let validators: Vec<String> = (0..10).map(|i| format!("val_{}", i)).collect();
        let selected = select_validators(&validators, "npm:test@1.0.0", 42, 7).unwrap();
        let unique: std::collections::HashSet<_> = selected.iter().collect();
        assert_eq!(unique.len(), selected.len());
    }
}
