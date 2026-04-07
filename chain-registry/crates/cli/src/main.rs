// crates/cli/src/main.rs
// `creg` — the main CLI. Wraps the shim logic with a friendly interface.
#![deny(clippy::unwrap_used)]

mod advanced;
mod audit;
mod batch;
mod blocks;
mod config_file;
mod dashboard;
mod dashboard_enhanced;
mod diff;
mod doctor;
mod graph;
mod info;
mod install;
mod intercept;
mod keygen;
mod lockfile;
mod multisig;
mod output;
mod policy;
mod publish;
mod retry;
mod sbom;
mod search;
mod stake;
mod testnet;
mod update;
mod verify;
mod watch;
// New UX modules
mod dashboard_interactive;
mod error_help;
mod explorer_tui;
mod wizard;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use colored::Colorize;
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
        /// Additional Ed25519 private keys for 2-of-3 multi-sig publishing.
        #[arg(long = "extra-key")]
        extra_keys: Vec<String>,
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
        #[arg(short = 'k', long = "key-path")]
        key_path: Option<std::path::PathBuf>,
        /// Rotate an existing key instead of generating a fresh one.
        #[arg(long)]
        rotate: bool,
    },

    /// Manage the local pkg-lock.chain file.
    Lockfile {
        /// Directory containing the lockfile (defaults to cwd).
        #[arg(short, long)]
        dir: Option<std::path::PathBuf>,
        /// Clear the lockfile.
        #[arg(long)]
        clear: bool,
        /// Diff the lockfile against current chain state.
        #[arg(long)]
        diff: bool,
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
        /// Attempt to auto-remediate revoked packages.
        #[arg(long)]
        fix: bool,
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
        /// CI mode: exit 1 on any Critical security event.
        #[arg(long)]
        ci: bool,
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

    /// Check system prerequisites (node, IPFS, key, nsjail, gpg).
    Doctor,

    /// Search for packages in the chain registry.
    Search {
        /// Search query
        query: String,
        /// Ecosystem filter (npm, pip, cargo, …)
        #[arg(short, long)]
        ecosystem: Option<String>,
    },

    /// Show detailed information about a registered package.
    Info {
        /// Package canonical name or "name@version"
        package: String,
        /// Ecosystem
        #[arg(short, long)]
        ecosystem: Option<String>,
    },

    /// Visualize the dependency graph for a package.
    Graph {
        /// Package name (e.g. express@4.18.2)
        package: String,
        /// Ecosystem
        #[arg(short, long)]
        ecosystem: Option<String>,
        /// Maximum recursion depth
        #[arg(short, long, default_value = "3")]
        depth: u32,
    },

    /// Show file-level diff between two published package versions.
    Diff {
        /// First package version (e.g. express@4.17.0)
        pkg_a: String,
        /// Second package version (e.g. express@4.18.2)
        pkg_b: String,
    },

    /// Policy management (policy-as-code).
    Policy {
        #[command(subcommand)]
        command: PolicyCommands,
    },

    /// Export a Software Bill of Materials (SBOM) for a package.
    Sbom {
        /// Package name
        package: String,
        /// Ecosystem
        #[arg(short, long)]
        ecosystem: Option<String>,
        /// SBOM format: spdx (default) or cyclonedx
        #[arg(short, long, default_value = "spdx")]
        format: String,
        /// Save to this file (defaults to stdout)
        #[arg(long, value_name = "FILE")]
        save: Option<std::path::PathBuf>,
    },

    /// Self-update the creg binary.
    Update {
        /// Only check for updates, don't install.
        #[arg(long)]
        check: bool,
    },

    /// Multi-signature publish workflow.
    Multisig {
        #[command(subcommand)]
        command: MultisigCommands,
    },

    /// Interactive setup wizard for new users
    Init,

    /// Interactive TUI dashboard with controls
    DashboardInteractive,

    /// Launch the full TUI blockchain explorer
    Explorer,

    /// Testnet commands (drip, stake, status)
    Testnet {
        #[command(subcommand)]
        command: TestnetCommands,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Initialize a new configuration file
    Init,
    /// Show current configuration
    Show,
    /// Get a specific configuration value
    Get { key: String },
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
        #[arg(short = 'p', long = "proof-out")]
        proof_out: Option<std::path::PathBuf>,
        /// Verify an existing proof file instead of generating
        #[arg(long)]
        verify: Option<std::path::PathBuf>,
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
        #[arg(short = 'd', long = "out-dir")]
        out_dir: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum PolicyCommands {
    /// Show active policy and insurance records
    Show {
        /// Publisher pubkey to query (defaults to CREG_PUBLISHER_PUBKEY env var)
        #[arg(long)]
        pubkey: Option<String>,
    },
    /// Apply a policy file as the active org policy
    Apply {
        /// Path to the policy TOML file
        policy_file: std::path::PathBuf,
        /// Validate the policy file without applying it
        #[arg(long)]
        dry_run: bool,
    },
    /// Print an example policy template
    Init,
}

#[derive(Subcommand)]
enum MultisigCommands {
    /// Initialize a new multisig session for a tarball
    Init {
        /// Path to the package tarball
        tarball: std::path::PathBuf,
        /// Minimum number of signatures required (M-of-N)
        #[arg(short, long, default_value = "2")]
        threshold: usize,
        /// Output session file path
        #[arg(
            short = 's',
            long = "session-out",
            default_value = ".creg-multisig.json"
        )]
        session_out: std::path::PathBuf,
    },
    /// Add your signature to a multisig session
    Sign {
        /// Path to the multisig session file
        session: std::path::PathBuf,
        /// Your Ed25519 private key (hex)
        #[arg(short, long, env = "CREG_PUBLISHER_KEY")]
        key: String,
    },
    /// Submit a completed multisig session to the chain
    Submit {
        /// Path to the multisig session file
        session: std::path::PathBuf,
        /// Optional manifest file
        #[arg(short, long)]
        manifest: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum TestnetCommands {
    /// Request test tokens from the faucet
    Drip {
        /// Ethereum address to receive tokens
        address: String,
        /// Faucet URL (default: http://localhost:8081)
        #[arg(long)]
        faucet_url: Option<String>,
    },
    /// Check testnet status
    Status {
        /// Node URL to check (default: http://localhost:8080)
        #[arg(long)]
        node_url: Option<String>,
    },
    /// Stake test tokens as a publisher
    StakePublisher {
        /// Amount to stake (minimum 0.001 tCREG)
        amount: f64,
        /// Private key for staking (hex, with 0x prefix)
        #[arg(short, long)]
        key: String,
        /// RPC URL (default: http://localhost:8545)
        #[arg(long)]
        rpc_url: Option<String>,
    },
    /// Stake test tokens as a validator
    StakeValidator {
        /// Amount to stake (minimum 0.1 tCREG)
        amount: f64,
        /// Private key for staking (hex, with 0x prefix)
        #[arg(short, long)]
        key: String,
        /// RPC URL (default: http://localhost:8545)
        #[arg(long)]
        rpc_url: Option<String>,
    },
    /// Show testnet documentation
    Docs,
    /// Show testnet reset instructions
    Reset {
        /// Data directory to clear
        #[arg(short, long)]
        data_dir: Option<std::path::PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let json_out = matches!(cli.output, OutputFormat::Json);

    match cli.command {
        Commands::Install {
            package,
            ecosystem,
            unverified,
        } => {
            let allow_unverified = unverified || cli.unverified;
            install::run(
                &package,
                ecosystem.as_deref(),
                allow_unverified,
                cli.node_url.as_deref(),
            )
            .await?;
        }
        Commands::Status {
            package,
            ecosystem,
            json,
        } => {
            let verdict =
                resolver::resolve(&package, ecosystem.as_deref(), cli.node_url.as_deref()).await?;
            if json || json_out {
                println!("{}", serde_json::to_string_pretty(&verdict)?);
            } else {
                output::print_verdict(&verdict);
            }
        }
        Commands::Publish {
            tarball,
            manifest,
            key,
            extra_keys,
            shield,
        } => {
            publish::run(
                &tarball,
                manifest.as_deref(),
                &key,
                &extra_keys,
                cli.node_url.as_deref(),
                shield,
            )
            .await?;
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
        Commands::Keygen {
            role,
            key_path,
            rotate,
        } => {
            if rotate {
                keygen::rotate(key_path.as_deref(), &role)?;
            } else {
                keygen::run(key_path.as_deref(), &role)?;
            }
        }
        Commands::Lockfile { clear, dir, diff } => {
            let d = dir.unwrap_or_else(|| std::env::current_dir().expect("cannot determine current directory"));
            if clear {
                let path = d.join("pkg-lock.chain");
                if path.exists() {
                    std::fs::remove_file(&path)?;
                }
                println!("pkg-lock.chain cleared.");
            } else if diff {
                lockfile::diff(&d, cli.node_url.as_deref()).await?;
            } else {
                lockfile::print_lockfile(&d)?;
            }
        }
        Commands::Audit {
            ecosystem,
            strict,
            json,
            fix,
        } => {
            if fix {
                let code = audit::run_fix(ecosystem.as_deref(), cli.node_url.as_deref()).await?;
                std::process::exit(code);
            } else {
                let code = audit::run(
                    ecosystem.as_deref(),
                    cli.node_url.as_deref(),
                    strict,
                    json || json_out,
                )
                .await?;
                std::process::exit(code);
            }
        }
        Commands::Verify {
            package,
            ecosystem,
            checkpoint,
            json,
        } => {
            verify::run(
                &package,
                ecosystem.as_deref(),
                cli.node_url.as_deref(),
                checkpoint
                    .as_ref()
                    .and_then(|p: &std::path::PathBuf| p.to_str()),
                json || json_out,
            )
            .await?;
        }
        Commands::Watch {
            filter,
            node_url,
            ci,
        } => {
            let url = node_url.or(cli.node_url);
            watch::run(filter.as_deref(), url.as_deref(), ci).await?;
        }
        Commands::Stake {
            amount,
            role,
            staking_addr,
            rpc_url,
            key,
        } => {
            use stake::{parse_amount, StakeRole};
            let eth = parse_amount(&amount)?;
            let r = if role == "validator" {
                StakeRole::Validator
            } else {
                StakeRole::Publisher
            };
            stake::run(
                eth,
                r,
                key.as_deref(),
                rpc_url.as_deref(),
                staking_addr.as_deref(),
            )
            .await?;
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
        Commands::Config { command } => match command {
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
        },
        Commands::Batch { command } => match command {
            BatchCommands::Verify {
                packages,
                ecosystem,
            } => {
                batch::verify_packages(packages, ecosystem.as_deref(), cli.node_url.as_deref())
                    .await;
            }
            BatchCommands::Install {
                packages,
                ecosystem,
                unverified,
            } => {
                let allow_unverified = unverified || cli.unverified;
                batch::install_batch(
                    packages,
                    ecosystem.as_deref(),
                    allow_unverified,
                    cli.node_url.as_deref(),
                )
                .await?;
            }
            BatchCommands::VerifyDeps { manifest } => {
                batch::verify_dependencies(manifest.as_deref(), cli.node_url.as_deref()).await?;
            }
        },
        Commands::Advanced { command } => match command {
            AdvancedCommands::ZkProof {
                tarball,
                manifest,
                proof_out,
                verify,
            } => {
                if let Some(proof_path) = verify {
                    let valid = advanced::verify_zk_proof_file(&proof_path, &tarball).await?;
                    if json_out {
                        println!("{}", serde_json::json!({ "valid": valid }));
                    } else {
                        if valid {
                            println!("{} ZK proof is VALID", "✓".green().bold());
                        } else {
                            println!("{} ZK proof is INVALID", "✗".red().bold());
                            std::process::exit(1);
                        }
                    }
                } else {
                    let output_path =
                        proof_out.unwrap_or_else(|| std::path::PathBuf::from("proof.bin"));
                    advanced::generate_and_save_zk_proof(&tarball, manifest.as_ref(), &output_path)
                        .await?;
                }
            }
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
                            "confidence":   result.confidence,
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
                            "success":   result.success,
                            "exit_code": result.exit_code,
                            "findings":  result.findings.len(),
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
                zk,
                out_dir,
            } => {
                println!("Running full advanced validation pipeline...");
                println!("\n[1/3] ML-based threat detection...");
                let ml_result = advanced::ml_verify(&tarball, &ecosystem).await?;
                println!(
                    "  Score: {}/100 ({:?})",
                    ml_result.threat_score, ml_result.threat_level
                );

                println!("\n[2/3] WASM sandbox validation...");
                let wasm_result =
                    advanced::wasm_validate(&tarball, &name, &version, &ecosystem).await?;
                println!(
                    "  Success: {} (Exit: {})",
                    wasm_result.success, wasm_result.exit_code
                );

                if zk {
                    println!("\n[3/3] ZK proof generation...");
                    let proof = advanced::generate_zk_proof(&tarball, None).await?;
                    println!("  Proof size: {} bytes", proof.len());
                    if let Some(out_dir) = out_dir {
                        let proof_path = out_dir.join("proof.bin");
                        tokio::fs::write(&proof_path, &proof).await?;
                        println!("  Proof saved to {:?}", proof_path);
                    }
                }
                println!("\n{} Validation complete!", "✓".green().bold());
            }
        },

        // ── New commands ──────────────────────────────────────────────────────
        Commands::Doctor => {
            doctor::run(cli.node_url.as_deref()).await?;
        }
        Commands::Search { query, ecosystem } => {
            search::run(
                &query,
                ecosystem.as_deref(),
                cli.node_url.as_deref(),
                json_out,
            )
            .await?;
        }
        Commands::Info { package, ecosystem } => {
            info::run(
                &package,
                ecosystem.as_deref(),
                cli.node_url.as_deref(),
                json_out,
            )
            .await?;
        }
        Commands::Graph {
            package,
            ecosystem,
            depth,
        } => {
            graph::run(
                &package,
                ecosystem.as_deref(),
                depth,
                cli.node_url.as_deref(),
                json_out,
            )
            .await?;
        }
        Commands::Diff { pkg_a, pkg_b } => {
            diff::run(&pkg_a, &pkg_b, cli.node_url.as_deref(), json_out).await?;
        }
        Commands::Policy { command } => match command {
            PolicyCommands::Show { pubkey } => {
                policy::show(pubkey.as_deref(), cli.node_url.as_deref(), json_out).await?;
            }
            PolicyCommands::Apply {
                policy_file,
                dry_run,
            } => {
                policy::apply(&policy_file, dry_run).await?;
            }
            PolicyCommands::Init => {
                policy::show_policy_init()?;
            }
        },
        Commands::Sbom {
            package,
            ecosystem,
            format,
            save,
        } => {
            let fmt: sbom::SbomFormat = format.parse()?;
            sbom::run(
                &package,
                ecosystem.as_deref(),
                fmt,
                save.as_deref(),
                cli.node_url.as_deref(),
            )
            .await?;
        }
        Commands::Update { check } => {
            update::run(cli.node_url.as_deref(), check).await?;
        }
        Commands::Multisig { command } => match command {
            MultisigCommands::Init {
                tarball,
                threshold,
                session_out,
            } => {
                multisig::init(&tarball, threshold, cli.node_url.as_deref(), &session_out).await?;
            }
            MultisigCommands::Sign { session, key } => {
                multisig::sign(&session, &key)?;
            }
            MultisigCommands::Submit { session, manifest } => {
                multisig::submit(&session, manifest.as_deref(), cli.node_url.as_deref()).await?;
            }
        },
        Commands::Init => {
            wizard::run().await?;
        }
        Commands::DashboardInteractive => {
            dashboard_interactive::run(cli.node_url.as_deref()).await?;
        }
        Commands::Explorer => {
            explorer_tui::run(cli.node_url.as_deref()).await?;
        }
        Commands::Testnet { command } => match command {
            TestnetCommands::Drip {
                address,
                faucet_url,
            } => {
                testnet::drip(&address, faucet_url.as_deref()).await?;
            }
            TestnetCommands::Status { node_url } => {
                testnet::status(node_url.as_deref()).await?;
            }
            TestnetCommands::StakePublisher {
                amount,
                key,
                rpc_url,
            } => {
                testnet::stake_publisher(amount, &key, rpc_url.as_deref()).await?;
            }
            TestnetCommands::StakeValidator {
                amount,
                key,
                rpc_url,
            } => {
                testnet::stake_validator(amount, &key, rpc_url.as_deref()).await?;
            }
            TestnetCommands::Docs => {
                testnet::docs();
            }
            TestnetCommands::Reset { data_dir } => {
                testnet::reset(data_dir)?;
            }
        },
    }
    Ok(())
}
