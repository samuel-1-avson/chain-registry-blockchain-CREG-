// crates/cli/src/info.rs
// `creg info <package>` — detailed package record + v4 consensus evidence (`ECA-05`).

use crate::evidence_panel::{self, ProofSummaryLine};
use anyhow::{Context, Result};
use chrono::Utc;
use colored::Colorize;
use common::{DisplayText, Finding};
use registry_view::{PackageViewV4, ProofBundleV2, WitnessViewV1};

pub async fn run(
    package: &str,
    ecosystem: Option<&str>,
    node_url: Option<&str>,
    json: bool,
) -> Result<()> {
    let base = node_url.map(String::from).unwrap_or_else(|| {
        std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
    });

    // Build canonical
    let canonical = if package.contains(':') {
        package.to_string()
    } else {
        let eco = ecosystem.unwrap_or("npm");
        let (name, version) = if let Some(idx) = package.rfind('@') {
            (&package[..idx], &package[idx + 1..])
        } else {
            (package, "latest")
        };
        format!("{}:{}@{}", eco, name, version)
    };

    let encoded = urlencoding::encode(&canonical);
    let client = reqwest::Client::new();

    // Fetch v1 package record and v4 evidence views concurrently.
    let pkg_url = format!("{}/v1/packages/{}", base.trim_end_matches('/'), encoded);
    let proof_url = format!(
        "{}/v1/packages/{}/proof",
        base.trim_end_matches('/'),
        encoded
    );

    let (pkg_res, proof_res, v4_evidence, v4_bundle_res) = tokio::join!(
        client
            .get(&pkg_url)
            .timeout(std::time::Duration::from_secs(10))
            .send(),
        client
            .get(&proof_url)
            .timeout(std::time::Duration::from_secs(5))
            .send(),
        evidence_panel::fetch_v4_evidence(&client, &base, &canonical),
        resolver::light_client::fetch_proof_bundle(&canonical, &base),
    );

    let (v4_view, witnesses) = v4_evidence;
    let proof_bundle = v4_bundle_res.ok().flatten();

    let record: serde_json::Value = match pkg_res {
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND && v4_view.is_some() => {
            serde_json::json!({ "canonical": canonical })
        }
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
            anyhow::bail!("Package '{}' not found in the chain registry", canonical);
        }
        Ok(resp) if !resp.status().is_success() => {
            if v4_view.is_some() {
                serde_json::json!({ "canonical": canonical })
            } else {
                anyhow::bail!("Registry returned HTTP {}", resp.status());
            }
        }
        Ok(resp) => resp
            .json()
            .await
            .context("Failed to parse package record")?,
        Err(err) => {
            return Err(err).context("Failed to reach registry node");
        }
    };

    let proof: Option<serde_json::Value> = match proof_res {
        Ok(r) if r.status().is_success() => r.json().await.ok(),
        _ => None,
    };

    if json {
        let mut combined = record.clone();
        if let Some(obj) = combined.as_object_mut() {
            if let Some(p) = proof {
                obj.insert("spv_proof".into(), p);
            }
            if let Some(ref view) = v4_view {
                obj.insert("v4_status".into(), serde_json::to_value(view)?);
            }
            if !witnesses.is_empty() {
                obj.insert("witnesses".into(), serde_json::to_value(&witnesses)?);
            }
            if let Some(ref bundle) = proof_bundle {
                obj.insert(
                    "proof_bundle_summary".into(),
                    serde_json::json!({
                        "schema": bundle.schema,
                        "tip_height": bundle.tip_height,
                        "tip_block_hash": bundle.tip_block_hash,
                        "tip_timestamp_unix": bundle.tip_timestamp_unix,
                        "publication_height": bundle.publication_height,
                        "publication_block_hash": bundle.publication_block_hash,
                        "status_root": bundle.status_root,
                        "quorum": bundle.quorum,
                    }),
                );
            }
        }
        println!("{}", serde_json::to_string_pretty(&combined)?);
        return Ok(());
    }

    print_info(
        &record,
        proof.as_ref(),
        v4_view.as_ref(),
        &witnesses,
        proof_bundle.as_ref(),
    );
    Ok(())
}

/// String field of a node response, sanitised and bounded for the terminal
/// (ECA-17 / eco-adv-07). Truncation counts characters, never bytes, so a
/// multi-byte value cannot make a slice panic.
fn field(value: &serde_json::Value, key: &str, default: &str, max_chars: usize) -> String {
    let raw = value.get(key).and_then(|v| v.as_str()).unwrap_or(default);
    DisplayText::bounded(raw, max_chars).into_string()
}

fn build_proof_summary(bundle: &ProofBundleV2) -> ProofSummaryLine {
    let has_cert = bundle
        .status_transaction
        .as_ref()
        .is_some_and(|tx| match tx {
            common::Transaction::RevokeV2 {
                quorum_certificate, ..
            } => quorum_certificate.is_some(),
            common::Transaction::QuarantineV1 { .. }
            | common::Transaction::QuarantineResolutionV1 { .. } => true,
            _ => false,
        });
    ProofSummaryLine {
        tip_height: bundle.tip_height,
        tip_timestamp_unix: bundle.tip_timestamp_unix,
        status_proven_by_smt: bundle.status_root.is_some() && bundle.status_proof.is_some(),
        quorum: bundle.quorum,
        certified: has_cert,
        qc_note: has_cert.then(|| {
            "certificate attached (run `creg verify` for full cryptographic verification)"
                .to_string()
        }),
    }
}

fn print_info(
    record: &serde_json::Value,
    proof: Option<&serde_json::Value>,
    v4_view: Option<&PackageViewV4>,
    witnesses: &[WitnessViewV1],
    proof_bundle: Option<&ProofBundleV2>,
) {
    let canonical = field(record, "canonical", "?", 200);
    let status = field(record, "status", "unknown", 32);
    let publisher = field(record, "publisher", "unknown", 200);
    let content_hash = field(record, "content_hash", "?", 16);
    let ipfs_cid = field(record, "ipfs_cid", "?", 128);
    let published = field(record, "published_at", "?", 64);
    let block_hash = field(record, "block_hash", "?", 16);
    let revocation = record
        .get("revocation_reason")
        .and_then(|v| v.as_str())
        .map(DisplayText::for_terminal);
    let quarantine = record
        .get("quarantine_reason")
        .and_then(|v| v.as_str())
        .map(DisplayText::for_terminal);

    let status_str = match status.as_str() {
        "verified" => format!("{}", "VERIFIED".green().bold()),
        "revoked" => format!("{}", "REVOKED".red().bold()),
        "quarantined" => format!("{}", "QUARANTINED".magenta().bold()),
        "pending" => format!("{}", "PENDING".yellow().bold()),
        _ => status.dimmed().to_string(),
    };

    println!("\n{}", canonical.white().bold());
    println!("{}", "─".repeat(canonical.chars().count().max(40)).dimmed());
    println!("  Status:       {}", status_str);
    println!("  Publisher:    {}", publisher.cyan());
    println!("  Published:    {}", published.dimmed());
    if block_hash != "?" {
        println!("  Block:        {}", block_hash);
    }
    println!("  Content SHA:  {}", content_hash);
    println!("  IPFS CID:     {}", ipfs_cid);

    if let Some(reason) = revocation {
        println!("  {} Revocation:  {}", "⊘".red(), reason.to_string().red());
    }
    if let Some(reason) = quarantine {
        println!(
            "  {} Quarantine:  {} (no override; fails closed to REVOKED at expiry)",
            "⧗".magenta(),
            reason.to_string().magenta()
        );
    }

    let findings: Vec<Finding> = record
        .get("findings")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    if let Some(view) = v4_view {
        let proof_summary = proof_bundle.map(build_proof_summary);
        let lines = evidence_panel::render_evidence_panel_from_view(
            view,
            witnesses,
            proof_summary.as_ref(),
            &[],
            &findings,
            Utc::now().timestamp(),
        );
        println!("\n  {}", "Consensus Evidence (v4)".bold());
        evidence_panel::print_panel_lines(&lines);
    } else if !witnesses.is_empty() {
        println!("\n  {}", "Consensus Evidence (Witnesses)".bold());
        for w in witnesses {
            for line in evidence_panel::format_witness_lines(w) {
                println!("  {line}");
            }
        }
    }

    if let Some(p) = proof {
        println!("\n  {}", "SPV Proof".bold());
        let merkle_root = field(p, "merkle_root", "?", 16);
        let proof_height = p
            .get("block_height")
            .and_then(|v| v.as_u64())
            .map(|h| h.to_string())
            .unwrap_or_else(|| "?".into());
        let siblings = p
            .get("siblings")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        println!("    Block height: {}", proof_height);
        println!("    Merkle root:  {}", merkle_root);
        println!("    Proof depth:  {} sibling nodes", siblings);
    }

    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::CregFindingCategoryV1;
    use registry_view::{
        CommitteeSummaryV1, PackageStatusViewV1, QuorumLabelV1, PACKAGE_VIEW_SCHEMA,
    };
    use std::collections::BTreeMap;

    #[test]
    fn info_renders_v4_evidence_section_cleanly() {
        let mut families = BTreeMap::new();
        families.insert("gemini".to_string(), 2);
        let committee = CommitteeSummaryV1 {
            effective_size: 4,
            approvals: 2,
            quarantine_reviews: 2,
            rejections: 0,
            abstentions: 0,
            model_families: families,
            hybrid_signers: 4,
            classical_signers: 0,
        };
        let view = PackageViewV4 {
            schema: PACKAGE_VIEW_SCHEMA.to_string(),
            canonical: DisplayText::for_terminal("npm:demo@1.2.3"),
            package: DisplayText::for_terminal("npm:demo"),
            version: DisplayText::for_terminal("1.2.3"),
            content_hash: "aa".repeat(32),
            status: PackageStatusViewV1::Quarantined {
                height: 120,
                expires_at_unix: 1_800_000_000,
                reason_code: CregFindingCategoryV1::RemoteCodeFetch,
                committee: Some(committee.clone()),
            },
            quorum: QuorumLabelV1::V4Quorum,
            committee: Some(committee),
            witness_digests: Vec::new(),
        };
        let lines = evidence_panel::render_evidence_panel_from_view(
            &view,
            &[],
            None,
            &[],
            &[],
            1_799_990_000,
        );
        let joined = lines.join("\n");
        assert!(joined.contains("gemini ×2"));
        assert!(joined.contains("signers 4 hybrid / 0 classical"));
        assert!(joined.contains("remote_code_fetch"));
    }
}
