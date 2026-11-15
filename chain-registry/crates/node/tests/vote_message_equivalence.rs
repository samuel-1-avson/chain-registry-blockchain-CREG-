//! The node and the consensus crate each define `canonical_vote_message`.
//! Validators sign with one and the vote accumulator verifies with the other,
//! so the two must stay byte-identical. This test fails if either copy drifts.

#[test]
fn node_and_consensus_build_identical_vote_messages() {
    assert_eq!(
        node::gossip::VOTE_MESSAGE_DOMAIN,
        consensus::vote_accumulator::VOTE_MESSAGE_DOMAIN
    );

    for approved in [true, false] {
        for (canonical, content_hash, pubkey, profile, evidence) in [
            ("npm:ms@2.1.3", "ab12", "pk", "profile", "evidence"),
            ("pypi:requests@2.32.0", "", "", "", ""),
            (
                "cargo:serde@1.0.0",
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "k|k",
                "p|p",
                "e|e",
            ),
        ] {
            assert_eq!(
                node::gossip::canonical_vote_message(
                    canonical,
                    content_hash,
                    approved,
                    pubkey,
                    profile,
                    evidence,
                ),
                consensus::vote_accumulator::canonical_vote_message(
                    canonical,
                    content_hash,
                    approved,
                    pubkey,
                    profile,
                    evidence,
                ),
                "message drift for {canonical} approved={approved}"
            );
        }
    }
}
