use colored::Colorize;
use common::{DisplayText, TrustVerdict, VerdictSource, VerdictStatus};

/// ECA-17: every node- or package-derived string goes through this before it
/// reaches the terminal.
fn safe(text: &str) -> String {
    DisplayText::for_terminal(text).into_string()
}

pub fn print_verdict(v: &TrustVerdict) {
    let label = v.status.label();
    let canonical = safe(&v.package.canonical());

    let colored_label = match &v.status {
        VerdictStatus::Verified { .. } => label.green().bold().to_string(),
        VerdictStatus::Revoked { .. } => label.red().bold().to_string(),
        VerdictStatus::Quarantined { .. } => label.magenta().bold().to_string(),
        VerdictStatus::Unverified => label.yellow().bold().to_string(),
        VerdictStatus::Unknown => label.dimmed().bold().to_string(),
    };

    println!("\n  ▶ {} {}", colored_label, canonical.dimmed());

    match &v.status {
        VerdictStatus::Verified {
            block_hash,
            content_hash,
            findings,
            ipfs_cid: _,
        } => {
            if !block_hash.is_empty() {
                println!(
                    "  {} block:   {}",
                    " ".dimmed(),
                    safe_prefix(block_hash, 16)
                );
            }
            println!(
                "  {} sha256:  {}",
                " ".dimmed(),
                safe_prefix(content_hash, 16)
            );
            print_risk_summary(v.deterministic_risk.as_ref());
            print_findings(findings, &v.source);
        }
        VerdictStatus::Revoked { reason, findings } => {
            println!("  {} {}", "reason:".red(), safe(reason).red());
            print_risk_summary(v.deterministic_risk.as_ref());
            print_findings(findings, &v.source);
        }
        VerdictStatus::Quarantined {
            reason,
            expires_at_unix,
            ..
        } => {
            println!("  {} {}", "reason:".magenta(), safe(reason).magenta());
            let deadline = expires_at_unix
                .and_then(|secs| chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0))
                .map(|at| at.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "no deadline reported".into());
            println!(
                "  {} fails closed → REVOKED at {} · override: none",
                " ".dimmed(),
                deadline
            );
            print_risk_summary(v.deterministic_risk.as_ref());
        }
        VerdictStatus::Unverified => {
            println!(
                "  {}",
                "Package is in the pending pool — consensus not yet complete.".dimmed()
            );
            print_pipeline_stages(v);
        }
        VerdictStatus::Unknown => {
            println!("  {}", "Package not found in the chain registry.".dimmed());
        }
    }
}

/// First `max` characters of an untrusted hex-ish field, sanitised.
fn safe_prefix(text: &str, max: usize) -> String {
    safe(&text.chars().take(max).collect::<String>())
}

fn print_pipeline_stages(v: &TrustVerdict) {
    let evidence = safe(v.evidence_phase.as_deref().unwrap_or("unknown"));
    let block = safe(v.block_phase.as_deref().unwrap_or("unknown"));
    let chain = safe(v.chain_status.as_deref().unwrap_or("unknown"));
    println!(
        "  {} evidence: {} | block: {} | chain: {}",
        " ".dimmed(),
        evidence.cyan(),
        block.cyan(),
        chain.cyan(),
    );
}

/// MAL-004/LLM-002: surface the risk band, deterministic finding counts, and
/// the advisory (LLM) lane — clearly labeled as not part of consensus.
fn print_risk_summary(risk: Option<&common::DeterministicRiskSummary>) {
    let Some(r) = risk else { return };

    let band = if r.band.is_empty() {
        "unrated".to_string()
    } else {
        safe(&r.band)
    };
    let band = band.as_str();
    let colored_band = match band.to_ascii_lowercase().as_str() {
        "critical" | "high" => band.red().bold().to_string(),
        "medium" | "elevated" => band.yellow().bold().to_string(),
        "low" | "minimal" => band.green().bold().to_string(),
        _ => band.bold().to_string(),
    };

    println!(
        "  {} band {} — deterministic score {} ({} critical, {} high)",
        "risk:".dimmed(),
        colored_band,
        r.deterministic_score,
        r.critical_findings,
        r.high_findings,
    );
    if r.advisory_findings > 0 || r.advisory_score > 0 {
        println!(
            "  {} score {} with {} finding(s) {}",
            "llm:".dimmed(),
            r.advisory_score,
            r.advisory_findings,
            "(advisory only — not part of consensus)".dimmed(),
        );
    }
}

fn print_findings(findings: &[common::Finding], source: &VerdictSource) {
    if findings.is_empty() {
        return;
    }

    println!("  {}", "findings:".dimmed());
    for f in findings {
        let severity_str = format!("[{:?}]", f.severity);
        let colored_severity = match f.severity {
            common::FindingSeverity::Critical => severity_str.red().bold().to_string(),
            common::FindingSeverity::High => severity_str.red().to_string(),
            common::FindingSeverity::Medium => severity_str.yellow().to_string(),
            common::FindingSeverity::Low => severity_str.blue().to_string(),
        };
        let bullet = match f.severity {
            common::FindingSeverity::Critical => "●".red().bold().to_string(),
            common::FindingSeverity::High => "●".red().to_string(),
            common::FindingSeverity::Medium => "●".yellow().to_string(),
            common::FindingSeverity::Low => "●".blue().to_string(),
        };
        println!(
            "     {} {} {} ({}:{})",
            bullet,
            colored_severity,
            safe(&f.title).bold(),
            safe(&f.file),
            f.line.unwrap_or(0)
        );
    }

    let source_label = match source {
        VerdictSource::Cache { expires_at } => {
            format!("cache (expires {})", expires_at.format("%H:%M UTC"))
        }
        VerdictSource::Chain { node_url } => format!("live node ({})", safe(node_url)),
    };
    println!("  {} {}\n", "source:".dimmed(), source_label.dimmed());
}
