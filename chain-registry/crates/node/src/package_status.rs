//! ADR-019 Increment 2, part 2: deterministic package-status transitions and
//! the `BlockHeader.status_root` commitment (`ECA-02`, `ECA-10`,
//! `ECO-INV-02`, `ECO-INV-04`).
//!
//! Every block reduces to a set of [`StatusLeafV1`] changes, applied in this
//! order:
//!
//! 1. Quarantines whose `expires_at_unix <= header.timestamp` become
//!    `revoked` (fail-closed; `consensus::vote_v4::resolve_quarantine_timeout`).
//! 2. Transactions, in block order:
//!    * a publication makes the version `verified`;
//!    * a revocation makes it `revoked` (`certified` for an owner/controller
//!      self-revocation or a committee certificate);
//!    * `QuarantineV1` holds a verified version for
//!      `CLASS_C_QUARANTINE_TTL_SECS` from the block time;
//!    * `QuarantineResolutionV1` releases (`verified`) or rejects (`revoked`)
//!      a live quarantine.
//!
//! Leaves are kept for every height, so the first post-activation root
//! commits to the whole registry. From `hybrid_signature_activation_height`
//! on, every header must carry the root of the tree *after* its block.
//!
//! Every certificate is checked with the consensus crate's
//! [`consensus::vote_v4::verify_committee_qc_with_activation`] (verifier
//! parity, `ECO-INV-03`). The signing committee is either the record's
//! snapshot-bound analysis committee (valid assignment proof) or, for legacy
//! records and never-published versions, the full active validator set at the
//! certificate's authority height. A signer counts only when it is active with
//! the same key both at the certificate's authority height and at the block's
//! height ([`signing_authority`]), so a validator that has left or rotated its
//! key cannot sign under an old authority height.

use anyhow::{bail, Context, Result};
use common::status_tree::{split_canonical, Hash32};
use common::{
    Block, ChainRecord, CommitteeQuorumCertificateV1, CregFindingCategoryV1, PackageStatus,
    QuarantineOutcomeV1, StatusLeafStateV1, StatusLeafV1, StatusTree, Transaction,
    TriggerWitnessV1, Validator, APPROVAL_QC_DOMAIN, CLASS_C_QUARANTINE_TTL_SECS,
    QUARANTINE_QC_DOMAIN, QUARANTINE_RESOLUTION_QC_DOMAIN, REVOKE_QC_DOMAIN,
};
#[cfg(test)]
pub use consensus::vote_v4::full_set_committee;
pub use consensus::vote_v4::{assigned_committee, signing_authority};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::chain_store::ChainStore;
use crate::namespace_consensus::NamespaceConsensusConfig;

/// Witnesses kept per package version in the `witness:` index (newest last).
pub const MAX_WITNESSES_PER_VERSION: usize = 16;

const COMMITTEE_REJECTED_REASON: &str = "Class C quarantine rejected by committee certificate";

/// Who signed a `RevokeV2`, as established by namespace validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevocationAuthority {
    /// The version's publisher.
    Owner,
    /// The namespace controller (or legacy namespace owner).
    Controller,
    /// An active validator; needs a committee certificate after activation.
    Validator,
}

impl RevocationAuthority {
    pub fn is_self_revocation(self) -> bool {
        matches!(self, Self::Owner | Self::Controller)
    }
}

/// One indexed trigger witness (`witness:<package>:<version>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessIndexEntryV1 {
    pub package_canonical: String,
    pub witness_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qc_digest: Option<String>,
    pub height: u64,
    pub tx_index: u32,
    pub witness: TriggerWitnessV1,
}

/// Reverse index entry (`witness_id:<hex32>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessIdEntryV1 {
    pub package_canonical: String,
    pub height: u64,
}

/// Status changes one block makes, computed before anything is written.
#[derive(Debug, Clone, Default)]
pub struct StatusBlockEffects {
    /// True when these effects are complete and must be checked against the
    /// header and written. False for an idempotent replay of a block the
    /// index already reflects, and for transitions-only checks
    /// (`validate_block_transitions`).
    pub applied: bool,
    /// Final leaf for every version this block changed.
    pub leaves: BTreeMap<String, StatusLeafV1>,
    /// Quarantine expiry index: `true` adds, `false` removes.
    pub expiry_index: BTreeMap<(i64, String), bool>,
    /// `ChainRecord.status` for versions whose quarantine state changed.
    pub record_status: BTreeMap<String, PackageStatus>,
    pub witnesses: Vec<WitnessIndexEntryV1>,
    /// Root after this block; `Some` exactly from activation on.
    pub status_root: Option<Hash32>,
}

impl StatusBlockEffects {
    /// The header must commit to exactly the computed root (and to none
    /// before activation).
    pub fn check_header(&self, block: &Block) -> Result<()> {
        if !self.applied || block.header.status_root == self.status_root {
            return Ok(());
        }
        bail!(
            "block status_root {} does not match the computed status commitment {}",
            describe_root(block.header.status_root.as_ref()),
            describe_root(self.status_root.as_ref())
        )
    }
}

fn describe_root(root: Option<&Hash32>) -> String {
    root.map(common::status_tree::hex32_to_string)
        .unwrap_or_else(|| "<none>".to_string())
}

/// Wire code of a finding category (its serde name).
pub fn category_code(category: CregFindingCategoryV1) -> String {
    serde_json::to_value(category)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "other".to_string())
}

/// Leaf for `canonical`, or `None` when the id cannot be split.
pub fn leaf_for(
    canonical: &str,
    content_hash: &str,
    status: StatusLeafStateV1,
    height: u64,
) -> Option<StatusLeafV1> {
    let (package, version) = split_canonical(canonical)?;
    Some(StatusLeafV1 {
        package: package.trim().to_string(),
        version: version.trim().to_string(),
        content_hash: content_hash.trim().to_ascii_lowercase(),
        status,
        height,
    })
}

/// Block-local view over the committed index.
struct Overlay<'a> {
    chain: &'a ChainStore,
    leaves: BTreeMap<String, StatusLeafV1>,
    records: HashMap<String, ChainRecord>,
}

impl<'a> Overlay<'a> {
    fn new(chain: &'a ChainStore) -> Self {
        Self {
            chain,
            leaves: BTreeMap::new(),
            records: HashMap::new(),
        }
    }

    fn leaf(&self, canonical: &str) -> Result<Option<StatusLeafV1>> {
        match self.leaves.get(canonical) {
            Some(leaf) => Ok(Some(leaf.clone())),
            None => self.chain.status_leaf(canonical),
        }
    }

    fn record(&self, canonical: &str) -> Result<Option<ChainRecord>> {
        match self.records.get(canonical) {
            Some(record) => Ok(Some(record.clone())),
            None => self.chain.get_package(canonical),
        }
    }
}

struct Transition<'a, 'b> {
    chain: &'a ChainStore,
    config: &'b NamespaceConsensusConfig,
    height: u64,
    now: i64,
    active: bool,
    overlay: Overlay<'a>,
    effects: StatusBlockEffects,
}

impl Transition<'_, '_> {
    fn set_leaf(
        &mut self,
        canonical: &str,
        content_hash: &str,
        status: StatusLeafStateV1,
    ) -> Result<()> {
        match leaf_for(canonical, content_hash, status, self.height) {
            Some(leaf) => {
                self.overlay
                    .leaves
                    .insert(canonical.to_string(), leaf.clone());
                self.effects.leaves.insert(canonical.to_string(), leaf);
                Ok(())
            }
            // Historical ids that cannot be split stay out of the index; from
            // activation on every committed version must be provable.
            None if self.active => {
                bail!("package id {canonical:?} cannot be committed to the status tree")
            }
            None => Ok(()),
        }
    }

    fn set_expiry(&mut self, expires_at: i64, canonical: &str, present: bool) {
        self.effects
            .expiry_index
            .insert((expires_at, canonical.to_string()), present);
    }

    /// `force` records quarantine-driven changes; later changes in the same
    /// block keep an already recorded entry current.
    fn set_record_status(&mut self, canonical: &str, status: PackageStatus, force: bool) {
        if force || self.effects.record_status.contains_key(canonical) {
            self.effects
                .record_status
                .insert(canonical.to_string(), status);
        }
    }

    fn authority_set(&self, height: u64) -> Result<Vec<Validator>> {
        let bootstrap = self
            .config
            .validator_set_bootstrap
            .as_ref()
            .context("committee and witness checks need the signed validator-set bootstrap")?;
        let set = crate::validator_set_history::verification_set_at(
            self.chain.data_dir(),
            height,
            bootstrap,
        )
        .map_err(anyhow::Error::msg)?;
        Ok(set.validators)
    }

    fn verify_certificate(
        &self,
        qc: &CommitteeQuorumCertificateV1,
        domain: &str,
        package_canonical: &str,
        content_hash: &str,
        record: Option<&ChainRecord>,
    ) -> Result<usize> {
        let at_authority = self.authority_set(qc.authority_height)?;
        let at_block = self.authority_set(self.height)?;
        consensus::vote_v4::verify_committee_qc_for_record(
            qc,
            domain,
            package_canonical,
            content_hash,
            record,
            &at_authority,
            &at_block,
            self.config.vote_v4_activation_height,
            self.height,
        )
    }

    fn verify_witness(
        &self,
        witness: &TriggerWitnessV1,
        package_canonical: &str,
        content_hash: Option<&str>,
    ) -> Result<()> {
        if witness.package_canonical.trim() != package_canonical.trim() {
            bail!("trigger witness targets another package");
        }
        if let Some(content_hash) = content_hash {
            if !witness
                .content_hash
                .trim()
                .eq_ignore_ascii_case(content_hash.trim())
            {
                bail!("trigger witness targets another content hash");
            }
        }
        let authority = self.authority_set(self.height)?;
        let prover = authority
            .iter()
            .find(|validator| {
                validator.id == witness.witness_prover_id.trim()
                    && common::validator_status_is_active(&validator.status)
            })
            .context("trigger witness prover is not an active validator")?;
        let require_hybrid = self.config.vote_v4_active_at(self.height);
        let public_key = prover.verifying_key(require_hybrid)?;
        witness.verify(&public_key, require_hybrid)
    }

    fn expire_quarantines(&mut self) -> Result<()> {
        for (expires_at, canonical) in self.chain.quarantine_expiries_up_to(self.now)? {
            if let Some(leaf) = self.overlay.leaf(&canonical)? {
                if let StatusLeafStateV1::Quarantined {
                    expires_at_unix,
                    reason_code,
                } = leaf.status
                {
                    if expires_at_unix == expires_at {
                        let held = PackageStatus::Quarantined {
                            reason: category_code(reason_code),
                            expires_at_unix,
                        };
                        let revoked =
                            consensus::vote_v4::resolve_quarantine_timeout(&held, self.now)
                                .context("quarantine expiry index is ahead of its deadline")?;
                        self.set_leaf(
                            &canonical,
                            &leaf.content_hash,
                            StatusLeafStateV1::Revoked { certified: true },
                        )?;
                        self.set_record_status(&canonical, revoked, true);
                    }
                }
            }
            self.set_expiry(expires_at, &canonical, false);
        }
        Ok(())
    }

    fn apply_publication(&mut self, tx: &Transaction, record: &ChainRecord) -> Result<()> {
        let canonical = record.id.canonical();
        if self.active {
            if let Some(existing) = self.overlay.leaf(&canonical)? {
                if existing.status != StatusLeafStateV1::Verified {
                    bail!("{canonical} cannot be republished while quarantined or revoked");
                }
                if !existing
                    .content_hash
                    .eq_ignore_ascii_case(record.content_hash.trim())
                {
                    bail!("{canonical} is already committed with another content hash");
                }
            }
            if let Transaction::PublishAuthorizedV1 {
                committee_qc: Some(qc),
                ..
            } = tx
            {
                self.verify_certificate(
                    qc,
                    APPROVAL_QC_DOMAIN,
                    &canonical,
                    &record.content_hash,
                    Some(record),
                )?;
                if !qc
                    .evidence_digest
                    .trim()
                    .eq_ignore_ascii_case(record.evidence_digest.trim())
                {
                    bail!("approval QC does not commit to the record's evidence digest");
                }
            }
        }
        self.overlay
            .records
            .insert(canonical.clone(), record.clone());
        self.set_leaf(
            &canonical,
            &record.content_hash,
            StatusLeafStateV1::Verified,
        )?;
        self.set_record_status(&canonical, record.status.clone(), false);
        Ok(())
    }

    fn apply_revocation(
        &mut self,
        index: usize,
        tx: &Transaction,
        authority: Option<RevocationAuthority>,
    ) -> Result<()> {
        let Some((canonical, reason, _, evidence_hash)) = tx.revocation_target() else {
            return Ok(());
        };
        let record = self.overlay.record(canonical)?;
        let certified = match tx {
            Transaction::RevokeV2 {
                witness,
                quorum_certificate,
                ..
            } => {
                let authority = authority.unwrap_or(RevocationAuthority::Validator);
                let mut certified = authority.is_self_revocation();
                if self.active {
                    let content_hash = record
                        .as_ref()
                        .map(|record| record.content_hash.clone())
                        .or_else(|| {
                            quorum_certificate
                                .as_ref()
                                .map(|qc| qc.content_hash.clone())
                        });
                    match quorum_certificate {
                        Some(qc) => {
                            self.verify_certificate(
                                qc,
                                REVOKE_QC_DOMAIN,
                                canonical,
                                content_hash.as_deref().unwrap_or_default(),
                                record.as_ref(),
                            )?;
                            if !qc
                                .evidence_digest
                                .trim()
                                .eq_ignore_ascii_case(evidence_hash.trim())
                            {
                                bail!("revocation QC does not commit to the evidence hash");
                            }
                            certified = true;
                        }
                        None if authority.is_self_revocation() => {}
                        // A rejected submission that was never published
                        // changes no committed state; it stays uncertified,
                        // so clients do not treat it as a revocation.
                        None if record.is_none() => {}
                        None => bail!(
                            "a validator revocation of a committed version needs a committee quorum certificate after hybrid activation"
                        ),
                    }
                    let qc_witness = quorum_certificate
                        .as_ref()
                        .and_then(|qc| qc.witness_digest.as_deref())
                        .map(|digest| digest.trim().to_ascii_lowercase());
                    match witness {
                        Some(witness) => {
                            self.verify_witness(witness, canonical, content_hash.as_deref())?;
                            let digest = witness.canonical_digest();
                            if quorum_certificate.is_some()
                                && qc_witness.as_deref() != Some(digest.as_str())
                            {
                                bail!("revocation QC does not commit to the carried witness");
                            }
                            self.effects.witnesses.push(WitnessIndexEntryV1 {
                                package_canonical: canonical.trim().to_string(),
                                witness_digest: digest,
                                qc_digest: quorum_certificate.as_ref().map(|qc| qc.digest()),
                                height: self.height,
                                tx_index: u32::try_from(index).unwrap_or(u32::MAX),
                                witness: witness.clone(),
                            });
                        }
                        None if qc_witness.is_some() => {
                            bail!("revocation QC names a witness the transaction does not carry")
                        }
                        None => {}
                    }
                }
                certified
            }
            // Unsigned historical revocations are never certified and are
            // invalid from activation on (`ECO-INV-04`).
            _ if self.active => bail!("legacy unsigned revocation after hybrid activation"),
            _ => false,
        };
        let Some(record) = record else {
            // Mirrors the package index: revoking an unknown version records
            // the transaction but changes no package state.
            return Ok(());
        };
        if let Some(StatusLeafStateV1::Quarantined {
            expires_at_unix, ..
        }) = self.overlay.leaf(canonical)?.map(|leaf| leaf.status)
        {
            self.set_expiry(expires_at_unix, canonical, false);
        }
        self.set_leaf(
            canonical,
            &record.content_hash,
            StatusLeafStateV1::Revoked { certified },
        )?;
        self.set_record_status(
            canonical,
            PackageStatus::Revoked {
                reason: reason.to_string(),
            },
            false,
        );
        Ok(())
    }

    fn apply_quarantine(
        &mut self,
        record: &ChainRecord,
        reason_code: CregFindingCategoryV1,
        qc: &CommitteeQuorumCertificateV1,
    ) -> Result<()> {
        let canonical = record.id.canonical();
        let current = self
            .overlay
            .leaf(&canonical)?
            .with_context(|| format!("quarantine targets unpublished version {canonical}"))?;
        if current.status != StatusLeafStateV1::Verified {
            bail!("only a verified version can be quarantined");
        }
        let stored = self
            .overlay
            .record(&canonical)?
            .with_context(|| format!("quarantine target {canonical} has no package record"))?;
        if !stored
            .content_hash
            .trim()
            .eq_ignore_ascii_case(record.content_hash.trim())
            || !stored
                .publisher_pubkey
                .trim()
                .eq_ignore_ascii_case(record.publisher_pubkey.trim())
        {
            bail!("quarantine record does not match the committed version");
        }
        self.verify_certificate(
            qc,
            QUARANTINE_QC_DOMAIN,
            &canonical,
            &stored.content_hash,
            Some(&stored),
        )?;
        let expires_at_unix = self.now.saturating_add(CLASS_C_QUARANTINE_TTL_SECS);
        self.set_leaf(
            &canonical,
            &stored.content_hash,
            StatusLeafStateV1::Quarantined {
                expires_at_unix,
                reason_code,
            },
        )?;
        self.set_expiry(expires_at_unix, &canonical, true);
        self.set_record_status(
            &canonical,
            PackageStatus::Quarantined {
                reason: category_code(reason_code),
                expires_at_unix,
            },
            true,
        );
        Ok(())
    }

    fn apply_resolution(
        &mut self,
        package_canonical: &str,
        content_hash: &str,
        outcome: QuarantineOutcomeV1,
        qc: &CommitteeQuorumCertificateV1,
    ) -> Result<()> {
        let canonical = package_canonical.trim();
        let current = self
            .overlay
            .leaf(canonical)?
            .with_context(|| format!("resolution targets unknown version {canonical}"))?;
        let StatusLeafStateV1::Quarantined {
            expires_at_unix, ..
        } = current.status
        else {
            bail!("resolution targets a version that is not quarantined");
        };
        if expires_at_unix <= self.now {
            bail!("quarantine already expired");
        }
        if !current
            .content_hash
            .eq_ignore_ascii_case(content_hash.trim())
        {
            bail!("resolution targets another content hash");
        }
        if qc.decision != Some(outcome) {
            bail!("resolution QC does not commit to this outcome");
        }
        let record = self.overlay.record(canonical)?;
        self.verify_certificate(
            qc,
            QUARANTINE_RESOLUTION_QC_DOMAIN,
            canonical,
            &current.content_hash,
            record.as_ref(),
        )?;
        self.set_expiry(expires_at_unix, canonical, false);
        let (leaf, status) = match outcome {
            QuarantineOutcomeV1::Release => (StatusLeafStateV1::Verified, PackageStatus::Verified),
            QuarantineOutcomeV1::Reject => (
                StatusLeafStateV1::Revoked { certified: true },
                PackageStatus::Revoked {
                    reason: COMMITTEE_REJECTED_REASON.to_string(),
                },
            ),
        };
        self.set_leaf(canonical, &current.content_hash, leaf)?;
        self.set_record_status(canonical, status, true);
        Ok(())
    }

    fn compute_root(&self) -> Result<Hash32> {
        if self.effects.leaves.is_empty() {
            if let Some((height, root)) = self.chain.status_root_record()? {
                if height.saturating_add(1) == self.height {
                    return Ok(root);
                }
            }
        }
        let mut tree = StatusTree::new();
        for leaf in self.chain.all_status_leaves()? {
            tree.upsert(leaf);
        }
        for leaf in self.effects.leaves.values() {
            tree.upsert(leaf.clone());
        }
        Ok(tree.root())
    }
}

/// Compute (and validate) the status transitions of `block` against the
/// committed index. `revocations` maps the block index of each `RevokeV2` to
/// the signer's authority as established by namespace validation. Without
/// `compute_root` the effects are for checking only: `status_root` stays
/// `None` and [`StatusBlockEffects::applied`] is false, so they can neither
/// pass a header check nor be written.
pub fn apply_block(
    chain: &ChainStore,
    block: &Block,
    config: &NamespaceConsensusConfig,
    revocations: &BTreeMap<usize, RevocationAuthority>,
    compute_root: bool,
) -> Result<StatusBlockEffects> {
    let height = block.header.height;
    let indexed_tip = chain.status_indexed_tip()?;
    if indexed_tip.is_some_and(|indexed| indexed >= height)
        && chain
            .get_block_by_height(height)?
            .is_some_and(|stored| stored.hash() == block.hash())
    {
        return Ok(StatusBlockEffects::default());
    }
    // A root is only meaningful over the complete history. Before
    // activation a lagging index is repaired by the startup rebuild; from
    // activation on it must never be used.
    if config.vote_v4_active_at(height) && indexed_tip != height.checked_sub(1) {
        bail!(
            "package-status index is at height {:?}, not {:?}; rebuild the package index before validating post-activation blocks",
            indexed_tip,
            height.checked_sub(1)
        );
    }
    let mut transition = Transition {
        chain,
        config,
        height,
        now: block.header.timestamp.timestamp(),
        active: config.vote_v4_active_at(height),
        overlay: Overlay::new(chain),
        effects: StatusBlockEffects {
            applied: true,
            ..StatusBlockEffects::default()
        },
    };
    transition.expire_quarantines()?;
    for (index, tx) in block.transactions.iter().enumerate() {
        let outcome = match tx {
            Transaction::Publish(record) | Transaction::PublishAuthorizedV1 { record, .. } => {
                transition.apply_publication(tx, record)
            }
            Transaction::Revoke { .. } | Transaction::RevokeV2 { .. } => {
                transition.apply_revocation(index, tx, revocations.get(&index).copied())
            }
            Transaction::QuarantineV1 {
                record,
                reason_code,
                quorum_certificate,
            } if transition.active => {
                transition.apply_quarantine(record, *reason_code, quorum_certificate)
            }
            Transaction::QuarantineResolutionV1 {
                package_canonical,
                content_hash,
                outcome,
                quorum_certificate,
            } if transition.active => transition.apply_resolution(
                package_canonical,
                content_hash,
                *outcome,
                quorum_certificate,
            ),
            Transaction::QuarantineV1 { .. } | Transaction::QuarantineResolutionV1 { .. } => Err(
                anyhow::anyhow!("quarantine transactions are invalid before hybrid activation"),
            ),
            _ => Ok(()),
        };
        outcome
            .with_context(|| format!("invalid package-status transition at block index {index}"))?;
    }
    if !compute_root {
        transition.effects.applied = false;
    } else if transition.active {
        let root = transition.compute_root()?;
        transition.effects.status_root = Some(root);
    }
    Ok(transition.effects)
}

/// Store fixture with hybrid validators and producer-style sealing, shared by
/// the package-status and v4 read-route tests.
#[cfg(test)]
pub(crate) mod test_fixture {
    use super::*;
    use crate::namespace_consensus::validate_block_body;
    use chrono::{DateTime, Duration, Utc};
    use common::crypto::SigningKey;
    use common::{BlockHeader, PackageId, SignedRevocationRequest, ValidatorSet};

    pub(crate) struct Member {
        pub(crate) validator: Validator,
        pub(crate) hybrid: SigningKey,
        pub(crate) ed: SigningKey,
    }

    pub(crate) fn member(index: u8) -> Member {
        let ed_seed = [index; 32];
        let ed = SigningKey::from_ed25519_seed(&ed_seed);
        let hybrid = SigningKey::hybrid_from_seeds(&ed_seed, &[index.wrapping_add(90); 32]);
        let (_, pq_pubkey) = hybrid.public_key().hybrid_parts().expect("hybrid key");
        let id = format!("validator-{index}");
        Member {
            validator: Validator {
                id: id.clone(),
                alias: id,
                pubkey: ed.public_key().to_hex(),
                pq_pubkey: Some(hex::encode(pq_pubkey)),
                status: "active".into(),
                stake: 1,
                ..Validator::default()
            },
            hybrid,
            ed,
        }
    }

    pub(crate) fn record(name: &str, publisher_pubkey: &str) -> ChainRecord {
        ChainRecord {
            id: PackageId::new("npm", name, "1.0.0"),
            content_hash: common::sha256_hex(name.as_bytes()),
            publisher_pubkey: publisher_pubkey.to_string(),
            status: PackageStatus::Verified,
            evidence_digest: common::sha256_hex(b"evidence"),
            ..ChainRecord::default()
        }
    }

    pub(crate) struct Fixture {
        _dir: tempfile::TempDir,
        pub(crate) store: ChainStore,
        pub(crate) config: NamespaceConsensusConfig,
        pub(crate) members: Vec<Member>,
        pub(crate) start: DateTime<Utc>,
    }

    pub(crate) fn fixture(activation: Option<u64>) -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open store");
        let members: Vec<Member> = (1..=4).map(member).collect();
        let config = NamespaceConsensusConfig {
            vote_v4_activation_height: activation,
            validator_set_bootstrap: Some(ValidatorSet::new(
                members.iter().map(|m| m.validator.clone()).collect(),
            )),
            ..NamespaceConsensusConfig::default()
        };
        store
            .configure_namespace_consensus(config.clone())
            .expect("configure store");
        let start = store
            .get_block_by_height(0)
            .expect("read genesis")
            .expect("genesis")
            .header
            .timestamp;
        Fixture {
            _dir: dir,
            store,
            config,
            members,
            start,
        }
    }

    impl Fixture {
        pub(crate) fn block(&self, offset_secs: i64, transactions: Vec<Transaction>) -> Block {
            let tip = self.store.tip_height().expect("tip");
            let parent = self
                .store
                .get_block_by_height(tip)
                .expect("read parent")
                .expect("parent");
            Block {
                header: BlockHeader {
                    height: tip + 1,
                    prev_hash: parent.hash(),
                    merkle_root: common::merkle_root(&transactions),
                    proposer_id: "status-test".into(),
                    timestamp: self.start + Duration::seconds(offset_secs),
                    validator_set_hash: "test".into(),
                    vrf_output: None,
                    vrf_proof: None,
                    view_number: 0,
                    view_change_certificate: vec![],
                    status_root: None,
                },
                transactions,
                pbft_signatures: vec![],
            }
        }

        /// Seal as the producer does (root from `validate_block_body`), then
        /// insert through the full validator.
        pub(crate) fn commit(&self, mut block: Block) -> Result<Block> {
            block.header.status_root = validate_block_body(&self.store, &block, &self.config)?
                .status
                .status_root;
            self.store.insert_block(&block)?;
            Ok(block)
        }

        pub(crate) fn qc(
            &self,
            domain: &str,
            canonical: &str,
            content_hash: &str,
            decision: Option<QuarantineOutcomeV1>,
            signers: usize,
        ) -> CommitteeQuorumCertificateV1 {
            let authority: Vec<Validator> =
                self.members.iter().map(|m| m.validator.clone()).collect();
            let mut qc = CommitteeQuorumCertificateV1::new(
                domain,
                canonical,
                content_hash,
                &common::sha256_hex(b"evidence"),
                &full_set_committee(&authority),
                1,
            );
            qc.decision = decision;
            for member in self.members.iter().take(signers) {
                qc.add_signature(qc.sign_member(&member.validator.id, &member.hybrid));
            }
            qc
        }

        pub(crate) fn leaf(&self, canonical: &str) -> StatusLeafV1 {
            self.store
                .status_leaf(canonical)
                .expect("read leaf")
                .expect("leaf")
        }
    }

    pub(crate) fn revocation(key: &SigningKey, canonical: &str) -> SignedRevocationRequest {
        let mut request = SignedRevocationRequest {
            chain_id: "creg-test".into(),
            genesis_hash: "0".repeat(64),
            package_canonical: canonical.to_string(),
            reason: "malicious install hook".into(),
            revoker_pubkey: key.public_key().to_hex(),
            signature: String::new(),
            signature_version: 1,
        };
        request.signature = key.sign(request.signing_message().as_bytes()).to_hex();
        request
    }

    pub(crate) fn revoke_v2(
        request: SignedRevocationRequest,
        quorum_certificate: Option<CommitteeQuorumCertificateV1>,
    ) -> Transaction {
        Transaction::RevokeV2 {
            request,
            evidence_hash: common::sha256_hex(b"evidence"),
            witness: None,
            quorum_certificate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixture::*;
    use super::*;
    use crate::namespace_consensus::validate_block_body;
    use chrono::Duration;
    use common::crypto::SigningKey;

    #[test]
    fn pre_activation_index_tracks_status_without_a_root() {
        let fx = fixture(None);
        let rec = record("alpha", "publisher");
        let canonical = rec.id.canonical();
        let published = fx
            .commit(fx.block(10, vec![Transaction::Publish(rec.clone())]))
            .expect("publish");
        assert_eq!(published.header.status_root, None);
        let leaf = fx.leaf(&canonical);
        assert_eq!(leaf.status, StatusLeafStateV1::Verified);
        assert_eq!(leaf.height, 1);

        fx.commit(fx.block(
            20,
            vec![Transaction::Revoke {
                package_canonical: canonical.clone(),
                reason: "legacy".into(),
                revoked_by: "validator".into(),
                evidence_hash: "e".into(),
            }],
        ))
        .expect("legacy revocation before activation");
        assert_eq!(
            fx.leaf(&canonical).status,
            StatusLeafStateV1::Revoked { certified: false }
        );
        assert_eq!(fx.store.status_root_record().expect("root record"), None);
        assert!(!fx.store.package_status_index_is_stale().expect("stale"));
    }

    #[test]
    fn post_activation_header_must_commit_to_the_status_root() {
        let fx = fixture(Some(1));
        let rec = record("alpha", "publisher");
        let canonical = rec.id.canonical();
        let block = fx.block(10, vec![Transaction::Publish(rec.clone())]);

        let missing = fx.store.insert_block(&block).unwrap_err();
        assert!(
            format!("{missing:#}").contains("status_root"),
            "{missing:#}"
        );
        let mut wrong = block.clone();
        wrong.header.status_root = Some([7; 32]);
        assert!(fx.store.insert_block(&wrong).is_err());
        assert_eq!(fx.store.tip_height().expect("tip"), 0);

        let committed = fx.commit(block).expect("commit");
        let root = committed.header.status_root.expect("post-activation root");
        let tree = fx.store.status_tree().expect("tree");
        assert_eq!(tree.root(), root);
        assert_eq!(
            fx.store.status_root_record().expect("record"),
            Some((1, root))
        );
        let (package, version) = split_canonical(&canonical).expect("split");
        let leaf = fx.leaf(&canonical);
        assert!(tree
            .prove(package, version)
            .verify(&root, package, version, Some(&leaf)));

        // A block without status changes keeps the root.
        let empty = fx.commit(fx.block(20, vec![])).expect("empty block");
        assert_eq!(empty.header.status_root, Some(root));

        // Rebuilding from the chain reproduces the commitment.
        fx.store.rebuild_package_index().expect("rebuild");
        assert_eq!(
            fx.store.status_root_record().expect("record"),
            Some((2, root))
        );

        // Unsigned historical revocations end at activation (ECO-INV-04).
        let legacy = fx.block(
            30,
            vec![Transaction::Revoke {
                package_canonical: canonical,
                reason: "legacy".into(),
                revoked_by: "validator".into(),
                evidence_hash: "e".into(),
            }],
        );
        let error = validate_block_body(&fx.store, &legacy, &fx.config).unwrap_err();
        assert!(format!("{error:#}").contains("legacy unsigned revocation"));
    }

    #[test]
    fn quarantine_needs_a_committee_quorum_and_expires_fail_closed() {
        let fx = fixture(Some(1));
        let rec = record("beta", "publisher");
        let canonical = rec.id.canonical();
        fx.commit(fx.block(10, vec![Transaction::Publish(rec.clone())]))
            .expect("publish");
        let quarantine = |qc| Transaction::QuarantineV1 {
            record: rec.clone(),
            reason_code: CregFindingCategoryV1::Backdoor,
            quorum_certificate: qc,
        };

        // 4 signers -> threshold 3: two signatures are not a certificate.
        let weak = fx.qc(QUARANTINE_QC_DOMAIN, &canonical, &rec.content_hash, None, 2);
        let error = fx.commit(fx.block(20, vec![quarantine(weak)])).unwrap_err();
        assert!(format!("{error:#}").contains("valid signer"), "{error:#}");
        // A certificate for another domain is refused too.
        let revoke_qc = fx.qc(REVOKE_QC_DOMAIN, &canonical, &rec.content_hash, None, 4);
        assert!(fx
            .commit(fx.block(20, vec![quarantine(revoke_qc)]))
            .is_err());

        let strong = fx.qc(QUARANTINE_QC_DOMAIN, &canonical, &rec.content_hash, None, 3);
        fx.commit(fx.block(20, vec![quarantine(strong)]))
            .expect("certified quarantine");
        let deadline = (fx.start + Duration::seconds(20)).timestamp() + CLASS_C_QUARANTINE_TTL_SECS;
        assert_eq!(
            fx.leaf(&canonical).status,
            StatusLeafStateV1::Quarantined {
                expires_at_unix: deadline,
                reason_code: CregFindingCategoryV1::Backdoor,
            }
        );
        assert!(matches!(
            fx.store.get_package(&canonical).expect("read").expect("record").status,
            PackageStatus::Quarantined { expires_at_unix, .. } if expires_at_unix == deadline
        ));
        // A held version cannot be republished as verified.
        let republish = fx.block(30, vec![Transaction::Publish(rec.clone())]);
        assert!(validate_block_body(&fx.store, &republish, &fx.config).is_err());

        // One second before the deadline the hold stands.
        fx.commit(fx.block(20 + CLASS_C_QUARANTINE_TTL_SECS - 1, vec![]))
            .expect("before deadline");
        assert!(matches!(
            fx.leaf(&canonical).status,
            StatusLeafStateV1::Quarantined { .. }
        ));

        // At the deadline it is revoked, fail-closed and certified.
        fx.commit(fx.block(20 + CLASS_C_QUARANTINE_TTL_SECS, vec![]))
            .expect("at deadline");
        let leaf = fx.leaf(&canonical);
        assert_eq!(leaf.status, StatusLeafStateV1::Revoked { certified: true });
        assert_eq!(leaf.height, 4);
        match fx
            .store
            .get_package(&canonical)
            .expect("read")
            .expect("record")
            .status
        {
            PackageStatus::Revoked { reason } => {
                assert!(reason.contains("TTL expired"), "{reason}")
            }
            other => panic!("expected revoked, got {other:?}"),
        }
        assert!(fx
            .store
            .quarantine_expiries_up_to(i64::MAX)
            .expect("expiries")
            .is_empty());

        let root = fx.store.status_root_record().expect("record");
        fx.store.rebuild_package_index().expect("rebuild");
        assert_eq!(fx.store.status_root_record().expect("record"), root);
        assert_eq!(fx.leaf(&canonical), leaf);
    }

    #[test]
    fn resolution_and_validator_revocation_need_matching_certificates() {
        let fx = fixture(Some(1));
        let rec = record("gamma", "publisher");
        let canonical = rec.id.canonical();
        fx.commit(fx.block(10, vec![Transaction::Publish(rec.clone())]))
            .expect("publish");
        let hold = fx.qc(QUARANTINE_QC_DOMAIN, &canonical, &rec.content_hash, None, 3);
        fx.commit(fx.block(
            20,
            vec![Transaction::QuarantineV1 {
                record: rec.clone(),
                reason_code: CregFindingCategoryV1::InstallHookAbuse,
                quorum_certificate: hold,
            }],
        ))
        .expect("quarantine");

        let resolution = |outcome, qc| Transaction::QuarantineResolutionV1 {
            package_canonical: canonical.clone(),
            content_hash: rec.content_hash.clone(),
            outcome,
            quorum_certificate: qc,
        };
        // The certificate binds the decision.
        let reject_qc = fx.qc(
            QUARANTINE_RESOLUTION_QC_DOMAIN,
            &canonical,
            &rec.content_hash,
            Some(QuarantineOutcomeV1::Reject),
            3,
        );
        let error = fx
            .commit(fx.block(
                30,
                vec![resolution(QuarantineOutcomeV1::Release, reject_qc)],
            ))
            .unwrap_err();
        assert!(format!("{error:#}").contains("outcome"), "{error:#}");

        let release_qc = fx.qc(
            QUARANTINE_RESOLUTION_QC_DOMAIN,
            &canonical,
            &rec.content_hash,
            Some(QuarantineOutcomeV1::Release),
            3,
        );
        fx.commit(fx.block(
            30,
            vec![resolution(QuarantineOutcomeV1::Release, release_qc)],
        ))
        .expect("release");
        assert_eq!(fx.leaf(&canonical).status, StatusLeafStateV1::Verified);
        assert_eq!(
            fx.store
                .get_package(&canonical)
                .expect("read")
                .expect("record")
                .status,
            PackageStatus::Verified
        );
        // The released hold no longer expires.
        fx.commit(fx.block(20 + CLASS_C_QUARANTINE_TTL_SECS + 5, vec![]))
            .expect("past old deadline");
        assert_eq!(fx.leaf(&canonical).status, StatusLeafStateV1::Verified);

        // A validator alone can no longer revoke (ECA-02).
        let validator_key = &fx.members[0].ed;
        let bare = fx.block(
            40 + CLASS_C_QUARANTINE_TTL_SECS,
            vec![revoke_v2(revocation(validator_key, &canonical), None)],
        );
        let error = validate_block_body(&fx.store, &bare, &fx.config).unwrap_err();
        assert!(
            format!("{error:#}").contains("committee quorum certificate"),
            "{error:#}"
        );
        // A certificate that names a witness the transaction lacks is refused.
        let mut named = fx.qc(REVOKE_QC_DOMAIN, &canonical, &rec.content_hash, None, 0);
        named.witness_digest = Some("cd".repeat(32));
        for member in fx.members.iter().take(3) {
            named.add_signature(named.sign_member(&member.validator.id, &member.hybrid));
        }
        let unbacked = fx.block(
            40 + CLASS_C_QUARANTINE_TTL_SECS,
            vec![revoke_v2(
                revocation(validator_key, &canonical),
                Some(named),
            )],
        );
        assert!(validate_block_body(&fx.store, &unbacked, &fx.config).is_err());

        let certified = fx.qc(REVOKE_QC_DOMAIN, &canonical, &rec.content_hash, None, 3);
        // Authority height 1 is still inside the window at this height.
        fx.commit(fx.block(
            40 + CLASS_C_QUARANTINE_TTL_SECS,
            vec![revoke_v2(
                revocation(validator_key, &canonical),
                Some(certified),
            )],
        ))
        .expect("certified validator revocation");
        assert_eq!(
            fx.leaf(&canonical).status,
            StatusLeafStateV1::Revoked { certified: true }
        );
    }

    #[test]
    fn owner_self_revocation_is_certified_without_a_committee() {
        let fx = fixture(Some(1));
        let owner = SigningKey::from_ed25519_seed(&[77; 32]);
        let rec = record("delta", &owner.public_key().to_hex());
        let canonical = rec.id.canonical();
        fx.commit(fx.block(10, vec![Transaction::Publish(rec.clone())]))
            .expect("publish");
        fx.commit(fx.block(20, vec![revoke_v2(revocation(&owner, &canonical), None)]))
            .expect("owner revocation");
        assert_eq!(
            fx.leaf(&canonical).status,
            StatusLeafStateV1::Revoked { certified: true }
        );
    }

    #[test]
    fn signers_must_be_active_with_the_same_key_at_both_heights() {
        let members: Vec<Member> = (1..=4).map(member).collect();
        let at_authority: Vec<Validator> = members.iter().map(|m| m.validator.clone()).collect();

        // Later: validator-1 left, validator-2 is inactive, validator-3
        // rotated its post-quantum key, validator-4 is unchanged.
        let rotated = SigningKey::hybrid_from_seeds(&[3; 32], &[200; 32]);
        let (_, rotated_pq) = rotated.public_key().hybrid_parts().expect("hybrid key");
        let mut at_block: Vec<Validator> = at_authority[1..].to_vec();
        at_block[0].status = "inactive".into();
        at_block[1].pq_pubkey = Some(hex::encode(rotated_pq));

        let hybrid: Vec<String> = signing_authority(&at_authority, &at_block, true)
            .into_iter()
            .map(|validator| validator.id)
            .collect();
        assert_eq!(hybrid, vec!["validator-4".to_string()]);
        // Before activation only the Ed25519 key matters, so the PQ rotation
        // does not exclude validator-3.
        let classical: Vec<String> = signing_authority(&at_authority, &at_block, false)
            .into_iter()
            .map(|validator| validator.id)
            .collect();
        assert_eq!(
            classical,
            vec!["validator-3".to_string(), "validator-4".to_string()]
        );
    }

    #[test]
    fn expanded_record_committee_keeps_its_assignment_proof() {
        use common::AnalysisCommittee;
        let mut committee = AnalysisCommittee {
            size: 2,
            members: vec![
                "validator-1".into(),
                "validator-2".into(),
                "validator-3".into(),
            ],
            seed: "seed".into(),
            effective_size: 2,
            validator_snapshot_hash: "snapshot".into(),
            selection_randomness: "randomness".into(),
            ..AnalysisCommittee::default()
        };
        let mut rec = record("zeta", "publisher");
        committee.assignment_proof =
            common::committee_assignment_proof(&rec.id.canonical(), &rec.content_hash, &committee);
        // The pipeline expanded the committee after admission.
        committee.effective_size = 3;
        rec.analysis_committee = Some(committee.clone());
        assert_eq!(assigned_committee(&rec).map(|c| c.effective_size), Some(3));
        // Any other edit still invalidates the assignment.
        committee.members.swap(0, 1);
        rec.analysis_committee = Some(committee);
        assert!(assigned_committee(&rec).is_none());
    }

    #[test]
    fn witness_replay_guard_survives_rebuild_and_reopen() {
        use consensus::vote_v4::{
            WitnessBogusPenaltyV1, WitnessReplayGuard, WitnessReplayGuardStateV1,
            WITNESS_REPLAY_GUARD_STATE_VERSION,
        };
        let fx = fixture(Some(1));
        assert_eq!(
            fx.store
                .load_witness_replay_guard()
                .expect("load empty")
                .bogus_count("validator-9"),
            0
        );
        let guard = WitnessReplayGuard::restore(&WitnessReplayGuardStateV1 {
            schema_version: WITNESS_REPLAY_GUARD_STATE_VERSION,
            bogus_penalties: vec![WitnessBogusPenaltyV1 {
                validator_id: "validator-9".into(),
                count: 2,
            }],
            ..WitnessReplayGuardStateV1::default()
        })
        .expect("restore");
        fx.store.save_witness_replay_guard(&guard).expect("save");
        fx.store.rebuild_package_index().expect("rebuild");
        assert_eq!(
            fx.store
                .load_witness_replay_guard()
                .expect("load")
                .bogus_count("validator-9"),
            2
        );
    }

    #[test]
    fn uncertified_rejection_of_an_unpublished_version_changes_nothing() {
        let fx = fixture(Some(1));
        let unpublished = record("eta", "publisher").id.canonical();
        let validator_key = &fx.members[0].ed;
        fx.commit(fx.block(
            10,
            vec![revoke_v2(revocation(validator_key, &unpublished), None)],
        ))
        .expect("rejection of a never-published submission");
        assert!(fx.store.status_leaf(&unpublished).expect("read").is_none());
        assert!(fx.store.get_package(&unpublished).expect("read").is_none());
    }
}
