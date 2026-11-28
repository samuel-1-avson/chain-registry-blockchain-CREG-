//! Height-gated validator signatures (ADR-017 §3).
//!
//! Every consensus signature a validator makes (PBFT PREPARE and COMMIT,
//! view-change votes, re-proposal envelopes and validator-set transition
//! approvals) is carried as an Ed25519 hex signature plus an optional
//! ML-DSA-44 hex component. The scheme is decided by the height the
//! signature governs, never by which fields are present:
//!
//! - below the activation height `H`, the ML-DSA-44 component must be absent
//!   and the Ed25519 signature is over the message `M`;
//! - at and above `H`, both components are required and both are over
//!   `M' = "creg-hybrid-sig-v1" || 0x00 || M`, so a stripped Ed25519 half
//!   never verifies as a pre-activation signature.

use crate::crypto::{
    PublicKey, Signature, SignatureScheme, SigningKey, VerifyMode, ED25519_PUBLIC_KEY_LEN,
    ED25519_SIGNATURE_LEN,
};
use crate::vote_v4::{
    HYBRID_SIGNATURE_HEX_LEN, MLDSA44_PUBLIC_KEY_HEX_LEN, MLDSA44_SIGNATURE_HEX_LEN,
};
use anyhow::{bail, Context, Result};

/// Whether hybrid signatures govern `height` under the signed activation
/// height. `None` means hybrid signing is not scheduled.
pub fn hybrid_signatures_required(activation_height: Option<u64>, height: u64) -> bool {
    activation_height.is_some_and(|activation| height >= activation)
}

fn is_lower_hex_of_len(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn decode_ed25519_pubkey(pubkey_hex: &str) -> Result<Vec<u8>> {
    let bytes = hex::decode(pubkey_hex.trim().trim_start_matches("0x"))
        .context("validator Ed25519 public key is not hex")?;
    if bytes.len() != ED25519_PUBLIC_KEY_LEN {
        bail!(
            "validator Ed25519 public key must be {ED25519_PUBLIC_KEY_LEN} bytes, got {}",
            bytes.len()
        );
    }
    Ok(bytes)
}

fn decode_ed25519_signature(signature_hex: &str) -> Result<Vec<u8>> {
    let bytes = hex::decode(signature_hex.trim().trim_start_matches("0x"))
        .context("Ed25519 signature is not hex")?;
    if bytes.len() != ED25519_SIGNATURE_LEN {
        bail!(
            "Ed25519 signature must be {ED25519_SIGNATURE_LEN} bytes, got {}",
            bytes.len()
        );
    }
    Ok(bytes)
}

/// Check the shape of an ML-DSA-44 signature component without verifying it:
/// exactly 4,840 lowercase hex characters. Lengths are checked before any
/// decoding (ADR-017 §3).
pub fn validate_mldsa44_signature_hex(signature_hex: &str) -> Result<()> {
    if !is_lower_hex_of_len(signature_hex, MLDSA44_SIGNATURE_HEX_LEN) {
        bail!("ML-DSA-44 signature must be {MLDSA44_SIGNATURE_HEX_LEN} lowercase hex characters");
    }
    Ok(())
}

/// Verify one validator signature over `message` under the scheme that
/// governs it. `require_hybrid` comes from [`hybrid_signatures_required`] for
/// the height the signature governs.
///
/// Ed25519 verification is always strict (`verify_strict`), as at every
/// consensus signing site before activation.
pub fn verify_validator_signature(
    ed25519_pubkey_hex: &str,
    mldsa44_pubkey_hex: Option<&str>,
    message: &[u8],
    ed25519_signature_hex: &str,
    mldsa44_signature_hex: Option<&str>,
    require_hybrid: bool,
) -> Result<()> {
    let ed_pubkey = decode_ed25519_pubkey(ed25519_pubkey_hex)?;
    let ed_signature = decode_ed25519_signature(ed25519_signature_hex)?;
    if !require_hybrid {
        if mldsa44_signature_hex.is_some() {
            bail!("an ML-DSA-44 signature component is not allowed before hybrid activation");
        }
        let key = PublicKey::from_bytes(SignatureScheme::Ed25519, &ed_pubkey)?;
        let signature = Signature::from_bytes(SignatureScheme::Ed25519, &ed_signature)?;
        key.verify(message, &signature, VerifyMode::Strict)
            .context("Ed25519 signature verification failed")?;
        return Ok(());
    }

    let pq_signature_hex = mldsa44_signature_hex
        .context("a hybrid signature requires its ML-DSA-44 component from hybrid activation on")?;
    validate_mldsa44_signature_hex(pq_signature_hex)?;
    let pq_pubkey_hex = mldsa44_pubkey_hex.map(str::trim).context(
        "validator has no ML-DSA-44 public key, so it cannot sign after hybrid activation",
    )?;
    if !is_lower_hex_of_len(pq_pubkey_hex, MLDSA44_PUBLIC_KEY_HEX_LEN) {
        bail!(
            "validator ML-DSA-44 public key must be {MLDSA44_PUBLIC_KEY_HEX_LEN} lowercase hex characters"
        );
    }
    let pq_pubkey = hex::decode(pq_pubkey_hex).context("ML-DSA-44 public key is not hex")?;
    let pq_signature = hex::decode(pq_signature_hex).context("ML-DSA-44 signature is not hex")?;
    let key = PublicKey::hybrid_from_parts(&ed_pubkey, &pq_pubkey)?;
    let signature = Signature::hybrid_from_parts(&ed_signature, &pq_signature)?;
    key.verify(message, &signature, VerifyMode::Strict)
        .context("hybrid Ed25519 + ML-DSA-44 signature verification failed")?;
    Ok(())
}

/// Split a single-field hybrid signature (4,968 lowercase hex characters:
/// the Ed25519 signature followed by the ML-DSA-44 signature) into its two
/// components. The length is checked before anything is decoded.
pub fn split_hybrid_signature_hex(signature_hex: &str) -> Result<(&str, &str)> {
    if !is_lower_hex_of_len(signature_hex, HYBRID_SIGNATURE_HEX_LEN) {
        bail!("hybrid signature must be {HYBRID_SIGNATURE_HEX_LEN} lowercase hex characters");
    }
    Ok(signature_hex.split_at(ED25519_SIGNATURE_LEN * 2))
}

/// [`verify_validator_signature`] for messages that carry their signature in
/// one hex field (view-change votes, re-proposal envelopes): Ed25519 hex
/// before activation, the full hybrid signature hex from it.
pub fn verify_validator_signature_combined(
    ed25519_pubkey_hex: &str,
    mldsa44_pubkey_hex: Option<&str>,
    message: &[u8],
    signature_hex: &str,
    require_hybrid: bool,
) -> Result<()> {
    if !require_hybrid {
        return verify_validator_signature(
            ed25519_pubkey_hex,
            mldsa44_pubkey_hex,
            message,
            signature_hex,
            None,
            false,
        );
    }
    let (ed25519, mldsa44) = split_hybrid_signature_hex(signature_hex)?;
    verify_validator_signature(
        ed25519_pubkey_hex,
        mldsa44_pubkey_hex,
        message,
        ed25519,
        Some(mldsa44),
        true,
    )
}

/// Sign `message` and split the result into the hex fields that consensus
/// messages carry: the Ed25519 signature and, for a hybrid key, the
/// ML-DSA-44 component.
pub fn sign_validator_message(key: &SigningKey, message: &[u8]) -> (String, Option<String>) {
    let signature = key.sign(message);
    (
        hex::encode(signature.ed25519_bytes()),
        signature.mldsa44_bytes().map(hex::encode),
    )
}

/// Sign `message` for a single-field signature: Ed25519 hex for an Ed25519
/// key, the full hybrid signature hex for a hybrid key.
pub fn sign_validator_message_combined(key: &SigningKey, message: &[u8]) -> String {
    match sign_validator_message(key, message) {
        (ed25519, Some(mldsa44)) => ed25519 + &mldsa44,
        (ed25519, None) => ed25519,
    }
}

/// The Ed25519 part of a single-field signature, normalized. Re-signing the
/// same message with the same key gives the same Ed25519 part, while the
/// hedged ML-DSA-44 part differs, so duplicate detection must compare this
/// (together with the signed message), not the whole field.
pub fn ed25519_part_of_signature_hex(signature_hex: &str) -> String {
    let normalized = signature_hex
        .trim()
        .trim_start_matches("0x")
        .to_ascii_lowercase();
    match normalized.get(..ED25519_SIGNATURE_LEN * 2) {
        Some(ed25519) => ed25519.to_owned(),
        None => normalized,
    }
}

/// The Ed25519 public key (hex) that identifies a validator for either key
/// scheme. A hybrid key is identified by its Ed25519 half.
pub fn validator_ed25519_pubkey_hex(key: &SigningKey) -> String {
    hex::encode(key.public_key().ed25519_bytes())
}

/// The ML-DSA-44 public key (hex) of a hybrid key; `None` for Ed25519.
pub fn validator_mldsa44_pubkey_hex(key: &SigningKey) -> Option<String> {
    key.public_key().mldsa44_bytes().map(hex::encode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> (SigningKey, SigningKey) {
        let ed_seed = [7u8; 32];
        let pq_seed = [9u8; 32];
        (
            SigningKey::from_ed25519_seed(&ed_seed),
            SigningKey::hybrid_from_seeds(&ed_seed, &pq_seed),
        )
    }

    #[test]
    fn classical_signatures_verify_only_before_activation() {
        let (ed, hybrid) = keys();
        let pubkey = validator_ed25519_pubkey_hex(&ed);
        let pq_pubkey = validator_mldsa44_pubkey_hex(&hybrid).unwrap();
        let (signature, pq) = sign_validator_message(&ed, b"message");
        assert!(pq.is_none());
        verify_validator_signature(
            &pubkey,
            Some(&pq_pubkey),
            b"message",
            &signature,
            None,
            false,
        )
        .unwrap();
        assert!(verify_validator_signature(
            &pubkey,
            Some(&pq_pubkey),
            b"message",
            &signature,
            None,
            true
        )
        .is_err());
        assert!(
            verify_validator_signature(&pubkey, None, b"other", &signature, None, false).is_err()
        );
    }

    #[test]
    fn hybrid_signatures_verify_only_from_activation() {
        let (_, hybrid) = keys();
        let pubkey = validator_ed25519_pubkey_hex(&hybrid);
        let pq_pubkey = validator_mldsa44_pubkey_hex(&hybrid).unwrap();
        let (signature, pq) = sign_validator_message(&hybrid, b"message");
        let pq = pq.unwrap();
        verify_validator_signature(
            &pubkey,
            Some(&pq_pubkey),
            b"message",
            &signature,
            Some(&pq),
            true,
        )
        .unwrap();
        // Before activation a hybrid signature is refused outright.
        assert!(verify_validator_signature(
            &pubkey,
            Some(&pq_pubkey),
            b"message",
            &signature,
            Some(&pq),
            false
        )
        .is_err());
        // The Ed25519 half alone is over M', so it is not a classical
        // signature over M either.
        assert!(
            verify_validator_signature(&pubkey, None, b"message", &signature, None, false).is_err()
        );
        // A validator without an ML-DSA-44 key cannot sign after activation.
        assert!(
            verify_validator_signature(&pubkey, None, b"message", &signature, Some(&pq), true)
                .is_err()
        );
        // Wrong message, missing component, malformed component.
        assert!(verify_validator_signature(
            &pubkey,
            Some(&pq_pubkey),
            b"other",
            &signature,
            Some(&pq),
            true
        )
        .is_err());
        assert!(verify_validator_signature(
            &pubkey,
            Some(&pq_pubkey),
            b"message",
            &signature,
            None,
            true
        )
        .is_err());
        let upper = pq.to_ascii_uppercase();
        assert!(verify_validator_signature(
            &pubkey,
            Some(&pq_pubkey),
            b"message",
            &signature,
            Some(&upper),
            true
        )
        .is_err());
        assert!(verify_validator_signature(
            &pubkey,
            Some(&pq_pubkey),
            b"message",
            &signature,
            Some(&pq[..pq.len() - 2]),
            true
        )
        .is_err());
    }

    #[test]
    fn single_field_signatures_follow_the_same_height_rule() {
        let (ed, hybrid) = keys();
        let pubkey = validator_ed25519_pubkey_hex(&hybrid);
        let pq_pubkey = validator_mldsa44_pubkey_hex(&hybrid).unwrap();
        let classical = sign_validator_message_combined(&ed, b"vote");
        let combined = sign_validator_message_combined(&hybrid, b"vote");
        assert_eq!(classical.len(), 128);
        assert_eq!(combined.len(), HYBRID_SIGNATURE_HEX_LEN);

        verify_validator_signature_combined(&pubkey, Some(&pq_pubkey), b"vote", &classical, false)
            .unwrap();
        verify_validator_signature_combined(&pubkey, Some(&pq_pubkey), b"vote", &combined, true)
            .unwrap();
        // Wrong scheme for the height, either way.
        assert!(verify_validator_signature_combined(
            &pubkey,
            Some(&pq_pubkey),
            b"vote",
            &classical,
            true
        )
        .is_err());
        assert!(verify_validator_signature_combined(
            &pubkey,
            Some(&pq_pubkey),
            b"vote",
            &combined,
            false
        )
        .is_err());
        // Only the Ed25519 prefix of a hybrid signature: not a classical
        // signature over M.
        assert!(verify_validator_signature_combined(
            &pubkey,
            Some(&pq_pubkey),
            b"vote",
            &combined[..128],
            false
        )
        .is_err());
        // Swapped halves and truncation are refused.
        let (ed_half, pq_half) = combined.split_at(128);
        let swapped = format!("{pq_half}{ed_half}");
        assert!(verify_validator_signature_combined(
            &pubkey,
            Some(&pq_pubkey),
            b"vote",
            &swapped,
            true
        )
        .is_err());
        assert!(split_hybrid_signature_hex(&combined[..combined.len() - 2]).is_err());

        // Re-signing keeps the Ed25519 part and changes the hedged part.
        let again = sign_validator_message_combined(&hybrid, b"vote");
        assert_eq!(
            ed25519_part_of_signature_hex(&again),
            ed25519_part_of_signature_hex(&combined)
        );
        assert_ne!(again, combined);
        assert_eq!(
            ed25519_part_of_signature_hex(&format!("0x{}", classical.to_ascii_uppercase())),
            classical
        );
    }

    #[test]
    fn activation_height_is_inclusive() {
        assert!(!hybrid_signatures_required(None, 10));
        assert!(!hybrid_signatures_required(Some(11), 10));
        assert!(hybrid_signatures_required(Some(10), 10));
        assert!(hybrid_signatures_required(Some(10), 11));
    }
}
