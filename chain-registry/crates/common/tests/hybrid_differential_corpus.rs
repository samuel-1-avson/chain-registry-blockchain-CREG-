//! Differential corpus generator and verifier for ADR-017 hybrid signatures
//! (`hybrid-ed25519-mldsa44-v1`).
//!
//! Run `cargo test -p common --test hybrid_differential_corpus` to verify that
//! `common::crypto` matches every expected verdict and `key_id` in
//! `tests/vectors/hybrid_differential_corpus.json`.
//!
//! To regenerate `tests/vectors/hybrid_differential_corpus.json` after an
//! intentional vector update, run:
//! `cargo test -p common --test hybrid_differential_corpus -- --ignored generate_hybrid_differential_corpus`

use common::crypto::{
    PublicKey, Signature, SignatureScheme, SigningKey, VerifyMode, ED25519_PUBLIC_KEY_LEN,
    ED25519_SIGNATURE_LEN, HYBRID_DOMAIN_PREFIX, HYBRID_MLDSA_CONTEXT, KEY_ID_DOMAIN_PREFIX,
    MLDSA44_PUBLIC_KEY_LEN, MLDSA44_SIGNATURE_LEN,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::convert::Infallible;
use std::path::PathBuf;

const CORPUS_SCHEMA_VERSION: u32 = 1;
const CORPUS_PATH_RELATIVE: &str = "tests/vectors/hybrid_differential_corpus.json";
const HYBRID: SignatureScheme = SignatureScheme::HybridEd25519MlDsa44V1;

/// Ed25519 basepoint subgroup order L = 2^252 + 27742317777372353535851937790883648493,
/// stored as four little-endian u64 limbs.
const ED25519_L: [u64; 4] = [
    0x5812_631a_5cf5_d3ed,
    0x14de_f9de_a2f7_9cd6,
    0x0000_0000_0000_0000,
    0x1000_0000_0000_0000,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Corpus {
    schema_version: u32,
    scheme: String,
    domain_prefix: String,
    mldsa44_context: String,
    cases: Vec<CorpusCase>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CorpusCase {
    id: String,
    category: String,
    description: String,
    ed25519_pk: String,
    mldsa44_pk: String,
    ed25519_sig: String,
    mldsa44_sig: String,
    message: String,
    expected_valid: bool,
    expected_key_id: Option<String>,
}

struct FixedBytesRng<'a>(&'a [u8]);

impl ml_dsa::signature::rand_core::TryRng for FixedBytesRng<'_> {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        unreachable!("ML-DSA signing uses try_fill_bytes")
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        unreachable!("ML-DSA signing uses try_fill_bytes")
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        assert_eq!(dst.len(), self.0.len(), "unexpected rnd length");
        dst.copy_from_slice(self.0);
        Ok(())
    }
}

impl ml_dsa::signature::rand_core::TryCryptoRng for FixedBytesRng<'_> {}

fn hybrid_message(message: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HYBRID_DOMAIN_PREFIX.len() + 1 + message.len());
    out.extend_from_slice(HYBRID_DOMAIN_PREFIX);
    out.push(0x00);
    out.extend_from_slice(message);
    out
}

fn raw_key_id_if_valid_lengths(ed_pk: &[u8], pq_pk: &[u8]) -> Option<String> {
    if ed_pk.len() != ED25519_PUBLIC_KEY_LEN || pq_pk.len() != MLDSA44_PUBLIC_KEY_LEN {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(KEY_ID_DOMAIN_PREFIX);
    hasher.update([0x00]);
    hasher.update(ed_pk);
    hasher.update(pq_pk);
    Some(hex::encode(hasher.finalize()))
}

fn limbs_ge(a: &[u64; 4], b: &[u64; 4]) -> bool {
    for idx in (0..4).rev() {
        if a[idx] != b[idx] {
            return a[idx] > b[idx];
        }
    }
    true
}

fn limbs_add_in_place(a: &mut [u64; 4], b: &[u64; 4]) -> u64 {
    let mut carry = 0u128;
    for idx in 0..4 {
        let sum = u128::from(a[idx]) + u128::from(b[idx]) + carry;
        a[idx] = sum as u64;
        carry = sum >> 64;
    }
    carry as u64
}

fn limbs_sub_in_place(a: &mut [u64; 4], b: &[u64; 4]) {
    let mut borrow = 0i128;
    for idx in 0..4 {
        let diff = i128::from(a[idx]) - i128::from(b[idx]) - borrow;
        if diff < 0 {
            a[idx] = (diff + (1i128 << 64)) as u64;
            borrow = 1;
        } else {
            a[idx] = diff as u64;
            borrow = 0;
        }
    }
    assert_eq!(borrow, 0);
}

fn add_mod_l(mut a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let carry = limbs_add_in_place(&mut a, &b);
    assert_eq!(carry, 0, "operands < L never overflow 256 bits");
    if limbs_ge(&a, &ED25519_L) {
        limbs_sub_in_place(&mut a, &ED25519_L);
    }
    a
}

fn shl1_mod_l(mut a: [u64; 4]) -> [u64; 4] {
    let top = a[3] >> 63;
    assert_eq!(top, 0, "operand < L < 2^253 never shifts out");
    a[3] = (a[3] << 1) | (a[2] >> 63);
    a[2] = (a[2] << 1) | (a[1] >> 63);
    a[1] = (a[1] << 1) | (a[0] >> 63);
    a[0] <<= 1;
    if limbs_ge(&a, &ED25519_L) {
        limbs_sub_in_place(&mut a, &ED25519_L);
    }
    a
}

fn reduce_le_bytes_mod_l(bytes: &[u8]) -> [u64; 4] {
    let mut acc = [0u64; 4];
    for &byte in bytes.iter().rev() {
        for bit in (0..8).rev() {
            acc = shl1_mod_l(acc);
            if (byte >> bit) & 1 == 1 {
                acc = add_mod_l(acc, [1, 0, 0, 0]);
            }
        }
    }
    acc
}

fn mul_mod_l(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut acc = [0u64; 4];
    for limb_idx in (0..4).rev() {
        for bit in (0..64).rev() {
            acc = shl1_mod_l(acc);
            if (b[limb_idx] >> bit) & 1 == 1 {
                acc = add_mod_l(acc, a);
            }
        }
    }
    acc
}

fn limbs_to_le_bytes(limbs: [u64; 4]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (idx, limb) in limbs.iter().enumerate() {
        out[idx * 8..(idx + 1) * 8].copy_from_slice(&limb.to_le_bytes());
    }
    out
}

fn le_bytes_to_limbs(bytes: &[u8; 32]) -> [u64; 4] {
    let mut limbs = [0u64; 4];
    for (idx, limb) in limbs.iter_mut().enumerate() {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&bytes[idx * 8..(idx + 1) * 8]);
        *limb = u64::from_le_bytes(buf);
    }
    limbs
}

/// Derive the RFC 8032 clamped Ed25519 private scalar mod L from a 32-byte seed.
fn ed25519_clamped_scalar_mod_l(seed: &[u8; 32]) -> [u64; 4] {
    let digest = Sha512::digest(seed);
    let mut clamped = [0u8; 32];
    clamped.copy_from_slice(&digest[..32]);
    clamped[0] &= 248;
    clamped[31] &= 127;
    clamped[31] |= 64;
    reduce_le_bytes_mod_l(&clamped)
}

#[allow(clippy::too_many_arguments)]
fn make_case(
    id: &str,
    category: &str,
    description: &str,
    ed_pk: &[u8],
    pq_pk: &[u8],
    ed_sig: &[u8],
    pq_sig: &[u8],
    message: &[u8],
    expected_valid: bool,
) -> CorpusCase {
    CorpusCase {
        id: id.to_string(),
        category: category.to_string(),
        description: description.to_string(),
        ed25519_pk: hex::encode(ed_pk),
        mldsa44_pk: hex::encode(pq_pk),
        ed25519_sig: hex::encode(ed_sig),
        mldsa44_sig: hex::encode(pq_sig),
        message: hex::encode(message),
        expected_valid,
        expected_key_id: raw_key_id_if_valid_lengths(ed_pk, pq_pk),
    }
}

fn build_corpus() -> Corpus {
    let sk1 = SigningKey::hybrid_from_seeds(&[0x11u8; 32], &[0x22u8; 32]);
    let sk2 = SigningKey::hybrid_from_seeds(&[0x33u8; 32], &[0x44u8; 32]);
    let pk1 = sk1.public_key();
    let pk2 = sk2.public_key();
    let (pk1_ed, pk1_pq) = pk1.hybrid_parts().unwrap();
    let (pk2_ed, pk2_pq) = pk2.hybrid_parts().unwrap();

    let pq1_sk = ml_dsa::SigningKey::<ml_dsa::MlDsa44>::from_seed(&ml_dsa::Seed::from(
        *sk1.mldsa44_seed().unwrap(),
    ));
    let pq2_sk = ml_dsa::SigningKey::<ml_dsa::MlDsa44>::from_seed(&ml_dsa::Seed::from(
        *sk2.mldsa44_seed().unwrap(),
    ));

    let det_sign = |ed_seed: &[u8; 32],
                    pq_sk: &ml_dsa::SigningKey<ml_dsa::MlDsa44>,
                    msg: &[u8]|
     -> ([u8; 64], Vec<u8>) {
        use ed25519_dalek::Signer;
        let m_prime = hybrid_message(msg);
        let ed_sk = ed25519_dalek::SigningKey::from_bytes(ed_seed);
        let ed_sig = ed_sk.sign(&m_prime).to_bytes();
        let pq_sig = pq_sk
            .expanded_key()
            .sign_deterministic(&m_prime, HYBRID_MLDSA_CONTEXT)
            .expect("deterministic ML-DSA-44 signing succeeds")
            .encode()
            .as_slice()
            .to_vec();
        (ed_sig, pq_sig)
    };

    let mut cases = Vec::new();

    // ── 1. Valid cases (5) ──────────────────────────────────────────────────

    let msg_empty = b"";
    let (ed_empty, pq_empty) = det_sign(&sk1.ed25519_seed(), &pq1_sk, msg_empty);
    cases.push(make_case(
        "valid_empty_message_deterministic",
        "valid",
        "Empty protocol message M signed deterministically under M' and creg-hybrid-sig-v1 context",
        &pk1_ed,
        &pk1_pq,
        &ed_empty,
        &pq_empty,
        msg_empty,
        true,
    ));

    let msg_pbft = b"creg-pbft-v1:commit:0123456789abcdef";
    let (ed1, pq1) = det_sign(&sk1.ed25519_seed(), &pq1_sk, msg_pbft);
    cases.push(make_case(
        "valid_pbft_commit_deterministic",
        "valid",
        "PBFT commit payload signed deterministically by signer 1",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &pq1,
        msg_pbft,
        true,
    ));

    let mut msg_binary = Vec::with_capacity(512);
    msg_binary.extend_from_slice(b"creg-hybrid-sig-v1\x00");
    while msg_binary.len() < 512 {
        let idx = msg_binary.len();
        msg_binary.push(match idx % 3 {
            0 => 0x00,
            1 => 0xff,
            _ => (idx & 0xff) as u8,
        });
    }
    let (ed2, pq2) = det_sign(&sk2.ed25519_seed(), &pq2_sk, &msg_binary);
    cases.push(make_case(
        "valid_binary_nul_payload_deterministic",
        "valid",
        "512-byte binary message containing NUL, 0xff, and embedded hybrid prefix signed by signer 2",
        &pk2_ed,
        &pk2_pq,
        &ed2,
        &pq2,
        &msg_binary,
        true,
    ));

    let msg_hedged = b"creg-pbft-v1:prepare:hedged-check";
    let m_prime_hedged = hybrid_message(msg_hedged);
    let ed_hedged = {
        use ed25519_dalek::Signer;
        ed25519_dalek::SigningKey::from_bytes(&sk1.ed25519_seed())
            .sign(&m_prime_hedged)
            .to_bytes()
    };
    let pq_hedged_a = pq1_sk
        .expanded_key()
        .sign_randomized(
            &m_prime_hedged,
            HYBRID_MLDSA_CONTEXT,
            &mut FixedBytesRng(&[0xaau8; 32]),
        )
        .expect("hedged signing succeeds")
        .encode()
        .as_slice()
        .to_vec();
    let pq_hedged_b = pq1_sk
        .expanded_key()
        .sign_randomized(
            &m_prime_hedged,
            HYBRID_MLDSA_CONTEXT,
            &mut FixedBytesRng(&[0xbbu8; 32]),
        )
        .expect("hedged signing succeeds")
        .encode()
        .as_slice()
        .to_vec();
    assert_ne!(pq_hedged_a, pq_hedged_b);

    cases.push(make_case(
        "valid_hedged_rnd_a",
        "valid",
        "Hedged ML-DSA-44 signature produced with fixed rnd = 0xaa*32",
        &pk1_ed,
        &pk1_pq,
        &ed_hedged,
        &pq_hedged_a,
        msg_hedged,
        true,
    ));
    cases.push(make_case(
        "valid_hedged_rnd_b",
        "valid",
        "Hedged ML-DSA-44 signature for the same key and message with rnd = 0xbb*32",
        &pk1_ed,
        &pk1_pq,
        &ed_hedged,
        &pq_hedged_b,
        msg_hedged,
        true,
    ));

    // ── 2. Wrong length cases (7) ───────────────────────────────────────────

    cases.push(make_case(
        "wrong_ed25519_pk_len_short",
        "wrong_length",
        "Ed25519 public key truncated to 31 bytes",
        &pk1_ed[..31],
        &pk1_pq,
        &ed1,
        &pq1,
        msg_pbft,
        false,
    ));
    let mut pk1_ed_long = pk1_ed.to_vec();
    pk1_ed_long.push(0x00);
    cases.push(make_case(
        "wrong_ed25519_pk_len_long",
        "wrong_length",
        "Ed25519 public key extended to 33 bytes",
        &pk1_ed_long,
        &pk1_pq,
        &ed1,
        &pq1,
        msg_pbft,
        false,
    ));
    cases.push(make_case(
        "wrong_mldsa44_pk_len_short",
        "wrong_length",
        "ML-DSA-44 public key truncated to 1311 bytes",
        &pk1_ed,
        &pk1_pq[..MLDSA44_PUBLIC_KEY_LEN - 1],
        &ed1,
        &pq1,
        msg_pbft,
        false,
    ));
    let mut pk1_pq_long = pk1_pq.clone();
    pk1_pq_long.push(0x00);
    cases.push(make_case(
        "wrong_mldsa44_pk_len_long",
        "wrong_length",
        "ML-DSA-44 public key extended to 1313 bytes",
        &pk1_ed,
        &pk1_pq_long,
        &ed1,
        &pq1,
        msg_pbft,
        false,
    ));
    cases.push(make_case(
        "wrong_ed25519_sig_len_short",
        "wrong_length",
        "Ed25519 signature truncated to 63 bytes",
        &pk1_ed,
        &pk1_pq,
        &ed1[..ED25519_SIGNATURE_LEN - 1],
        &pq1,
        msg_pbft,
        false,
    ));
    cases.push(make_case(
        "wrong_mldsa44_sig_len_short",
        "wrong_length",
        "ML-DSA-44 signature truncated to 2419 bytes",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &pq1[..MLDSA44_SIGNATURE_LEN - 1],
        msg_pbft,
        false,
    ));
    let mut pq1_sig_long = pq1.clone();
    pq1_sig_long.push(0x00);
    cases.push(make_case(
        "wrong_mldsa44_sig_len_long",
        "wrong_length",
        "ML-DSA-44 signature extended to 2421 bytes",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &pq1_sig_long,
        msg_pbft,
        false,
    ));

    // ── 3. Malformed ML-DSA-44 encodings (6) ────────────────────────────────
    // Layout: c_tilde[0..32], z[32..2336], hint[2336..2420] = indices[0..80] || cuts[80..84]
    const HINT_OFF: usize = 2336;
    const CUTS_OFF: usize = 2336 + 80;

    let mut repeated_hint = pq1.clone();
    repeated_hint[HINT_OFF..].fill(0);
    repeated_hint[HINT_OFF] = 5;
    repeated_hint[HINT_OFF + 1] = 5;
    repeated_hint[CUTS_OFF..].copy_from_slice(&[2, 2, 2, 2]);
    cases.push(make_case(
        "mldsa44_cve_2026_24850_repeated_hint_indices",
        "malformed_mldsa44",
        "CVE-2026-24850: repeated hint indices [5, 5] within the first polynomial",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &repeated_hint,
        msg_pbft,
        false,
    ));

    let mut unordered_hint = pq1.clone();
    unordered_hint[HINT_OFF..].fill(0);
    unordered_hint[HINT_OFF] = 10;
    unordered_hint[HINT_OFF + 1] = 5;
    unordered_hint[CUTS_OFF..].copy_from_slice(&[2, 2, 2, 2]);
    cases.push(make_case(
        "mldsa44_out_of_order_hint_indices",
        "malformed_mldsa44",
        "Decreasing hint indices [10, 5] within the first polynomial",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &unordered_hint,
        msg_pbft,
        false,
    ));

    let mut nonzero_pad_hint = pq1.clone();
    nonzero_pad_hint[HINT_OFF..].fill(0);
    nonzero_pad_hint[HINT_OFF] = 5;
    nonzero_pad_hint[HINT_OFF + 1] = 1;
    nonzero_pad_hint[CUTS_OFF..].copy_from_slice(&[1, 1, 1, 1]);
    cases.push(make_case(
        "mldsa44_nonzero_trailing_hint_padding",
        "malformed_mldsa44",
        "Non-zero padding byte past final hint cut",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &nonzero_pad_hint,
        msg_pbft,
        false,
    ));

    let mut non_mono_cuts = pq1.clone();
    non_mono_cuts[HINT_OFF..].fill(0);
    non_mono_cuts[HINT_OFF] = 1;
    non_mono_cuts[HINT_OFF + 1] = 2;
    non_mono_cuts[CUTS_OFF..].copy_from_slice(&[2, 1, 2, 2]);
    cases.push(make_case(
        "mldsa44_non_monotonic_hint_cuts",
        "malformed_mldsa44",
        "Non-monotonic hint cut array [2, 1, 2, 2]",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &non_mono_cuts,
        msg_pbft,
        false,
    ));

    let mut over_omega_cuts = pq1.clone();
    over_omega_cuts[HINT_OFF..].fill(0);
    over_omega_cuts[CUTS_OFF..].copy_from_slice(&[81, 81, 81, 81]);
    cases.push(make_case(
        "mldsa44_hint_cut_exceeds_omega",
        "malformed_mldsa44",
        "Hint cut value 81 exceeds ML-DSA-44 omega bound (80)",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &over_omega_cuts,
        msg_pbft,
        false,
    ));

    let mut out_of_range_z = pq1.clone();
    out_of_range_z[32] = 0;
    out_of_range_z[33] = 0;
    out_of_range_z[34] &= 0b1111_1100;
    cases.push(make_case(
        "mldsa44_out_of_range_z",
        "malformed_mldsa44",
        "First packed 18-bit z coefficient zeroed so z_0 = gamma1 >= gamma1 - beta",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &out_of_range_z,
        msg_pbft,
        false,
    ));

    // ── 4. Wrong/empty ML-DSA context (2) ───────────────────────────────────

    let m_prime_pbft = hybrid_message(msg_pbft);
    let pq_wrong_ctx = pq1_sk
        .expanded_key()
        .sign_deterministic(&m_prime_pbft, b"wrong-context")
        .unwrap()
        .encode()
        .as_slice()
        .to_vec();
    cases.push(make_case(
        "mldsa44_wrong_context",
        "wrong_context",
        "Valid Ed25519 over M' paired with ML-DSA-44 signed with ctx = 'wrong-context'",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &pq_wrong_ctx,
        msg_pbft,
        false,
    ));

    let pq_empty_ctx = pq1_sk
        .expanded_key()
        .sign_deterministic(&m_prime_pbft, b"")
        .unwrap()
        .encode()
        .as_slice()
        .to_vec();
    cases.push(make_case(
        "mldsa44_empty_context",
        "wrong_context",
        "Valid Ed25519 over M' paired with ML-DSA-44 signed with empty context ''",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &pq_empty_ctx,
        msg_pbft,
        false,
    ));

    // ── 5. Tampering, non-separability, and key substitution (9) ────────────

    let mut ed1_flipped = ed1;
    ed1_flipped[0] ^= 0x01;
    cases.push(make_case(
        "tampered_ed25519_sig_bitflip",
        "tampered",
        "Single bit flipped in Ed25519 signature half while ML-DSA-44 half stays valid",
        &pk1_ed,
        &pk1_pq,
        &ed1_flipped,
        &pq1,
        msg_pbft,
        false,
    ));

    let mut pq1_flipped = pq1.clone();
    pq1_flipped[0] ^= 0x01;
    cases.push(make_case(
        "tampered_mldsa44_sig_bitflip",
        "tampered",
        "Single bit flipped in ML-DSA-44 signature commitment c_tilde while Ed25519 half stays valid",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &pq1_flipped,
        msg_pbft,
        false,
    ));

    cases.push(make_case(
        "tampered_message",
        "tampered",
        "Valid hybrid signature checked against a different message",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &pq1,
        b"creg-pbft-v1:commit:different-hash",
        false,
    ));

    let (ed1_other, pq1_other) = det_sign(&sk1.ed25519_seed(), &pq1_sk, b"other-msg");
    cases.push(make_case(
        "swapped_components_ed1_pq2",
        "non_separability",
        "Ed25519 half for msg1 paired with ML-DSA-44 half for msg2 (same key)",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &pq1_other,
        msg_pbft,
        false,
    ));
    cases.push(make_case(
        "swapped_components_ed2_pq1",
        "non_separability",
        "Ed25519 half for msg2 paired with ML-DSA-44 half for msg1 (same key)",
        &pk1_ed,
        &pk1_pq,
        &ed1_other,
        &pq1,
        msg_pbft,
        false,
    ));

    let plain_ed_sig = {
        use ed25519_dalek::Signer;
        ed25519_dalek::SigningKey::from_bytes(&sk1.ed25519_seed())
            .sign(msg_pbft)
            .to_bytes()
    };
    cases.push(make_case(
        "non_separability_plain_ed25519_over_m_with_valid_mldsa_over_m_prime",
        "non_separability",
        "Downgrade attempt: Ed25519 signature over raw M paired with valid ML-DSA-44 over M'",
        &pk1_ed,
        &pk1_pq,
        &plain_ed_sig,
        &pq1,
        msg_pbft,
        false,
    ));

    let pq_over_raw_m = pq1_sk
        .expanded_key()
        .sign_deterministic(msg_pbft, HYBRID_MLDSA_CONTEXT)
        .unwrap()
        .encode()
        .as_slice()
        .to_vec();
    cases.push(make_case(
        "non_separability_mldsa_over_m_instead_of_m_prime",
        "non_separability",
        "Valid Ed25519 over M' paired with ML-DSA-44 signed over raw M instead of M'",
        &pk1_ed,
        &pk1_pq,
        &ed1,
        &pq_over_raw_m,
        msg_pbft,
        false,
    ));

    cases.push(make_case(
        "substituted_ed25519_pk_from_signer_2",
        "key_substitution",
        "Signer 2 Ed25519 pk paired with signer 1 ML-DSA-44 pk under signer 1 signature",
        &pk2_ed,
        &pk1_pq,
        &ed1,
        &pq1,
        msg_pbft,
        false,
    ));
    cases.push(make_case(
        "substituted_mldsa44_pk_from_signer_2",
        "key_substitution",
        "Signer 1 Ed25519 pk paired with signer 2 ML-DSA-44 pk under signer 1 signature",
        &pk1_ed,
        &pk2_pq,
        &ed1,
        &pq1,
        msg_pbft,
        false,
    ));

    // ── 6. Strict Ed25519 & invalid curve point cases (5) ───────────────────

    let mut identity_point = [0u8; 32];
    identity_point[0] = 1;
    let mut forged_identity_ed_sig = [0u8; 64];
    forged_identity_ed_sig[0] = 1; // R = identity, S = 0

    cases.push(make_case(
        "ed25519_small_order_identity_pk_forged_sig_with_valid_mldsa",
        "strict_ed25519",
        "Order-1 Ed25519 identity pk with (R=identity, S=0) and valid ML-DSA-44 over M'",
        &identity_point,
        &pk1_pq,
        &forged_identity_ed_sig,
        &pq1,
        msg_pbft,
        false,
    ));

    let order8_pk = hex::decode("c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a")
        .expect("valid hex");
    cases.push(make_case(
        "ed25519_small_order_order8_pk_forged_sig_with_valid_mldsa",
        "strict_ed25519",
        "Order-8 Ed25519 torsion pk with (R=identity, S=0) and valid ML-DSA-44 over M'",
        &order8_pk,
        &pk1_pq,
        &forged_identity_ed_sig,
        &pq1,
        msg_pbft,
        false,
    ));

    // Honest prime-order pk1_ed paired with order-2 torsion R and S = (H(R||A||M') * a) mod L:
    // satisfies cofactored [8](R + kA) == [8]SB while failing cofactorless/strict verification.
    let order2_r = hex::decode("ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f")
        .expect("valid hex");
    let mut k_hasher = Sha512::new();
    k_hasher.update(&order2_r);
    k_hasher.update(pk1_ed);
    k_hasher.update(&m_prime_pbft);
    let k_limbs = reduce_le_bytes_mod_l(&k_hasher.finalize());
    let a_limbs = ed25519_clamped_scalar_mod_l(&sk1.ed25519_seed());
    let s_forged_limbs = mul_mod_l(k_limbs, a_limbs);
    let mut cofactored_ed_sig = [0u8; 64];
    cofactored_ed_sig[..32].copy_from_slice(&order2_r);
    cofactored_ed_sig[32..].copy_from_slice(&limbs_to_le_bytes(s_forged_limbs));
    cases.push(make_case(
        "ed25519_small_order_r_cofactored_forgery_with_valid_mldsa",
        "strict_ed25519",
        "Honest Ed25519 pk with order-2 R where cofactored [8]R=[8](SB-kA) holds but verify_strict rejects",
        &pk1_ed,
        &pk1_pq,
        &cofactored_ed_sig,
        &pq1,
        msg_pbft,
        false,
    ));

    // Non-canonical scalar S + L >= L (still < 2^254 so no 256-bit wrap).
    let mut s_raw = [0u8; 32];
    s_raw.copy_from_slice(&ed1[32..]);
    let mut s_plus_l = le_bytes_to_limbs(&s_raw);
    let carry = limbs_add_in_place(&mut s_plus_l, &ED25519_L);
    assert_eq!(carry, 0, "S + L must fit in 32 bytes without overflow");
    let mut non_canon_s_ed_sig = ed1;
    non_canon_s_ed_sig[32..].copy_from_slice(&limbs_to_le_bytes(s_plus_l));
    cases.push(make_case(
        "ed25519_non_canonical_s_ge_l_with_valid_mldsa",
        "strict_ed25519",
        "Valid Ed25519 signature mutated to non-canonical S + L >= L",
        &pk1_ed,
        &pk1_pq,
        &non_canon_s_ed_sig,
        &pq1,
        msg_pbft,
        false,
    ));

    // 32-byte Ed25519 encoding that does not decompress onto Curve25519.
    let mut non_curve_pk = [0u8; 32];
    let mut found_non_curve = false;
    for byte in 2u8..=u8::MAX {
        let candidate = [byte; 32];
        if ed25519_dalek::VerifyingKey::from_bytes(&candidate).is_err() {
            non_curve_pk = candidate;
            found_non_curve = true;
            break;
        }
    }
    assert!(found_non_curve, "must find a 32-byte non-curve point");
    cases.push(make_case(
        "ed25519_invalid_pk_not_on_curve",
        "strict_ed25519",
        "32-byte Ed25519 public key that fails Edwards point decompression",
        &non_curve_pk,
        &pk1_pq,
        &ed1,
        &pq1,
        msg_pbft,
        false,
    ));

    assert_eq!(cases.len(), 34);

    Corpus {
        schema_version: CORPUS_SCHEMA_VERSION,
        scheme: HYBRID.id().to_string(),
        domain_prefix: String::from_utf8(HYBRID_DOMAIN_PREFIX.to_vec()).unwrap(),
        mldsa44_context: String::from_utf8(HYBRID_MLDSA_CONTEXT.to_vec()).unwrap(),
        cases,
    }
}

fn corpus_file_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CORPUS_PATH_RELATIVE)
}

#[test]
#[ignore = "run explicitly to regenerate vectors/hybrid_differential_corpus.json"]
fn generate_hybrid_differential_corpus() {
    let corpus = build_corpus();
    let mut json = serde_json::to_string_pretty(&corpus).expect("serialize corpus JSON");
    json.push('\n');
    std::fs::write(corpus_file_path(), json.as_bytes()).expect("write corpus JSON");
}

#[test]
fn rust_verifies_checked_in_hybrid_differential_corpus() {
    let raw = std::fs::read_to_string(corpus_file_path())
        .expect("read checked-in hybrid_differential_corpus.json");
    let corpus: Corpus = serde_json::from_str(&raw).expect("parse corpus JSON");

    // Prevent fixture drift: checked-in JSON must match the deterministic Rust builder.
    assert_eq!(
        corpus,
        build_corpus(),
        "checked-in hybrid_differential_corpus.json drifted from build_corpus(); regenerate with --ignored generate_hybrid_differential_corpus"
    );

    assert_eq!(corpus.schema_version, CORPUS_SCHEMA_VERSION);
    assert_eq!(corpus.scheme, "hybrid-ed25519-mldsa44-v1");
    assert_eq!(corpus.domain_prefix, "creg-hybrid-sig-v1");
    assert_eq!(corpus.mldsa44_context, "creg-hybrid-sig-v1");
    assert!(!corpus.cases.is_empty());

    let mut valid_count = 0usize;
    let mut invalid_count = 0usize;

    for case in &corpus.cases {
        let ed_pk = hex::decode(&case.ed25519_pk).expect("valid lowercase hex ed25519_pk");
        let pq_pk = hex::decode(&case.mldsa44_pk).expect("valid lowercase hex mldsa44_pk");
        let ed_sig = hex::decode(&case.ed25519_sig).expect("valid lowercase hex ed25519_sig");
        let pq_sig = hex::decode(&case.mldsa44_sig).expect("valid lowercase hex mldsa44_sig");
        let msg = if case.message.is_empty() {
            Vec::new()
        } else {
            hex::decode(&case.message).expect("valid lowercase hex message")
        };

        assert_eq!(
            raw_key_id_if_valid_lengths(&ed_pk, &pq_pk),
            case.expected_key_id,
            "raw key_id mismatch for case {}",
            case.id
        );

        let mut concat_pk = Vec::with_capacity(ed_pk.len() + pq_pk.len());
        concat_pk.extend_from_slice(&ed_pk);
        concat_pk.extend_from_slice(&pq_pk);

        let mut concat_sig = Vec::with_capacity(ed_sig.len() + pq_sig.len());
        concat_sig.extend_from_slice(&ed_sig);
        concat_sig.extend_from_slice(&pq_sig);

        for mode in [VerifyMode::Strict, VerifyMode::Permissive] {
            let parts_verdict = (|| -> Result<(), common::crypto::CryptoError> {
                let pk = PublicKey::hybrid_from_parts(&ed_pk, &pq_pk)?;
                assert_eq!(
                    Some(pk.key_id()),
                    case.expected_key_id,
                    "PublicKey::key_id mismatch for case {}",
                    case.id
                );
                let sig = Signature::hybrid_from_parts(&ed_sig, &pq_sig)?;
                pk.verify(&msg, &sig, mode)
            })();

            let concat_verdict = (|| -> Result<(), common::crypto::CryptoError> {
                let pk = PublicKey::from_bytes(HYBRID, &concat_pk)?;
                let sig = Signature::from_bytes(HYBRID, &concat_sig)?;
                pk.verify(&msg, &sig, mode)
            })();

            assert_eq!(
                parts_verdict.is_ok(),
                case.expected_valid,
                "parts verdict mismatch for case {} in mode {:?}",
                case.id,
                mode
            );
            assert_eq!(
                concat_verdict.is_ok(),
                case.expected_valid,
                "concat verdict mismatch for case {} in mode {:?}",
                case.id,
                mode
            );
        }

        if case.expected_valid {
            valid_count += 1;
        } else {
            invalid_count += 1;
        }
    }

    assert_eq!(valid_count, 5);
    assert_eq!(invalid_count, 29);
}

#[test]
fn rust_live_hedged_hybrid_signatures_verify() {
    let sk = SigningKey::hybrid_from_seeds(&[0x55u8; 32], &[0x66u8; 32]);
    let pk = sk.public_key();
    let msg = b"creg-pbft-v1:prepare:live-hedged";

    let s1 = sk.sign(msg);
    let s2 = sk.sign(msg);
    assert_ne!(s1.mldsa44_bytes().unwrap(), s2.mldsa44_bytes().unwrap());
    for mode in [VerifyMode::Strict, VerifyMode::Permissive] {
        pk.verify(msg, &s1, mode).unwrap();
        pk.verify(msg, &s2, mode).unwrap();
    }
}
