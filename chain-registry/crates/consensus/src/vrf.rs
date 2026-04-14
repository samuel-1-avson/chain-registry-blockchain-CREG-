// crates/consensus/src/vrf.rs
// Verifiable Random Function (VRF) — randomly assigns validator subsets
// to each package submission so colluders can't predict their assignments.

use anyhow::Result;
use common::sha256_hex;
use rand::{rngs::StdRng, seq::SliceRandom, SeedableRng};

/// Uses a deterministic VRF seed to select N validators from the active set
/// without repetition.
///
/// When a `vrf_output` is provided (the hex-encoded SHA-256 of the proposer's
/// Ed25519 VRF signature), it is used as the shuffle seed — making the
/// selection unpredictable without the proposer's private key.
///
/// Falls back to SHA-256 of public data only when no VRF output is available
/// (dev / bootstrap mode).
pub fn select_validators(
    active_validators: &[String],
    package_canonical: &str,
    block_height: u64,
    n: usize,
    vrf_output: Option<&str>,
) -> Result<Vec<String>> {
    if active_validators.len() < n {
        anyhow::bail!(
            "Need {} validators but only {} are active",
            n,
            active_validators.len()
        );
    }

    // Use VRF output when available; otherwise fall back to deterministic hash.
    let seed = match vrf_output {
        Some(output) => sha256_hex(
            format!("{}:{}:{}", output, block_height, package_canonical).as_bytes(),
        ),
        None => sha256_hex(format!("{}:{}", block_height, package_canonical).as_bytes()),
    };

    // Bias-free Fisher-Yates shuffle using a CSPRNG seeded from the VRF output.
    // The hand-rolled modulo approach used previously introduced statistical
    // bias for validator sets larger than 256 (byte range 0–255 < slot range).
    let mut indices: Vec<usize> = (0..active_validators.len()).collect();
    let seed_bytes = hex::decode(&seed)?;
    let seed_arr: [u8; 32] = seed_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("VRF seed must be exactly 32 bytes (SHA-256 output)"))?;
    let mut rng = StdRng::from_seed(seed_arr);
    indices.shuffle(&mut rng);

    let selected = indices[..n]
        .iter()
        .map(|&i| active_validators[i].clone())
        .collect();

    Ok(selected)
}

/// Minimal VRF proof using deterministic Ed25519 signatures.
/// Returns `(output_hex, proof_hex)` where output = SHA256(signature).
pub fn prove(seed: &[u8], privkey_hex: &str) -> Result<(String, String)> {
    use ed25519_dalek::{Signer, SigningKey};
    let key_bytes = hex::decode(privkey_hex.trim())?;
    let key_arr: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("Private key must be 32 bytes"))?;
    let sk = SigningKey::from_bytes(&key_arr);
    let sig = sk.sign(seed);
    let proof = hex::encode(sig.to_bytes());
    let output = sha256_hex(&sig.to_bytes());
    Ok((output, proof))
}

/// Verify a VRF proof: checks that `proof` is a valid Ed25519 signature over `seed`
/// by `pubkey_hex` and that `sha256(proof) == output_hex`.
pub fn verify(seed: &[u8], pubkey_hex: &str, output_hex: &str, proof_hex: &str) -> Result<()> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let pk_bytes = hex::decode(pubkey_hex)?;
    let vk = VerifyingKey::try_from(pk_bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("Invalid Ed25519 public key"))?;
    let sig_bytes = hex::decode(proof_hex)?;
    let sig = Signature::try_from(sig_bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("Invalid Ed25519 signature"))?;
    vk.verify(seed, &sig)
        .map_err(|_| anyhow::anyhow!("VRF proof verification failed"))?;
    if sha256_hex(&sig_bytes) != output_hex {
        anyhow::bail!("VRF output does not match proof");
    }
    Ok(())
}

/// Lightweight validator info for VRF proposer selection.
#[derive(Debug, Clone)]
pub struct VrfValidator {
    pub id: String,
    pub pubkey: String,
    /// VRF output for this epoch (hex-encoded SHA256 of Ed25519 signature).
    pub vrf_output: Option<String>,
    /// VRF proof for this epoch (hex-encoded Ed25519 signature).
    pub vrf_proof: Option<String>,
}

/// Compute the selection score for a VRF output.
/// Score = SHA256(decoded_output_bytes) so that comparisons are uniform.
fn vrf_score(output_hex: &str) -> Result<String> {
    let bytes = hex::decode(output_hex)?;
    Ok(sha256_hex(&bytes))
}

/// Select the block proposer from the active set using VRF proofs.
///
/// * If a validator provides a `vrf_output` + `vrf_proof`, the proof is verified
///   and the score is derived from the output.
/// * Validators without proofs fall back to deterministic hashing for backward
///   compatibility / dev mode.
/// * The validator with the lowest score is chosen.
pub fn select_proposer(validators: &[VrfValidator], epoch_seed: &str) -> Option<String> {
    validators
        .iter()
        .filter_map(|v| {
            if let (Some(ref output), Some(ref proof)) = (&v.vrf_output, &v.vrf_proof) {
                match verify(epoch_seed.as_bytes(), &v.pubkey, output, proof) {
                    Ok(()) => match vrf_score(output) {
                        Ok(score) => Some((v.id.clone(), score)),
                        Err(_) => None,
                    },
                    Err(_) => None,
                }
            } else {
                let score = sha256_hex(format!("{}:{}", v.pubkey, epoch_seed).as_bytes());
                Some((v.id.clone(), score))
            }
        })
        .min_by(|a, b| a.1.cmp(&b.1))
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    #[test]
    fn selection_is_deterministic() {
        let validators: Vec<String> = (0..10).map(|i| format!("val_{}", i)).collect();
        let a = select_validators(&validators, "npm:express@4.0.0", 100, 5, None).unwrap();
        let b = select_validators(&validators, "npm:express@4.0.0", 100, 5, None).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_packages_get_different_sets() {
        let validators: Vec<String> = (0..20).map(|i| format!("val_{}", i)).collect();
        let a = select_validators(&validators, "npm:express@4.0.0", 100, 5, None).unwrap();
        let b = select_validators(&validators, "npm:lodash@4.0.0", 100, 5, None).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn no_duplicate_selections() {
        let validators: Vec<String> = (0..10).map(|i| format!("val_{}", i)).collect();
        let selected = select_validators(&validators, "npm:test@1.0.0", 42, 7, None).unwrap();
        let unique: std::collections::HashSet<_> = selected.iter().collect();
        assert_eq!(unique.len(), selected.len());
    }

    #[test]
    fn vrf_output_changes_selection() {
        let validators: Vec<String> = (0..20).map(|i| format!("val_{}", i)).collect();
        let a = select_validators(&validators, "npm:express@4.0.0", 100, 5, None).unwrap();
        let b = select_validators(&validators, "npm:express@4.0.0", 100, 5, Some("abcdef1234567890")).unwrap();
        assert_ne!(a, b, "VRF output should change the selection");
    }

    #[test]
    fn vrf_prove_and_verify() {
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        let sk = SigningKey::from_bytes(&bytes);
        let pubkey = hex::encode(sk.verifying_key().as_bytes());
        let privkey = hex::encode(sk.to_bytes());
        let seed = b"epoch_seed_123";
        let (output, proof) = prove(seed, &privkey).unwrap();
        assert!(verify(seed, &pubkey, &output, &proof).is_ok());
    }

    #[test]
    fn proposer_selection_is_deterministic() {
        let validators = vec![
            VrfValidator {
                id: "val_1".into(),
                pubkey: "aa".into(),
                vrf_output: None,
                vrf_proof: None,
            },
            VrfValidator {
                id: "val_2".into(),
                pubkey: "bb".into(),
                vrf_output: None,
                vrf_proof: None,
            },
            VrfValidator {
                id: "val_3".into(),
                pubkey: "cc".into(),
                vrf_output: None,
                vrf_proof: None,
            },
        ];
        let a = select_proposer(&validators, "seed1").unwrap();
        let b = select_proposer(&validators, "seed1").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn vrf_proposer_selects_lowest_output() {
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        let seed = b"epoch_seed_vrf";
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        let sk1 = SigningKey::from_bytes(&bytes);
        rng.fill_bytes(&mut bytes);
        let sk2 = SigningKey::from_bytes(&bytes);
        rng.fill_bytes(&mut bytes);
        let sk3 = SigningKey::from_bytes(&bytes);

        let (out1, prf1) = prove(seed, &hex::encode(sk1.to_bytes())).unwrap();
        let (out2, prf2) = prove(seed, &hex::encode(sk2.to_bytes())).unwrap();
        let (out3, prf3) = prove(seed, &hex::encode(sk3.to_bytes())).unwrap();

        let validators = vec![
            VrfValidator {
                id: "val_2".into(),
                pubkey: hex::encode(sk2.verifying_key().as_bytes()),
                vrf_output: Some(out2),
                vrf_proof: Some(prf2),
            },
            VrfValidator {
                id: "val_1".into(),
                pubkey: hex::encode(sk1.verifying_key().as_bytes()),
                vrf_output: Some(out1),
                vrf_proof: Some(prf1),
            },
            VrfValidator {
                id: "val_3".into(),
                pubkey: hex::encode(sk3.verifying_key().as_bytes()),
                vrf_output: Some(out3),
                vrf_proof: Some(prf3),
            },
        ];

        let winner = select_proposer(&validators, std::str::from_utf8(seed).unwrap()).unwrap();

        // Compute expected winner manually.
        let scores: Vec<(String, String)> = validators
            .iter()
            .map(|v| {
                let out = v.vrf_output.as_ref().unwrap();
                (v.id.clone(), vrf_score(out).unwrap())
            })
            .collect();
        let expected = scores
            .iter()
            .min_by(|a, b| a.1.cmp(&b.1))
            .map(|(id, _)| id.clone())
            .unwrap();

        assert_eq!(winner, expected);
    }

    #[test]
    fn vrf_proposer_rejects_invalid_proof() {
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        let seed = b"epoch_seed_vrf";
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        let sk1 = SigningKey::from_bytes(&bytes);
        rng.fill_bytes(&mut bytes);
        let sk2 = SigningKey::from_bytes(&bytes);

        let (out1, prf1) = prove(seed, &hex::encode(sk1.to_bytes())).unwrap();
        let (_out2, _prf2) = prove(seed, &hex::encode(sk2.to_bytes())).unwrap();

        // Corrupt the proof for val_1.
        let validators = vec![
            VrfValidator {
                id: "val_1".into(),
                pubkey: hex::encode(sk1.verifying_key().as_bytes()),
                vrf_output: Some(out1),
                vrf_proof: Some("deadbeef".repeat(8)),
            },
            VrfValidator {
                id: "val_2".into(),
                pubkey: hex::encode(sk2.verifying_key().as_bytes()),
                vrf_output: None,
                vrf_proof: None,
            },
        ];

        let winner = select_proposer(&validators, std::str::from_utf8(seed).unwrap()).unwrap();
        // val_1 has invalid proof, so it should be skipped; val_2 wins by fallback.
        assert_eq!(winner, "val_2");
    }

    #[test]
    fn vrf_proposer_with_mixed_proofs_and_fallback() {
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        let seed = b"mixed_seed";
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        let sk1 = SigningKey::from_bytes(&bytes);

        let (out1, prf1) = prove(seed, &hex::encode(sk1.to_bytes())).unwrap();

        let validators = vec![
            VrfValidator {
                id: "val_1".into(),
                pubkey: hex::encode(sk1.verifying_key().as_bytes()),
                vrf_output: Some(out1),
                vrf_proof: Some(prf1),
            },
            VrfValidator {
                id: "val_2".into(),
                pubkey: "bb".into(),
                vrf_output: None,
                vrf_proof: None,
            },
        ];

        let winner = select_proposer(&validators, std::str::from_utf8(seed).unwrap()).unwrap();
        // Both are valid candidates; winner depends on scores.
        // Just ensure a winner is selected and it is one of the two.
        assert!(winner == "val_1" || winner == "val_2");
    }
}
