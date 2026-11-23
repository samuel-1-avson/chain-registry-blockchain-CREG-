//! Versioned namespace-ownership evidence and lifecycle-event schemas.
//!
//! These types intentionally separate *evidence supplied by a registry or
//! trusted-publisher system* from CREG's local controller state. Structural
//! validation is useful at every trust boundary, but it is not cryptographic
//! verification: a node must use an ecosystem authority adapter before it can
//! treat an evidence record as authoritative.

use crate::{
    canonical_publisher_address, is_canonical_single_line, sha256_hex,
    validate_authoritative_namespace, validate_namespace_prefix,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const OWNERSHIP_SCHEMA_VERSION: u16 = 1;
pub const NAMESPACE_AUTHORITY_PROOF_SCHEMA_VERSION: u16 = 2;
pub const NAMESPACE_AUTHORITY_CERTIFICATE_SCHEMA_VERSION: u16 = 2;
/// Maximum lifetime of a claim/recovery statement certified by validators.
pub const MAX_NAMESPACE_AUTHORITY_CERTIFICATE_TTL_SECS: i64 = 15 * 60;
/// Maximum inclusion window for a chain-bound namespace lifecycle event.
pub const MAX_NAMESPACE_LIFECYCLE_EVENT_TTL_SECS: i64 = 24 * 60 * 60;
/// Limits authority certificates to a recent authenticated validator set and
/// prevents a retired committee from creating long-range namespace claims.
pub const MAX_NAMESPACE_AUTHORITY_ANCHOR_LAG_BLOCKS: u64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipAuthority {
    /// npm trusted-publisher OIDC identity plus npm provenance evidence.
    NpmTrustedPublisher,
    /// PyPI trusted-publisher OIDC identity plus PyPI attestation evidence.
    PypiTrustedPublisher,
}

impl OwnershipAuthority {
    pub fn ecosystem(self) -> &'static str {
        match self {
            Self::NpmTrustedPublisher => "npm",
            Self::PypiTrustedPublisher => "pypi",
        }
    }

    pub fn expected_audience(self) -> &'static str {
        match self {
            // npm's OIDC token-exchange audience is registry-qualified.  The
            // shorter historic value (`npm`) is ambiguous and must not be
            // accepted by new authority certificates.
            Self::NpmTrustedPublisher => "npm:registry.npmjs.org",
            Self::PypiTrustedPublisher => "pypi",
        }
    }
}

/// A registry-issued or registry-verified assertion that a CI identity may
/// publish a particular namespace. `evidence_sha256` is the digest of the
/// original OIDC/provenance/attestation bundle stored outside the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamespaceAuthorityProof {
    pub schema_version: u16,
    pub authority: OwnershipAuthority,
    /// `<ecosystem>:<name>@`, shared by all versions of the package.
    pub namespace: String,
    /// OIDC issuer URL asserted by the verified bundle.
    pub issuer: String,
    /// Workload identity, such as a repository/workflow subject.
    pub subject: String,
    /// Registry audience for the exchanged OIDC identity.
    pub audience: String,
    /// Immutable or content-addressed location of the original evidence.
    pub evidence_uri: String,
    /// Official registry location from which the validator fetched the exact
    /// evidence bytes. This may be mutable or eventually unavailable, so the
    /// content-addressed `evidence_uri` remains the replay/audit reference.
    pub registry_evidence_uri: String,
    /// SHA-256 of the exact evidence bytes at `evidence_uri`.
    pub evidence_sha256: String,
    /// Digest of the artifact-independent controller intent embedded in the
    /// registry-published artifact. Keeping the artifact digest out of that
    /// intent avoids an impossible self-hash dependency.
    pub challenge_sha256: String,
    /// Registry-native artifact identity proven by the signed provenance or
    /// attestation envelope (`sha512` for npm, `sha256` for PyPI).
    pub artifact_digest_algorithm: String,
    pub artifact_digest: String,
    /// CREG controller identities that the evidence authorizes.
    pub controller_evm_address: String,
    pub controller_ed25519_pubkey: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl NamespaceAuthorityProof {
    /// Validate only stable schema and binding properties. A successful result
    /// does not establish that OIDC or provenance signatures are genuine.
    pub fn validate_structure(&self) -> Result<(), &'static str> {
        if self.schema_version != NAMESPACE_AUTHORITY_PROOF_SCHEMA_VERSION {
            return Err("unsupported ownership-proof schema version");
        }
        validate_authoritative_namespace(&self.namespace)?;
        if !self
            .namespace
            .starts_with(&format!("{}:", self.authority.ecosystem()))
        {
            return Err("ownership authority does not match namespace ecosystem");
        }
        if !is_canonical_single_line(&self.issuer)
            || !is_canonical_single_line(&self.subject)
            || !is_canonical_single_line(&self.evidence_uri)
            || !is_canonical_single_line(&self.registry_evidence_uri)
        {
            return Err("ownership proof issuer and subject are required");
        }
        if self.audience != self.authority.expected_audience() {
            return Err("ownership proof audience does not match authority");
        }
        if self.evidence_sha256.len() != 64 || self.challenge_sha256.len() != 64 {
            return Err("ownership proof evidence URI and SHA-256 are required");
        }
        if !is_lower_hex_64(&self.evidence_sha256) || !is_lower_hex_64(&self.challenge_sha256) {
            return Err("ownership proof digest must be hexadecimal SHA-256");
        }
        if self.evidence_uri != format!("creg-evidence:sha256:{}", self.evidence_sha256) {
            return Err("ownership proof evidence URI is not content-addressed");
        }
        let (expected_algorithm, artifact_digest_len) = match self.authority {
            OwnershipAuthority::NpmTrustedPublisher => ("sha512", 128),
            OwnershipAuthority::PypiTrustedPublisher => ("sha256", 64),
        };
        if self.artifact_digest_algorithm != expected_algorithm
            || self.artifact_digest.len() != artifact_digest_len
            || self.artifact_digest != self.artifact_digest.to_ascii_lowercase()
            || !self
                .artifact_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("ownership proof artifact identity is invalid");
        }
        let controller_evm_address = canonical_publisher_address(&self.controller_evm_address);
        let valid_evm_address = controller_evm_address.len() == 42
            && controller_evm_address[2..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit());
        if !valid_evm_address
            || self.controller_evm_address != controller_evm_address
            || self.controller_ed25519_pubkey.trim().len() != 64
            || self.controller_ed25519_pubkey != self.controller_ed25519_pubkey.to_ascii_lowercase()
            || !self
                .controller_ed25519_pubkey
                .trim()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("ownership proof controller identities are invalid");
        }
        if self.evidence_sha256 != self.evidence_sha256.to_ascii_lowercase()
            || self.issued_at.timestamp() < 0
            || self.issued_at.timestamp_subsec_nanos() != 0
            || self.expires_at.timestamp_subsec_nanos() != 0
            || self.expires_at <= self.issued_at
        {
            return Err("ownership proof expiry must be after issuance");
        }
        Ok(())
    }

    pub fn digest(&self) -> String {
        // Do not hash serde's timestamp spelling or caller-controlled case.
        // This canonical text is stable across languages and binds every
        // authority-relevant field while treating equivalent hex encodings as
        // the same proof.
        sha256_hex(
            format!(
                "creg-namespace-authority-proof-v2\nservice:creg-package-ownership\nauthority:{}\nnamespace:{}\nissuer:{}\nsubject:{}\naudience:{}\nevidence_uri:{}\nregistry_evidence_uri:{}\nevidence_sha256:{}\nchallenge_sha256:{}\nartifact_digest_algorithm:{}\nartifact_digest:{}\ncontroller_evm_address:{}\ncontroller_ed25519_pubkey:{}\nissued_at:{}\nexpires_at:{}",
                self.authority.ecosystem(),
                self.namespace.trim(),
                self.issuer.trim(),
                self.subject.trim(),
                self.audience.trim(),
                self.evidence_uri.trim(),
                self.registry_evidence_uri.trim(),
                self.evidence_sha256.trim().to_ascii_lowercase(),
                self.challenge_sha256.trim().to_ascii_lowercase(),
                self.artifact_digest_algorithm,
                self.artifact_digest.trim().to_ascii_lowercase(),
                canonical_publisher_address(&self.controller_evm_address),
                self.controller_ed25519_pubkey.trim().to_ascii_lowercase(),
                self.issued_at.timestamp(),
                self.expires_at.timestamp(),
            )
            .as_bytes(),
        )
    }
}

/// What an independently observed upstream authority statement authorizes.
/// A first claim and a recovery have distinct domains so a certificate cannot
/// be replayed between the two state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceAuthorityPurpose {
    Claim,
    Recover,
}

impl NamespaceAuthorityPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::Recover => "recover",
        }
    }
}

/// One validator's signature over a namespace-authority statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamespaceAuthoritySignature {
    pub validator_id: String,
    pub validator_pubkey: String,
    pub signature: String,
}

/// Deterministic result of an npm/PyPI adapter observation. Raw registry
/// traffic and bearer tokens are deliberately excluded. Validators sign this
/// immutable statement only after independently fetching and verifying the
/// evidence out of band.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamespaceAuthorityStatement {
    pub schema_version: u16,
    /// Prevents a proof from being replayed on a sibling deployment.
    pub chain_id: String,
    pub genesis_hash: String,
    pub purpose: NamespaceAuthorityPurpose,
    pub namespace: String,
    #[serde(default)]
    pub current_controller_ed25519_pubkey: String,
    #[serde(default)]
    pub current_controller_evm_address: String,
    pub next_controller_ed25519_pubkey: String,
    pub next_controller_evm_address: String,
    /// Non-zero nonce from the artifact-independent controller intent.
    pub authority_nonce: u64,
    /// SHA-256 of the exact fresh controller intent embedded in the upstream
    /// artifact. The artifact/provenance envelope, not the intent itself,
    /// commits to the artifact digest and thereby avoids a self-hash cycle.
    pub authority_intent_digest: String,
    pub authority_proof: NamespaceAuthorityProof,
    /// Adapter/policy/trust-root identities are consensus data. A verifier
    /// upgrade therefore creates a new certificate rather than silently
    /// changing the meaning of old history.
    pub adapter_id: String,
    pub adapter_version: String,
    pub verifier_sha256: String,
    pub policy_sha256: String,
    pub trust_root_sha256: String,
    /// Exact canonical block observed when validators formed the statement.
    pub anchor_height: u64,
    pub anchor_block_hash: String,
    pub validator_set_hash: String,
    /// Whole Unix seconds avoid sub-second transaction malleability.
    pub issued_at: i64,
    pub expires_at: i64,
}

impl NamespaceAuthorityStatement {
    pub fn signing_message(&self) -> String {
        format!(
            "creg-namespace-authority-certificate-v2\nservice:creg-package-ownership\nchain_id:{}\ngenesis_hash:{}\npurpose:{}\nnamespace:{}\ncurrent_ed25519_pubkey:{}\ncurrent_evm_address:{}\nnext_ed25519_pubkey:{}\nnext_evm_address:{}\nauthority_nonce:{}\nauthority_intent_sha256:{}\nauthority_proof_sha256:{}\nadapter_id:{}\nadapter_version:{}\nverifier_sha256:{}\npolicy_sha256:{}\ntrust_root_sha256:{}\nanchor_height:{}\nanchor_block_hash:{}\nvalidator_set_hash:{}\nissued_at:{}\nexpires_at:{}",
            self.chain_id.trim(),
            self.genesis_hash.trim().to_ascii_lowercase(),
            self.purpose.as_str(),
            self.namespace.trim(),
            self.current_controller_ed25519_pubkey
                .trim()
                .to_ascii_lowercase(),
            canonical_publisher_address(&self.current_controller_evm_address),
            self.next_controller_ed25519_pubkey
                .trim()
                .to_ascii_lowercase(),
            canonical_publisher_address(&self.next_controller_evm_address),
            self.authority_nonce,
            self.authority_intent_digest.trim().to_ascii_lowercase(),
            self.authority_proof.digest(),
            self.adapter_id.trim(),
            self.adapter_version.trim(),
            self.verifier_sha256.trim().to_ascii_lowercase(),
            self.policy_sha256.trim().to_ascii_lowercase(),
            self.trust_root_sha256.trim().to_ascii_lowercase(),
            self.anchor_height,
            self.anchor_block_hash.trim().to_ascii_lowercase(),
            self.validator_set_hash.trim().to_ascii_lowercase(),
            self.issued_at,
            self.expires_at,
        )
    }

    pub fn digest(&self) -> String {
        sha256_hex(self.signing_message().as_bytes())
    }

    pub fn validate_structure(&self) -> Result<(), &'static str> {
        if self.schema_version != NAMESPACE_AUTHORITY_CERTIFICATE_SCHEMA_VERSION {
            return Err("unsupported namespace authority certificate version");
        }
        if !is_canonical_single_line(&self.chain_id) || !is_lower_hex_64(&self.genesis_hash) {
            return Err("namespace authority certificate lacks a chain/genesis binding");
        }
        validate_authoritative_namespace(&self.namespace)?;
        if self.namespace != self.authority_proof.namespace {
            return Err("authority certificate namespace does not match its proof");
        }
        self.authority_proof.validate_structure()?;
        let next_evm = canonical_publisher_address(&self.next_controller_evm_address);
        if !is_hex_ed25519_key(&self.next_controller_ed25519_pubkey)
            || !self
                .next_controller_ed25519_pubkey
                .eq_ignore_ascii_case(&self.authority_proof.controller_ed25519_pubkey)
            || self.next_controller_evm_address != next_evm
            || next_evm != self.authority_proof.controller_evm_address
        {
            return Err("authority certificate successor does not match its proof");
        }
        if matches!(self.purpose, NamespaceAuthorityPurpose::Recover)
            && (!is_hex_ed25519_key(&self.current_controller_ed25519_pubkey)
                || !is_canonical_evm_address(&self.current_controller_evm_address))
        {
            return Err("recovery certificate lacks the current controller");
        }
        if matches!(self.purpose, NamespaceAuthorityPurpose::Claim)
            && (!self.current_controller_ed25519_pubkey.is_empty()
                || !self.current_controller_evm_address.is_empty())
        {
            return Err("first-claim certificate must not name a current controller");
        }
        if !self.current_controller_ed25519_pubkey.is_empty()
            && self.current_controller_ed25519_pubkey
                != self.current_controller_ed25519_pubkey.to_ascii_lowercase()
        {
            return Err("authority certificate current controller is not canonical");
        }
        for digest in [
            &self.authority_intent_digest,
            &self.verifier_sha256,
            &self.policy_sha256,
            &self.trust_root_sha256,
            &self.anchor_block_hash,
            &self.validator_set_hash,
        ] {
            if !is_lower_hex_64(digest) {
                return Err("authority certificate contains a non-canonical digest");
            }
        }
        if self.authority_nonce == 0
            || self.authority_intent_digest != self.authority_proof.challenge_sha256
        {
            return Err("authority certificate intent binding is invalid");
        }
        if self.issued_at < 0
            || self.expires_at <= self.issued_at
            || self.expires_at.saturating_sub(self.issued_at)
                > MAX_NAMESPACE_AUTHORITY_CERTIFICATE_TTL_SECS
            || self.authority_proof.expires_at.timestamp() < self.expires_at
        {
            return Err("authority certificate validity window is invalid");
        }
        if !is_canonical_single_line(&self.adapter_id)
            || !is_canonical_single_line(&self.adapter_version)
            || self.authority_proof.issued_at.timestamp() < self.issued_at.saturating_sub(60)
            || self.authority_proof.issued_at.timestamp() > self.expires_at
        {
            return Err("authority certificate adapter or validity metadata is invalid");
        }
        if self.authority_proof.issued_at.timestamp_subsec_nanos() != 0
            || self.authority_proof.expires_at.timestamp_subsec_nanos() != 0
        {
            return Err("authority proof timestamps must use whole seconds");
        }
        Ok(())
    }
}

/// Quorum certificate produced by independently operated validators. The
/// complete validator set is carried so block replay can verify signatures
/// without a live registry lookup; the enclosing block's authenticated
/// validator-set hash must match it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedNamespaceAuthority {
    pub statement: NamespaceAuthorityStatement,
    pub validators: crate::ValidatorSet,
    #[serde(default)]
    pub signatures: Vec<NamespaceAuthoritySignature>,
}

impl CertifiedNamespaceAuthority {
    pub fn digest(&self) -> String {
        let mut signatures = self.signatures.clone();
        signatures.sort_by(|left, right| {
            left.validator_id
                .cmp(&right.validator_id)
                .then_with(|| left.validator_pubkey.cmp(&right.validator_pubkey))
                .then_with(|| left.signature.cmp(&right.signature))
        });
        let mut canonical = format!(
            "creg-namespace-authority-certificate-digest-v1\nstatement_sha256:{}\nvalidator_set_sha256:{}",
            self.statement.digest(),
            crate::canonical_validator_set_hash(&self.validators),
        );
        for signature in signatures {
            canonical.push_str(&format!(
                "\nsigner:{}:{}:{}",
                signature.validator_id, signature.validator_pubkey, signature.signature
            ));
        }
        sha256_hex(canonical.as_bytes())
    }

    /// Verify the quorum against a validator-set hash authenticated by chain
    /// history. The set carried in the certificate is replay material, not a
    /// self-authenticating trust root.
    pub fn validate_quorum_against_hash(
        &self,
        expected_validator_set_hash: &str,
    ) -> Result<(), &'static str> {
        use crate::crypto::{PublicKey, Signature, SignatureScheme, VerifyMode};
        use std::collections::HashSet;

        self.statement.validate_structure()?;
        if self.validators.validators.is_empty()
            || self.statement.validator_set_hash
                != crate::canonical_validator_set_hash(&self.validators)
            || self.statement.validator_set_hash != expected_validator_set_hash
        {
            return Err("authority certificate validator-set hash is invalid");
        }
        let mut validator_ids = HashSet::new();
        let mut validator_pubkeys = HashSet::new();
        let mut previous_validator: Option<(String, String)> = None;
        for validator in &self.validators.validators {
            let pubkey = validator.pubkey.trim().to_ascii_lowercase();
            if !is_canonical_single_line(&validator.id)
                || !is_hex_ed25519_key(&pubkey)
                || validator.pubkey != pubkey
                || !validator_ids.insert(validator.id.clone())
                || !validator_pubkeys.insert(pubkey.clone())
            {
                return Err("authority certificate validator set is ambiguous");
            }
            let canonical = (validator.id.clone(), pubkey);
            if previous_validator
                .as_ref()
                .is_some_and(|previous| previous >= &canonical)
            {
                return Err("authority certificate validator set is not canonically ordered");
            }
            previous_validator = Some(canonical);
        }
        let message = self.statement.signing_message();
        let mut declared = HashSet::new();
        let mut declared_pubkeys = HashSet::new();
        let mut previous_signer: Option<(String, String, String)> = None;
        for signature in &self.signatures {
            if signature.signature.len() != 128
                || signature.signature != signature.signature.to_ascii_lowercase()
                || !signature
                    .signature
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                return Err("authority certificate signature encoding is not canonical");
            }
            let canonical = (
                signature.validator_id.clone(),
                signature.validator_pubkey.clone(),
                signature.signature.clone(),
            );
            if previous_signer
                .as_ref()
                .is_some_and(|previous| previous >= &canonical)
            {
                return Err("authority certificate signatures are not canonically ordered");
            }
            previous_signer = Some(canonical);
            let Some(validator) = self.validators.validators.iter().find(|validator| {
                validator.id == signature.validator_id
                    && validator.pubkey == signature.validator_pubkey
            }) else {
                return Err("authority certificate signer is not in the authenticated set");
            };
            if !declared.insert(signature.validator_id.clone())
                || !declared_pubkeys.insert(signature.validator_pubkey.clone())
            {
                return Err("authority certificate contains a duplicate signer");
            }
            let Ok(pubkey) = hex::decode(&validator.pubkey) else {
                return Err("authority certificate signer public key is invalid");
            };
            let Ok(signature_bytes) = hex::decode(&signature.signature) else {
                return Err("authority certificate signature encoding is invalid");
            };
            let Ok(pubkey) = PublicKey::from_bytes(SignatureScheme::Ed25519, &pubkey) else {
                return Err("authority certificate signer public key is invalid");
            };
            let Ok(signature) = Signature::from_bytes(SignatureScheme::Ed25519, &signature_bytes)
            else {
                return Err("authority certificate signature is invalid");
            };
            if pubkey
                .verify(message.as_bytes(), &signature, VerifyMode::Strict)
                .is_err()
            {
                return Err("authority certificate signature verification failed");
            }
        }
        let quorum = (2 * self.validators.validators.len()) / 3 + 1;
        if declared.len() < quorum {
            return Err("namespace authority certificate lacks validator quorum");
        }
        Ok(())
    }

    pub fn validate_quorum_against(
        &self,
        expected_validators: &crate::ValidatorSet,
    ) -> Result<(), &'static str> {
        self.validate_quorum_against_hash(&crate::canonical_validator_set_hash(expected_validators))
    }
}

fn is_hex_ed25519_key(value: &str) -> bool {
    value.len() == 64
        && value == value.to_ascii_lowercase()
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && value == value.to_ascii_lowercase()
}

fn is_canonical_evm_address(value: &str) -> bool {
    value.len() == 42
        && value == canonical_publisher_address(value)
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceLifecycleAction {
    Delegate,
    Transfer,
    Recover,
}

/// Signed, append-only lifecycle request. The controller signatures are over
/// [`namespace_lifecycle_message`]; recovery additionally requires a verified
/// authority proof at the node layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamespaceLifecycleEvent {
    pub schema_version: u16,
    pub action: NamespaceLifecycleAction,
    pub namespace: String,
    pub current_controller_ed25519_pubkey: String,
    pub next_controller_ed25519_pubkey: String,
    pub nonce: u64,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub current_controller_signature: String,
    pub next_controller_signature: String,
    /// Required for recovery; may be omitted only for delegate/transfer.
    #[serde(default)]
    pub authority_proof_digest: String,
}

/// Chain-bound lifecycle event used after the authoritative-namespace
/// activation height. Legacy v1 events remain decodable for historical replay
/// but must not be admitted as new authoritative state transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamespaceLifecycleEventV2 {
    pub schema_version: u16,
    pub chain_id: String,
    pub genesis_hash: String,
    pub action: NamespaceLifecycleAction,
    pub namespace: String,
    pub current_controller_ed25519_pubkey: String,
    pub current_controller_evm_address: String,
    pub next_controller_ed25519_pubkey: String,
    pub next_controller_evm_address: String,
    /// Delegation is scoped and time-bounded; it never replaces the root
    /// controller. Schema v2 permits only the `publish` scope.
    #[serde(default)]
    pub delegation_scopes: Vec<String>,
    #[serde(default)]
    pub delegation_expires_at: Option<i64>,
    pub nonce: u64,
    /// Whole Unix seconds; sub-second values are intentionally unrepresentable.
    pub issued_at: i64,
    pub expires_at: i64,
    pub current_controller_signature: String,
    pub next_controller_signature: String,
    #[serde(default)]
    pub authority_proof_digest: String,
    /// Required only for recovery. The event signs the proof digest while the
    /// quorum certificate signs the complete event authorization digest,
    /// avoiding a circular signature dependency.
    #[serde(default)]
    pub authority_certificate: Option<CertifiedNamespaceAuthority>,
}

impl NamespaceLifecycleEventV2 {
    pub fn signing_message(&self) -> String {
        namespace_lifecycle_message_v2(
            &self.chain_id,
            &self.genesis_hash,
            self.action,
            &self.namespace,
            &self.current_controller_ed25519_pubkey,
            &self.current_controller_evm_address,
            &self.next_controller_ed25519_pubkey,
            &self.next_controller_evm_address,
            &self.delegation_scopes,
            self.delegation_expires_at,
            self.nonce,
            self.issued_at,
            self.expires_at,
            &self.authority_proof_digest,
            &self
                .authority_certificate
                .as_ref()
                .map(CertifiedNamespaceAuthority::digest)
                .unwrap_or_default(),
        )
    }

    pub fn authorization_digest(&self) -> String {
        sha256_hex(self.signing_message().as_bytes())
    }

    pub fn authority_intent_message(&self) -> String {
        namespace_authority_intent_message_v1(
            &self.chain_id,
            &self.genesis_hash,
            NamespaceAuthorityPurpose::Recover,
            &self.namespace,
            &self.current_controller_ed25519_pubkey,
            &self.current_controller_evm_address,
            &self.next_controller_ed25519_pubkey,
            &self.next_controller_evm_address,
            self.nonce,
            self.issued_at,
            self.expires_at,
        )
    }

    pub fn authority_intent_digest(&self) -> String {
        sha256_hex(self.authority_intent_message().as_bytes())
    }

    pub fn validate_structure(&self) -> Result<(), &'static str> {
        if self.schema_version != 2 {
            return Err("unsupported namespace lifecycle v2 schema version");
        }
        if !is_canonical_single_line(&self.chain_id) || !is_lower_hex_64(&self.genesis_hash) {
            return Err("namespace lifecycle v2 lacks a chain/genesis binding");
        }
        validate_authoritative_namespace(&self.namespace)?;
        if !is_hex_ed25519_key(&self.current_controller_ed25519_pubkey)
            || !is_hex_ed25519_key(&self.next_controller_ed25519_pubkey)
        {
            return Err("namespace lifecycle v2 controller key must be hex Ed25519");
        }
        if self
            .current_controller_ed25519_pubkey
            .eq_ignore_ascii_case(&self.next_controller_ed25519_pubkey)
        {
            return Err("namespace lifecycle v2 must change the controller");
        }
        if !self.current_controller_evm_address.is_empty()
            && !is_canonical_evm_address(&self.current_controller_evm_address)
        {
            return Err("namespace lifecycle v2 current EVM address is invalid");
        }
        if matches!(self.action, NamespaceLifecycleAction::Recover)
            && self.current_controller_evm_address.is_empty()
        {
            return Err("namespace recovery v2 requires the current EVM controller");
        }
        let next_evm = canonical_publisher_address(&self.next_controller_evm_address);
        if next_evm != self.next_controller_evm_address
            || next_evm.len() != 42
            || !next_evm[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("namespace lifecycle v2 successor EVM address is invalid");
        }
        if self.issued_at < 0
            || self.expires_at <= self.issued_at
            || self.expires_at.saturating_sub(self.issued_at)
                > MAX_NAMESPACE_LIFECYCLE_EVENT_TTL_SECS
        {
            return Err("namespace lifecycle v2 expiry must be after issuance");
        }
        match self.action {
            NamespaceLifecycleAction::Recover => {
                if !is_lower_hex_64(&self.authority_proof_digest) {
                    return Err("namespace recovery v2 requires an authority proof digest");
                }
                let certificate = self
                    .authority_certificate
                    .as_ref()
                    .ok_or("namespace recovery v2 requires an authority certificate")?;
                certificate.statement.validate_structure()?;
                if !matches!(
                    certificate.statement.purpose,
                    NamespaceAuthorityPurpose::Recover
                ) || !self.current_controller_signature.is_empty()
                    || !self.delegation_scopes.is_empty()
                    || self.delegation_expires_at.is_some()
                    || certificate.statement.chain_id != self.chain_id
                    || certificate.statement.genesis_hash != self.genesis_hash
                    || self.issued_at != certificate.statement.issued_at
                    || self.expires_at != certificate.statement.expires_at
                    || certificate.statement.namespace != self.namespace
                    || certificate.statement.authority_nonce != self.nonce
                    || !certificate
                        .statement
                        .current_controller_ed25519_pubkey
                        .eq_ignore_ascii_case(&self.current_controller_ed25519_pubkey)
                    || !canonical_publisher_address(
                        &certificate.statement.current_controller_evm_address,
                    )
                    .eq_ignore_ascii_case(&self.current_controller_evm_address)
                    || !certificate
                        .statement
                        .next_controller_ed25519_pubkey
                        .eq_ignore_ascii_case(&self.next_controller_ed25519_pubkey)
                    || !canonical_publisher_address(
                        &certificate.statement.next_controller_evm_address,
                    )
                    .eq_ignore_ascii_case(&next_evm)
                    || certificate.statement.authority_intent_digest
                        != self.authority_intent_digest()
                    || certificate.statement.authority_proof.digest() != self.authority_proof_digest
                {
                    return Err("namespace recovery v2 certificate does not bind the event");
                }
            }
            NamespaceLifecycleAction::Delegate => {
                if !self.authority_proof_digest.is_empty() || self.authority_certificate.is_some() {
                    return Err("non-recovery lifecycle v2 must not carry authority evidence");
                }
                if self.delegation_scopes != ["publish"]
                    || self
                        .delegation_expires_at
                        .is_none_or(|expiry| expiry <= self.issued_at)
                {
                    return Err("namespace delegation v2 must be a time-bounded publish scope");
                }
            }
            NamespaceLifecycleAction::Transfer => {
                if !self.authority_proof_digest.is_empty()
                    || self.authority_certificate.is_some()
                    || !self.delegation_scopes.is_empty()
                    || self.delegation_expires_at.is_some()
                {
                    return Err(
                        "namespace transfer v2 must not carry delegation or authority data",
                    );
                }
            }
        }
        Ok(())
    }

    pub fn is_valid_at(&self, reference_time: i64) -> bool {
        self.issued_at <= reference_time && reference_time < self.expires_at
    }

    pub fn verify_controller_signatures(&self) -> Result<(), &'static str> {
        use crate::crypto::{PublicKey, Signature, SignatureScheme, VerifyMode};

        self.validate_structure()?;
        let message = self.signing_message();
        let verify = |pubkey: &str, signature: &str| -> Result<(), &'static str> {
            let pubkey = hex::decode(pubkey).map_err(|_| "invalid controller public key")?;
            let signature =
                hex::decode(signature).map_err(|_| "invalid controller signature encoding")?;
            let key = PublicKey::from_bytes(SignatureScheme::Ed25519, &pubkey)
                .map_err(|_| "invalid controller public key")?;
            let signature = Signature::from_bytes(SignatureScheme::Ed25519, &signature)
                .map_err(|_| "invalid controller signature")?;
            key.verify(message.as_bytes(), &signature, VerifyMode::Strict)
                .map_err(|_| "controller signature verification failed")
        };

        if !matches!(self.action, NamespaceLifecycleAction::Recover) {
            verify(
                &self.current_controller_ed25519_pubkey,
                &self.current_controller_signature,
            )?;
        }
        verify(
            &self.next_controller_ed25519_pubkey,
            &self.next_controller_signature,
        )
    }
}

impl NamespaceLifecycleEvent {
    pub fn signing_message(&self) -> String {
        namespace_lifecycle_message(
            self.action,
            &self.namespace,
            &self.current_controller_ed25519_pubkey,
            &self.next_controller_ed25519_pubkey,
            self.nonce,
            self.issued_at.timestamp(),
            self.expires_at.timestamp(),
            &self.authority_proof_digest,
        )
    }

    pub fn validate_structure(&self) -> Result<(), &'static str> {
        if self.schema_version != OWNERSHIP_SCHEMA_VERSION {
            return Err("unsupported namespace lifecycle schema version");
        }
        validate_namespace_prefix(&self.namespace)?;
        for key in [
            &self.current_controller_ed25519_pubkey,
            &self.next_controller_ed25519_pubkey,
        ] {
            if key.trim().len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err("namespace lifecycle controller key must be hex Ed25519");
            }
        }
        if self
            .current_controller_ed25519_pubkey
            .eq_ignore_ascii_case(&self.next_controller_ed25519_pubkey)
        {
            return Err("namespace lifecycle event must change the controller");
        }
        if self.expires_at <= self.issued_at
            || self.expires_at.signed_duration_since(self.issued_at)
                > chrono::Duration::seconds(MAX_NAMESPACE_LIFECYCLE_EVENT_TTL_SECS)
        {
            return Err("namespace lifecycle expiry must be after issuance");
        }
        match self.action {
            NamespaceLifecycleAction::Recover if self.authority_proof_digest.len() != 64 => {
                return Err("namespace recovery requires an authority proof digest")
            }
            NamespaceLifecycleAction::Delegate | NamespaceLifecycleAction::Transfer
                if !self.authority_proof_digest.is_empty() =>
            {
                return Err("delegate and transfer events must not carry a recovery proof digest")
            }
            _ => {}
        }
        Ok(())
    }

    /// Whether an event was live at a deterministic chain-time reference.
    /// Block replay must use the block timestamp rather than a validator's
    /// wall clock, otherwise an old event could be accepted differently by
    /// different nodes after it has expired.
    pub fn is_valid_at(&self, reference_time: DateTime<Utc>) -> bool {
        self.issued_at <= reference_time && reference_time < self.expires_at
    }

    /// Verify lifecycle controller signatures over the same canonical request.
    /// Delegate/transfer require both controllers. Recovery deliberately does
    /// not require the old controller: it instead requires the successor
    /// signature plus a separately verified authority proof before application.
    pub fn verify_controller_signatures(&self) -> Result<(), &'static str> {
        use crate::crypto::{PublicKey, Signature, SignatureScheme, VerifyMode};

        self.validate_structure()?;
        let message = self.signing_message();
        let verify = |pubkey: &str, signature: &str| -> Result<(), &'static str> {
            let pubkey = hex::decode(pubkey).map_err(|_| "invalid controller public key")?;
            let signature =
                hex::decode(signature).map_err(|_| "invalid controller signature encoding")?;
            let key = PublicKey::from_bytes(SignatureScheme::Ed25519, &pubkey)
                .map_err(|_| "invalid controller public key")?;
            let signature = Signature::from_bytes(SignatureScheme::Ed25519, &signature)
                .map_err(|_| "invalid controller signature")?;
            key.verify(message.as_bytes(), &signature, VerifyMode::Strict)
                .map_err(|_| "controller signature verification failed")
        };

        if !matches!(self.action, NamespaceLifecycleAction::Recover) {
            verify(
                &self.current_controller_ed25519_pubkey,
                &self.current_controller_signature,
            )?;
        }
        verify(
            &self.next_controller_ed25519_pubkey,
            &self.next_controller_signature,
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub fn namespace_lifecycle_message(
    action: NamespaceLifecycleAction,
    namespace: &str,
    current_pubkey: &str,
    next_pubkey: &str,
    nonce: u64,
    issued_at: i64,
    expires_at: i64,
    authority_proof_digest: &str,
) -> String {
    let action = match action {
        NamespaceLifecycleAction::Delegate => "delegate",
        NamespaceLifecycleAction::Transfer => "transfer",
        NamespaceLifecycleAction::Recover => "recover",
    };
    format!(
        "creg-namespace-lifecycle-v1\nservice:creg-package-ownership\naction:{}\nnamespace:{}\ncurrent_ed25519_pubkey:{}\nnext_ed25519_pubkey:{}\nnonce:{}\nissued_at:{}\nexpires_at:{}\nauthority_proof_sha256:{}",
        action,
        namespace.trim(),
        current_pubkey.trim().to_ascii_lowercase(),
        next_pubkey.trim().to_ascii_lowercase(),
        nonce,
        issued_at,
        expires_at,
        authority_proof_digest.trim().to_ascii_lowercase(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn namespace_lifecycle_message_v2(
    chain_id: &str,
    genesis_hash: &str,
    action: NamespaceLifecycleAction,
    namespace: &str,
    current_pubkey: &str,
    current_evm_address: &str,
    next_pubkey: &str,
    next_evm_address: &str,
    delegation_scopes: &[String],
    delegation_expires_at: Option<i64>,
    nonce: u64,
    issued_at: i64,
    expires_at: i64,
    authority_proof_digest: &str,
    authority_certificate_digest: &str,
) -> String {
    let action = match action {
        NamespaceLifecycleAction::Delegate => "delegate",
        NamespaceLifecycleAction::Transfer => "transfer",
        NamespaceLifecycleAction::Recover => "recover",
    };
    format!(
        "creg-namespace-lifecycle-v2\nservice:creg-package-ownership\nchain_id:{}\ngenesis_hash:{}\naction:{}\nnamespace:{}\ncurrent_ed25519_pubkey:{}\ncurrent_evm_address:{}\nnext_ed25519_pubkey:{}\nnext_evm_address:{}\ndelegation_scopes:{}\ndelegation_expires_at:{}\nnonce:{}\nissued_at:{}\nexpires_at:{}\nauthority_proof_sha256:{}\nauthority_certificate_sha256:{}",
        chain_id.trim(),
        genesis_hash.trim().to_ascii_lowercase(),
        action,
        namespace.trim(),
        current_pubkey.trim().to_ascii_lowercase(),
        canonical_publisher_address(current_evm_address),
        next_pubkey.trim().to_ascii_lowercase(),
        canonical_publisher_address(next_evm_address),
        delegation_scopes.join(","),
        delegation_expires_at.unwrap_or(0),
        nonce,
        issued_at,
        expires_at,
        authority_proof_digest.trim().to_ascii_lowercase(),
        authority_certificate_digest.trim().to_ascii_lowercase(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn namespace_authority_intent_message_v1(
    chain_id: &str,
    genesis_hash: &str,
    purpose: NamespaceAuthorityPurpose,
    namespace: &str,
    current_pubkey: &str,
    current_evm_address: &str,
    next_pubkey: &str,
    next_evm_address: &str,
    nonce: u64,
    issued_at: i64,
    expires_at: i64,
) -> String {
    let document = match purpose {
        NamespaceAuthorityPurpose::Claim => "creg-claim.json",
        NamespaceAuthorityPurpose::Recover => "creg-recovery.json",
    };
    format!(
        "creg-namespace-authority-intent-v1\nservice:creg-package-ownership\npurpose:{}\ndocument:{}\nchain_id:{}\ngenesis_hash:{}\nnamespace:{}\ncurrent_ed25519_pubkey:{}\ncurrent_evm_address:{}\nnext_ed25519_pubkey:{}\nnext_evm_address:{}\nnonce:{}\nissued_at:{}\nexpires_at:{}",
        purpose.as_str(),
        document,
        chain_id.trim(),
        genesis_hash.trim().to_ascii_lowercase(),
        namespace.trim(),
        current_pubkey.trim().to_ascii_lowercase(),
        canonical_publisher_address(current_evm_address),
        next_pubkey.trim().to_ascii_lowercase(),
        canonical_publisher_address(next_evm_address),
        nonce,
        issued_at,
        expires_at,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    fn proof_for_controller(
        controller_ed25519_pubkey: String,
        controller_evm_address: &str,
    ) -> NamespaceAuthorityProof {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        NamespaceAuthorityProof {
            schema_version: NAMESPACE_AUTHORITY_PROOF_SCHEMA_VERSION,
            authority: OwnershipAuthority::NpmTrustedPublisher,
            namespace: "npm:owned-package@".into(),
            issuer: "https://token.actions.githubusercontent.com".into(),
            subject: "repo:creg/owned-package:ref:refs/tags/v1.0.0".into(),
            audience: "npm:registry.npmjs.org".into(),
            evidence_uri: format!("creg-evidence:sha256:{}", "a".repeat(64)),
            registry_evidence_uri:
                "https://registry.npmjs.org/-/npm/v1/attestations/owned-package@1.0.0".into(),
            evidence_sha256: "a".repeat(64),
            challenge_sha256: "c".repeat(64),
            artifact_digest_algorithm: "sha512".into(),
            artifact_digest: "d".repeat(128),
            controller_evm_address: controller_evm_address.into(),
            controller_ed25519_pubkey,
            issued_at: now,
            expires_at: now + Duration::hours(1),
        }
    }

    fn proof() -> NamespaceAuthorityProof {
        proof_for_controller("b".repeat(64), "0x1111111111111111111111111111111111111111")
    }

    fn validator_fixture() -> (Vec<SigningKey>, crate::ValidatorSet) {
        let keys = (0..4)
            .map(|_| SigningKey::generate(&mut OsRng))
            .collect::<Vec<_>>();
        let validators = crate::ValidatorSet::new(
            keys.iter()
                .enumerate()
                .map(|(index, key)| crate::Validator {
                    id: format!("validator-{index}"),
                    pubkey: hex::encode(key.verifying_key().as_bytes()),
                    ..Default::default()
                })
                .collect(),
        );
        (keys, validators)
    }

    fn authority_statement(
        purpose: NamespaceAuthorityPurpose,
        authority_proof: NamespaceAuthorityProof,
        validators: &crate::ValidatorSet,
        current_ed25519_pubkey: &str,
        current_evm_address: &str,
        authority_nonce: u64,
    ) -> NamespaceAuthorityStatement {
        NamespaceAuthorityStatement {
            schema_version: NAMESPACE_AUTHORITY_CERTIFICATE_SCHEMA_VERSION,
            chain_id: "creg-testnet-1".into(),
            genesis_hash: "1".repeat(64),
            purpose,
            namespace: authority_proof.namespace.clone(),
            current_controller_ed25519_pubkey: current_ed25519_pubkey.into(),
            current_controller_evm_address: current_evm_address.into(),
            next_controller_ed25519_pubkey: authority_proof.controller_ed25519_pubkey.clone(),
            next_controller_evm_address: authority_proof.controller_evm_address.clone(),
            authority_nonce,
            authority_intent_digest: authority_proof.challenge_sha256.clone(),
            authority_proof,
            adapter_id: "npm-provenance".into(),
            adapter_version: "1.0.0".into(),
            verifier_sha256: "2".repeat(64),
            policy_sha256: "3".repeat(64),
            trust_root_sha256: "4".repeat(64),
            anchor_height: 42,
            anchor_block_hash: "5".repeat(64),
            validator_set_hash: crate::canonical_validator_set_hash(validators),
            issued_at: 1_700_000_000,
            expires_at: 1_700_000_600,
        }
    }

    fn certificate(
        statement: NamespaceAuthorityStatement,
        keys: &[SigningKey],
        validators: &crate::ValidatorSet,
        signer_count: usize,
    ) -> CertifiedNamespaceAuthority {
        let message = statement.signing_message();
        let signatures = keys
            .iter()
            .zip(&validators.validators)
            .take(signer_count)
            .map(|(key, validator)| NamespaceAuthoritySignature {
                validator_id: validator.id.clone(),
                validator_pubkey: validator.pubkey.clone(),
                signature: hex::encode(key.sign(message.as_bytes()).to_bytes()),
            })
            .collect();
        CertifiedNamespaceAuthority {
            statement,
            validators: validators.clone(),
            signatures,
        }
    }

    fn signed_lifecycle_event(
        mut event: NamespaceLifecycleEventV2,
        current: Option<&SigningKey>,
        next: &SigningKey,
    ) -> NamespaceLifecycleEventV2 {
        let message = event.signing_message();
        event.current_controller_signature = current
            .map(|key| hex::encode(key.sign(message.as_bytes()).to_bytes()))
            .unwrap_or_default();
        event.next_controller_signature = hex::encode(next.sign(message.as_bytes()).to_bytes());
        event
    }

    #[test]
    fn authority_proof_rejects_cross_ecosystem_and_wrong_audience() {
        let mut value = proof();
        assert!(value.validate_structure().is_ok());
        value.namespace = "pypi:owned-package@".into();
        assert!(value.validate_structure().is_err());
        value.namespace = "npm:owned-package@".into();
        value.audience = "pypi".into();
        assert!(value.validate_structure().is_err());
        value.audience = "npm:registry.npmjs.org".into();
        value.controller_evm_address = "0xnot-an-address".into();
        assert!(value.validate_structure().is_err());
    }

    #[test]
    fn authority_proof_binds_registry_and_content_addressed_evidence() {
        let value = proof();
        assert!(value.validate_structure().is_ok());
        let digest = value.digest();

        let mut changed = value.clone();
        changed.registry_evidence_uri.push_str("?mirror=other");
        assert_ne!(digest, changed.digest());

        let mut changed = value.clone();
        changed.evidence_uri = format!("creg-evidence:sha256:{}", "b".repeat(64));
        assert_ne!(digest, changed.digest());
        assert!(changed.validate_structure().is_err());

        let mut malformed = value.clone();
        malformed.evidence_uri = "https://mutable.example/evidence.json".into();
        assert!(malformed.validate_structure().is_err());

        let mut malformed = value.clone();
        malformed.registry_evidence_uri.push_str("\r\ninjected");
        assert!(malformed.validate_structure().is_err());

        let mut malformed = value.clone();
        malformed.evidence_sha256 = "A".repeat(64);
        assert!(malformed.validate_structure().is_err());

        let mut malformed = value;
        malformed.issued_at =
            DateTime::from_timestamp(1_700_000_000, 1).expect("valid test timestamp");
        assert!(malformed.validate_structure().is_err());
    }

    #[test]
    fn authority_proof_enforces_registry_native_artifact_identity() {
        let mut npm = proof();
        npm.artifact_digest_algorithm = "sha256".into();
        npm.artifact_digest = "d".repeat(64);
        assert!(npm.validate_structure().is_err());

        let mut pypi = proof();
        pypi.authority = OwnershipAuthority::PypiTrustedPublisher;
        pypi.namespace = "pypi:owned-package@".into();
        pypi.audience = "pypi".into();
        pypi.registry_evidence_uri =
            "https://pypi.org/integrity/owned-package/1.0.0/owned-package.whl/provenance".into();
        pypi.artifact_digest_algorithm = "sha256".into();
        pypi.artifact_digest = "d".repeat(64);
        assert!(pypi.validate_structure().is_ok());
    }

    #[test]
    fn authority_certificate_quorum_is_pinned_to_authenticated_set() {
        let (keys, validators) = validator_fixture();
        let statement = authority_statement(
            NamespaceAuthorityPurpose::Claim,
            proof(),
            &validators,
            "",
            "",
            7,
        );
        let trusted_hash = crate::canonical_validator_set_hash(&validators);
        let certified = certificate(statement.clone(), &keys, &validators, 3);
        assert!(certified.validate_quorum_against(&validators).is_ok());
        assert!(certified
            .validate_quorum_against_hash(&trusted_hash)
            .is_ok());

        let mut unknown_certificate = serde_json::to_value(&certified).unwrap();
        unknown_certificate["unsigned_extension"] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<CertifiedNamespaceAuthority>(unknown_certificate).is_err()
        );

        let mut unknown_statement = serde_json::to_value(&certified).unwrap();
        unknown_statement["statement"]["unsigned_extension"] = serde_json::json!(true);
        assert!(serde_json::from_value::<CertifiedNamespaceAuthority>(unknown_statement).is_err());

        let mut unknown_proof = serde_json::to_value(&certified).unwrap();
        unknown_proof["statement"]["authority_proof"]["unsigned_extension"] =
            serde_json::json!(true);
        assert!(serde_json::from_value::<CertifiedNamespaceAuthority>(unknown_proof).is_err());

        let under_quorum = certificate(statement, &keys, &validators, 2);
        assert_eq!(
            under_quorum.validate_quorum_against_hash(&trusted_hash),
            Err("namespace authority certificate lacks validator quorum")
        );

        let (rogue_keys, rogue_validators) = validator_fixture();
        let rogue_statement = authority_statement(
            NamespaceAuthorityPurpose::Claim,
            proof(),
            &rogue_validators,
            "",
            "",
            7,
        );
        let rogue = certificate(rogue_statement, &rogue_keys, &rogue_validators, 3);
        assert_eq!(
            rogue.validate_quorum_against_hash(&trusted_hash),
            Err("authority certificate validator-set hash is invalid")
        );
    }

    #[test]
    fn authority_certificate_requires_bounded_validity_covered_by_its_proof() {
        let (_, validators) = validator_fixture();
        let statement = authority_statement(
            NamespaceAuthorityPurpose::Claim,
            proof(),
            &validators,
            "",
            "",
            7,
        );
        assert!(statement.validate_structure().is_ok());

        let mut exact_limit = statement.clone();
        exact_limit.expires_at =
            exact_limit.issued_at + MAX_NAMESPACE_AUTHORITY_CERTIFICATE_TTL_SECS;
        assert!(exact_limit.validate_structure().is_ok());

        let mut too_long = statement.clone();
        too_long.expires_at = too_long.issued_at + MAX_NAMESPACE_AUTHORITY_CERTIFICATE_TTL_SECS + 1;
        assert_eq!(
            too_long.validate_structure(),
            Err("authority certificate validity window is invalid")
        );

        let mut proof_expires_first = statement;
        proof_expires_first.authority_proof.expires_at =
            DateTime::from_timestamp(proof_expires_first.expires_at - 1, 0)
                .expect("valid proof expiry");
        assert_eq!(
            proof_expires_first.validate_structure(),
            Err("authority certificate validity window is invalid")
        );
    }

    #[test]
    fn authority_certificate_rejects_tampering_and_ambiguous_signers() {
        let (keys, validators) = validator_fixture();
        let statement = authority_statement(
            NamespaceAuthorityPurpose::Claim,
            proof(),
            &validators,
            "",
            "",
            7,
        );
        let trusted_hash = crate::canonical_validator_set_hash(&validators);
        let certified = certificate(statement, &keys, &validators, 3);

        let mut tampered = certified.clone();
        tampered
            .statement
            .authority_proof
            .registry_evidence_uri
            .push_str("?different=true");
        assert_eq!(
            tampered.validate_quorum_against_hash(&trusted_hash),
            Err("authority certificate signature verification failed")
        );

        let mut duplicate = certified.clone();
        duplicate
            .signatures
            .insert(1, duplicate.signatures[0].clone());
        assert!(duplicate
            .validate_quorum_against_hash(&trusted_hash)
            .is_err());

        let mut reordered = certified;
        reordered.signatures.swap(0, 1);
        assert_eq!(
            reordered.validate_quorum_against_hash(&trusted_hash),
            Err("authority certificate signatures are not canonically ordered")
        );
    }

    #[test]
    fn lifecycle_v2_signatures_bind_chain_scope_and_expiry() {
        let current = SigningKey::generate(&mut OsRng);
        let next = SigningKey::generate(&mut OsRng);
        let event = NamespaceLifecycleEventV2 {
            schema_version: 2,
            chain_id: "creg-testnet-1".into(),
            genesis_hash: "1".repeat(64),
            action: NamespaceLifecycleAction::Delegate,
            namespace: "npm:owned-package@".into(),
            current_controller_ed25519_pubkey: hex::encode(current.verifying_key().as_bytes()),
            current_controller_evm_address: "0x1111111111111111111111111111111111111111".into(),
            next_controller_ed25519_pubkey: hex::encode(next.verifying_key().as_bytes()),
            next_controller_evm_address: "0x2222222222222222222222222222222222222222".into(),
            delegation_scopes: vec!["publish".into()],
            delegation_expires_at: Some(1_700_001_800),
            nonce: 1,
            issued_at: 1_700_000_000,
            expires_at: 1_700_000_600,
            current_controller_signature: String::new(),
            next_controller_signature: String::new(),
            authority_proof_digest: String::new(),
            authority_certificate: None,
        };
        let event = signed_lifecycle_event(event, Some(&current), &next);
        assert!(event.verify_controller_signatures().is_ok());

        let mut exact_limit = event.clone();
        exact_limit.expires_at = exact_limit.issued_at + MAX_NAMESPACE_LIFECYCLE_EVENT_TTL_SECS;
        assert!(exact_limit.validate_structure().is_ok());

        let mut too_long = event.clone();
        too_long.expires_at = too_long.issued_at + MAX_NAMESPACE_LIFECYCLE_EVENT_TTL_SECS + 1;
        assert_eq!(
            too_long.validate_structure(),
            Err("namespace lifecycle v2 expiry must be after issuance")
        );

        let mut unknown = serde_json::to_value(&event).unwrap();
        unknown["unsigned_extension"] = serde_json::json!("must-reject");
        assert!(serde_json::from_value::<NamespaceLifecycleEventV2>(unknown).is_err());

        let mut cross_chain = event.clone();
        cross_chain.genesis_hash = "2".repeat(64);
        assert!(cross_chain.verify_controller_signatures().is_err());

        let mut changed_expiry = event.clone();
        changed_expiry.delegation_expires_at = Some(1_700_001_801);
        assert!(changed_expiry.verify_controller_signatures().is_err());

        let mut invalid_scope = event;
        invalid_scope.delegation_scopes = vec!["admin".into()];
        assert_eq!(
            invalid_scope.validate_structure(),
            Err("namespace delegation v2 must be a time-bounded publish scope")
        );
    }

    #[test]
    fn recovery_v2_requires_certificate_bound_to_exact_intent() {
        let current = SigningKey::generate(&mut OsRng);
        let next = SigningKey::generate(&mut OsRng);
        let current_pubkey = hex::encode(current.verifying_key().as_bytes());
        let next_pubkey = hex::encode(next.verifying_key().as_bytes());
        let current_evm = "0x1111111111111111111111111111111111111111";
        let next_evm = "0x2222222222222222222222222222222222222222";
        let mut event = NamespaceLifecycleEventV2 {
            schema_version: 2,
            chain_id: "creg-testnet-1".into(),
            genesis_hash: "1".repeat(64),
            action: NamespaceLifecycleAction::Recover,
            namespace: "npm:owned-package@".into(),
            current_controller_ed25519_pubkey: current_pubkey.clone(),
            current_controller_evm_address: current_evm.into(),
            next_controller_ed25519_pubkey: next_pubkey.clone(),
            next_controller_evm_address: next_evm.into(),
            delegation_scopes: Vec::new(),
            delegation_expires_at: None,
            nonce: 9,
            issued_at: 1_700_000_000,
            expires_at: 1_700_000_600,
            current_controller_signature: String::new(),
            next_controller_signature: String::new(),
            authority_proof_digest: String::new(),
            authority_certificate: None,
        };
        let mut authority_proof = proof_for_controller(next_pubkey, next_evm);
        authority_proof.challenge_sha256 = event.authority_intent_digest();
        let (keys, validators) = validator_fixture();
        let statement = authority_statement(
            NamespaceAuthorityPurpose::Recover,
            authority_proof,
            &validators,
            &current_pubkey,
            current_evm,
            event.nonce,
        );
        let certificate = certificate(statement, &keys, &validators, 3);
        event.authority_proof_digest = certificate.statement.authority_proof.digest();
        event.authority_certificate = Some(certificate);
        let event = signed_lifecycle_event(event, None, &next);
        assert!(event.verify_controller_signatures().is_ok());
        assert!(event
            .authority_certificate
            .as_ref()
            .expect("certificate")
            .validate_quorum_against(&validators)
            .is_ok());

        let mut missing = event.clone();
        missing.authority_certificate = None;
        assert_eq!(
            missing.validate_structure(),
            Err("namespace recovery v2 requires an authority certificate")
        );

        let mut replayed = event.clone();
        replayed.nonce += 1;
        assert_eq!(
            replayed.validate_structure(),
            Err("namespace recovery v2 certificate does not bind the event")
        );

        let mut conflicting_chain = event.clone();
        conflicting_chain.chain_id = "creg-othernet-1".into();
        assert_eq!(
            conflicting_chain.validate_structure(),
            Err("namespace recovery v2 certificate does not bind the event")
        );

        let mut old_controller_signature = event;
        old_controller_signature.current_controller_signature = "a".repeat(128);
        assert_eq!(
            old_controller_signature.validate_structure(),
            Err("namespace recovery v2 certificate does not bind the event")
        );
    }

    #[test]
    fn lifecycle_message_binds_namespace_nonce_and_recovery_proof() {
        let now = Utc::now();
        let current = SigningKey::generate(&mut OsRng);
        let next = SigningKey::generate(&mut OsRng);
        let event = NamespaceLifecycleEvent {
            schema_version: OWNERSHIP_SCHEMA_VERSION,
            action: NamespaceLifecycleAction::Recover,
            namespace: "npm:owned-package@".into(),
            current_controller_ed25519_pubkey: hex::encode(current.verifying_key().as_bytes()),
            next_controller_ed25519_pubkey: hex::encode(next.verifying_key().as_bytes()),
            nonce: 4,
            issued_at: now,
            expires_at: now + Duration::hours(1),
            current_controller_signature: String::new(),
            next_controller_signature: String::new(),
            authority_proof_digest: "e".repeat(64),
        };
        let mut event = event;
        let message = event.signing_message();
        event.current_controller_signature =
            hex::encode(current.sign(message.as_bytes()).to_bytes());
        event.next_controller_signature = hex::encode(next.sign(message.as_bytes()).to_bytes());
        assert!(event.verify_controller_signatures().is_ok());
        let first = event.signing_message();
        let changed_nonce = namespace_lifecycle_message(
            event.action,
            &event.namespace,
            &event.current_controller_ed25519_pubkey,
            &event.next_controller_ed25519_pubkey,
            5,
            event.issued_at.timestamp(),
            event.expires_at.timestamp(),
            &event.authority_proof_digest,
        );
        assert_ne!(first, changed_nonce);
        event.nonce += 1;
        assert!(event.verify_controller_signatures().is_err());

        event.nonce -= 1;
        let message = event.signing_message();
        event.next_controller_signature = hex::encode(next.sign(message.as_bytes()).to_bytes());
        event.current_controller_signature.clear();
        assert!(event.verify_controller_signatures().is_ok());
    }

    #[test]
    fn lifecycle_validity_is_bounded_by_chain_time() {
        let now = Utc::now();
        let mut event = NamespaceLifecycleEvent {
            schema_version: OWNERSHIP_SCHEMA_VERSION,
            action: NamespaceLifecycleAction::Transfer,
            namespace: "npm:owned-package@".into(),
            current_controller_ed25519_pubkey: "a".repeat(64),
            next_controller_ed25519_pubkey: "b".repeat(64),
            nonce: 1,
            issued_at: now,
            expires_at: now + Duration::hours(1),
            current_controller_signature: String::new(),
            next_controller_signature: String::new(),
            authority_proof_digest: String::new(),
        };
        assert!(!event.is_valid_at(now - Duration::seconds(1)));
        assert!(event.is_valid_at(now));
        assert!(event.is_valid_at(now + Duration::minutes(30)));
        assert!(!event.is_valid_at(event.expires_at));

        let mut exact_limit = event.clone();
        exact_limit.expires_at =
            exact_limit.issued_at + Duration::seconds(MAX_NAMESPACE_LIFECYCLE_EVENT_TTL_SECS);
        assert!(exact_limit.validate_structure().is_ok());

        let mut too_long = exact_limit;
        too_long.expires_at += Duration::seconds(1);
        assert_eq!(
            too_long.validate_structure(),
            Err("namespace lifecycle expiry must be after issuance")
        );

        event.expires_at = now;
        assert!(!event.is_valid_at(now));
    }
}
