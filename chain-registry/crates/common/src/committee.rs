//! ADR-019 Increment 2 wire additions: closed display vocabularies and the
//! committee quorum certificate (`ECA-02`, `ECA-06`).
//!
//! Everything here is pre-activation safe: the types only appear on the wire
//! in transactions and fields that are omitted (or invalid) before
//! `hybrid_signature_activation_height`, so historical encodings and the
//! frozen golden vectors are unchanged.

use crate::crypto::{
    Signature, SignatureScheme, SigningKey, VerifyMode, ED25519_SIGNATURE_LEN,
    MLDSA44_SIGNATURE_LEN,
};
use crate::vote_v4::MLDSA44_SIGNATURE_HEX_LEN;
use crate::{sha256_hex, validator_status_is_active, AnalysisCommittee, Validator};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Committee certificate over a `RevokeV2` (Class A/B rejection).
pub const REVOKE_QC_DOMAIN: &str = "creg-revoke-qc-v1";
/// Committee certificate over a `QuarantineV1` hold (Class C).
pub const QUARANTINE_QC_DOMAIN: &str = "creg-quarantine-qc-v1";
/// Committee certificate over a `QuarantineResolutionV1`.
pub const QUARANTINE_RESOLUTION_QC_DOMAIN: &str = "creg-quarantine-resolution-qc-v1";
/// Committee certificate attached to an approval (`PublishAuthorizedV1.committee_qc`).
pub const APPROVAL_QC_DOMAIN: &str = "creg-approval-qc-v1";

/// Upper bound on signatures carried by one certificate (`ECO-INV-08`).
pub const MAX_QC_SIGNATURES: usize = 256;

const QC_DOMAINS: [&str; 4] = [
    REVOKE_QC_DOMAIN,
    QUARANTINE_QC_DOMAIN,
    QUARANTINE_RESOLUTION_QC_DOMAIN,
    APPROVAL_QC_DOMAIN,
];

/// Closed vocabulary for `TriggerWitnessV1.observed_violations`. This is the
/// sandbox's existing `KernelViolationCode`, promoted so the wire validator,
/// indexer and renderers share one dictionary. The wire stays `Vec<String>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum KernelViolationCode {
    #[serde(rename = "KB-CANARY-EXFIL")]
    KbCanaryExfil,
    #[serde(rename = "KB-MEMFD-EXEC")]
    KbMemfdExec,
    #[serde(rename = "KB-PTRACE-ESCAPE")]
    KbPtraceEscape,
    #[serde(rename = "KB-IOURING-EVASION")]
    KbIouringEvasion,
    #[serde(rename = "KB-NET-UNDECLARED")]
    KbNetUndeclared,
    #[serde(rename = "KB-FS-PERSIST")]
    KbFsPersist,
    #[serde(rename = "KB-SECCOMP-VIOLATION")]
    KbSeccompViolation,
    #[serde(rename = "KB-ROOTFS-DIGEST-MISMATCH")]
    KbRootfsDigestMismatch,
    #[serde(rename = "KB-PROC-UNDECLARED")]
    KbProcUndeclared,
}

impl KernelViolationCode {
    pub const ALL: [KernelViolationCode; 9] = [
        Self::KbCanaryExfil,
        Self::KbMemfdExec,
        Self::KbPtraceEscape,
        Self::KbIouringEvasion,
        Self::KbNetUndeclared,
        Self::KbFsPersist,
        Self::KbSeccompViolation,
        Self::KbRootfsDigestMismatch,
        Self::KbProcUndeclared,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::KbCanaryExfil => "KB-CANARY-EXFIL",
            Self::KbMemfdExec => "KB-MEMFD-EXEC",
            Self::KbPtraceEscape => "KB-PTRACE-ESCAPE",
            Self::KbIouringEvasion => "KB-IOURING-EVASION",
            Self::KbNetUndeclared => "KB-NET-UNDECLARED",
            Self::KbFsPersist => "KB-FS-PERSIST",
            Self::KbSeccompViolation => "KB-SECCOMP-VIOLATION",
            Self::KbRootfsDigestMismatch => "KB-ROOTFS-DIGEST-MISMATCH",
            Self::KbProcUndeclared => "KB-PROC-UNDECLARED",
        }
    }

    /// Exact (case-sensitive) wire code lookup; anything else is unknown.
    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|known| known.as_str() == code)
    }

    /// Static human title (renderers never display wire text for codes).
    pub fn title(self) -> &'static str {
        match self {
            Self::KbCanaryExfil => "Canary secret exfiltration",
            Self::KbMemfdExec => "Execution from anonymous memory (memfd)",
            Self::KbPtraceEscape => "ptrace-based sandbox escape attempt",
            Self::KbIouringEvasion => "io_uring syscall-filter evasion",
            Self::KbNetUndeclared => "Undeclared network access",
            Self::KbFsPersist => "Filesystem persistence outside allowed paths",
            Self::KbSeccompViolation => "Seccomp policy violation",
            Self::KbRootfsDigestMismatch => "Sandbox root filesystem digest mismatch",
            Self::KbProcUndeclared => "Undeclared process spawn",
        }
    }
}

/// Closed Class C finding taxonomy. Renderers never display free-form
/// category text; unknown values fail to decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CregFindingCategoryV1 {
    CredentialExfiltration,
    RemoteCodeFetch,
    InstallHookAbuse,
    ObfuscatedPayload,
    NativeDropper,
    Backdoor,
    AuthBypass,
    CryptoWeakening,
    Typosquat,
    DependencyConfusion,
    DestructivePayload,
    PromptInjectionBait,
    Other,
}

/// Final decision on a quarantined package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuarantineOutcomeV1 {
    /// The committee cleared the package: it becomes `Verified`.
    Release,
    /// The committee confirmed the hold: it becomes `Revoked`.
    Reject,
}

impl QuarantineOutcomeV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Reject => "reject",
        }
    }
}

/// One committee member's signature inside a certificate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QcSignatureV1 {
    pub validator_id: String,
    pub voter_key_id: String,
    /// Hex Ed25519 signature over [`CommitteeQuorumCertificateV1::signing_message`].
    pub signature: String,
    /// Hex ML-DSA-44 component; mandatory after hybrid activation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pq_signature: Option<String>,
}

/// Committee certificate that makes a validator-originated decision
/// independently verifiable (`ECA-02`, `ECO-INV-04`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitteeQuorumCertificateV1 {
    /// One of the `*_QC_DOMAIN` constants.
    pub domain: String,
    pub package_canonical: String,
    pub content_hash: String,
    pub evidence_digest: String,
    /// [`committee_digest`] of the analysis committee whose members signed.
    pub committee_digest: String,
    /// Height whose authenticated validator set authorizes the signers.
    pub authority_height: u64,
    /// `TriggerWitnessV1::canonical_digest` when the decision rests on a
    /// Class B witness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub witness_digest: Option<String>,
    /// Bound decision for `QUARANTINE_RESOLUTION_QC_DOMAIN` certificates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<QuarantineOutcomeV1>,
    /// Sorted by `validator_id`, unique.
    pub signatures: Vec<QcSignatureV1>,
}

/// Commitment to the ordered members and seed of an analysis committee:
/// `sha256("creg-committee-v2|" + members.join(",") + "|" + seed)`.
///
/// `effective_size` is deliberately excluded: the pipeline expands it while
/// votes arrive, and a signature made before an expansion must stay valid.
/// The signer set and the threshold still follow it ([`committee_signers`]).
pub fn committee_digest(committee: &AnalysisCommittee) -> String {
    sha256_hex(
        format!(
            "creg-committee-v2|{}|{}",
            committee.members.join(","),
            committee.seed.trim(),
        )
        .as_bytes(),
    )
}

/// Members of the primary committee that may sign (`effective_size` first members,
/// falling back to `size`, then to every member).
pub fn committee_signers(committee: &AnalysisCommittee) -> Vec<&str> {
    let size = if committee.effective_size > 0 {
        committee.effective_size
    } else if committee.size > 0 {
        committee.size
    } else {
        u32::MAX
    };
    committee
        .members
        .iter()
        .take(usize::try_from(size).unwrap_or(usize::MAX))
        .map(String::as_str)
        .collect()
}

/// `>= 2m/3 + 1` of `m` committee signers.
pub fn qc_quorum_threshold(committee_size: usize) -> usize {
    if committee_size == 0 {
        usize::MAX
    } else {
        (2 * committee_size / 3) + 1
    }
}

fn is_hex_exact(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn decode_hex_exact(value: &str, bytes: usize, label: &str) -> Result<Vec<u8>> {
    if !is_hex_exact(value, bytes * 2) {
        bail!("{label} must be {} hex chars", bytes * 2);
    }
    hex::decode(value).with_context(|| format!("{label} is not valid hex"))
}

impl CommitteeQuorumCertificateV1 {
    /// Unsigned certificate for `domain`; members then add signatures.
    pub fn new(
        domain: &str,
        package_canonical: &str,
        content_hash: &str,
        evidence_digest: &str,
        committee: &AnalysisCommittee,
        authority_height: u64,
    ) -> Self {
        Self {
            domain: domain.to_string(),
            package_canonical: package_canonical.trim().to_string(),
            content_hash: content_hash.trim().to_ascii_lowercase(),
            evidence_digest: evidence_digest.trim().to_ascii_lowercase(),
            committee_digest: committee_digest(committee),
            authority_height,
            witness_digest: None,
            decision: None,
            signatures: Vec::new(),
        }
    }

    /// Message every member signs. Signatures are excluded, so the identity
    /// is stable across hedged ML-DSA signatures.
    pub fn signing_message(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            self.domain.trim(),
            self.package_canonical.trim(),
            self.content_hash.trim().to_ascii_lowercase(),
            self.evidence_digest.trim().to_ascii_lowercase(),
            self.committee_digest.trim().to_ascii_lowercase(),
            self.authority_height,
            self.witness_digest
                .as_deref()
                .map(|digest| digest.trim().to_ascii_lowercase())
                .unwrap_or_default(),
            self.decision.map(QuarantineOutcomeV1::as_str).unwrap_or(""),
        )
    }

    /// Stable certificate identity (`witness_id`-style index key).
    pub fn digest(&self) -> String {
        sha256_hex(self.signing_message().as_bytes())
    }

    /// Structural checks that need no keys: known domain, bounded hex fields,
    /// bounded, sorted and unique signers.
    pub fn validate_shape(&self) -> Result<()> {
        if !QC_DOMAINS.contains(&self.domain.as_str()) {
            bail!("unknown committee QC domain {:?}", self.domain);
        }
        let package = self.package_canonical.trim();
        if package.is_empty() || package.len() > 512 {
            bail!("committee QC package_canonical must be 1..=512 chars");
        }
        for (label, value) in [
            ("content_hash", &self.content_hash),
            ("evidence_digest", &self.evidence_digest),
            ("committee_digest", &self.committee_digest),
        ] {
            if !is_hex_exact(value.trim(), 64) {
                bail!("committee QC {label} must be a 64-char hex digest");
            }
        }
        if let Some(witness) = &self.witness_digest {
            if !is_hex_exact(witness.trim(), 64) {
                bail!("committee QC witness_digest must be a 64-char hex digest");
            }
        }
        match (self.domain.as_str(), self.decision) {
            (QUARANTINE_RESOLUTION_QC_DOMAIN, None) => {
                bail!("quarantine resolution QC must bind a decision")
            }
            (QUARANTINE_RESOLUTION_QC_DOMAIN, Some(_)) => {}
            (_, Some(_)) => bail!("only quarantine resolution QCs carry a decision"),
            (_, None) => {}
        }
        if self.signatures.is_empty() || self.signatures.len() > MAX_QC_SIGNATURES {
            bail!("committee QC must carry 1..={MAX_QC_SIGNATURES} signatures");
        }
        for pair in self.signatures.windows(2) {
            if pair[0].validator_id >= pair[1].validator_id {
                bail!("committee QC signatures must be sorted by validator_id and unique");
            }
        }
        Ok(())
    }

    /// Sign as one committee member (hybrid keys add the ML-DSA component).
    pub fn sign_member(&self, validator_id: &str, key: &SigningKey) -> QcSignatureV1 {
        let signature = key.sign(self.signing_message().as_bytes());
        QcSignatureV1 {
            validator_id: validator_id.to_string(),
            voter_key_id: key.key_id(),
            signature: hex::encode(signature.ed25519_bytes()),
            pq_signature: signature.mldsa44_bytes().map(hex::encode),
        }
    }

    /// Insert a member signature, keeping the list sorted and unique.
    pub fn add_signature(&mut self, signature: QcSignatureV1) {
        match self
            .signatures
            .binary_search_by(|existing| existing.validator_id.cmp(&signature.validator_id))
        {
            Ok(index) => self.signatures[index] = signature,
            Err(index) => self.signatures.insert(index, signature),
        }
    }

    /// Count distinct valid committee signers. A signature counts only when
    /// the signer is in `committee`'s signing set, is an active validator in
    /// `authority` (the set authenticated at `authority_height`), its key id
    /// matches, and its (hybrid, when required) signature verifies. Invalid
    /// signatures are skipped, never fatal, so a forger cannot veto a
    /// certificate by appending garbage. Shape errors are fatal.
    pub fn count_valid_signers(
        &self,
        expected_domain: &str,
        committee: &AnalysisCommittee,
        authority: &[Validator],
        require_hybrid: bool,
    ) -> Result<usize> {
        self.validate_shape()?;
        if self.domain != expected_domain {
            bail!(
                "committee QC domain {:?} does not match expected {:?}",
                self.domain,
                expected_domain
            );
        }
        if !self
            .committee_digest
            .eq_ignore_ascii_case(&committee_digest(committee))
        {
            bail!("committee QC does not commit to this analysis committee");
        }
        let signers: HashSet<&str> = committee_signers(committee).into_iter().collect();
        let message = self.signing_message();
        let mut valid = 0usize;
        for entry in &self.signatures {
            if !signers.contains(entry.validator_id.as_str()) {
                continue;
            }
            let Some(validator) = authority
                .iter()
                .find(|v| v.id == entry.validator_id && validator_status_is_active(&v.status))
            else {
                continue;
            };
            let Ok(public_key) = validator.verifying_key(require_hybrid) else {
                continue;
            };
            if !entry
                .voter_key_id
                .eq_ignore_ascii_case(&public_key.key_id())
            {
                continue;
            }
            let Ok(ed) = decode_hex_exact(
                &entry.signature,
                ED25519_SIGNATURE_LEN,
                "QC Ed25519 signature",
            ) else {
                continue;
            };
            let signature = match (require_hybrid, &entry.pq_signature) {
                (false, None) => Signature::from_bytes(SignatureScheme::Ed25519, &ed).ok(),
                (true, Some(pq_hex)) if pq_hex.len() == MLDSA44_SIGNATURE_HEX_LEN => {
                    decode_hex_exact(pq_hex, MLDSA44_SIGNATURE_LEN, "QC ML-DSA signature")
                        .ok()
                        .and_then(|pq| Signature::hybrid_from_parts(&ed, &pq).ok())
                }
                // Pre-activation certificates must not carry PQ parts;
                // post-activation ones must.
                _ => None,
            };
            let Some(signature) = signature else {
                continue;
            };
            if public_key
                .verify(message.as_bytes(), &signature, VerifyMode::Strict)
                .is_ok()
            {
                valid += 1;
            }
        }
        Ok(valid)
    }

    /// Verify that at least `2m/3 + 1` of the committee's `m` signers signed.
    pub fn verify_quorum(
        &self,
        expected_domain: &str,
        committee: &AnalysisCommittee,
        authority: &[Validator],
        require_hybrid: bool,
    ) -> Result<usize> {
        let valid =
            self.count_valid_signers(expected_domain, committee, authority, require_hybrid)?;
        let required = qc_quorum_threshold(committee_signers(committee).len());
        if valid < required {
            bail!(
                "committee QC has {valid} valid signer(s); {required} required for a committee of {}",
                committee_signers(committee).len()
            );
        }
        Ok(valid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, seed: u8, hybrid: bool) -> (Validator, SigningKey) {
        let ed_seed = [seed; 32];
        let (key, pq_pubkey) = if hybrid {
            let key = SigningKey::hybrid_from_seeds(&ed_seed, &[seed.wrapping_add(90); 32]);
            let (_, pq) = key.public_key().hybrid_parts().unwrap();
            let pq_hex = hex::encode(pq);
            (key, Some(pq_hex))
        } else {
            (SigningKey::from_ed25519_seed(&ed_seed), None)
        };
        let ed_pub = if hybrid {
            hex::encode(key.public_key().hybrid_parts().unwrap().0)
        } else {
            key.public_key().to_hex()
        };
        (
            Validator {
                id: id.to_string(),
                alias: id.to_string(),
                pubkey: ed_pub,
                status: "active".to_string(),
                stake: 1,
                pq_pubkey,
                ..Validator::default()
            },
            key,
        )
    }

    fn committee(ids: &[&str]) -> AnalysisCommittee {
        AnalysisCommittee {
            size: ids.len() as u32,
            effective_size: ids.len() as u32,
            members: ids.iter().map(|id| id.to_string()).collect(),
            seed: "seed".to_string(),
            ..AnalysisCommittee::default()
        }
    }

    fn signed_qc(
        members: &[(Validator, SigningKey)],
        signers: usize,
        committee: &AnalysisCommittee,
    ) -> CommitteeQuorumCertificateV1 {
        let mut qc = CommitteeQuorumCertificateV1::new(
            REVOKE_QC_DOMAIN,
            "npm/left-padx@1.4.2",
            &"aa".repeat(32),
            &"bb".repeat(32),
            committee,
            42,
        );
        for (validator, key) in members.iter().take(signers) {
            qc.add_signature(qc.sign_member(&validator.id, key));
        }
        qc
    }

    #[test]
    fn classical_quorum_counts_distinct_valid_members() {
        let members: Vec<_> = (0..4)
            .map(|i| member(&format!("v{i}"), 10 + i as u8, false))
            .collect();
        let ids: Vec<&str> = members.iter().map(|(v, _)| v.id.as_str()).collect();
        let committee = committee(&ids);
        let authority: Vec<Validator> = members.iter().map(|(v, _)| v.clone()).collect();

        // 4 members -> threshold 3.
        let qc = signed_qc(&members, 3, &committee);
        assert_eq!(
            qc.verify_quorum(REVOKE_QC_DOMAIN, &committee, &authority, false)
                .unwrap(),
            3
        );
        let short = signed_qc(&members, 2, &committee);
        assert!(short
            .verify_quorum(REVOKE_QC_DOMAIN, &committee, &authority, false)
            .is_err());
        // The domain is bound.
        assert!(qc
            .verify_quorum(QUARANTINE_QC_DOMAIN, &committee, &authority, false)
            .is_err());
    }

    #[test]
    fn eco_adv_05_single_signer_and_outsiders_never_reach_quorum() {
        let members: Vec<_> = (0..4)
            .map(|i| member(&format!("v{i}"), 20 + i as u8, false))
            .collect();
        let ids: Vec<&str> = members.iter().map(|(v, _)| v.id.as_str()).collect();
        let committee = committee(&ids);
        let authority: Vec<Validator> = members.iter().map(|(v, _)| v.clone()).collect();

        let mut qc = signed_qc(&members, 1, &committee);
        // An outsider with a valid key and garbage signatures add nothing.
        let (outsider, outsider_key) = member("zz-outsider", 99, false);
        qc.add_signature(qc.sign_member(&outsider.id, &outsider_key));
        let mut garbage = qc.sign_member("v3", &members[3].1);
        garbage.signature = "00".repeat(64);
        qc.add_signature(garbage);
        assert_eq!(
            qc.count_valid_signers(REVOKE_QC_DOMAIN, &committee, &authority, false)
                .unwrap(),
            1
        );
        assert!(qc
            .verify_quorum(REVOKE_QC_DOMAIN, &committee, &authority, false)
            .is_err());
    }

    #[test]
    fn edits_after_signing_invalidate_signatures() {
        let members: Vec<_> = (0..3)
            .map(|i| member(&format!("v{i}"), 30 + i as u8, false))
            .collect();
        let ids: Vec<&str> = members.iter().map(|(v, _)| v.id.as_str()).collect();
        let committee = committee(&ids);
        let authority: Vec<Validator> = members.iter().map(|(v, _)| v.clone()).collect();
        let mut qc = signed_qc(&members, 3, &committee);
        qc.package_canonical = "npm/react@19.0.0".to_string();
        assert_eq!(
            qc.count_valid_signers(REVOKE_QC_DOMAIN, &committee, &authority, false)
                .unwrap(),
            0
        );

        // A different committee is rejected outright.
        let other = AnalysisCommittee {
            seed: "other-seed".into(),
            ..committee.clone()
        };
        let qc = signed_qc(&members, 3, &committee);
        assert!(qc
            .count_valid_signers(REVOKE_QC_DOMAIN, &other, &authority, false)
            .is_err());
    }

    #[test]
    fn signatures_survive_committee_expansion() {
        // 6 ranked members, primary committee of 3: threshold 3 of 3.
        let members: Vec<_> = (0..6)
            .map(|i| member(&format!("v{i}"), 60 + i as u8, false))
            .collect();
        let ids: Vec<&str> = members.iter().map(|(v, _)| v.id.as_str()).collect();
        let primary = AnalysisCommittee {
            size: 3,
            effective_size: 3,
            ..committee(&ids)
        };
        let authority: Vec<Validator> = members.iter().map(|(v, _)| v.clone()).collect();
        // Two primary members signed before the pipeline expanded the
        // committee; two volunteers signed after.
        let mut qc = signed_qc(&members, 2, &primary);
        let expanded = AnalysisCommittee {
            effective_size: 6,
            ..primary.clone()
        };
        assert_eq!(committee_digest(&primary), committee_digest(&expanded));
        for (validator, key) in members.iter().skip(3).take(2) {
            qc.add_signature(qc.sign_member(&validator.id, key));
        }
        // Against the primary committee only its members count (2 < 3).
        assert_eq!(
            qc.count_valid_signers(REVOKE_QC_DOMAIN, &primary, &authority, false)
                .unwrap(),
            2
        );
        assert!(qc
            .verify_quorum(REVOKE_QC_DOMAIN, &primary, &authority, false)
            .is_err());
        // Against the expanded committee all four count, and 4 < 2*6/3+1 = 5.
        assert_eq!(
            qc.count_valid_signers(REVOKE_QC_DOMAIN, &expanded, &authority, false)
                .unwrap(),
            4
        );
        assert!(qc
            .verify_quorum(REVOKE_QC_DOMAIN, &expanded, &authority, false)
            .is_err());
        qc.add_signature(qc.sign_member(&members[2].0.id, &members[2].1));
        assert_eq!(
            qc.verify_quorum(REVOKE_QC_DOMAIN, &expanded, &authority, false)
                .unwrap(),
            5
        );
    }

    #[test]
    fn hybrid_is_required_after_activation_and_refused_before() {
        let members: Vec<_> = (0..3)
            .map(|i| member(&format!("v{i}"), 40 + i as u8, true))
            .collect();
        let ids: Vec<&str> = members.iter().map(|(v, _)| v.id.as_str()).collect();
        let committee = committee(&ids);
        let authority: Vec<Validator> = members.iter().map(|(v, _)| v.clone()).collect();
        let qc = signed_qc(&members, 3, &committee);
        assert!(qc.signatures.iter().all(|s| s.pq_signature.is_some()));
        assert_eq!(
            qc.verify_quorum(REVOKE_QC_DOMAIN, &committee, &authority, true)
                .unwrap(),
            3
        );
        // Pre-activation verification refuses PQ-carrying entries.
        assert_eq!(
            qc.count_valid_signers(REVOKE_QC_DOMAIN, &committee, &authority, false)
                .unwrap(),
            0
        );

        // Ed25519-only halves do not satisfy a post-activation check (eco-adv-04 analogue).
        let mut stripped = qc.clone();
        for signature in &mut stripped.signatures {
            signature.pq_signature = None;
        }
        assert_eq!(
            stripped
                .count_valid_signers(REVOKE_QC_DOMAIN, &committee, &authority, true)
                .unwrap(),
            0
        );
    }

    #[test]
    fn shape_rules_are_enforced() {
        let members: Vec<_> = (0..2)
            .map(|i| member(&format!("v{i}"), 50 + i as u8, false))
            .collect();
        let ids: Vec<&str> = members.iter().map(|(v, _)| v.id.as_str()).collect();
        let committee = committee(&ids);
        let mut qc = signed_qc(&members, 2, &committee);
        assert!(qc.validate_shape().is_ok());

        let mut unsorted = qc.clone();
        unsorted.signatures.reverse();
        assert!(unsorted.validate_shape().is_err());

        let mut unknown = qc.clone();
        unknown.domain = "creg-made-up".into();
        assert!(unknown.validate_shape().is_err());

        let mut resolution = qc.clone();
        resolution.domain = QUARANTINE_RESOLUTION_QC_DOMAIN.into();
        assert!(resolution.validate_shape().is_err(), "decision required");
        resolution.decision = Some(QuarantineOutcomeV1::Release);
        assert!(resolution.validate_shape().is_ok());

        qc.decision = Some(QuarantineOutcomeV1::Reject);
        assert!(
            qc.validate_shape().is_err(),
            "only resolutions carry a decision"
        );

        let json = serde_json::to_value(&resolution).unwrap();
        assert_eq!(json["decision"], "release");
        assert!(
            serde_json::from_value::<CommitteeQuorumCertificateV1>(serde_json::json!({
                "domain": REVOKE_QC_DOMAIN, "package_canonical": "x", "content_hash": "aa",
                "evidence_digest": "bb", "committee_digest": "cc", "authority_height": 1,
                "signatures": [], "unexpected": true
            }))
            .is_err()
        );
    }

    #[test]
    fn closed_vocabularies_round_trip() {
        for code in KernelViolationCode::ALL {
            assert_eq!(KernelViolationCode::parse(code.as_str()), Some(code));
            let json = serde_json::to_value(code).unwrap();
            assert_eq!(json, code.as_str());
        }
        assert_eq!(KernelViolationCode::parse("kb-canary-exfil"), None);
        assert_eq!(
            serde_json::to_value(CregFindingCategoryV1::PromptInjectionBait).unwrap(),
            "prompt_injection_bait"
        );
        assert!(serde_json::from_str::<CregFindingCategoryV1>("\"made_up\"").is_err());
    }
}
