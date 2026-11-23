// crates/consensus/src/validator_set.rs
// Active validator set — tracks who can vote on a block.

use common::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorInfo {
    pub id: String,
    /// Ed25519 public key (hex-encoded) — used for VRF proofs and block signing.
    pub pubkey: String,
    /// Ethereum-style address (0x-prefixed, 20-byte hex) derived from the
    /// validator's secp256k1/ECDSA key.  Used for cross-referencing on-chain
    /// contract votes and slashing evidence.  `None` for legacy validators
    /// that have not yet registered an ECDSA key.
    pub eth_address: Option<String>,
    /// Staked tokens — slashed on bad behaviour.
    pub stake: u64,
    /// Reputation score 0–100. Starts at 50 for new validators.
    pub reputation: u8,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct ValidatorSet {
    pub(crate) validators: HashMap<String, ValidatorInfo>,
}

impl Default for ValidatorSet {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidatorSet {
    pub fn new() -> Self {
        Self {
            validators: HashMap::new(),
        }
    }

    pub fn add(&mut self, info: ValidatorInfo) {
        self.validators.insert(info.id.clone(), info);
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(v) = self.validators.get_mut(id) {
            v.is_active = false;
        }
    }

    pub fn is_member(&self, id: &str) -> bool {
        self.validators
            .get(id)
            .map(|v| v.is_active)
            .unwrap_or(false)
    }

    pub fn len(&self) -> usize {
        self.validators.values().filter(|v| v.is_active).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn active_ids(&self) -> Vec<String> {
        self.validators
            .values()
            .filter(|v| v.is_active)
            .map(|v| v.id.clone())
            .collect()
    }

    /// Slash a validator's stake by `amount`. If stake drops to zero,
    /// they are removed from the active set.
    pub fn slash(&mut self, id: &str, amount: u64, reason: &str) {
        if let Some(v) = self.validators.get_mut(id) {
            tracing::warn!("Slashing {} by {} stake. Reason: {}", id, amount, reason);
            v.stake = v.stake.saturating_sub(amount);
            v.reputation = v.reputation.saturating_sub(10);
            if v.stake == 0 {
                v.is_active = false;
                tracing::warn!("Validator {} removed from active set (stake depleted)", id);
            }
        }
    }

    /// Reward good behaviour with a small reputation boost.
    pub fn reward(&mut self, id: &str) {
        if let Some(v) = self.validators.get_mut(id) {
            v.reputation = v.reputation.saturating_add(1).min(100);
        }
    }

    /// Hash of the current active set — embedded in every block header
    /// so any change to validators is immediately visible on-chain.
    pub fn set_hash(&self) -> String {
        let mut ids = self.active_ids();
        ids.sort();
        sha256_hex(ids.join(",").as_bytes())
    }
}

impl From<common::ValidatorSet> for ValidatorSet {
    fn from(common_vs: common::ValidatorSet) -> Self {
        let mut vs = Self::new();
        for v in common_vs.validators {
            vs.add(ValidatorInfo {
                id: v.id.clone(),
                pubkey: v.pubkey.clone(),
                eth_address: Some(v.eth_address.clone()),
                stake: v.stake,
                reputation: v.reputation.try_into().unwrap_or(100),
                is_active: common::validator_status_is_active(&v.status),
            });
        }
        vs
    }
}

#[cfg(test)]
mod tests {
    use super::{ValidatorInfo, ValidatorSet};

    fn common_validator(id: &str, status: &str) -> common::Validator {
        common::Validator {
            id: id.into(),
            alias: id.into(),
            pubkey: "11".repeat(32),
            eth_address: format!("0x{:040}", 1),
            stake: 100,
            reputation: 100,
            status: status.into(),
            performance_score: 500,
            contribution_count: 0,
            pq_pubkey: None,
        }
    }

    #[test]
    fn spec_status_active_counts_toward_pbft_quorum() {
        let common_vs = common::ValidatorSet::new(vec![
            common_validator("core-1", "active"),
            common_validator("core-2", "active"),
            common_validator("core-3", "ACTIVE"),
            common_validator("core-4", "online"),
        ]);
        let vs = ValidatorSet::from(common_vs);
        assert_eq!(vs.len(), 4);
        assert!(vs.is_member("core-1"));
        assert!(vs.is_member("core-3"));
    }

    #[test]
    fn offline_and_pending_are_not_pbft_active() {
        let common_vs = common::ValidatorSet::new(vec![
            common_validator("core-1", "offline"),
            common_validator("core-2", "pending"),
            common_validator("core-3", "jailed"),
            common_validator("core-4", "self"),
        ]);
        let vs = ValidatorSet::from(common_vs);
        assert_eq!(vs.len(), 1);
        assert!(vs.is_member("core-4"));
        assert!(!vs.is_member("core-1"));
    }

    #[test]
    fn inactive_flag_excludes_member_even_if_present() {
        let mut vs = ValidatorSet::new();
        vs.add(ValidatorInfo {
            id: "core-1".into(),
            pubkey: "11".repeat(32),
            eth_address: None,
            stake: 100,
            reputation: 100,
            is_active: false,
        });
        assert_eq!(vs.len(), 0);
        assert!(!vs.is_member("core-1"));
    }
}
