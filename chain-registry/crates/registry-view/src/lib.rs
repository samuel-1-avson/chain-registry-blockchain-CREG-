//! Versioned public read contract for CREG v4 data (ADR-019 §3.3, `ECA-05`,
//! `ECA-06`, `ECO-INV-05`).
//!
//! This crate is pure data: the node serves these shapes, and the CLI,
//! explorer, hub and indexer consume them. Every attacker-influenced string
//! is a [`DisplayText`] (sanitised on construction *and* on decode), every
//! code is a closed enum, and statuses use one internally tagged lattice.
//! Display data is never evidence: security decisions verify
//! [`ProofBundleV2`] with the consensus verifiers.

use common::status_tree::Hash32;
use common::{
    BlockSignature, CommitteeQuorumCertificateV1, CregFindingCategoryV1, DisplayText,
    KernelViolationCode, StatusLeafV1, StatusProofV2, TriggerWitnessV1, ValidatorSignature,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PACKAGE_VIEW_SCHEMA: &str = "creg.package-view.v4";
pub const PROOF_BUNDLE_SCHEMA: &str = "creg.proof-bundle.v2";
pub const REVOCATION_STATUS_SCHEMA: &str = "creg.revocation-status.v2";
pub const WITNESS_BUNDLE_SCHEMA: &str = "creg.witness-bundle.v1";
pub const QUARANTINE_LIST_SCHEMA: &str = "creg.quarantine-list.v1";

/// Bound for free-text reasons shown next to a status.
pub const MAX_REASON_CHARS: usize = 256;
/// Bound for one trigger env key/value or argument.
pub const MAX_TRIGGER_FIELD_CHARS: usize = 128;

fn hex32(hash: &Hash32) -> String {
    common::status_tree::hex32_to_string(hash)
}

/// Which quorum rules produced a record (ADR-019 decision #7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum QuorumLabelV1 {
    /// Accepted before `hybrid_signature_activation_height` under the v3
    /// evidence quorum. Resolvers accept it below activation unless
    /// `--require-v4-quorum` is set.
    LegacyV3Quorum,
    /// Accepted with a `creg-vote-v4` committee certificate.
    V4Quorum,
}

impl QuorumLabelV1 {
    pub fn for_height(height: u64, activation_height: Option<u64>) -> Self {
        match activation_height {
            Some(activation) if height >= activation => Self::V4Quorum,
            _ => Self::LegacyV3Quorum,
        }
    }
}

/// Who authorised a revocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RevocationAuthorityV1 {
    Owner,
    Controller,
    CommitteeQuorum,
    /// Pre-activation single-validator `RevokeV2` (not certified).
    LegacySingleValidator,
    /// Deterministic 72h quarantine expiry.
    QuarantineExpiry,
}

impl RevocationAuthorityV1 {
    /// Certified revocations are sticky in client journals (`ECO-INV-04`).
    pub fn is_certified(self) -> bool {
        !matches!(self, Self::LegacySingleValidator)
    }
}

/// Committee composition and outcome, chain-derived.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CommitteeSummaryV1 {
    pub effective_size: u32,
    pub approvals: u32,
    pub quarantine_reviews: u32,
    pub rejections: u32,
    pub abstentions: u32,
    /// Normalised model family -> votes.
    pub model_families: BTreeMap<String, u32>,
    pub hybrid_signers: u32,
    pub classical_signers: u32,
}

impl CommitteeSummaryV1 {
    /// Summarise the recorded votes of a committee of `effective_size`.
    pub fn from_votes(effective_size: u32, votes: &[ValidatorSignature]) -> Self {
        let mut summary = Self {
            effective_size,
            ..Self::default()
        };
        for vote in votes {
            if vote.pq_signature.is_some() {
                summary.hybrid_signers += 1;
            } else {
                summary.classical_signers += 1;
            }
            let disposition = vote
                .semantic_vote
                .as_ref()
                .map(|semantic| semantic.disposition);
            match (&vote.vote, disposition) {
                (_, Some(common::SemanticDispositionV4::QuarantineReview)) => {
                    summary.quarantine_reviews += 1
                }
                (common::ValidatorVote::Reject { .. }, _)
                | (_, Some(common::SemanticDispositionV4::Reject)) => summary.rejections += 1,
                (common::ValidatorVote::Approve, _) => summary.approvals += 1,
            }
            if let Some(semantic) = &vote.semantic_vote {
                let family =
                    DisplayText::bounded(&semantic.model_family.trim().to_ascii_lowercase(), 64)
                        .into_string();
                *summary.model_families.entry(family).or_insert(0) += 1;
            }
        }
        let voted = summary.approvals + summary.quarantine_reviews + summary.rejections;
        summary.abstentions = effective_size.saturating_sub(voted);
        summary
    }
}

/// Current status lattice (`ECO-INV-02`): `Revoked` ⊐ `Quarantined` ⊐
/// {`Pending`, `Unknown`} ⊐ `Verified`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PackageStatusViewV1 {
    Pending,
    Verified {
        height: u64,
        block_hash: String,
    },
    Quarantined {
        height: u64,
        expires_at_unix: i64,
        #[cfg_attr(feature = "openapi", schema(value_type = String))]
        reason_code: CregFindingCategoryV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        committee: Option<CommitteeSummaryV1>,
    },
    Revoked {
        height: u64,
        authority: RevocationAuthorityV1,
        #[cfg_attr(feature = "openapi", schema(value_type = String))]
        reason: DisplayText,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        witness_digest: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        certificate_digest: Option<String>,
    },
    /// Status could not be established (unknown value, missing data).
    Unknown,
}

impl PackageStatusViewV1 {
    /// Position in the fail-closed lattice (higher dominates).
    pub fn severity(&self) -> u8 {
        match self {
            Self::Verified { .. } => 0,
            Self::Pending | Self::Unknown => 1,
            Self::Quarantined { .. } => 2,
            Self::Revoked { .. } => 3,
        }
    }

    /// A quarantine past its deadline is revoked (deterministic expiry).
    pub fn at(self, now_unix: i64) -> Self {
        match self {
            Self::Quarantined {
                height,
                expires_at_unix,
                reason_code,
                ..
            } if now_unix >= expires_at_unix => Self::Revoked {
                height,
                authority: RevocationAuthorityV1::QuarantineExpiry,
                reason: DisplayText::bounded(
                    &format!("quarantine expired without resolution ({reason_code:?})"),
                    MAX_REASON_CHARS,
                ),
                witness_digest: None,
                certificate_digest: None,
            },
            other => other,
        }
    }

    pub fn is_installable(&self) -> bool {
        matches!(self, Self::Verified { .. })
    }
}

/// Evidence class of one vote (ADR-018 Class A/B/C).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum EvidenceClass {
    /// Deterministic static/sandbox findings.
    A,
    /// Proof-carrying trigger witness.
    B,
    /// Quantized semantic (LLM-assisted) vote.
    C,
}

/// Pure classification of a recorded vote.
pub fn evidence_class(
    vote: &ValidatorSignature,
    witness: Option<&TriggerWitnessV1>,
) -> EvidenceClass {
    let witness_bound = match (witness, &vote.trigger_witness_digest) {
        (Some(witness), Some(digest)) => witness.canonical_digest().eq_ignore_ascii_case(digest),
        (None, Some(_)) => true,
        _ => false,
    };
    if witness_bound {
        EvidenceClass::B
    } else if vote.semantic_vote.is_some() {
        EvidenceClass::C
    } else {
        EvidenceClass::A
    }
}

/// One package version as shown by explorers and `creg info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackageViewV4 {
    pub schema: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub canonical: DisplayText,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub package: DisplayText,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub version: DisplayText,
    pub content_hash: String,
    pub status: PackageStatusViewV1,
    pub quorum: QuorumLabelV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committee: Option<CommitteeSummaryV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub witness_digests: Vec<String>,
}

impl PackageViewV4 {
    pub fn new(
        canonical: &str,
        content_hash: &str,
        status: PackageStatusViewV1,
        quorum: QuorumLabelV1,
    ) -> Self {
        let (package, version) =
            common::status_tree::split_canonical(canonical).unwrap_or((canonical, ""));
        Self {
            schema: PACKAGE_VIEW_SCHEMA.to_string(),
            canonical: DisplayText::bounded(canonical, 512),
            package: DisplayText::bounded(package, 512),
            version: DisplayText::bounded(version, 128),
            content_hash: content_hash.trim().to_ascii_lowercase(),
            status,
            quorum,
            committee: None,
            witness_digests: Vec::new(),
        }
    }
}

/// One step of a transaction Merkle path, leaf to root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct MerkleStepV1 {
    pub sibling_hash: String,
    /// True when the sibling is the right-hand child.
    pub is_right: bool,
}

/// Everything a client needs to verify a package's current status and
/// publication offline (consumed by the Increment 3 resolver).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProofBundleV2 {
    pub schema: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub canonical: DisplayText,
    pub tip_height: u64,
    pub tip_block_hash: String,
    pub tip_timestamp_unix: i64,
    /// PBFT commit signatures over the tip block hash.
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<Object>))]
    pub tip_commit_signatures: Vec<BlockSignature>,
    /// `status_root` of the tip header; absent before hybrid activation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_root: Option<String>,
    /// Present with `status_proof` exactly when `status_root` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub status_leaf: Option<StatusLeafV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub status_proof: Option<StatusProofV2>,
    /// Block, index, transaction hash and Merkle path of the publication.
    pub publication_height: u64,
    pub publication_block_hash: String,
    pub publication_tx_index: u32,
    pub publication_tx_hash: String,
    pub publication_merkle_path: Vec<MerkleStepV1>,
    /// PBFT commit signatures over the publication block hash.
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<Object>))]
    pub publication_commit_signatures: Vec<BlockSignature>,
    pub quorum: QuorumLabelV1,
}

impl ProofBundleV2 {
    pub fn status_root_from(root: Option<&Hash32>) -> Option<String> {
        root.map(hex32)
    }
}

/// Revocation detail with its authority and certificate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RevocationStatusV2 {
    pub schema: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub canonical: DisplayText,
    pub content_hash: String,
    pub revoked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<RevocationAuthorityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub reason: Option<DisplayText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub witness_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub certificate: Option<CommitteeQuorumCertificateV1>,
}

/// A violation as rendered: closed code and static title, or an opaque
/// digest of an unknown wire value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ViolationViewV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub code: Option<KernelViolationCode>,
    pub title: String,
    /// `UNKNOWN(<sha256-8>)` handle for values outside the dictionary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown: Option<String>,
}

impl ViolationViewV1 {
    pub fn from_wire(raw: &str) -> Self {
        match KernelViolationCode::parse(raw.trim()) {
            Some(code) => Self {
                code: Some(code),
                title: code.title().to_string(),
                unknown: None,
            },
            None => {
                let digest = common::sha256_hex(raw.as_bytes());
                Self {
                    code: None,
                    title: "Unknown violation code".to_string(),
                    unknown: Some(format!("UNKNOWN({})", &digest[..8])),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WitnessVerificationV1 {
    /// Committed on chain inside a certified decision at `height`.
    OnChainCertified { height: u64 },
    /// Committed on chain at `height` with an uncertified decision (for
    /// example an owner self-revocation that attached a witness).
    OnChainUncertified { height: u64 },
    /// Seen only in gossip; not certified.
    GossipOnly,
}

/// Inspector view of one `TriggerWitnessV1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WitnessViewV1 {
    pub digest: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub package_canonical: DisplayText,
    pub content_hash: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub prover_id: DisplayText,
    pub prover_key_id: String,
    pub hybrid: bool,
    /// `[key, value]` pairs.
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<Vec<String>>))]
    pub env_overrides: Vec<(DisplayText, DisplayText)>,
    pub clock_offset_secs: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub entry_function: Option<DisplayText>,
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<String>))]
    pub args: Vec<DisplayText>,
    pub violations: Vec<ViolationViewV1>,
    pub graph_canonical_digest: String,
    pub verification: WitnessVerificationV1,
}

impl WitnessViewV1 {
    pub fn from_witness(witness: &TriggerWitnessV1, verification: WitnessVerificationV1) -> Self {
        let field = |value: &str| DisplayText::bounded(value, MAX_TRIGGER_FIELD_CHARS);
        Self {
            digest: witness.canonical_digest(),
            package_canonical: DisplayText::bounded(&witness.package_canonical, 512),
            content_hash: witness.content_hash.trim().to_ascii_lowercase(),
            prover_id: DisplayText::bounded(&witness.witness_prover_id, 128),
            prover_key_id: witness.prover_key_id.trim().to_ascii_lowercase(),
            hybrid: witness.prover_pq_signature.is_some(),
            env_overrides: witness
                .trigger_spec
                .env_overrides
                .iter()
                .map(|(key, value)| (field(key), field(value)))
                .collect(),
            clock_offset_secs: witness.trigger_spec.clock_offset_secs,
            entry_function: witness.trigger_spec.entry_function.as_deref().map(field),
            args: witness
                .trigger_spec
                .args
                .iter()
                .map(|arg| field(arg))
                .collect(),
            violations: witness
                .observed_violations
                .iter()
                .map(|raw| ViolationViewV1::from_wire(raw))
                .collect(),
            graph_canonical_digest: witness.graph_canonical_digest.trim().to_ascii_lowercase(),
            verification,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WitnessBundleV1 {
    pub schema: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub canonical: DisplayText,
    pub witnesses: Vec<WitnessViewV1>,
}

/// Versions currently held in Class C quarantine, earliest deadline first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QuarantineListV1 {
    pub schema: String,
    pub tip_height: u64,
    pub packages: Vec<PackageViewV4>,
    /// More quarantined versions exist than `packages` lists.
    pub truncated: bool,
}

/// Typed SSE v2 event kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SseEventKindV2 {
    PackageVerified,
    PackageQuarantined,
    QuarantineResolved,
    QuarantineExpired,
    PackageRevokedV2,
    WitnessCommitted,
    BountyAwarded,
    BlockCommitted,
}

impl SseEventKindV2 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PackageVerified => "package_verified",
            Self::PackageQuarantined => "package_quarantined",
            Self::QuarantineResolved => "quarantine_resolved",
            Self::QuarantineExpired => "quarantine_expired",
            Self::PackageRevokedV2 => "package_revoked_v2",
            Self::WitnessCommitted => "witness_committed",
            Self::BountyAwarded => "bounty_awarded",
            Self::BlockCommitted => "block_committed",
        }
    }

    /// Kinds that must fail a `creg watch --ci` run.
    pub fn fails_ci(self) -> bool {
        matches!(
            self,
            Self::PackageQuarantined
                | Self::QuarantineExpired
                | Self::PackageRevokedV2
                | Self::WitnessCommitted
        )
    }
}

/// One SSE v2 event. `id` is `<height>.<tx_index>` for `Last-Event-ID` resume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SseEnvelopeV2 {
    pub id: String,
    pub kind: SseEventKindV2,
    pub height: u64,
    pub tx_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub canonical: Option<DisplayText>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub data: serde_json::Value,
}

impl SseEnvelopeV2 {
    pub fn new(
        kind: SseEventKindV2,
        height: u64,
        tx_index: u32,
        canonical: Option<&str>,
        data: serde_json::Value,
    ) -> Self {
        Self {
            id: format!("{height}.{tx_index}"),
            kind,
            height,
            tx_index,
            canonical: canonical.map(|value| DisplayText::bounded(value, 512)),
            data,
        }
    }

    /// Parse a `Last-Event-ID` (`<height>.<tx_index>`).
    pub fn parse_id(id: &str) -> Option<(u64, u32)> {
        let (height, index) = id.trim().split_once('.')?;
        Some((height.parse().ok()?, index.parse().ok()?))
    }

    /// Wire frame. The JSON payload is single-line, so no `data:` line can
    /// smuggle an extra SSE field.
    pub fn to_sse_frame(&self) -> String {
        let payload = serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string());
        format!(
            "id: {}\nevent: {}\ndata: {}\n\n",
            self.id,
            self.kind.as_str(),
            payload
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_lattice_orders_and_expires() {
        let verified = PackageStatusViewV1::Verified {
            height: 1,
            block_hash: "h".into(),
        };
        let quarantined = PackageStatusViewV1::Quarantined {
            height: 2,
            expires_at_unix: 1_000,
            reason_code: CregFindingCategoryV1::Backdoor,
            committee: None,
        };
        assert!(verified.is_installable());
        assert!(!quarantined.is_installable());
        assert!(quarantined.severity() > PackageStatusViewV1::Unknown.severity());
        assert!(PackageStatusViewV1::Unknown.severity() > verified.severity());

        assert_eq!(quarantined.clone().at(999), quarantined);
        match quarantined.at(1_000) {
            PackageStatusViewV1::Revoked { authority, .. } => {
                assert_eq!(authority, RevocationAuthorityV1::QuarantineExpiry);
                assert!(authority.is_certified());
            }
            other => panic!("expected revoked, got {other:?}"),
        }
        assert!(!RevocationAuthorityV1::LegacySingleValidator.is_certified());
    }

    #[test]
    fn decoded_text_is_resanitised() {
        let json = serde_json::json!({
            "state": "revoked",
            "height": 3,
            "authority": "committee_quorum",
            "reason": "ok\u{1b}]52;c;Zm9v\u{7}\r\u{202e}bad"
        });
        let status: PackageStatusViewV1 = serde_json::from_value(json).unwrap();
        let PackageStatusViewV1::Revoked { reason, .. } = status else {
            panic!("revoked expected");
        };
        let text = reason.as_str();
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\r'));
        assert!(!text.contains('\u{202e}'));
    }

    #[test]
    fn quorum_label_follows_activation() {
        assert_eq!(
            QuorumLabelV1::for_height(10, None),
            QuorumLabelV1::LegacyV3Quorum
        );
        assert_eq!(
            QuorumLabelV1::for_height(9, Some(10)),
            QuorumLabelV1::LegacyV3Quorum
        );
        assert_eq!(
            QuorumLabelV1::for_height(10, Some(10)),
            QuorumLabelV1::V4Quorum
        );
        assert_eq!(
            serde_json::to_value(QuorumLabelV1::LegacyV3Quorum).unwrap(),
            "legacy_v3_quorum"
        );
    }

    #[test]
    fn unknown_violations_never_echo_wire_text() {
        let known = ViolationViewV1::from_wire("KB-MEMFD-EXEC");
        assert_eq!(known.code, Some(KernelViolationCode::KbMemfdExec));
        let unknown = ViolationViewV1::from_wire("<img src=x onerror=alert(1)>");
        assert!(unknown.code.is_none());
        let json = serde_json::to_string(&unknown).unwrap();
        assert!(!json.contains("onerror"));
        assert!(unknown.unknown.unwrap().starts_with("UNKNOWN("));
    }

    #[test]
    fn sse_frames_are_single_line_json_with_resumable_ids() {
        let event = SseEnvelopeV2::new(
            SseEventKindV2::PackageQuarantined,
            48_211,
            3,
            Some("npm/left-padx@1.4.2\nevent: fake"),
            serde_json::json!({"reason": "multi\nline"}),
        );
        let frame = event.to_sse_frame();
        assert!(frame.starts_with("id: 48211.3\nevent: package_quarantined\ndata: {"));
        assert_eq!(frame.matches('\n').count(), 4, "{frame}");
        assert_eq!(SseEnvelopeV2::parse_id("48211.3"), Some((48_211, 3)));
        assert_eq!(SseEnvelopeV2::parse_id("garbage"), None);
        assert!(SseEventKindV2::PackageRevokedV2.fails_ci());
        assert!(!SseEventKindV2::BlockCommitted.fails_ci());
    }

    #[test]
    fn committee_summary_counts_dispositions_and_families() {
        let vote = |approve: bool, family: Option<&str>, pq: bool| ValidatorSignature {
            vote: if approve {
                common::ValidatorVote::Approve
            } else {
                common::ValidatorVote::Reject { reason: "r".into() }
            },
            semantic_vote: family.map(|family| common::SemanticVoteV4 {
                disposition: common::SemanticDispositionV4::Approve,
                model_family: family.into(),
                finding_digests: vec![],
            }),
            pq_signature: pq.then(|| "00".into()),
            ..ValidatorSignature::default()
        };
        let summary = CommitteeSummaryV1::from_votes(
            5,
            &[
                vote(true, Some("Qwen2.5-Coder"), true),
                vote(true, Some("qwen2.5-coder"), true),
                vote(false, None, false),
            ],
        );
        assert_eq!(summary.approvals, 2);
        assert_eq!(summary.rejections, 1);
        assert_eq!(summary.abstentions, 2);
        assert_eq!(summary.model_families.get("qwen2.5-coder"), Some(&2));
        assert_eq!((summary.hybrid_signers, summary.classical_signers), (2, 1));
    }
}
