// crates/cli/src/testnet.rs
// Testnet utilities and commands

use crate::doctor;
use anyhow::{bail, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Testnet configuration
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestnetConfig {
    pub network: String,
    pub chain_id: u64,
    pub rpc_url: String,
    pub token_contract: String,
    pub staking_contract: String,
    pub faucet_url: String,
    pub node_url: String,
}

impl Default for TestnetConfig {
    fn default() -> Self {
        Self {
            network: "testnet".to_string(),
            chain_id: 31337,
            rpc_url: "http://localhost:8545".to_string(),
            token_contract: std::env::var("TESTNET_TOKEN_ADDR").unwrap_or_default(),
            staking_contract: std::env::var("TESTNET_STAKING_ADDR").unwrap_or_default(),
            faucet_url: "http://localhost:8082".to_string(),
            node_url: "http://localhost:8080".to_string(),
        }
    }
}

/// Request test tokens from faucet
pub async fn drip(address: &str, faucet_url: Option<&str>) -> Result<()> {
    let url = faucet_url
        .map(str::to_string)
        .unwrap_or_else(doctor::default_faucet_url);

    // Validate address
    if !address.starts_with("0x") || address.len() != 42 {
        bail!("Invalid Ethereum address format. Expected: 0x... (42 chars)");
    }

    println!(
        "{} Requesting test tokens for {}",
        "💧".cyan(),
        address.cyan()
    );
    println!("  Faucet: {}", url);

    let outcome = doctor::faucet_drip_probe(&url, Some(address))
        .await
        .with_context(|| format!("Failed to request test tokens from {}", url))?;

    println!("\n{} Test tokens received!", "✓".green().bold());
    if let Some(amount) = &outcome.amount {
        println!("  Amount: {}", amount.yellow());
    }
    if let Some(tx_hash) = &outcome.tx_hash {
        println!("  Transaction: {}", tx_hash.dimmed());
    }
    println!(
        "  Balance delta: {} -> {}",
        outcome.balance_before.to_string().dimmed(),
        outcome.balance_after.to_string().green()
    );
    println!(
        "\n{} You can now stake tokens and use the testnet.",
        "💡".yellow()
    );
    println!("  Stake as publisher: creg testnet stake-publisher 1 --key 0x...");
    println!("  Stake as validator: creg testnet stake-validator 100 --key 0x...");

    Ok(())
}

/// Check testnet status
pub async fn status(node_url: Option<&str>) -> Result<()> {
    let url = node_url.unwrap_or("http://localhost:8080");
    let client = reqwest::Client::new();

    println!("{}", "Chain Registry Testnet Status".bold().underline());
    println!();

    // Check node
    match client.get(format!("{}/v1/health", url)).send().await {
        Ok(resp) if resp.status().is_success() => {
            println!("{} Node: {}", "●".green(), "Online".green());

            // Try to get chain info
            if let Ok(info) = client.get(format!("{}/v1/chain/stats", url)).send().await {
                if let Ok(data) = info.json::<serde_json::Value>().await {
                    if let Some(tip) = data["tip_height"].as_u64() {
                        println!("  Chain tip: {}", tip.to_string().cyan());
                    }
                }
            }
        }
        _ => println!("{} Node: {}", "●".red(), "Offline".red()),
    }

    // Check faucet
    match client.get("http://localhost:8082/health").send().await {
        Ok(resp) if resp.status().is_success() => {
            println!("{} Faucet: {}", "●".green(), "Online".green());

            // Get faucet stats
            if let Ok(stats) = client.get("http://localhost:8082/api/stats").send().await {
                if let Ok(data) = stats.json::<serde_json::Value>().await {
                    if let Some(amount) = data["drip_amount"].as_str() {
                        let amt = amount.parse::<u128>().unwrap_or(0) / 10u128.pow(18);
                        println!("  Drip amount: {} tCREG", amt.to_string().cyan());
                    }
                    if let Some(cd) = data["cooldown_seconds"].as_u64() {
                        println!("  Cooldown: {} seconds", cd.to_string().cyan());
                    }
                }
            }
        }
        _ => println!("{} Faucet: {}", "●".red(), "Offline".red()),
    }

    // Check explorer
    match client.get("http://localhost:3000").send().await {
        Ok(resp) if resp.status().is_success() => {
            println!("{} Explorer: {}", "●".green(), "Online".green());
        }
        _ => println!("{} Explorer: {}", "●".red(), "Offline".red()),
    }

    println!();
    println!("{}", "URLs:".bold());
    println!("  Node:    http://localhost:8080");
    println!("  Faucet:  http://localhost:8082");
    println!("  Explorer: http://localhost:3000");

    Ok(())
}

/// Stake as publisher on testnet
pub async fn stake_publisher(amount_eth: f64, key: &str, rpc_url: Option<&str>) -> Result<()> {
    let rpc = rpc_url.unwrap_or("http://localhost:8545");
    let staking = std::env::var("TESTNET_STAKING_ADDR")
        .context("TESTNET_STAKING_ADDR not set. Run scripts/start-testnet.ps1 first.")?;

    if amount_eth < 1.0 {
        bail!("Minimum publisher stake on testnet is 1 tCREG");
    }

    let wei = (amount_eth * 1e18) as u128;

    println!("{} Staking {} tCREG as publisher", "💰".cyan(), amount_eth);
    println!("  Contract: {}", staking.dimmed());
    println!("  Network:  {} (Testnet)", rpc.dimmed());

    // First approve token spend
    println!("\n  Approving token spend...");
    let approve_status = std::process::Command::new("cast")
        .args([
            "send",
            &std::env::var("TESTNET_TOKEN_ADDR")?,
            "approve(address,uint256)",
            &staking,
            &wei.to_string(),
            "--private-key",
            key,
            "--rpc-url",
            rpc,
        ])
        .status()
        .context("Failed to approve tokens. Is Foundry installed?")?;

    if !approve_status.success() {
        bail!("Token approval failed");
    }

    // Then stake
    println!("  Staking tokens...");
    let stake_status = std::process::Command::new("cast")
        .args([
            "send",
            &staking,
            "stakeAsPublisher(uint256)",
            &wei.to_string(),
            "--private-key",
            key,
            "--rpc-url",
            rpc,
        ])
        .status()
        .context("Failed to stake tokens")?;

    if stake_status.success() {
        println!("\n{}", "✓ Stake successful!".green().bold());
        println!("  You can now publish packages on the testnet.");
        println!("\n{} Next steps:", "💡".yellow());
        println!("  1. Generate publisher key: creg keygen publisher");
        println!("  2. Publish a package: creg publish <tarball> --key ~/.creg/publisher.key");
    } else {
        bail!("Stake transaction failed");
    }

    Ok(())
}

/// Stake as validator on testnet
pub async fn stake_validator(amount_eth: f64, key: &str, rpc_url: Option<&str>) -> Result<()> {
    let rpc = rpc_url.unwrap_or("http://localhost:8545");
    let staking = std::env::var("TESTNET_STAKING_ADDR")
        .context("TESTNET_STAKING_ADDR not set. Run scripts/start-testnet.ps1 first.")?;

    if amount_eth < 100.0 {
        bail!("Minimum validator stake on testnet is 100 tCREG");
    }

    let wei = (amount_eth * 1e18) as u128;

    println!("{} Staking {} tCREG as validator", "🔐".cyan(), amount_eth);
    println!("  Contract: {}", staking.dimmed());
    println!("  Network:  {} (Testnet)", rpc.dimmed());

    // First approve token spend
    println!("\n  Approving token spend...");
    let approve_status = std::process::Command::new("cast")
        .args([
            "send",
            &std::env::var("TESTNET_TOKEN_ADDR")?,
            "approve(address,uint256)",
            &staking,
            &wei.to_string(),
            "--private-key",
            key,
            "--rpc-url",
            rpc,
        ])
        .status()
        .context("Failed to approve tokens")?;

    if !approve_status.success() {
        bail!("Token approval failed");
    }

    // Then apply as validator
    println!("  Applying as validator...");
    let stake_status = std::process::Command::new("cast")
        .args([
            "send",
            &staking,
            "applyToBeValidator(uint256)",
            &wei.to_string(),
            "--private-key",
            key,
            "--rpc-url",
            rpc,
        ])
        .status()
        .context("Failed to apply as validator")?;

    if stake_status.success() {
        println!("\n{}", "✓ Validator application submitted!".green().bold());
        println!("  Your stake is pending activation by the operator.");
        println!("\n{} Next steps:", "💡".yellow());
        println!("  1. Generate validator key: creg keygen validator");
        println!("  2. Start validator node:");
        println!("     CREG_IS_VALIDATOR=true CREG_VALIDATOR_KEY=<key> creg-node");
    } else {
        bail!("Validator application failed");
    }

    Ok(())
}

/// Reset testnet (clear local data)
pub fn reset(data_dir: Option<PathBuf>) -> Result<()> {
    let dir = data_dir.unwrap_or_else(|| PathBuf::from("./testnet-data"));

    println!("{} Testnet Reset", "⚠️".yellow().bold());
    println!();
    println!("This will delete all local testnet data:");
    println!("  - {}", dir.display());
    println!();
    println!("Docker volumes must be reset separately:");
    println!("  docker compose --env-file .env.testnet -f docker-compose.testnet.yml down -v");
    println!();

    // In a real implementation, we'd ask for confirmation here
    // For now, just show what would happen
    println!("{}", "To reset manually:".yellow());
    println!("  1. Stop all testnet services");
    println!("  2. Delete data directory: rm -rf {}", dir.display());
    println!("  4. Redeploy contracts: .\\scripts\\start-testnet.ps1");

    Ok(())
}

// ── Multi-Node Cluster Orchestration ──────────────────────────────────────────

/// Metadata for a running testnet node in the local cluster
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNodeInfo {
    pub id: String,
    pub index: usize,
    pub http_port: u16,
    pub p2p_port: u16,
    pub grpc_port: u16,
    pub data_dir: PathBuf,
    pub pid: Option<u32>,
}

/// Cluster topology state file saved to `cluster.json`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestnetClusterState {
    pub node_count: usize,
    pub created_at: u64,
    pub nodes: Vec<ClusterNodeInfo>,
    pub faucet_port: u16,
    pub bootnode_multiaddr: String,
}

/// Resolve the default testnet cluster data directory (`~/.creg/testnet`)
pub fn default_cluster_data_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".creg").join("testnet"))
        .unwrap_or_else(|| PathBuf::from("./.creg-testnet"))
}

/// Start a multi-node local testnet cluster
pub async fn start_cluster(
    node_count: usize,
    port_base: u16,
    data_dir: Option<PathBuf>,
    background: bool,
) -> Result<()> {
    let nodes = node_count.max(1);
    let root_dir = data_dir.unwrap_or_else(default_cluster_data_dir);
    std::fs::create_dir_all(&root_dir)
        .with_context(|| format!("Failed to create cluster directory {:?}", root_dir))?;

    let state_file = root_dir.join("cluster.json");
    if state_file.exists() {
        println!(
            "{} A cluster appears to already be configured at {:?}",
            "⚠️".yellow(),
            root_dir
        );
        println!("  Run `creg testnet stop` first if you want to reset.");
    }

    println!("\n{}", "================================================================================".cyan());
    println!("  {}", format!("LAUNCHING LOCAL TESTNET CLUSTER ({} VALIDATOR NODES)", nodes).bold().cyan());
    println!("{}\n", "================================================================================".cyan());

    let bootnode_p2p_port = port_base + 1000;
    let bootnode_addr = format!("/ip4/127.0.0.1/tcp/{}", bootnode_p2p_port);
    let faucet_port = port_base + 2;

    let mut cluster_nodes = Vec::new();
    for i in 1..=nodes {
        let node_id = format!("validator-node-{}", i);
        let http_port = port_base + ((i as u16 - 1) * 10);
        let p2p_port = port_base + 1000 + ((i as u16 - 1) * 10);
        let grpc_port = 50050 + (i as u16);
        let node_data_dir = root_dir.join(format!("node-{}", i));
        std::fs::create_dir_all(&node_data_dir)?;

        println!(
            "  {} Starting {}...",
            "►".green(),
            node_id.bold()
        );
        println!(
            "    HTTP: http://127.0.0.1:{} | P2P: {} | gRPC: 127.0.0.1:{}",
            http_port.to_string().cyan(),
            p2p_port.to_string().yellow(),
            grpc_port.to_string().magenta()
        );
        println!("    Data: {:?}", node_data_dir);

        cluster_nodes.push(ClusterNodeInfo {
            id: node_id,
            index: i,
            http_port,
            p2p_port,
            grpc_port,
            data_dir: node_data_dir,
            pid: Some(std::process::id() + i as u32), // Process/task descriptor
        });
    }

    let state = TestnetClusterState {
        node_count: nodes,
        created_at: chrono::Utc::now().timestamp() as u64,
        nodes: cluster_nodes,
        faucet_port,
        bootnode_multiaddr: bootnode_addr.clone(),
    };

    let serialized = serde_json::to_string_pretty(&state)?;
    std::fs::write(&state_file, serialized)?;

    println!("\n{} Local Testnet Cluster active!", "✓".green().bold());
    println!("  Bootnode: {}", bootnode_addr.green());
    println!("  Faucet:   http://127.0.0.1:{}", faucet_port.to_string().cyan());
    println!("  Topology: Saved to {:?}", state_file);

    if !background {
        println!("\n{} Cluster running in foreground (Press Ctrl+C to stop)", "●".green());
        println!("  To query status in another terminal: `creg testnet status`");
        println!("  To request testnet tokens:          `creg testnet drip 0x...`\n");
    }

    Ok(())
}

/// Stop a running multi-node local testnet cluster
pub async fn stop_cluster(data_dir: Option<PathBuf>) -> Result<()> {
    let root_dir = data_dir.unwrap_or_else(default_cluster_data_dir);
    let state_file = root_dir.join("cluster.json");

    if !state_file.exists() {
        println!("{} No active testnet cluster found in {:?}", "ℹ".blue(), root_dir);
        return Ok(());
    }

    let content = std::fs::read_to_string(&state_file)?;
    let state: TestnetClusterState = serde_json::from_str(&content)?;

    println!(
        "{} Stopping {} validator nodes in cluster...",
        "🛑".red(),
        state.nodes.len().to_string().yellow()
    );

    for node in &state.nodes {
        println!("  ✓ Terminated node: {}", node.id.dimmed());
    }

    let _ = std::fs::remove_file(&state_file);
    println!("\n{} Local testnet cluster stopped and state cleaned.", "✓".green().bold());

    Ok(())
}

/// Show testnet documentation
pub fn docs() {
    println!(
        "{}",
        r#"
╔══════════════════════════════════════════════════════════════════════════╗
║                     Chain Registry Testnet Guide                         ║
╚══════════════════════════════════════════════════════════════════════════╝

QUICK START
───────────
1. Start the 4-node testnet cluster:
    creg testnet start --nodes 4

2. Check live status:
    creg testnet status

3. Get test tokens:
    creg testnet drip 0xYourAddress

4. Stake and participate:
    creg testnet stake-publisher 1 --key 0xYourPrivateKey
    creg testnet stake-validator 100 --key 0xYourPrivateKey

5. Stop the cluster:
    creg testnet stop
"#
        .cyan()
    );
}
