// crates/cli/src/blocks.rs
// `creg blocks` — non-interactive chain explorer command.

use anyhow::{Context, Result};
use colored::Colorize;
use serde_json::Value;

pub async fn run(node_url: Option<&str>, limit: usize) -> Result<()> {
    anyhow::ensure!(
        limit > 0 && limit <= 100,
        "Block limit must be between 1 and 100"
    );
    let api_base = crate::config_file::effective_node_url(node_url)
        .trim_end_matches('/')
        .to_string();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    // 1. Get chain stats
    let stats: Value = client
        .get(format!("{}/v1/chain/stats", api_base))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("Failed to fetch chain stats")?;

    let tip = stats["tip_height"]
        .as_u64()
        .context("Chain stats missing tip_height")?;
    let pkg_count = stats["package_count"]
        .as_u64()
        .context("Chain stats missing package_count")?;

    println!(
        "\n{} {} {}",
        "⛓".bold(),
        "Chain Registry Explorer".bold(),
        "⛓".bold()
    );
    println!(
        "{} Height: {} | Packages: {}\n",
        "→".cyan(),
        tip.to_string().yellow(),
        pkg_count.to_string().yellow()
    );
    println!(
        "Chain: {} | Genesis: {}\n",
        stats["chain_id"].as_str().unwrap_or("unknown"),
        stats["genesis_hash"].as_str().unwrap_or("unknown")
    );

    println!(
        "{:<8} {:<18} {:<10} {:<30}",
        "HEIGHT", "PROPOSER", "TXS", "CONTENT / EVENTS"
    );
    println!("{}", "─".repeat(80).dimmed());

    // 2. Fetch blocks
    let start = tip.saturating_sub(limit as u64 - 1);
    for h in (start..=tip).rev() {
        let block = client
            .get(format!("{}/v1/blocks/{}", api_base, h))
            .send()
            .await
            .with_context(|| format!("Failed to fetch block {h}"))?
            .error_for_status()
            .with_context(|| format!("Block {h} request failed"))?
            .json::<Value>()
            .await?;
        anyhow::ensure!(
            block["header"]["height"].as_u64() == Some(h),
            "Invalid block response for height {h}"
        );
        render_block(&block);
    }

    Ok(())
}

fn render_block(block: &Value) {
    let height = block["header"]["height"].as_u64().unwrap_or(0);
    let proposer = block["header"]["proposer_id"].as_str().unwrap_or("?");
    let txs = block["transactions"].as_array();
    let tx_count = txs.map(|v| v.len()).unwrap_or(0);

    let height_str = format!("#{}", height).white().bold();

    print!(
        "{:<16} {:<18} {:<10} ",
        height_str,
        proposer.dimmed(),
        tx_count
    );

    if let Some(tx_list) = txs {
        if tx_list.is_empty() {
            println!("{}", "Empty Block".dimmed());
        } else {
            for (i, tx) in tx_list.iter().enumerate() {
                if i > 0 {
                    print!("\n{:<44} ", "");
                }
                render_tx(tx);
            }
            println!();
        }
    } else {
        println!();
    }
}

fn render_tx(tx: &Value) {
    let tx_type = tx["type"].as_str().unwrap_or("unknown");
    match tx_type {
        "publish" | "publish_authorized_v1" => {
            let record = tx.get("record").unwrap_or(tx);
            let eco = record["id"]["ecosystem"].as_str().unwrap_or("?");
            let name = record["id"]["name"].as_str().unwrap_or("?");
            let ver = record["id"]["version"].as_str().unwrap_or("?");
            let canonical = format!("{}:{}@{}", eco, name, ver);
            let status = record["status"].as_str().unwrap_or("?");
            let color_status = if status == "Verified" {
                "Verified".green()
            } else {
                status.yellow()
            };
            print!("{} {} [{}]", "📦".cyan(), canonical.bold(), color_status);
        }
        "revoke" => {
            let canonical = tx["package_canonical"].as_str().unwrap_or("?");
            let reason = tx["reason"].as_str().unwrap_or("?");
            print!(
                "{} {} {} ({})",
                "⚡".red(),
                "REVOKED".red().bold(),
                canonical,
                reason.dimmed()
            );
        }
        _ => {
            print!("{} {}", "❓".dimmed(), tx_type.dimmed());
        }
    }
}
