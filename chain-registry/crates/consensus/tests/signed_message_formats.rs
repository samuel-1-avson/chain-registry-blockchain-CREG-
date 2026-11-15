//! Pins the exact bytes validators sign and the strictness of signature checks.
//!
//! These messages are part of the wire protocol: changing one invalidates
//! every signature produced by a node on the previous format. A failure here
//! means a consensus-breaking change, so update the expected value only with a
//! coordinated protocol version bump.

use consensus::pbft::{pbft_signature_message, verify_pbft_phase_signature};
use consensus::vote_accumulator::{canonical_vote_message, VOTE_MESSAGE_DOMAIN};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

#[test]
fn package_vote_message_format_is_pinned() {
    assert_eq!(VOTE_MESSAGE_DOMAIN, "creg-vote-v2");
    assert_eq!(
        canonical_vote_message(
            "npm:ms@2.1.3",
            "ab12",
            true,
            "validator-pubkey",
            "scanner-profile",
            "evidence-digest",
        ),
        "creg-vote-v2|npm:ms@2.1.3|ab12|true|validator-pubkey|scanner-profile|evidence-digest"
    );
}

#[test]
fn a_vote_signature_cannot_be_replayed_in_another_context() {
    let base = canonical_vote_message("npm:ms@2.1.3", "ab12", true, "pk", "profile", "evidence");
    let variants = [
        canonical_vote_message("npm:ms@2.1.4", "ab12", true, "pk", "profile", "evidence"),
        canonical_vote_message("npm:ms@2.1.3", "ab13", true, "pk", "profile", "evidence"),
        canonical_vote_message("npm:ms@2.1.3", "ab12", false, "pk", "profile", "evidence"),
        canonical_vote_message("npm:ms@2.1.3", "ab12", true, "pk2", "profile", "evidence"),
        canonical_vote_message("npm:ms@2.1.3", "ab12", true, "pk", "profile2", "evidence"),
        canonical_vote_message("npm:ms@2.1.3", "ab12", true, "pk", "profile", "evidence2"),
    ];
    for (index, variant) in variants.iter().enumerate() {
        assert_ne!(
            &base, variant,
            "changing field {index} must change the signed message"
        );
    }
}

#[test]
fn pbft_phase_message_format_is_pinned() {
    assert_eq!(
        pbft_signature_message("prepare", "abc"),
        "creg-pbft-v1:prepare:abc"
    );
    assert_eq!(
        pbft_signature_message("commit", "abc"),
        "creg-pbft-v1:commit:abc"
    );
}

fn signed(key: &SigningKey, phase: &str, block_hash: &str) -> (String, String) {
    let message = pbft_signature_message(phase, block_hash);
    (
        hex::encode(key.verifying_key().as_bytes()),
        hex::encode(key.sign(message.as_bytes()).to_bytes()),
    )
}

#[test]
fn honest_phase_signatures_verify_and_cannot_be_replayed() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let (pubkey, signature) = signed(&key, "commit", "block-a");

    verify_pbft_phase_signature("commit", "block-a", &pubkey, &signature)
        .expect("an honest signature passes strict verification");
    // Accepts the 0x prefix and upper-case hex used by some clients.
    verify_pbft_phase_signature(
        "commit",
        "block-a",
        &format!("0x{}", pubkey.to_ascii_uppercase()),
        &format!("0x{signature}"),
    )
    .expect("0x-prefixed and upper-case keys are accepted");

    assert!(
        verify_pbft_phase_signature("prepare", "block-a", &pubkey, &signature).is_err(),
        "a COMMIT signature must not satisfy PREPARE"
    );
    assert!(
        verify_pbft_phase_signature("commit", "block-b", &pubkey, &signature).is_err(),
        "a signature for one block must not satisfy another"
    );
    let other = SigningKey::from_bytes(&[8u8; 32]);
    let (other_pubkey, _) = signed(&other, "commit", "block-a");
    assert!(
        verify_pbft_phase_signature("commit", "block-a", &other_pubkey, &signature).is_err(),
        "a signature must not verify under a different key"
    );
}

#[test]
fn malformed_keys_and_signatures_are_errors_not_panics() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let (pubkey, signature) = signed(&key, "commit", "block-a");
    for (bad_pubkey, bad_signature) in [
        ("zz", signature.as_str()),
        ("", signature.as_str()),
        (pubkey.as_str(), "zz"),
        (pubkey.as_str(), ""),
        (pubkey.as_str(), &signature[..signature.len() - 2]),
    ] {
        assert!(
            verify_pbft_phase_signature("commit", "block-a", bad_pubkey, bad_signature).is_err()
        );
    }
}

/// The identity point is a small-order ("weak") public key. With R equal to the
/// identity and S equal to zero, the cofactorless verification equation holds
/// for every message, so plain `verify` accepts a "signature" that nobody had
/// to produce. Strict verification must reject it.
#[test]
fn weak_key_signatures_are_rejected_by_phase_verification() {
    let mut identity = [0u8; 32];
    identity[0] = 1;
    let forged: Vec<u8> = identity.iter().copied().chain([0u8; 32]).collect();

    let key = VerifyingKey::from_bytes(&identity).expect("the identity point decompresses");
    let signature = Signature::try_from(forged.as_slice()).expect("64-byte signature");
    let message = pbft_signature_message("commit", "block-a");
    assert!(
        key.verify(message.as_bytes(), &signature).is_ok(),
        "premise: plain verify accepts a weak-key forgery"
    );

    assert!(
        verify_pbft_phase_signature(
            "commit",
            "block-a",
            &hex::encode(identity),
            &hex::encode(&forged),
        )
        .is_err(),
        "strict verification must reject a weak-key forgery"
    );
}
