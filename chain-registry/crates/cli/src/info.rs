// crates/cli/src/info.rs
// `creg info <package>` — detailed package record, history, risk score, insurance.

use anyhow::{Context, Result};
use colored::Colorize;

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

    // Fetch package record
    let pkg_url = format!("{}/v1/packages/{}", base.trim_end_matches('/'), encoded);
    let resp = client
        .get(&pkg_url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("Failed to reach registry node")?;

    if resp.status() == 404 {
        anyhow::bail!("Package '{}' not found in the chain registry", canonical);
    }
    if !resp.status().is_success() {
        anyhow::bail!("Registry returned HTTP {}", resp.status());
    }

    let record: serde_json::Value = resp
        .json()
        .await
        .context("Failed to parse package record")?;

    // Fetch SPV proof / verification details
    let proof_url = format!(
        "{}/v1/packages/{}/proof",
        base.trim_end_matches('/'),
        encoded
    );
    let proof: Option<serde_json::Value> = match client
        .get(&proof_url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r.json().await.ok(),
        _ => None,
    };

    if json {
        let mut combined = record.clone();
        if let Some(p) = proof {
            if let Some(obj) = combined.as_object_mut() {
                obj.insert("spv_proof".into(), p);
            }
        }
        println!("{}", serde_json::to_string_pretty(&combined)?);
        return Ok(());
    }

    print_info(&record, proof.as_ref());
    Ok(())
}

/// String field of a node response, sanitised and bounded for the terminal
/// (ECA-17 / eco-adv-07). Truncation counts characters, never bytes, so a
/// multi-byte value cannot make a slice panic.
fn field(value: &serde_json::Value, key: &str, default: &str, max_chars: usize) -> String {
    let raw = value.get(key).and_then(|v| v.as_str()).unwrap_or(default);
    common::DisplayText::bounded(raw, max_chars).into_string()
}

fn print_info(record: &serde_json::Value, proof: Option<&serde_json::Value>) {
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
        .map(common::DisplayText::for_terminal);
    let quarantine = record
        .get("quarantine_reason")
        .and_then(|v| v.as_str())
        .map(common::DisplayText::for_terminal);

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
