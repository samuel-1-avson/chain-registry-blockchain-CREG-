// crates/cli/src/evidence.rs
// `creg evidence verify-witness <digest|file>` — offline cryptographic
// verification of `TriggerWitnessV1` against a signed chain spec (`ADR-019` §4.2,
// `ECO-INV-03`). Performs zero sandbox replay and zero LLM calls.

use crate::evidence_panel;
use anyhow::{bail, Context, Result};
use colored::Colorize;
use common::{
    validator_status_is_active, AuthenticatedValidatorSetSnapshot, ChainSpec, Transaction,
    TriggerWitnessV1,
};
use registry_view::{ProofBundleV2, WitnessVerificationV1, WitnessViewV1};
use resolver::light_client::AuthorityTimeline;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MAX_WITNESS_FILE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessVerifyReportV1 {
    pub schema_version: u16,
    pub ok: bool,
    pub digest: String,
    pub height: u64,
    pub require_hybrid: bool,
    pub prover_active_at_height: bool,
    pub wire_budget_ok: bool,
    pub signature_ok: bool,
    pub witness: WitnessViewV1,
}

/// Offline `TriggerWitnessV1` verifier delegating exclusively to `common`:
/// checks wire budget, resolves prover membership at `height` via
/// [`AuthorityTimeline`], checks active validator status, and calls
/// [`TriggerWitnessV1::verify`].
#[cfg(test)]
pub fn verify_witness_offline(
    witness: &TriggerWitnessV1,
    spec: &ChainSpec,
    authority_history: &[AuthenticatedValidatorSetSnapshot],
    height: u64,
) -> Result<WitnessVerifyReportV1> {
    verify_witness_offline_with_verification(
        witness,
        spec,
        authority_history,
        height,
        WitnessVerificationV1::OnChainCertified { height },
    )
}

pub fn verify_witness_offline_with_verification(
    witness: &TriggerWitnessV1,
    spec: &ChainSpec,
    authority_history: &[AuthenticatedValidatorSetSnapshot],
    height: u64,
    verification: WitnessVerificationV1,
) -> Result<WitnessVerifyReportV1> {
    witness
        .validate_wire_budget()
        .context("TriggerWitnessV1 wire budget check failed")?;

    let timeline = AuthorityTimeline::from_spec_and_history(spec, authority_history, height)
        .context("build validator authority timeline from signed spec")?;
    let set = timeline.set_at(height);
    let validator = set
        .validators
        .iter()
        .find(|v| v.id == witness.witness_prover_id)
        .with_context(|| {
            format!(
                "witness prover '{}' is not in validator set at height {height}",
                witness.witness_prover_id
            )
        })?;

    if !validator_status_is_active(&validator.status) {
        bail!(
            "witness prover '{}' is not active at height {height} (status '{}')",
            validator.id,
            validator.status
        );
    }

    let require_hybrid = spec.consensus_params.vote_v4_active_at(height);

    let pk = validator.verifying_key(require_hybrid).with_context(|| {
        format!(
            "decode verifying key for witness prover '{}' at height {height}",
            validator.id
        )
    })?;

    witness
        .verify(&pk, require_hybrid)
        .context("TriggerWitnessV1 signature verification failed")?;

    Ok(WitnessVerifyReportV1 {
        schema_version: 1,
        ok: true,
        digest: witness.canonical_digest(),
        height,
        require_hybrid,
        prover_active_at_height: true,
        wire_budget_ok: true,
        signature_ok: true,
        witness: WitnessViewV1::from_witness(witness, verification),
    })
}

fn load_signed_chain_spec(
    spec_path: Option<&Path>,
    spec_sig_path: Option<&Path>,
    spec_pubkey: Option<&str>,
) -> Result<ChainSpec> {
    let resolved_path: PathBuf = match spec_path {
        Some(p) => p.to_path_buf(),
        None => std::env::var("CREG_CHAIN_SPEC_PATH")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .context(
                "signed chain spec path is required (--spec <PATH> or CREG_CHAIN_SPEC_PATH)",
            )?,
    };

    let raw = std::fs::read_to_string(&resolved_path)
        .with_context(|| format!("read chain spec from {}", resolved_path.display()))?;
    let spec: ChainSpec = serde_json::from_str(&raw).context("parse ChainSpec JSON")?;

    let allow_unsigned = std::env::var("CREG_ALLOW_UNSIGNED_SPEC")
        .ok()
        .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true"))
        && spec_sig_path.is_none()
        && spec_pubkey.is_none();

    if !allow_unsigned {
        let sig_path: PathBuf = match spec_sig_path {
            Some(p) => p.to_path_buf(),
            None => std::env::var("CREG_CHAIN_SPEC_SIG_PATH")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(format!("{}.sig", resolved_path.display()))),
        };
        let sig_hex = std::fs::read_to_string(&sig_path)
            .with_context(|| {
                format!(
                    "read chain spec signature at {} (pass --spec-sig or CREG_CHAIN_SPEC_SIG_PATH)",
                    sig_path.display()
                )
            })?
            .trim()
            .to_string();

        let pubkey_hex = spec_pubkey
            .map(str::to_string)
            .or_else(|| {
                std::env::var("CREG_SPEC_SIGNING_PUBKEY")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| {
                option_env!("CREG_SPEC_SIGNING_PUBKEY")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .context("chain spec signing public key required (--spec-pubkey or CREG_SPEC_SIGNING_PUBKEY)")?;

        spec.verify_signature(&sig_hex, &pubkey_hex)
            .context("chain spec signature verification failed")?;
    }

    Ok(spec)
}

fn is_64_hex(s: &str) -> bool {
    let clean = s
        .trim()
        .strip_prefix("sha256:")
        .or_else(|| s.trim().strip_prefix("0x"))
        .unwrap_or(s.trim());
    clean.len() == 64 && clean.bytes().all(|b| b.is_ascii_hexdigit())
}

fn read_bounded_json(path: &Path) -> Result<String> {
    let meta =
        std::fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        bail!("'{}' must be a regular non-symlink file", path.display());
    }
    if meta.len() == 0 || meta.len() > MAX_WITNESS_FILE_BYTES {
        bail!(
            "'{}' size {} bytes outside allowed 1..={MAX_WITNESS_FILE_BYTES} range",
            path.display(),
            meta.len()
        );
    }
    std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
}

fn extract_witness_from_bundle(
    bundle: &ProofBundleV2,
    expected_digest: Option<&str>,
) -> Option<TriggerWitnessV1> {
    let witness = match &bundle.status_transaction {
        Some(Transaction::RevokeV2 {
            witness: Some(w), ..
        }) => w.clone(),
        _ => return None,
    };
    if let Some(expected) = expected_digest {
        if !witness.canonical_digest().eq_ignore_ascii_case(expected) {
            return None;
        }
    }
    Some(witness)
}

pub struct VerifyWitnessArgs<'a> {
    pub target: &'a str,
    pub spec_path: Option<&'a Path>,
    pub spec_sig_path: Option<&'a Path>,
    pub spec_pubkey: Option<&'a str>,
    pub package: Option<&'a str>,
    pub height: Option<u64>,
    pub node_url: Option<&'a str>,
    pub json: bool,
}

pub async fn run_verify_witness(args: VerifyWitnessArgs<'_>) -> Result<()> {
    let spec = load_signed_chain_spec(args.spec_path, args.spec_sig_path, args.spec_pubkey)?;

    let target_path = Path::new(args.target);
    let (witness, authority_history, height, verification) = if target_path.exists() {
        let raw = read_bounded_json(target_path)?;
        if let Ok(witness) = serde_json::from_str::<TriggerWitnessV1>(&raw) {
            let h = args.height.unwrap_or(0);
            let ver = if args.height.is_some() {
                WitnessVerificationV1::OnChainCertified { height: h }
            } else {
                WitnessVerificationV1::GossipOnly
            };
            (witness, Vec::new(), h, ver)
        } else if let Ok(bundle) = serde_json::from_str::<ProofBundleV2>(&raw) {
            let witness = extract_witness_from_bundle(&bundle, None)
                .context("proof bundle file does not contain a RevokeV2 TriggerWitnessV1")?;
            let h = args
                .height
                .or_else(|| bundle.status_leaf.as_ref().map(|l| l.height))
                .unwrap_or(bundle.tip_height);
            (
                witness,
                bundle.authority_history,
                h,
                WitnessVerificationV1::OnChainCertified { height: h },
            )
        } else {
            bail!(
                "file '{}' did not decode as TriggerWitnessV1 or ProofBundleV2",
                target_path.display()
            );
        }
    } else if is_64_hex(args.target) {
        let clean_digest = args
            .target
            .trim()
            .strip_prefix("sha256:")
            .or_else(|| args.target.trim().strip_prefix("0x"))
            .unwrap_or(args.target.trim());
        let canonical = args.package.context(
            "when verifying a witness by digest, pass --package <ecosystem:name@version>",
        )?;
        let node = args
            .node_url
            .map(String::from)
            .or_else(|| std::env::var("CREG_NODE_URL").ok())
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
        let bundle = resolver::light_client::fetch_proof_bundle(canonical, &node)
            .await?
            .with_context(|| format!("no proof bundle found for '{canonical}' at {node}"))?;
        let witness =
            extract_witness_from_bundle(&bundle, Some(clean_digest)).with_context(|| {
                format!(
                    "proof bundle for '{canonical}' did not contain witness digest {clean_digest}"
                )
            })?;
        let h = args
            .height
            .or_else(|| bundle.status_leaf.as_ref().map(|l| l.height))
            .unwrap_or(bundle.tip_height);
        (
            witness,
            bundle.authority_history,
            h,
            WitnessVerificationV1::OnChainCertified { height: h },
        )
    } else {
        bail!(
            "target '{}' is neither an existing witness file nor a 64-char hex digest",
            evidence_panel::safe_text(args.target)
        );
    };

    let report = verify_witness_offline_with_verification(
        &witness,
        &spec,
        &authority_history,
        height,
        verification,
    )?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "\n  {} Trigger witness verified offline (wire budget + prover membership + signature)",
            "✓".green().bold()
        );
        println!("  digest   {}", report.digest);
        println!(
            "  height   {} (hybrid={})",
            report.height, report.require_hybrid
        );
        for line in evidence_panel::format_witness_lines(&report.witness) {
            println!("{line}");
        }
        println!();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::crypto::{SignatureScheme, SigningKey};
    use common::vote_v4::{TriggerSpecWireV1, MAX_TRIGGER_WITNESS_WIRE_BYTES};
    use common::KernelViolationCode;
    use std::collections::BTreeMap;

    fn sample_witness(prover_id: &str) -> TriggerWitnessV1 {
        let mut env_overrides = BTreeMap::new();
        env_overrides.insert("NODE_ENV".to_string(), "production".to_string());
        TriggerWitnessV1 {
            schema_version: 1,
            package_canonical: "npm:cond-mal@1.0.0".to_string(),
            content_hash: "aa".repeat(32),
            manifest_sha256: "bb".repeat(32),
            sandbox_policy_bundle_id: "bundle-v1".to_string(),
            trigger_spec: TriggerSpecWireV1 {
                schema_version: 1,
                env_overrides,
                clock_offset_secs: 86_400,
                entry_function: Some("runHook".to_string()),
                args: vec!["--trigger".to_string()],
            },
            observed_violations: vec![KernelViolationCode::KbNetUndeclared.as_str().to_string()],
            graph_canonical_digest: "cc".repeat(32),
            witness_prover_id: prover_id.to_string(),
            prover_key_id: String::new(),
            prover_signature: String::new(),
            prover_pq_signature: None,
        }
    }

    fn build_spec_with_validator(
        id: &str,
        key: &SigningKey,
        hybrid_activation: Option<u64>,
    ) -> ChainSpec {
        use common::chain_spec::{
            Bootnode, ConsensusParams, FeatureFlags, L1Config, Network, Phase, Services, Signing,
            ValidatorSetSpec, ValidatorSpecEntry, CURRENT_SPEC_VERSION,
        };
        let pk = key.public_key();
        let (ed_pk, pq_pk_opt) = if let Some((ed, pq)) = pk.hybrid_parts() {
            (hex::encode(ed), Some(hex::encode(pq)))
        } else {
            (pk.to_hex(), None)
        };
        let mut spec = ChainSpec {
            spec_version: CURRENT_SPEC_VERSION,
            chain_id: "creg-testnet-1".into(),
            network: Network::Testnet,
            phase: Phase::Alpha,
            genesis_time: "2026-05-01T00:00:00Z".into(),
            genesis_hash: format!("0x{}", "00".repeat(32)),
            consensus_params: ConsensusParams {
                consensus_encoding: None,
                block_time_seconds: 5,
                vote_timeout_ms: 120_000,
                quorum_percentage: 67,
                min_validator_stake_wei: "100000000000000000000".into(),
                min_publisher_stake_wei: "1000000000000000000".into(),
                unbonding_period_seconds: 86_400,
                slash_penalty_low_bp: 200,
                slash_penalty_medium_bp: 1_000,
                slash_penalty_critical_bp: 3_000,
                max_validators: 50,
                namespace_authority_activation_height: None,
                namespace_authority_policies: vec![],
                hybrid_signature_activation_height: hybrid_activation,
                vote_v4_quorum: None,
            },
            feature_flags: FeatureFlags {
                zk_validation: false,
                ml_validation: true,
                wasm_sandbox: true,
                private_registries: true,
                cross_chain: false,
                insurance: false,
                threshold_encryption: false,
            },
            l1: L1Config {
                name: "sepolia".into(),
                chain_id: 11_155_111,
                block_explorer: "https://sepolia.etherscan.io".into(),
                min_finality_blocks: 6,
            },
            contracts: BTreeMap::new(),
            bootnodes: vec![Bootnode {
                id: "boot-1".into(),
                operator: "test".into(),
                region: "local".into(),
                multiaddr: "/ip4/127.0.0.1/tcp/9000".into(),
            }],
            validator_set: ValidatorSetSpec {
                version: 1,
                last_updated: "2026-05-01T00:00:00Z".into(),
                epoch_block_height: 0,
                validators: vec![ValidatorSpecEntry {
                    id: id.to_string(),
                    alias: id.to_string(),
                    pubkey: ed_pk,
                    eth_address: format!("0x{}", "01".repeat(20)),
                    stake: 100,
                    reputation: 100,
                    status: "active".into(),
                    pq_pubkey: pq_pk_opt,
                }],
            },
            services: Services {
                ipfs_gateway: "http://127.0.0.1:8081".into(),
                ipfs_api: "http://127.0.0.1:5001".into(),
                faucet: "http://127.0.0.1:8082".into(),
                explorer: "http://127.0.0.1:3007".into(),
                metrics: "http://127.0.0.1:9090".into(),
            },
            support: Default::default(),
            signing: Signing {
                signature_algorithm: "ed25519".into(),
                signing_key_pubkey_hex: "00".repeat(32),
                detached_signature_url: String::new(),
            },
        };
        spec.genesis_hash = spec.compute_genesis_hash().unwrap();
        spec
    }

    #[test]
    fn verify_witness_offline_accepts_valid_classical_and_hybrid_prover() {
        let classical_key = SigningKey::generate(SignatureScheme::Ed25519, &mut rand::rngs::OsRng);
        let classical_spec = build_spec_with_validator("val-1", &classical_key, None);
        let mut w_classical = sample_witness("val-1");
        w_classical.sign_with(&classical_key).unwrap();
        let rep_classical = verify_witness_offline(&w_classical, &classical_spec, &[], 5).unwrap();
        assert!(rep_classical.ok);
        assert!(!rep_classical.require_hybrid);
        assert!(rep_classical.prover_active_at_height);

        let hybrid_key = SigningKey::generate(
            SignatureScheme::HybridEd25519MlDsa44V1,
            &mut rand::rngs::OsRng,
        );
        let hybrid_spec = build_spec_with_validator("val-h", &hybrid_key, Some(10));
        let mut w_hybrid = sample_witness("val-h");
        w_hybrid.sign_with(&hybrid_key).unwrap();
        let rep_hybrid = verify_witness_offline(&w_hybrid, &hybrid_spec, &[], 12).unwrap();
        assert!(rep_hybrid.ok);
        assert!(rep_hybrid.require_hybrid);
        assert!(rep_hybrid.witness.hybrid);
    }

    #[test]
    fn verify_witness_offline_rejects_classical_after_hybrid_activation() {
        let classical_key = SigningKey::generate(SignatureScheme::Ed25519, &mut rand::rngs::OsRng);
        let spec = build_spec_with_validator("val-1", &classical_key, Some(10));
        let mut witness = sample_witness("val-1");
        witness.sign_with(&classical_key).unwrap();

        let err = verify_witness_offline(&witness, &spec, &[], 10).unwrap_err();
        assert!(
            format!("{err:#}").contains("PQ") || format!("{err:#}").contains("key"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn verify_witness_offline_rejects_tampered_witness_and_unknown_prover() {
        let key = SigningKey::generate(SignatureScheme::Ed25519, &mut rand::rngs::OsRng);
        let spec = build_spec_with_validator("val-1", &key, None);

        let mut tampered = sample_witness("val-1");
        tampered.sign_with(&key).unwrap();
        tampered
            .observed_violations
            .push(KernelViolationCode::KbPtraceEscape.as_str().to_string());
        assert!(verify_witness_offline(&tampered, &spec, &[], 3).is_err());

        let stranger_key = SigningKey::generate(SignatureScheme::Ed25519, &mut rand::rngs::OsRng);
        let mut unknown_prover = sample_witness("stranger");
        unknown_prover.sign_with(&stranger_key).unwrap();
        let err = verify_witness_offline(&unknown_prover, &spec, &[], 3).unwrap_err();
        assert!(format!("{err:#}").contains("not in validator set"));
    }

    #[test]
    fn verify_witness_offline_rejects_oversized_wire_budget() {
        let key = SigningKey::generate(SignatureScheme::Ed25519, &mut rand::rngs::OsRng);
        let spec = build_spec_with_validator("val-1", &key, None);
        let mut witness = sample_witness("val-1");
        witness.trigger_spec.args = vec!["A".repeat(900); 6];
        witness.prover_key_id = key.key_id();
        let sig = key.sign(witness.signing_message().as_bytes());
        witness.prover_signature = hex::encode(sig.ed25519_bytes());

        let wire_len = serde_json::to_vec(&witness).unwrap().len();
        assert!(wire_len > MAX_TRIGGER_WITNESS_WIRE_BYTES);
        let err = verify_witness_offline(&witness, &spec, &[], 1).unwrap_err();
        assert!(format!("{err:#}").contains("wire"));
    }
}
