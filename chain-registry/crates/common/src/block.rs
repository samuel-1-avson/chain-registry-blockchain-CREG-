// crates/common/src/block.rs

use crate::{
    sha256_hex, ChainRecord, NamespaceLifecycleEvent, NamespaceLifecycleEventV2,
    PublisherAuthorizationV1, SignedRevocationRequest,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single block in the package registry blockchain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
    /// Ed25519 signatures from PBFT Commit phase over the block hash.
    #[serde(default)]
    pub pbft_signatures: Vec<BlockSignature>,
}

impl Block {
    /// Compute the block hash over the serialized header.
    pub fn hash(&self) -> String {
        // The view-change certificate authorizes the view and leader but is
        // proof material rather than block identity. Keep it out of the block
        // digest so the prepare certificate it carries can refer to prior
        // block hashes without recursively hashing certificates.
        let mut header = self.header.clone();
        header.view_change_certificate.clear();
        let header_bytes = serde_json::to_vec(&header).expect("BlockHeader must be serializable");
        sha256_hex(&header_bytes)
    }

    /// Genesis block — the first block in the chain with no parent.
    pub fn genesis() -> Self {
        Self {
            header: BlockHeader {
                height: 0,
                prev_hash: "0".repeat(64),
                merkle_root: "0".repeat(64),
                proposer_id: "genesis".into(),
                timestamp: DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
                    .expect("genesis timestamp must be valid")
                    .with_timezone(&Utc),
                validator_set_hash: "0".repeat(64),
                vrf_output: None,
                vrf_proof: None,
                view_number: 0,
                view_change_certificate: vec![],
            },
            transactions: vec![],
            pbft_signatures: vec![],
        }
    }
}

impl Block {
    /// Bind Phase C's consensus parent to its verified signed spec. Old
    /// view-change votes and signing locks cannot address this new height one.
    pub fn genesis_for_phase_c(spec_genesis_hash: &str) -> anyhow::Result<Self> {
        let identity = spec_genesis_hash.trim_start_matches("0x");
        if identity.len() != 64 || !identity.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            anyhow::bail!("Phase C genesis requires a verified SHA-256 spec identity");
        }
        let mut genesis = Self::genesis();
        genesis.header.proposer_id = "genesis-phase-c-sorted-json-v1".into();
        genesis.header.validator_set_hash = identity.to_ascii_lowercase();
        Ok(genesis)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockHeader {
    pub height: u64,
    /// Hash of the previous block — links the chain.
    pub prev_hash: String,
    /// Merkle root of all transaction hashes in this block.
    pub merkle_root: String,
    /// ID of the validator node that proposed this block.
    pub proposer_id: String,
    pub timestamp: DateTime<Utc>,
    /// Hash of the current active validator set (detects set changes).
    pub validator_set_hash: String,
    /// VRF output (hex) used for proposer selection.
    #[serde(default)]
    pub vrf_output: Option<String>,
    /// VRF proof (hex signature) proving the proposer's legitimacy.
    #[serde(default)]
    pub vrf_proof: Option<String>,
    /// PBFT view in which this proposal was authorized. Omitted from legacy
    /// view-zero serialization to preserve existing block hashes.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub view_number: u32,
    /// Quorum of validator view-change votes authorizing this view. This is
    /// carried with the header for historical verification but excluded from
    /// `Block::hash()`; each vote signs the height, parent, view, and any
    /// prepared certificate it reports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub view_change_certificate: Vec<ViewChangeVote>,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

/// Header commitments used by a prepared certificate. It deliberately omits
/// the view-change certificate, which is proof material and not part of the
/// block hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedBlockHeader {
    pub height: u64,
    pub prev_hash: String,
    pub merkle_root: String,
    pub proposer_id: String,
    pub timestamp: DateTime<Utc>,
    pub validator_set_hash: String,
    #[serde(default)]
    pub vrf_output: Option<String>,
    #[serde(default)]
    pub vrf_proof: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub view_number: u32,
}

impl From<&BlockHeader> for PreparedBlockHeader {
    fn from(header: &BlockHeader) -> Self {
        Self {
            height: header.height,
            prev_hash: header.prev_hash.clone(),
            merkle_root: header.merkle_root.clone(),
            proposer_id: header.proposer_id.clone(),
            timestamp: header.timestamp,
            validator_set_hash: header.validator_set_hash.clone(),
            vrf_output: header.vrf_output.clone(),
            vrf_proof: header.vrf_proof.clone(),
            view_number: header.view_number,
        }
    }
}

/// Proof that a value reached PBFT PREPARE quorum in an earlier view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedCertificate {
    pub header: PreparedBlockHeader,
    /// PBFT view in which this immutable block reached PREPARE quorum.
    #[serde(default)]
    pub prepared_view: u32,
    /// Authorization proof carried by the original proposal, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub original_view_change_certificate: Vec<ViewChangeVote>,
    /// Signed proof that the immutable block was carried into a later view.
    /// Required when `prepared_view` exceeds the original header view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reproposal_proof: Option<Box<ReproposalProof>>,
    pub transactions: Vec<Transaction>,
    pub prepare_signatures: Vec<BlockSignature>,
}

/// Replacement-primary authorization for retransmitting an immutable block.
/// The proof is outside the block hash but bound to it by `leader_signature`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReproposalProof {
    pub target_view: u32,
    pub leader_id: String,
    pub view_change_certificate: Vec<ViewChangeVote>,
    pub leader_signature: String,
}

impl PreparedCertificate {
    /// Reconstruct the exact value that was prepared. The view-change proof is
    /// deliberately outside the block hash and is supplied by a later leader.
    pub fn block(&self) -> Block {
        Block {
            header: BlockHeader {
                height: self.header.height,
                prev_hash: self.header.prev_hash.clone(),
                merkle_root: self.header.merkle_root.clone(),
                proposer_id: self.header.proposer_id.clone(),
                timestamp: self.header.timestamp,
                validator_set_hash: self.header.validator_set_hash.clone(),
                vrf_output: self.header.vrf_output.clone(),
                vrf_proof: self.header.vrf_proof.clone(),
                view_number: self.header.view_number,
                view_change_certificate: self.original_view_change_certificate.clone(),
            },
            transactions: self.transactions.clone(),
            pbft_signatures: Vec::new(),
        }
    }
}

/// One validator's signed request to move a height/parent pair to a new view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewChangeVote {
    pub height: u64,
    pub prev_hash: String,
    pub new_view: u32,
    pub validator_id: String,
    pub prepared: Option<PreparedCertificate>,
    pub signature: String,
}

impl ViewChangeVote {
    /// Prepared signatures are ordered by validator ID. Phase C also sorts
    /// nested map keys throughout the prepared value and its proof ancestry.
    pub fn signing_message(&self) -> String {
        let mut prepared = self.prepared.clone();
        if let Some(certificate) = prepared.as_mut() {
            certificate
                .prepare_signatures
                .sort_by(|a, b| a.validator_id.cmp(&b.validator_id));
        }
        let bytes = crate::consensus_json_bytes(&(
            "creg-pbft-view-change-v1",
            self.height,
            &self.prev_hash,
            self.new_view,
            &self.validator_id,
            prepared,
        ))
        .expect("view-change signing payload must be serializable");
        sha256_hex(&bytes)
    }
}

/// A signature from a validator explicitly voting for a block in PBFT.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockSignature {
    pub validator_id: String,
    /// Hex-encoded Ed25519 public key.
    pub pubkey: String,
    /// Hex-encoded Ed25519 signature over the block hash.
    pub signature: String,
    /// Optional hex-encoded FIPS 204 ML-DSA-44 signature component (`ADR-017`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pq_signature: Option<String>,
}

/// Every action recorded on the chain is a Transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transaction {
    /// A new package accepted by consensus.
    Publish(ChainRecord),
    /// A package accepted with the complete publisher authorization retained
    /// for deterministic peer validation and chain rebuilds. This variant is
    /// mandatory for new publications after the namespace-authority activation
    /// height; `Publish` remains decodable for historical blocks.
    PublishAuthorizedV1 {
        record: ChainRecord,
        authorization: PublisherAuthorizationV1,
    },
    /// A previously accepted package has been revoked.
    Revoke {
        package_canonical: String,
        reason: String,
        revoked_by: String,
        evidence_hash: String,
    },
    /// Cryptographically authenticated package revocation carrying the full
    /// signed revocation proof for deterministic consensus replay (ADR-018 / VNA-30).
    RevokeV2 {
        request: SignedRevocationRequest,
        evidence_hash: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        witness: Option<crate::vote_v4::TriggerWitnessV1>,
    },
    /// A validator's stake was slashed for bad behaviour.
    Slash {
        validator_id: String,
        amount: u64,
        reason: String,
    },
    /// A new validator joined the active set.
    ValidatorJoin {
        validator_id: String,
        pubkey: String,
        stake: u64,
    },
    /// A validator left the active set.
    ValidatorLeave { validator_id: String },
    /// A publisher rotated their Ed25519 signing key.
    RotatePublisherKey {
        canonical_prefix: String,
        old_pubkey: String,
        new_pubkey: String,
        sig_from_old: String,
        sig_from_new: String,
        timestamp: DateTime<Utc>,
        /// Monotonic nonce — must be strictly greater than the last
        /// rotation nonce for this publisher.  Prevents replay attacks.
        #[serde(default)]
        nonce: u64,
    },
    /// A signed delegation, transfer, or recovery request for a package
    /// namespace. Nodes independently verify both controller signatures before
    /// applying the derived namespace-controller index.
    NamespaceLifecycle(NamespaceLifecycleEvent),
    /// Chain-bound lifecycle transition used after authoritative namespace
    /// activation. Recovery carries its independently signed upstream-authority
    /// certificate so peers and rebuilding nodes never need a live registry
    /// lookup during consensus replay.
    NamespaceLifecycleV2(NamespaceLifecycleEventV2),
}

impl Transaction {
    /// Package record carried by either the historical or proof-retaining
    /// publication transaction.
    pub fn published_record(&self) -> Option<&ChainRecord> {
        match self {
            Self::Publish(record) | Self::PublishAuthorizedV1 { record, .. } => Some(record),
            _ => None,
        }
    }

    /// Namespace lifecycle event carried by the historical transaction.
    pub fn namespace_lifecycle_v1(&self) -> Option<&NamespaceLifecycleEvent> {
        match self {
            Self::NamespaceLifecycle(event) => Some(event),
            _ => None,
        }
    }

    /// Chain-bound namespace lifecycle event carried by the authoritative
    /// transaction variant.
    pub fn namespace_lifecycle_v2(&self) -> Option<&NamespaceLifecycleEventV2> {
        match self {
            Self::NamespaceLifecycleV2(event) => Some(event),
            _ => None,
        }
    }

    /// Package revocation fields shared across historical `Revoke` and
    /// cryptographically authenticated `RevokeV2` transactions:
    /// `(package_canonical, reason, revoked_by, evidence_hash)`.
    pub fn revocation_target(&self) -> Option<(&str, &str, &str, &str)> {
        match self {
            Self::Revoke {
                package_canonical,
                reason,
                revoked_by,
                evidence_hash,
            } => Some((package_canonical, reason, revoked_by, evidence_hash)),
            Self::RevokeV2 {
                request,
                evidence_hash,
                ..
            } => Some((
                &request.package_canonical,
                &request.reason,
                &request.revoker_pubkey,
                evidence_hash,
            )),
            _ => None,
        }
    }
}

/// SHA-256 leaf hash used by both durable work and [`merkle_root`]. Phase C
/// canonicalizes nested map keys. Ordinary builds retain the legacy encoding.
pub fn transaction_hash(tx: &Transaction) -> String {
    sha256_hex(
        crate::consensus_json_bytes(tx)
            .expect("transaction must be serializable")
            .as_slice(),
    )
}

/// Hashes for every transaction in a block (same encoding as [`merkle_root`]).
pub fn block_transaction_hashes(block: &Block) -> Vec<String> {
    block.transactions.iter().map(transaction_hash).collect()
}

/// Computes the Merkle root of a list of transaction hashes.
/// Returns a fixed "empty" hash for an empty list.
pub fn merkle_root(txs: &[Transaction]) -> String {
    if txs.is_empty() {
        return sha256_hex(b"empty");
    }
    let mut hashes: Vec<String> = txs.iter().map(transaction_hash).collect();

    while hashes.len() > 1 {
        if hashes.len() % 2 != 0 {
            let last = hashes.last().expect("hashes is non-empty").clone();
            hashes.push(last);
        }
        hashes = hashes
            .chunks(2)
            .map(|pair| sha256_hex(format!("{}{}", pair[0], pair[1]).as_bytes()))
            .collect();
    }
    hashes.remove(0)
}

#[cfg(test)]
mod view_hash_tests {
    use super::*;

    #[derive(Serialize)]
    struct LegacyHeader {
        height: u64,
        prev_hash: String,
        merkle_root: String,
        proposer_id: String,
        timestamp: DateTime<Utc>,
        validator_set_hash: String,
        vrf_output: Option<String>,
        vrf_proof: Option<String>,
    }

    #[test]
    fn view_zero_preserves_legacy_hash_and_view_certificate_is_proof_only() {
        let block = Block::genesis();
        let legacy = LegacyHeader {
            height: block.header.height,
            prev_hash: block.header.prev_hash.clone(),
            merkle_root: block.header.merkle_root.clone(),
            proposer_id: block.header.proposer_id.clone(),
            timestamp: block.header.timestamp,
            validator_set_hash: block.header.validator_set_hash.clone(),
            vrf_output: block.header.vrf_output.clone(),
            vrf_proof: block.header.vrf_proof.clone(),
        };
        let legacy_hash = sha256_hex(&serde_json::to_vec(&legacy).unwrap());
        assert_eq!(block.hash(), legacy_hash);

        let mut with_certificate = block.clone();
        with_certificate
            .header
            .view_change_certificate
            .push(ViewChangeVote {
                height: 1,
                prev_hash: "11".repeat(32),
                new_view: 1,
                validator_id: "validator-1".into(),
                prepared: None,
                signature: "proof-bytes".into(),
            });
        assert_eq!(block.hash(), with_certificate.hash());

        with_certificate.header.view_number = 1;
        assert_ne!(block.hash(), with_certificate.hash());
    }
}
