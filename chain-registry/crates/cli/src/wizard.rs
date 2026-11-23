// crates/cli/src/wizard.rs
// `creg init` — Interactive setup wizard for developers and node operators.

use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::{Confirm, Input, Select};
use std::path::PathBuf;

pub async fn run() -> Result<()> {
    println!("\n{}", "🎉 Welcome to Chain Registry!".bold().cyan());
    println!("{}", "═".repeat(60).dimmed());
    println!(
        "\nThis wizard will guide you through setting up Chain Registry in a few simple steps.\n"
    );

    // Detect existing authenticated profile
    let existing_profile = crate::profile::load_active_profile().ok().flatten();
    if let Some(ref prof) = existing_profile {
        println!(
            "  {} Active authenticated profile detected:",
            "✓".green().bold()
        );
        println!("    • EVM Wallet:      {}", prof.wallet_address.cyan());
        println!("    • Consensus Key:   {}", prof.ed25519_pubkey.dimmed());
        println!("    • Network RPC:     {}", prof.network_url.dimmed());
        println!();
    }

    // Step 1: Choose role
    let role_idx = Select::new()
        .with_prompt("What would you like to do?")
        .items(&[
            "Publish packages (I'm a developer)",
            "Become a validator (Run a node)",
            "Both",
            "Just explore (no setup required)",
        ])
        .default(0)
        .interact()?;

    let setup_publisher = role_idx == 0 || role_idx == 2;
    let setup_validator = role_idx == 1 || role_idx == 2;

    // Step 2: Check prerequisites
    println!("\n{}", "Step 1: Checking Prerequisites".bold());
    println!("{}", "─".repeat(40).dimmed());

    let doctor_ok = run_doctor_check(setup_validator).await?;
    if !doctor_ok {
        let proceed = Confirm::new()
            .with_prompt("Some checks failed. Continue anyway?")
            .default(false)
            .interact()?;
        if !proceed {
            println!(
                "\n{}",
                "Setup cancelled. Run 'creg doctor' for help.".yellow()
            );
            return Ok(());
        }
    }

    // Step 3: Configure identity & keys
    if setup_publisher || setup_validator {
        println!("\n{}", "Step 2: Identity & Cryptographic Keys".bold());
        println!("{}", "─".repeat(40).dimmed());

        if setup_publisher {
            if let Some(ref prof) = existing_profile {
                let keep = Confirm::new()
                    .with_prompt(format!(
                        "Use active Web3 profile ({}) for package publishing?",
                        &prof.wallet_address[..std::cmp::min(10, prof.wallet_address.len())]
                    ))
                    .default(true)
                    .interact()?;

                if !keep {
                    configure_publisher_identity().await?;
                } else {
                    println!("  {} Keeping active Web3 profile", "✓".green());
                }
            } else {
                configure_publisher_identity().await?;
            }
        }

        if setup_validator {
            println!("\n  Validator PBFT Consensus Key:");
            generate_key("validator").await?;
        }
    }

    // Step 4: Configure validator node
    if setup_validator {
        println!("\n{}", "Step 3: Validator Configuration".bold());
        println!("{}", "─".repeat(40).dimmed());

        configure_validator().await?;
    }

    // Step 5: Staking
    if setup_validator {
        println!("\n{}", "Step 4: Staking".bold());
        println!("{}", "─".repeat(40).dimmed());

        let stake_amount: f64 = Input::new()
            .with_prompt("How much CREG would you like to stake? (minimum 100 for validator)")
            .default(100.0)
            .validate_with(|input: &f64| {
                if *input >= 100.0 {
                    Ok(())
                } else {
                    Err("Minimum stake for validator is 100 CREG")
                }
            })
            .interact()?;

        println!("\n  Staking {} CREG...", stake_amount);
        println!("  {}", "✓ Stake parameters recorded".green());
        println!(
            "  To submit on-chain stake when ready, run: {}",
            format!("creg stake {}", stake_amount).cyan()
        );
    }

    // Step 6: Start node
    if setup_validator {
        println!("\n{}", "Step 5: Starting Node".bold());
        println!("{}", "─".repeat(40).dimmed());

        let mode_idx = Select::new()
            .with_prompt("Select node mode")
            .items(&[
                "Light (4GB RAM, 100GB storage) - Recommended for beginners",
                "Standard (8GB RAM, 200GB storage)",
                "Full (16GB RAM, 500GB storage)",
            ])
            .default(0)
            .interact()?;

        let mode = match mode_idx {
            0 => "light",
            1 => "standard",
            2 => "full",
            _ => "light",
        };

        println!("\n  Creating docker-compose.{}yml...", mode);
        create_docker_compose(mode).await?;
        println!("  {}", "✓ Configuration created".green());

        let start_now = Confirm::new()
            .with_prompt("Start the node now?")
            .default(true)
            .interact()?;

        if start_now {
            println!("\n  Starting node with Docker...");
            start_node(mode).await?;
        }
    }

    // Final summary
    println!("\n{}", "═".repeat(60).dimmed());
    println!("{}", "✓ Setup Complete!".bold().green());
    println!("{}", "═".repeat(60).dimmed());

    if setup_publisher {
        println!("\n📦 Package Publishing (Zero-Flag Workflow):");
        println!("   creg publish <tarball>.tgz    # Uses your active authenticated profile");
        println!("   creg profile                 # View active wallet and consensus identity");
    }

    if setup_validator {
        println!("\n🔐 Validator Commands:");
        println!("   creg console         # Launch the live interactive TUI console");
        println!("   creg doctor          # Run full node health diagnostics");
        println!("   creg stake 100       # Stake tokens for validator admission");
        println!("   docker-compose logs -f  # Tail node logs");
    }

    println!("\n📚 Help Resources:");
    println!("   creg --help          # Show all available subcommands");
    println!("   creg doctor          # Diagnostic health check");
    println!("   https://docs.creg.dev # Documentation & Testnet Guides");
    println!();

    Ok(())
}

async fn configure_publisher_identity() -> Result<()> {
    let choices = &[
        "Connect with MetaMask / Web3 Wallet (Recommended — Dual-Key Architecture)",
        "Generate standalone offline Ed25519 keypair (Legacy / Offline dev)",
    ];

    let choice = Select::new()
        .with_prompt("Select publisher identity method")
        .items(choices)
        .default(0)
        .interact()?;

    if choice == 0 {
        println!("\n  Connecting via Web3 Loopback Bridge (Port 18888)...");
        let node_url = std::env::var("CREG_NODE_URL")
            .unwrap_or_else(|_| "https://api.testnet.cregnet.dev".into());

        match crate::auth::start_loopback_auth(
            18888,
            false,
            crate::auth::DEFAULT_CHAIN_ID,
            crate::auth::DEFAULT_VERIFYING_CONTRACT,
        )
        .await
        {
            Ok(identity) => {
                let prof = crate::profile::save_identity(&identity, &node_url)?;
                println!("\n  {} Web3 Authentication Successful!", "✓".green().bold());
                crate::profile::print_profile(&prof);
            }
            Err(err) => {
                println!(
                    "  {} Web3 login could not be completed right now: {}",
                    "⚠".yellow(),
                    err
                );
                println!(
                    "  You can link your wallet anytime later by running: {}",
                    "creg login".cyan().bold()
                );
            }
        }
    } else {
        generate_key("publisher").await?;
    }

    Ok(())
}

async fn run_doctor_check(is_validator: bool) -> Result<bool> {
    let node_target =
        std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "https://api.testnet.cregnet.dev".into());

    let node_ok = check_node_endpoint(&node_target).await;

    if is_validator {
        let docker_ok = check_docker().await?;
        let ipfs_ok = check_ipfs().await?;

        println!(
            "  {} {}",
            if node_ok { "✓".green() } else { "✗".red() },
            format!("Node connectivity ({})", node_target).dimmed()
        );
        println!(
            "  {} {}",
            if docker_ok {
                "✓".green()
            } else {
                "✗".red()
            },
            "Docker daemon".dimmed()
        );
        println!(
            "  {} {}",
            if ipfs_ok { "✓".green() } else { "✗".red() },
            "IPFS daemon".dimmed()
        );

        Ok(node_ok && docker_ok)
    } else {
        println!(
            "  {} {}",
            if node_ok { "✓".green() } else { "✗".red() },
            format!("Testnet connectivity ({})", node_target).dimmed()
        );
        Ok(node_ok)
    }
}

async fn check_docker() -> Result<bool> {
    Ok(std::process::Command::new("docker")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false))
}

async fn check_node_endpoint(url: &str) -> bool {
    let health_url = format!("{}/v1/health", url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    match client
        .get(&health_url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    {
        Ok(r) => r.status().is_success(),
        Err(_) => false,
    }
}

async fn check_ipfs() -> Result<bool> {
    let client = reqwest::Client::new();
    match client
        .post("http://localhost:5001/api/v0/id")
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        Ok(r) => Ok(r.status().is_success()),
        Err(_) => Ok(false),
    }
}

async fn generate_key(role: &str) -> Result<()> {
    let key_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".creg")
        .join(format!("{}.key", role));

    if key_path.exists() {
        let overwrite = Confirm::new()
            .with_prompt(format!("{} key already exists. Overwrite?", role))
            .default(false)
            .interact()?;
        if !overwrite {
            println!("  Using existing key");
            return Ok(());
        }
    }

    println!("  Generating {} key...", role);

    // Call keygen module
    crate::keygen::run(Some(&key_path), role)?;

    println!("  {} Key saved to {}", "✓".green(), key_path.display());

    if role == "validator" {
        println!("  ⚠️  Keep this key secure! Backup to offline storage.");
    }

    Ok(())
}

async fn configure_validator() -> Result<()> {
    let config_dir = dirs::home_dir().unwrap_or_default().join(".creg");
    std::fs::create_dir_all(&config_dir)?;

    let validator_key_path = config_dir
        .join("validator.key")
        .to_string_lossy()
        .to_string();

    // Create .env file with platform-agnostic path
    let env_content = format!(
        r#"# Chain Registry Validator Configuration
# Generated by creg wizard

CREG_IS_VALIDATOR=true
CREG_VALIDATOR_KEY={}
CREG_NODE_MODE=light
CREG_MAX_PEERS=10

# Ethereum RPC (Arbitrum for low gas)
CREG_ETH_RPC=https://arb1.arbitrum.io/rpc

# Optional: Custom settings
# CREG_NODE_URL=https://api.testnet.cregnet.dev
# CREG_P2P_LISTEN=/ip4/0.0.0.0/tcp/4001
"#,
        validator_key_path
    );

    let env_path = config_dir.join(".env");
    std::fs::write(&env_path, env_content)?;

    println!("  {} Config saved to {}", "✓".green(), env_path.display());

    Ok(())
}

async fn create_docker_compose(mode: &str) -> Result<()> {
    let compose_file = match mode {
        "light" => include_str!("../../../docker-compose.light.yml"),
        "standard" => include_str!("../../../docker-compose.yml"),
        "full" => include_str!("../../../docker-compose.yml"),
        _ => include_str!("../../../docker-compose.light.yml"),
    };

    let output_path = PathBuf::from(format!("docker-compose.{}yml", mode));
    std::fs::write(&output_path, compose_file)?;

    println!("  {} Created {}", "✓".green(), output_path.display());

    Ok(())
}

async fn start_node(mode: &str) -> Result<()> {
    let compose_file = format!("docker-compose.{}yml", mode);

    let status = std::process::Command::new("docker-compose")
        .args(["-f", &compose_file, "up", "-d"])
        .status()?;

    if status.success() {
        println!("  {} Node started successfully!", "✓".green());
        println!("\n  Monitoring: creg console");
        println!("  Logs: docker-compose -f {} logs -f", compose_file);
    } else {
        bail!("Failed to start node. Check docker installation.");
    }

    Ok(())
}
