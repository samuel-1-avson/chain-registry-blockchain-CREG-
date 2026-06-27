use common::{ValidatorSignature, ValidatorVote};
use std::collections::HashMap;

// Re-export for convenience; definition in common for sharing with node.
pub use common::AnalysisCommittee;

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
    Some(format!(
        "{}:{}",
        common::scanner_profile_digest(&sig.ml_model_version, &sig.analysis_bundles),
        sig.evidence_digest
    ))
}

pub fn aggregate_evidence_votes(
    signatures: &[ValidatorSignature],
    committee: &AnalysisCommittee,   // Hybrid: use committee instead of raw assigned_count
) -> Option<EvidenceVoteOutcome> {
    let m = committee.size;                    // primary mandatory target
    let mandatory_quorum = (2 * m / 3) + 1;
    let effective = if committee.effective_size > 0 { committee.effective_size } else { m };

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

    let mut mandatory_approvals_by_profile: HashMap<String, Vec<ValidatorSignature>> = HashMap::new();
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
                    let approvals = mandatory_approvals_by_profile.entry(profile_key).or_default();
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
    let limit = if committee.effective_size > 0 { committee.effective_size } else { committee.size };
    committee.members.iter().take(limit as usize).any(|id| id == &sig.validator_id)
}

#[cfg(test)]
mod tests {
    use super::{aggregate_evidence_votes, EvidenceVoteOutcome};
    use chrono::Utc;
    use common::{ValidatorSignature, ValidatorVote};

    fn sig(id: &str, vote: ValidatorVote, model_version: &str, is_mand: bool) -> ValidatorSignature {
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
    fn approvals_must_share_the_same_profile_and_evidence_digest() {
        let committee = test_committee(3, &["v1", "v2", "v3"]);
        let mut v1 = sig("v1", ValidatorVote::Approve, "creg-detect-v1.0.0", true);
        let mut v2 = sig("v2", ValidatorVote::Approve, "creg-detect-v1.0.0", true);
        let mut v3 = sig("v3", ValidatorVote::Approve, "creg-detect-v1.0.0", true);
        v1.evidence_digest = common::sha256_hex(b"evidence-a");
        v2.evidence_digest = common::sha256_hex(b"evidence-a");
        v3.evidence_digest = common::sha256_hex(b"evidence-b");

        let outcome = aggregate_evidence_votes(&[v1, v2, v3], &committee);

        assert!(
            outcome.is_none(),
            "mixed evidence bundles must not satisfy a single quorum"
        );
    }
}
