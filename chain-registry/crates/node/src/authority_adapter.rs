//! Out-of-consensus npm/PyPI authority verification boundary.
//!
//! This module deliberately performs no network requests. An operator-managed
//! worker fetches evidence from an allow-listed registry origin and passes the
//! exact response bytes here. A pinned verifier process checks registry/OIDC/
//! attestation signatures and emits one canonical JSON result. Consensus code
//! must consume only a quorum-certified [`common::NamespaceAuthorityStatement`]
//! derived from that result; it must never invoke this adapter while replaying
//! or validating a block.

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use chrono::{DateTime, Utc};
use common::{
    canonical_publisher_address, sha256_hex, to_jcs_canonical_json,
    validate_authoritative_namespace, NamespaceAuthorityProof, NamespaceAuthorityPurpose,
    NamespaceAuthorityStatement, OwnershipAuthority,
    NAMESPACE_AUTHORITY_CERTIFICATE_SCHEMA_VERSION, OWNERSHIP_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

pub const AUTHORITY_ADAPTER_PROTOCOL_VERSION: u16 = 1;
pub const AUTHORITY_CHALLENGE_SCHEMA_VERSION: u16 = 1;
pub const DEFAULT_MAX_EVIDENCE_BYTES: usize = 2 * 1024 * 1024;
pub const HARD_MAX_EVIDENCE_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_MAX_RESULT_BYTES: usize = 64 * 1024;
pub const HARD_MAX_RESULT_BYTES: usize = 1024 * 1024;
pub const MAX_CHALLENGE_TTL_SECS: i64 = 15 * 60;
pub const MAX_CLOCK_SKEW_SECS: i64 = 60;
const MAX_STDERR_BYTES: usize = 8 * 1024;

#[derive(Debug, Error)]
pub enum AuthorityAdapterError {
    #[error("invalid authority-adapter configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("invalid authority-adapter request: {0}")]
    InvalidRequest(&'static str),
    #[error("authority evidence exceeds the configured size limit")]
    EvidenceTooLarge,
    #[error("authority evidence is not canonical lowercase hexadecimal")]
    EvidenceEncoding,
    #[error("authority evidence SHA-256 does not match its exact bytes")]
    EvidenceDigestMismatch,
    #[error("authority verifier executable is unavailable")]
    VerifierUnavailable,
    #[error("authority verifier executable digest does not match its pin")]
    VerifierDigestMismatch,
    #[error("authority verifier could not be started: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("authority verifier I/O failed: {0}")]
    Io(#[source] std::io::Error),
    #[error("authority verifier exceeded its time limit")]
    Timeout,
    #[error("authority verifier exited unsuccessfully")]
    VerifierRejected,
    #[error("authority verifier wrote diagnostics on a successful invocation")]
    UnexpectedDiagnostics,
    #[error("authority verifier result exceeds the configured size limit")]
    ResultTooLarge,
    #[error("authority verifier returned invalid JSON: {0}")]
    InvalidResultJson(#[source] serde_json::Error),
    #[error("authority verifier result is not canonical RFC 8785 JSON")]
    NonCanonicalResult,
    #[error("authority verifier result does not match the request: {0}")]
    ResultMismatch(&'static str),
}

/// Immutable verifier and policy pins. The executable digest is checked before
/// every invocation, preventing a replaced sidecar from merely claiming the
/// expected adapter version in its output.
#[derive(Debug, Clone)]
pub struct AuthorityAdapterConfig {
    pub verifier_path: PathBuf,
    pub verifier_sha256: String,
    pub adapter_id: String,
    pub adapter_version: String,
    pub policy_sha256: String,
    pub trust_root_sha256: String,
    pub timeout: Duration,
    pub max_evidence_bytes: usize,
    pub max_result_bytes: usize,
}

impl AuthorityAdapterConfig {
    fn validate_protocol(&self) -> Result<(), AuthorityAdapterError> {
        if !is_canonical_text(&self.adapter_id, 128)
            || !is_canonical_text(&self.adapter_version, 64)
        {
            return Err(AuthorityAdapterError::InvalidConfig(
                "adapter id/version must be canonical single-line text",
            ));
        }
        if !is_lower_hex_digest(&self.verifier_sha256, 64)
            || !is_lower_hex_digest(&self.policy_sha256, 64)
            || !is_lower_hex_digest(&self.trust_root_sha256, 64)
        {
            return Err(AuthorityAdapterError::InvalidConfig(
                "verifier, policy, and trust-root SHA-256 pins are required",
            ));
        }
        if self.timeout.is_zero() || self.timeout > Duration::from_secs(120) {
            return Err(AuthorityAdapterError::InvalidConfig(
                "timeout must be between one nanosecond and 120 seconds",
            ));
        }
        if self.max_evidence_bytes == 0
            || self.max_evidence_bytes > HARD_MAX_EVIDENCE_BYTES
            || self.max_result_bytes == 0
            || self.max_result_bytes > HARD_MAX_RESULT_BYTES
        {
            return Err(AuthorityAdapterError::InvalidConfig(
                "evidence/result limits exceed the protocol hard limit",
            ));
        }
        Ok(())
    }

    fn verified_executable(&self) -> Result<PathBuf, AuthorityAdapterError> {
        self.validate_protocol()?;
        if !self.verifier_path.is_absolute() {
            return Err(AuthorityAdapterError::InvalidConfig(
                "verifier path must be absolute",
            ));
        }
        let path = std::fs::canonicalize(&self.verifier_path)
            .map_err(|_| AuthorityAdapterError::VerifierUnavailable)?;
        let metadata =
            std::fs::metadata(&path).map_err(|_| AuthorityAdapterError::VerifierUnavailable)?;
        if !metadata.is_file() {
            return Err(AuthorityAdapterError::VerifierUnavailable);
        }
        if sha256_file(&path)? != self.verifier_sha256 {
            return Err(AuthorityAdapterError::VerifierDigestMismatch);
        }
        Ok(path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactDigestAlgorithm {
    Sha256,
    Sha512,
}

impl ArtifactDigestAlgorithm {
    fn name(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }

    fn hex_len(self) -> usize {
        match self {
            Self::Sha256 => 64,
            Self::Sha512 => 128,
        }
    }
}

/// Fresh, package-published challenge which prevents historical provenance
/// from authorizing an arbitrary new CREG controller. The registry verifier is
/// responsible for proving that this exact challenge is present in the
/// artifact identified by `artifact_digest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityChallengeV1 {
    pub schema_version: u16,
    pub purpose: NamespaceAuthorityPurpose,
    /// Exactly `creg-claim.json` or `creg-recovery.json`, according to purpose.
    pub challenge_document: String,
    pub chain_id: String,
    pub genesis_hash: String,
    pub namespace: String,
    #[serde(default)]
    pub current_controller_ed25519_pubkey: String,
    #[serde(default)]
    pub current_controller_evm_address: String,
    pub next_controller_ed25519_pubkey: String,
    pub next_controller_evm_address: String,
    pub nonce: u64,
    pub issued_at: i64,
    pub expires_at: i64,
    pub authorization_sha256: String,
    pub artifact_digest_algorithm: ArtifactDigestAlgorithm,
    pub artifact_digest: String,
}

impl AuthorityChallengeV1 {
    pub fn signing_message(&self) -> String {
        format!(
            "creg-namespace-authority-challenge-v1\nservice:creg-package-ownership\npurpose:{}\ndocument:{}\nchain_id:{}\ngenesis_hash:{}\nnamespace:{}\ncurrent_ed25519_pubkey:{}\ncurrent_evm_address:{}\nnext_ed25519_pubkey:{}\nnext_evm_address:{}\nnonce:{}\nissued_at:{}\nexpires_at:{}\nauthorization_sha256:{}\nartifact_digest_algorithm:{}\nartifact_digest:{}",
            purpose_name(self.purpose),
            self.challenge_document,
            self.chain_id,
            self.genesis_hash,
            self.namespace,
            self.current_controller_ed25519_pubkey,
            self.current_controller_evm_address,
            self.next_controller_ed25519_pubkey,
            self.next_controller_evm_address,
            self.nonce,
            self.issued_at,
            self.expires_at,
            self.authorization_sha256,
            self.artifact_digest_algorithm.name(),
            self.artifact_digest,
        )
    }

    pub fn digest(&self) -> String {
        sha256_hex(self.signing_message().as_bytes())
    }

    pub fn validate_at(&self, now: i64) -> Result<(), AuthorityAdapterError> {
        if self.schema_version != AUTHORITY_CHALLENGE_SCHEMA_VERSION {
            return Err(AuthorityAdapterError::InvalidRequest(
                "unsupported challenge schema version",
            ));
        }
        if !is_canonical_text(&self.chain_id, 128) || !is_lower_hex_digest(&self.genesis_hash, 64) {
            return Err(AuthorityAdapterError::InvalidRequest(
                "challenge chain/genesis binding is invalid",
            ));
        }
        validate_authoritative_namespace(&self.namespace).map_err(|_| {
            AuthorityAdapterError::InvalidRequest("challenge namespace is not canonical")
        })?;
        let expected_document = match self.purpose {
            NamespaceAuthorityPurpose::Claim => "creg-claim.json",
            NamespaceAuthorityPurpose::Recover => "creg-recovery.json",
        };
        if self.challenge_document != expected_document {
            return Err(AuthorityAdapterError::InvalidRequest(
                "challenge document does not match its purpose",
            ));
        }
        if self.nonce == 0 {
            return Err(AuthorityAdapterError::InvalidRequest(
                "challenge nonce must be non-zero",
            ));
        }
        if !is_ed25519_key(&self.next_controller_ed25519_pubkey)
            || !is_evm_address(&self.next_controller_evm_address)
        {
            return Err(AuthorityAdapterError::InvalidRequest(
                "challenge successor controller is invalid",
            ));
        }
        match self.purpose {
            NamespaceAuthorityPurpose::Claim => {
                if !self.current_controller_ed25519_pubkey.is_empty()
                    || !self.current_controller_evm_address.is_empty()
                {
                    return Err(AuthorityAdapterError::InvalidRequest(
                        "first-claim challenge must not name a current controller",
                    ));
                }
            }
            NamespaceAuthorityPurpose::Recover => {
                if !is_ed25519_key(&self.current_controller_ed25519_pubkey)
                    || !is_evm_address(&self.current_controller_evm_address)
                {
                    return Err(AuthorityAdapterError::InvalidRequest(
                        "recovery challenge current controller is invalid",
                    ));
                }
            }
        }
        if !is_lower_hex_digest(&self.authorization_sha256, 64)
            || !is_lower_hex_digest(
                &self.artifact_digest,
                self.artifact_digest_algorithm.hex_len(),
            )
        {
            return Err(AuthorityAdapterError::InvalidRequest(
                "challenge authorization/artifact digest is invalid",
            ));
        }
        let expected_algorithm = match namespace_ecosystem(&self.namespace) {
            Some("npm") => ArtifactDigestAlgorithm::Sha512,
            Some("pypi") => ArtifactDigestAlgorithm::Sha256,
            _ => {
                return Err(AuthorityAdapterError::InvalidRequest(
                    "only npm and PyPI authority challenges are supported",
                ))
            }
        };
        if self.artifact_digest_algorithm != expected_algorithm {
            return Err(AuthorityAdapterError::InvalidRequest(
                "artifact digest algorithm does not match the registry",
            ));
        }
        if self.issued_at < 0
            || self.expires_at <= self.issued_at
            || self.expires_at - self.issued_at > MAX_CHALLENGE_TTL_SECS
            || now > self.expires_at
            || self.issued_at > now.saturating_add(MAX_CLOCK_SKEW_SECS)
        {
            return Err(AuthorityAdapterError::InvalidRequest(
                "challenge is expired, premature, or too long-lived",
            ));
        }
        Ok(())
    }
}

/// Strict input contract passed to the verifier. Evidence is lowercase hex so
/// the sidecar sees the exact upstream bytes without JSON re-encoding them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityAdapterRequestV1 {
    pub schema_version: u16,
    pub authority: OwnershipAuthority,
    pub evidence_url: String,
    pub evidence_sha256: String,
    pub evidence_hex: String,
    pub expected_issuer: String,
    pub expected_subject: String,
    pub expected_audience: String,
    pub challenge: AuthorityChallengeV1,
}

impl AuthorityAdapterRequestV1 {
    pub fn validate_at(
        &self,
        now: i64,
        max_evidence_bytes: usize,
    ) -> Result<Vec<u8>, AuthorityAdapterError> {
        if self.schema_version != AUTHORITY_ADAPTER_PROTOCOL_VERSION {
            return Err(AuthorityAdapterError::InvalidRequest(
                "unsupported request schema version",
            ));
        }
        self.challenge.validate_at(now)?;
        let authority_ecosystem = self.authority.ecosystem();
        if namespace_ecosystem(&self.challenge.namespace) != Some(authority_ecosystem) {
            return Err(AuthorityAdapterError::InvalidRequest(
                "authority does not match challenge namespace",
            ));
        }
        validate_evidence_url(&self.evidence_url, self.authority)?;
        if !is_canonical_text(&self.expected_issuer, 512)
            || !is_canonical_text(&self.expected_subject, 1024)
            || self.expected_audience != self.authority.expected_audience()
        {
            return Err(AuthorityAdapterError::InvalidRequest(
                "expected issuer, subject, or audience is invalid",
            ));
        }
        if !is_lower_hex_digest(&self.evidence_sha256, 64) {
            return Err(AuthorityAdapterError::InvalidRequest(
                "evidence SHA-256 is not canonical",
            ));
        }
        if self.evidence_hex.is_empty()
            || self.evidence_hex.len() % 2 != 0
            || self.evidence_hex.len() / 2 > max_evidence_bytes
            || self.evidence_hex.len() / 2 > HARD_MAX_EVIDENCE_BYTES
        {
            return Err(AuthorityAdapterError::EvidenceTooLarge);
        }
        if self.evidence_hex != self.evidence_hex.to_ascii_lowercase()
            || !self
                .evidence_hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AuthorityAdapterError::EvidenceEncoding);
        }
        let evidence =
            hex::decode(&self.evidence_hex).map_err(|_| AuthorityAdapterError::EvidenceEncoding)?;
        if sha256_hex(&evidence) != self.evidence_sha256 {
            return Err(AuthorityAdapterError::EvidenceDigestMismatch);
        }
        Ok(evidence)
    }
}

/// Canonical stdout contract emitted by the pinned sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityAdapterResultV1 {
    pub schema_version: u16,
    pub verified: bool,
    pub adapter_id: String,
    pub adapter_version: String,
    pub authority: OwnershipAuthority,
    pub evidence_url: String,
    pub evidence_sha256: String,
    pub issuer: String,
    pub subject: String,
    pub audience: String,
    pub purpose: NamespaceAuthorityPurpose,
    pub namespace: String,
    pub challenge_sha256: String,
    pub current_controller_ed25519_pubkey: String,
    pub current_controller_evm_address: String,
    pub next_controller_ed25519_pubkey: String,
    pub next_controller_evm_address: String,
    pub authorization_sha256: String,
    pub artifact_digest_algorithm: ArtifactDigestAlgorithm,
    pub artifact_digest: String,
    pub policy_sha256: String,
    pub trust_root_sha256: String,
    pub proof_issued_at: i64,
    pub proof_expires_at: i64,
    pub verified_at: i64,
}

/// Result accepted from the sidecar and suitable for validator attestation.
/// It is not itself consensus authority; a validator quorum still has to sign
/// the resulting chain-bound namespace statement.
#[derive(Debug, Clone)]
pub struct VerifiedAuthorityObservation {
    pub proof: NamespaceAuthorityProof,
    pub challenge: AuthorityChallengeV1,
    pub challenge_sha256: String,
    pub artifact_digest_algorithm: ArtifactDigestAlgorithm,
    pub artifact_digest: String,
    pub adapter_id: String,
    pub adapter_version: String,
    pub policy_sha256: String,
    pub trust_root_sha256: String,
    pub verified_at: i64,
}

impl VerifiedAuthorityObservation {
    /// Convert a verified out-of-consensus observation into the deterministic
    /// statement validators sign. The supplied anchor and validator-set hash
    /// still have to be checked against canonical chain state by each signer.
    pub fn into_statement(
        self,
        anchor_height: u64,
        anchor_block_hash: String,
        validator_set_hash: String,
    ) -> Result<NamespaceAuthorityStatement, AuthorityAdapterError> {
        let statement = NamespaceAuthorityStatement {
            schema_version: NAMESPACE_AUTHORITY_CERTIFICATE_SCHEMA_VERSION,
            chain_id: self.challenge.chain_id.clone(),
            genesis_hash: self.challenge.genesis_hash.clone(),
            purpose: self.challenge.purpose,
            namespace: self.challenge.namespace.clone(),
            current_controller_ed25519_pubkey: self
                .challenge
                .current_controller_ed25519_pubkey
                .clone(),
            next_controller_ed25519_pubkey: self.challenge.next_controller_ed25519_pubkey.clone(),
            next_controller_evm_address: self.challenge.next_controller_evm_address.clone(),
            authorization_digest: self.challenge.authorization_sha256.clone(),
            authority_proof: self.proof,
            adapter_id: self.adapter_id,
            adapter_version: self.adapter_version,
            policy_sha256: self.policy_sha256,
            trust_root_sha256: self.trust_root_sha256,
            anchor_height,
            anchor_block_hash,
            validator_set_hash,
            issued_at: self.challenge.issued_at,
            expires_at: self.challenge.expires_at,
        };
        statement.validate_structure().map_err(|_| {
            AuthorityAdapterError::ResultMismatch(
                "verified observation cannot form a canonical authority statement",
            )
        })?;
        Ok(statement)
    }
}

#[derive(Serialize)]
struct AuthorityAdapterInvocationV1<'a> {
    schema_version: u16,
    adapter_id: &'a str,
    adapter_version: &'a str,
    policy_sha256: &'a str,
    trust_root_sha256: &'a str,
    request: &'a AuthorityAdapterRequestV1,
}

/// Invoke the pinned verifier directly, with no shell and no inherited
/// environment. Call this only from an admission/attestation worker, never
/// from block proposal, PBFT validation, sync, replay, or state rebuild paths.
pub async fn verify_with_sidecar(
    config: &AuthorityAdapterConfig,
    request: &AuthorityAdapterRequestV1,
) -> Result<VerifiedAuthorityObservation, AuthorityAdapterError> {
    let now = Utc::now().timestamp();
    config.validate_protocol()?;
    request.validate_at(now, config.max_evidence_bytes)?;
    let verifier = config.verified_executable()?;
    let invocation = AuthorityAdapterInvocationV1 {
        schema_version: AUTHORITY_ADAPTER_PROTOCOL_VERSION,
        adapter_id: &config.adapter_id,
        adapter_version: &config.adapter_version,
        policy_sha256: &config.policy_sha256,
        trust_root_sha256: &config.trust_root_sha256,
        request,
    };
    let input = to_jcs_canonical_json(&invocation).map_err(|_| {
        AuthorityAdapterError::InvalidRequest("request cannot be encoded as canonical JSON")
    })?;

    let mut command = Command::new(&verifier);
    command
        .env_clear()
        .env("CREG_AUTHORITY_ADAPTER_PROTOCOL", "1")
        .current_dir(verifier.parent().unwrap_or_else(|| Path::new(".")))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(AuthorityAdapterError::Spawn)?;
    let mut stdin = child.stdin.take().ok_or_else(|| {
        AuthorityAdapterError::Io(std::io::Error::other("verifier stdin unavailable"))
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        AuthorityAdapterError::Io(std::io::Error::other("verifier stdout unavailable"))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        AuthorityAdapterError::Io(std::io::Error::other("verifier stderr unavailable"))
    })?;

    let io = async move {
        let write = async move {
            stdin.write_all(&input).await?;
            stdin.shutdown().await
        };
        let read_stdout = read_limited(stdout, config.max_result_bytes);
        let read_stderr = read_limited(stderr, MAX_STDERR_BYTES);
        let wait = child.wait();
        let (_, stdout, stderr, status) = tokio::try_join!(write, read_stdout, read_stderr, wait)?;
        Ok::<_, std::io::Error>((stdout, stderr, status))
    };
    let (stdout, stderr, status) = tokio::time::timeout(config.timeout, io)
        .await
        .map_err(|_| AuthorityAdapterError::Timeout)?
        .map_err(AuthorityAdapterError::Io)?;
    if stdout.len() > config.max_result_bytes {
        return Err(AuthorityAdapterError::ResultTooLarge);
    }
    if !status.success() {
        return Err(AuthorityAdapterError::VerifierRejected);
    }
    if !stderr.is_empty() {
        return Err(AuthorityAdapterError::UnexpectedDiagnostics);
    }
    validate_canonical_result(config, request, &stdout, now)
}

/// Validate a sidecar's raw stdout. Exposed separately so protocol fixtures can
/// be tested without executing an external process on the host platform.
pub fn validate_canonical_result(
    config: &AuthorityAdapterConfig,
    request: &AuthorityAdapterRequestV1,
    stdout: &[u8],
    now: i64,
) -> Result<VerifiedAuthorityObservation, AuthorityAdapterError> {
    config.validate_protocol()?;
    request.validate_at(now, config.max_evidence_bytes)?;
    if stdout.len() > config.max_result_bytes {
        return Err(AuthorityAdapterError::ResultTooLarge);
    }
    let result: AuthorityAdapterResultV1 =
        serde_json::from_slice(stdout).map_err(AuthorityAdapterError::InvalidResultJson)?;
    let canonical =
        to_jcs_canonical_json(&result).map_err(|_| AuthorityAdapterError::NonCanonicalResult)?;
    if canonical != stdout {
        return Err(AuthorityAdapterError::NonCanonicalResult);
    }
    validate_result_fields(config, request, result, now)
}

fn validate_result_fields(
    config: &AuthorityAdapterConfig,
    request: &AuthorityAdapterRequestV1,
    result: AuthorityAdapterResultV1,
    now: i64,
) -> Result<VerifiedAuthorityObservation, AuthorityAdapterError> {
    let challenge = &request.challenge;
    if result.schema_version != AUTHORITY_ADAPTER_PROTOCOL_VERSION || !result.verified {
        return Err(AuthorityAdapterError::ResultMismatch(
            "sidecar did not return a verified v1 result",
        ));
    }
    if result.adapter_id != config.adapter_id || result.adapter_version != config.adapter_version {
        return Err(AuthorityAdapterError::ResultMismatch(
            "adapter id/version differs from its pin",
        ));
    }
    if result.policy_sha256 != config.policy_sha256
        || result.trust_root_sha256 != config.trust_root_sha256
    {
        return Err(AuthorityAdapterError::ResultMismatch(
            "policy or trust-root digest differs from its pin",
        ));
    }
    if result.authority != request.authority
        || result.evidence_url != request.evidence_url
        || result.evidence_sha256 != request.evidence_sha256
        || result.issuer != request.expected_issuer
        || result.subject != request.expected_subject
        || result.audience != request.expected_audience
    {
        return Err(AuthorityAdapterError::ResultMismatch(
            "registry identity/evidence differs from the request",
        ));
    }
    if result.purpose != challenge.purpose
        || result.namespace != challenge.namespace
        || result.challenge_sha256 != challenge.digest()
        || result.current_controller_ed25519_pubkey != challenge.current_controller_ed25519_pubkey
        || result.current_controller_evm_address != challenge.current_controller_evm_address
        || result.next_controller_ed25519_pubkey != challenge.next_controller_ed25519_pubkey
        || result.next_controller_evm_address != challenge.next_controller_evm_address
        || result.authorization_sha256 != challenge.authorization_sha256
        || result.artifact_digest_algorithm != challenge.artifact_digest_algorithm
        || result.artifact_digest != challenge.artifact_digest
    {
        return Err(AuthorityAdapterError::ResultMismatch(
            "challenge, controller, authorization, or artifact binding differs",
        ));
    }
    if result.proof_issued_at < 0
        || result.proof_issued_at > challenge.issued_at
        || result.proof_expires_at < challenge.expires_at
        || result.proof_expires_at <= result.proof_issued_at
        || result.verified_at < challenge.issued_at.saturating_sub(MAX_CLOCK_SKEW_SECS)
        || result.verified_at > now.saturating_add(MAX_CLOCK_SKEW_SECS)
        || result.verified_at > challenge.expires_at
    {
        return Err(AuthorityAdapterError::ResultMismatch(
            "proof or verification time does not cover the fresh challenge",
        ));
    }
    let issued_at = DateTime::from_timestamp(result.proof_issued_at, 0).ok_or(
        AuthorityAdapterError::ResultMismatch("proof issuance timestamp is out of range"),
    )?;
    let expires_at = DateTime::from_timestamp(result.proof_expires_at, 0).ok_or(
        AuthorityAdapterError::ResultMismatch("proof expiry timestamp is out of range"),
    )?;
    let proof = NamespaceAuthorityProof {
        schema_version: OWNERSHIP_SCHEMA_VERSION,
        authority: result.authority,
        namespace: result.namespace,
        issuer: result.issuer,
        subject: result.subject,
        audience: result.audience,
        evidence_uri: result.evidence_url,
        evidence_sha256: result.evidence_sha256,
        controller_evm_address: result.next_controller_evm_address,
        controller_ed25519_pubkey: result.next_controller_ed25519_pubkey,
        issued_at,
        expires_at,
    };
    proof.validate_structure().map_err(|_| {
        AuthorityAdapterError::ResultMismatch("result cannot form a canonical authority proof")
    })?;
    Ok(VerifiedAuthorityObservation {
        proof,
        challenge: challenge.clone(),
        challenge_sha256: result.challenge_sha256,
        artifact_digest_algorithm: result.artifact_digest_algorithm,
        artifact_digest: result.artifact_digest,
        adapter_id: result.adapter_id,
        adapter_version: result.adapter_version,
        policy_sha256: result.policy_sha256,
        trust_root_sha256: result.trust_root_sha256,
        verified_at: result.verified_at,
    })
}

fn validate_evidence_url(
    raw: &str,
    authority: OwnershipAuthority,
) -> Result<(), AuthorityAdapterError> {
    let url = reqwest::Url::parse(raw).map_err(|_| {
        AuthorityAdapterError::InvalidRequest("evidence URL is not an absolute URL")
    })?;
    let expected_host = match authority {
        OwnershipAuthority::NpmTrustedPublisher => "registry.npmjs.org",
        OwnershipAuthority::PypiTrustedPublisher => "pypi.org",
    };
    if url.scheme() != "https"
        || url.host_str() != Some(expected_host)
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.as_str() != raw
    {
        return Err(AuthorityAdapterError::InvalidRequest(
            "evidence URL is outside the exact official registry origin",
        ));
    }
    Ok(())
}

async fn read_limited<R: AsyncRead + Unpin>(reader: R, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut output)
        .await?;
    Ok(output)
}

fn sha256_file(path: &Path) -> Result<String, AuthorityAdapterError> {
    let mut file = File::open(path).map_err(|_| AuthorityAdapterError::VerifierUnavailable)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| AuthorityAdapterError::VerifierUnavailable)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn namespace_ecosystem(namespace: &str) -> Option<&str> {
    namespace.split_once(':').map(|(ecosystem, _)| ecosystem)
}

fn purpose_name(purpose: NamespaceAuthorityPurpose) -> &'static str {
    match purpose {
        NamespaceAuthorityPurpose::Claim => "claim",
        NamespaceAuthorityPurpose::Recover => "recover",
    }
}

fn is_canonical_text(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value == value.trim()
        && !value.chars().any(char::is_control)
}

fn is_lower_hex_digest(value: &str, len: usize) -> bool {
    value.len() == len
        && value == value.to_ascii_lowercase()
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_ed25519_key(value: &str) -> bool {
    is_lower_hex_digest(value, 64)
}

fn is_evm_address(value: &str) -> bool {
    value.len() == 42
        && value == canonical_publisher_address(value)
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_900_000_000;

    fn config() -> AuthorityAdapterConfig {
        AuthorityAdapterConfig {
            verifier_path: PathBuf::from("unused-for-protocol-validation"),
            verifier_sha256: "11".repeat(32),
            adapter_id: "creg-pypi-sigstore".to_owned(),
            adapter_version: "1.0.0".to_owned(),
            policy_sha256: "22".repeat(32),
            trust_root_sha256: "33".repeat(32),
            timeout: Duration::from_secs(5),
            max_evidence_bytes: DEFAULT_MAX_EVIDENCE_BYTES,
            max_result_bytes: DEFAULT_MAX_RESULT_BYTES,
        }
    }

    fn request() -> AuthorityAdapterRequestV1 {
        let evidence = br#"{"attestation":"fixture"}"#;
        AuthorityAdapterRequestV1 {
            schema_version: AUTHORITY_ADAPTER_PROTOCOL_VERSION,
            authority: OwnershipAuthority::PypiTrustedPublisher,
            evidence_url: "https://pypi.org/integrity/demo/1.0/demo.whl/provenance".to_owned(),
            evidence_sha256: sha256_hex(evidence),
            evidence_hex: hex::encode(evidence),
            expected_issuer: "https://token.actions.githubusercontent.com".to_owned(),
            expected_subject: "repo:owner/demo:ref:refs/heads/main".to_owned(),
            expected_audience: "pypi".to_owned(),
            challenge: AuthorityChallengeV1 {
                schema_version: AUTHORITY_CHALLENGE_SCHEMA_VERSION,
                purpose: NamespaceAuthorityPurpose::Recover,
                challenge_document: "creg-recovery.json".to_owned(),
                chain_id: "creg-testnet-1".to_owned(),
                genesis_hash: "44".repeat(32),
                namespace: "pypi:demo@".to_owned(),
                current_controller_ed25519_pubkey: "55".repeat(32),
                current_controller_evm_address: format!("0x{}", "66".repeat(20)),
                next_controller_ed25519_pubkey: "77".repeat(32),
                next_controller_evm_address: format!("0x{}", "88".repeat(20)),
                nonce: 7,
                issued_at: NOW - 10,
                expires_at: NOW + 300,
                authorization_sha256: "99".repeat(32),
                artifact_digest_algorithm: ArtifactDigestAlgorithm::Sha256,
                artifact_digest: "aa".repeat(32),
            },
        }
    }

    fn result(request: &AuthorityAdapterRequestV1) -> AuthorityAdapterResultV1 {
        let config = config();
        AuthorityAdapterResultV1 {
            schema_version: AUTHORITY_ADAPTER_PROTOCOL_VERSION,
            verified: true,
            adapter_id: config.adapter_id,
            adapter_version: config.adapter_version,
            authority: request.authority,
            evidence_url: request.evidence_url.clone(),
            evidence_sha256: request.evidence_sha256.clone(),
            issuer: request.expected_issuer.clone(),
            subject: request.expected_subject.clone(),
            audience: request.expected_audience.clone(),
            purpose: request.challenge.purpose,
            namespace: request.challenge.namespace.clone(),
            challenge_sha256: request.challenge.digest(),
            current_controller_ed25519_pubkey: request
                .challenge
                .current_controller_ed25519_pubkey
                .clone(),
            current_controller_evm_address: request
                .challenge
                .current_controller_evm_address
                .clone(),
            next_controller_ed25519_pubkey: request
                .challenge
                .next_controller_ed25519_pubkey
                .clone(),
            next_controller_evm_address: request.challenge.next_controller_evm_address.clone(),
            authorization_sha256: request.challenge.authorization_sha256.clone(),
            artifact_digest_algorithm: request.challenge.artifact_digest_algorithm,
            artifact_digest: request.challenge.artifact_digest.clone(),
            policy_sha256: config.policy_sha256,
            trust_root_sha256: config.trust_root_sha256,
            proof_issued_at: NOW - 60,
            proof_expires_at: NOW + 600,
            verified_at: NOW,
        }
    }

    #[test]
    fn accepts_exact_canonical_pypi_result() {
        let request = request();
        let raw = to_jcs_canonical_json(&result(&request)).unwrap();
        let observation = validate_canonical_result(&config(), &request, &raw, NOW).unwrap();
        assert_eq!(observation.proof.namespace, "pypi:demo@");
        assert_eq!(observation.proof.evidence_sha256, request.evidence_sha256);
        assert_eq!(observation.challenge_sha256, request.challenge.digest());
    }

    #[test]
    fn rejects_noncanonical_or_unknown_result_json() {
        let request = request();
        let pretty = serde_json::to_vec_pretty(&result(&request)).unwrap();
        assert!(matches!(
            validate_canonical_result(&config(), &request, &pretty, NOW),
            Err(AuthorityAdapterError::NonCanonicalResult)
        ));

        let mut value = serde_json::to_value(result(&request)).unwrap();
        value["untrusted_extra"] = serde_json::json!(true);
        let raw = to_jcs_canonical_json(&value).unwrap();
        assert!(matches!(
            validate_canonical_result(&config(), &request, &raw, NOW),
            Err(AuthorityAdapterError::InvalidResultJson(_))
        ));
    }

    #[test]
    fn rejects_registry_origin_aliases_and_mismatches() {
        for url in [
            "http://pypi.org/integrity/demo",
            "https://files.pythonhosted.org/demo",
            "https://pypi.org.evil.example/demo",
            "https://pypi.org:443/demo",
            "https://PYPI.org/demo",
        ] {
            let mut request = request();
            request.evidence_url = url.to_owned();
            assert!(matches!(
                request.validate_at(NOW, DEFAULT_MAX_EVIDENCE_BYTES),
                Err(AuthorityAdapterError::InvalidRequest(_))
            ));
        }
    }

    #[test]
    fn rejects_tampered_evidence_and_stale_challenge() {
        let mut tampered = request();
        tampered.evidence_hex = hex::encode(b"different");
        assert!(matches!(
            tampered.validate_at(NOW, DEFAULT_MAX_EVIDENCE_BYTES),
            Err(AuthorityAdapterError::EvidenceDigestMismatch)
        ));

        let mut stale = request();
        stale.challenge.expires_at = NOW - 1;
        assert!(matches!(
            stale.validate_at(NOW, DEFAULT_MAX_EVIDENCE_BYTES),
            Err(AuthorityAdapterError::InvalidRequest(_))
        ));
    }

    #[test]
    fn rejects_controller_and_policy_substitution() {
        let request = request();
        let mut substituted = result(&request);
        substituted.next_controller_ed25519_pubkey = "ab".repeat(32);
        let raw = to_jcs_canonical_json(&substituted).unwrap();
        assert!(matches!(
            validate_canonical_result(&config(), &request, &raw, NOW),
            Err(AuthorityAdapterError::ResultMismatch(_))
        ));

        let mut substituted = result(&request);
        substituted.policy_sha256 = "cd".repeat(32);
        let raw = to_jcs_canonical_json(&substituted).unwrap();
        assert!(matches!(
            validate_canonical_result(&config(), &request, &raw, NOW),
            Err(AuthorityAdapterError::ResultMismatch(_))
        ));
    }

    #[test]
    fn npm_requires_registry_origin_audience_and_sha512_artifact() {
        let mut request = request();
        request.authority = OwnershipAuthority::NpmTrustedPublisher;
        request.evidence_url =
            "https://registry.npmjs.org/-/npm/v1/attestations/demo@1.0.0".to_owned();
        request.expected_audience = "npm:registry.npmjs.org".to_owned();
        request.challenge.namespace = "npm:demo@".to_owned();
        request.challenge.artifact_digest_algorithm = ArtifactDigestAlgorithm::Sha512;
        request.challenge.artifact_digest = "ab".repeat(64);
        assert!(request.validate_at(NOW, DEFAULT_MAX_EVIDENCE_BYTES).is_ok());

        request.expected_audience = "npm".to_owned();
        assert!(matches!(
            request.validate_at(NOW, DEFAULT_MAX_EVIDENCE_BYTES),
            Err(AuthorityAdapterError::InvalidRequest(_))
        ));
    }
}
