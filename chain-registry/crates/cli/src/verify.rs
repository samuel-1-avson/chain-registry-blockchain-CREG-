// crates/cli/src/verify.rs
// `creg verify` — proof-carrying verification directly from `ProofBundleV2`
// (`ADR-019` §4.2, `ECA-13`). Removes any cached resolution pre-check.

use crate::evidence_panel;
use anyhow::{bail, Result};
use chrono::Utc;
use colored::Colorize;
use common::{DisplayText, PackageId};
use registry_view::{CommitteeSummaryV1, PackageStatusViewV1, QuorumLabelV1, WitnessViewV1};
use resolver::light_client::{
    default_checkpoint, verify_package_with_options, Checkpoint, VerifiedProofBundle, VerifyOptions,
};
use serde::{Deserialize, Serialize};

pub const PROOF_CARRYING_VERDICT_SCHEMA: &str = "creg.proof-carrying-verdict.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofCarryingVerdict {
    pub schema: String,
    pub canonical: DisplayText,
    pub verified: bool,
    pub decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quorum: Option<QuorumLabelV1>,
    pub certified: bool,
    pub tip_height: u64,
    pub tip_block_hash: String,
    pub tip_timestamp_unix: i64,
    pub publication_height: u64,
    pub publication_block_hash: String,
    pub status_height: u64,
    pub status_proven_by_smt: bool,
    pub status: PackageStatusViewV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committee: Option<CommitteeSummaryV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub witnesses: Vec<WitnessViewV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn build_proof_carrying_verdict(
    verified: &VerifiedProofBundle,
    witnesses: Vec<WitnessViewV1>,
    now_unix: i64,
) -> ProofCarryingVerdict {
    let effective_status = verified.status_view.clone().at(now_unix);
    let (ok, decision) = match &effective_status {
        PackageStatusViewV1::Verified { .. } => (true, "allow"),
        PackageStatusViewV1::Quarantined { .. } => (false, "block_quarantined"),
        PackageStatusViewV1::Revoked { .. } => (false, "block_revoked"),
        PackageStatusViewV1::Pending | PackageStatusViewV1::Unknown => (false, "block_unproven"),
    };

    let effective_size = verified
        .record
        .get_analysis_committee()
        .map(|c| {
            if c.effective_size > 0 {
                c.effective_size
            } else {
                c.size
            }
        })
        .filter(|n| *n > 0)
        .unwrap_or(verified.record.validator_signatures.len() as u32);
    let committee = if verified.record.validator_signatures.is_empty() && effective_size == 0 {
        None
    } else {
        Some(CommitteeSummaryV1::from_votes(
            effective_size,
            &verified.record.validator_signatures,
        ))
    };

    ProofCarryingVerdict {
        schema: PROOF_CARRYING_VERDICT_SCHEMA.to_string(),
        canonical: DisplayText::for_terminal(&verified.canonical),
        verified: ok,
        decision: decision.to_string(),
        quorum: Some(verified.quorum),
        certified: verified.certified,
        tip_height: verified.tip_height,
        tip_block_hash: verified.tip_block_hash.clone(),
        tip_timestamp_unix: verified.tip_timestamp_unix,
        publication_height: verified.publication_height,
        publication_block_hash: verified.publication_block_hash.clone(),
        status_height: verified.status_height,
        status_proven_by_smt: verified.status_proven_by_smt,
        status: effective_status,
        committee,
        witnesses,
        error: None,
    }
}

pub fn unproven_verdict(canonical: &str, error: impl Into<String>) -> ProofCarryingVerdict {
    ProofCarryingVerdict {
        schema: PROOF_CARRYING_VERDICT_SCHEMA.to_string(),
        canonical: DisplayText::for_terminal(canonical),
        verified: false,
        decision: "block_unproven".to_string(),
        quorum: None,
        certified: false,
        tip_height: 0,
        tip_block_hash: String::new(),
        tip_timestamp_unix: 0,
        publication_height: 0,
        publication_block_hash: String::new(),
        status_height: 0,
        status_proven_by_smt: false,
        status: PackageStatusViewV1::Unknown,
        committee: None,
        witnesses: Vec::new(),
        error: Some(DisplayText::for_terminal(&error.into()).into_string()),
    }
}

pub async fn run(
    package: &str,
    ecosystem: Option<&str>,
    node_url: Option<&str>,
    checkpoint: Option<&str>,
    json_out: bool,
    verify_opts: &VerifyOptions,
) -> Result<()> {
    let (qualified_eco, package) = match package.split_once(':') {
        Some((eco, rest)) if !eco.is_empty() && !rest.is_empty() => (Some(eco), rest),
        _ => (None, package),
    };
    if let (Some(qualified), Some(requested)) = (qualified_eco, ecosystem) {
        anyhow::ensure!(
            qualified == requested,
            "package ecosystem {qualified} conflicts with --ecosystem {requested}"
        );
    }
    let (name, version) = parse_pkg(package);
    anyhow::ensure!(!name.is_empty(), "package name is empty");
    anyhow::ensure!(version.as_deref() != Some(""), "package version is empty");
    let eco = qualified_eco
        .or(ecosystem)
        .map(str::to_owned)
        .unwrap_or_else(|| detect_eco().to_owned());
    let id = PackageId::new(eco, &name, version.as_deref().unwrap_or("latest"));

    let url = node_url.map(String::from).unwrap_or_else(|| {
        std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
    });

    // Load checkpoint from file or use genesis from the verified chain spec.
    let cp = match checkpoint {
        Some(path) => {
            let raw = std::fs::read_to_string(path)?;
            serde_json::from_str::<Checkpoint>(&raw)?
        }
        None => default_checkpoint().await?,
    };

    if !json_out {
        println!(
            "\n  {} Proof-carrying verification for {}",
            "→".cyan(),
            DisplayText::for_terminal(&id.canonical())
                .to_string()
                .white()
                .bold()
        );
        println!(
            "  {} Node:       {}",
            "·".dimmed(),
            evidence_panel::safe_text(&url).dimmed()
        );
        println!("  {} Checkpoint: height={}", "·".dimmed(), cp.height);
        println!(
            "  {} Quorum:     {}",
            "·".dimmed(),
            if verify_opts.require_v4_quorum {
                "require_v4_quorum"
            } else {
                "default (v4 from activation)"
            }
        );
    }

    let canonical = id.canonical();
    match verify_package_with_options(&canonical, &url, Some(&cp), verify_opts).await {
        Ok(Some(verified)) => {
            let now_unix = Utc::now().timestamp();
            let client = reqwest::Client::new();
            let (_view, witnesses) =
                evidence_panel::fetch_v4_evidence(&client, &url, &canonical).await;

            let pcv = build_proof_carrying_verdict(&verified, witnesses.clone(), now_unix);
            if json_out {
                println!("{}", serde_json::to_string_pretty(&pcv)?);
            } else {
                let panel =
                    evidence_panel::render_verified_proof_panel(&verified, &witnesses, now_unix);
                evidence_panel::print_panel_lines(&panel);
            }

            if !pcv.verified {
                bail!(
                    "Verification did not clear {} ({})",
                    canonical,
                    pcv.decision
                );
            }
            Ok(())
        }
        Ok(None) => {
            let msg = format!("no v4 proof bundle found for {canonical}");
            if json_out {
                let pcv = unproven_verdict(&canonical, &msg);
                println!("{}", serde_json::to_string_pretty(&pcv)?);
            } else {
                println!(
                    "\n  {} {} — {}",
                    "✗".red().bold(),
                    evidence_panel::safe_text(&canonical).red(),
                    "proof bundle unavailable; failing closed".red()
                );
            }
            bail!("{msg}");
        }
        Err(err) => {
            let msg = format!("{err:#}");
            if json_out {
                let pcv = unproven_verdict(&canonical, &msg);
                println!("{}", serde_json::to_string_pretty(&pcv)?);
            } else {
                println!(
                    "\n  {} {} — proof verification FAILED: {}",
                    "✗".red().bold(),
                    evidence_panel::safe_text(&canonical).red(),
                    evidence_panel::safe_text(&msg).red()
                );
            }
            bail!("Light-client verification failed: {err:#}");
        }
    }
}

fn parse_pkg(raw: &str) -> (String, Option<String>) {
    if let Some(rest) = raw.strip_prefix('@') {
        if let Some(idx) = rest.rfind('@') {
            return (
                format!("@{}", &rest[..idx]),
                Some(rest[idx + 1..].to_string()),
            );
        }
        return (raw.to_string(), None);
    }
    match raw.rfind('@') {
        Some(idx) => (raw[..idx].to_string(), Some(raw[idx + 1..].to_string())),
        None => (raw.to_string(), None),
    }
}

fn detect_eco() -> &'static str {
    let cwd = std::env::current_dir().unwrap_or_default();
    if cwd.join("package.json").exists() {
        return "npm";
    }
    if cwd.join("Cargo.toml").exists() {
        return "cargo";
    }
    if cwd.join("requirements.txt").exists() {
        return "pypi";
    }
    if cwd.join("Gemfile").exists() {
        return "rubygems";
    }
    "npm"
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{ChainRecord, CregFindingCategoryV1, PackageStatus, VerdictStatus};
    use registry_view::RevocationAuthorityV1;

    fn sample_verified_bundle(status_view: PackageStatusViewV1) -> VerifiedProofBundle {
        let record = ChainRecord {
            content_hash: "aa".repeat(32),
            analysis_committee_size: 7,
            analysis_effective_size: 7,
            ..ChainRecord::default()
        };
        VerifiedProofBundle {
            canonical: "npm:demo@1.0.0".to_string(),
            tip_height: 50,
            tip_block_hash: "bb".repeat(32),
            tip_timestamp_unix: 1_795_680_000,
            publication_height: 45,
            publication_block_hash: "cc".repeat(32),
            record,
            status: VerdictStatus::Verified {
                content_hash: "aa".repeat(32),
                block_hash: "cc".repeat(32),
                ipfs_cid: String::new(),
                findings: Vec::new(),
            },
            status_view,
            quorum: QuorumLabelV1::V4Quorum,
            certified: true,
            status_height: 50,
            status_proven_by_smt: true,
        }
    }

    #[test]
    fn proof_carrying_verdict_maps_lattice_decisions_and_sanitizes_fields() {
        let now_unix = 1_795_680_100;
        let ok_bundle = sample_verified_bundle(PackageStatusViewV1::Verified {
            height: 45,
            block_hash: "cc".repeat(32),
        });
        let pcv_ok = build_proof_carrying_verdict(&ok_bundle, Vec::new(), now_unix);
        assert_eq!(pcv_ok.schema, PROOF_CARRYING_VERDICT_SCHEMA);
        assert!(pcv_ok.verified);
        assert_eq!(pcv_ok.decision, "allow");
        assert!(pcv_ok.status_proven_by_smt);

        let q_bundle = sample_verified_bundle(PackageStatusViewV1::Quarantined {
            height: 48,
            expires_at_unix: now_unix + 3600,
            reason_code: CregFindingCategoryV1::InstallHookAbuse,
            committee: None,
        });
        let pcv_q = build_proof_carrying_verdict(&q_bundle, Vec::new(), now_unix);
        assert!(!pcv_q.verified);
        assert_eq!(pcv_q.decision, "block_quarantined");

        // Expired quarantine transitions cleanly to block_revoked.
        let pcv_exp = build_proof_carrying_verdict(&q_bundle, Vec::new(), now_unix + 7200);
        assert!(!pcv_exp.verified);
        assert_eq!(pcv_exp.decision, "block_revoked");
        assert!(matches!(
            pcv_exp.status,
            PackageStatusViewV1::Revoked {
                authority: RevocationAuthorityV1::QuarantineExpiry,
                ..
            }
        ));

        let rev_bundle = sample_verified_bundle(PackageStatusViewV1::Revoked {
            height: 49,
            authority: RevocationAuthorityV1::CommitteeQuorum,
            reason: DisplayText::for_terminal("evil\u{1b}[31mpayload"),
            witness_digest: Some("dd".repeat(32)),
            certificate_digest: None,
        });
        let pcv_rev = build_proof_carrying_verdict(&rev_bundle, Vec::new(), now_unix);
        assert!(!pcv_rev.verified);
        assert_eq!(pcv_rev.decision, "block_revoked");
        assert!(matches!(
            pcv_rev.status,
            PackageStatusViewV1::Revoked { ref reason, .. } if !reason.to_string().contains('\u{1b}')
        ));

        let mut unp = unproven_verdict("npm:bad\u{1b}[2J@1.0.0", "bad\u{07}error");
        assert!(!unp.verified);
        assert_eq!(unp.decision, "block_unproven");
        assert!(!unp.canonical.to_string().contains('\u{1b}'));
        assert!(!unp.error.take().unwrap().contains('\u{07}'));

        let _ = PackageStatus::Verified;
    }
}
