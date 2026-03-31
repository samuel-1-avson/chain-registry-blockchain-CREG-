// crates/cli/src/main.rs
// `creg` — the main CLI. Wraps the shim logic with a friendly interface.

mod intercept;
mod output;
mod install;
mod publish;
mod keygen;
mod stake;
mod watch;
mod blocks;
mod audit;
mod verify;
mod lockfile;
mod dashboard;
mod dashboard_enhanced;
mod config_file;
mod batch;
mod advanced;

use clap::{Parser, Subcommand, CommandFactory};
use clap_complete::{generate, Shell};
use anyhow::Result;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "creg",
    version = "0.1.0",
    about = "Chain Registry — decentralised, consensus-verified package management"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Skip chain verification (same as --unverified on any sub-command).
    #[arg(long, global = true)]
    unverified: bool,

    /// Chain node URL to query. Overrides config file.
    #[arg(long, global = true, env = "CREG_NODE_URL")]
    node_url: Option<String>,

    /// Disable colored output.
    #[arg(long, global = true)]
    no_color: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Install a package through the chain registry.
    Install {
        /// Package name and optional version (e.g. express@4.18.0)
        package: String,
        /// Ecosystem: npm | pip | cargo | gem (auto-detected from cwd if omitted)
        #[arg(short, long)]
        ecosystem: Option<String>,
        /// Allow installing unverified (pending-pool) packages with a warning.
        #[arg(long)]
        unverified: bool,
    },

    /// Look up the trust verdict for a package without installing.
    Status {
        package: String,
        #[arg(short, long)]
        ecosystem: Option<String>,
        /// Output raw JSON instead of formatted table.
        #[arg(long)]
        json: bool,
    },

    /// Publish a package to the pending pool.
    Publish {
        /// Path to the tarball.
        tarball: std::path::PathBuf,
        /// Path to the package manifest TOML/JSON.
        #[arg(short, long)]
        manifest: Option<std::path::PathBuf>,
        /// Publisher's Ed25519 private key file (hex-encoded).
        #[arg(short, long, env = "CREG_PUBLISHER_KEY")]
        key: String,
        /// Encrypt the package for the validator quorum (Shielded).
        #[arg(long)]
        shield: bool,
    },

    /// Install the PATH shims so `npm`, `pip`, etc. go through chain-registry.
    SetupShims {
        /// Directory to place shim binaries. Defaults to ~/.local/bin
        #[arg(long)]
        shim_dir: Option<std::path::PathBuf>,
    },

    /// Remove the PATH shims and restore the originals.
    RemoveShims,

    /// Show the contents of the local verification cache.
    Cache {
        #[arg(long)]
        clear: bool,
    },

    /// Generate a new Ed25519 keypair for publishing or validator use.
    Keygen {
        /// Role: "publisher" or "validator"
        #[arg(default_value = "publisher")]
        role: String,
        /// Output file path (defaults to ~/.creg/<role>.key)
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },

    /// Manage the local pkg-lock.chain file.
    Lockfile {
        /// Directory containing the lockfile (defaults to cwd).
        #[arg(short, long)]
        dir: Option<std::path::PathBuf>,
        /// Clear the lockfile.
        #[arg(long)]
        clear: bool,
    },

    /// Audit all currently-installed packages against the chain.
    Audit {
        /// Ecosystem to audit (auto-detected if omitted).
        #[arg(short, long)]
        ecosystem: Option<String>,
        /// Exit non-zero if any packages are unverified.
        #[arg(long)]
        strict: bool,
        /// Output raw JSON.
        #[arg(long)]
        json: bool,
    },

    /// Verify a single package and optionally save a proof checkpoint.
    Verify {
        package: String,
        #[arg(short, long)]
        ecosystem: Option<String>,
        /// Save proof to this file path.
        #[arg(long)]
        checkpoint: Option<std::path::PathBuf>,
        /// Output raw JSON.
        #[arg(long)]
        json: bool,
    },

    /// Stream real-time events from the chain node (SSE).
    Watch {
        /// Filter: "packages" | "blocks" | "votes" | all (default).
        #[arg(short, long)]
        filter: Option<String>,
        /// Override the node URL for this session.
        #[arg(long)]
        node_url: Option<String>,
    },

    /// Stake ETH as a publisher or validator.
    Stake {
        /// Amount in ETH (e.g. "1.5").
        amount: String,
        /// Role: "publisher" | "validator".
        #[arg(short, long, default_value = "publisher")]
        role: String,
        /// Staking contract address (0x…).
        #[arg(long)]
        staking_addr: Option<String>,
        /// EVM RPC URL.
        #[arg(long)]
        rpc_url: Option<String>,
        /// Deployer/caller private key (hex).
        #[arg(long)]
        key: Option<String>,
    },

    /// Launch the interactive Premium TUI Dashboard.
    Dashboard,

    /// Launch the enhanced interactive TUI Dashboard (with more features).
    DashboardEnhanced,

    /// Non-interactive chain explorer.
    Blocks {
        /// Number of blocks to show.
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Generate shell completion scripts.
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Manage configuration file.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// Batch operations for multiple packages.
    Batch {
        #[command(subcommand)]
        command: BatchCommands,
    },

    /// Advanced validation commands (ZK, ML, WASM).
    Advanced {
        #[command(subcommand)]
        command: AdvancedCommands,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Initialize a new configuration file
    Init,
    /// Show current configuration
    Show,
    /// Get a specific configuration value
    Get {
        key: String,
    },
}

#[derive(Subcommand)]
enum BatchCommands {
    /// Verify multiple packages in parallel
    Verify {
        /// Package names to verify
        #[arg(required = true)]
        packages: Vec<String>,
        /// Ecosystem (npm, pip, cargo, etc.)
        #[arg(short, long)]
        ecosystem: Option<String>,
    },
    /// Install multiple packages with batch verification
    Install {
        /// Package names to install
        #[arg(required = true)]
        packages: Vec<String>,
        /// Ecosystem
        #[arg(short, long)]
        ecosystem: Option<String>,
        /// Allow unverified packages
        #[arg(long)]
        unverified: bool,
    },
    /// Verify all dependencies from manifest file
    VerifyDeps {
        /// Path to manifest file (auto-detected if not specified)
        #[arg(short, long)]
        manifest: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum AdvancedCommands {
    /// Generate ZK proof for a package
    ZkProof {
        /// Path to package tarball
        tarball: std::path::PathBuf,
        /// Path to manifest file
        #[arg(short, long)]
        manifest: Option<std::path::PathBuf>,
        /// Output file for ZK proof
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },
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
        /// Generate ZK proof
        #[arg(long)]
        zk: bool,
        /// Output directory for results
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Install { package, ecosystem, unverified } => {
            let allow_unverified = unverified || cli.unverified;
            install::run(&package, ecosystem.as_deref(), allow_unverified, cli.node_url.as_deref()).await?;
        }
        Commands::Status { package, ecosystem, json } => {
            let verdict = resolver::resolve(
                &package,
                ecosystem.as_deref(),
                cli.node_url.as_deref(),
            ).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&verdict)?);
            } else {
                output::print_verdict(&verdict);
            }
        }
        Commands::Publish { tarball, manifest, key, shield } => {
            publish::run(&tarball, manifest.as_deref(), &key, cli.node_url.as_deref(), shield).await?;
        }
        Commands::SetupShims { shim_dir } => {
            intercept::setup_shims(shim_dir.as_deref())?;
        }
        Commands::RemoveShims => {
            intercept::remove_shims()?;
        }
        Commands::Cache { clear } => {
            if clear {
                resolver::cache::clear()?;
                println!("Verification cache cleared.");
            } else {
                resolver::cache::print_entries()?;
            }
        }
        Commands::Keygen { role, output } => {
            keygen::run(output.as_deref(), &role)?;
        }
        Commands::Lockfile { clear, dir } => {
            let d = dir.unwrap_or_else(|| std::env::current_dir().unwrap());
            if clear {
                let path = d.join("pkg-lock.chain");
                if path.exists() { std::fs::remove_file(&path)?; }
                println!("pkg-lock.chain cleared.");
            } else {
                lockfile::print_lockfile(&d)?;
            }
        }
        Commands::Audit { ecosystem, strict, json } => {
            let code = audit::run(
                ecosystem.as_deref(),
                cli.node_url.as_deref(),
                strict,
                json,
            ).await?;
            std::process::exit(code);
        }
        Commands::Verify { package, ecosystem, checkpoint, json } => {
            verify::run(
                &package,
                ecosystem.as_deref(),
                cli.node_url.as_deref(),
                checkpoint.as_ref().and_then(|p: &std::path::PathBuf| p.to_str()),
                json,
            ).await?;
        }
        Commands::Watch { filter, node_url } => {
            let url = node_url.or(cli.node_url);
            watch::run(filter.as_deref(), url.as_deref()).await?;
        }
        Commands::Stake { amount, role, staking_addr, rpc_url, key } => {
            use stake::{parse_amount, StakeRole};
            let eth = parse_amount(&amount)?;
            let r   = if role == "validator" { StakeRole::Validator } else { StakeRole::Publisher };
            stake::run(eth, r, key.as_deref(), rpc_url.as_deref(), staking_addr.as_deref()).await?;
        }
        Commands::Dashboard => {
            dashboard::run(cli.node_url.as_deref()).await?;
        }
        Commands::DashboardEnhanced => {
            dashboard_enhanced::run(cli.node_url.as_deref()).await?;
        }
        Commands::Blocks { limit } => {
            blocks::run(cli.node_url.as_deref(), limit).await?;
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            generate(shell, &mut cmd, name, &mut std::io::stdout());
        }
        Commands::Config { command } => {
            match command {
                ConfigCommands::Init => {
                    config_file::Config::init()?;
                }
                ConfigCommands::Show => {
                    let config = config_file::Config::load()?;
                    println!("{}", toml::to_string_pretty(&config)?);
                }
                ConfigCommands::Get { key } => {
                    let config = config_file::Config::load()?;
                    let value = match key.as_str() {
                        "node.url" => config.node.url,
                        "node.timeout" => config.node.timeout.to_string(),
                        "ipfs.url" => config.ipfs.url,
                        "display.colors" => config.display.colors.to_string(),
                        _ => {
                            eprintln!("Unknown config key: {}", key);
                            std::process::exit(1);
                        }
                    };
                    println!("{}", value);
                }
            }
        }
        Commands::Batch { command } => {
            match command {
                BatchCommands::Verify { packages, ecosystem } => {
                    batch::verify_packages(packages, ecosystem.as_deref(), cli.node_url.as_deref()).await;
                }
                BatchCommands::Install { packages, ecosystem, unverified } => {
                    let allow_unverified = unverified || cli.unverified;
                    batch::install_batch(packages, ecosystem.as_deref(), allow_unverified, cli.node_url.as_deref()).await?;
                }
                BatchCommands::VerifyDeps { manifest } => {
                    batch::verify_dependencies(manifest.as_deref(), cli.node_url.as_deref()).await?;
                }
            }
        }
        Commands::Advanced { command } => {
            match command {
                AdvancedCommands::ZkProof { tarball, manifest, output } => {
                    let output_path = output.unwrap_or_else(|| {
                        std::path::PathBuf::from("proof.bin")
                    });
                    advanced::generate_and_save_zk_proof(&tarball, manifest.as_ref(), &output_path).await?;
                }
                AdvancedCommands::MlVerify { tarball, ecosystem, json: _json } => {
                    let result = advanced::ml_verify(&tarball, &ecosystem).await?;
                    println!("ML Verification Result:");
                    println!("  Threat Score: {}/100", result.threat_score);
                    println!("  Threat Level: {:?}", result.threat_level);
                    println!("  Confidence: {:.2}%", result.confidence * 100.0);
                    println!("  Description: {}", result.threat_level.description());
                }
                AdvancedCommands::WasmValidate { tarball, name, version, ecosystem } => {
                    let result = advanced::wasm_validate(&tarball, &name, &version, &ecosystem).await?;
                    println!("WASM Validation Result:");
                    println!("  Success: {}", result.success);
                    println!("  Exit Code: {}", result.exit_code);
                    println!("  CPU Time: {}ms", result.resource_usage.cpu_time_ms);
                    if !result.findings.is_empty() {
                        println!("  Findings:");
                        for finding in &result.findings {
                            println!("    - [{:?}] {}", finding.severity, finding.description);
                        }
                    }
                }
                AdvancedCommands::FullValidate { tarball, name, version, ecosystem, zk, output } => {
                    println!("Running full advanced validation pipeline...");
                    
                    // Step 1: ML Verification
                    println!("\n[1/3] ML-based threat detection...");
                    let ml_result = advanced::ml_verify(&tarball, &ecosystem).await?;
                    println!("  Score: {}/100 ({:?})", ml_result.threat_score, ml_result.threat_level);
                    
                    // Step 2: WASM Validation
                    println!("\n[2/3] WASM sandbox validation...");
                    let wasm_result = advanced::wasm_validate(&tarball, &name, &version, &ecosystem).await?;
                    println!("  Success: {} (Exit: {})", wasm_result.success, wasm_result.exit_code);
                    
                    // Step 3: ZK Proof Generation (if requested)
                    if zk {
                        println!("\n[3/3] ZK proof generation...");
                        let proof = advanced::generate_zk_proof(&tarball, None).await?;
                        println!("  Proof size: {} bytes", proof.len());
                        
                        if let Some(out_dir) = output {
                            let proof_path = out_dir.join("proof.bin");
                            tokio::fs::write(&proof_path, &proof).await?;
                            println!("  Proof saved to {:?}", proof_path);
                        }
                    }
                    
                    println!("\n✓ Validation complete!");
                }
            }
        }
    }
    Ok(())
}
