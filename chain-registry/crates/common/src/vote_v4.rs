//! `creg-vote-v4` Wire Schemas, `TriggerWitnessV1`, `PqKeyBindingV1`, and
//! Quantized `SemanticVoteV4` (`ADR-017` + `ADR-018` Increment 6).
//!
//! Pre-activation consensus encodings (`creg-vote-v2`, `creg-vote-v3`,
//! `BlockHeader`, `Transaction`) remain byte-identical until the chain-spec
//! `hybrid_signature_activation_height` is reached.

use crate::crypto::{
    PublicKey, Signature, SignatureScheme, SigningKey, VerifyMode, ED25519_PUBLIC_KEY_LEN,
    ED25519_SIGNATURE_LEN, MLDSA44_PUBLIC_KEY_LEN, MLDSA44_SIGNATURE_LEN,
};
use crate::{sha256_hex, AnalysisBundleRefs, DeterministicRiskSummary, Validator};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const VOTE_V4_MESSAGE_DOMAIN: &str = "creg-vote-v4";
pub const PQ_BINDING_DOMAIN: &str = "creg-pq-binding-v1";
pub const TRIGGER_WITNESS_DOMAIN: &str = "creg-trigger-witness-v1";

/// Maximum wire size in bytes for the unsigned `TriggerWitnessV1` payload + Ed25519
/// signature (`4 KiB`). Hybrid post-activation witnesses add the fixed 2,420-byte
/// (`4,840`-hex-char) `prover_pq_signature`.
pub const MAX_TRIGGER_WITNESS_WIRE_BYTES: usize = 4_096;
/// Fixed lowercase-hex byte length of a 2,420-byte ML-DSA-44 signature.
pub const MLDSA44_SIGNATURE_HEX_LEN: usize = MLDSA44_SIGNATURE_LEN * 2;
/// Fixed lowercase-hex byte length of a 1,312-byte ML-DSA-44 public key.
pub const MLDSA44_PUBLIC_KEY_HEX_LEN: usize = MLDSA44_PUBLIC_KEY_LEN * 2;
/// Maximum serialized wire size of a signed hybrid `TriggerWitnessV1`.
pub const MAX_HYBRID_TRIGGER_WITNESS_WIRE_BYTES: usize =
    MAX_TRIGGER_WITNESS_WIRE_BYTES + MLDSA44_SIGNATURE_HEX_LEN + 64;

/// Class C `Quarantined` timeout duration (72 hours -> fail-closed `Revoked`).
pub const CLASS_C_QUARANTINE_TTL_SECS: i64 = 72 * 3600;
/// Maximum witness replays any validator performs per package (`INV-04`, ADR-018 #2).
pub const MAX_WITNESS_REPLAYS_PER_PACKAGE: usize = 2;
/// Maximum structured finding digests inside a `SemanticVoteV4`.
pub const MAX_SEMANTIC_FINDING_DIGESTS: usize = 32;

const MAX_TRIGGER_ENV_VARS: usize = 64;
const MAX_TRIGGER_ARGS: usize = 32;

/// Bounded declarative trigger specification transmitted inside `TriggerWitnessV1`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerSpecWireV1 {
    pub schema_version: u16,
    #[serde(default)]
    pub env_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub clock_offset_secs: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_function: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
}

impl TriggerSpecWireV1 {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "unsupported TriggerSpecWireV1 schema_version {} (expected 1)",
                self.schema_version
            );
        }
        if self.env_overrides.len() > MAX_TRIGGER_ENV_VARS {
            bail!(
                "TriggerSpecWireV1 env_overrides count {} exceeds max {}",
                self.env_overrides.len(),
                MAX_TRIGGER_ENV_VARS
            );
        }
        for (k, v) in &self.env_overrides {
            if k.is_empty()
                || k.len() > 128
                || k.contains('=')
                || k.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f)
            {
                bail!("invalid TriggerSpecWireV1 env key '{}'", k);
            }
            if v.len() > 4096 || v.bytes().any(|b| b == 0) {
                bail!("invalid TriggerSpecWireV1 env value for '{}'", k);
            }
        }
        if self.args.len() > MAX_TRIGGER_ARGS {
            bail!(
                "TriggerSpecWireV1 args count {} exceeds max {}",
                self.args.len(),
                MAX_TRIGGER_ARGS
            );
        }
        for arg in &self.args {
            if arg.len() > 1024 || arg.bytes().any(|b| b == 0) {
                bail!("invalid TriggerSpecWireV1 arg");
            }
        }
        if let Some(ref entry) = self.entry_function {
            if entry.is_empty()
                || entry.len() > 256
                || entry.bytes().any(|b| b == 0 || b < 0x20 || b == 0x7f)
            {
                bail!("invalid TriggerSpecWireV1 entry_function");
            }
        }
        Ok(())
    }

    pub fn canonical_digest(&self) -> String {
        let encoded = serde_json::to_vec(self).unwrap_or_default();
        sha256_hex(&encoded)
    }
}

fn is_lower_hex_exact(s: &str, expected_len: usize) -> bool {
    s.len() == expected_len
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn decode_exact_lower_or_mixed_hex(
    hex_str: &str,
    expected_bytes: usize,
    label: &str,
) -> Result<Vec<u8>> {
    let clean = hex_str.trim().strip_prefix("0x").unwrap_or(hex_str.trim());
    let bytes = hex::decode(clean).with_context(|| format!("invalid {label} hex"))?;
    if bytes.len() != expected_bytes {
        bail!(
            "invalid {label} byte length: expected {expected_bytes}, got {}",
            bytes.len()
        );
    }
    Ok(bytes)
}

/// Proof-carrying trigger witness emitted when a conditional/evasive malware
/// exploit triggers kernel violations on deterministic Layer 3 sandbox replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerWitnessV1 {
    pub schema_version: u16,
    pub package_canonical: String,
    pub content_hash: String,
    pub manifest_sha256: String,
    pub sandbox_policy_bundle_id: String,
    pub trigger_spec: TriggerSpecWireV1,
    pub observed_violations: Vec<String>,
    pub graph_canonical_digest: String,
    pub witness_prover_id: String,
    pub prover_key_id: String,
    pub prover_signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prover_pq_signature: Option<String>,
}

impl TriggerWitnessV1 {
    pub fn signing_message(&self) -> String {
        let mut sorted_violations = self.observed_violations.clone();
        sorted_violations.sort();
        sorted_violations.dedup();
        format!(
            "{}|v{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            TRIGGER_WITNESS_DOMAIN,
            self.schema_version,
            self.package_canonical.trim(),
            self.content_hash.trim().to_ascii_lowercase(),
            self.manifest_sha256.trim().to_ascii_lowercase(),
            self.sandbox_policy_bundle_id.trim(),
            self.trigger_spec.canonical_digest(),
            sorted_violations.join(","),
            self.graph_canonical_digest.trim().to_ascii_lowercase(),
            self.witness_prover_id.trim(),
            self.prover_key_id.trim().to_ascii_lowercase(),
        )
    }

    /// Logical witness digest excluding replaceable signature fields so hedged
    /// ML-DSA-44 signatures yield an identical witness identity across peers.
    pub fn canonical_digest(&self) -> String {
        sha256_hex(self.signing_message().as_bytes())
    }

    pub fn validate_wire_budget(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "unsupported TriggerWitnessV1 schema_version {} (expected 1)",
                self.schema_version
            );
        }
        if self.package_canonical.trim().is_empty() {
            bail!("TriggerWitnessV1 package_canonical cannot be empty");
        }
        if self.witness_prover_id.trim().is_empty() {
            bail!("TriggerWitnessV1 witness_prover_id cannot be empty");
        }
        self.trigger_spec.validate()?;
        if self.observed_violations.is_empty() || self.observed_violations.len() > 16 {
            bail!("TriggerWitnessV1 observed_violations must contain 1..=16 entries");
        }
        if self.graph_canonical_digest.len() != 64
            || !self
                .graph_canonical_digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            bail!("TriggerWitnessV1 graph_canonical_digest must be a 64-char hex digest");
        }
        if let Some(ref pq_sig) = self.prover_pq_signature {
            if !is_lower_hex_exact(pq_sig, MLDSA44_SIGNATURE_HEX_LEN) {
                bail!(
                    "TriggerWitnessV1 prover_pq_signature must be {} lowercase hex chars",
                    MLDSA44_SIGNATURE_HEX_LEN
                );
            }
        }
        // Check the <= 4 KiB core wire budget (unsigned core + Ed25519 signature),
        // preventing arbitrary payload inflation even when a fixed 4,840-char
        // ML-DSA-44 signature is attached post-activation.
        let mut core_view = self.clone();
        core_view.prover_pq_signature = None;
        let core_len = serde_json::to_vec(&core_view)
            .context("serialize TriggerWitnessV1 core view")?
            .len();
        if core_len > MAX_TRIGGER_WITNESS_WIRE_BYTES {
            bail!(
                "TriggerWitnessV1 core size {} bytes exceeds {} byte wire limit",
                core_len,
                MAX_TRIGGER_WITNESS_WIRE_BYTES
            );
        }
        let full_len = serde_json::to_vec(self)
            .context("serialize full TriggerWitnessV1")?
            .len();
        if full_len > MAX_HYBRID_TRIGGER_WITNESS_WIRE_BYTES {
            bail!(
                "TriggerWitnessV1 total size {} bytes exceeds {} byte hybrid limit",
                full_len,
                MAX_HYBRID_TRIGGER_WITNESS_WIRE_BYTES
            );
        }
        Ok(())
    }

    pub fn sign_with(&mut self, key: &SigningKey) -> Result<()> {
        self.prover_key_id = key.key_id();
        let sig = key.sign(self.signing_message().as_bytes());
        self.prover_signature = hex::encode(sig.ed25519_bytes());
        self.prover_pq_signature = sig.mldsa44_bytes().map(hex::encode);
        self.validate_wire_budget()
    }

    pub fn verify(&self, pk: &PublicKey, require_hybrid: bool) -> Result<()> {
        self.validate_wire_budget()?;
        if !self.prover_key_id.eq_ignore_ascii_case(&pk.key_id()) {
            bail!(
                "TriggerWitnessV1 prover_key_id {} does not match key_id {}",
                self.prover_key_id,
                pk.key_id()
            );
        }
        let ed_sig = decode_exact_lower_or_mixed_hex(
            &self.prover_signature,
            ED25519_SIGNATURE_LEN,
            "prover Ed25519 signature",
        )?;
        let sig = match (require_hybrid, &self.prover_pq_signature) {
            (false, Some(_)) => {
                bail!("pre-activation TriggerWitnessV1 must not carry prover_pq_signature")
            }
            (false, None) => Signature::from_bytes(SignatureScheme::Ed25519, &ed_sig)?,
            (true, None) => {
                bail!("post-activation TriggerWitnessV1 requires prover_pq_signature")
            }
            (true, Some(pq_hex)) => {
                let pq_sig = decode_exact_lower_or_mixed_hex(
                    pq_hex,
                    MLDSA44_SIGNATURE_LEN,
                    "prover ML-DSA-44 signature",
                )?;
                Signature::hybrid_from_parts(&ed_sig, &pq_sig)?
            }
        };
        pk.verify(self.signing_message().as_bytes(), &sig, VerifyMode::Strict)
            .context("TriggerWitnessV1 signature verification failed")
    }
}

/// Dual-signed possession proof binding a validator's Ed25519 identity key to
/// its independent FIPS 204 ML-DSA-44 public key (`ADR-017` §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PqKeyBindingV1 {
    pub validator_id: String,
    pub ed25519_pubkey: String,
    pub pq_pubkey: String,
    pub chain_id: String,
    pub ed25519_signature: String,
    pub pq_signature: String,
}

impl PqKeyBindingV1 {
    pub fn binding_message(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            PQ_BINDING_DOMAIN,
            self.validator_id.trim(),
            self.ed25519_pubkey.trim().to_ascii_lowercase(),
            self.pq_pubkey.trim().to_ascii_lowercase(),
            self.chain_id.trim(),
        )
    }

    pub fn sign_with(
        validator_id: impl Into<String>,
        chain_id: impl Into<String>,
        hybrid_key: &SigningKey,
    ) -> Result<Self> {
        if hybrid_key.scheme() != SignatureScheme::HybridEd25519MlDsa44V1 {
            bail!("PqKeyBindingV1 requires a HybridEd25519MlDsa44V1 signing key");
        }
        let pk = hybrid_key.public_key();
        let (ed_pk, pq_pk) = pk
            .hybrid_parts()
            .context("missing hybrid public key components")?;
        let mut binding = Self {
            validator_id: validator_id.into(),
            ed25519_pubkey: hex::encode(ed_pk),
            pq_pubkey: hex::encode(pq_pk),
            chain_id: chain_id.into(),
            ed25519_signature: String::new(),
            pq_signature: String::new(),
        };
        let sig = hybrid_key.sign(binding.binding_message().as_bytes());
        let (ed_sig, pq_sig) = sig
            .hybrid_parts()
            .context("missing hybrid signature components")?;
        binding.ed25519_signature = hex::encode(ed_sig);
        binding.pq_signature = hex::encode(pq_sig);
        Ok(binding)
    }

    pub fn verify(&self) -> Result<()> {
        if self.validator_id.trim().is_empty() || self.chain_id.trim().is_empty() {
            bail!("PqKeyBindingV1 validator_id and chain_id must be non-empty");
        }
        let ed_pk = decode_exact_lower_or_mixed_hex(
            &self.ed25519_pubkey,
            ED25519_PUBLIC_KEY_LEN,
            "Ed25519 public key",
        )?;
        let pq_pk = decode_exact_lower_or_mixed_hex(
            &self.pq_pubkey,
            MLDSA44_PUBLIC_KEY_LEN,
            "ML-DSA-44 public key",
        )?;
        let pk = PublicKey::hybrid_from_parts(&ed_pk, &pq_pk)?;

        let ed_sig = decode_exact_lower_or_mixed_hex(
            &self.ed25519_signature,
            ED25519_SIGNATURE_LEN,
            "Ed25519 signature",
        )?;
        let pq_sig = decode_exact_lower_or_mixed_hex(
            &self.pq_signature,
            MLDSA44_SIGNATURE_LEN,
            "ML-DSA-44 signature",
        )?;
        let sig = Signature::hybrid_from_parts(&ed_sig, &pq_sig)?;

        pk.verify(self.binding_message().as_bytes(), &sig, VerifyMode::Strict)
            .context("PqKeyBindingV1 dual signature verification failed")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticDispositionV4 {
    Approve,
    QuarantineReview,
    Reject,
}

/// Quantized Layer 4 semantic vote committed onto `creg-vote-v4`.
/// Never contains free-form LLM prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticVoteV4 {
    pub disposition: SemanticDispositionV4,
    pub model_family: String,
    #[serde(default)]
    pub finding_digests: Vec<String>,
}

impl SemanticVoteV4 {
    pub fn validate(&self) -> Result<()> {
        let fam = self.model_family.trim();
        if fam.is_empty() || fam.len() > 64 {
            bail!("SemanticVoteV4 model_family must be 1..=64 chars");
        }
        if self.finding_digests.len() > MAX_SEMANTIC_FINDING_DIGESTS {
            bail!(
                "SemanticVoteV4 finding_digests count {} exceeds max {}",
                self.finding_digests.len(),
                MAX_SEMANTIC_FINDING_DIGESTS
            );
        }
        for digest in &self.finding_digests {
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("SemanticVoteV4 finding_digest must be a 64-char hex digest");
            }
        }
        Ok(())
    }

    pub fn digest(&self) -> String {
        let mut normalized = self.clone();
        normalized.model_family = normalized.model_family.trim().to_ascii_lowercase();
        for d in &mut normalized.finding_digests {
            *d = d.trim().to_ascii_lowercase();
        }
        normalized.finding_digests.sort();
        normalized.finding_digests.dedup();
        let bytes = serde_json::to_vec(&normalized).unwrap_or_default();
        sha256_hex(&bytes)
    }
}

/// Unified post-activation `creg-vote-v4` envelope combining Hybrid PQ
/// authentication (`voter_key_id`), deterministic Class A risk, optional
/// `TriggerWitnessV1` (Class B), and optional quantized `SemanticVoteV4` (Class C).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoteV4Envelope {
    pub package_canonical: String,
    pub content_hash: String,
    pub approved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<String>,
    pub validator_id: String,
    pub voter_key_id: String,
    pub scanner_profile_digest: String,
    pub evidence_digest: String,
    pub publisher_authorization_digest: String,
    pub vote_metadata_digest: String,
    pub ml_model_version: String,
    pub analysis_bundles: AnalysisBundleRefs,
    pub deterministic_risk: DeterministicRiskSummary,
    pub is_mandatory: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub witness: Option<TriggerWitnessV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_vote: Option<SemanticVoteV4>,
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pq_signature: Option<String>,
}

impl VoteV4Envelope {
    pub fn canonical_vote_v4_message(&self) -> String {
        let witness_digest = self
            .witness
            .as_ref()
            .map(TriggerWitnessV1::canonical_digest)
            .unwrap_or_default();
        let semantic_digest = self
            .semantic_vote
            .as_ref()
            .map(SemanticVoteV4::digest)
            .unwrap_or_default();
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            VOTE_V4_MESSAGE_DOMAIN,
            self.package_canonical.trim(),
            self.content_hash.trim().to_ascii_lowercase(),
            self.approved,
            self.voter_key_id.trim().to_ascii_lowercase(),
            self.scanner_profile_digest.trim().to_ascii_lowercase(),
            self.evidence_digest.trim().to_ascii_lowercase(),
            self.publisher_authorization_digest
                .trim()
                .to_ascii_lowercase(),
            self.vote_metadata_digest.trim().to_ascii_lowercase(),
            witness_digest,
            semantic_digest,
        )
    }

    pub fn sign_with(&mut self, key: &SigningKey) -> Result<()> {
        if let Some(ref w) = self.witness {
            w.validate_wire_budget()?;
        }
        if let Some(ref sv) = self.semantic_vote {
            sv.validate()?;
        }
        self.voter_key_id = key.key_id();
        let sig = key.sign(self.canonical_vote_v4_message().as_bytes());
        self.signature = hex::encode(sig.ed25519_bytes());
        self.pq_signature = sig.mldsa44_bytes().map(hex::encode);
        Ok(())
    }

    pub fn verify(&self, validator: &Validator, require_hybrid: bool) -> Result<()> {
        if let Some(ref w) = self.witness {
            w.validate_wire_budget()?;
            if w.package_canonical != self.package_canonical || w.content_hash != self.content_hash
            {
                bail!("VoteV4 witness package/content_hash does not match vote envelope");
            }
        }
        if let Some(ref sv) = self.semantic_vote {
            sv.validate()?;
        }
        let pk = validator.verifying_key(require_hybrid)?;
        if !self.voter_key_id.eq_ignore_ascii_case(&pk.key_id()) {
            bail!(
                "VoteV4 voter_key_id {} does not match validator {} key_id {}",
                self.voter_key_id,
                validator.id,
                pk.key_id()
            );
        }
        let ed_sig = decode_exact_lower_or_mixed_hex(
            &self.signature,
            ED25519_SIGNATURE_LEN,
            "VoteV4 Ed25519 signature",
        )?;
        let sig = match (require_hybrid, &self.pq_signature) {
            (false, Some(_)) => bail!("pre-activation VoteV4 must not carry pq_signature"),
            (false, None) => Signature::from_bytes(SignatureScheme::Ed25519, &ed_sig)?,
            (true, None) => bail!("post-activation VoteV4 requires pq_signature"),
            (true, Some(pq_hex)) => {
                if !is_lower_hex_exact(pq_hex, MLDSA44_SIGNATURE_HEX_LEN) {
                    bail!(
                        "VoteV4 pq_signature must be {} lowercase hex chars",
                        MLDSA44_SIGNATURE_HEX_LEN
                    );
                }
                let pq_sig = decode_exact_lower_or_mixed_hex(
                    pq_hex,
                    MLDSA44_SIGNATURE_LEN,
                    "VoteV4 ML-DSA-44 signature",
                )?;
                Signature::hybrid_from_parts(&ed_sig, &pq_sig)?
            }
        };
        pk.verify(
            self.canonical_vote_v4_message().as_bytes(),
            &sig,
            VerifyMode::Strict,
        )
        .context("VoteV4 signature verification failed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnalysisBundleRefs, DeterministicRiskSummary};

    fn hybrid_validator(id: &str, seed_byte: u8) -> (Validator, SigningKey) {
        let ed = [seed_byte; 32];
        let pq = [seed_byte.wrapping_add(100); 32];
        let sk = SigningKey::hybrid_from_seeds(&ed, &pq);
        let pk = sk.public_key();
        let (ed_pk, pq_pk) = pk.hybrid_parts().unwrap();
        let val = Validator {
            id: id.into(),
            alias: id.into(),
            pubkey: hex::encode(ed_pk),
            pq_pubkey: Some(hex::encode(pq_pk)),
            eth_address: format!("0x{:040x}", seed_byte),
            stake: 1_000,
            reputation: 100,
            status: "active".into(),
            ..Default::default()
        };
        (val, sk)
    }

    fn sample_witness(prover_id: &str, prover_key_id: &str) -> TriggerWitnessV1 {
        let mut env = BTreeMap::new();
        env.insert("CI".into(), "true".into());
        TriggerWitnessV1 {
            schema_version: 1,
            package_canonical: "npm:ci-timebomb@1.0.0".into(),
            content_hash: "aa".repeat(32),
            manifest_sha256: "bb".repeat(32),
            sandbox_policy_bundle_id: "policy-v1".into(),
            trigger_spec: TriggerSpecWireV1 {
                schema_version: 1,
                env_overrides: env,
                clock_offset_secs: 7_776_000,
                entry_function: None,
                args: Vec::new(),
            },
            observed_violations: vec!["KB-CANARY-EXFIL".into(), "KB-NET-UNDECLARED".into()],
            graph_canonical_digest: "cc".repeat(32),
            witness_prover_id: prover_id.into(),
            prover_key_id: prover_key_id.into(),
            prover_signature: String::new(),
            prover_pq_signature: None,
        }
    }

    #[test]
    fn trigger_witness_budget_and_hybrid_non_separability() {
        let (val, hybrid_sk) = hybrid_validator("v1", 7);
        let ed_sk = SigningKey::from_ed25519_seed(&[7u8; 32]);

        // Ed25519 witness passes pre-activation, fails post-activation.
        let mut w_ed = sample_witness(&val.id, &ed_sk.key_id());
        w_ed.sign_with(&ed_sk).unwrap();
        assert!(serde_json::to_vec(&w_ed).unwrap().len() <= MAX_TRIGGER_WITNESS_WIRE_BYTES);
        assert!(w_ed.verify(&ed_sk.public_key(), false).is_ok());
        assert!(w_ed.verify(&hybrid_sk.public_key(), true).is_err());

        // Hybrid witness passes post-activation, fails if stripped or checked pre-activation.
        let mut w_hyb = sample_witness(&val.id, &hybrid_sk.key_id());
        w_hyb.sign_with(&hybrid_sk).unwrap();
        let digest1 = w_hyb.canonical_digest();
        assert!(w_hyb.verify(&hybrid_sk.public_key(), true).is_ok());
        assert!(w_hyb.verify(&hybrid_sk.public_key(), false).is_err());

        // Hedged re-signing preserves canonical logical witness digest.
        let mut w_hyb_2 = w_hyb.clone();
        w_hyb_2.sign_with(&hybrid_sk).unwrap();
        assert_eq!(digest1, w_hyb_2.canonical_digest());

        // Stripping PQ half fails non-separably.
        let mut stripped = w_hyb.clone();
        stripped.prover_pq_signature = None;
        assert!(stripped.verify(&hybrid_sk.public_key(), true).is_err());
        assert!(stripped.verify(&ed_sk.public_key(), false).is_err());

        // Core payload > 4 KiB is rejected.
        let mut bloated = w_ed;
        bloated.trigger_spec.args = vec!["A".repeat(900); 6];
        assert!(bloated.validate_wire_budget().is_err());
    }

    #[test]
    fn pq_key_binding_and_vote_v4_envelope_roundtrip() {
        let (val, sk) = hybrid_validator("v-alpha", 11);
        let mut binding = PqKeyBindingV1::sign_with("v-alpha", "creg-testnet-1", &sk).unwrap();
        assert!(binding.verify().is_ok());
        binding.chain_id = "creg-other".into();
        assert!(binding.verify().is_err());

        let mut witness = sample_witness(&val.id, &sk.key_id());
        witness.sign_with(&sk).unwrap();

        let mut vote = VoteV4Envelope {
            package_canonical: "npm:ci-timebomb@1.0.0".into(),
            content_hash: "aa".repeat(32),
            approved: false,
            reject_reason: Some("KB-CANARY-EXFIL".into()),
            validator_id: val.id.clone(),
            voter_key_id: sk.key_id(),
            scanner_profile_digest: "dd".repeat(32),
            evidence_digest: "ee".repeat(32),
            publisher_authorization_digest: "ff".repeat(32),
            vote_metadata_digest: "11".repeat(32),
            ml_model_version: "creg-static-v1".into(),
            analysis_bundles: AnalysisBundleRefs::default(),
            deterministic_risk: DeterministicRiskSummary::default(),
            is_mandatory: true,
            witness: Some(witness),
            semantic_vote: Some(SemanticVoteV4 {
                disposition: SemanticDispositionV4::Reject,
                model_family: "gemini".into(),
                finding_digests: vec!["22".repeat(32)],
            }),
            signature: String::new(),
            pq_signature: None,
        };
        vote.sign_with(&sk).unwrap();
        assert!(vote.verify(&val, true).is_ok());
        assert!(vote.verify(&val, false).is_err());

        // Stripping PQ signature fails strict post-activation & classical verification.
        let mut stripped = vote.clone();
        stripped.pq_signature = None;
        assert!(stripped.verify(&val, true).is_err());
        assert!(stripped.verify(&val, false).is_err());
    }
}
