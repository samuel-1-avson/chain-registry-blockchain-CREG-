//! Canonical primitives for the CREG witnessed append-only transparency log.
//!
//! These hashes deliberately use a different domain from block Merkle roots.
//! A valid block proof therefore cannot be presented as a transparency-log
//! proof.  The tree follows the RFC 6962 split rule and uses domain-separated
//! SHA-256 leaf (`0x00`) and node (`0x01`) hashes.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const TRANSPARENCY_LOG_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransparencyEvent {
    pub version: u8,
    /// Monotonic sequence assigned by the log store, starting at zero.
    pub sequence: u64,
    /// Stable event category (for example `package-finalized` or `revocation`).
    pub kind: String,
    /// Canonical package/namespace subject, never a display label.
    pub subject: String,
    /// SHA-256 digest of the versioned event payload held in durable storage.
    pub payload_hash: String,
    /// Source-system identifier. Imported legacy chain records must say so.
    pub source: String,
    /// RFC 3339 timestamp supplied by the authoritative event producer.
    pub occurred_at: String,
}

impl TransparencyEvent {
    /// A length-delimited encoding avoids JSON map-order ambiguity in the
    /// cryptographic commitment while retaining JSON for transport/storage.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"creg-transparency-event-v1\0");
        out.push(self.version);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        for value in [
            self.kind.as_bytes(),
            self.subject.as_bytes(),
            self.payload_hash.as_bytes(),
            self.source.as_bytes(),
            self.occurred_at.as_bytes(),
        ] {
            out.extend_from_slice(&(value.len() as u64).to_be_bytes());
            out.extend_from_slice(value);
        }
        out
    }

    pub fn leaf_hash(&self) -> String {
        leaf_hash(&self.canonical_bytes())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InclusionProof {
    pub log_id: String,
    pub tree_size: u64,
    pub leaf_index: u64,
    pub leaf_hash: String,
    pub audit_path: Vec<String>,
    pub root_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsistencyProof {
    pub log_id: String,
    pub first_tree_size: u64,
    pub second_tree_size: u64,
    pub first_root_hash: String,
    pub second_root_hash: String,
    pub audit_path: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeHead {
    pub version: u8,
    pub log_id: String,
    pub tree_size: u64,
    pub root_hash: String,
    pub issued_at: String,
}

impl TreeHead {
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"creg-tree-head-v1\0");
        out.push(self.version);
        out.extend_from_slice(&self.tree_size.to_be_bytes());
        for value in [
            self.log_id.as_bytes(),
            self.root_hash.as_bytes(),
            self.issued_at.as_bytes(),
        ] {
            out.extend_from_slice(&(value.len() as u64).to_be_bytes());
            out.extend_from_slice(value);
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedTreeHead {
    pub head: TreeHead,
    pub log_signer_id: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WitnessSignature {
    pub witness_id: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WitnessedTreeHead {
    pub signed_head: SignedTreeHead,
    pub witness_signatures: Vec<WitnessSignature>,
}

/// Trusted witness identities are explicit configuration, not data supplied by
/// a log operator. `minimum_witnesses` must be non-zero for install eligibility.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WitnessPolicy {
    pub minimum_witnesses: usize,
    pub trusted_witness_keys: BTreeMap<String, String>,
}

/// Client-side acceptance policy for a witnessed tree head. A caller must
/// persist the previously accepted head and supply its consistency proof on
/// every update; accepting a standalone newer head permits rollback/freeze
/// attacks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransparencyVerificationPolicy {
    pub maximum_head_age_seconds: i64,
    pub maximum_future_skew_seconds: i64,
    pub witness_policy: WitnessPolicy,
}

pub fn leaf_hash(bytes: &[u8]) -> String {
    let mut input = Vec::with_capacity(bytes.len() + 1);
    input.push(0);
    input.extend_from_slice(bytes);
    crate::sha256_hex(&input)
}

fn node_hash(left: &str, right: &str) -> Result<String> {
    let left = hex::decode(left).context("invalid left transparency hash")?;
    let right = hex::decode(right).context("invalid right transparency hash")?;
    if left.len() != 32 || right.len() != 32 {
        bail!("transparency tree hashes must be 32 bytes");
    }
    let mut input = Vec::with_capacity(65);
    input.push(1);
    input.extend_from_slice(&left);
    input.extend_from_slice(&right);
    Ok(crate::sha256_hex(&input))
}

pub fn empty_root() -> String {
    crate::sha256_hex(b"creg-transparency-empty-v1")
}

fn largest_power_of_two_less_than(n: usize) -> usize {
    debug_assert!(n > 1);
    1usize << ((usize::BITS - (n - 1).leading_zeros() - 1) as usize)
}

pub fn tree_root(leaves: &[String]) -> Result<String> {
    match leaves.len() {
        0 => Ok(empty_root()),
        1 => {
            let leaf = hex::decode(&leaves[0]).context("invalid transparency leaf hash")?;
            if leaf.len() != 32 {
                bail!("transparency leaf hash must be 32 bytes");
            }
            Ok(leaves[0].clone())
        }
        n => {
            let split = largest_power_of_two_less_than(n);
            node_hash(&tree_root(&leaves[..split])?, &tree_root(&leaves[split..])?)
        }
    }
}

pub fn inclusion_proof(
    log_id: impl Into<String>,
    leaves: &[String],
    index: usize,
) -> Result<InclusionProof> {
    if index >= leaves.len() {
        bail!("leaf index is outside the transparency tree");
    }
    fn build(leaves: &[String], index: usize, path: &mut Vec<String>) -> Result<()> {
        if leaves.len() <= 1 {
            return Ok(());
        }
        let split = largest_power_of_two_less_than(leaves.len());
        if index < split {
            build(&leaves[..split], index, path)?;
            path.push(tree_root(&leaves[split..])?);
        } else {
            build(&leaves[split..], index - split, path)?;
            path.push(tree_root(&leaves[..split])?);
        }
        Ok(())
    }

    let mut audit_path = Vec::new();
    build(leaves, index, &mut audit_path)?;
    Ok(InclusionProof {
        log_id: log_id.into(),
        tree_size: leaves.len() as u64,
        leaf_index: index as u64,
        leaf_hash: leaves[index].clone(),
        audit_path,
        root_hash: tree_root(leaves)?,
    })
}

impl InclusionProof {
    pub fn verify(&self) -> Result<bool> {
        if self.tree_size == 0 || self.leaf_index >= self.tree_size {
            return Ok(false);
        }
        let mut index = self.leaf_index;
        let mut last = self.tree_size - 1;
        let mut hash = self.leaf_hash.clone();
        for sibling in &self.audit_path {
            hash = if index & 1 == 1 || index == last {
                let result = node_hash(sibling, &hash)?;
                while index & 1 == 0 && index != 0 {
                    index >>= 1;
                    last >>= 1;
                }
                result
            } else {
                node_hash(&hash, sibling)?
            };
            index >>= 1;
            last >>= 1;
        }
        Ok(index == 0 && last == 0 && hash == self.root_hash)
    }
}

pub fn consistency_proof(
    log_id: impl Into<String>,
    leaves: &[String],
    first_tree_size: usize,
) -> Result<ConsistencyProof> {
    if first_tree_size == 0 || first_tree_size > leaves.len() {
        bail!("invalid consistency proof tree sizes");
    }
    fn build(
        leaves: &[String],
        first: usize,
        include_first_root: bool,
        path: &mut Vec<String>,
    ) -> Result<()> {
        if first == leaves.len() {
            if !include_first_root {
                path.push(tree_root(leaves)?);
            }
            return Ok(());
        }
        let split = largest_power_of_two_less_than(leaves.len());
        if first <= split {
            build(&leaves[..split], first, include_first_root, path)?;
            path.push(tree_root(&leaves[split..])?);
        } else {
            build(&leaves[split..], first - split, false, path)?;
            path.push(tree_root(&leaves[..split])?);
        }
        Ok(())
    }

    let mut audit_path = Vec::new();
    if first_tree_size != leaves.len() {
        // The verifier initializes both reconstructed roots from the first
        // proof element, so include the old-tree root even when its size is a
        // power of two.
        build(leaves, first_tree_size, false, &mut audit_path)?;
    }
    Ok(ConsistencyProof {
        log_id: log_id.into(),
        first_tree_size: first_tree_size as u64,
        second_tree_size: leaves.len() as u64,
        first_root_hash: tree_root(&leaves[..first_tree_size])?,
        second_root_hash: tree_root(leaves)?,
        audit_path,
    })
}

impl ConsistencyProof {
    pub fn verify(&self) -> Result<bool> {
        if self.first_tree_size == 0 || self.first_tree_size > self.second_tree_size {
            return Ok(false);
        }
        if self.first_tree_size == self.second_tree_size {
            return Ok(self.audit_path.is_empty() && self.first_root_hash == self.second_root_hash);
        }
        let mut first = self.first_tree_size - 1;
        let mut second = self.second_tree_size - 1;
        while first & 1 == 1 {
            first >>= 1;
            second >>= 1;
        }
        let Some(initial) = self.audit_path.first() else {
            return Ok(false);
        };
        let mut first_hash = initial.clone();
        let mut second_hash = initial.clone();
        for proof_hash in &self.audit_path[1..] {
            if second == 0 {
                return Ok(false);
            }
            if first & 1 == 1 || first == second {
                first_hash = node_hash(proof_hash, &first_hash)?;
                second_hash = node_hash(proof_hash, &second_hash)?;
                while first & 1 == 0 && first != 0 {
                    first >>= 1;
                    second >>= 1;
                }
            } else {
                second_hash = node_hash(&second_hash, proof_hash)?;
            }
            first >>= 1;
            second >>= 1;
        }
        Ok(second == 0
            && first_hash == self.first_root_hash
            && second_hash == self.second_root_hash)
    }
}

pub fn sign_tree_head(
    head: TreeHead,
    signer_id: impl Into<String>,
    key: &SigningKey,
) -> SignedTreeHead {
    SignedTreeHead {
        signature: hex::encode(key.sign(&head.signing_bytes()).to_bytes()),
        head,
        log_signer_id: signer_id.into(),
    }
}

pub fn verify_signed_tree_head(signed: &SignedTreeHead, public_key_hex: &str) -> Result<()> {
    let key_bytes =
        hex::decode(public_key_hex.trim_start_matches("0x")).context("invalid log public key")?;
    let key: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("log public key must be 32 bytes"))?;
    let signature_bytes = hex::decode(&signed.signature).context("invalid tree-head signature")?;
    let signature =
        Signature::from_slice(&signature_bytes).context("invalid tree-head signature length")?;
    VerifyingKey::from_bytes(&key)
        .context("invalid log public key")?
        .verify(&signed.head.signing_bytes(), &signature)
        .context("tree-head signature verification failed")
}

fn witness_bytes(signed: &SignedTreeHead) -> Vec<u8> {
    let mut bytes = b"creg-tree-head-witness-v1\0".to_vec();
    bytes.extend_from_slice(&signed.head.signing_bytes());
    bytes.extend_from_slice(&(signed.signature.len() as u64).to_be_bytes());
    bytes.extend_from_slice(signed.signature.as_bytes());
    bytes
}

pub fn sign_witnessed_tree_head(
    signed: SignedTreeHead,
    witness_id: impl Into<String>,
    key: &SigningKey,
) -> WitnessedTreeHead {
    let signature = hex::encode(key.sign(&witness_bytes(&signed)).to_bytes());
    WitnessedTreeHead {
        signed_head: signed,
        witness_signatures: vec![WitnessSignature {
            witness_id: witness_id.into(),
            signature,
        }],
    }
}

impl WitnessedTreeHead {
    pub fn verify(&self, log_public_key: &str, policy: &WitnessPolicy) -> Result<()> {
        if policy.minimum_witnesses == 0 {
            bail!("witness policy must require at least one independent witness");
        }
        verify_signed_tree_head(&self.signed_head, log_public_key)?;
        let payload = witness_bytes(&self.signed_head);
        let mut seen = HashSet::new();
        let mut valid_witnesses = 0usize;
        for witness in &self.witness_signatures {
            if witness.witness_id == self.signed_head.log_signer_id
                || !seen.insert(witness.witness_id.clone())
            {
                continue;
            }
            let Some(key) = policy.trusted_witness_keys.get(&witness.witness_id) else {
                continue;
            };
            let key_bytes = match hex::decode(key.trim_start_matches("0x")) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let Ok(key) = <[u8; 32]>::try_from(key_bytes.as_slice()) else {
                continue;
            };
            let Ok(signature_bytes) = hex::decode(&witness.signature) else {
                continue;
            };
            let Ok(signature) = Signature::from_slice(&signature_bytes) else {
                continue;
            };
            if VerifyingKey::from_bytes(&key)
                .and_then(|key| key.verify(&payload, &signature))
                .is_ok()
            {
                valid_witnesses += 1;
                if valid_witnesses >= policy.minimum_witnesses {
                    return Ok(());
                }
            }
        }
        bail!("tree head lacks the required number of valid independent witness signatures")
    }

    /// Combine signatures from independent witness operators for the same signed head.
    pub fn merge_from(heads: &[WitnessedTreeHead]) -> Result<WitnessedTreeHead> {
        let Some(first) = heads.first() else {
            bail!("no witnessed tree heads to merge");
        };
        let mut merged = first.clone();
        for other in &heads[1..] {
            if other.signed_head != merged.signed_head {
                bail!("cannot merge witness signatures for distinct signed tree heads");
            }
            for signature in &other.witness_signatures {
                if !merged
                    .witness_signatures
                    .iter()
                    .any(|existing| existing.witness_id == signature.witness_id)
                {
                    merged.witness_signatures.push(signature.clone());
                }
            }
        }
        Ok(merged)
    }
}

/// Verify the complete client trust statement: a particular immutable event
/// is included in a fresh, independently witnessed tree head, and that head
/// extends the caller's persisted checkpoint without rewriting history.
pub fn verify_witnessed_inclusion(
    event: &TransparencyEvent,
    inclusion: &InclusionProof,
    witnessed_head: &WitnessedTreeHead,
    log_public_key: &str,
    policy: &TransparencyVerificationPolicy,
    previous_head: Option<&TreeHead>,
    consistency: Option<&ConsistencyProof>,
    now: DateTime<Utc>,
) -> Result<()> {
    if policy.maximum_head_age_seconds < 0 || policy.maximum_future_skew_seconds < 0 {
        bail!("transparency freshness bounds must be non-negative");
    }
    witnessed_head.verify(log_public_key, &policy.witness_policy)?;
    let head = &witnessed_head.signed_head.head;
    let issued_at = DateTime::parse_from_rfc3339(&head.issued_at)
        .context("invalid tree-head issued_at")?
        .with_timezone(&Utc);
    let age = now.signed_duration_since(issued_at).num_seconds();
    if age > policy.maximum_head_age_seconds {
        bail!("witnessed tree head is stale");
    }
    if age < -policy.maximum_future_skew_seconds {
        bail!("witnessed tree head exceeds permitted future clock skew");
    }
    if event.leaf_hash() != inclusion.leaf_hash
        || inclusion.log_id != head.log_id
        || inclusion.tree_size != head.tree_size
        || inclusion.root_hash != head.root_hash
        || !inclusion.verify()?
    {
        bail!("event is not validly included in the witnessed tree head");
    }
    if let Some(previous) = previous_head {
        let proof = consistency
            .ok_or_else(|| anyhow::anyhow!("missing consistency proof from persisted tree head"))?;
        if proof.log_id != head.log_id
            || proof.first_tree_size != previous.tree_size
            || proof.first_root_hash != previous.root_hash
            || proof.second_tree_size != head.tree_size
            || proof.second_root_hash != head.root_hash
            || !proof.verify()?
        {
            bail!("new tree head is not consistent with the persisted tree head");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(count: usize) -> Vec<String> {
        (0..count)
            .map(|sequence| {
                TransparencyEvent {
                    version: TRANSPARENCY_LOG_VERSION,
                    sequence: sequence as u64,
                    kind: "package-finalized".into(),
                    subject: format!("npm:pkg-{sequence}@1.0.0"),
                    payload_hash: crate::sha256_hex(format!("payload-{sequence}").as_bytes()),
                    source: "test".into(),
                    occurred_at: "2026-07-14T00:00:00Z".into(),
                }
                .leaf_hash()
            })
            .collect()
    }

    #[test]
    fn inclusion_and_consistency_proofs_verify_for_all_small_tree_sizes() {
        for size in 1..=16 {
            let tree = leaves(size);
            for index in 0..size {
                assert!(inclusion_proof("test-log", &tree, index)
                    .unwrap()
                    .verify()
                    .unwrap());
            }
            for old_size in 1..=size {
                assert!(consistency_proof("test-log", &tree, old_size)
                    .unwrap()
                    .verify()
                    .unwrap());
            }
        }
    }

    #[test]
    fn altered_proof_is_rejected() {
        let tree = leaves(5);
        let mut proof = inclusion_proof("test-log", &tree, 3).unwrap();
        proof.root_hash = "00".repeat(32);
        assert!(!proof.verify().unwrap());
        let mut consistency = consistency_proof("test-log", &tree, 3).unwrap();
        consistency.second_root_hash = "00".repeat(32);
        assert!(!consistency.verify().unwrap());
    }

    #[test]
    fn witnessed_head_requires_distinct_trusted_witnesses() {
        let log_key = SigningKey::from_bytes(&[1; 32]);
        let witness_key = SigningKey::from_bytes(&[2; 32]);
        let head = TreeHead {
            version: TRANSPARENCY_LOG_VERSION,
            log_id: "test-log".into(),
            tree_size: 1,
            root_hash: leaves(1)[0].clone(),
            issued_at: "2026-07-14T00:00:00Z".into(),
        };
        let signed = sign_tree_head(head, "log-operator", &log_key);
        let witnessed = sign_witnessed_tree_head(signed, "witness-a", &witness_key);
        let mut policy = WitnessPolicy {
            minimum_witnesses: 1,
            trusted_witness_keys: BTreeMap::new(),
        };
        policy.trusted_witness_keys.insert(
            "witness-a".into(),
            hex::encode(witness_key.verifying_key().as_bytes()),
        );
        witnessed
            .verify(&hex::encode(log_key.verifying_key().as_bytes()), &policy)
            .unwrap();
        policy.minimum_witnesses = 2;
        assert!(witnessed
            .verify(&hex::encode(log_key.verifying_key().as_bytes()), &policy)
            .is_err());
    }

    #[test]
    fn client_verification_rejects_stale_and_accepts_consistent_head() {
        let log_key = SigningKey::from_bytes(&[3; 32]);
        let witness_key = SigningKey::from_bytes(&[4; 32]);
        let events: Vec<TransparencyEvent> = (0..2)
            .map(|sequence| TransparencyEvent {
                version: TRANSPARENCY_LOG_VERSION,
                sequence,
                kind: "package-finalized".into(),
                subject: format!("npm:pkg-{sequence}@1.0.0"),
                payload_hash: crate::sha256_hex(format!("payload-{sequence}").as_bytes()),
                source: "test".into(),
                occurred_at: "2026-07-14T00:00:00Z".into(),
            })
            .collect();
        let leaves: Vec<String> = events.iter().map(TransparencyEvent::leaf_hash).collect();
        let previous = TreeHead {
            version: TRANSPARENCY_LOG_VERSION,
            log_id: "test-log".into(),
            tree_size: 1,
            root_hash: tree_root(&leaves[..1]).unwrap(),
            issued_at: "2026-07-14T00:00:00Z".into(),
        };
        let current = TreeHead {
            version: TRANSPARENCY_LOG_VERSION,
            log_id: "test-log".into(),
            tree_size: 2,
            root_hash: tree_root(&leaves).unwrap(),
            issued_at: "2026-07-14T00:00:01Z".into(),
        };
        let witnessed = sign_witnessed_tree_head(
            sign_tree_head(current, "log-operator", &log_key),
            "witness-a",
            &witness_key,
        );
        let mut keys = BTreeMap::new();
        keys.insert(
            "witness-a".into(),
            hex::encode(witness_key.verifying_key().as_bytes()),
        );
        let policy = TransparencyVerificationPolicy {
            maximum_head_age_seconds: 60,
            maximum_future_skew_seconds: 5,
            witness_policy: WitnessPolicy {
                minimum_witnesses: 1,
                trusted_witness_keys: keys,
            },
        };
        verify_witnessed_inclusion(
            &events[1],
            &inclusion_proof("test-log", &leaves, 1).unwrap(),
            &witnessed,
            &hex::encode(log_key.verifying_key().as_bytes()),
            &policy,
            Some(&previous),
            Some(&consistency_proof("test-log", &leaves, 1).unwrap()),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();
        assert!(verify_witnessed_inclusion(
            &events[1],
            &inclusion_proof("test-log", &leaves, 1).unwrap(),
            &witnessed,
            &hex::encode(log_key.verifying_key().as_bytes()),
            &policy,
            Some(&previous),
            Some(&consistency_proof("test-log", &leaves, 1).unwrap()),
            DateTime::parse_from_rfc3339("2026-07-14T00:02:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )
        .is_err());
    }

    #[test]
    fn merge_from_combines_independent_witness_signatures() {
        let log_key = SigningKey::from_bytes(&[5; 32]);
        let witness_a = SigningKey::from_bytes(&[6; 32]);
        let witness_b = SigningKey::from_bytes(&[7; 32]);
        let head = TreeHead {
            version: TRANSPARENCY_LOG_VERSION,
            log_id: "test-log".into(),
            tree_size: 1,
            root_hash: leaves(1)[0].clone(),
            issued_at: "2026-08-24T00:00:00Z".into(),
        };
        let signed = sign_tree_head(head, "log-operator", &log_key);
        let a = sign_witnessed_tree_head(signed.clone(), "org-a", &witness_a);
        let b = sign_witnessed_tree_head(signed, "org-b", &witness_b);
        let merged = WitnessedTreeHead::merge_from(&[a, b]).unwrap();
        assert_eq!(merged.witness_signatures.len(), 2);
        let mut keys = BTreeMap::new();
        keys.insert(
            "org-a".into(),
            hex::encode(witness_a.verifying_key().as_bytes()),
        );
        keys.insert(
            "org-b".into(),
            hex::encode(witness_b.verifying_key().as_bytes()),
        );
        merged
            .verify(
                &hex::encode(log_key.verifying_key().as_bytes()),
                &WitnessPolicy {
                    minimum_witnesses: 2,
                    trusted_witness_keys: keys,
                },
            )
            .unwrap();
    }

    #[test]
    fn merge_from_rejects_distinct_signed_heads() {
        let log_key = SigningKey::from_bytes(&[8; 32]);
        let witness = SigningKey::from_bytes(&[9; 32]);
        let first = TreeHead {
            version: TRANSPARENCY_LOG_VERSION,
            log_id: "test-log".into(),
            tree_size: 1,
            root_hash: leaves(1)[0].clone(),
            issued_at: "2026-08-24T00:00:00Z".into(),
        };
        let mut second = first.clone();
        second.issued_at = "2026-08-24T00:00:01Z".into();
        let a = sign_witnessed_tree_head(
            sign_tree_head(first, "log-operator", &log_key),
            "org-a",
            &witness,
        );
        let b = sign_witnessed_tree_head(
            sign_tree_head(second, "log-operator", &log_key),
            "org-b",
            &witness,
        );
        assert!(WitnessedTreeHead::merge_from(&[a, b])
            .unwrap_err()
            .to_string()
            .contains("distinct signed tree heads"));
    }
}
