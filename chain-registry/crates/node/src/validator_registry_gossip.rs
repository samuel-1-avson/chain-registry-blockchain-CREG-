// crates/node/src/validator_registry_gossip.rs
//
// Fleet-wide propagation + persistence of validator identity registrations.
//
// Problem this solves: registrations used to be applied only on the single
// node that received the `/v1/validators/register` POST. Operators had to
// re-POST the same proof to every fleet node and the observer pool, and a
// missed node showed up as `active_validators: 1` even though L1 said the
// validator was Active (the documented drift incident).
//
// Fix: a successful registration is (a) persisted to disk so it survives
// restarts, and (b) gossiped to peers carrying its original ownership proofs.
// Every receiver re-verifies the proofs before applying, so propagation is
// trustless. A periodic re-broadcast lets late-joining / restarted nodes
// converge without operator action.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::p2p::P2PCommand;

/// Gossip topic for validator identity registrations.
pub const REGISTRATION_TOPIC: &str = "creg/v1/validators";
pub const VALIDATOR_SET_TRANSITIONS_TOPIC: &str = "creg/v1/validator-set-transitions";

const REGISTRATIONS_FILE: &str = "validator-registrations.json";

/// A validator identity registration plus the ownership proofs needed for any
/// node to re-verify it. Serializable for both the on-disk journal and the
/// gossip wire format.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistrationProof {
    pub evm_address: String,
    pub node_id: String,
    pub ed25519_pubkey: String,
    pub alias: Option<String>,
    pub nonce: String,
    pub evm_signature: String,
    pub ed25519_signature: String,
    /// ADR-017 ML-DSA-44 public key; see [`RegistrationProof::pq_binding_signature`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pq_pubkey: Option<String>,
    /// Compact `creg-pq-binding-v1` hybrid signature proving possession of
    /// `pq_pubkey`. Sent together with it (both or neither).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pq_binding_signature: Option<String>,
}

impl RegistrationProof {
    pub fn into_gossip(self) -> common::GossipMessage {
        common::GossipMessage::ValidatorRegistration {
            evm_address: self.evm_address,
            node_id: self.node_id,
            ed25519_pubkey: self.ed25519_pubkey,
            alias: self.alias,
            nonce: self.nonce,
            evm_signature: self.evm_signature,
            ed25519_signature: self.ed25519_signature,
            pq_pubkey: self.pq_pubkey,
            pq_binding_signature: self.pq_binding_signature,
        }
    }

    fn same_identity(&self, other: &Self) -> bool {
        self.evm_address
            .trim()
            .eq_ignore_ascii_case(other.evm_address.trim())
            && self.node_id.trim() == other.node_id.trim()
            && self
                .ed25519_pubkey
                .trim()
                .trim_start_matches("0x")
                .eq_ignore_ascii_case(other.ed25519_pubkey.trim().trim_start_matches("0x"))
    }
}

// ── Gossip sender (set once at startup) ──────────────────────────────────────

static GOSSIP_SENDER: OnceLock<mpsc::Sender<P2PCommand>> = OnceLock::new();

/// Install the p2p command sender so API/worker code can broadcast
/// registrations. Called once during node startup. Idempotent.
pub fn set_gossip_sender(sender: mpsc::Sender<P2PCommand>) {
    let _ = GOSSIP_SENDER.set(sender);
}

/// Broadcast a registration proof to peers. Best-effort: a missing sender
/// (e.g. p2p disabled in tests) or full channel is logged, not fatal.
pub async fn broadcast(proof: RegistrationProof) {
    let Some(sender) = GOSSIP_SENDER.get() else {
        return;
    };
    let msg = proof.into_gossip();
    let data = match serde_json::to_vec(&msg) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("Failed to serialize validator registration gossip: {}", e);
            return;
        }
    };
    if let Err(e) = sender
        .send(P2PCommand::Broadcast {
            topic: REGISTRATION_TOPIC.to_string(),
            data,
        })
        .await
    {
        tracing::debug!("Failed to enqueue validator registration broadcast: {}", e);
    }
}

pub async fn broadcast_validator_set_transition(
    snapshot: common::AuthenticatedValidatorSetSnapshot,
) {
    let Some(sender) = GOSSIP_SENDER.get() else {
        return;
    };
    let data = match serde_json::to_vec(&common::GossipMessage::ValidatorSetTransition { snapshot })
    {
        Ok(data) => data,
        Err(error) => {
            tracing::warn!("Failed to serialize validator-set transition gossip: {error}");
            return;
        }
    };
    if let Err(error) = sender
        .send(P2PCommand::Broadcast {
            topic: VALIDATOR_SET_TRANSITIONS_TOPIC.into(),
            data,
        })
        .await
    {
        tracing::debug!("Failed to enqueue validator-set transition broadcast: {error}");
    }
}

// ── On-disk journal ──────────────────────────────────────────────────────────

fn registrations_path(data_dir: &Path) -> PathBuf {
    data_dir.join(REGISTRATIONS_FILE)
}

/// Load all persisted registration proofs (empty if none/unreadable).
pub fn load_all(data_dir: &Path) -> Vec<RegistrationProof> {
    match std::fs::read(registrations_path(data_dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Persist a registration proof, replacing any prior entry for the same EVM
/// address (case-insensitive). Atomic write (temp file + rename).
///
/// A proof without an ML-DSA-44 binding keeps the binding already journaled
/// for the same (wallet, node ID, Ed25519 key) identity, so replaying an
/// older classical proof cannot strip a validator's post-quantum key
/// (ADR-017). Changing the identity drops the binding, which covers it.
pub fn persist(data_dir: &Path, proof: &RegistrationProof) -> anyhow::Result<()> {
    let mut all = load_all(data_dir);
    let key = proof.evm_address.trim().to_ascii_lowercase();
    let mut proof = proof.clone();
    if proof.pq_pubkey.is_none() && proof.pq_binding_signature.is_none() {
        if let Some(previous) = all.iter().find(|p| p.same_identity(&proof)) {
            proof.pq_pubkey = previous.pq_pubkey.clone();
            proof.pq_binding_signature = previous.pq_binding_signature.clone();
        }
    }
    all.retain(|p| p.evm_address.trim().to_ascii_lowercase() != key);
    all.push(proof);

    let bytes = serde_json::to_vec_pretty(&all)?;
    std::fs::create_dir_all(data_dir).ok();
    let path = registrations_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proof(addr: &str, node: &str) -> RegistrationProof {
        RegistrationProof {
            evm_address: addr.into(),
            node_id: node.into(),
            ed25519_pubkey: "ab".repeat(32),
            alias: Some(node.into()),
            nonce: "1".into(),
            evm_signature: "0xsig".into(),
            ed25519_signature: "0xsig".into(),
            pq_pubkey: None,
            pq_binding_signature: None,
        }
    }

    fn with_pq(mut proof: RegistrationProof, pq: &str) -> RegistrationProof {
        proof.pq_pubkey = Some(pq.repeat(2624 / pq.len()));
        proof.pq_binding_signature = Some(pq.repeat(4968 / pq.len()));
        proof
    }

    #[test]
    fn classical_replay_keeps_the_journaled_pq_binding() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bound = with_pq(proof("0xAbC", "node-1"), "ab");
        persist(dir.path(), &bound).expect("persist bound");

        // An older classical proof for the same identity keeps the binding.
        persist(dir.path(), &proof("0xabc", "node-1")).expect("persist replay");
        let all = load_all(dir.path());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].pq_pubkey, bound.pq_pubkey);
        assert_eq!(all[0].pq_binding_signature, bound.pq_binding_signature);

        // A new binding replaces the old one.
        let rebound = with_pq(proof("0xabc", "node-1"), "cd");
        persist(dir.path(), &rebound).expect("persist rebound");
        assert_eq!(load_all(dir.path())[0].pq_pubkey, rebound.pq_pubkey);

        // A different identity for the wallet drops the binding.
        persist(dir.path(), &proof("0xabc", "node-1-renamed")).expect("persist renamed");
        let all = load_all(dir.path());
        assert_eq!(all[0].node_id, "node-1-renamed");
        assert_eq!(all[0].pq_pubkey, None);
        assert_eq!(all[0].pq_binding_signature, None);
    }

    #[test]
    fn classical_proofs_serialize_without_pq_fields() {
        let value = serde_json::to_value(proof("0xabc", "node-1").into_gossip()).unwrap();
        let fields = &value["ValidatorRegistration"];
        assert!(fields.get("pq_pubkey").is_none());
        assert!(fields.get("pq_binding_signature").is_none());

        // Journals written before ADR-017 Increment 3 still load.
        let legacy = serde_json::json!([{
            "evm_address": "0xabc", "node_id": "node-1", "ed25519_pubkey": "ab",
            "alias": null, "nonce": "1", "evm_signature": "0x", "ed25519_signature": "0x"
        }]);
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(REGISTRATIONS_FILE),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        assert_eq!(load_all(dir.path()).len(), 1);
    }

    #[test]
    fn persist_dedupes_by_evm_address_case_insensitive() {
        let dir = tempfile::tempdir().expect("tempdir");
        persist(dir.path(), &proof("0xAbC", "node-1")).expect("persist 1");
        // Same address, different case + updated node id → replaces.
        persist(dir.path(), &proof("0xabc", "node-1-renamed")).expect("persist 2");
        persist(dir.path(), &proof("0xDEF", "node-2")).expect("persist 3");

        let all = load_all(dir.path());
        assert_eq!(
            all.len(),
            2,
            "duplicate EVM address must not create two rows"
        );
        let renamed = all
            .iter()
            .find(|p| p.evm_address.eq_ignore_ascii_case("0xabc"))
            .expect("addr present");
        assert_eq!(renamed.node_id, "node-1-renamed");
    }

    #[test]
    fn load_all_empty_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(load_all(dir.path()).is_empty());
    }
}
