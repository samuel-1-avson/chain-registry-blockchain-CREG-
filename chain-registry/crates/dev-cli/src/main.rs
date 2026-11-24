//! Developer & operator utility CLI (`creg-dev`) housing heavy local
//! analysis/benchmarking commands (`advanced`, `osv-snapshot`, `stress`)
//! separated from the slim `creg` user CLI (`ADR-019` `ECA-16`, `ECO-INV-06`).

#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod advanced;
mod osv_snapshot;
mod stress_test;

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "creg-dev",
    version,
    about = "Chain Registry developer & validator utilities (ML/WASM checks, OSV snapshots, stress tests)"
)]
struct DevCli {
    #[command(subcommand)]
    command: DevCommands,

    /// Output format: text (default) or json.
    #[arg(long, global = true, default_value = "text", value_name = "FORMAT")]
    output: OutputFormat,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
enum DevCommands {
    /// Advanced local validation commands (ML, WASM).
    Advanced {
        #[command(subcommand)]
        command: AdvancedCommands,
    },
    /// Build or manage pinned OSV vulnerability snapshots.
    OsvSnapshot {
        #[command(subcommand)]
        command: OsvSnapshotCommands,
    },
    /// Run high-throughput stress test against the testnet pipeline (1,000+ TPS).
    Stress {
        /// Target throughput in packages/sec
        #[arg(short, long, default_value = "1000")]
        target_tps: usize,
        /// Test duration in seconds
        #[arg(short, long, default_value = "5")]
        duration_secs: u64,
        /// Concurrent pipeline worker count
        #[arg(short, long, default_value = "8")]
        concurrent_clients: usize,
        /// Target node URL (omitted runs in-process high-throughput consensus benchmark)
        #[arg(long)]
        node_url: Option<String>,
    },
}

#[derive(Subcommand)]
enum AdvancedCommands {
    /// Verify using ML-based threat detection
    MlVerify {
        /// Package tarball path
        tarball: std::path::PathBuf,
        /// Ecosystem (npm, pip, cargo)
        #[arg(short, long, default_value = "npm")]
        ecosystem: String,
        /// Output raw JSON
        #[arg(long)]
        json: bool,
    },
    /// Validate package in WASM sandbox
    WasmValidate {
        /// Package tarball path
        tarball: std::path::PathBuf,
        /// Package name
        #[arg(short, long)]
        name: String,
        /// Package version
        #[arg(short, long)]
        version: String,
        /// Ecosystem
        #[arg(short, long, default_value = "npm")]
        ecosystem: String,
    },
    /// Run full advanced validation pipeline
    FullValidate {
        /// Package tarball path
        tarball: std::path::PathBuf,
        /// Package name
        #[arg(short, long)]
        name: String,
        /// Package version
        #[arg(short, long)]
        version: String,
        /// Ecosystem
        #[arg(short, long, default_value = "npm")]
        ecosystem: String,
    },
}

#[derive(Subcommand)]
enum OsvSnapshotCommands {
    /// Query OSV and write a creg-osv-snapshot-v1 JSON file for validators.
    Build {
        /// Snapshot epoch (must match CREG_OSV_SNAPSHOT_EPOCH on validators).
        #[arg(long, default_value = "osv-epoch-0")]
        epoch: String,
        /// Snapshot file path (`-` for stdout). The CLI-wide `--output` selects the display format.
        #[arg(short = 'o', long = "snapshot-output", value_name = "FILE")]
        snapshot_output: Option<std::path::PathBuf>,
        /// Package list file (`ecosystem:name@version` per line).
        #[arg(long, value_name = "FILE")]
        packages: Option<std::path::PathBuf>,
        /// Package keys as `ecosystem:name@version` arguments.
        #[arg(value_name = "PACKAGE")]
        package_keys: Vec<String>,
        /// Delay between OSV API queries in milliseconds.
        #[arg(long, default_value_t = 250)]
        delay_ms: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = DevCli::parse();
    let json_out = matches!(cli.output, OutputFormat::Json);
    let default_level = if json_out { "error" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level)),
        )
        .init();

    match cli.command {
        DevCommands::Advanced { command } => match command {
            AdvancedCommands::MlVerify {
                tarball,
                ecosystem,
                json,
            } => {
                let result = advanced::ml_verify(&tarball, &ecosystem).await?;
                if json || json_out {
                    println!(
                        "{}",
                        serde_json::json!({
                            "threat_score": result.threat_score,
                            "threat_level": format!("{:?}", result.threat_level),
                            "confidence": result.confidence,
                        })
                    );
                } else {
                    println!("ML Verification Result:");
                    println!("  Threat Score: {}/100", result.threat_score);
                    println!("  Threat Level: {:?}", result.threat_level);
                    println!("  Confidence:   {:.2}%", result.confidence * 100.0);
                    println!("  Description:  {}", result.threat_level.description());
                }
            }
            AdvancedCommands::WasmValidate {
                tarball,
                name,
                version,
                ecosystem,
            } => {
                let result = advanced::wasm_validate(&tarball, &name, &version, &ecosystem).await?;
                if json_out {
                    println!(
                        "{}",
                        serde_json::json!({
                            "success": result.success,
                            "exit_code": result.exit_code,
                            "findings": result.findings.len(),
                        })
                    );
                } else {
                    println!("WASM Validation Result:");
                    println!("  Success:   {}", result.success);
                    println!("  Exit Code: {}", result.exit_code);
                    println!("  CPU Time:  {}ms", result.resource_usage.cpu_time_ms);
                    if !result.findings.is_empty() {
                        println!("  Findings:");
                        for f in &result.findings {
                            println!("    - [{:?}] {}", f.severity, f.description);
                        }
                    }
                }
            }
            AdvancedCommands::FullValidate {
                tarball,
                name,
                version,
                ecosystem,
            } => {
                println!("Running full advanced validation pipeline...");
                println!("\n[1/2] ML-based threat detection...");
                let ml_result = advanced::ml_verify(&tarball, &ecosystem).await?;
                println!(
                    "  Score: {}/100 ({:?})",
                    ml_result.threat_score, ml_result.threat_level
                );

                println!("\n[2/2] WASM sandbox validation...");
                let wasm_result =
                    advanced::wasm_validate(&tarball, &name, &version, &ecosystem).await?;
                println!(
                    "  Success: {} (Exit: {})",
                    wasm_result.success, wasm_result.exit_code
                );
                println!("\n{} Validation complete!", "✓".green().bold());
            }
        },
        DevCommands::OsvSnapshot { command } => match command {
            OsvSnapshotCommands::Build {
                epoch,
                snapshot_output,
                packages,
                package_keys,
                delay_ms,
            } => osv_snapshot::run_build(
                &epoch,
                snapshot_output.as_deref(),
                packages.as_deref(),
                &package_keys,
                delay_ms,
            )?,
        },
        DevCommands::Stress {
            target_tps,
            duration_secs,
            concurrent_clients,
            node_url,
        } => {
            let config = stress_test::StressTestConfig {
                target_tps,
                duration_secs,
                concurrent_clients,
                node_url,
            };
            stress_test::run_stress_test(config).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_cli_parses_advanced_osv_and_stress_subcommands() {
        let ml = DevCli::try_parse_from([
            "creg-dev",
            "advanced",
            "ml-verify",
            "pkg.tgz",
            "--ecosystem",
            "npm",
            "--json",
        ])
        .unwrap();
        assert!(matches!(ml.command, DevCommands::Advanced { .. }));

        let osv = DevCli::try_parse_from([
            "creg-dev",
            "osv-snapshot",
            "build",
            "--epoch",
            "osv-2026-11-24",
            "-o",
            "snap.json",
            "npm:lodash@4.17.20",
        ])
        .unwrap();
        assert!(matches!(osv.command, DevCommands::OsvSnapshot { .. }));

        let stress = DevCli::try_parse_from([
            "creg-dev",
            "stress",
            "--target-tps",
            "250",
            "--duration-secs",
            "2",
        ])
        .unwrap();
        assert!(matches!(stress.command, DevCommands::Stress { .. }));
    }
}
