use colored::Colorize;
use common::{TrustVerdict, VerdictSource, VerdictStatus};

pub fn print_verdict(v: &TrustVerdict) {
    let label = v.status.label();
    let canonical = v.package.canonical();

    let colored_label = match &v.status {
        VerdictStatus::Verified { .. } => label.green().bold().to_string(),
        VerdictStatus::Revoked { .. } => label.red().bold().to_string(),
        VerdictStatus::Unverified => label.yellow().bold().to_string(),
        VerdictStatus::Unknown => label.dimmed().bold().to_string(),
    };

    println!("\n  {} {} {}", "▶", colored_label, canonical.dimmed());

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
                    &block_hash[..std::cmp::min(16, block_hash.len())]
                );
            }
            println!(
                "  {} sha256:  {}",
                " ".dimmed(),
                &content_hash[..std::cmp::min(16, content_hash.len())]
            );
            print_findings(findings, &v.source);
        }
        VerdictStatus::Revoked { reason, findings } => {
            println!("  {} {}", "reason:".red(), reason.red());
            print_findings(findings, &v.source);
        }
        VerdictStatus::Unverified => {
            println!(
                "  {}",
                "Package is in the pending pool — consensus not yet complete.".dimmed()
            );
        }
        VerdictStatus::Unknown => {
            println!("  {}", "Package not found in the chain registry.".dimmed());
        }
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
            f.title.bold(),
            f.file,
            f.line.unwrap_or(0)
        );
    }

    let source_label = match source {
        VerdictSource::Cache { expires_at } => {
            format!("cache (expires {})", expires_at.format("%H:%M UTC"))
        }
        VerdictSource::Chain { node_url } => format!("live node ({})", node_url),
    };
    println!("  {} {}\n", "source:".dimmed(), source_label.dimmed());
}
