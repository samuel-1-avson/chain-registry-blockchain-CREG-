//! `creg-vote-v4` & `ADR-017` Hybrid Post-Quantum Convergence Integration Suite
//! (`ADR-018` Increment 6).
//!
//! Uses ONLY `common` and `consensus` crates so deterministic consensus verification
//! never depends on `validator` internals or LLM runtimes.

use common::crypto::SigningKey;
use common::{
    merkle_root, AnalysisBundleRefs, BlockSignature, ConsensusParams, DeterministicRiskSummary,
    PackageStatus, PqKeyBindingV1, PreparedBlockHeader, PreparedCertificate, SemanticDispositionV4,
    SemanticVoteV4, TriggerSpecWireV1, TriggerWitnessV1, Validator, ViewChangeVote, VoteV4Envelope,
    CLASS_C_QUARANTINE_TTL_SECS,
};
use consensus::vote_v4::{
    aggregate_vote_v4_quorum, deduplicate_view_change_certificate, resolve_quarantine_timeout,
    sign_pbft_phase_at_height, verify_pbft_phase_signature_at_height, VoteV4Outcome,
    VoteV4QuorumConfig, WitnessPrecheckDecision, WitnessReplayGuard,
};
use std::collections::{BTreeMap, HashSet};

fn test_params(activation_height: Option<u64>) -> ConsensusParams {
    ConsensusParams {
        consensus_encoding: Some("phase-c-v1".into()),
        block_time_seconds: 5,
        vote_timeout_ms: 5_000,
        quorum_percentage: 67,
        min_validator_stake_wei: "0".into(),
        min_publisher_stake_wei: "0".into(),
        unbonding_period_seconds: 604_800,
        slash_penalty_low_bp: 500,
        slash_penalty_medium_bp: 2500,
        slash_penalty_critical_bp: 10000,
        max_validators: 50,
        namespace_authority_activation_height: None,
        namespace_authority_policies: vec![],
        hybrid_signature_activation_height: activation_height,
    }
}

fn validator_fixture(id: &str, seed_byte: u8) -> (Validator, SigningKey, SigningKey) {
    let ed_seed = [seed_byte; 32];
    let pq_seed = [seed_byte.wrapping_add(80); 32];
    let ed_sk = SigningKey::from_ed25519_seed(&ed_seed);
    let hyb_sk = SigningKey::hybrid_from_seeds(&ed_seed, &pq_seed);
    let (_, pq_pk) = hyb_sk.public_key().hybrid_parts().unwrap();
    let val = Validator {
        id: id.into(),
        alias: id.into(),
        pubkey: ed_sk.public_key().to_hex(),
        pq_pubkey: Some(hex::encode(pq_pk)),
        eth_address: format!("0x{:040x}", seed_byte),
        stake: 1_000,
        reputation: 100,
        status: "active".into(),
        ..Default::default()
    };
    (val, ed_sk, hyb_sk)
}

#[allow(clippy::too_many_arguments)]
fn signed_vote(
    id: &str,
    sk: &SigningKey,
    pkg: &str,
    content_hash: &str,
    approved: bool,
    reject_reason: Option<&str>,
    semantic_vote: Option<SemanticVoteV4>,
    witness: Option<TriggerWitnessV1>,
    ml_model_version: &str,
) -> VoteV4Envelope {
    let mut vote = VoteV4Envelope {
        package_canonical: pkg.into(),
        content_hash: content_hash.into(),
        approved,
        reject_reason: reject_reason.map(|s| s.to_string()),
        validator_id: id.into(),
        voter_key_id: sk.key_id(),
        scanner_profile_digest: "bb".repeat(32),
        evidence_digest: "cc".repeat(32),
        publisher_authorization_digest: "dd".repeat(32),
        vote_metadata_digest: "ee".repeat(32),
        ml_model_version: ml_model_version.into(),
        analysis_bundles: AnalysisBundleRefs::default(),
        deterministic_risk: DeterministicRiskSummary::default(),
        is_mandatory: true,
        witness,
        semantic_vote,
        signature: String::new(),
        pq_signature: None,
    };
    vote.sign_with(sk).expect("sign VoteV4Envelope");
    vote
}

fn timebomb_witness(prover_id: &str, sk: &SigningKey, content_hash: &str) -> TriggerWitnessV1 {
    let mut env = BTreeMap::new();
    env.insert("CI".into(), "true".into());
    let mut w = TriggerWitnessV1 {
        schema_version: 1,
        package_canonical: "npm:adv-ci-timebomb-canary@1.0.0".into(),
        content_hash: content_hash.into(),
        manifest_sha256: "11".repeat(32),
        sandbox_policy_bundle_id: "creg-policy-v1".into(),
        trigger_spec: TriggerSpecWireV1 {
            schema_version: 1,
            env_overrides: env,
            clock_offset_secs: 86_400 * 90,
            entry_function: None,
            args: Vec::new(),
        },
        observed_violations: vec!["KB-CANARY-EXFIL".into(), "KB-NET-UNDECLARED".into()],
        graph_canonical_digest: "22".repeat(32),
        witness_prover_id: prover_id.into(),
        prover_key_id: sk.key_id(),
        prover_signature: String::new(),
        prover_pq_signature: None,
    };
    w.sign_with(sk).expect("sign TriggerWitnessV1");
    w
}

#[test]
fn mixed_4_validator_cluster_converges_on_trigger_witness() {
    let params = test_params(Some(50));
    let height = 50;
    let pkg = "npm:adv-ci-timebomb-canary@1.0.0";
    let content_hash = "aa".repeat(32);

    let (v1, _, sk1) = validator_fixture("val-1", 1);
    let (v2, _, sk2) = validator_fixture("val-2", 2);
    let (v3, _, sk3) = validator_fixture("val-3", 3);
    let (v4, _, sk4) = validator_fixture("val-4", 4);
    let validators = vec![v1.clone(), v2.clone(), v3.clone(), v4.clone()];

    // Three oblivious validators vote Approve across distinct families.
    let vote1 = signed_vote(
        &v1.id,
        &sk1,
        pkg,
        &content_hash,
        true,
        None,
        Some(SemanticVoteV4 {
            disposition: SemanticDispositionV4::Approve,
            model_family: "gemini".into(),
            finding_digests: Vec::new(),
        }),
        None,
        "creg-static-v1",
    );
    let vote2 = signed_vote(
        &v2.id,
        &sk2,
        pkg,
        &content_hash,
        true,
        None,
        Some(SemanticVoteV4 {
            disposition: SemanticDispositionV4::Approve,
            model_family: "claude".into(),
            finding_digests: Vec::new(),
        }),
        None,
        "creg-static-v1",
    );
    let vote3 = signed_vote(
        &v3.id,
        &sk3,
        pkg,
        &content_hash,
        true,
        None,
        Some(SemanticVoteV4 {
            disposition: SemanticDispositionV4::Approve,
            model_family: "llama".into(),
            finding_digests: Vec::new(),
        }),
        None,
        "creg-static-v1",
    );

    // 4th validator discovers the CI +90d timebomb canary and produces a proof-carrying witness.
    let witness = timebomb_witness(&v4.id, &sk4, &content_hash);
    let vote4 = signed_vote(
        &v4.id,
        &sk4,
        pkg,
        &content_hash,
        false,
        Some("KB-CANARY-EXFIL"),
        Some(SemanticVoteV4 {
            disposition: SemanticDispositionV4::Reject,
            model_family: "qwen".into(),
            finding_digests: vec!["33".repeat(32)],
        }),
        Some(witness.clone()),
        "creg-static-v1",
    );

    // Peer validators run pre-replay cryptographic check & deterministic replay without LLMs.
    let mut guard = WitnessReplayGuard::new();
    let precheck = guard
        .preflight_and_reserve(
            &witness,
            pkg,
            &content_hash,
            Some("creg-policy-v1"),
            &validators,
            height,
            &params,
        )
        .expect("valid witness preflight");
    let witness_digest = match precheck {
        WitnessPrecheckDecision::RunReplay { witness_digest } => witness_digest,
        other => panic!("expected RunReplay decision, got {other:?}"),
    };
    guard.record_replay_outcome(&witness, true);
    let mut verified_digests = HashSet::new();
    verified_digests.insert(witness_digest.clone());

    // Cluster MUST converge to Rejected overriding the 3 oblivious approvals.
    let outcome = aggregate_vote_v4_quorum(
        &[vote1, vote2, vote3, vote4],
        pkg,
        &content_hash,
        &validators,
        height,
        &params,
        &VoteV4QuorumConfig::default(),
        &verified_digests,
        1_700_000_000,
    )
    .expect("aggregate quorum");

    match outcome {
        VoteV4Outcome::Rejected {
            witness_digest: Some(digest),
            deterministic_floor: true,
            ..
        } => {
            assert_eq!(digest, witness_digest);
        }
        other => panic!("expected deterministic witness rejection, got {other:?}"),
    }
}

#[test]
fn height_boundary_h_minus_1_and_h_non_separable() {
    let params = test_params(Some(100));
    let (val, ed_sk, hyb_sk) = validator_fixture("v-boundary", 9);
    let hash = "f0".repeat(32);

    // 1. PBFT BlockSignature boundary.
    let pre_pbft = sign_pbft_phase_at_height("prepare", &hash, &val.id, 99, &params, &ed_sk)
        .expect("pre-activation sign");
    let post_pbft = sign_pbft_phase_at_height("prepare", &hash, &val.id, 100, &params, &hyb_sk)
        .expect("post-activation sign");

    assert!(
        verify_pbft_phase_signature_at_height("prepare", &hash, 99, &params, &val, &pre_pbft)
            .is_ok()
    );
    assert!(
        verify_pbft_phase_signature_at_height("prepare", &hash, 100, &params, &val, &pre_pbft)
            .is_err()
    );

    assert!(verify_pbft_phase_signature_at_height(
        "prepare", &hash, 100, &params, &val, &post_pbft
    )
    .is_ok());
    assert!(
        verify_pbft_phase_signature_at_height("prepare", &hash, 99, &params, &val, &post_pbft)
            .is_err()
    );

    let mut stripped_pbft = post_pbft.clone();
    stripped_pbft.pq_signature = None;
    assert!(verify_pbft_phase_signature_at_height(
        "prepare",
        &hash,
        100,
        &params,
        &val,
        &stripped_pbft
    )
    .is_err());
    assert!(verify_pbft_phase_signature_at_height(
        "prepare",
        &hash,
        99,
        &params,
        &val,
        &stripped_pbft
    )
    .is_err());

    // 2. VoteV4Envelope + PqKeyBindingV1 boundary.
    let pre_vote = signed_vote(
        &val.id,
        &ed_sk,
        "npm:x@1.0.0",
        &hash,
        true,
        None,
        None,
        None,
        "creg-static-v1",
    );
    let post_vote = signed_vote(
        &val.id,
        &hyb_sk,
        "npm:x@1.0.0",
        &hash,
        true,
        None,
        None,
        None,
        "creg-static-v1",
    );
    assert!(pre_vote.verify(&val, params.vote_v4_active_at(99)).is_ok());
    assert!(pre_vote
        .verify(&val, params.vote_v4_active_at(100))
        .is_err());
    assert!(post_vote
        .verify(&val, params.vote_v4_active_at(100))
        .is_ok());
    assert!(post_vote
        .verify(&val, params.vote_v4_active_at(99))
        .is_err());

    let mut stripped_vote = post_vote.clone();
    stripped_vote.pq_signature = None;
    assert!(stripped_vote.verify(&val, true).is_err());
    assert!(stripped_vote.verify(&val, false).is_err());

    let mut binding = PqKeyBindingV1::sign_with(&val.id, "creg-testnet-1", &hyb_sk).unwrap();
    assert!(binding.verify().is_ok());
    binding.pq_pubkey = "00".repeat(1312);
    assert!(binding.verify().is_err());
}

#[test]
fn fifty_validator_view_change_dedup_stays_under_4mib_and_reconstructs() {
    let params = test_params(Some(1));
    let height = 250u64;
    let prev_hash = "aa".repeat(32);
    let block_hash = "bb".repeat(32);

    let header = PreparedBlockHeader {
        height,
        prev_hash: prev_hash.clone(),
        merkle_root: merkle_root(&[]),
        proposer_id: "val-00".into(),
        timestamp: chrono::Utc::now(),
        validator_set_hash: "cc".repeat(32),
        vrf_output: None,
        vrf_proof: None,
        view_number: 0,
        status_root: None,
    };

    let mut sigs: Vec<BlockSignature> = Vec::with_capacity(34);
    for i in 0..34u8 {
        let (val, _, hyb_sk) = validator_fixture(&format!("val-{i:02}"), i + 10);
        let sig =
            sign_pbft_phase_at_height("prepare", &block_hash, &val.id, height, &params, &hyb_sk)
                .expect("sign hybrid prepare");
        sigs.push(sig);
    }

    let prepared = PreparedCertificate {
        header,
        prepared_view: 0,
        original_view_change_certificate: Vec::new(),
        reproposal_proof: None,
        transactions: Vec::new(),
        prepare_signatures: sigs,
    };

    // 34 out of 50 quorum view-change votes, each holding the prepared certificate.
    let mut votes: Vec<ViewChangeVote> = Vec::with_capacity(34);
    for i in 0..34u8 {
        votes.push(ViewChangeVote {
            height,
            prev_hash: prev_hash.clone(),
            new_view: 1,
            validator_id: format!("val-{i:02}"),
            prepared: Some(prepared.clone()),
            signature: "11".repeat(64),
        });
    }

    let naive_bytes = serde_json::to_vec(&votes).unwrap().len();
    assert!(
        naive_bytes > 4 * 1024 * 1024,
        "expected naive 34-quorum hybrid view-change JSON to exceed 4 MiB, got {naive_bytes}"
    );

    let dedup = deduplicate_view_change_certificate(&votes).unwrap();
    let dedup_bytes = dedup.wire_bytes().unwrap();
    assert!(
        dedup_bytes < 4 * 1024 * 1024,
        "deduplicated view-change cert ({dedup_bytes} bytes) must stay below 4 MiB"
    );

    let reconstructed = dedup.reconstruct().unwrap();
    assert_eq!(
        serde_json::to_vec(&reconstructed).unwrap(),
        serde_json::to_vec(&votes).unwrap()
    );
}

#[test]
fn class_c_quantized_diversity_abstention_and_72h_timeout() {
    let params = test_params(Some(10));
    let height = 20;
    let pkg = "npm:class-c-candidate@1.0.0";
    let hash = "dd".repeat(32);

    let (v1, _, sk1) = validator_fixture("c1", 31);
    let (v2, _, sk2) = validator_fixture("c2", 32);
    let (v3, _, sk3) = validator_fixture("c3", 33);
    let (v4, _, sk4) = validator_fixture("c4", 34);
    let validators = vec![v1.clone(), v2.clone(), v3.clone(), v4.clone()];
    let empty_witnesses = HashSet::new();
    let cfg = VoteV4QuorumConfig::default();

    // (a) 3 diverse Approve + 1 abstention (QuarantineReview) -> Approved.
    let diverse_with_abstain = vec![
        signed_vote(
            &v1.id,
            &sk1,
            pkg,
            &hash,
            true,
            None,
            Some(SemanticVoteV4 {
                disposition: SemanticDispositionV4::Approve,
                model_family: "gemini".into(),
                finding_digests: Vec::new(),
            }),
            None,
            "creg-static-v1",
        ),
        signed_vote(
            &v2.id,
            &sk2,
            pkg,
            &hash,
            true,
            None,
            Some(SemanticVoteV4 {
                disposition: SemanticDispositionV4::Approve,
                model_family: "claude".into(),
                finding_digests: Vec::new(),
            }),
            None,
            "creg-static-v1",
        ),
        signed_vote(
            &v3.id,
            &sk3,
            pkg,
            &hash,
            true,
            None,
            Some(SemanticVoteV4 {
                disposition: SemanticDispositionV4::Approve,
                model_family: "llama".into(),
                finding_digests: Vec::new(),
            }),
            None,
            "creg-static-v1",
        ),
        signed_vote(
            &v4.id,
            &sk4,
            pkg,
            &hash,
            false,
            None,
            Some(SemanticVoteV4 {
                disposition: SemanticDispositionV4::QuarantineReview,
                model_family: "qwen".into(),
                finding_digests: Vec::new(),
            }),
            None,
            "creg-static-v1",
        ),
    ];
    match aggregate_vote_v4_quorum(
        &diverse_with_abstain,
        pkg,
        &hash,
        &validators,
        height,
        &params,
        &cfg,
        &empty_witnesses,
        10_000,
    )
    .unwrap()
    {
        VoteV4Outcome::Approved {
            distinct_model_families,
            ..
        } => {
            assert_eq!(distinct_model_families.len(), 3);
        }
        other => panic!("expected Approved across 3 families + 1 abstention, got {other:?}"),
    }

    // (b) 3 single-family Approve ("openai" only) -> Quarantined.
    let single_family = vec![
        signed_vote(
            &v1.id,
            &sk1,
            pkg,
            &hash,
            true,
            None,
            Some(SemanticVoteV4 {
                disposition: SemanticDispositionV4::Approve,
                model_family: "openai".into(),
                finding_digests: Vec::new(),
            }),
            None,
            "creg-static-v1",
        ),
        signed_vote(
            &v2.id,
            &sk2,
            pkg,
            &hash,
            true,
            None,
            Some(SemanticVoteV4 {
                disposition: SemanticDispositionV4::Approve,
                model_family: "openai".into(),
                finding_digests: Vec::new(),
            }),
            None,
            "creg-static-v1",
        ),
        signed_vote(
            &v3.id,
            &sk3,
            pkg,
            &hash,
            true,
            None,
            Some(SemanticVoteV4 {
                disposition: SemanticDispositionV4::Approve,
                model_family: "openai".into(),
                finding_digests: Vec::new(),
            }),
            None,
            "creg-static-v1",
        ),
    ];
    assert!(matches!(
        aggregate_vote_v4_quorum(
            &single_family,
            pkg,
            &hash,
            &validators,
            height,
            &params,
            &cfg,
            &empty_witnesses,
            10_000,
        )
        .unwrap(),
        VoteV4Outcome::Quarantined { .. }
    ));

    // (c) 2 abstentions (1 degraded + 1 QuarantineReview) block Approve quorum -> Quarantined.
    let blocked_by_abstention = vec![
        signed_vote(
            &v1.id,
            &sk1,
            pkg,
            &hash,
            true,
            None,
            Some(SemanticVoteV4 {
                disposition: SemanticDispositionV4::Approve,
                model_family: "gemini".into(),
                finding_digests: Vec::new(),
            }),
            None,
            "creg-static-v1",
        ),
        signed_vote(
            &v2.id,
            &sk2,
            pkg,
            &hash,
            true,
            None,
            Some(SemanticVoteV4 {
                disposition: SemanticDispositionV4::Approve,
                model_family: "claude".into(),
                finding_digests: Vec::new(),
            }),
            None,
            "creg-static-v1",
        ),
        signed_vote(
            &v3.id,
            &sk3,
            pkg,
            &hash,
            true,
            None,
            None,
            None,
            "degraded-llm-timeout",
        ),
        signed_vote(
            &v4.id,
            &sk4,
            pkg,
            &hash,
            false,
            None,
            Some(SemanticVoteV4 {
                disposition: SemanticDispositionV4::QuarantineReview,
                model_family: "llama".into(),
                finding_digests: Vec::new(),
            }),
            None,
            "creg-static-v1",
        ),
    ];
    let q_outcome = aggregate_vote_v4_quorum(
        &blocked_by_abstention,
        pkg,
        &hash,
        &validators,
        height,
        &params,
        &cfg,
        &empty_witnesses,
        50_000,
    )
    .unwrap();
    let (reason, expires_at_unix) = match q_outcome {
        VoteV4Outcome::Quarantined {
            reason,
            expires_at_unix,
            ..
        } => (reason, expires_at_unix),
        other => panic!("expected Quarantined from abstentions, got {other:?}"),
    };
    assert_eq!(expires_at_unix, 50_000 + CLASS_C_QUARANTINE_TTL_SECS);

    // (d) 72h TTL timeout fails closed to PackageStatus::Revoked.
    let status = PackageStatus::Quarantined {
        reason,
        expires_at_unix,
    };
    assert_eq!(
        resolve_quarantine_timeout(&status, expires_at_unix - 1),
        None
    );
    assert!(matches!(
        resolve_quarantine_timeout(&status, expires_at_unix),
        Some(PackageStatus::Revoked { .. })
    ));
}

#[test]
fn witness_replay_guard_dos_cap_and_fraud_penalties() {
    let params = test_params(Some(10));
    let height = 25;
    let pkg = "npm:adv-ci-timebomb-canary@1.0.0";
    let hash = "ee".repeat(32);

    let (v1, _, sk1) = validator_fixture("g1", 61);
    let (v2, _, sk2) = validator_fixture("g2", 62);
    let validators = vec![v1.clone(), v2.clone()];
    let mut guard = WitnessReplayGuard::new();

    // Unauthenticated / tampered witness spam never consumes any of the 2 replay slots.
    for _ in 0..5 {
        let mut forged = timebomb_witness(&v1.id, &sk1, &hash);
        forged.prover_pq_signature = Some("00".repeat(2420));
        assert!(guard
            .preflight_and_reserve(
                &forged,
                pkg,
                &hash,
                Some("creg-policy-v1"),
                &validators,
                height,
                &params,
            )
            .is_err());
    }
    assert_eq!(guard.replays_used(pkg, &hash), 0);
    assert_eq!(guard.bogus_count(&v1.id), 5);

    // 2 valid distinct witnesses consume the full budget (2/2); 3rd distinct fails pre-sandbox.
    let w1 = timebomb_witness(&v1.id, &sk1, &hash);
    assert!(matches!(
        guard
            .preflight_and_reserve(
                &w1,
                pkg,
                &hash,
                Some("creg-policy-v1"),
                &validators,
                height,
                &params,
            )
            .unwrap(),
        WitnessPrecheckDecision::RunReplay { .. }
    ));
    guard.record_replay_outcome(&w1, true);
    assert_eq!(guard.replays_used(pkg, &hash), 1);

    // Duplicate of w1 hits cache without consuming slot 2.
    assert!(matches!(
        guard
            .preflight_and_reserve(
                &w1,
                pkg,
                &hash,
                Some("creg-policy-v1"),
                &validators,
                height,
                &params,
            )
            .unwrap(),
        WitnessPrecheckDecision::CachedResult {
            reproduced: true,
            ..
        }
    ));
    assert_eq!(guard.replays_used(pkg, &hash), 1);

    // 2nd distinct witness by v2 fails to reproduce -> flagged slashable & consumes slot 2.
    let mut w2 = timebomb_witness(&v2.id, &sk2, &hash);
    w2.trigger_spec.clock_offset_secs = 3600;
    w2.sign_with(&sk2).unwrap();
    assert!(matches!(
        guard
            .preflight_and_reserve(
                &w2,
                pkg,
                &hash,
                Some("creg-policy-v1"),
                &validators,
                height,
                &params,
            )
            .unwrap(),
        WitnessPrecheckDecision::RunReplay { .. }
    ));
    guard.record_replay_outcome(&w2, false);
    assert_eq!(guard.replays_used(pkg, &hash), 2);
    assert!(guard.should_slash_for_bogus_witness(&v2.id));

    // 3rd distinct witness hits DoS cap before reaching sandbox execution.
    let mut w3 = timebomb_witness(&v2.id, &sk2, &hash);
    w3.trigger_spec.clock_offset_secs = 7200;
    w3.sign_with(&sk2).unwrap();
    assert!(guard
        .preflight_and_reserve(
            &w3,
            pkg,
            &hash,
            Some("creg-policy-v1"),
            &validators,
            height,
            &params,
        )
        .is_err());
}
