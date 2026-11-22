//! Equivocation (double-signing) detection and portable, verifiable evidence.
//!
//! A validator equivocates when it signs two different blocks in a slot where
//! its durable signing journal forbids it. Two PBFT votes prove that only if a
//! third party can tell *which slot* each vote was cast in, and the signed
//! message (`creg-pbft-v1:<phase>:<block_hash>`) binds only the phase and the
//! block hash. The height, parent and view come from the block header, which
//! the block hash commits to. Evidence therefore carries both headers, and
//! [`EquivocationEvidence::verify`] re-derives everything from them.
//!
//! Slot rules, mirroring the locks in `node::pbft_safety` exactly:
//!
//! | Phase   | Conflict                                                         |
//! |---------|------------------------------------------------------------------|
//! | PREPARE | two blocks with the same height, same parent and **same view**   |
//! | COMMIT  | two blocks at the **same height**, whatever the parent or view   |
//!
//! The journal keeps PREPARE locks per (height, parent, view) and refuses a
//! second COMMIT at a height even on another fork ("a chain rollback cannot
//! authorize a different COMMIT at the same height"). Anything the journal
//! allows is therefore never evidence: PREPAREs in different views, or at
//! different heights, are legitimate after a view change.
//!
//! See `docs/adr/ADR-016-EQUIVOCATION-SLASHING.md` for the full design,
//! including how evidence reaches `Staking.slashSeverity`.

use crate::pbft::{pbft_signature_message, verify_pbft_phase_signature};
use anyhow::{bail, ensure, Result};
use common::{sha256_hex, Block, BlockHeader};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Bumped only with a change to what evidence means or how its id is derived.
pub const EVIDENCE_VERSION: u8 = 1;
const EVIDENCE_DOMAIN: &str = "creg-equivocation-v1";
/// Upper bound on remembered first-votes before the lowest heights are evicted.
pub const DEFAULT_MAX_SLOTS: usize = 50_000;
/// Upper bound on retained equivocation proofs to prevent memory DoS.
pub const DEFAULT_MAX_EVIDENCE: usize = 4_096;

/// PBFT phase a vote was cast in. Only these two phases carry a signed vote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VotePhase {
    Prepare,
    Commit,
}

impl VotePhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            VotePhase::Prepare => "prepare",
            VotePhase::Commit => "commit",
        }
    }
}

/// A validator's signed PBFT vote as seen on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedVote {
    pub validator_id: String,
    pub pubkey_hex: String,
    pub phase: VotePhase,
    pub block_hash: String,
    pub signature_hex: String,
}

/// Hash a block header exactly as `Block::hash` does. The view-change
/// certificate is excluded by `Block::hash`, so it never affects this value.
pub fn header_hash(header: &BlockHeader) -> String {
    Block {
        header: header.clone(),
        transactions: Vec::new(),
        pbft_signatures: Vec::new(),
    }
    .hash()
}

fn normalize_pubkey(value: &str) -> String {
    value.trim_start_matches("0x").to_ascii_lowercase()
}

/// Whether two distinct blocks signed by one validator in `phase` are a
/// slashable conflict. See the module documentation for the rules.
fn conflicts(phase: VotePhase, a: &BlockHeader, b: &BlockHeader) -> bool {
    if a.height != b.height {
        return false;
    }
    match phase {
        VotePhase::Prepare => a.prev_hash == b.prev_hash && a.view_number == b.view_number,
        VotePhase::Commit => true,
    }
}

/// Proof that one validator signed two conflicting blocks. Self-contained:
/// anyone can check it with [`EquivocationEvidence::verify`] and no node state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquivocationEvidence {
    pub version: u8,
    /// Deterministic id; identical on every node that observes the same offense.
    pub evidence_id: String,
    pub validator_id: String,
    pub pubkey_hex: String,
    pub phase: VotePhase,
    pub height: u64,
    /// The vote on the block with the lexicographically smaller hash.
    pub vote_a: SignedVote,
    pub header_a: BlockHeader,
    pub vote_b: SignedVote,
    pub header_b: BlockHeader,
}

impl EquivocationEvidence {
    /// Build evidence from two observed (vote, header) pairs, in either order,
    /// and verify it before returning.
    pub fn new(
        first: (SignedVote, BlockHeader),
        second: (SignedVote, BlockHeader),
    ) -> Result<Self> {
        let (a, b) = if first.0.block_hash <= second.0.block_hash {
            (first, second)
        } else {
            (second, first)
        };
        let mut evidence = Self {
            version: EVIDENCE_VERSION,
            evidence_id: String::new(),
            validator_id: a.0.validator_id.clone(),
            pubkey_hex: normalize_pubkey(&a.0.pubkey_hex),
            phase: a.0.phase,
            height: a.1.height,
            vote_a: a.0,
            header_a: a.1,
            vote_b: b.0,
            header_b: b.1,
        };
        evidence.evidence_id = evidence.compute_id();
        evidence.verify()?;
        Ok(evidence)
    }

    fn compute_id(&self) -> String {
        // Both hashes are sorted, and parent and view are only part of a
        // PREPARE slot (a COMMIT slot is the height alone), so two observers
        // always derive the same id for the same offense and a PREPARE offense
        // can never collide with a COMMIT offense.
        let (parent, view) = match self.phase {
            VotePhase::Prepare => (
                self.header_a.prev_hash.clone(),
                self.header_a.view_number.to_string(),
            ),
            VotePhase::Commit => ("any".to_string(), "any".to_string()),
        };
        sha256_hex(
            format!(
                "{EVIDENCE_DOMAIN}|{}|{}|{}|{}|{}|{}|{}",
                self.phase.as_str(),
                self.pubkey_hex,
                self.header_a.height,
                parent,
                view,
                self.vote_a.block_hash,
                self.vote_b.block_hash,
            )
            .as_bytes(),
        )
    }

    /// Check every claim in the evidence from first principles: both
    /// signatures (strictly) under the accused key, each header against the
    /// block hash that was signed, and the slot rules.
    pub fn verify(&self) -> Result<()> {
        ensure!(
            self.version == EVIDENCE_VERSION,
            "unsupported evidence version {}",
            self.version
        );
        ensure!(
            self.vote_a.validator_id == self.vote_b.validator_id
                && self.vote_a.validator_id == self.validator_id,
            "votes name different validators"
        );
        for (vote, header) in [
            (&self.vote_a, &self.header_a),
            (&self.vote_b, &self.header_b),
        ] {
            ensure!(vote.phase == self.phase, "vote phase differs from evidence");
            ensure!(
                normalize_pubkey(&vote.pubkey_hex) == self.pubkey_hex,
                "vote key differs from the accused key"
            );
            ensure!(
                header_hash(header) == vote.block_hash,
                "header does not hash to the signed block hash"
            );
            verify_pbft_phase_signature(
                self.phase.as_str(),
                &vote.block_hash,
                &vote.pubkey_hex,
                &vote.signature_hex,
            )?;
        }
        ensure!(
            self.vote_a.block_hash < self.vote_b.block_hash,
            "votes must be ordered by distinct block hash"
        );
        ensure!(
            self.header_a.height == self.height,
            "evidence height differs from the signed headers"
        );
        ensure!(
            conflicts(self.phase, &self.header_a, &self.header_b),
            "the two blocks do not conflict in one {} slot",
            self.phase.as_str()
        );
        ensure!(
            self.evidence_id == self.compute_id(),
            "evidence id does not match its contents"
        );
        Ok(())
    }

    /// Reason string for `Staking.slashSeverity` / `Staking.slash`. It is
    /// emitted on-chain in the `Slashed` event, tying the penalty to the
    /// evidence that justified it.
    pub fn slash_reason(&self) -> String {
        format!(
            "equivocation:v{}:{}:{}",
            self.version,
            self.phase.as_str(),
            self.evidence_id
        )
    }
}

/// First vote remembered for a slot, keyed by what the journal locks on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Slot {
    pubkey_hex: String,
    phase: VotePhase,
    height: u64,
    /// Parent and view are part of the slot for PREPARE only. The journal
    /// allows one COMMIT per height regardless of either.
    prev_hash: Option<String>,
    view: Option<u32>,
}

impl Slot {
    fn of(vote: &SignedVote, header: &BlockHeader) -> Self {
        let (prev_hash, view) = match vote.phase {
            VotePhase::Prepare => (Some(header.prev_hash.clone()), Some(header.view_number)),
            VotePhase::Commit => (None, None),
        };
        Self {
            pubkey_hex: normalize_pubkey(&vote.pubkey_hex),
            phase: vote.phase,
            height: header.height,
            prev_hash,
            view,
        }
    }
}

/// Watches validated PBFT votes and produces [`EquivocationEvidence`].
///
/// State is bounded: remembered votes are dropped below a finalized height
/// ([`prune_below`](Self::prune_below)) and, as a backstop, the lowest heights
/// are evicted past `max_slots`. Evidence itself is kept.
pub struct EquivocationDetector {
    seen: HashMap<Slot, (SignedVote, BlockHeader)>,
    evidence: BTreeMap<String, EquivocationEvidence>,
    max_slots: usize,
    max_evidence: usize,
}

impl Default for EquivocationDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl EquivocationDetector {
    pub fn new() -> Self {
        Self::with_capacities(DEFAULT_MAX_SLOTS, DEFAULT_MAX_EVIDENCE)
    }

    pub fn with_capacity(max_slots: usize) -> Self {
        Self::with_capacities(max_slots, DEFAULT_MAX_EVIDENCE)
    }

    pub fn with_capacities(max_slots: usize, max_evidence: usize) -> Self {
        Self {
            seen: HashMap::new(),
            evidence: BTreeMap::new(),
            max_slots: max_slots.max(1),
            max_evidence: max_evidence.max(1),
        }
    }

    /// Record a vote together with the header of the block it names.
    ///
    /// The signature is verified here, before anything is stored, so a forged
    /// vote can neither frame a validator nor occupy a slot that a genuine
    /// vote would later conflict with. Returns new evidence when this vote
    /// conflicts with the first one remembered for its slot.
    pub fn record_vote(
        &mut self,
        mut vote: SignedVote,
        header: BlockHeader,
    ) -> Result<Option<EquivocationEvidence>> {
        if header_hash(&header) != vote.block_hash {
            bail!("header does not hash to the voted block hash");
        }
        verify_pbft_phase_signature(
            vote.phase.as_str(),
            &vote.block_hash,
            &vote.pubkey_hex,
            &vote.signature_hex,
        )?;
        vote.pubkey_hex = normalize_pubkey(&vote.pubkey_hex);

        let slot = Slot::of(&vote, &header);
        let Some((first_vote, first_header)) = self.seen.get(&slot) else {
            self.make_room();
            self.seen.insert(slot, (vote, header));
            return Ok(None);
        };
        if first_vote.block_hash == vote.block_hash {
            return Ok(None);
        }

        let evidence =
            EquivocationEvidence::new((first_vote.clone(), first_header.clone()), (vote, header))?;
        if self.evidence.contains_key(&evidence.evidence_id) {
            return Ok(None);
        }
        tracing::warn!(
            validator = %evidence.validator_id,
            phase = evidence.phase.as_str(),
            height = evidence.height,
            evidence_id = %evidence.evidence_id,
            "[SlashingRelay] equivocation detected"
        );
        self.make_room_for_evidence();
        self.evidence
            .insert(evidence.evidence_id.clone(), evidence.clone());
        Ok(Some(evidence))
    }

    /// Accept evidence produced elsewhere (a peer or a persisted copy). It is
    /// fully verified first. Returns whether it was new.
    pub fn import_evidence(&mut self, evidence: EquivocationEvidence) -> Result<bool> {
        evidence.verify()?;
        if self.evidence.contains_key(&evidence.evidence_id) {
            return Ok(false);
        }
        self.make_room_for_evidence();
        Ok(self
            .evidence
            .insert(evidence.evidence_id.clone(), evidence)
            .is_none())
    }

    /// All known evidence, ordered by id.
    pub fn evidence(&self) -> impl Iterator<Item = &EquivocationEvidence> {
        self.evidence.values()
    }

    pub fn get(&self, evidence_id: &str) -> Option<&EquivocationEvidence> {
        self.evidence.get(evidence_id)
    }

    pub fn evidence_count(&self) -> usize {
        self.evidence.len()
    }

    /// Number of votes currently remembered.
    pub fn tracked_votes(&self) -> usize {
        self.seen.len()
    }

    /// Forget remembered votes below `height` (typically a finalized height
    /// minus a safety window). Returns how many were dropped.
    pub fn prune_below(&mut self, height: u64) -> usize {
        let before = self.seen.len();
        self.seen.retain(|slot, _| slot.height >= height);
        before - self.seen.len()
    }

    fn make_room(&mut self) {
        while self.seen.len() >= self.max_slots {
            let Some(lowest) = self.seen.keys().map(|slot| slot.height).min() else {
                break;
            };
            self.seen.retain(|slot, _| slot.height > lowest);
        }
    }

    fn make_room_for_evidence(&mut self) {
        while self.evidence.len() >= self.max_evidence {
            let Some(evict_id) = self
                .evidence
                .iter()
                .min_by(|(id_a, ev_a), (id_b, ev_b)| {
                    ev_a.height.cmp(&ev_b.height).then_with(|| id_a.cmp(id_b))
                })
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            self.evidence.remove(&evict_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    /// A header whose hash differs per `tag`, at the given slot coordinates.
    fn header(height: u64, parent: u8, view: u32, tag: &str) -> BlockHeader {
        let mut header = Block::genesis().header;
        header.height = height;
        header.prev_hash = format!("{parent:02x}").repeat(32);
        header.view_number = view;
        header.proposer_id = "node-1".into();
        header.merkle_root = sha256_hex(tag.as_bytes());
        header
    }

    fn vote(key: &SigningKey, id: &str, phase: VotePhase, header: &BlockHeader) -> SignedVote {
        let block_hash = header_hash(header);
        let message = pbft_signature_message(phase.as_str(), &block_hash);
        SignedVote {
            validator_id: id.into(),
            pubkey_hex: hex::encode(key.verifying_key().as_bytes()),
            phase,
            block_hash,
            signature_hex: hex::encode(key.sign(message.as_bytes()).to_bytes()),
        }
    }

    fn detect(
        detector: &mut EquivocationDetector,
        key: &SigningKey,
        phase: VotePhase,
        header: &BlockHeader,
    ) -> Option<EquivocationEvidence> {
        detector
            .record_vote(vote(key, "node-1", phase, header), header.clone())
            .expect("a genuine vote is accepted")
    }

    #[test]
    fn prepare_double_vote_in_one_view_is_detected_and_verifies() {
        let key = key(7);
        let (a, b) = (header(10, 1, 0, "block-a"), header(10, 1, 0, "block-b"));
        let mut detector = EquivocationDetector::new();

        assert!(detect(&mut detector, &key, VotePhase::Prepare, &a).is_none());
        let evidence = detect(&mut detector, &key, VotePhase::Prepare, &b)
            .expect("two blocks in one PREPARE slot are equivocation");

        evidence.verify().expect("fresh evidence verifies");
        assert_eq!(evidence.height, 10);
        assert_eq!(evidence.phase, VotePhase::Prepare);
        assert_eq!(evidence.validator_id, "node-1");
        assert!(evidence.vote_a.block_hash < evidence.vote_b.block_hash);
        assert_eq!(detector.evidence_count(), 1);
        assert!(detector.get(&evidence.evidence_id).is_some());
    }

    #[test]
    fn repeating_the_same_vote_is_not_equivocation() {
        let key = key(7);
        let a = header(10, 1, 0, "block-a");
        let mut detector = EquivocationDetector::new();
        assert!(detect(&mut detector, &key, VotePhase::Prepare, &a).is_none());
        assert!(detect(&mut detector, &key, VotePhase::Prepare, &a).is_none());
        assert_eq!(detector.evidence_count(), 0);
    }

    #[test]
    fn prepare_votes_in_different_views_are_legitimate() {
        // After a view change a validator may PREPARE a different block.
        let key = key(7);
        let mut detector = EquivocationDetector::new();
        assert!(detect(
            &mut detector,
            &key,
            VotePhase::Prepare,
            &header(10, 1, 0, "a")
        )
        .is_none());
        assert!(detect(
            &mut detector,
            &key,
            VotePhase::Prepare,
            &header(10, 1, 1, "b")
        )
        .is_none());
        assert_eq!(detector.evidence_count(), 0);
    }

    #[test]
    fn commit_votes_in_different_views_at_one_height_are_equivocation() {
        // COMMIT locks span every view at a height.
        let key = key(7);
        let mut detector = EquivocationDetector::new();
        assert!(detect(
            &mut detector,
            &key,
            VotePhase::Commit,
            &header(10, 1, 0, "a")
        )
        .is_none());
        let evidence = detect(
            &mut detector,
            &key,
            VotePhase::Commit,
            &header(10, 1, 2, "b"),
        )
        .expect("conflicting COMMITs across views are equivocation");
        evidence.verify().unwrap();
        assert_eq!(evidence.phase, VotePhase::Commit);
    }

    #[test]
    fn prepare_votes_on_different_parents_or_heights_are_not_evidence() {
        let key = key(7);
        let mut detector = EquivocationDetector::new();
        assert!(detect(
            &mut detector,
            &key,
            VotePhase::Prepare,
            &header(10, 1, 0, "a")
        )
        .is_none());
        assert!(
            detect(
                &mut detector,
                &key,
                VotePhase::Prepare,
                &header(10, 2, 0, "b")
            )
            .is_none(),
            "a different parent is a different PREPARE slot"
        );
        assert!(
            detect(
                &mut detector,
                &key,
                VotePhase::Prepare,
                &header(11, 1, 0, "c")
            )
            .is_none(),
            "a different height is a different slot"
        );
        assert_eq!(detector.evidence_count(), 0);
    }

    #[test]
    fn commit_votes_on_different_parents_at_one_height_are_equivocation() {
        // The journal refuses a second COMMIT at a height even on another
        // fork, so a different parent is no excuse.
        let key = key(7);
        let mut detector = EquivocationDetector::new();
        assert!(detect(
            &mut detector,
            &key,
            VotePhase::Commit,
            &header(10, 1, 0, "a")
        )
        .is_none());
        let evidence = detect(
            &mut detector,
            &key,
            VotePhase::Commit,
            &header(10, 2, 0, "b"),
        )
        .expect("two COMMITs at one height are equivocation on any fork");
        evidence.verify().unwrap();
    }

    #[test]
    fn commit_votes_at_different_heights_are_not_evidence() {
        let key = key(7);
        let mut detector = EquivocationDetector::new();
        assert!(detect(
            &mut detector,
            &key,
            VotePhase::Commit,
            &header(10, 1, 0, "a")
        )
        .is_none());
        assert!(detect(
            &mut detector,
            &key,
            VotePhase::Commit,
            &header(11, 1, 0, "b")
        )
        .is_none());
        assert_eq!(detector.evidence_count(), 0);
    }

    #[test]
    fn different_validators_never_conflict_with_each_other() {
        let mut detector = EquivocationDetector::new();
        let (a, b) = (header(10, 1, 0, "a"), header(10, 1, 0, "b"));
        assert!(detect(&mut detector, &key(7), VotePhase::Prepare, &a).is_none());
        assert!(detect(&mut detector, &key(8), VotePhase::Prepare, &b).is_none());
        assert_eq!(detector.evidence_count(), 0);
    }

    #[test]
    fn evidence_id_ignores_observation_order_and_is_scoped_by_phase() {
        let key = key(7);
        let (a, b) = (header(10, 1, 0, "block-a"), header(10, 1, 0, "block-b"));

        let mut first = EquivocationDetector::new();
        detect(&mut first, &key, VotePhase::Prepare, &a);
        let from_first = detect(&mut first, &key, VotePhase::Prepare, &b).unwrap();

        let mut second = EquivocationDetector::new();
        detect(&mut second, &key, VotePhase::Prepare, &b);
        let from_second = detect(&mut second, &key, VotePhase::Prepare, &a).unwrap();

        assert_eq!(
            from_first.evidence_id, from_second.evidence_id,
            "two observers must derive one id for one offense"
        );

        let mut commits = EquivocationDetector::new();
        detect(&mut commits, &key, VotePhase::Commit, &a);
        let commit = detect(&mut commits, &key, VotePhase::Commit, &b).unwrap();
        assert_ne!(
            from_first.evidence_id, commit.evidence_id,
            "a PREPARE offense and a COMMIT offense are distinct"
        );
    }

    #[test]
    fn forged_votes_are_rejected_and_cannot_poison_a_slot() {
        let (honest, attacker) = (key(7), key(9));
        let (a, b) = (header(10, 1, 0, "a"), header(10, 1, 0, "b"));
        let mut detector = EquivocationDetector::new();

        // The attacker signs with its own key but claims the honest key.
        let mut forged = vote(&attacker, "node-1", VotePhase::Prepare, &a);
        forged.pubkey_hex = hex::encode(honest.verifying_key().as_bytes());
        assert!(detector.record_vote(forged, a.clone()).is_err());
        assert_eq!(detector.tracked_votes(), 0, "a forged vote is not stored");

        // A genuine equivocation by the honest key is still caught afterwards.
        assert!(detect(&mut detector, &honest, VotePhase::Prepare, &a).is_none());
        assert!(detect(&mut detector, &honest, VotePhase::Prepare, &b).is_some());
    }

    #[test]
    fn a_header_that_does_not_match_the_vote_is_rejected() {
        let key = key(7);
        let (a, b) = (header(10, 1, 0, "a"), header(10, 1, 0, "b"));
        let mut detector = EquivocationDetector::new();
        let vote_for_a = vote(&key, "node-1", VotePhase::Prepare, &a);
        assert!(detector.record_vote(vote_for_a, b).is_err());
    }

    #[test]
    fn tampered_evidence_fails_verification() {
        let key = key(7);
        let mut detector = EquivocationDetector::new();
        detect(
            &mut detector,
            &key,
            VotePhase::Prepare,
            &header(10, 1, 0, "a"),
        );
        let valid = detect(
            &mut detector,
            &key,
            VotePhase::Prepare,
            &header(10, 1, 0, "b"),
        )
        .unwrap();
        valid.verify().unwrap();

        type Mutation = fn(&mut EquivocationEvidence);
        let mutations: [(&str, Mutation); 9] = [
            ("version", |e| e.version = 2),
            ("height", |e| e.height += 1),
            ("evidence id", |e| e.evidence_id = "00".repeat(32)),
            ("phase", |e| e.phase = VotePhase::Commit),
            ("swap votes", |e| {
                std::mem::swap(&mut e.vote_a, &mut e.vote_b)
            }),
            ("same block twice", |e| {
                e.vote_b = e.vote_a.clone();
                e.header_b = e.header_a.clone();
            }),
            ("header", |e| e.header_a.merkle_root = "ff".repeat(32)),
            ("signature", |e| {
                e.vote_a.signature_hex = e.vote_b.signature_hex.clone()
            }),
            ("accused key", |e| {
                e.pubkey_hex = hex::encode(
                    SigningKey::from_bytes(&[9u8; 32])
                        .verifying_key()
                        .as_bytes(),
                )
            }),
        ];
        for (name, mutate) in mutations {
            let mut broken = valid.clone();
            mutate(&mut broken);
            assert!(broken.verify().is_err(), "tampering with {name} must fail");
        }
    }

    #[test]
    fn evidence_for_legitimate_votes_cannot_be_constructed() {
        // Even with valid signatures, different views in PREPARE are not a conflict.
        let key = key(7);
        let (a, b) = (header(10, 1, 0, "a"), header(10, 1, 1, "b"));
        let result = EquivocationEvidence::new(
            (vote(&key, "node-1", VotePhase::Prepare, &a), a),
            (vote(&key, "node-1", VotePhase::Prepare, &b), b),
        );
        assert!(result.is_err());
    }

    #[test]
    fn evidence_survives_json_and_still_verifies() {
        let key = key(7);
        let mut detector = EquivocationDetector::new();
        detect(
            &mut detector,
            &key,
            VotePhase::Commit,
            &header(10, 1, 0, "a"),
        );
        let evidence = detect(
            &mut detector,
            &key,
            VotePhase::Commit,
            &header(10, 1, 3, "b"),
        )
        .unwrap();

        let json = serde_json::to_string(&evidence).unwrap();
        let decoded: EquivocationEvidence = serde_json::from_str(&json).unwrap();
        decoded.verify().expect("evidence is portable");
        assert_eq!(decoded.evidence_id, evidence.evidence_id);
    }

    #[test]
    fn imported_evidence_is_verified_and_deduplicated() {
        let key = key(7);
        let mut source = EquivocationDetector::new();
        detect(
            &mut source,
            &key,
            VotePhase::Prepare,
            &header(10, 1, 0, "a"),
        );
        let evidence = detect(
            &mut source,
            &key,
            VotePhase::Prepare,
            &header(10, 1, 0, "b"),
        )
        .unwrap();

        let mut target = EquivocationDetector::new();
        assert!(target.import_evidence(evidence.clone()).unwrap());
        assert!(
            !target.import_evidence(evidence.clone()).unwrap(),
            "second import is a duplicate"
        );

        let mut forged = evidence;
        forged.height += 1;
        assert!(target.import_evidence(forged).is_err());
        assert_eq!(target.evidence_count(), 1);
    }

    #[test]
    fn pruning_bounds_memory_but_keeps_evidence() {
        let key = key(7);
        let mut detector = EquivocationDetector::new();
        for height in 1..=20u64 {
            detect(
                &mut detector,
                &key,
                VotePhase::Prepare,
                &header(height, 1, 0, "a"),
            );
        }
        detect(
            &mut detector,
            &key,
            VotePhase::Prepare,
            &header(5, 1, 0, "b"),
        )
        .unwrap();
        assert_eq!(detector.tracked_votes(), 20);

        assert_eq!(detector.prune_below(11), 10);
        assert_eq!(detector.tracked_votes(), 10);
        assert_eq!(detector.evidence_count(), 1, "evidence outlives pruning");

        // A conflict against a pruned slot can no longer be seen.
        assert!(detect(
            &mut detector,
            &key,
            VotePhase::Prepare,
            &header(6, 1, 0, "c")
        )
        .is_none());
    }

    #[test]
    fn capacity_evicts_the_lowest_heights_first() {
        let key = key(7);
        let mut detector = EquivocationDetector::with_capacity(5);
        for height in 1..=12u64 {
            detect(
                &mut detector,
                &key,
                VotePhase::Prepare,
                &header(height, 1, 0, "a"),
            );
            assert!(detector.tracked_votes() <= 5);
        }
        // The most recent heights are still tracked, so a fresh conflict is caught.
        assert!(detect(
            &mut detector,
            &key,
            VotePhase::Prepare,
            &header(12, 1, 0, "b")
        )
        .is_some());
    }

    #[test]
    fn slash_reason_ties_the_penalty_to_the_evidence() {
        let key = key(7);
        let mut detector = EquivocationDetector::new();
        detect(
            &mut detector,
            &key,
            VotePhase::Commit,
            &header(10, 1, 0, "a"),
        );
        let evidence = detect(
            &mut detector,
            &key,
            VotePhase::Commit,
            &header(10, 1, 1, "b"),
        )
        .unwrap();
        assert_eq!(
            evidence.slash_reason(),
            format!("equivocation:v1:commit:{}", evidence.evidence_id)
        );
    }

    #[test]
    fn evidence_capacity_evicts_lowest_height_proofs() {
        let key = key(7);
        let mut detector = EquivocationDetector::with_capacities(100, 3);
        for height in [50u64, 10, 40, 20, 30] {
            let a = header(height, 1, 0, &format!("a-{height}"));
            let b = header(height, 1, 0, &format!("b-{height}"));
            assert!(detect(&mut detector, &key, VotePhase::Prepare, &a).is_none());
            assert!(detect(&mut detector, &key, VotePhase::Prepare, &b).is_some());
            assert!(detector.evidence_count() <= 3);
        }
        assert_eq!(detector.evidence_count(), 3);
        let retained_heights: Vec<u64> = detector.evidence().map(|e| e.height).collect();
        assert!(!retained_heights.contains(&10));
        assert!(!retained_heights.contains(&20));
        assert!(retained_heights.contains(&50));
    }
}
