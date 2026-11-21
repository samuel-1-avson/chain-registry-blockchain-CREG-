//! NIST ACVP ML-DSA-44 (FIPS 204 final) conformance tests.
//!
//! Fixtures live in `tests/vectors/` and are trimmed subsets of the official
//! `usnistgov/ACVP-Server` vectors. See `tests/vectors/README.md` for pinned
//! commit URLs and upstream SHA-256 digests.

use common::crypto::SigningKey as CregSigningKey;
use ml_dsa::{
    EncodedVerifyingKey, ExpandedSigningKey, ExpandedSigningKeyBytes, Keypair, MlDsa44, Seed,
    Signature, SigningKey, VerifyingKey,
};
use serde::Deserialize;
use std::convert::Infallible;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyGenFixture {
    cases: Vec<KeyGenCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyGenCase {
    tc_id: u64,
    seed: String,
    pk: String,
    sk: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SigGenFixture {
    deterministic_pure: Vec<DetSigGenCase>,
    hedged_pure: Vec<HedgedSigGenCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetSigGenCase {
    tc_id: u64,
    sk: String,
    message: String,
    context: String,
    signature: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HedgedSigGenCase {
    tc_id: u64,
    sk: String,
    message: String,
    context: String,
    rnd: String,
    signature: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SigVerFixture {
    cases: Vec<SigVerCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SigVerCase {
    tc_id: u64,
    pk: String,
    message: String,
    context: String,
    signature: String,
    test_passed: bool,
}

fn unhex(s: &str) -> Vec<u8> {
    if s.is_empty() {
        Vec::new()
    } else {
        hex::decode(s).expect("valid ACVP hex")
    }
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

#[test]
fn acvp_mldsa44_keygen() {
    let fixture: KeyGenFixture =
        serde_json::from_str(include_str!("vectors/mldsa44_keygen_acvp.json"))
            .expect("parse keyGen ACVP fixture");
    assert!(!fixture.cases.is_empty());

    for case in fixture.cases {
        let seed_bytes = unhex(&case.seed);
        let expected_pk = unhex(&case.pk);
        let expected_sk = unhex(&case.sk);

        let seed = Seed::try_from(seed_bytes.as_slice()).expect("32-byte seed");
        let sk = SigningKey::<MlDsa44>::from_seed(&seed);
        assert_eq!(
            sk.verifying_key().encode().as_slice(),
            expected_pk.as_slice(),
            "keyGen pk mismatch for tcId {}",
            case.tc_id
        );

        #[allow(deprecated)]
        let expanded = sk.expanded_key().to_expanded();
        assert_eq!(
            expanded.as_slice(),
            expected_sk.as_slice(),
            "keyGen sk mismatch for tcId {}",
            case.tc_id
        );

        let seed_arr: [u8; 32] = seed_bytes.try_into().expect("32-byte seed array");
        let hybrid = CregSigningKey::hybrid_from_seeds(&[0x11; 32], &seed_arr);
        assert_eq!(
            hybrid.public_key().mldsa44_bytes().unwrap(),
            expected_pk,
            "hybrid wrapper ML-DSA-44 pk mismatch for tcId {}",
            case.tc_id
        );
    }
}

#[test]
fn acvp_mldsa44_siggen_deterministic_and_hedged_pure_with_context() {
    let fixture: SigGenFixture =
        serde_json::from_str(include_str!("vectors/mldsa44_siggen_acvp.json"))
            .expect("parse sigGen ACVP fixture");
    assert!(!fixture.deterministic_pure.is_empty());
    assert!(!fixture.hedged_pure.is_empty());

    for case in fixture.deterministic_pure {
        let sk_bytes = unhex(&case.sk);
        let message = unhex(&case.message);
        let context = unhex(&case.context);
        let expected_sig = unhex(&case.signature);

        let sk_enc = ExpandedSigningKeyBytes::<MlDsa44>::try_from(sk_bytes.as_slice())
            .expect("valid expanded sk length");
        #[allow(deprecated)]
        let sk = ExpandedSigningKey::<MlDsa44>::from_expanded(&sk_enc);
        let sig = sk
            .sign_deterministic(&message, &context)
            .expect("deterministic ACVP signing");
        assert_eq!(
            sig.encode().as_slice(),
            expected_sig.as_slice(),
            "deterministic sigGen mismatch for tcId {}",
            case.tc_id
        );
        assert!(
            sk.verifying_key()
                .verify_with_context(&message, &context, &sig),
            "generated deterministic signature failed verification for tcId {}",
            case.tc_id
        );
    }

    for case in fixture.hedged_pure {
        let sk_bytes = unhex(&case.sk);
        let message = unhex(&case.message);
        let context = unhex(&case.context);
        let rnd = unhex(&case.rnd);
        let expected_sig = unhex(&case.signature);

        let sk_enc = ExpandedSigningKeyBytes::<MlDsa44>::try_from(sk_bytes.as_slice())
            .expect("valid expanded sk length");
        #[allow(deprecated)]
        let sk = ExpandedSigningKey::<MlDsa44>::from_expanded(&sk_enc);
        let mut rng = FixedBytesRng(&rnd);
        let sig = sk
            .sign_randomized(&message, &context, &mut rng)
            .expect("hedged ACVP signing");
        assert_eq!(
            sig.encode().as_slice(),
            expected_sig.as_slice(),
            "hedged sigGen mismatch for tcId {}",
            case.tc_id
        );
        assert!(
            sk.verifying_key()
                .verify_with_context(&message, &context, &sig),
            "generated hedged signature failed verification for tcId {}",
            case.tc_id
        );
    }
}

#[test]
fn acvp_mldsa44_sigver_valid_and_invalid() {
    let fixture: SigVerFixture =
        serde_json::from_str(include_str!("vectors/mldsa44_sigver_acvp.json"))
            .expect("parse sigVer ACVP fixture");
    assert!(!fixture.cases.is_empty());

    let mut passed_count = 0usize;
    let mut failed_count = 0usize;

    for case in fixture.cases {
        let pk_bytes = unhex(&case.pk);
        let message = unhex(&case.message);
        let context = unhex(&case.context);
        let sig_bytes = unhex(&case.signature);

        let pk_enc = EncodedVerifyingKey::<MlDsa44>::try_from(pk_bytes.as_slice())
            .expect("valid ACVP pk length");
        let vk = VerifyingKey::<MlDsa44>::decode(&pk_enc);
        let verified = Signature::<MlDsa44>::try_from(sig_bytes.as_slice())
            .map(|sig| vk.verify_with_context(&message, &context, &sig))
            .unwrap_or(false);

        assert_eq!(
            verified, case.test_passed,
            "sigVer mismatch for tcId {}",
            case.tc_id
        );
        if case.test_passed {
            passed_count += 1;
        } else {
            failed_count += 1;
        }
    }

    assert!(passed_count > 0 && failed_count > 0);
}
