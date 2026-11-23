//! ADR-019 Increment 2: committee quorum certificates collected over vote
//! gossip (`ECA-02`, decision #3).
//!
//! Every committee member that votes on a package also signs the certificate
//! message for its own decision (approval or rejection, part 2b) with its
//! hybrid key and gossips that signature with the vote. When the v4 outcome
//! is a Class C hold, members sign a quarantine certificate too (part 3c).
//! The finalizing node assembles the signatures for one domain into a
//! [`CommitteeQuorumCertificateV1`] and checks it exactly as block validation
//! will before attaching it.
//!
//! All members derive the same message from data they already share: the
//! domain, the package, content hash and evidence digest of the vote, the
//! admitted committee (`committee_digest` ignores later expansion) and the
//! committee's snapshot height as `authority_height`.

use anyhow::{Context, Result};
use common::{
    AnalysisCommittee, CommitteeQuorumCertificateV1, QcSignatureV1, ValidatorSet,
    APPROVAL_QC_DOMAIN, QUARANTINE_QC_DOMAIN, QUARANTINE_RESOLUTION_QC_DOMAIN, REVOKE_QC_DOMAIN,
};
use std::collections::HashSet;
use std::path::Path;

/// Certificate domains a vote round collects signatures for.
pub const ROUND_QC_DOMAINS: [&str; 4] = [
    APPROVAL_QC_DOMAIN,
    REVOKE_QC_DOMAIN,
    QUARANTINE_QC_DOMAIN,
    QUARANTINE_RESOLUTION_QC_DOMAIN,
];

/// A committee member's certificate signature received with its vote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcVoteSignature {
    pub domain: String,
    pub content_hash: String,
    pub evidence_digest: String,
    pub signature: QcSignatureV1,
}

/// Certificate domain for a vote's own decision.
pub fn vote_qc_domain(approved: bool) -> &'static str {
    if approved {
        APPROVAL_QC_DOMAIN
    } else {
        REVOKE_QC_DOMAIN
    }
}

/// The unsigned certificate every committee member signs in `domain`.
pub fn unsigned_qc(
    committee: &AnalysisCommittee,
    domain: &str,
    package_canonical: &str,
    content_hash: &str,
    evidence_digest: &str,
) -> CommitteeQuorumCertificateV1 {
    CommitteeQuorumCertificateV1::new(
        domain,
        package_canonical,
        content_hash,
        evidence_digest,
        committee,
        committee.validator_snapshot_height,
    )
}

/// Sign the certificate in `domain` as one committee member.
pub fn sign_qc(
    committee: &AnalysisCommittee,
    domain: &str,
    package_canonical: &str,
    content_hash: &str,
    evidence_digest: &str,
    validator_id: &str,
    key: &common::crypto::SigningKey,
) -> QcSignatureV1 {
    unsigned_qc(
        committee,
        domain,
        package_canonical,
        content_hash,
        evidence_digest,
    )
    .sign_member(validator_id, key)
}

/// Assemble the received `domain` signatures that agree with
/// `content_hash` and `evidence_digest`. Only committee signers are added,
/// which also bounds the certificate (`MAX_QC_SIGNATURES`).
pub fn assemble_qc<'a>(
    committee: &AnalysisCommittee,
    domain: &str,
    package_canonical: &str,
    content_hash: &str,
    evidence_digest: &str,
    received: impl IntoIterator<Item = &'a QcVoteSignature>,
) -> CommitteeQuorumCertificateV1 {
    let signers: HashSet<&str> = common::committee::committee_signers(committee)
        .into_iter()
        .collect();
    let mut qc = unsigned_qc(
        committee,
        domain,
        package_canonical,
        content_hash,
        evidence_digest,
    );
    for entry in received {
        if entry.domain == domain
            && entry
                .content_hash
                .trim()
                .eq_ignore_ascii_case(content_hash.trim())
            && entry
                .evidence_digest
                .trim()
                .eq_ignore_ascii_case(evidence_digest.trim())
            && signers.contains(entry.signature.validator_id.as_str())
            && qc.signatures.len() < common::committee::MAX_QC_SIGNATURES
        {
            qc.add_signature(entry.signature.clone());
        }
    }
    qc
}

/// Check an assembled certificate with the rule block validation applies at
/// `height` (`package_status::signing_authority` and the consensus
/// verifier), so a node never proposes a certificate its peers refuse.
pub fn verify_for_inclusion(
    qc: &CommitteeQuorumCertificateV1,
    domain: &str,
    committee: &AnalysisCommittee,
    data_dir: &Path,
    bootstrap: &ValidatorSet,
    vote_v4_activation_height: Option<u64>,
    height: u64,
) -> Result<usize> {
    let set_at = |at: u64| {
        crate::validator_set_history::verification_set_at(data_dir, at, bootstrap)
            .map(|set| set.validators)
            .map_err(anyhow::Error::msg)
    };
    let at_authority = set_at(qc.authority_height).context("authority validator set")?;
    let at_block = set_at(height).context("validator set at the inclusion height")?;
    let require_hybrid = vote_v4_activation_height.is_some_and(|activation| height >= activation);
    let authority =
        crate::package_status::signing_authority(&at_authority, &at_block, require_hybrid);
    consensus::vote_v4::verify_committee_qc_with_activation(
        qc,
        domain,
        committee,
        &authority,
        vote_v4_activation_height,
        height,
    )
}

/// `domain` certificate over `record` at `height`, or `None` before hybrid
/// activation. The committee is resolved from the record exactly as block
/// validation resolves it (`package_status::assigned_committee`); an error
/// means the collected signatures do not yet form a certificate.
pub fn qc_for_record<'a>(
    domain: &str,
    record: &common::ChainRecord,
    received: impl IntoIterator<Item = &'a QcVoteSignature>,
    data_dir: &Path,
    bootstrap: &ValidatorSet,
    vote_v4_activation_height: Option<u64>,
    height: u64,
) -> Result<Option<CommitteeQuorumCertificateV1>> {
    if !vote_v4_activation_height.is_some_and(|activation| height >= activation) {
        return Ok(None);
    }
    let committee = crate::package_status::assigned_committee(record)
        .context("record carries no valid committee assignment")?;
    let qc = assemble_qc(
        &committee,
        domain,
        &record.id.canonical(),
        &record.content_hash,
        &record.evidence_digest,
        received,
    );
    verify_for_inclusion(
        &qc,
        domain,
        &committee,
        data_dir,
        bootstrap,
        vote_v4_activation_height,
        height,
    )?;
    Ok(Some(qc))
}

/// Approval certificate to attach to `record` (part 2b).
pub fn approval_qc_for_record<'a>(
    record: &common::ChainRecord,
    received: impl IntoIterator<Item = &'a QcVoteSignature>,
    data_dir: &Path,
    bootstrap: &ValidatorSet,
    vote_v4_activation_height: Option<u64>,
    height: u64,
) -> Result<Option<CommitteeQuorumCertificateV1>> {
    qc_for_record(
        APPROVAL_QC_DOMAIN,
        record,
        received,
        data_dir,
        bootstrap,
        vote_v4_activation_height,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::crypto::SigningKey;
    use common::Validator;

    fn member(index: u8) -> (Validator, SigningKey) {
        let ed_seed = [index; 32];
        let key = SigningKey::hybrid_from_seeds(&ed_seed, &[index.wrapping_add(90); 32]);
        let (_, pq) = key.public_key().hybrid_parts().expect("hybrid key");
        (
            Validator {
                id: format!("validator-{index}"),
                pubkey: SigningKey::from_ed25519_seed(&ed_seed)
                    .public_key()
                    .to_hex(),
                pq_pubkey: Some(hex::encode(pq)),
                status: "active".into(),
                stake: 1,
                ..Validator::default()
            },
            key,
        )
    }

    #[test]
    fn gossiped_signatures_assemble_into_a_certificate_peers_accept() {
        let dir = tempfile::tempdir().expect("tempdir");
        let members: Vec<(Validator, SigningKey)> = (1..=4).map(member).collect();
        let bootstrap = ValidatorSet::new(members.iter().map(|(v, _)| v.clone()).collect());
        let committee = AnalysisCommittee {
            size: 4,
            effective_size: 4,
            members: members.iter().map(|(v, _)| v.id.clone()).collect(),
            seed: "seed".into(),
            validator_snapshot_height: 3,
            ..AnalysisCommittee::default()
        };
        let evidence = common::sha256_hex(b"evidence");
        let content = common::sha256_hex(b"content");
        let vote = |(validator, key): &(Validator, SigningKey), domain: &str, digest: &str| {
            QcVoteSignature {
                domain: domain.to_string(),
                content_hash: content.clone(),
                evidence_digest: digest.to_string(),
                signature: sign_qc(
                    &committee,
                    domain,
                    "npm:left-pad@1.0.0",
                    &content,
                    digest,
                    &validator.id,
                    key,
                ),
            }
        };
        let verify = |qc: &CommitteeQuorumCertificateV1, domain: &str| {
            verify_for_inclusion(qc, domain, &committee, dir.path(), &bootstrap, Some(1), 10)
        };
        // Two approvals agree; one approval saw other evidence; one rejected.
        let received = [
            vote(&members[0], APPROVAL_QC_DOMAIN, &evidence),
            vote(&members[1], APPROVAL_QC_DOMAIN, &evidence),
            vote(
                &members[2],
                APPROVAL_QC_DOMAIN,
                &common::sha256_hex(b"other"),
            ),
            vote(&members[3], REVOKE_QC_DOMAIN, &evidence),
        ];
        let assemble = |domain: &str, received: &[QcVoteSignature]| {
            assemble_qc(
                &committee,
                domain,
                "npm:left-pad@1.0.0",
                &content,
                &evidence,
                received,
            )
        };
        let qc = assemble(APPROVAL_QC_DOMAIN, &received);
        assert_eq!(qc.signatures.len(), 2);
        assert_eq!(qc.authority_height, 3);
        assert!(verify(&qc, APPROVAL_QC_DOMAIN).is_err());

        // With a third agreeing approval it is a 3-of-4 certificate.
        let mut received = received.to_vec();
        received[2] = vote(&members[2], APPROVAL_QC_DOMAIN, &evidence);
        let qc = assemble(APPROVAL_QC_DOMAIN, &received);
        assert_eq!(verify(&qc, APPROVAL_QC_DOMAIN).expect("certificate"), 3);
        // The same signatures do not form a rejection certificate.
        assert!(verify(&qc, REVOKE_QC_DOMAIN).is_err());

        // A hold is certified by separate quarantine-domain signatures;
        // approval signatures never count towards it.
        let mut held = received.clone();
        assert!(verify(&assemble(QUARANTINE_QC_DOMAIN, &held), QUARANTINE_QC_DOMAIN).is_err());
        for member in members.iter().take(3) {
            held.push(vote(member, QUARANTINE_QC_DOMAIN, &evidence));
        }
        let hold = assemble(QUARANTINE_QC_DOMAIN, &held);
        assert_eq!(hold.signatures.len(), 3);
        assert_eq!(
            verify(&hold, QUARANTINE_QC_DOMAIN).expect("hold certificate"),
            3
        );
    }
}
