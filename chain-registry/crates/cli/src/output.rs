use common::{TrustVerdict, VerdictSource, VerdictStatus};

pub fn print_verdict(v: &TrustVerdict) {
    let reset  = "\x1b[0m";
    let bold   = "\x1b[1m";
    let dim    = "\x1b[2m";
    let color  = v.status.ansi_color();
    let label  = v.status.label();

    println!(
        "\n  {} {}{}{} {}{}{}",
        "▶",
        color, bold, label, reset,
        dim, v.package.canonical()
    );
    println!("  {}reset{}", dim, reset);

    match &v.status {
        VerdictStatus::Verified { block_hash, content_hash, findings, ipfs_cid: _ } => {
            if !block_hash.is_empty() {
                println!("  {} block:   {}", dim, &block_hash[..std::cmp::min(16, block_hash.len())]);
            }
            println!("  {} sha256:  {}", dim, &content_hash[..std::cmp::min(16, content_hash.len())]);
            print_findings(findings, &v.source);
        }
        VerdictStatus::Revoked { reason, findings } => {
            println!("  {}reason:  {}{}", color, reason, reset);
            print_findings(findings, &v.source);
        }
        VerdictStatus::Unverified => {
            println!("  {}Package is in the pending pool — consensus not yet complete.{}", dim, reset);
        }
        VerdictStatus::Unknown => {
            println!("  {}Package not found in the chain registry.{}", dim, reset);
        }
    }
}

fn print_findings(findings: &[common::Finding], source: &VerdictSource) {
    if findings.is_empty() { return; }
    
    let reset = "\x1b[0m";
    let dim   = "\x1b[2m";
    let bold  = "\x1b[1m";

    println!("  {}findings:{}", dim, reset);
    for f in findings {
        let color = match f.severity {
            common::FindingSeverity::Critical => "\x1b[31;1m", // Bright Red
            common::FindingSeverity::High     => "\x1b[31m",   // Red
            common::FindingSeverity::Medium   => "\x1b[33m",   // Yellow
            common::FindingSeverity::Low      => "\x1b[34m",   // Blue
        };
        println!(
            "     {}●{} [{:?}] {}{}{} ({}:{})",
            color, reset, f.severity, bold, f.title, reset, f.file, f.line.unwrap_or(0)
        );
    }

    let source_label = match source {
        VerdictSource::Cache { expires_at } =>
            format!("cache (expires {})", expires_at.format("%H:%M UTC")),
        VerdictSource::Chain { node_url } =>
            format!("live node ({})", node_url),
    };
    println!("  {}source:  {}{}\n", dim, source_label, reset);
}
