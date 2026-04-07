// crates/validator/src/lib.rs
// Mechanical consensus validator — runs all three stages concurrently
// and returns a signed vote to the consensus engine.

pub mod diff;
pub mod llm;
pub mod pgp;
pub mod report;
pub mod reputation;
pub mod sandbox;
pub mod static_analysis;
pub mod typosquat;
pub mod wasm_sandbox;

use anyhow::Result;
use common::{Finding, PublishRequest, ValidatorVote};
use report::{AuditProof, ValidationReport};
use reputation::{assess_publisher, final_decision, FinalDecision};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationResult {
    pub vote: ValidatorVote,
    pub pgp_fingerprint: Option<String>,
    pub findings: Vec<Finding>,
}

/// Run all three validator stages concurrently and produce a vote.
/// Stage 1: static AST analysis
/// Stage 2: sandbox behavioral analysis
/// Stage 3: publisher reputation assessment
pub async fn validate_package(
    req: &PublishRequest,
    tarball: &[u8],
    _privkey: &str,
    prev_manifest: Option<&common::PackageManifest>,
) -> Result<ValidationResult> {
    let canonical = req.id.canonical();
    tracing::info!("Starting 3-stage validation for {}", canonical);

    let node_url =
        std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".into());

    // ── All three stages run concurrently ─────────────────────────────────────
    let (static_result, rep_result) = tokio::join!(
        static_analysis::run(tarball, &req.manifest),
        assess_publisher(&req.publisher_pubkey, &node_url),
    );

    let mut report = ValidationReport::new(req.id.clone());
    report.apply_static(static_result?);

    let sandbox_result = sandbox::run(&req.id, tarball, &req.manifest).await?;
    report.apply_sandbox(sandbox_result.clone());

    // ── Differential Analysis ────────────────────────────────────────────────
    let diff_result = diff::analyze(&req.manifest, &sandbox_result, prev_manifest, None);
    report.apply_diff(diff_result);

    // ── Web-of-Trust PGP Verification ────────────────────────────────────────
    let mut pgp_fingerprint = None;
    if let (Some(sig_hex), Some(pubk_hex)) = (&req.pgp_signature, &req.pgp_public_key) {
        if let (Ok(sig_bytes), Ok(pubk_bytes)) = (hex::decode(sig_hex), hex::decode(pubk_hex)) {
            let pgp_res = pgp::verify_signature(tarball, &sig_bytes, &pubk_bytes);
            pgp_fingerprint = pgp_res.fingerprint.clone();
            report.apply_pgp(pgp_res);
        }
    }

    let rep = rep_result.unwrap_or_else(|_| reputation::ReputationAssessment {
        confidence_delta: 0,
        publisher_pubkey: req.publisher_pubkey.clone(),
        notes: vec!["Reputation check unreachable — neutral".into()],
    });

    for note in &rep.notes {
        tracing::debug!("[{}] rep: {}", canonical, note);
    }

    // ── Final decision combines all three stages ───────────────────────────────
    let sandbox_has_critical = sandbox_result
        .findings
        .iter()
        .any(|f| matches!(f.severity, common::FindingSeverity::Critical));
    let mut decision = final_decision(report.has_critical_findings(), sandbox_has_critical, rep.confidence_delta);

    // ── AAA (Automated AI Auditor) Stage ──────────────────────────────────────
    // Only runs when CREG_AAA_ENABLED=true is explicitly set. The AAA auditor
    // is an external service that may not be deployed — calling it unconditionally
    // causes silent failures.
    let aaa_enabled = std::env::var("CREG_AAA_ENABLED")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    if decision.is_reject() && aaa_enabled {
        tracing::info!("[{}] Triggering Automated AI Audit (AAA)...", canonical);
        match aaa_audit(&report, tarball).await {
            Ok(proof) => {
                // Only overrule if the proof's verdict explicitly clears the package
                // and the proof includes a valid signature.
                if proof.verdict == "cleared" && !proof.signature.is_empty() {
                    report.aaa_verdict = Some(proof);
                    decision = FinalDecision::Approve { confidence: 85 };
                    tracing::info!(
                        "[{}] AAA cleared the package with a signed proof",
                        canonical
                    );
                } else {
                    tracing::warn!(
                        "[{}] AAA returned verdict='{}' — rejection stands",
                        canonical,
                        proof.verdict
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    "[{}] AAA audit failed: {} — original rejection stands",
                    canonical,
                    e
                );
            }
        }
    } else if decision.is_reject() {
        tracing::debug!(
            "[{}] AAA is not enabled (set CREG_AAA_ENABLED=true to activate)",
            canonical
        );
    }

    let vote = if decision.is_reject() {
        let base = decision
            .reject_reason()
            .unwrap_or("Validation failed")
            .to_string();
        let detail = if report.has_critical_findings() {
            format!("{}; {}", base, report.critical_finding_summary())
        } else {
            base
        };
        tracing::warn!("[{}] REJECT — {}", canonical, detail);
        ValidatorVote::Reject { reason: detail }
    } else {
        if let reputation::FinalDecision::ApproveWithWarning { warning, .. } = &decision {
            tracing::warn!("[{}] APPROVE WITH WARNING — {}", canonical, warning);
        } else {
            tracing::info!("[{}] APPROVE", canonical);
        }
        ValidatorVote::Approve
    };

    Ok(ValidationResult {
        vote,
        pgp_fingerprint,
        findings: report.findings,
    })
}

/// Deep Audit call to an external AI Auditor provider.
async fn aaa_audit(report: &ValidationReport, tarball: &[u8]) -> Result<AuditProof> {
    let auditor_url = std::env::var("AAA_AUDITOR_URL")
        .unwrap_or_else(|_| "http://ai-auditor-central.service.cluster.local/v1/audit".into());

    tracing::info!("Dispatching Deep Audit to {}", auditor_url);

    #[derive(serde::Serialize)]
    struct AuditReq<'a> {
        package: &'a common::PackageId,
        findings: &'a [Finding],
        tarball_hex: String,
    }

    let req = AuditReq {
        package: &report.package,
        findings: &report.findings,
        tarball_hex: hex::encode(tarball),
    };

    let resp = reqwest::Client::new()
        .post(&auditor_url)
        .json(&req)
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("AI Auditor returned error: {}", resp.status());
    }

    let proof: AuditProof = resp.json().await?;
    Ok(proof)
}
