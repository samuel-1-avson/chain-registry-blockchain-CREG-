// crates/cli/src/relayer_cli.rs
// L1 state-root relayer calldata simulation.
//
// ECA-26 (ADR-019 Increment 1): `--broadcast` was removed. The cross-chain
// "live" path never contacted an RPC and fabricated confirmed receipts, so the
// command now only simulates calldata encoding and labels every receipt
// SIMULATED. Real anchoring is performed by the node bridge.

use anyhow::Result;
use colored::Colorize;
use cross_chain::{CheckpointRelayQueue, L1RelayerConfig, L1RelayerService};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// Simulate anchoring `count` checkpoints (calldata encoding only; nothing is broadcast).
pub async fn soak(
    count: usize,
    start_height: u64,
    rpc_url: &str,
    contract_address: &str,
    json_output: bool,
) -> Result<()> {
    let dry_run = true;
    if !json_output {
        println!(
            "\n{}",
            "================================================================================"
                .cyan()
        );
        println!(
            "  {}",
            "ETHEREUM SEPOLIA L1 STATE ROOT RELAYER SOAK TEST"
                .bold()
                .cyan()
        );
        println!(
            "{}\n",
            "================================================================================"
                .cyan()
        );

        println!("{}", "1. Relayer Pipeline Configuration".bold().yellow());
        println!("  Target Contract:     {}", contract_address.cyan());
        println!("  Sepolia RPC URL:     {}", rpc_url.cyan());
        println!("  Target Chain ID:     {}", "11155111 (Sepolia)".cyan());
        println!(
            "  Checkpoints to Soak: {}",
            count.to_string().bold().green()
        );
        println!(
            "  Starting Height:     #{}",
            start_height.to_string().bold()
        );
        println!(
            "  Execution Mode:      {}",
            "Simulation only (calldata encoding; nothing is broadcast)".yellow()
        );
        println!(
            "  Anchoring Calldata:  {}",
            "anchorStateRoot(uint64,bytes32,bytes32) [0x6e96996d]".dimmed()
        );
        println!();
    }

    let queue_cap = (count * 2).max(100);
    let queue = Arc::new(Mutex::new(CheckpointRelayQueue::new(queue_cap)));

    let config = L1RelayerConfig {
        rpc_url: rpc_url.to_string(),
        chain_id: 11155111,
        contract_address: contract_address.to_string(),
        relayer_private_key: None,
        required_confirmations: 2,
        poll_interval_secs: 1,
        dry_run,
    };

    let service = L1RelayerService::new(config, queue);

    if !json_output {
        println!(
            "{}",
            "2. Executing Checkpoint Anchoring Stream..."
                .bold()
                .yellow()
        );
    }

    let start_ts = Instant::now();
    let report = service.run_soak(count, start_height).await?;
    let elapsed = start_ts.elapsed();

    if json_output {
        let json = serde_json::to_string_pretty(&report)?;
        println!("{}", json);
        return Ok(());
    }

    // Display sample receipts
    println!(
        "\n{}",
        "3. Sample Anchored Checkpoint Receipts".bold().yellow()
    );
    println!(
        "  {:<8} {:<18} {:<18} {:<18} {:<10}",
        "HEIGHT".bold(),
        "STATE ROOT".bold(),
        "TRANSPARENCY".bold(),
        "TX HASH".bold(),
        "STATUS".bold()
    );
    println!(
        "  {}",
        "------------------------------------------------------------------------".dimmed()
    );

    let display_count = report.receipts.len().min(8);
    for r in &report.receipts[..display_count] {
        let state_abbr = if r.state_root.len() > 14 {
            format!("{}...", &r.state_root[..14])
        } else {
            r.state_root.clone()
        };
        let trans_abbr = if r.transparency_root.len() > 14 {
            format!("{}...", &r.transparency_root[..14])
        } else {
            r.transparency_root.clone()
        };
        let tx_abbr = if r.tx_hash.len() > 14 {
            format!("{}...", &r.tx_hash[..14])
        } else {
            r.tx_hash.clone()
        };

        println!(
            "  #{:<7} {:<18} {:<18} {:<18} {}",
            r.block_height,
            state_abbr.dimmed(),
            trans_abbr.dimmed(),
            tx_abbr.cyan(),
            "SIMULATED".yellow()
        );
    }
    if report.receipts.len() > display_count {
        println!(
            "  ... and {} more checkpoint receipts verified monotonically",
            report.receipts.len() - display_count
        );
    }

    // Telemetry and Summary
    println!(
        "\n{}",
        "4. Soak Verification & Performance Telemetry"
            .bold()
            .yellow()
    );
    let success_rate = (report.successful_batches as f64 / report.total_batches as f64) * 100.0;

    println!("  Checkpoints Submitted:     {}", report.total_batches);
    println!(
        "  Checkpoints Simulated:     {} ({:.1}%)",
        report.successful_batches.to_string().green().bold(),
        success_rate
    );
    println!(
        "  Failed / Dropped:          {}",
        report.failed_batches.to_string().bold()
    );
    println!(
        "  Block Height Range:        #{} -> #{}",
        report.start_height, report.end_height
    );
    println!(
        "  Estimated L1 Gas:          {} gas",
        report.total_gas_consumed.to_string().cyan().bold()
    );
    println!(
        "  Est. Gas Per Anchor:       {} gas",
        format!("{:.0}", report.avg_gas_per_batch).cyan()
    );
    println!(
        "  Duration:                  {:.3}s ({} ms)",
        elapsed.as_secs_f64(),
        report.duration_ms
    );
    println!(
        "  Anchoring Throughput:      {} checkpoints/sec",
        format!("{:.1}", report.throughput_anchors_per_sec)
            .green()
            .bold()
    );

    println!(
        "\n{}",
        "================================================================================".cyan()
    );
    if report.failed_batches == 0 && report.successful_batches == count {
        println!(
            "  {} Calldata simulation passed (100% continuity, monotonic heights). Nothing was anchored on L1.",
            "SIMULATED:".yellow().bold()
        );
    } else {
        println!(
            "  {} Soak failed with {} drops or failures",
            "FAILURE:".red().bold(),
            report.failed_batches
        );
    }
    println!(
        "{}\n",
        "================================================================================".cyan()
    );

    Ok(())
}
