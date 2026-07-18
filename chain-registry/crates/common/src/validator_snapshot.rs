//! Authenticated validator-set snapshots and transition certificates.
//!
//! A snapshot is usable as a consensus or committee-selection trust anchor only
//! when its transition is certified by a quorum of the preceding validator set.

use crate::{sha256_hex, ValidatorSet};
use serde::{Deserialize, Serialize};

pub const VALIDATOR_SNAPSHOT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorSetTransitionSignature {
    pub validator_id: String,
    pub validator_pubkey: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Stable digest used by downstream assignments to commit to the exact
/// certified transition, including its signers. This is an identifier, not a
/// substitute for `validate_against`.
pub fn validator_snapshot_certificate_hash(snapshot: &AuthenticatedValidatorSetSnapshot) -> String {
    let mut canonical = snapshot.clone();
    canonical.transition_signatures.sort_by(|left, right| {
        left.validator_id
            .cmp(&right.validator_id)
            .then_with(|| left.validator_pubkey.cmp(&right.validator_pubkey))
            .then_with(|| left.signature.cmp(&right.signature))
    });
    sha256_hex(&serde_json::to_vec(&canonical).expect("validator snapshot serializes"))
}

fn is_lower_hex_hash(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && value == value.to_ascii_lowercase()
}

impl AuthenticatedValidatorSetSnapshot {
    pub fn validate_against(&self, previous: &ValidatorSet) -> Result<(), &'static str> {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};
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
        if previous.validators.is_empty() {
            return Err("validator snapshot has no prior validator set");
        }
        if self.validators.validators.is_empty()
            || self.validator_set_hash != canonical_validator_set_hash(&self.validators)
            || self.previous_validator_set_hash != canonical_validator_set_hash(previous)
        {
            return Err("validator snapshot hashes do not match their sets");
        }
        let message = validator_set_transition_message(self);
        let mut signers = HashSet::new();
        let mut declared_signers = HashSet::new();
        for signature in &self.transition_signatures {
            if !declared_signers.insert(signature.validator_id.clone()) {
                return Err("validator-set transition contains duplicate signer identities");
            }
            let Some(validator) = previous.validators.iter().find(|validator| {
                validator.id == signature.validator_id
                    && validator
                        .pubkey
                        .eq_ignore_ascii_case(&signature.validator_pubkey)
            }) else {
                continue;
            };
            let Ok(pubkey) = hex::decode(&validator.pubkey) else {
                continue;
            };
            let Ok(signature_bytes) = hex::decode(&signature.signature) else {
                continue;
            };
            let Ok(pubkey) = VerifyingKey::try_from(pubkey.as_slice()) else {
                continue;
            };
            let Ok(signature) = Signature::try_from(signature_bytes.as_slice()) else {
                continue;
            };
            if pubkey.verify(message.as_bytes(), &signature).is_ok() {
                signers.insert(validator.id.clone());
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
        }
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
        snapshot.effective_from_height += 1;
        assert!(snapshot.validate_against(&previous).is_err());
    }
}
