//! Golden vectors for consensus hashing.
//!
//! Block hashes, transaction hashes, Merkle roots and view-change signing
//! digests are what validators sign and what nodes compare across the fleet.
//! If any of these values changes, nodes built from different sources stop
//! agreeing, so a failure here means a consensus-breaking change. Update a
//! vector only together with a protocol or chain-ID change.
//!
//! The Phase C build (`phase-c-canonical-transactions`) sorts object keys
//! before hashing, so transaction-level vectors exist per encoding. CI runs
//! this file in both modes.

use common::{
    compiled_consensus_encoding, consensus_json_bytes, merkle_root, sha256_hex, transaction_hash,
    Block, Transaction, ViewChangeVote,
};
use serde::Serialize;

fn revoke(index: u32) -> Transaction {
    Transaction::Revoke {
        package_canonical: format!("npm:golden@1.0.{index}"),
        reason: "golden vector".into(),
        revoked_by: "governance".into(),
        evidence_hash: "ab".repeat(32),
    }
}

fn view_change_vote() -> ViewChangeVote {
    ViewChangeVote {
        height: 5,
        prev_hash: "11".repeat(32),
        new_view: 1,
        validator_id: "validator-1".into(),
        prepared: None,
        signature: String::new(),
    }
}

// ── Encoding-independent vectors ─────────────────────────────────────────────
// `Block::hash` serializes the header directly, so these hold in both modes.

#[test]
fn genesis_block_hash_is_pinned() {
    assert_eq!(
        Block::genesis().hash(),
        "068364cf778ede5a0b1c814c59b0314f9a8598a959fa5a8e08ea78bc1b296b53"
    );
}

#[test]
fn phase_c_genesis_binds_the_spec_identity() {
    let spec_hash = "11".repeat(32);
    let genesis = Block::genesis_for_phase_c(&spec_hash).expect("valid spec identity");
    assert_eq!(genesis.header.proposer_id, "genesis-phase-c-sorted-json-v1");
    assert_eq!(genesis.header.validator_set_hash, spec_hash);
    assert_eq!(
        genesis.hash(),
        "ff96b995cac8f13b228b931958b04b4984381392fd6b0703e66a49c83b80ccd3"
    );

    // A different spec produces a different chain.
    let other = Block::genesis_for_phase_c(&"22".repeat(32)).expect("valid spec identity");
    assert_ne!(genesis.hash(), other.hash());
    // 0x prefixes are accepted, malformed identities are not.
    assert!(Block::genesis_for_phase_c(&format!("0x{spec_hash}")).is_ok());
    assert!(Block::genesis_for_phase_c("1234").is_err());
    assert!(Block::genesis_for_phase_c(&"zz".repeat(32)).is_err());
}

#[test]
fn empty_block_merkle_root_is_pinned() {
    assert_eq!(merkle_root(&[]), sha256_hex(b"empty"));
    assert_eq!(
        merkle_root(&[]),
        "2e1cfa82b035c26cbbbdae632cea070514eb8b773f616aaeaf668e2f0be8f10d"
    );
}

#[test]
fn the_view_number_is_part_of_the_block_hash_but_the_certificate_is_not() {
    let mut view_zero = Block::genesis();
    view_zero.header.height = 1;
    let mut view_one = view_zero.clone();
    view_one.header.view_number = 1;
    assert_ne!(view_zero.hash(), view_one.hash());

    let mut with_certificate = view_one.clone();
    with_certificate
        .header
        .view_change_certificate
        .push(view_change_vote());
    assert_eq!(
        view_one.hash(),
        with_certificate.hash(),
        "the view-change certificate is proof material, not block identity"
    );
}

/// Rebuild the Merkle tree by hand from the transaction hashes, so the
/// pairing rule (and duplication of an odd last leaf) is asserted
/// independently of the library implementation.
#[test]
fn merkle_root_follows_the_documented_pairing_rule() {
    let hashes: Vec<String> = (0..3).map(|i| transaction_hash(&revoke(i))).collect();
    let pair = |a: &str, b: &str| sha256_hex(format!("{a}{b}").as_bytes());

    assert_eq!(merkle_root(&[revoke(0)]), hashes[0]);
    assert_eq!(
        merkle_root(&[revoke(0), revoke(1)]),
        pair(&hashes[0], &hashes[1])
    );
    assert_eq!(
        merkle_root(&[revoke(0), revoke(1), revoke(2)]),
        pair(&pair(&hashes[0], &hashes[1]), &pair(&hashes[2], &hashes[2])),
        "an odd last leaf is paired with itself"
    );
    assert_ne!(
        merkle_root(&[revoke(0), revoke(1)]),
        merkle_root(&[revoke(1), revoke(0)]),
        "order matters"
    );
}

// ── Encoding-dependent vectors ───────────────────────────────────────────────

#[derive(Serialize)]
struct KeyOrderProbe {
    zeta: u8,
    alpha: u8,
}

#[cfg(feature = "phase-c-canonical-transactions")]
mod encoding {
    pub const MODE: Option<&str> = Some("creg-sorted-json-v1");
    pub const PROBE: &str = r#"{"alpha":2,"zeta":1}"#;
    pub const REVOKE_0_HASH: &str =
        "23955061c8cb21c290ed8b2988242141fb57b70fc268a25a5f9a1a2efb578cec";
    pub const MERKLE_ROOT_3: &str =
        "7f15124740911c4b2d41ecb785d58b7a5b03b0860acbbfec8a97fd85fd8ee6e3";
}

#[cfg(not(feature = "phase-c-canonical-transactions"))]
mod encoding {
    pub const MODE: Option<&str> = None;
    pub const PROBE: &str = r#"{"zeta":1,"alpha":2}"#;
    pub const REVOKE_0_HASH: &str =
        "3881106c0a36ab1fe4e1fa6336ec408c82bdebc9f4d40e5f8e2073a362bbe490";
    pub const MERKLE_ROOT_3: &str =
        "ac1c94720332493b816b4d790dd12c84606f07bc90e892aa55a6b7c1c3431b2e";
}

#[test]
fn the_compiled_encoding_matches_the_build_profile() {
    assert_eq!(compiled_consensus_encoding(), encoding::MODE);
}

#[test]
fn object_keys_are_sorted_only_in_the_phase_c_encoding() {
    let bytes = consensus_json_bytes(&KeyOrderProbe { zeta: 1, alpha: 2 }).unwrap();
    assert_eq!(String::from_utf8(bytes).unwrap(), encoding::PROBE);
}

// Phase C only: the legacy encoding writes a JSON value in whatever key order
// serde_json holds, which depends on whether another crate in the build turns
// on serde_json's `preserve_order`. Only the Phase C encoding guarantees sorting.
#[cfg(feature = "phase-c-canonical-transactions")]
#[test]
fn nested_objects_and_arrays_are_encoded_deterministically() {
    // Arrays keep their order; objects nested anywhere are sorted in Phase C.
    let value = serde_json::json!({ "b": [{ "y": 1, "x": 2 }, 3], "a": null });
    let bytes = String::from_utf8(consensus_json_bytes(&value).unwrap()).unwrap();
    assert_eq!(bytes, r#"{"a":null,"b":[{"x":2,"y":1},3]}"#);
}

#[test]
fn transaction_hash_is_pinned() {
    assert_eq!(transaction_hash(&revoke(0)), encoding::REVOKE_0_HASH);
    assert_ne!(transaction_hash(&revoke(0)), transaction_hash(&revoke(1)));
}

#[test]
fn three_transaction_merkle_root_is_pinned() {
    assert_eq!(
        merkle_root(&[revoke(0), revoke(1), revoke(2)]),
        encoding::MERKLE_ROOT_3
    );
}

#[test]
fn view_change_signing_digest_is_pinned() {
    // The payload is a tuple of scalars (no objects), so both encodings agree.
    assert_eq!(
        view_change_vote().signing_message(),
        "dc3faed08f92493df74c805ddb4e6a947311bf81e34e47cb3bd4258df4bfc8a3"
    );

    let mut other_view = view_change_vote();
    other_view.new_view = 2;
    assert_ne!(
        view_change_vote().signing_message(),
        other_view.signing_message()
    );

    // The signature field is excluded, so signing does not change the digest.
    let mut signed = view_change_vote();
    signed.signature = "00".repeat(64);
    assert_eq!(
        view_change_vote().signing_message(),
        signed.signing_message()
    );
}
