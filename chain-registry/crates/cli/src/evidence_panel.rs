// crates/cli/src/evidence_panel.rs
// Shared terminal and TUI evidence panels (`ADR-019` §4.2, `ECA-05`, `ECA-07`, `ECA-17`).
//
// Every untrusted wire string is routed through `DisplayText::for_terminal` /
// `DisplayText::bounded` before formatting so ANSI escapes, bidi overrides, and
// control characters cannot reach the operator's terminal.

use chrono::{DateTime, Utc};
use colored::Colorize;
use common::{CregFindingCategoryV1, DisplayText, Finding, FindingSeverity};
use registry_view::{
    CommitteeSummaryV1, PackageStatusViewV1, PackageViewV4, QuorumLabelV1, RevocationAuthorityV1,
    WitnessBundleV1, WitnessVerificationV1, WitnessViewV1,
};
use resolver::light_client::VerifiedProofBundle;
use std::time::Duration;

const EVIDENCE_FETCH_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofSummaryLine {
    pub tip_height: u64,
    pub tip_timestamp_unix: i64,
    pub status_proven_by_smt: bool,
    pub quorum: QuorumLabelV1,
    pub certified: bool,
    pub qc_note: Option<String>,
}

pub fn safe_text(input: &str) -> String {
    DisplayText::for_terminal(input).into_string()
}

pub fn safe_bounded(input: &str, max_chars: usize) -> String {
    DisplayText::bounded(input, max_chars).into_string()
}

pub fn safe_prefix(input: &str, chars: usize) -> String {
    let prefix: String = input.trim().chars().take(chars).collect();
    safe_text(&prefix)
}

pub const fn quorum_label_str(quorum: QuorumLabelV1) -> &'static str {
    match quorum {
        QuorumLabelV1::LegacyV3Quorum => "legacy_v3_quorum",
        QuorumLabelV1::V4Quorum => "v4_quorum",
    }
}

pub const fn revocation_authority_label(authority: RevocationAuthorityV1) -> &'static str {
    match authority {
        RevocationAuthorityV1::Owner => "owner",
        RevocationAuthorityV1::Controller => "controller",
        RevocationAuthorityV1::CommitteeQuorum => "committee_quorum",
        RevocationAuthorityV1::LegacySingleValidator => "legacy_single_validator",
        RevocationAuthorityV1::QuarantineExpiry => "quarantine_expiry",
    }
}

pub const fn category_label(category: CregFindingCategoryV1) -> &'static str {
    match category {
        CregFindingCategoryV1::CredentialExfiltration => "credential_exfiltration",
        CregFindingCategoryV1::RemoteCodeFetch => "remote_code_fetch",
        CregFindingCategoryV1::InstallHookAbuse => "install_hook_abuse",
        CregFindingCategoryV1::ObfuscatedPayload => "obfuscated_payload",
        CregFindingCategoryV1::NativeDropper => "native_dropper",
        CregFindingCategoryV1::Backdoor => "backdoor",
        CregFindingCategoryV1::AuthBypass => "auth_bypass",
        CregFindingCategoryV1::CryptoWeakening => "crypto_weakening",
        CregFindingCategoryV1::Typosquat => "typosquat",
        CregFindingCategoryV1::DependencyConfusion => "dependency_confusion",
        CregFindingCategoryV1::DestructivePayload => "destructive_payload",
        CregFindingCategoryV1::PromptInjectionBait => "prompt_injection_bait",
        CregFindingCategoryV1::Other => "other",
    }
}

/// Render a human-readable relative duration (`in 61h 12m`, `3m`, `45s`, or `expired Xh Ym ago`).
pub fn format_relative_duration(delta_secs: i64) -> String {
    let abs = delta_secs.unsigned_abs();
    let mag = if abs >= 3600 {
        let hours = abs / 3600;
        let mins = (abs % 3600) / 60;
        format!("{hours}h {mins}m")
    } else if abs >= 60 {
        let mins = abs / 60;
        format!("{mins}m")
    } else {
        format!("{abs}s")
    };
    if delta_secs < 0 {
        format!("expired {mag} ago")
    } else {
        mag
    }
}

/// Format a trigger clock skew offset concisely (`+30d 0h`, `-2h 15m`, `0s`).
pub fn format_signed_offset(offset_secs: i64) -> String {
    if offset_secs == 0 {
        return "0s".to_string();
    }
    let sign = if offset_secs < 0 { "-" } else { "+" };
    let abs = offset_secs.unsigned_abs();
    if abs >= 86_400 {
        let days = abs / 86_400;
        let hours = (abs % 86_400) / 3600;
        format!("{sign}{days}d {hours}h")
    } else if abs >= 3600 {
        let hours = abs / 3600;
        let mins = (abs % 3600) / 60;
        format!("{sign}{hours}h {mins}m")
    } else if abs >= 60 {
        let mins = abs / 60;
        format!("{sign}{mins}m")
    } else {
        format!("{sign}{abs}s")
    }
}

fn format_utc_timestamp(ts_unix: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts_unix, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| format!("unix:{ts_unix}"))
}

pub fn format_committee_lines(committee: &CommitteeSummaryV1) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!(
        "  committee  {} assigned · {} approve · {} quarantine_review · {} reject · {} abstain",
        committee.effective_size,
        committee.approvals,
        committee.quarantine_reviews,
        committee.rejections,
        committee.abstentions
    ));

    let families = if committee.model_families.is_empty() {
        "none".to_string()
    } else {
        committee
            .model_families
            .iter()
            .map(|(family, count)| format!("{} ×{}", safe_bounded(family, 48), count))
            .collect::<Vec<_>>()
            .join(" · ")
    };
    lines.push(format!(
        "  families   {families}      signers {} hybrid / {} classical",
        committee.hybrid_signers, committee.classical_signers
    ));
    lines
}

/// Class B witness lines: violations come exclusively from the static dictionary
/// (`ViolationViewV1`), never raw wire text, and trigger env/args are sanitized.
pub fn format_witness_lines(witness: &WitnessViewV1) -> Vec<String> {
    let mut lines = Vec::new();
    let verification = match witness.verification {
        WitnessVerificationV1::OnChainCertified { height } => {
            format!("on_chain_certified@h={height}")
        }
        WitnessVerificationV1::OnChainUncertified { height } => {
            format!("on_chain_uncertified@h={height}")
        }
        WitnessVerificationV1::GossipOnly => "gossip_only".to_string(),
    };
    let scheme = if witness.hybrid {
        "hybrid"
    } else {
        "classical"
    };
    lines.push(format!(
        "  witness    {} · prover {} (key {}) · {} · {}",
        safe_prefix(&witness.digest, 12),
        safe_bounded(witness.prover_id.as_ref(), 40),
        safe_prefix(&witness.prover_key_id, 12),
        scheme,
        verification
    ));

    let rendered_violations = if witness.violations.is_empty() {
        "none".to_string()
    } else {
        witness
            .violations
            .iter()
            .map(|v| {
                if let Some(code) = v.code {
                    format!("{} ({})", code.as_str(), code.title())
                } else if let Some(unk) = &v.unknown {
                    format!("{unk} ({})", safe_text(&v.title))
                } else {
                    safe_text(&v.title)
                }
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    lines.push(format!("    violations: {rendered_violations}"));

    let mut trigger_parts = vec![format!(
        "clock_offset={}",
        format_signed_offset(witness.clock_offset_secs)
    )];
    if let Some(entry) = &witness.entry_function {
        trigger_parts.push(format!("entry={}", safe_bounded(entry.as_ref(), 64)));
    }
    if !witness.env_overrides.is_empty() {
        let envs = witness
            .env_overrides
            .iter()
            .take(4)
            .map(|(k, v)| {
                format!(
                    "{}={}",
                    safe_bounded(k.as_ref(), 32),
                    safe_bounded(v.as_ref(), 64)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if witness.env_overrides.len() > 4 {
            format!(" (+{})", witness.env_overrides.len() - 4)
        } else {
            String::new()
        };
        trigger_parts.push(format!("env=[{envs}{suffix}]"));
    }
    if !witness.args.is_empty() {
        let args = witness
            .args
            .iter()
            .take(4)
            .map(|a| safe_bounded(a.as_ref(), 64))
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if witness.args.len() > 4 {
            format!(" (+{})", witness.args.len() - 4)
        } else {
            String::new()
        };
        trigger_parts.push(format!("args=[{args}{suffix}]"));
    }
    trigger_parts.push(format!(
        "graph={}",
        safe_prefix(&witness.graph_canonical_digest, 12)
    ));
    lines.push(format!("    trigger:    {}", trigger_parts.join(" · ")));
    lines
}

/// Class A deterministic static/sandbox findings lines (`ECA-17` sanitized).
pub fn format_class_a_findings_lines(findings: &[Finding]) -> Vec<String> {
    if findings.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::with_capacity(findings.len() + 1);
    lines.push(format!(
        "  class_a    {} deterministic finding(s)",
        findings.len()
    ));
    for finding in findings {
        let sev = match finding.severity {
            FindingSeverity::Critical => "critical",
            FindingSeverity::High => "high",
            FindingSeverity::Medium => "medium",
            FindingSeverity::Low => "low",
        };
        let loc = match finding.line {
            Some(line) => format!("{}:{}", safe_bounded(&finding.file, 64), line),
            None => safe_bounded(&finding.file, 64),
        };
        lines.push(format!(
            "    - [{}] {}: {} ({})",
            sev,
            safe_bounded(&finding.id, 48),
            safe_bounded(&finding.title, 96),
            loc
        ));
    }
    lines
}

fn push_proof_line(
    lines: &mut Vec<String>,
    quorum: QuorumLabelV1,
    proof_summary: Option<&ProofSummaryLine>,
    fallback_height: Option<u64>,
    now_unix: i64,
) {
    let quorum_str = quorum_label_str(quorum);
    match proof_summary {
        Some(proof) => {
            let root_kind = if proof.status_proven_by_smt {
                "status_root"
            } else {
                "publication_proof"
            };
            let age_secs = now_unix.saturating_sub(proof.tip_timestamp_unix).max(0);
            let age_str = format_relative_duration(age_secs);
            let cert_str = if proof.certified {
                "certified"
            } else {
                "uncertified"
            };
            let note = proof
                .qc_note
                .as_deref()
                .map(|n| format!(" · {}", safe_text(n)))
                .unwrap_or_default();
            lines.push(format!(
                "  proof      {root_kind} @ h={} (tip {age_str} old) · {quorum_str} · {cert_str}{note}",
                proof.tip_height
            ));
        }
        None => {
            if let Some(height) = fallback_height {
                lines.push(format!("  proof      h={height} · {quorum_str}"));
            } else {
                lines.push(format!("  proof      {quorum_str}"));
            }
        }
    }
}

/// Render an ANSI-free evidence panel usable by both terminal commands and the TUI.
pub fn render_evidence_panel_from_view(
    view: &PackageViewV4,
    witnesses: &[WitnessViewV1],
    proof_summary: Option<&ProofSummaryLine>,
    categories: &[CregFindingCategoryV1],
    findings: &[Finding],
    now_unix: i64,
) -> Vec<String> {
    let mut lines = Vec::new();
    let canonical = safe_text(view.canonical.as_ref());
    let effective = view.status.clone().at(now_unix);

    let mut all_categories: Vec<CregFindingCategoryV1> = categories.to_vec();
    if let PackageStatusViewV1::Quarantined { reason_code, .. } = &effective {
        if !all_categories.contains(reason_code) {
            all_categories.insert(0, *reason_code);
        }
    }

    match &effective {
        PackageStatusViewV1::Quarantined {
            height,
            expires_at_unix,
            committee,
            ..
        } => {
            lines.push(format!(
                "✗ BLOCKED  {canonical} — QUARANTINED (Class C committee hold)"
            ));
            let remaining = expires_at_unix.saturating_sub(now_unix);
            let countdown = format_relative_duration(remaining);
            let deadline_str = format_utc_timestamp(*expires_at_unix);
            lines.push(format!(
                "  fails closed → REVOKED at {deadline_str} (in {countdown})"
            ));
            if let Some(summary) = committee.as_ref().or(view.committee.as_ref()) {
                lines.extend(format_committee_lines(summary));
            }
            if !all_categories.is_empty() {
                let cats = all_categories
                    .iter()
                    .map(|c| category_label(*c))
                    .collect::<Vec<_>>()
                    .join(" · ");
                lines.push(format!("  categories {cats}"));
            }
            lines.extend(format_class_a_findings_lines(findings));
            for witness in witnesses {
                lines.extend(format_witness_lines(witness));
            }
            push_proof_line(
                &mut lines,
                view.quorum,
                proof_summary,
                Some(*height),
                now_unix,
            );
            lines.push("  override   none (ECO-INV-02)".to_string());
        }
        PackageStatusViewV1::Revoked {
            height,
            authority,
            reason,
            witness_digest,
            certificate_digest,
        } => {
            let auth_str = revocation_authority_label(*authority);
            lines.push(format!("✗ BLOCKED  {canonical} — REVOKED ({auth_str})"));
            lines.push(format!("  reason     {}", safe_text(reason.as_ref())));
            if witness_digest.is_some() || certificate_digest.is_some() {
                let mut ev = Vec::new();
                if let Some(wd) = witness_digest {
                    ev.push(format!("witness={}", safe_prefix(wd, 16)));
                }
                if let Some(cd) = certificate_digest {
                    ev.push(format!("certificate={}", safe_prefix(cd, 16)));
                }
                lines.push(format!("  artifacts  {}", ev.join(" · ")));
            }
            if let Some(summary) = view.committee.as_ref() {
                lines.extend(format_committee_lines(summary));
            }
            if !all_categories.is_empty() {
                let cats = all_categories
                    .iter()
                    .map(|c| category_label(*c))
                    .collect::<Vec<_>>()
                    .join(" · ");
                lines.push(format!("  categories {cats}"));
            }
            lines.extend(format_class_a_findings_lines(findings));
            for witness in witnesses {
                lines.extend(format_witness_lines(witness));
            }
            push_proof_line(
                &mut lines,
                view.quorum,
                proof_summary,
                Some(*height),
                now_unix,
            );
            lines.push("  override   none (ECO-INV-02)".to_string());
        }
        PackageStatusViewV1::Verified { height, block_hash } => {
            lines.push(format!("✓ VERIFIED {canonical}"));
            lines.push(format!(
                "  record     h={height} · block={} · sha256={}",
                safe_prefix(block_hash, 16),
                safe_prefix(&view.content_hash, 16)
            ));
            if let Some(summary) = view.committee.as_ref() {
                lines.extend(format_committee_lines(summary));
            }
            if !all_categories.is_empty() {
                let cats = all_categories
                    .iter()
                    .map(|c| category_label(*c))
                    .collect::<Vec<_>>()
                    .join(" · ");
                lines.push(format!("  categories {cats}"));
            }
            lines.extend(format_class_a_findings_lines(findings));
            for witness in witnesses {
                lines.extend(format_witness_lines(witness));
            }
            push_proof_line(
                &mut lines,
                view.quorum,
                proof_summary,
                Some(*height),
                now_unix,
            );
        }
        PackageStatusViewV1::Pending => {
            lines.push(format!("⚠ PENDING  {canonical}"));
            if let Some(summary) = view.committee.as_ref() {
                lines.extend(format_committee_lines(summary));
            }
            lines.extend(format_class_a_findings_lines(findings));
            for witness in witnesses {
                lines.extend(format_witness_lines(witness));
            }
            push_proof_line(&mut lines, view.quorum, proof_summary, None, now_unix);
        }
        PackageStatusViewV1::Unknown => {
            lines.push(format!("⚠ UNKNOWN  {canonical}"));
            push_proof_line(&mut lines, view.quorum, proof_summary, None, now_unix);
        }
    }

    lines
}

/// Build an evidence panel directly from a locally verified `VerifiedProofBundle`.
pub fn render_verified_proof_panel(
    verified: &VerifiedProofBundle,
    witnesses: &[WitnessViewV1],
    now_unix: i64,
) -> Vec<String> {
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
    let committee =
        CommitteeSummaryV1::from_votes(effective_size, &verified.record.validator_signatures);

    let mut view = PackageViewV4::new(
        &verified.canonical,
        &verified.record.content_hash,
        verified.status_view.clone(),
        verified.quorum,
    );
    view.committee = Some(committee);
    view.witness_digests = witnesses.iter().map(|w| w.digest.clone()).collect();

    let qc_note = match &verified.status_view {
        PackageStatusViewV1::Quarantined {
            committee: Some(c), ..
        } => {
            let votes = c.approvals + c.quarantine_reviews + c.rejections;
            Some(format!("QC {votes}/{} verified locally", c.effective_size))
        }
        _ if verified.certified => Some("verified locally".to_string()),
        _ => None,
    };

    let proof_summary = ProofSummaryLine {
        tip_height: verified.tip_height,
        tip_timestamp_unix: verified.tip_timestamp_unix,
        status_proven_by_smt: verified.status_proven_by_smt,
        quorum: verified.quorum,
        certified: verified.certified,
        qc_note,
    };

    render_evidence_panel_from_view(
        &view,
        witnesses,
        Some(&proof_summary),
        &[],
        &verified.record.findings,
        now_unix,
    )
}

pub fn print_panel_lines(lines: &[String]) {
    println!();
    for line in lines {
        if line.starts_with("✗ BLOCKED") && line.contains("QUARANTINED") {
            println!("  {}", line.magenta().bold());
        } else if line.starts_with("✗ BLOCKED") {
            println!("  {}", line.red().bold());
        } else if line.starts_with("✓ VERIFIED") {
            println!("  {}", line.green().bold());
        } else if line.starts_with("⚠") {
            println!("  {}", line.yellow().bold());
        } else if line.trim_start().starts_with("override") {
            println!("  {}", line.trim_start().red());
        } else if line.trim_start().starts_with("fails closed") {
            println!("  {}", line.trim_start().magenta());
        } else if line.trim_start().starts_with("reason") {
            println!("  {}", line.trim_start().red());
        } else {
            println!("  {}", line.trim_start());
        }
    }
    println!();
}

/// Best-effort fetch of v4 package status and witness views from public read routes.
pub async fn fetch_v4_evidence(
    client: &reqwest::Client,
    node_url: &str,
    canonical: &str,
) -> (Option<PackageViewV4>, Vec<WitnessViewV1>) {
    let Some((name, version)) = common::status_tree::split_canonical(canonical) else {
        return (None, Vec::new());
    };
    let base = node_url.trim_end_matches('/');
    let enc_name = urlencoding::encode(name);
    let enc_ver = urlencoding::encode(version);
    let timeout = Duration::from_secs(EVIDENCE_FETCH_TIMEOUT_SECS);

    let status_url = format!("{base}/v1/public/v4/packages/{enc_name}/{enc_ver}/status");
    let witness_url = format!("{base}/v1/public/v4/packages/{enc_name}/{enc_ver}/witnesses");

    let (status_res, witness_res) = tokio::join!(
        client.get(&status_url).timeout(timeout).send(),
        client.get(&witness_url).timeout(timeout).send()
    );

    let status_view = match status_res {
        Ok(resp) if resp.status().is_success() => resp.json::<PackageViewV4>().await.ok(),
        _ => None,
    };
    let witnesses = match witness_res {
        Ok(resp) if resp.status().is_success() => resp
            .json::<WitnessBundleV1>()
            .await
            .map(|b| b.witnesses)
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    (status_view, witnesses)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::vote_v4::{TriggerSpecWireV1, TriggerWitnessV1};
    use common::KernelViolationCode;
    use registry_view::PACKAGE_VIEW_SCHEMA;
    use std::collections::BTreeMap;

    #[test]
    fn renders_class_c_quarantine_panel_with_countdown_committee_and_no_override() {
        let mut families = BTreeMap::new();
        families.insert("claude".to_string(), 1);
        families.insert("gemini".to_string(), 1);
        families.insert("qwen2.5-coder".to_string(), 2);

        let committee = CommitteeSummaryV1 {
            effective_size: 7,
            approvals: 1,
            quarantine_reviews: 4,
            rejections: 0,
            abstentions: 2,
            model_families: families,
            hybrid_signers: 7,
            classical_signers: 0,
        };

        let now_unix = 1_795_680_000; // 2026-11-24 ~13:20 UTC equivalent epoch
        let expires_at_unix = now_unix + 61 * 3600 + 12 * 60;

        let view = PackageViewV4 {
            schema: PACKAGE_VIEW_SCHEMA.to_string(),
            canonical: DisplayText::for_terminal("npm:left-padx@1.4.2"),
            package: DisplayText::for_terminal("npm:left-padx"),
            version: DisplayText::for_terminal("1.4.2"),
            content_hash: "aa".repeat(32),
            status: PackageStatusViewV1::Quarantined {
                height: 48_211,
                expires_at_unix,
                reason_code: CregFindingCategoryV1::CredentialExfiltration,
                committee: Some(committee.clone()),
            },
            quorum: QuorumLabelV1::V4Quorum,
            committee: Some(committee),
            witness_digests: Vec::new(),
        };

        let proof = ProofSummaryLine {
            tip_height: 48_211,
            tip_timestamp_unix: now_unix - 180,
            status_proven_by_smt: true,
            quorum: QuorumLabelV1::V4Quorum,
            certified: true,
            qc_note: Some("QC 5/7 verified locally".to_string()),
        };

        let lines = render_evidence_panel_from_view(
            &view,
            &[],
            Some(&proof),
            &[CregFindingCategoryV1::ObfuscatedPayload],
            &[],
            now_unix,
        );
        let joined = lines.join("\n");

        assert!(joined.contains("BLOCKED  npm:left-padx@1.4.2 — QUARANTINED"));
        assert!(joined.contains("in 61h 12m"));
        assert!(
            joined.contains("7 assigned · 1 approve · 4 quarantine_review · 0 reject · 2 abstain")
        );
        assert!(joined.contains("claude ×1 · gemini ×1 · qwen2.5-coder ×2"));
        assert!(joined.contains("signers 7 hybrid / 0 classical"));
        assert!(joined.contains("credential_exfiltration · obfuscated_payload"));
        assert!(joined.contains(
            "status_root @ h=48211 (tip 3m old) · v4_quorum · certified · QC 5/7 verified locally"
        ));
        assert!(joined.contains("override   none (ECO-INV-02)"));

        // Past expiry fails closed into REVOKED via QuarantineExpiry lattice step.
        let expired_lines = render_evidence_panel_from_view(
            &view,
            &[],
            Some(&proof),
            &[],
            &[],
            expires_at_unix + 5,
        );
        let expired_joined = expired_lines.join("\n");
        assert!(expired_joined.contains("REVOKED (quarantine_expiry)"));
        assert!(expired_joined.contains("override   none (ECO-INV-02)"));
    }

    #[test]
    fn renders_class_b_witness_using_static_dictionary_and_sanitised_trigger_fields() {
        let mut env_overrides = BTreeMap::new();
        env_overrides.insert(
            "EVIL\u{1b}[31m_KEY".to_string(),
            "val\u{202e}\u{1b}]52;c;ZXY=\u{07}".to_string(),
        );

        let witness = TriggerWitnessV1 {
            schema_version: 1,
            package_canonical: "npm:evil@1.0.0".to_string(),
            content_hash: "bb".repeat(32),
            manifest_sha256: "cc".repeat(32),
            sandbox_policy_bundle_id: "bundle-1".to_string(),
            trigger_spec: TriggerSpecWireV1 {
                schema_version: 1,
                env_overrides,
                clock_offset_secs: 30 * 86_400 + 3 * 3600,
                entry_function: Some("run\u{1b}[2JMain".to_string()),
                args: vec!["--flag\u{202e}bad".to_string()],
            },
            observed_violations: vec![
                KernelViolationCode::KbCanaryExfil.as_str().to_string(),
                "UNREGISTERED_RAW_WIRE_EXPLOIT_PROSE".to_string(),
            ],
            graph_canonical_digest: "dd".repeat(32),
            witness_prover_id: "val-01\u{1b}[31m".to_string(),
            prover_key_id: "ee".repeat(16),
            prover_signature: "ff".repeat(64),
            prover_pq_signature: None,
        };

        let wview = WitnessViewV1::from_witness(
            &witness,
            WitnessVerificationV1::OnChainCertified { height: 77 },
        );
        let lines = format_witness_lines(&wview);
        let joined = lines.join("\n");

        // Static dictionary title shown for known code; raw wire string NEVER shown for unknown code.
        assert!(joined.contains("KB-CANARY-EXFIL (Canary secret exfiltration)"));
        assert!(joined.contains("UNKNOWN("));
        assert!(!joined.contains("UNREGISTERED_RAW_WIRE_EXPLOIT_PROSE"));
        // Terminal escape/bidi sequences stripped.
        assert!(!joined.contains('\u{1b}'));
        assert!(!joined.contains('\u{202e}'));
        assert!(!joined.contains('\u{07}'));
        // Humanized clock offset & certified height.
        assert!(joined.contains("clock_offset=+30d 3h"));
        assert!(joined.contains("on_chain_certified@h=77"));
    }

    #[test]
    fn renders_class_a_deterministic_findings() {
        let findings = vec![Finding {
            id: "YARA-001".to_string(),
            title: "Reads SSH private key\u{1b}[31m".to_string(),
            severity: FindingSeverity::Critical,
            description: "desc".to_string(),
            file: "index.js".to_string(),
            line: Some(14),
        }];
        let lines = format_class_a_findings_lines(&findings);
        let joined = lines.join("\n");
        assert!(joined.contains("1 deterministic finding(s)"));
        assert!(joined.contains("[critical] YARA-001: Reads SSH private key[31m (index.js:14)"));
        assert!(!joined.contains('\u{1b}'));
    }
}
