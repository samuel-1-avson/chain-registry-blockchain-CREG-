use common::crypto::{PublicKey, Signature, SignatureScheme, VerifyMode};
use common::{ValidatorSignature, ValidatorVote};
use std::collections::{HashMap, HashSet};

// Re-export for convenience; definition in common for sharing with node.
pub use common::AnalysisCommittee;

/// Target mandatory committee size for `active_validator_count` active validators.
pub fn expected_target_committee_size(active_validator_count: usize) -> usize {
    if active_validator_count <= 50 {
        active_validator_count
    } else {
        100.min(active_validator_count)
    }
}

#[derive(Debug, Clone)]
pub enum EvidenceVoteOutcome {
    Verified(Vec<ValidatorSignature>),
    Rejected { reason: String },
}

fn consensus_profile_key(sig: &ValidatorSignature) -> Option<String> {
    if !common::is_consensus_grade_vote(
        &sig.ml_model_version,
        &sig.analysis_bundles,
        &sig.evidence_digest,
    ) {
        return None;
    }
    let decision = match &sig.vote {
        ValidatorVote::Approve => "approve",
        ValidatorVote::Reject { .. } => "reject",
    };
    Some(format!(
        "{}:{}:{}:{}:{}",
        common::scanner_profile_digest(&sig.ml_model_version, &sig.analysis_bundles),
        decision,
        sig.evidence_digest,
        sig.deterministic_risk.band,
        sig.deterministic_risk.disposition,
    ))
}

pub fn aggregate_evidence_votes(
    signatures: &[ValidatorSignature],
    committee: &AnalysisCommittee, // Hybrid: use committee instead of raw assigned_count
) -> Option<EvidenceVoteOutcome> {
    let m = committee.size; // primary mandatory target
    let mandatory_quorum = (2 * m / 3) + 1;
    let effective = if committee.effective_size > 0 {
        committee.effective_size
    } else {
        m
    };

    let mut seen_indices = HashMap::new();
    let mut unique_votes = Vec::new();

    for sig in signatures {
        let unique_key = if sig.validator_pubkey.is_empty() {
            format!("id:{}", sig.validator_id.to_ascii_lowercase())
        } else {
            format!("pubkey:{}", sig.validator_pubkey.to_ascii_lowercase())
        };
        if let Some(index) = seen_indices.get(&unique_key).copied() {
            unique_votes[index] = sig.clone();
        } else {
            seen_indices.insert(unique_key, unique_votes.len());
            unique_votes.push(sig.clone());
        }
    }

    let mut mandatory_approvals_by_profile: HashMap<String, Vec<ValidatorSignature>> =
        HashMap::new();
    let mut voluntary_approvals: Vec<ValidatorSignature> = Vec::new();
    let mut rejections = Vec::new();

    for sig in unique_votes {
        let Some(profile_key) = consensus_profile_key(&sig) else {
            continue;
        };

        let is_in_committee = is_in_effective_committee(&sig, committee);

        match &sig.vote {
            ValidatorVote::Approve => {
                if sig.is_mandatory && is_in_committee {
                    let approvals = mandatory_approvals_by_profile
                        .entry(profile_key)
                        .or_default();
                    approvals.push(sig.clone());
                    if approvals.len() >= mandatory_quorum as usize {
                        // Success: return mandatory + any voluntary for the record
                        let mut combined = approvals.clone();
                        combined.extend(voluntary_approvals.iter().cloned());
                        return Some(EvidenceVoteOutcome::Verified(combined));
                    }
                } else if is_in_committee {
                    voluntary_approvals.push(sig.clone());
                }
            }
            ValidatorVote::Reject { reason } => {
                if is_in_committee {
                    rejections.push(reason.clone());
                }
            }
        }
    }

    // Early rejection check using effective size
    let max_possible_approvals = (effective as usize).saturating_sub(rejections.len());
    if max_possible_approvals < mandatory_quorum as usize {
        return Some(EvidenceVoteOutcome::Rejected {
            reason: rejections
                .into_iter()
                .next()
                .unwrap_or_else(|| "Consensus rejected".to_string()),
        });
    }

    None
}

fn is_in_effective_committee(sig: &ValidatorSignature, committee: &AnalysisCommittee) -> bool {
    if committee.members.is_empty() {
        return true; // fallback for small sets / tests
    }
    let limit = if committee.effective_size > 0 {
        committee.effective_size
    } else {
        committee.size
    };
    committee
        .members
        .iter()
        .take(limit as usize)
        .any(|id| id == &sig.validator_id)
}

/// Shared pure committee quorum verifier unifying live `aggregate_evidence_votes`
/// and block-sync/proposal signature verification (VNA-19).
///
/// Validates both cryptographic signatures and deterministic consensus agreement
/// `(scanner_profile_digest, Approve, evidence_digest, band, disposition)` over
/// either an authenticated `AnalysisCommittee` (`2m/3 + 1`, fixing the `n > 150`
/// halt) or the full active `validator_set` for legacy/bootstrap records lacking
/// a recorded committee.
pub fn verify_package_committee_quorum(
    record: &common::ChainRecord,
    publisher_authorization_digest: Option<&str>,
    validator_set: &common::ValidatorSet,
) -> anyhow::Result<()> {
    let active: Vec<&common::Validator> = validator_set
        .validators
        .iter()
        .filter(|v| common::validator_status_is_active(&v.status))
        .collect();
    let n = active.len();
    if n == 0 {
        anyhow::bail!("cannot verify block: local validator set is empty");
    }

    let active_by_pubkey: HashMap<String, &common::Validator> = active
        .iter()
        .copied()
        .map(|v| (v.pubkey.to_ascii_lowercase(), v))
        .collect();
    let active_by_id: HashMap<&str, &common::Validator> =
        active.iter().copied().map(|v| (v.id.as_str(), v)).collect();

    let canonical = record.id.canonical();
    let (eligible_ids, quorum, denominator): (Option<HashSet<String>>, usize, usize) =
        if let Some(committee) = record
            .analysis_committee
            .as_ref()
            .filter(|c| !c.members.is_empty())
        {
            let target_size = expected_target_committee_size(n);
            let m = committee.size as usize;
            if m == 0 || m < target_size || m > committee.members.len() {
                anyhow::bail!(
                    "invalid analysis committee size {m} (expected >= {target_size}, members={})",
                    committee.members.len()
                );
            }
            if !committee.validator_snapshot_certificate_hash.is_empty()
                || !committee.assignment_proof.is_empty()
            {
                if !committee.assignment_proof_is_valid(&canonical, &record.content_hash) {
                    anyhow::bail!("analysis committee assignment proof is invalid");
                }
                if !committee.validator_snapshot_hash.is_empty()
                    && committee.validator_snapshot_hash
                        != common::canonical_validator_set_hash(validator_set)
                {
                    anyhow::bail!(
                        "analysis committee snapshot hash does not match authority validator set"
                    );
                }
            }
            let mut seen_members = HashSet::new();
            for member in &committee.members {
                if !seen_members.insert(member.as_str()) {
                    anyhow::bail!("analysis committee contains duplicate member {member}");
                }
            }
            for primary_member in committee.members.iter().take(m) {
                if !active_by_id.contains_key(primary_member.as_str()) {
                    anyhow::bail!(
                    "analysis committee primary member {primary_member} is not an active validator"
                );
                }
            }
            let limit = committee
                .effective_size
                .max(committee.size)
                .min(committee.members.len() as u32) as usize;
            let allowed: HashSet<String> = committee
                .members
                .iter()
                .take(limit)
                .filter(|id| active_by_id.contains_key(id.as_str()))
                .cloned()
                .collect();
            (Some(allowed), (2 * m / 3) + 1, m)
        } else {
            (None, (2 * n / 3) + 1, n)
        };

    let mut approval_groups: HashMap<String, usize> = HashMap::new();
    let mut seen_pubkeys = HashSet::new();

    for sig in &record.validator_signatures {
        if !matches!(sig.vote, ValidatorVote::Approve) {
            continue;
        }
        let Some(profile_key) = consensus_profile_key(sig) else {
            continue;
        };
        if let Some(allowed) = eligible_ids.as_ref() {
            if !allowed.contains(&sig.validator_id) {
                continue;
            }
        }
        let pubkey_key = sig.validator_pubkey.to_ascii_lowercase();
        if active_by_pubkey
            .get(&pubkey_key)
            .is_none_or(|validator| validator.id != sig.validator_id)
        {
            continue;
        }
        if !seen_pubkeys.insert(pubkey_key) {
            continue;
        }

        let Ok(pubkey_bytes) = hex::decode(&sig.validator_pubkey) else {
            continue;
        };
        let Ok(vk) = PublicKey::from_bytes(SignatureScheme::Ed25519, &pubkey_bytes) else {
            continue;
        };
        let Ok(sig_bytes) = hex::decode(&sig.signature) else {
            continue;
        };
        let Ok(ed_sig) = Signature::from_bytes(SignatureScheme::Ed25519, &sig_bytes) else {
            continue;
        };

        let scanner_profile =
            common::scanner_profile_digest(&sig.ml_model_version, &sig.analysis_bundles);
        let vote_metadata = crate::vote_accumulator::vote_metadata_digest(
            sig.is_mandatory,
            &sig.deterministic_risk,
            None,
        );
        let message = match publisher_authorization_digest {
            Some(digest) => crate::vote_accumulator::canonical_authorized_vote_message(
                &canonical,
                &record.content_hash,
                true,
                &sig.validator_pubkey,
                &scanner_profile,
                &sig.evidence_digest,
                digest,
                &vote_metadata,
            ),
            None => crate::vote_accumulator::canonical_vote_message(
                &canonical,
                &record.content_hash,
                true,
                &sig.validator_pubkey,
                &scanner_profile,
                &sig.evidence_digest,
            ),
        };

        if vk
            .verify(message.as_bytes(), &ed_sig, VerifyMode::Strict)
            .is_ok()
        {
            *approval_groups.entry(profile_key).or_default() += 1;
        }
    }

    let approvals = approval_groups.values().copied().max().unwrap_or(0);
    if approvals < quorum {
        anyhow::bail!(
            "only {} valid signatures, need {} ({}/{} validators)",
            approvals,
            quorum,
            approvals,
            denominator
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{aggregate_evidence_votes, EvidenceVoteOutcome};
    use chrono::Utc;
    use common::{ValidatorSignature, ValidatorVote};

    fn sig(
        id: &str,
        vote: ValidatorVote,
        model_version: &str,
        is_mand: bool,
    ) -> ValidatorSignature {
        let analysis_bundles = common::AnalysisBundleRefs {
            policy_bundle_id: "policy-v1".into(),
            feature_schema_id: "features-v1".into(),
            expert_bundle_id: "experts-v1".into(),
            embedding_model_id: "embeddings-v1".into(),
            index_epoch: "index-epoch-1".into(),
            threshold_profile_id: "thresholds-v1".into(),
            llm_prompt_profile_id: "llm-prompt-v1".into(),
            osv_snapshot_epoch: "osv-off".into(),
        };
        ValidatorSignature {
            validator_id: id.to_string(),
            validator_pubkey: format!("{}-pubkey", id),
            signature: format!("{}-sig", id),
            vote,
            signed_at: Utc::now(),
            ml_model_version: model_version.to_string(),
            analysis_bundles,
            evidence_digest: common::sha256_hex(b"pipeline-test-evidence"),
            deterministic_risk: common::DeterministicRiskSummary::default(),
            is_mandatory: is_mand,
            ..Default::default()
        }
    }

    fn test_committee(size: usize, members: &[&str]) -> super::AnalysisCommittee {
        super::AnalysisCommittee {
            size: size as u32,
            members: members.iter().map(|s| s.to_string()).collect(),
            effective_size: size as u32,
            seed: "test".to_string(),
            performance_bias: true,
            ..Default::default()
        }
    }

    #[test]
    fn aggregates_to_verified_from_approval_quorum() {
        let committee = test_committee(4, &["v1", "v2", "v3", "v4"]);
        let outcome = aggregate_evidence_votes(
            &[
                sig("v1", ValidatorVote::Approve, "creg-detect-v1.0.0", true),
                sig("v2", ValidatorVote::Approve, "creg-detect-v1.0.0", true),
                sig("v3", ValidatorVote::Approve, "creg-detect-v1.0.0", true),
                sig(
                    "v4",
                    ValidatorVote::Reject {
                        reason: "malicious".to_string(),
                    },
                    "creg-detect-v1.0.0",
                    true,
                ),
            ],
            &committee,
        );

        match outcome {
            Some(EvidenceVoteOutcome::Verified(sigs)) => assert_eq!(sigs.len(), 3),
            other => panic!("expected verified quorum, got {:?}", other),
        }
    }

    #[test]
    fn aggregates_to_rejected_when_quorum_is_impossible() {
        let committee = test_committee(3, &["v1", "v2", "v3"]);
        let outcome = aggregate_evidence_votes(
            &[
                sig(
                    "v1",
                    ValidatorVote::Reject {
                        reason: "malicious".to_string(),
                    },
                    "creg-detect-v1.0.0",
                    true,
                ),
                sig(
                    "v2",
                    ValidatorVote::Reject {
                        reason: "backdoor".to_string(),
                    },
                    "creg-detect-v1.0.0",
                    true,
                ),
            ],
            &committee,
        );

        match outcome {
            Some(EvidenceVoteOutcome::Rejected { reason }) => assert_eq!(reason, "malicious"),
            other => panic!("expected rejection quorum failure, got {:?}", other),
        }
    }

    #[test]
    fn duplicate_votes_do_not_count_twice() {
        let committee = test_committee(3, &["v1", "v2"]);
        let outcome = aggregate_evidence_votes(
            &[
                sig("v1", ValidatorVote::Approve, "creg-detect-v1.0.0", true),
                sig("v1", ValidatorVote::Approve, "creg-detect-v1.0.0", true),
                sig("v2", ValidatorVote::Approve, "creg-detect-v1.0.0", true),
            ],
            &committee,
        );

        assert!(
            outcome.is_none(),
            "duplicate validator votes must not satisfy quorum"
        );
    }

    #[test]
    fn degraded_votes_are_excluded_from_quorum() {
        let committee = test_committee(4, &["v1", "v2", "v3", "v4"]);
        let outcome = aggregate_evidence_votes(
            &[
                sig("v1", ValidatorVote::Approve, "degraded-no-model", true),
                sig("v2", ValidatorVote::Approve, "creg-detect-v1.0.0", true),
                sig("v3", ValidatorVote::Approve, "creg-detect-v1.0.0", true),
            ],
            &committee,
        );

        assert!(
            outcome.is_none(),
            "degraded validator votes must not satisfy quorum"
        );
    }

    #[test]
    fn approvals_require_matching_deterministic_evidence_digest() {
        let committee = test_committee(3, &["v1", "v2", "v3"]);
        let mut v1 = sig("v1", ValidatorVote::Approve, "creg-detect-v1.0.0", true);
        let mut v2 = sig("v2", ValidatorVote::Approve, "creg-detect-v1.0.0", true);
        let mut v3 = sig("v3", ValidatorVote::Approve, "creg-detect-v1.0.0", true);
        v1.evidence_digest = common::sha256_hex(b"evidence-a");
        v2.evidence_digest = common::sha256_hex(b"evidence-a");
        v3.evidence_digest = common::sha256_hex(b"evidence-b");

        let mismatched =
            aggregate_evidence_votes(&[v1.clone(), v2.clone(), v3.clone()], &committee);
        assert!(
            mismatched.is_none(),
            "divergent deterministic evidence_digest must prevent quorum"
        );

        v3.evidence_digest = common::sha256_hex(b"evidence-a");
        let matched = aggregate_evidence_votes(&[v1, v2, v3], &committee);
        assert!(
            matches!(matched, Some(EvidenceVoteOutcome::Verified(_))),
            "matching deterministic evidence digests must reach quorum"
        );
    }

    #[test]
    fn committee_quorum_succeeds_above_150_validators_and_rejects_forgery() {
        use ed25519_dalek::{Signer, SigningKey};

        let keys: Vec<SigningKey> = (0..200u16)
            .map(|i| {
                let mut seed = [0u8; 32];
                seed[0..2].copy_from_slice(&i.to_le_bytes());
                seed[2] = 0x42;
                SigningKey::from_bytes(&seed)
            })
            .collect();
        let validator_set = common::ValidatorSet::new(
            keys.iter()
                .enumerate()
                .map(|(i, key)| common::Validator {
                    id: format!("val-{i}"),
                    alias: format!("val-{i}"),
                    pubkey: hex::encode(key.verifying_key().as_bytes()),
                    eth_address: String::new(),
                    stake: 100,
                    reputation: 100,
                    status: "online".into(),
                    performance_score: 500,
                    contribution_count: 0,
                    pq_pubkey: None,
                })
                .collect(),
        );

        let package_id = common::PackageId::new("npm", "large-fleet-pkg", "1.0.0");
        let canonical = package_id.canonical();
        let content_hash = common::sha256_hex(b"large-fleet-payload");
        let bundles = common::AnalysisBundleRefs {
            policy_bundle_id: "policy-v1".into(),
            feature_schema_id: "features-v1".into(),
            expert_bundle_id: "experts-v1".into(),
            embedding_model_id: "embeddings-v1".into(),
            index_epoch: "index-epoch-1".into(),
            threshold_profile_id: "thresholds-v1".into(),
            llm_prompt_profile_id: "llm-prompt-v1".into(),
            osv_snapshot_epoch: "osv-off".into(),
        };
        let model_version = "creg-detect-v1.0.0".to_string();
        let evidence_digest = common::sha256_hex(b"deterministic-evidence");
        let members: Vec<String> = (0..100).map(|i| format!("val-{i}")).collect();

        let mut committee = common::AnalysisCommittee {
            size: 100,
            members: members.clone(),
            seed: "seed".into(),
            effective_size: 100,
            performance_bias: true,
            validator_snapshot_hash: common::canonical_validator_set_hash(&validator_set),
            validator_snapshot_height: 0,
            validator_snapshot_anchor_block_hash: "00".repeat(32),
            validator_snapshot_certificate_hash: String::new(),
            selection_randomness: "rand".into(),
            assignment_proof: String::new(),
            member_performance_at_finalization: Default::default(),
        };
        committee.assignment_proof =
            common::committee_assignment_proof(&canonical, &content_hash, &committee);

        let signatures: Vec<ValidatorSignature> = (0..67)
            .map(|i| {
                let pubkey = hex::encode(keys[i].verifying_key().as_bytes());
                let scanner_profile = common::scanner_profile_digest(&model_version, &bundles);
                let msg = crate::vote_accumulator::canonical_vote_message(
                    &canonical,
                    &content_hash,
                    true,
                    &pubkey,
                    &scanner_profile,
                    &evidence_digest,
                );
                ValidatorSignature {
                    validator_id: format!("val-{i}"),
                    validator_pubkey: pubkey,
                    signature: hex::encode(keys[i].sign(msg.as_bytes()).to_bytes()),
                    vote: ValidatorVote::Approve,
                    signed_at: Utc::now(),
                    ml_model_version: model_version.clone(),
                    analysis_bundles: bundles.clone(),
                    evidence_digest: evidence_digest.clone(),
                    deterministic_risk: common::DeterministicRiskSummary::default(),
                    is_mandatory: true,
                    ..Default::default()
                }
            })
            .collect();

        let record = common::ChainRecord {
            id: package_id,
            content_hash: content_hash.clone(),
            ipfs_cid: "bafytest".into(),
            publisher_pubkey: "pub".into(),
            block_hash: String::new(),
            published_at: Utc::now(),
            validator_signatures: signatures,
            status: common::PackageStatus::Verified,
            analysis_committee_size: 100,
            analysis_effective_size: 100,
            analysis_mandatory_contributed: 67,
            analysis_committee: Some(committee),
            ..Default::default()
        };

        super::verify_package_committee_quorum(&record, None, &validator_set)
            .expect("67/100 committee approvals must succeed when active fleet has 200 nodes");

        let mut shrunk = record.clone();
        if let Some(c) = shrunk.analysis_committee.as_mut() {
            c.size = 10;
            c.effective_size = 10;
        }
        assert!(
            super::verify_package_committee_quorum(&shrunk, None, &validator_set).is_err(),
            "shrunk committee below expected target size must be rejected"
        );

        let mut divergent = record;
        divergent.validator_signatures[0].evidence_digest = common::sha256_hex(b"other-evidence");
        assert!(
            super::verify_package_committee_quorum(&divergent, None, &validator_set).is_err(),
            "divergent deterministic evidence digest must break 67/100 quorum"
        );
    }
}
