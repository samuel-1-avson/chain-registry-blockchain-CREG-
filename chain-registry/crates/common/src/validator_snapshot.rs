//! Authenticated validator-set snapshots and transition certificates.
//!
//! A snapshot is usable as a consensus or committee-selection trust anchor only
//! when its transition is certified by a quorum of the preceding validator set.

use crate::{sha256_hex, ValidatorSet};
use serde::{Deserialize, Serialize};

pub const VALIDATOR_SNAPSHOT_SCHEMA_VERSION: u16 = 1;
/// Absolute protocol resource bound for authenticated validator-set envelopes.
/// Deployments may use a lower signed chain-spec limit; no receiver performs
/// unbounded identity or signature work above this ceiling.
pub const MAX_VALIDATOR_SET_MEMBERS: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidatorSetTransitionSignature {
    pub validator_id: String,
    pub validator_pubkey: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedValidatorSetSnapshot {
    pub schema_version: u16,
    /// First canonical block height governed by this set.
    pub effective_from_height: u64,
    /// Hash of the final block under the preceding set; empty only for an
    /// explicitly provisioned genesis trust anchor.
    pub previous_block_hash: String,
    pub previous_validator_set_hash: String,
    pub validators: ValidatorSet,
    pub validator_set_hash: String,
    #[serde(default)]
    pub transition_signatures: Vec<ValidatorSetTransitionSignature>,
}

pub fn canonical_validator_set_hash(set: &ValidatorSet) -> String {
    let mut validators = set.validators.clone();
    validators.sort_by(|left, right| {
        left.id.cmp(&right.id).then_with(|| {
            left.pubkey
                .to_ascii_lowercase()
                .cmp(&right.pubkey.to_ascii_lowercase())
        })
    });
    // Normalize node-local liveness labels before hashing. Chain-authoritative
    // sync marks the local member "self" and every other member "online", so
    // raw status strings always differ across nodes even for identical sets.
    // Only the active/inactive distinction is consensus-meaningful.
    // Eth addresses and pubkeys are case-insensitive identities: reconcile
    // lowercases them while history/spec snapshots keep original case.
    for validator in validators.iter_mut() {
        if crate::validator_status_is_active(&validator.status) {
            validator.status = "active".to_string();
        }
        validator.eth_address = validator.eth_address.to_ascii_lowercase();
        validator.pubkey = validator.pubkey.to_ascii_lowercase();
        if let Some(ref mut pq) = validator.pq_pubkey {
            *pq = pq.trim().to_ascii_lowercase();
        }
    }
    sha256_hex(&serde_json::to_vec(&validators).expect("validator set serializes"))
}

pub fn validator_set_transition_message(snapshot: &AuthenticatedValidatorSetSnapshot) -> String {
    format!(
        "creg-validator-set-transition-v1\nservice:creg-consensus\neffective_from_height:{}\nprevious_block_hash:{}\nprevious_validator_set_hash:{}\nvalidator_set_hash:{}",
        snapshot.effective_from_height,
        snapshot.previous_block_hash.trim().to_ascii_lowercase(),
        snapshot.previous_validator_set_hash.trim().to_ascii_lowercase(),
        snapshot.validator_set_hash.trim().to_ascii_lowercase(),
    )
}

/// Logical identifier for a certified validator-set transition.
///
/// The legacy function name is retained for wire/storage compatibility, but
/// the digest deliberately excludes the replaceable signature witness. Two
/// independently valid prior-set quorum subsets certify the same transition
/// and must converge on the same checkpoint, history, and committee identity.
/// The validator-set hashes commit to the complete preceding/next sets; the
/// schema/domain, effective height, and fork anchor are bound explicitly.
/// This identifier is never a substitute for independently calling
/// `validate_against` on the received witness.
pub fn validator_snapshot_certificate_hash(snapshot: &AuthenticatedValidatorSetSnapshot) -> String {
    sha256_hex(
        format!(
            "creg-certified-validator-set-transition-id-v1\nschema_version:{}\n{}",
            snapshot.schema_version,
            validator_set_transition_message(snapshot)
        )
        .as_bytes(),
    )
}

/// Exact audit digest of a particular signature witness. Consensus identity
/// must use `validator_snapshot_certificate_hash`; this digest is useful only
/// when operators need to distinguish the precise certificate bytes/signers.
pub fn validator_snapshot_witness_hash(snapshot: &AuthenticatedValidatorSetSnapshot) -> String {
    let mut canonical = snapshot.clone();
    canonical.transition_signatures.sort_by(|left, right| {
        left.validator_id
            .cmp(&right.validator_id)
            .then_with(|| left.validator_pubkey.cmp(&right.validator_pubkey))
            .then_with(|| left.signature.cmp(&right.signature))
    });
    sha256_hex(&serde_json::to_vec(&canonical).expect("validator snapshot serializes"))
}

fn is_lower_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && value == value.to_ascii_lowercase()
}

fn is_lower_hex_hash(value: &str) -> bool {
    is_lower_hex(value, 64)
}

/// Validate bootstrap membership without inventing a validator-set transition.
pub fn validate_validator_identities(set: &ValidatorSet) -> Result<(), &'static str> {
    use std::collections::HashSet;

    if set.validators.is_empty() {
        return Err("validator set is empty");
    }
    if set.validators.len() > MAX_VALIDATOR_SET_MEMBERS {
        return Err("validator set exceeds the protocol member limit");
    }
    let mut ids = HashSet::with_capacity(set.validators.len());
    let mut pubkeys = HashSet::with_capacity(set.validators.len());
    let mut pq_pubkeys = HashSet::new();
    for validator in &set.validators {
        if validator.id.trim().is_empty() {
            return Err("validator set contains an empty validator identity");
        }
        if validator.pubkey.trim().is_empty() {
            return Err("validator set contains an empty validator public key");
        }
        if validator.pubkey.len() != 64
            || !validator
                .pubkey
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("validator set contains a non-canonical Ed25519 public key");
        }
        if !ids.insert(validator.id.as_str()) {
            return Err("validator set contains duplicate validator identities");
        }
        if !pubkeys.insert(validator.pubkey.to_ascii_lowercase()) {
            return Err("validator set contains duplicate validator public keys");
        }
        if let Some(ref pq) = validator.pq_pubkey {
            if !is_lower_hex(pq, crate::vote_v4::MLDSA44_PUBLIC_KEY_HEX_LEN) {
                return Err("validator set contains a non-canonical ML-DSA-44 public key");
            }
            if !pq_pubkeys.insert(pq.as_str()) {
                return Err("validator set contains duplicate ML-DSA-44 public keys");
            }
        }
    }
    Ok(())
}

impl AuthenticatedValidatorSetSnapshot {
    /// Validate fields that are self-authenticating or structurally required
    /// without assuming access to the preceding validator set. This is the
    /// minimum gate for an out-of-band weak-subjectivity trust root.
    pub fn validate_structure(&self) -> Result<(), &'static str> {
        use std::collections::HashSet;

        if self.schema_version != VALIDATOR_SNAPSHOT_SCHEMA_VERSION {
            return Err("unsupported validator snapshot schema version");
        }
        if self.effective_from_height == 0 {
            return Err("genesis is a configured trust anchor, not a signed transition");
        }
        if !is_lower_hex_hash(&self.previous_block_hash) {
            return Err("validator snapshot lacks a canonical prior block hash");
        }
        if !is_lower_hex_hash(&self.previous_validator_set_hash) {
            return Err("validator snapshot lacks a canonical prior validator-set hash");
        }
        validate_validator_identities(&self.validators)?;
        if self.validator_set_hash != canonical_validator_set_hash(&self.validators) {
            return Err("validator snapshot hash does not match its validator set");
        }
        if self.transition_signatures.is_empty() {
            return Err("validator-set transition has no signature witness");
        }
        if self.transition_signatures.len() > MAX_VALIDATOR_SET_MEMBERS {
            return Err("validator-set transition witness exceeds the protocol member limit");
        }
        let mut signer_ids = HashSet::with_capacity(self.transition_signatures.len());
        for witness in &self.transition_signatures {
            if witness.validator_id.trim().is_empty() {
                return Err("validator-set transition witness has an empty signer identity");
            }
            if !signer_ids.insert(witness.validator_id.as_str()) {
                return Err("validator-set transition contains duplicate signer identities");
            }
            if !is_lower_hex(&witness.validator_pubkey, 64) {
                return Err(
                    "validator-set transition witness has a non-canonical Ed25519 public key",
                );
            }
            if !is_lower_hex(&witness.signature, 128) {
                return Err("validator-set transition witness has a malformed signature");
            }
        }
        Ok(())
    }

    pub fn validate_against(&self, previous: &ValidatorSet) -> Result<(), &'static str> {
        use crate::crypto::{PublicKey, Signature, SignatureScheme, VerifyMode};
        use std::collections::{HashMap, HashSet};

        self.validate_structure()?;
        validate_validator_identities(previous)?;
        if self.previous_validator_set_hash != canonical_validator_set_hash(previous) {
            return Err("validator snapshot hashes do not match their sets");
        }
        if self.transition_signatures.len() > previous.validators.len() {
            return Err("validator-set transition witness exceeds the prior-set size");
        }
        let message = validator_set_transition_message(self);
        let mut signers = HashSet::new();
        let validators_by_id: HashMap<&str, _> = previous
            .validators
            .iter()
            .map(|validator| (validator.id.as_str(), validator))
            .collect();
        for signature in &self.transition_signatures {
            if !signers.insert(signature.validator_id.clone()) {
                return Err("validator-set transition contains duplicate signer identities");
            }
            let Some(validator) = validators_by_id.get(signature.validator_id.as_str()) else {
                return Err("validator-set transition contains an unknown signer");
            };
            if validator.pubkey != signature.validator_pubkey {
                return Err("validator-set transition signer public key does not match");
            }
            let Ok(pubkey) = hex::decode(&validator.pubkey) else {
                return Err("validator-set transition signer public key is malformed");
            };
            let Ok(signature_bytes) = hex::decode(&signature.signature) else {
                return Err("validator-set transition signature is malformed");
            };
            let Ok(pubkey) = PublicKey::from_bytes(SignatureScheme::Ed25519, &pubkey) else {
                return Err("validator-set transition signer public key is malformed");
            };
            let Ok(signature) = Signature::from_bytes(SignatureScheme::Ed25519, &signature_bytes)
            else {
                return Err("validator-set transition signature is malformed");
            };
            if pubkey
                .verify(message.as_bytes(), &signature, VerifyMode::Strict)
                .is_err()
            {
                return Err("validator-set transition signature is invalid");
            }
        }
        let quorum = (2 * previous.validators.len()) / 3 + 1;
        if signers.len() < quorum {
            return Err("validator-set transition lacks prior-set quorum");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Validator;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    fn validator(id: &str, key: &SigningKey) -> Validator {
        Validator {
            id: id.into(),
            alias: id.into(),
            pubkey: hex::encode(key.verifying_key().as_bytes()),
            eth_address: format!("0x{id}"),
            stake: 100,
            reputation: 100,
            status: "online".into(),
            performance_score: 500,
            contribution_count: 0,
            pq_pubkey: None,
        }
    }

    #[test]
    fn canonical_hash_ignores_node_local_liveness_labels() {
        let key_a = SigningKey::generate(&mut OsRng);
        let key_b = SigningKey::generate(&mut OsRng);
        // Node A's view: itself "self", peer "online".
        let view_a = ValidatorSet::new(vec![
            Validator {
                status: "self".into(),
                ..validator("validator-1", &key_a)
            },
            Validator {
                status: "online".into(),
                ..validator("validator-2", &key_b)
            },
        ]);
        // Node B's view: mirrored labels.
        let view_b = ValidatorSet::new(vec![
            Validator {
                status: "online".into(),
                ..validator("validator-1", &key_a)
            },
            Validator {
                status: "self".into(),
                ..validator("validator-2", &key_b)
            },
        ]);
        // Spec bootstrap view: "active" everywhere.
        let view_spec = ValidatorSet::new(vec![
            Validator {
                status: "active".into(),
                ..validator("validator-1", &key_a)
            },
            Validator {
                status: "active".into(),
                ..validator("validator-2", &key_b)
            },
        ]);
        let hash_a = canonical_validator_set_hash(&view_a);
        assert_eq!(hash_a, canonical_validator_set_hash(&view_b));
        assert_eq!(hash_a, canonical_validator_set_hash(&view_spec));
        // A genuinely inactive member must still change the commitment.
        let view_jailed = ValidatorSet::new(vec![
            Validator {
                status: "active".into(),
                ..validator("validator-1", &key_a)
            },
            Validator {
                status: "jailed".into(),
                ..validator("validator-2", &key_b)
            },
        ]);
        assert_ne!(hash_a, canonical_validator_set_hash(&view_jailed));
        // History/spec snapshots keep original hex case while reconcile
        // lowercases: both spellings must hash identically.
        let mut mixed = validator("validator-2", &key_b);
        mixed.eth_address = "0xF4C0bdBB681A61Aa0B123E82C04b0D692F53d58E".into();
        mixed.pubkey = hex::encode(key_b.verifying_key().as_bytes()).to_ascii_uppercase();
        mixed.status = "active".into();
        let mut lowered = validator("validator-2", &key_b);
        lowered.eth_address = mixed.eth_address.to_ascii_lowercase();
        lowered.pubkey = lowered.pubkey.to_ascii_lowercase();
        lowered.status = "online".into();
        let view_mixed = ValidatorSet::new(vec![validator("validator-1", &key_a), mixed]);
        let view_lowered = ValidatorSet::new(vec![validator("validator-1", &key_a), lowered]);
        assert_eq!(
            canonical_validator_set_hash(&view_mixed),
            canonical_validator_set_hash(&view_lowered)
        );
    }

    #[test]
    fn transition_requires_a_distinct_prior_set_quorum_and_binds_all_fields() {
        let keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let previous = ValidatorSet::new(
            keys.iter()
                .enumerate()
                .map(|(i, key)| validator(&format!("v{i}"), key))
                .collect(),
        );
        let next = ValidatorSet::new(vec![validator("next", &SigningKey::generate(&mut OsRng))]);
        let mut snapshot = AuthenticatedValidatorSetSnapshot {
            schema_version: VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
            effective_from_height: 7,
            previous_block_hash: "a".repeat(64),
            previous_validator_set_hash: canonical_validator_set_hash(&previous),
            validator_set_hash: canonical_validator_set_hash(&next),
            validators: next,
            transition_signatures: vec![],
        };
        let message = validator_set_transition_message(&snapshot);
        for (index, key) in keys.iter().take(3).enumerate() {
            snapshot
                .transition_signatures
                .push(ValidatorSetTransitionSignature {
                    validator_id: format!("v{index}"),
                    validator_pubkey: hex::encode(key.verifying_key().as_bytes()),
                    signature: hex::encode(key.sign(message.as_bytes()).to_bytes()),
                });
        }
        assert!(snapshot.validate_against(&previous).is_ok());

        let mut alternate_witness = snapshot.clone();
        alternate_witness.transition_signatures.clear();
        for index in [1usize, 2, 3] {
            let key = &keys[index];
            alternate_witness
                .transition_signatures
                .push(ValidatorSetTransitionSignature {
                    validator_id: format!("v{index}"),
                    validator_pubkey: hex::encode(key.verifying_key().as_bytes()),
                    signature: hex::encode(key.sign(message.as_bytes()).to_bytes()),
                });
        }
        assert!(alternate_witness.validate_against(&previous).is_ok());
        assert_eq!(
            validator_snapshot_certificate_hash(&snapshot),
            validator_snapshot_certificate_hash(&alternate_witness),
            "honest quorum subsets must share one logical transition id"
        );
        assert_ne!(
            validator_snapshot_witness_hash(&snapshot),
            validator_snapshot_witness_hash(&alternate_witness),
            "the audit digest still distinguishes exact witnesses"
        );

        let mut quorum_plus_invalid = snapshot.clone();
        quorum_plus_invalid
            .transition_signatures
            .push(ValidatorSetTransitionSignature {
                validator_id: "v3".into(),
                validator_pubkey: hex::encode(keys[3].verifying_key().as_bytes()),
                signature: "00".repeat(64),
            });
        assert!(
            quorum_plus_invalid.validate_against(&previous).is_err(),
            "a valid quorum must not hide an invalid declared witness"
        );

        let mut duplicate_previous_id = previous.clone();
        duplicate_previous_id.validators[1].id = duplicate_previous_id.validators[0].id.clone();
        assert_eq!(
            snapshot.validate_against(&duplicate_previous_id),
            Err("validator set contains duplicate validator identities")
        );
        let mut duplicate_previous_pubkey = previous.clone();
        duplicate_previous_pubkey.validators[1].pubkey =
            duplicate_previous_pubkey.validators[0].pubkey.clone();
        assert_eq!(
            snapshot.validate_against(&duplicate_previous_pubkey),
            Err("validator set contains duplicate validator public keys")
        );
        let mut malformed_previous_non_signer = previous.clone();
        malformed_previous_non_signer.validators[3].pubkey = "not-ed25519".into();
        assert_eq!(
            snapshot.validate_against(&malformed_previous_non_signer),
            Err("validator set contains a non-canonical Ed25519 public key")
        );
        let mut duplicate_next_pubkey = snapshot.clone();
        let mut duplicate_member = duplicate_next_pubkey.validators.validators[0].clone();
        duplicate_member.id = "another-next-id".into();
        duplicate_next_pubkey
            .validators
            .validators
            .push(duplicate_member);
        assert_eq!(
            duplicate_next_pubkey.validate_against(&previous),
            Err("validator set contains duplicate validator public keys")
        );
        let mut malformed_next_pubkey = snapshot.clone();
        malformed_next_pubkey.validators.validators[0].pubkey = "ab".repeat(31);
        assert_eq!(
            malformed_next_pubkey.validate_against(&previous),
            Err("validator set contains a non-canonical Ed25519 public key")
        );
        let mut empty_next_id = snapshot.clone();
        empty_next_id.validators.validators[0].id.clear();
        assert_eq!(
            empty_next_id.validate_against(&previous),
            Err("validator set contains an empty validator identity")
        );
        let mut oversized_next = snapshot.clone();
        oversized_next.validators.validators =
            vec![snapshot.validators.validators[0].clone(); MAX_VALIDATOR_SET_MEMBERS + 1];
        assert_eq!(
            oversized_next.validate_against(&previous),
            Err("validator set exceeds the protocol member limit")
        );

        let mut unknown_snapshot_field = serde_json::to_value(&snapshot).expect("snapshot json");
        unknown_snapshot_field["unsigned_extension"] = serde_json::json!(true);
        assert!(serde_json::from_value::<AuthenticatedValidatorSetSnapshot>(
            unknown_snapshot_field
        )
        .is_err());
        let mut unknown_signature_field = serde_json::to_value(&snapshot).expect("snapshot json");
        unknown_signature_field["transition_signatures"][0]["unsigned_extension"] =
            serde_json::json!(true);
        assert!(serde_json::from_value::<AuthenticatedValidatorSetSnapshot>(
            unknown_signature_field
        )
        .is_err());

        snapshot.effective_from_height += 1;
        assert!(snapshot.validate_against(&previous).is_err());
    }
}
