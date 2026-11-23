//! ADR-019 Increment 2, part 3c: how a package round decides once
//! `creg-vote-v4` is active.
//!
//! Before activation only the v3 evidence quorum decides. From activation
//! the committee's signed `VoteV4Envelope`s are aggregated with
//! `consensus::vote_v4::aggregate_vote_v4_quorum` under the signed spec's
//! `vote_v4_quorum` rules, and combined with the v3 outcome (whose
//! signatures the publication record still carries):
//!
//! | v3 | v4 | decision |
//! | --- | --- | --- |
//! | rejected | any | rejected (deterministic floor) |
//! | any | rejected | rejected |
//! | verified | approved | verified |
//! | verified | quarantined | Class C hold (publish + `QuarantineV1`) |
//! | otherwise | | keep waiting |

use common::{
    AnalysisCommittee, CommitteeQuorumCertificateV1, ConsensusParams, ValidatorSignature,
    CLASS_C_QUARANTINE_TTL_SECS,
};
use consensus::vote_v4::{VoteV4Outcome, VoteV4QuorumConfig};
use consensus::EvidenceVoteOutcome;

/// What a round decided.
#[derive(Debug, Clone)]
pub enum RoundDecision {
    /// Publish (`Verified`) or reject, exactly as before activation.
    Final(EvidenceVoteOutcome),
    /// Class C hold: publish with these v3 signatures and quarantine in the
    /// same block, once the committee's quarantine certificate verifies.
    Hold {
        final_sigs: Vec<ValidatorSignature>,
        reason: String,
    },
    /// A hold whose quarantine certificate passed the block-validation
    /// check; ready to finalize as publish + `QuarantineV1`.
    HoldCertified {
        final_sigs: Vec<ValidatorSignature>,
        certificate: Box<CommitteeQuorumCertificateV1>,
    },
}

/// Combine the v3 evidence outcome with the v4 committee outcome.
pub fn combine_outcomes(
    v3: Option<EvidenceVoteOutcome>,
    v4: &VoteV4Outcome,
) -> Option<RoundDecision> {
    match (v3, v4) {
        (Some(EvidenceVoteOutcome::Rejected { reason }), _) => {
            Some(RoundDecision::Final(EvidenceVoteOutcome::Rejected {
                reason,
            }))
        }
        (_, VoteV4Outcome::Rejected { reason, .. }) => {
            Some(RoundDecision::Final(EvidenceVoteOutcome::Rejected {
                reason: reason.clone(),
            }))
        }
        (Some(EvidenceVoteOutcome::Verified(sigs)), VoteV4Outcome::Approved { .. }) => {
            Some(RoundDecision::Final(EvidenceVoteOutcome::Verified(sigs)))
        }
        (
            Some(EvidenceVoteOutcome::Verified(final_sigs)),
            VoteV4Outcome::Quarantined { reason, .. },
        ) => Some(RoundDecision::Hold {
            final_sigs,
            reason: reason.clone(),
        }),
        _ => None,
    }
}

/// Aggregation rules for one committee under the signed spec.
pub fn quorum_config(
    params: &ConsensusParams,
    committee: &AnalysisCommittee,
    allow_small_cluster_quorum: bool,
) -> VoteV4QuorumConfig {
    let rules = params.vote_v4_quorum();
    VoteV4QuorumConfig {
        assigned_count: common::committee::committee_signers(committee).len(),
        allow_small_cluster_quorum,
        min_model_families: usize::from(rules.min_model_families),
        // The chain derives the deadline itself; this only shapes the
        // advisory `expires_at_unix` of the local outcome.
        quarantine_ttl_secs: CLASS_C_QUARANTINE_TTL_SECS,
        require_semantic_vote_for_approval: rules.require_semantic_vote_for_approval,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn verified() -> Option<EvidenceVoteOutcome> {
        Some(EvidenceVoteOutcome::Verified(vec![
            ValidatorSignature::default(),
        ]))
    }

    fn approved() -> VoteV4Outcome {
        VoteV4Outcome::Approved {
            approving_validators: vec!["v1".into()],
            distinct_model_families: BTreeSet::new(),
        }
    }

    fn quarantined() -> VoteV4Outcome {
        VoteV4Outcome::Quarantined {
            reason: "single model family".into(),
            expires_at_unix: 0,
            approvals: 3,
            quarantine_reviews: 0,
            distinct_model_families: BTreeSet::new(),
        }
    }

    fn pending() -> VoteV4Outcome {
        VoteV4Outcome::Pending {
            valid_votes: 1,
            approvals: 1,
            rejections: 0,
            quarantine_reviews: 0,
            abstentions: 0,
            quorum: 3,
        }
    }

    #[test]
    fn rejections_win_and_holds_need_a_v3_quorum() {
        let rejected_v3 = Some(EvidenceVoteOutcome::Rejected {
            reason: "class A".into(),
        });
        assert!(matches!(
            combine_outcomes(rejected_v3, &approved()),
            Some(RoundDecision::Final(EvidenceVoteOutcome::Rejected { .. }))
        ));
        let rejected_v4 = VoteV4Outcome::Rejected {
            reason: "witness".into(),
            witness_digest: None,
            deterministic_floor: true,
        };
        assert!(matches!(
            combine_outcomes(verified(), &rejected_v4),
            Some(RoundDecision::Final(EvidenceVoteOutcome::Rejected { .. }))
        ));
        assert!(matches!(
            combine_outcomes(verified(), &approved()),
            Some(RoundDecision::Final(EvidenceVoteOutcome::Verified(_)))
        ));
        assert!(matches!(
            combine_outcomes(verified(), &quarantined()),
            Some(RoundDecision::Hold { .. })
        ));
        // Undecided on either side keeps the round open.
        assert!(combine_outcomes(None, &approved()).is_none());
        assert!(combine_outcomes(None, &quarantined()).is_none());
        assert!(combine_outcomes(verified(), &pending()).is_none());
    }

    #[test]
    fn quorum_rules_come_from_the_signed_spec() {
        let committee = AnalysisCommittee {
            size: 4,
            effective_size: 4,
            members: (1..=6).map(|i| format!("v{i}")).collect(),
            ..AnalysisCommittee::default()
        };
        let mut params: ConsensusParams = serde_json::from_value(serde_json::json!({
            "block_time_seconds": 5, "vote_timeout_ms": 5000, "quorum_percentage": 67,
            "min_validator_stake_wei": "0", "min_publisher_stake_wei": "0",
            "unbonding_period_seconds": 1, "slash_penalty_low_bp": 1,
            "slash_penalty_medium_bp": 1, "slash_penalty_critical_bp": 1,
            "max_validators": 10
        }))
        .expect("params");
        // Unset: no model-family requirement (v3-equivalent).
        let config = quorum_config(&params, &committee, false);
        assert_eq!(config.assigned_count, 4);
        assert_eq!(config.min_model_families, 0);
        assert!(!config.require_semantic_vote_for_approval);

        params.vote_v4_quorum = Some(common::VoteV4QuorumSpec {
            min_model_families: 2,
            require_semantic_vote_for_approval: true,
        });
        let config = quorum_config(&params, &committee, true);
        assert_eq!(config.min_model_families, 2);
        assert!(config.require_semantic_vote_for_approval && config.allow_small_cluster_quorum);
    }
}
