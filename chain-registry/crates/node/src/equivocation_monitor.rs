// crates/node/src/equivocation_monitor.rs
//! Equivocation detection inside the node (ADR-016, increment 2).
//!
//! **Detection only.** This module never signs, never changes PBFT behaviour
//! and never triggers a slash. It watches PBFT votes that ingress has already
//! validated, and the block headers they name, and records self-contained
//! evidence when one validator signs two conflicting blocks. Off by default;
//! enable on one node at a time with `CREG_EQUIVOCATION_DETECTION=true`.
//!
//! A signed PBFT vote carries only `(phase, block_hash)`, so proving which slot
//! it belongs to needs the block header. The monitor keeps a bounded cache of
//! proposal headers seen near the local tip, including proposals PBFT later
//! rejects (a Byzantine leader's second proposal is usually refused), and holds
//! a vote for an unseen header briefly until that header arrives.
//!
//! Evidence is written as one JSON file per offense under
//! `<data_dir>/equivocation-evidence/`, deliberately outside the chain
//! database: adding a RocksDB column family would stop an older binary from
//! opening the data directory, which would break a canary rollback.

use common::{BlockHeader, BlockSignature};
use consensus::slashing_relay::{
    header_hash, EquivocationDetector, EquivocationEvidence, SignedVote, VotePhase,
};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Environment switch. Detection runs only when this is `true` or `1`.
pub const ENABLE_ENV: &str = "CREG_EQUIVOCATION_DETECTION";
/// Directory under the node data directory that holds persisted evidence.
pub const EVIDENCE_DIR: &str = "equivocation-evidence";

const MAX_CACHED_HEADERS: usize = 1_024;
const MAX_HEADERS_PER_HEIGHT: usize = 16;
const MAX_PENDING_VOTES: usize = 1_024;
const PENDING_VOTE_TTL: Duration = Duration::from_secs(120);
/// Proposals are tracked only this close to the local tip. The lower bound
/// matches the deepest reorg the node follows; the upper bound allows a
/// proposal for the next height or two while this node catches up.
const WINDOW_BEHIND: u64 = 64;
const WINDOW_AHEAD: u64 = 4;

/// Counters exposed through `/metrics` and the evidence API.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MonitorStats {
    pub votes_observed: u64,
    /// Votes the detector refused (bad signature or a header mismatch).
    pub votes_rejected: u64,
    /// Votes dropped because their header never arrived in time or the
    /// waiting queue was full.
    pub votes_unresolved_dropped: u64,
    /// Proposal headers ignored as outside the tip window or over the
    /// per-height cap.
    pub headers_ignored: u64,
    pub persist_failures: u64,
}

struct PendingVote {
    vote: SignedVote,
    received_at: Instant,
}

pub struct EquivocationMonitor {
    enabled: bool,
    evidence_dir: Option<PathBuf>,
    detector: EquivocationDetector,
    headers: HashMap<String, BlockHeader>,
    header_order: VecDeque<String>,
    pending: VecDeque<PendingVote>,
    stats: MonitorStats,
}

impl EquivocationMonitor {
    /// A monitor that ignores everything. Used when detection is switched off.
    pub fn disabled() -> Self {
        Self::new(false, None)
    }

    pub fn new(enabled: bool, evidence_dir: Option<PathBuf>) -> Self {
        Self {
            enabled,
            evidence_dir,
            detector: EquivocationDetector::new(),
            headers: HashMap::new(),
            header_order: VecDeque::new(),
            pending: VecDeque::new(),
            stats: MonitorStats::default(),
        }
    }

    /// Build from `CREG_EQUIVOCATION_DETECTION` (enabled by default; set to
    /// `false`/`0`/`no`/`off` to disable) and reload saved evidence.
    pub fn from_env(data_dir: &Path) -> Self {
        let enabled = std::env::var(ENABLE_ENV)
            .map(|value| {
                let value = value.trim();
                !matches!(
                    value.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
            })
            .unwrap_or(true);
        if !enabled {
            return Self::disabled();
        }
        let mut monitor = Self::new(true, Some(data_dir.join(EVIDENCE_DIR)));
        let loaded = monitor.load_persisted();
        tracing::info!(
            loaded,
            "Equivocation detection enabled (detection only, no slashing; ADR-016)"
        );
        monitor
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Reload evidence saved by an earlier run. Every file is verified again;
    /// a file that does not verify is skipped and logged, never trusted.
    pub fn load_persisted(&mut self) -> usize {
        let Some(dir) = self.evidence_dir.clone() else {
            return 0;
        };
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return 0,
            Err(error) => {
                tracing::error!(dir = %dir.display(), error = %error, "Cannot read saved equivocation evidence");
                return 0;
            }
        };
        let mut loaded = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let result = std::fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| {
                    serde_json::from_slice::<EquivocationEvidence>(&bytes)
                        .map_err(anyhow::Error::from)
                })
                .and_then(|evidence| self.detector.import_evidence(evidence));
            match result {
                Ok(true) => loaded += 1,
                Ok(false) => {}
                Err(error) => tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "Skipping saved equivocation evidence that does not verify"
                ),
            }
        }
        loaded
    }

    /// Remember a proposal header seen on the wire. Call it before any
    /// validation that might reject the proposal.
    pub fn observe_header(
        &mut self,
        header: &BlockHeader,
        tip_height: u64,
    ) -> Vec<EquivocationEvidence> {
        self.observe_header_at(header, tip_height, Instant::now())
    }

    fn observe_header_at(
        &mut self,
        header: &BlockHeader,
        tip_height: u64,
        now: Instant,
    ) -> Vec<EquivocationEvidence> {
        if !self.enabled {
            return Vec::new();
        }
        self.expire_pending(now);
        let in_window = header.height.saturating_add(WINDOW_BEHIND) >= tip_height
            && header.height <= tip_height.saturating_add(WINDOW_AHEAD);
        if !in_window {
            self.stats.headers_ignored += 1;
            return Vec::new();
        }
        let hash = header_hash(header);
        if !self.cache_header(&hash, header) {
            return Vec::new();
        }

        let mut waiting = Vec::new();
        self.pending.retain(|pending| {
            if pending.vote.block_hash == hash {
                waiting.push(pending.vote.clone());
                false
            } else {
                true
            }
        });
        let mut found = Vec::new();
        for vote in waiting {
            found.extend(self.record(vote, header.clone()));
        }
        found
    }

    /// Record a PBFT vote that ingress has already validated against the
    /// active validator set. The detector re-verifies the signature.
    pub fn observe_vote(
        &mut self,
        phase: VotePhase,
        block_hash: &str,
        signature: &BlockSignature,
    ) -> Vec<EquivocationEvidence> {
        self.observe_vote_at(phase, block_hash, signature, Instant::now())
    }

    fn observe_vote_at(
        &mut self,
        phase: VotePhase,
        block_hash: &str,
        signature: &BlockSignature,
        now: Instant,
    ) -> Vec<EquivocationEvidence> {
        if !self.enabled {
            return Vec::new();
        }
        self.stats.votes_observed += 1;
        self.expire_pending(now);
        let vote = SignedVote {
            validator_id: signature.validator_id.clone(),
            pubkey_hex: signature.pubkey.clone(),
            phase,
            block_hash: block_hash.to_string(),
            signature_hex: signature.signature.clone(),
        };
        if let Some(header) = self.headers.get(block_hash).cloned() {
            return self.record(vote, header);
        }
        if self.pending.len() >= MAX_PENDING_VOTES {
            self.pending.pop_front();
            self.stats.votes_unresolved_dropped += 1;
        }
        self.pending.push_back(PendingVote {
            vote,
            received_at: now,
        });
        Vec::new()
    }

    /// Forget state too far below the finalized height to matter. Evidence is
    /// kept.
    pub fn prune(&mut self, finalized_height: u64) {
        if !self.enabled {
            return;
        }
        let floor = finalized_height.saturating_sub(WINDOW_BEHIND);
        self.detector.prune_below(floor);
        let headers = &mut self.headers;
        self.header_order.retain(|hash| {
            let keep = headers
                .get(hash)
                .is_some_and(|header| header.height >= floor);
            if !keep {
                headers.remove(hash);
            }
            keep
        });
    }

    /// All verified evidence known to this node, ordered by id.
    pub fn evidence(&self) -> Vec<EquivocationEvidence> {
        self.detector.evidence().cloned().collect()
    }

    pub fn get(&self, evidence_id: &str) -> Option<&EquivocationEvidence> {
        self.detector.get(evidence_id)
    }

    pub fn evidence_count(&self) -> usize {
        self.detector.evidence_count()
    }

    pub fn tracked_votes(&self) -> usize {
        self.detector.tracked_votes()
    }

    pub fn pending_votes(&self) -> usize {
        self.pending.len()
    }

    pub fn cached_headers(&self) -> usize {
        self.headers.len()
    }

    pub fn stats(&self) -> &MonitorStats {
        &self.stats
    }

    fn cache_header(&mut self, hash: &str, header: &BlockHeader) -> bool {
        if self.headers.contains_key(hash) {
            return false;
        }
        let same_height = self
            .headers
            .values()
            .filter(|cached| cached.height == header.height)
            .count();
        if same_height >= MAX_HEADERS_PER_HEIGHT {
            self.stats.headers_ignored += 1;
            return false;
        }
        while self.headers.len() >= MAX_CACHED_HEADERS {
            let Some(oldest) = self.header_order.pop_front() else {
                break;
            };
            self.headers.remove(&oldest);
        }
        self.header_order.push_back(hash.to_string());
        self.headers.insert(hash.to_string(), header.clone());
        true
    }

    fn expire_pending(&mut self, now: Instant) {
        let before = self.pending.len();
        self.pending.retain(|pending| {
            now.saturating_duration_since(pending.received_at) < PENDING_VOTE_TTL
        });
        self.stats.votes_unresolved_dropped += (before - self.pending.len()) as u64;
    }

    fn record(&mut self, vote: SignedVote, header: BlockHeader) -> Vec<EquivocationEvidence> {
        match self.detector.record_vote(vote, header) {
            Ok(Some(evidence)) => {
                self.persist(&evidence);
                vec![evidence]
            }
            Ok(None) => Vec::new(),
            Err(error) => {
                self.stats.votes_rejected += 1;
                tracing::debug!(error = %error, "Equivocation monitor rejected a vote");
                Vec::new()
            }
        }
    }

    fn persist(&mut self, evidence: &EquivocationEvidence) {
        let Some(dir) = &self.evidence_dir else {
            return;
        };
        let result = (|| -> anyhow::Result<()> {
            use std::io::Write;
            std::fs::create_dir_all(dir)?;
            let path = dir.join(format!("{}.json", evidence.evidence_id));
            let temp = dir.join(format!("{}.json.tmp", evidence.evidence_id));
            let encoded = serde_json::to_vec_pretty(evidence)?;
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(&encoded)?;
            file.sync_all()?;
            drop(file);
            if let Err(error) = std::fs::rename(&temp, &path) {
                let _ = std::fs::remove_file(&temp);
                return Err(error.into());
            }
            #[cfg(unix)]
            std::fs::File::open(dir)?.sync_all()?;
            Ok(())
        })();
        if let Err(error) = result {
            self.stats.persist_failures += 1;
            tracing::error!(
                evidence_id = %evidence.evidence_id,
                error = %error,
                "Could not save equivocation evidence; it is kept in memory only"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{sha256_hex, Block};
    use ed25519_dalek::{Signer, SigningKey};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn header(height: u64, view: u32, tag: &str) -> BlockHeader {
        let mut header = Block::genesis().header;
        header.height = height;
        header.prev_hash = "11".repeat(32);
        header.view_number = view;
        header.merkle_root = sha256_hex(tag.as_bytes());
        header
    }

    fn signed(
        key: &SigningKey,
        phase: VotePhase,
        header: &BlockHeader,
    ) -> (String, BlockSignature) {
        let block_hash = header_hash(header);
        let message = consensus::pbft::pbft_signature_message(phase.as_str(), &block_hash);
        (
            block_hash,
            BlockSignature {
                validator_id: "validator-1".into(),
                pubkey: hex::encode(key.verifying_key().as_bytes()),
                signature: hex::encode(key.sign(message.as_bytes()).to_bytes()),
            },
        )
    }

    fn enabled_in(dir: &Path) -> EquivocationMonitor {
        EquivocationMonitor::new(true, Some(dir.join(EVIDENCE_DIR)))
    }

    #[test]
    fn a_disabled_monitor_ignores_everything() {
        let mut monitor = EquivocationMonitor::disabled();
        let (a, b) = (header(10, 0, "a"), header(10, 0, "b"));
        let key = key(7);
        assert!(monitor.observe_header(&a, 9).is_empty());
        assert!(monitor.observe_header(&b, 9).is_empty());
        let (hash_a, sig_a) = signed(&key, VotePhase::Prepare, &a);
        let (hash_b, sig_b) = signed(&key, VotePhase::Prepare, &b);
        assert!(monitor
            .observe_vote(VotePhase::Prepare, &hash_a, &sig_a)
            .is_empty());
        assert!(monitor
            .observe_vote(VotePhase::Prepare, &hash_b, &sig_b)
            .is_empty());
        assert_eq!(monitor.cached_headers(), 0);
        assert_eq!(monitor.stats().votes_observed, 0);
    }

    #[test]
    fn conflicting_votes_become_saved_evidence_that_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut monitor = enabled_in(dir.path());
        let key = key(7);
        let (a, b) = (header(10, 0, "a"), header(10, 0, "b"));
        monitor.observe_header(&a, 9);
        monitor.observe_header(&b, 9);

        let (hash_a, sig_a) = signed(&key, VotePhase::Prepare, &a);
        let (hash_b, sig_b) = signed(&key, VotePhase::Prepare, &b);
        assert!(monitor
            .observe_vote(VotePhase::Prepare, &hash_a, &sig_a)
            .is_empty());
        let found = monitor.observe_vote(VotePhase::Prepare, &hash_b, &sig_b);
        assert_eq!(found.len(), 1);
        let evidence = &found[0];
        evidence.verify().unwrap();

        let saved = dir
            .path()
            .join(EVIDENCE_DIR)
            .join(format!("{}.json", evidence.evidence_id));
        assert!(saved.exists(), "evidence is written to disk");

        // A restarted node reloads and re-verifies it.
        let mut restarted = enabled_in(dir.path());
        assert_eq!(restarted.load_persisted(), 1);
        assert!(restarted.get(&evidence.evidence_id).is_some());
    }

    #[test]
    fn a_vote_that_arrives_before_its_header_is_resolved_later() {
        let mut monitor = EquivocationMonitor::new(true, None);
        let key = key(7);
        let (a, b) = (header(10, 0, "a"), header(10, 0, "b"));
        monitor.observe_header(&a, 9);
        let (hash_a, sig_a) = signed(&key, VotePhase::Commit, &a);
        let (hash_b, sig_b) = signed(&key, VotePhase::Commit, &b);
        monitor.observe_vote(VotePhase::Commit, &hash_a, &sig_a);

        // The conflicting header has not been seen yet: the vote waits.
        assert!(monitor
            .observe_vote(VotePhase::Commit, &hash_b, &sig_b)
            .is_empty());
        assert_eq!(monitor.pending_votes(), 1);

        let found = monitor.observe_header(&b, 9);
        assert_eq!(found.len(), 1, "the waiting vote resolves into evidence");
        assert_eq!(monitor.pending_votes(), 0);
    }

    #[test]
    fn waiting_votes_expire() {
        let mut monitor = EquivocationMonitor::new(true, None);
        let key = key(7);
        let a = header(10, 0, "a");
        let (hash_a, sig_a) = signed(&key, VotePhase::Prepare, &a);
        let start = Instant::now();
        monitor.observe_vote_at(VotePhase::Prepare, &hash_a, &sig_a, start);
        assert_eq!(monitor.pending_votes(), 1);

        let later = start + PENDING_VOTE_TTL + Duration::from_secs(1);
        monitor.observe_header_at(&header(11, 0, "other"), 10, later);
        assert_eq!(monitor.pending_votes(), 0);
        assert_eq!(monitor.stats().votes_unresolved_dropped, 1);
    }

    #[test]
    fn headers_far_from_the_tip_are_ignored() {
        let mut monitor = EquivocationMonitor::new(true, None);
        monitor.observe_header(&header(1_000, 0, "ahead"), 10);
        monitor.observe_header(&header(1, 0, "behind"), 1_000);
        assert_eq!(monitor.cached_headers(), 0);
        assert_eq!(monitor.stats().headers_ignored, 2);

        monitor.observe_header(&header(11, 0, "next"), 10);
        assert_eq!(monitor.cached_headers(), 1);
    }

    #[test]
    fn one_height_cannot_flood_the_header_cache() {
        let mut monitor = EquivocationMonitor::new(true, None);
        for index in 0..(MAX_HEADERS_PER_HEIGHT + 10) {
            monitor.observe_header(&header(11, 0, &format!("spam-{index}")), 10);
        }
        assert_eq!(monitor.cached_headers(), MAX_HEADERS_PER_HEIGHT);
        // A proposal at another height is still accepted.
        monitor.observe_header(&header(12, 0, "real"), 10);
        assert_eq!(monitor.cached_headers(), MAX_HEADERS_PER_HEIGHT + 1);
    }

    #[test]
    fn forged_votes_are_counted_and_ignored() {
        let mut monitor = EquivocationMonitor::new(true, None);
        let a = header(10, 0, "a");
        monitor.observe_header(&a, 9);
        let (hash_a, mut forged) = signed(&key(9), VotePhase::Prepare, &a);
        forged.pubkey = hex::encode(key(7).verifying_key().as_bytes());
        assert!(monitor
            .observe_vote(VotePhase::Prepare, &hash_a, &forged)
            .is_empty());
        assert_eq!(monitor.stats().votes_rejected, 1);
        assert_eq!(monitor.tracked_votes(), 0);
    }

    #[test]
    fn pruning_drops_old_headers_but_keeps_evidence() {
        let mut monitor = EquivocationMonitor::new(true, None);
        let key = key(7);
        let (a, b) = (header(10, 0, "a"), header(10, 0, "b"));
        monitor.observe_header(&a, 9);
        monitor.observe_header(&b, 9);
        let (hash_a, sig_a) = signed(&key, VotePhase::Prepare, &a);
        let (hash_b, sig_b) = signed(&key, VotePhase::Prepare, &b);
        monitor.observe_vote(VotePhase::Prepare, &hash_a, &sig_a);
        monitor.observe_vote(VotePhase::Prepare, &hash_b, &sig_b);
        assert_eq!(monitor.evidence_count(), 1);

        monitor.prune(10 + WINDOW_BEHIND + 1);
        assert_eq!(monitor.cached_headers(), 0);
        assert_eq!(monitor.tracked_votes(), 0);
        assert_eq!(monitor.evidence_count(), 1, "evidence outlives pruning");
    }

    #[test]
    fn tampered_saved_evidence_is_not_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let mut monitor = enabled_in(dir.path());
        let key = key(7);
        let (a, b) = (header(10, 0, "a"), header(10, 0, "b"));
        monitor.observe_header(&a, 9);
        monitor.observe_header(&b, 9);
        let (hash_a, sig_a) = signed(&key, VotePhase::Prepare, &a);
        let (hash_b, sig_b) = signed(&key, VotePhase::Prepare, &b);
        monitor.observe_vote(VotePhase::Prepare, &hash_a, &sig_a);
        let evidence = monitor
            .observe_vote(VotePhase::Prepare, &hash_b, &sig_b)
            .pop()
            .unwrap();

        let path = dir
            .path()
            .join(EVIDENCE_DIR)
            .join(format!("{}.json", evidence.evidence_id));
        let mut tampered = evidence.clone();
        tampered.height += 1;
        std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
        std::fs::write(path.with_extension("txt"), b"not evidence").unwrap();

        let mut restarted = enabled_in(dir.path());
        assert_eq!(restarted.load_persisted(), 0);
        assert_eq!(restarted.evidence_count(), 0);
    }
}
