//! Node-to-sidecar launch contract: cleared environment plus private scratch.
//!
//! The default case uses the in-repo rejection fixture so Windows CI does not
//! depend on a gitignored PyInstaller artifact. If a freshly built one-file
//! candidate is present under `tools/authority-adapters/pypi-verifier/dist`,
//! the same Rust launcher is also exercised against that binary.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use common::{sha256_hex, NamespaceAuthorityPurpose, OwnershipAuthority};
use ed25519_dalek::{Signer, SigningKey};
use node::authority_adapter::{
    ArtifactDigestAlgorithm, AuthorityAdapterConfig, AuthorityAdapterError,
    AuthorityAdapterRequestV2, AuthorityChallengeV1, AUTHORITY_ADAPTER_PROTOCOL_VERSION,
    AUTHORITY_CHALLENGE_SCHEMA_VERSION, DEFAULT_MAX_ARTIFACT_BYTES, DEFAULT_MAX_EVIDENCE_BYTES,
    DEFAULT_MAX_RESULT_BYTES,
};
use sha2::{Digest, Sha256};

fn sha256_file(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("verifier fixture must be readable");
    hex::encode(Sha256::digest(bytes))
}

fn install_verifier(source: &Path, parent: &Path) -> PathBuf {
    let install_dir = parent.join("verifier");
    std::fs::create_dir_all(&install_dir).unwrap();
    let installed = install_dir.join(source.file_name().expect("verifier file name"));
    std::fs::copy(source, &installed).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::canonicalize(installed).unwrap()
}

fn adapter_config(
    verifier_path: PathBuf,
    scratch_root: PathBuf,
    timeout: Duration,
) -> AuthorityAdapterConfig {
    AuthorityAdapterConfig {
        verifier_sha256: sha256_file(&verifier_path),
        verifier_path,
        scratch_root,
        adapter_id: "creg-pypi-sigstore".to_owned(),
        adapter_version: "1.0.0".to_owned(),
        policy_sha256: "22".repeat(32),
        trust_root_sha256: "33".repeat(32),
        timeout,
        max_evidence_bytes: DEFAULT_MAX_EVIDENCE_BYTES,
        max_artifact_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
        max_result_bytes: DEFAULT_MAX_RESULT_BYTES,
    }
}

fn live_request() -> AuthorityAdapterRequestV2 {
    let evidence = br#"{"attestation":"fixture"}"#;
    let artifact = b"fixture wheel bytes containing creg-recovery.json";
    let next_controller = SigningKey::from_bytes(&[7; 32]);
    let now = Utc::now().timestamp();
    let mut challenge = AuthorityChallengeV1 {
        schema_version: AUTHORITY_CHALLENGE_SCHEMA_VERSION,
        purpose: NamespaceAuthorityPurpose::Recover,
        challenge_document: "creg-recovery.json".to_owned(),
        chain_id: "creg-testnet-1".to_owned(),
        genesis_hash: "44".repeat(32),
        namespace: "pypi:demo@".to_owned(),
        current_controller_ed25519_pubkey: "55".repeat(32),
        current_controller_evm_address: format!("0x{}", "66".repeat(20)),
        next_controller_ed25519_pubkey: hex::encode(next_controller.verifying_key().as_bytes()),
        next_controller_evm_address: format!("0x{}", "88".repeat(20)),
        nonce: 7,
        issued_at: now - 10,
        expires_at: now + 300,
        next_controller_signature: String::new(),
    };
    challenge.next_controller_signature = hex::encode(
        next_controller
            .sign(challenge.signing_message().as_bytes())
            .to_bytes(),
    );
    AuthorityAdapterRequestV2 {
        schema_version: AUTHORITY_ADAPTER_PROTOCOL_VERSION,
        authority: OwnershipAuthority::PypiTrustedPublisher,
        evidence_url: "https://pypi.org/integrity/demo/1.0/demo.whl/provenance".to_owned(),
        evidence_sha256: sha256_hex(evidence),
        evidence_hex: hex::encode(evidence),
        artifact_url: "https://files.pythonhosted.org/packages/aa/bb/demo-1.0-py3-none-any.whl"
            .to_owned(),
        artifact_digest_algorithm: ArtifactDigestAlgorithm::Sha256,
        artifact_digest: ArtifactDigestAlgorithm::Sha256.digest(artifact),
        artifact_hex: hex::encode(artifact),
        expected_issuer: "https://token.actions.githubusercontent.com".to_owned(),
        expected_subject: "repo:owner/demo:ref:refs/heads/main".to_owned(),
        expected_audience: "pypi".to_owned(),
        challenge,
    }
}

async fn assert_rejected_and_scratch_removed(config: &AuthorityAdapterConfig) {
    let error = node::authority_adapter::verify_with_sidecar(config, &live_request())
        .await
        .expect_err("authority sidecar fixture must never verify");
    assert!(
        matches!(error, AuthorityAdapterError::VerifierRejected),
        "expected VerifierRejected after private-temp launch, got {error:?}"
    );
    assert!(
        std::fs::read_dir(&config.scratch_root)
            .map(|entries| entries.count() == 0)
            .unwrap_or(true),
        "private invocation scratch must be removed after rejection"
    );
}

fn rejection_fixture_path() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_creg_authority_rejection_fixture") {
        return PathBuf::from(path);
    }
    let name = format!(
        "creg-authority-rejection-fixture{}",
        std::env::consts::EXE_SUFFIX
    );
    let mut dir = std::env::current_exe().expect("test executable path");
    dir.pop();
    if dir.file_name().is_some_and(|component| component == "deps") {
        dir.pop();
    }
    let path = dir.join(name);
    assert!(
        path.is_file(),
        "rejection fixture binary missing at {} — build creg-authority-rejection-fixture first",
        path.display()
    );
    path
}

fn frozen_pypi_verifier() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "creg-pypi-authority-verifier.exe"
    } else {
        "creg-pypi-authority-verifier"
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/authority-adapters/pypi-verifier/dist")
        .join(name);
    std::fs::canonicalize(path)
        .ok()
        .filter(|path| path.is_file())
}

#[tokio::test]
async fn rust_launcher_gives_fixture_a_private_temp_scratch() {
    let parent = tempfile::tempdir().unwrap();
    let source = rejection_fixture_path();
    let verifier = install_verifier(&source, parent.path());
    let config = adapter_config(
        verifier,
        parent.path().join("scratch"),
        Duration::from_secs(10),
    );
    assert_rejected_and_scratch_removed(&config).await;
}

#[tokio::test]
async fn rust_launcher_gives_frozen_pypi_verifier_a_private_temp_scratch() {
    let Some(source) = frozen_pypi_verifier() else {
        eprintln!(
            "skipping frozen PyPI sidecar smoke: dist artifact is absent; \
             rebuild with tools/authority-adapters/pypi-verifier/scripts/build_onefile.py"
        );
        return;
    };
    let parent = tempfile::tempdir().unwrap();
    let verifier = install_verifier(&source, parent.path());
    let config = adapter_config(
        verifier,
        parent.path().join("scratch"),
        Duration::from_secs(30),
    );
    assert_rejected_and_scratch_removed(&config).await;
}
