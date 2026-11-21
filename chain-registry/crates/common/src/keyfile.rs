// crates/common/src/keyfile.rs
//! ADR-017 §4 v2 hybrid key file format and strict parser.
//!
//! Hybrid signing keys store two independent 32-byte seeds (`ed25519_seed` and
//! `mldsa44_seed`) in a compact single-line JSON envelope:
//! `{"version":2,"scheme":"hybrid-ed25519-mldsa44-v1","ed25519_seed":"<64 hex>","mldsa44_seed":"<64 hex>"}`
//!
//! Compact single-line output keeps plaintext files one line and stays compatible
//! with `CREG-ENC-V1` encryption. Secret material is wrapped in
//! [`zeroize::Zeroizing`] and cleared on drop.

use crate::crypto::{SignatureScheme, SigningKey, ED25519_SEED_LEN, MLDSA44_SEED_LEN};
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroize;

/// Supported version number for ADR-017 §4 hybrid key files.
pub const KEYFILE_V2_VERSION: u32 = 2;

/// Errors returned when constructing, serializing, or parsing a [`KeyFileV2`].
#[derive(Debug, thiserror::Error)]
pub enum KeyFileError {
    /// JSON syntax, missing/duplicate field, wrong JSON type, or unknown field.
    #[error("invalid key file JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Key file `version` did not equal [`KEYFILE_V2_VERSION`].
    #[error("unsupported key file version: expected {expected}, got {actual}")]
    UnsupportedVersion { expected: u32, actual: u32 },
    /// Scheme identifier is not `hybrid-ed25519-mldsa44-v1`.
    #[error("unsupported key file scheme: {0}")]
    UnsupportedScheme(String),
    /// A seed field failed strict 64-character lowercase ASCII hex validation.
    #[error("invalid seed in `{field}`: {reason}")]
    InvalidSeed {
        field: &'static str,
        reason: &'static str,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFileV2Raw {
    version: u32,
    scheme: String,
    ed25519_seed: String,
    mldsa44_seed: String,
}

impl Drop for KeyFileV2Raw {
    fn drop(&mut self) {
        self.ed25519_seed.zeroize();
        self.mldsa44_seed.zeroize();
    }
}

/// Parsed ADR-017 §4 v2 hybrid key file containing independent 32-byte seeds.
#[derive(Clone)]
pub struct KeyFileV2 {
    ed25519_seed: zeroize::Zeroizing<[u8; ED25519_SEED_LEN]>,
    mldsa44_seed: zeroize::Zeroizing<[u8; MLDSA44_SEED_LEN]>,
}

impl KeyFileV2 {
    /// Extract a v2 key file representation from a hybrid [`SigningKey`].
    ///
    /// Non-hybrid signing keys are rejected with [`KeyFileError::UnsupportedScheme`].
    pub fn from_signing_key(key: &SigningKey) -> Result<Self, KeyFileError> {
        if key.scheme() != SignatureScheme::HybridEd25519MlDsa44V1 {
            return Err(KeyFileError::UnsupportedScheme(
                key.scheme().id().to_string(),
            ));
        }
        let ed25519_seed = zeroize::Zeroizing::new(key.ed25519_seed());
        let mldsa44_seed = key
            .mldsa44_seed()
            .ok_or_else(|| KeyFileError::UnsupportedScheme(key.scheme().id().to_string()))?;
        Ok(Self {
            ed25519_seed,
            mldsa44_seed,
        })
    }

    /// Construct a [`KeyFileV2`] directly from independent 32-byte seeds.
    pub fn from_seeds(
        ed25519_seed: [u8; ED25519_SEED_LEN],
        mldsa44_seed: [u8; MLDSA44_SEED_LEN],
    ) -> Self {
        Self {
            ed25519_seed: zeroize::Zeroizing::new(ed25519_seed),
            mldsa44_seed: zeroize::Zeroizing::new(mldsa44_seed),
        }
    }

    /// Strictly parse an ADR-017 §4 v2 hybrid key file from JSON.
    pub fn parse_json(json: &str) -> Result<Self, KeyFileError> {
        let raw: KeyFileV2Raw = serde_json::from_str(json)?;
        if raw.version != KEYFILE_V2_VERSION {
            return Err(KeyFileError::UnsupportedVersion {
                expected: KEYFILE_V2_VERSION,
                actual: raw.version,
            });
        }
        if raw.scheme != SignatureScheme::HybridEd25519MlDsa44V1.id() {
            return Err(KeyFileError::UnsupportedScheme(raw.scheme.clone()));
        }
        let ed25519_seed = decode_lowercase_seed_hex("ed25519_seed", &raw.ed25519_seed)?;
        let mldsa44_seed = decode_lowercase_seed_hex("mldsa44_seed", &raw.mldsa44_seed)?;
        Ok(Self {
            ed25519_seed,
            mldsa44_seed,
        })
    }

    /// Serialize to compact single-line JSON wrapped in [`zeroize::Zeroizing`].
    pub fn to_json(&self) -> Result<zeroize::Zeroizing<String>, KeyFileError> {
        let raw = KeyFileV2Raw {
            version: KEYFILE_V2_VERSION,
            scheme: SignatureScheme::HybridEd25519MlDsa44V1.id().to_string(),
            ed25519_seed: hex::encode(&*self.ed25519_seed),
            mldsa44_seed: hex::encode(&*self.mldsa44_seed),
        };
        Ok(zeroize::Zeroizing::new(serde_json::to_string(&raw)?))
    }

    /// Reconstruct the hybrid [`SigningKey`] from the stored seeds.
    pub fn to_signing_key(&self) -> SigningKey {
        SigningKey::hybrid_from_seeds(&self.ed25519_seed, &self.mldsa44_seed)
    }

    /// The 32-byte Ed25519 secret seed.
    pub fn ed25519_seed(&self) -> &zeroize::Zeroizing<[u8; ED25519_SEED_LEN]> {
        &self.ed25519_seed
    }

    /// The 32-byte ML-DSA-44 secret seed (`ξ`).
    pub fn mldsa44_seed(&self) -> &zeroize::Zeroizing<[u8; MLDSA44_SEED_LEN]> {
        &self.mldsa44_seed
    }

    /// Key file format version (`2`).
    pub fn version(&self) -> u32 {
        KEYFILE_V2_VERSION
    }

    /// Signature scheme represented by this key file (`HybridEd25519MlDsa44V1`).
    pub fn scheme(&self) -> SignatureScheme {
        SignatureScheme::HybridEd25519MlDsa44V1
    }
}

impl fmt::Debug for KeyFileV2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyFileV2")
            .field("version", &self.version())
            .field("scheme", &self.scheme().id())
            .field("key_id", &self.to_signing_key().key_id())
            .finish_non_exhaustive()
    }
}

fn decode_lowercase_seed_hex(
    field: &'static str,
    hex_str: &str,
) -> Result<zeroize::Zeroizing<[u8; 32]>, KeyFileError> {
    if hex_str.starts_with("0x") || hex_str.starts_with("0X") {
        return Err(KeyFileError::InvalidSeed {
            field,
            reason: "must not include a 0x prefix",
        });
    }
    if hex_str.chars().any(char::is_whitespace) {
        return Err(KeyFileError::InvalidSeed {
            field,
            reason: "must not contain whitespace",
        });
    }
    if hex_str.len() != 64 {
        return Err(KeyFileError::InvalidSeed {
            field,
            reason: "must be exactly 64 hex characters",
        });
    }
    if hex_str.bytes().any(|b| matches!(b, b'A'..=b'F')) {
        return Err(KeyFileError::InvalidSeed {
            field,
            reason: "must be lowercase hexadecimal (0-9a-f)",
        });
    }
    if !hex_str
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(KeyFileError::InvalidSeed {
            field,
            reason: "must contain only lowercase hexadecimal characters (0-9a-f)",
        });
    }

    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    hex::decode_to_slice(hex_str, &mut *seed).map_err(|_| KeyFileError::InvalidSeed {
        field,
        reason: "invalid hexadecimal encoding",
    })?;
    Ok(seed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::VerifyMode;

    const VALID_ED_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const VALID_PQ_HEX: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

    fn valid_json_with(ed_hex: &str, pq_hex: &str) -> String {
        format!(
            r#"{{"version":2,"scheme":"hybrid-ed25519-mldsa44-v1","ed25519_seed":"{ed_hex}","mldsa44_seed":"{pq_hex}"}}"#
        )
    }

    #[test]
    fn round_trip_signing_key_json_and_sign_verify() {
        let sk = SigningKey::generate(
            SignatureScheme::HybridEd25519MlDsa44V1,
            &mut rand::rngs::OsRng,
        );
        let kf = KeyFileV2::from_signing_key(&sk).expect("hybrid key converts to KeyFileV2");
        assert_eq!(kf.version(), KEYFILE_V2_VERSION);
        assert_eq!(kf.scheme(), SignatureScheme::HybridEd25519MlDsa44V1);
        assert_eq!(**kf.ed25519_seed(), sk.ed25519_seed());
        assert_eq!(**kf.mldsa44_seed(), *sk.mldsa44_seed().unwrap());

        let json = kf.to_json().expect("serializes to JSON");
        let parsed = KeyFileV2::parse_json(&json).expect("parses serialized JSON");
        assert_eq!(*parsed.ed25519_seed(), *kf.ed25519_seed());
        assert_eq!(*parsed.mldsa44_seed(), *kf.mldsa44_seed());

        let round_tripped = parsed.to_signing_key();
        assert_eq!(
            round_tripped.scheme(),
            SignatureScheme::HybridEd25519MlDsa44V1
        );
        assert_eq!(round_tripped.public_key(), sk.public_key());
        assert_eq!(round_tripped.key_id(), sk.key_id());

        let msg = b"adr-017-keyfile-v2-roundtrip";
        let sig_from_roundtrip = round_tripped.sign(msg);
        sk.public_key()
            .verify(msg, &sig_from_roundtrip, VerifyMode::Strict)
            .expect("original public key verifies round-tripped signature");

        let sig_from_orig = sk.sign(msg);
        round_tripped
            .public_key()
            .verify(msg, &sig_from_orig, VerifyMode::Strict)
            .expect("round-tripped public key verifies original signature");
    }

    #[test]
    fn from_signing_key_rejects_ed25519_and_from_seeds_matches() {
        let ed_seed = [0x17u8; 32];
        let pq_seed = [0x28u8; 32];

        let ed_only = SigningKey::from_ed25519_seed(&ed_seed);
        match KeyFileV2::from_signing_key(&ed_only) {
            Err(KeyFileError::UnsupportedScheme(scheme)) => assert_eq!(scheme, "ed25519"),
            other => panic!("expected UnsupportedScheme(\"ed25519\"), got {other:?}"),
        }

        let from_seeds = KeyFileV2::from_seeds(ed_seed, pq_seed);
        let direct = SigningKey::hybrid_from_seeds(&ed_seed, &pq_seed);
        assert_eq!(
            from_seeds.to_signing_key().public_key(),
            direct.public_key()
        );
    }

    #[test]
    fn to_json_outputs_compact_single_line_with_exact_four_keys() {
        let ed_seed = hex::decode(VALID_ED_HEX).unwrap();
        let pq_seed = hex::decode(VALID_PQ_HEX).unwrap();
        let kf = KeyFileV2::from_seeds(ed_seed.try_into().unwrap(), pq_seed.try_into().unwrap());
        let json = kf.to_json().unwrap();

        assert!(!json.contains('\n'));
        assert!(!json.contains('\r'));
        assert!(!json.contains(' '));
        assert_eq!(&*json, &valid_json_with(VALID_ED_HEX, VALID_PQ_HEX));

        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = val.as_object().expect("JSON object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["ed25519_seed", "mldsa44_seed", "scheme", "version"]
        );
    }

    #[test]
    fn rejects_wrong_or_missing_version() {
        for bad_version in [0u32, 1, 3] {
            let json = format!(
                r#"{{"version":{bad_version},"scheme":"hybrid-ed25519-mldsa44-v1","ed25519_seed":"{VALID_ED_HEX}","mldsa44_seed":"{VALID_PQ_HEX}"}}"#
            );
            match KeyFileV2::parse_json(&json) {
                Err(KeyFileError::UnsupportedVersion { expected, actual }) => {
                    assert_eq!(expected, 2);
                    assert_eq!(actual, bad_version);
                }
                other => panic!("expected UnsupportedVersion({bad_version}), got {other:?}"),
            }
        }

        let string_version = format!(
            r#"{{"version":"2","scheme":"hybrid-ed25519-mldsa44-v1","ed25519_seed":"{VALID_ED_HEX}","mldsa44_seed":"{VALID_PQ_HEX}"}}"#
        );
        assert!(matches!(
            KeyFileV2::parse_json(&string_version),
            Err(KeyFileError::Json(_))
        ));

        let missing_version = format!(
            r#"{{"scheme":"hybrid-ed25519-mldsa44-v1","ed25519_seed":"{VALID_ED_HEX}","mldsa44_seed":"{VALID_PQ_HEX}"}}"#
        );
        assert!(matches!(
            KeyFileV2::parse_json(&missing_version),
            Err(KeyFileError::Json(_))
        ));
    }

    #[test]
    fn rejects_wrong_or_missing_scheme() {
        for bad_scheme in [
            "ed25519",
            "hybrid",
            "Hybrid-Ed25519-MlDsa44-V1",
            "HYBRID-ED25519-MLDSA44-V1",
            " hybrid-ed25519-mldsa44-v1",
            "",
        ] {
            let json = format!(
                r#"{{"version":2,"scheme":"{bad_scheme}","ed25519_seed":"{VALID_ED_HEX}","mldsa44_seed":"{VALID_PQ_HEX}"}}"#
            );
            match KeyFileV2::parse_json(&json) {
                Err(KeyFileError::UnsupportedScheme(actual)) => assert_eq!(actual, bad_scheme),
                other => panic!("expected UnsupportedScheme({bad_scheme:?}), got {other:?}"),
            }
        }

        let missing_scheme = format!(
            r#"{{"version":2,"ed25519_seed":"{VALID_ED_HEX}","mldsa44_seed":"{VALID_PQ_HEX}"}}"#
        );
        assert!(matches!(
            KeyFileV2::parse_json(&missing_scheme),
            Err(KeyFileError::Json(_))
        ));
    }

    #[test]
    fn rejects_invalid_seeds_for_both_fields() {
        let invalid_cases = [
            "",
            "0123456789abcdef",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef00",
            "0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeF",
            "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcd",
            "0X0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcd",
            " 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef ",
            "0123456789abcdef0123456789abcdef 123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdef\n123456789abcdef0123456789abcdef",
            "g123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde!",
        ];

        for bad in invalid_cases {
            let escaped = serde_json::to_string(bad).unwrap();
            let bad_ed_json = format!(
                r#"{{"version":2,"scheme":"hybrid-ed25519-mldsa44-v1","ed25519_seed":{escaped},"mldsa44_seed":"{VALID_PQ_HEX}"}}"#
            );
            match KeyFileV2::parse_json(&bad_ed_json) {
                Err(KeyFileError::InvalidSeed { field, .. }) => assert_eq!(field, "ed25519_seed"),
                other => panic!("expected InvalidSeed for ed25519_seed={bad:?}, got {other:?}"),
            }

            let bad_pq_json = format!(
                r#"{{"version":2,"scheme":"hybrid-ed25519-mldsa44-v1","ed25519_seed":"{VALID_ED_HEX}","mldsa44_seed":{escaped}}}"#
            );
            match KeyFileV2::parse_json(&bad_pq_json) {
                Err(KeyFileError::InvalidSeed { field, .. }) => assert_eq!(field, "mldsa44_seed"),
                other => panic!("expected InvalidSeed for mldsa44_seed={bad:?}, got {other:?}"),
            }
        }

        let missing_ed = format!(
            r#"{{"version":2,"scheme":"hybrid-ed25519-mldsa44-v1","mldsa44_seed":"{VALID_PQ_HEX}"}}"#
        );
        assert!(matches!(
            KeyFileV2::parse_json(&missing_ed),
            Err(KeyFileError::Json(_))
        ));

        let missing_pq = format!(
            r#"{{"version":2,"scheme":"hybrid-ed25519-mldsa44-v1","ed25519_seed":"{VALID_ED_HEX}"}}"#
        );
        assert!(matches!(
            KeyFileV2::parse_json(&missing_pq),
            Err(KeyFileError::Json(_))
        ));
    }

    #[test]
    fn rejects_unknown_extra_fields_and_malformed_json() {
        let extra_field = format!(
            r#"{{"version":2,"scheme":"hybrid-ed25519-mldsa44-v1","ed25519_seed":"{VALID_ED_HEX}","mldsa44_seed":"{VALID_PQ_HEX}","extra":"nope"}}"#
        );
        assert!(matches!(
            KeyFileV2::parse_json(&extra_field),
            Err(KeyFileError::Json(_))
        ));

        let duplicate_field = format!(
            r#"{{"version":2,"scheme":"hybrid-ed25519-mldsa44-v1","ed25519_seed":"{VALID_ED_HEX}","ed25519_seed":"{VALID_ED_HEX}","mldsa44_seed":"{VALID_PQ_HEX}"}}"#
        );
        assert!(matches!(
            KeyFileV2::parse_json(&duplicate_field),
            Err(KeyFileError::Json(_))
        ));

        for malformed in [
            "",
            "{",
            "[]",
            "null",
            &format!("{} trailing", valid_json_with(VALID_ED_HEX, VALID_PQ_HEX)),
        ] {
            assert!(
                matches!(KeyFileV2::parse_json(malformed), Err(KeyFileError::Json(_))),
                "expected Json error for {malformed:?}"
            );
        }
    }

    #[test]
    fn debug_redacts_seeds_and_shows_metadata() {
        let ed_seed = [0x3bu8; 32];
        let pq_seed = [0xc4u8; 32];
        let kf = KeyFileV2::from_seeds(ed_seed, pq_seed);
        let key_id = kf.to_signing_key().key_id();

        let debug_out = format!("{kf:?}");
        assert!(!debug_out.contains(&hex::encode(ed_seed)));
        assert!(!debug_out.contains(&hex::encode(pq_seed)));
        assert!(!debug_out.contains("3b3b3b3b"));
        assert!(!debug_out.contains("c4c4c4c4"));
        assert!(debug_out.contains("version: 2"));
        assert!(debug_out.contains("hybrid-ed25519-mldsa44-v1"));
        assert!(debug_out.contains(&key_id));
    }
}
