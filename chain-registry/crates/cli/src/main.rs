// crates/cli/src/main.rs
// `creg` — the main CLI. Wraps the shim logic with a friendly interface.
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod audit;
mod auth;
mod batch;
mod blocks;
mod chain_spec;
mod config_file;
mod diff;
mod doctor;
mod error_help;
mod explorer_tui;
mod faucet_client;
mod graph;
mod info;
mod install;
mod intercept;
mod keygen;
mod lockfile;
pub mod lockfile_attestation;
mod multisig;
mod observer;
mod output;
mod policy;
mod profile;
mod publish;
mod recovery;
pub mod registry_proxy;
mod retry;
mod sbom;
mod search;
mod secret_input;
mod stake;
mod supply_chain;
mod testnet;
mod update;
mod verify;
mod watch;
mod witness;
mod wizard;

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use colored::Colorize;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "creg",
    version = env!("CREG_EFFECTIVE_VERSION"),
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

    /// gRPC endpoint for package submission. Overrides automatic host:50051 inference.
    #[arg(long, global = true, env = "CREG_GRPC_URL")]
    grpc_url: Option<String>,

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

    /// Inspect a validator's current performance_score (for committee selection bias).
    /// Higher score = better demonstrated ability to run verification (resources + reliability).
    Performance {
        /// Validator node id or EVM address
        id: String,
        /// Node API URL (default from CREG_NODE_URL or config)
        #[arg(long, env = "CREG_NODE_URL")]
        node_url: Option<String>,
        /// Output raw JSON
        #[arg(long)]
        json: bool,
    },

    /// Dump a ChainRecord (verified package on chain).
    /// Pretty-prints the entire raw record (including all fields) plus a focused
    /// committee section showing members with at-finalization-time performance scores.
    ChainRecord {
        /// Package canonical id (e.g. npm:express@4.18.0)
        canonical: String,
        /// Node API URL
        #[arg(long, env = "CREG_NODE_URL")]
        node_url: Option<String>,
        /// Output raw JSON instead of pretty summary
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
        /// Path to file containing publisher's Ed25519 private key (hex-encoded).
        /// Defaults to active profile key if omitted.
        #[arg(short, long = "key-file", env = "CREG_KEY_FILE")]
        key: Option<std::path::PathBuf>,
        /// Paths to additional Ed25519 private key files for 2-of-3 multi-sig publishing.
        #[arg(long = "extra-key-file")]
        extra_keys: Vec<std::path::PathBuf>,
        /// Experimental threshold encryption (requires CREG_SHIELDED_PUBLISH_ENABLED on CLI and node).
        #[arg(long, hide = true)]
        shield: bool,
        /// Publisher EVM address with active on-chain stake.
        /// Defaults to active profile wallet address if omitted.
        #[arg(long, env = "CREG_PUBLISHER_ADDRESS")]
        publisher_address: Option<String>,
        /// IPFS API URL for upload/pin. Overrides `CREG_IPFS_URL` and config file.
        #[arg(long, env = "CREG_IPFS_URL")]
        ipfs_url: Option<String>,
        /// Offline signing: produce a signed JSON file instead of
        /// submitting to the node.  Use `creg submit-signed <file>` later.
        #[arg(long)]
        offline: Option<std::path::PathBuf>,
    },

    /// Submit a pre-signed publish request (from `creg publish --offline`).
    SubmitSigned {
        /// Path to the signed JSON file produced by `creg publish --offline`.
        signed_file: std::path::PathBuf,
    },

    /// Social recovery: split a key into Shamir guardian shares or reconstruct it.
    Recovery {
        #[command(subcommand)]
        action: RecoveryAction,
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

    /// Generate a new keypair for publishing, validator, witness, or log-signer use.
    Keygen {
        /// Role: publisher | validator | witness | log-signer
        #[arg(default_value = "publisher")]
        role: String,
        /// Output file path (defaults to ~/.creg/<role>.key, or ~/.creg/<role>.hybrid.key for hybrid)
        #[arg(short = 'k', long = "key-path")]
        key_path: Option<std::path::PathBuf>,
        /// Signature scheme: ed25519 (default) | hybrid | hybrid-ed25519-mldsa44-v1
        #[arg(long, default_value = "ed25519")]
        scheme: String,
        /// Rotate an existing key instead of generating a fresh one.
        #[arg(long)]
        rotate: bool,
        /// Generate key from a BIP39 mnemonic phrase (12 or 24 words).
        /// If not provided, generates a new random key.
        #[arg(long)]
        mnemonic: bool,
        /// Restore key from an existing BIP39 mnemonic phrase.
        #[arg(long)]
        restore: bool,
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

    /// Independent transparency-log witness (not a validator; does not join PBFT).
    Witness {
        #[command(subcommand)]
        command: WitnessCommands,
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

    /// Stake CREG tokens as a publisher or validator (ERC-20 approve + stake).
    Stake {
        /// Amount of CREG tokens (e.g. "1" publisher, "100" validator).
        amount: String,
        /// Role: "publisher" | "validator".
        #[arg(short, long, default_value = "publisher")]
        role: String,
        /// Staking contract address (0x…).
        #[arg(long, env = "CREG_STAKING_ADDR")]
        staking_addr: Option<String>,
        /// CREG token (ERC-20) address for the approve step (0x…).
        #[arg(long, env = "CREG_TOKEN_ADDR")]
        token_addr: Option<String>,
        /// EVM RPC URL.
        #[arg(long)]
        rpc_url: Option<String>,
        /// Path to file containing the EVM (secp256k1) private key (hex).
        #[arg(long = "key-file", env = "CREG_KEY_FILE")]
        key: Option<std::path::PathBuf>,
    },

    /// Launch the validator console.
    ///
    /// Interactive TUI: tab bar, command palette (`:` / Ctrl+P), list filter (`/`),
    /// copy selection (`y`), and a confirm-to-quit flow. Legacy commands remain as
    /// hidden aliases: `dashboard`, `dashboard-enhanced`, `dashboard-interactive`,
    /// `explorer`.
    #[command(
        alias = "dashboard",
        alias = "dashboard-enhanced",
        alias = "dashboard-interactive",
        alias = "explorer"
    )]
    Console,

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

    /// Check system prerequisites or run end-to-end local testnet checks.
    Doctor {
        /// Run the local bootstrap testnet checks (node, contracts, faucet, explorer, IPFS, Anvil).
        #[arg(long)]
        testnet: bool,
        /// Faucet base URL for `--testnet` checks.
        #[arg(long, env = "CREG_FAUCET_URL")]
        faucet_url: Option<String>,
        /// Ethereum RPC URL for `--testnet` checks.
        #[arg(long, env = "CREG_ETH_RPC")]
        eth_rpc_url: Option<String>,
        /// Explorer base URL for `--testnet` checks.
        #[arg(long, env = "CREG_EXPLORER_URL")]
        explorer_url: Option<String>,
        /// Skip the explorer HTTP probe in `--testnet` mode.
        #[arg(long)]
        skip_explorer: bool,
        /// Skip the faucet drip probe in `--testnet` mode.
        #[arg(long)]
        skip_drip: bool,
        /// Recipient address for the faucet drip probe. Defaults to a random throwaway address.
        #[arg(long)]
        recipient: Option<String>,
    },

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

    /// Export a Software Bill of Materials (SBOM).
    Sbom {
        /// Package name (omit when using --workspace)
        package: Option<String>,
        /// Ecosystem
        #[arg(short, long)]
        ecosystem: Option<String>,
        /// SBOM format: spdx (default) or cyclonedx
        #[arg(short, long, default_value = "spdx")]
        format: String,
        /// Save to this file (defaults to stdout)
        #[arg(long, value_name = "FILE")]
        save: Option<std::path::PathBuf>,
        /// Inventory every crate in this repo's Cargo.lock (not a registry package)
        #[arg(long)]
        workspace: bool,
        /// Cargo.lock path (workspace mode)
        #[arg(long)]
        lockfile: Option<std::path::PathBuf>,
    },

    /// First-party Cargo.lock policy and provenance (not SLSA).
    SupplyChain {
        #[command(subcommand)]
        command: SupplyChainCommands,
    },

    /// Self-update the creg binary.
    Update {
        /// Only check for updates, don't install.
        #[arg(long)]
        check: bool,
        /// Install only after the release has been verified and explicitly confirmed.
        #[arg(long)]
        yes: bool,
        /// Update to a specific semver release tag (for rollback or rehearsal).
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
    },

    /// Multi-signature publish workflow.
    Multisig {
        #[command(subcommand)]
        command: MultisigCommands,
    },

    /// Interactive setup wizard for new users
    Init,

    /// Testnet commands (drip, stake, status)
    Testnet {
        #[command(subcommand)]
        command: TestnetCommands,
    },

    /// Chain spec operations
    ChainSpec {
        #[command(subcommand)]
        command: chain_spec::ChainSpecCommands,
    },

    /// High-Availability Observer pool operations and failover verification
    Observer {
        #[command(subcommand)]
        command: ObserverCommands,
    },

    /// Authenticate your Ethereum wallet (MetaMask) and derive CREG consensus identity.
    Login {
        /// File holding the Ethereum private key (hex) for headless/CI login
        /// without a browser. Must not be group/world-readable on Unix.
        #[arg(long, value_name = "PATH")]
        private_key_file: Option<std::path::PathBuf>,

        /// Reference to the Ethereum private key: `env:VAR` or `file:PATH`.
        #[arg(long, value_name = "env:VAR|file:PATH")]
        private_key_ref: Option<String>,

        /// Deprecated: exposes the key on argv. Refused unless
        /// CREG_ALLOW_ARGV_SECRETS=1.
        #[arg(long, hide = true)]
        private_key: Option<String>,

        /// Base64 or JSON authorization token from the Web bridge.
        #[arg(long)]
        token: Option<String>,

        /// Local callback port for browser OAuth loopback (defaults to 18888).
        #[arg(long, default_value_t = 18888)]
        port: u16,

        /// Do not automatically open the browser; print the URL only.
        #[arg(long)]
        no_browser: bool,
    },

    /// Log out and clear the active authenticated profile.
    Logout,

    /// View or manage your authenticated CREG profile.
    Profile {
        #[command(subcommand)]
        command: Option<ProfileCommands>,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum ProfileCommands {
    /// Show the active authenticated profile.
    Show,
    /// Clear the active authenticated profile.
    Clear,
}

#[derive(Subcommand)]
enum ObserverCommands {
    /// Probe observer load balancer and replica pool status
    Status {
        /// Load Balancer URL (defaults to http://localhost:8090)
        #[arg(long, default_value = "http://localhost:8090")]
        lb_url: String,
        /// Direct observer replica URLs (comma-separated or multiple flags)
        #[arg(short, long, value_delimiter = ',')]
        replicas: Vec<String>,
    },
    /// Run burst query drill against the observer load balancer to verify continuous routing
    Probe {
        /// Load Balancer URL (defaults to http://localhost:8090)
        #[arg(long, default_value = "http://localhost:8090")]
        lb_url: String,
        /// Number of queries to dispatch
        #[arg(short, long, default_value = "100")]
        count: usize,
    },
}

#[derive(Subcommand)]
enum WitnessCommands {
    /// Poll a log's signed tree head, detect split-view, and write witnessed-head.json.
    Run {
        /// Log node URL (the log operator, not this witness).
        #[arg(long, env = "CREG_NODE_URL")]
        node_url: Option<String>,
        /// Transparency log id.
        #[arg(long, env = "CREG_TRANSPARENCY_LOG_ID")]
        log_id: String,
        /// Log operator Ed25519 public key (hex).
        #[arg(long, env = "CREG_TRANSPARENCY_LOG_PUBKEY")]
        log_pubkey: String,
        /// This operator's witness id (must not equal the log signer id).
        #[arg(long, env = "CREG_WITNESS_ID")]
        witness_id: String,
        /// Path to this operator's witness secret (default ~/.creg/witness.key).
        #[arg(long, env = "CREG_WITNESS_KEY")]
        key_path: Option<std::path::PathBuf>,
        /// Persisted last-accepted head and split-view alert.
        #[arg(long)]
        state_path: Option<std::path::PathBuf>,
        /// Where to write WitnessedTreeHead JSON for clients to fetch.
        #[arg(long)]
        out_path: Option<std::path::PathBuf>,
        /// Fetch, sign, and exit (for tests and systemd oneshot).
        #[arg(long)]
        once: bool,
        /// Poll interval when not --once.
        #[arg(long, default_value_t = 30)]
        interval_secs: u64,
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
enum SupplyChainCommands {
    /// Verify Cargo.lock against config/supply-chain/policy.toml
    Check {
        #[arg(long)]
        lockfile: Option<std::path::PathBuf>,
        #[arg(long)]
        policy: Option<std::path::PathBuf>,
    },
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
        /// Publisher EVM address with active on-chain stake.
        #[arg(long, env = "CREG_PUBLISHER_ADDRESS")]
        publisher_address: String,
    },
    /// Add your signature to a multisig session
    Sign {
        /// Path to the multisig session file
        session: std::path::PathBuf,
        /// Path to file containing your Ed25519 private key (hex)
        #[arg(short, long = "key-file", env = "CREG_KEY_FILE")]
        key: std::path::PathBuf,
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
    /// Start a local multi-node testnet cluster
    Start {
        /// Number of validator nodes to launch (default: 4)
        #[arg(short, long, default_value = "4")]
        nodes: usize,
        /// Base port for cluster services (default: 8080)
        #[arg(short, long, default_value = "8080")]
        port_base: u16,
        /// Cluster data directory (defaults to ~/.creg/testnet)
        #[arg(long)]
        data_dir: Option<std::path::PathBuf>,
        /// Run cluster in background
        #[arg(long)]
        background: bool,
    },
    /// Stop running local multi-node testnet cluster
    Stop {
        /// Cluster data directory
        #[arg(long)]
        data_dir: Option<std::path::PathBuf>,
    },
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
        /// Amount to stake in tCREG, as a decimal (minimum 1)
        amount: String,
        /// File holding the staking private key (hex, with 0x prefix)
        #[arg(long, value_name = "PATH")]
        key_file: Option<std::path::PathBuf>,
        /// Reference to the staking private key: `env:VAR` or `file:PATH`
        #[arg(long, value_name = "env:VAR|file:PATH")]
        key_ref: Option<String>,
        /// Deprecated: exposes the key on argv. Refused unless
        /// CREG_ALLOW_ARGV_SECRETS=1.
        #[arg(short, long, hide = true)]
        key: Option<String>,
        /// RPC URL (default: http://localhost:8545)
        #[arg(long)]
        rpc_url: Option<String>,
    },
    /// Stake test tokens as a validator
    StakeValidator {
        /// Amount to stake in tCREG, as a decimal (minimum 100)
        amount: String,
        /// File holding the staking private key (hex, with 0x prefix)
        #[arg(long, value_name = "PATH")]
        key_file: Option<std::path::PathBuf>,
        /// Reference to the staking private key: `env:VAR` or `file:PATH`
        #[arg(long, value_name = "env:VAR|file:PATH")]
        key_ref: Option<String>,
        /// Deprecated: exposes the key on argv. Refused unless
        /// CREG_ALLOW_ARGV_SECRETS=1.
        #[arg(short, long, hide = true)]
        key: Option<String>,
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

#[derive(Subcommand)]
enum RecoveryAction {
    /// Split a private key into Shamir guardian shares.
    Split {
        /// Path to the private key file to split.
        #[arg(long)]
        key: std::path::PathBuf,
        /// Guardian names (comma-separated, e.g. "alice,bob,carol").
        #[arg(long, value_delimiter = ',')]
        guardians: Vec<String>,
        /// Minimum number of shares needed to reconstruct (default: 2).
        #[arg(long, default_value = "2")]
        threshold: u8,
        /// Directory to write share files into.
        #[arg(long, default_value = "./recovery-shares")]
        output_dir: std::path::PathBuf,
    },
    /// Reconstruct a private key from guardian shares.
    Reconstruct {
        /// Paths to share JSON files.
        #[arg(required = true)]
        shares: Vec<std::path::PathBuf>,
        /// Output path for the reconstructed key.
        #[arg(long, default_value = "./recovered-key.enc")]
        output: std::path::PathBuf,
    },
}

fn main() -> Result<()> {
    let builder = std::thread::Builder::new()
        .name("creg-main".into())
        .stack_size(16 * 1024 * 1024);

    let handler = builder.spawn(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_stack_size(16 * 1024 * 1024)
            .build()
            .expect("Failed to build Tokio runtime");
        rt.block_on(async_main())
    })?;

    match handler.join() {
        Ok(res) => res,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

async fn async_main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();

    if let Err(e) = Box::pin(run(cli)).await {
        error_help::print_error_with_help(&e.to_string());
        std::process::exit(1);
    }
    Ok(())
}

async fn run(cli: Cli) -> Result<()> {
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

        Commands::Performance { id, node_url, json } => {
            let url = node_url
                .or_else(|| cli.node_url.clone())
                .unwrap_or_else(|| "http://localhost:28180".to_string());
            let normalized = id.trim().to_ascii_lowercase();
            // Simple query to profile endpoint which now includes performance_score
            let client = reqwest::Client::new();
            let resp = client
                .get(format!(
                    "{}/v1/validators/{}",
                    url.trim_end_matches('/'),
                    normalized
                ))
                .send()
                .await?;
            if resp.status().is_success() {
                let val: serde_json::Value = resp.json().await?;
                if json || json_out {
                    println!("{}", serde_json::to_string_pretty(&val)?);
                } else {
                    println!("Validator: {}", normalized);
                    println!(
                        "  performance_score: {}",
                        val.get("performance_score")
                            .unwrap_or(&serde_json::json!(0))
                    );
                    println!(
                        "  contribution_count: {}",
                        val.get("contribution_count")
                            .unwrap_or(&serde_json::json!(0))
                    );
                    println!(
                        "  stake: {}",
                        val.get("stake").unwrap_or(&serde_json::json!("?"))
                    );
                    println!(
                        "  reputation: {}",
                        val.get("reputation").unwrap_or(&serde_json::json!(0))
                    );
                    if let Some(inc) = val.get("incentives") {
                        println!("  incentives:");
                        println!(
                            "    rewards_contract: {}",
                            inc.get("validator_rewards_contract")
                                .unwrap_or(&serde_json::json!(""))
                        );
                        println!("    (See workflow.md §9.12 for emission on Verified, points, and selection feedback)");
                    }
                    println!("(Higher performance_score = preferred for mandatory analysis committees if resources/capable)");
                }
            } else {
                eprintln!(
                    "Failed to fetch validator performance for {}: {}",
                    normalized,
                    resp.status()
                );
            }
        }

        Commands::ChainRecord {
            canonical,
            node_url,
            json,
        } => {
            let url = node_url
                .or_else(|| cli.node_url.clone())
                .unwrap_or_else(|| "http://localhost:28180".to_string());
            let canon = urlencoding::encode(&canonical).into_owned();
            let client = reqwest::Client::new();
            // Fetch full package record; non-JSON mode pretty-prints the entire raw ChainRecord + committee details
            let resp = client
                .get(format!(
                    "{}/v1/packages/{}",
                    url.trim_end_matches('/'),
                    canon
                ))
                .send()
                .await?;
            if resp.status().is_success() {
                let val: serde_json::Value = resp.json().await?;
                if json || json_out {
                    println!("{}", serde_json::to_string_pretty(&val)?);
                } else {
                    println!("ChainRecord for {}:", canonical);
                    // Pretty-print the entire raw ChainRecord (not just the committee part)
                    println!("{}", serde_json::to_string_pretty(&val)?);

                    // Additionally provide a focused committee breakdown with at-finalization scores
                    if let Some(committee) = val.get("analysis_committee") {
                        println!(
                            "\n=== Committee details (at-finalization performance scores) ==="
                        );
                        println!(
                            "  size: {}",
                            committee.get("size").unwrap_or(&serde_json::json!(0))
                        );
                        println!(
                            "  performance_bias: {}",
                            committee
                                .get("performance_bias")
                                .unwrap_or(&serde_json::json!(false))
                        );
                        if let Some(members) = committee.get("members").and_then(|m| m.as_array()) {
                            println!(
                                "  members (id + performance_score_at_finalization + current):"
                            );
                            for m in members {
                                println!("    - {}", serde_json::to_string(m).unwrap_or_default());
                            }
                        }
                        if let Some(scores) = committee.get("member_performance_at_finalization") {
                            println!(
                                "  member_performance_at_finalization: {}",
                                serde_json::to_string_pretty(scores).unwrap_or_default()
                            );
                        }
                    } else if let Some(size) = val.get("analysis_committee_size") {
                        println!("\n(legacy scalars) committee_size: {}", size);
                    }
                }
            } else {
                eprintln!(
                    "Failed to fetch ChainRecord for {}: {}",
                    canonical,
                    resp.status()
                );
            }
        }
        Commands::Publish {
            tarball,
            manifest,
            key,
            extra_keys,
            shield,
            publisher_address,
            ipfs_url,
            offline,
        } => {
            let (key_str, resolved_publisher_address) = profile::resolve_publisher_credentials(
                key.as_deref(),
                publisher_address.as_deref(),
            )?;
            let extra_key_strs: Vec<String> = extra_keys
                .iter()
                .map(|p| {
                    std::fs::read_to_string(p)
                        .with_context(|| format!("Cannot read extra key file: {}", p.display()))
                        .map(|s| s.trim().to_string())
                })
                .collect::<Result<_>>()?;
            if let Some(output_path) = offline {
                publish::sign_offline(
                    &tarball,
                    manifest.as_deref(),
                    &key_str,
                    &extra_key_strs,
                    &resolved_publisher_address,
                    shield,
                    ipfs_url.as_deref(),
                    &output_path,
                )
                .await?;
            } else {
                publish::run(
                    &tarball,
                    manifest.as_deref(),
                    &key_str,
                    &extra_key_strs,
                    &resolved_publisher_address,
                    cli.node_url.as_deref(),
                    cli.grpc_url.as_deref(),
                    ipfs_url.as_deref(),
                    shield,
                )
                .await?;
            }
        }
        Commands::SubmitSigned { signed_file } => {
            publish::submit_signed(&signed_file, cli.node_url.as_deref()).await?;
        }
        Commands::Login {
            private_key_file,
            private_key_ref,
            private_key,
            token,
            port,
            no_browser,
        } => {
            let node_url = config_file::effective_node_url(cli.node_url.as_deref());
            let private_key = secret_input::resolve_secret(secret_input::SecretSources {
                file: private_key_file.as_deref(),
                reference: private_key_ref.as_deref(),
                argv: private_key.as_deref(),
                argv_flag: "--private-key",
                replacement: "--private-key-file <path> or --private-key-ref env:VAR|file:PATH",
            })?;

            let identity = if let Some(ref tok) = token {
                println!("  Verifying authorization token...");
                auth::derive_from_token(
                    tok,
                    auth::DEFAULT_CHAIN_ID,
                    auth::DEFAULT_VERIFYING_CONTRACT,
                )?
            } else if let Some(ref pk) = private_key {
                println!("  Deriving CREG identity from provided Ethereum private key...");
                auth::derive_from_private_key(
                    pk,
                    auth::DEFAULT_CHAIN_ID,
                    auth::DEFAULT_VERIFYING_CONTRACT,
                )?
            } else {
                auth::start_loopback_auth(
                    port,
                    no_browser,
                    auth::DEFAULT_CHAIN_ID,
                    auth::DEFAULT_VERIFYING_CONTRACT,
                )
                .await?
            };

            let prof = profile::save_identity(&identity, &node_url)?;
            println!();
            println!("  {} Authentication successful!", "✓".green().bold());
            profile::print_profile(&prof);
            println!("  You can now publish packages without any key flags:");
            println!("    {}", "creg publish <tarball.tgz>".cyan());
            println!();
        }
        Commands::Logout => {
            if profile::clear_profile()? {
                println!(
                    "  {} Successfully logged out. Active profile removed.",
                    "✓".green()
                );
            } else {
                println!("  No active profile found. You are already logged out.");
            }
        }
        Commands::Profile { command } => match command.unwrap_or(ProfileCommands::Show) {
            ProfileCommands::Show => {
                if let Some(prof) = profile::load_active_profile()? {
                    profile::print_profile(&prof);
                } else {
                    println!();
                    println!("  No active authenticated profile found.");
                    println!(
                        "  To link your Ethereum wallet, run: {}",
                        "creg login".cyan().bold()
                    );
                    println!();
                }
            }
            ProfileCommands::Clear => {
                if profile::clear_profile()? {
                    println!("  {} Active profile cleared.", "✓".green());
                } else {
                    println!("  No profile found to clear.");
                }
            }
        },
        Commands::Recovery { action } => match action {
            RecoveryAction::Split {
                key,
                guardians,
                threshold,
                output_dir,
            } => {
                recovery::run_split(&key, &guardians, threshold, &output_dir)?;
            }
            RecoveryAction::Reconstruct { shares, output } => {
                recovery::run_reconstruct(&shares, &output)?;
            }
        },
        Commands::SetupShims { shim_dir } => {
            intercept::setup_shims(shim_dir.as_deref())?;
        }
        Commands::RemoveShims => {
            intercept::remove_shims()?;
        }
        Commands::Cache { clear } => {
            if clear {
                resolver::cache::clear()?;
                println!("Verification cache and freshness journal cleared.");
            } else {
                resolver::cache::print_entries()?;
            }
        }
        Commands::Keygen {
            role,
            key_path,
            scheme,
            rotate,
            mnemonic,
            restore,
        } => match keygen::parse_keygen_scheme(&scheme)? {
            common::crypto::SignatureScheme::Ed25519 => {
                if rotate {
                    keygen::rotate(key_path.as_deref(), &role)?;
                } else if mnemonic {
                    keygen::run_with_mnemonic(key_path.as_deref(), &role, false)?;
                } else if restore {
                    keygen::run_with_mnemonic(key_path.as_deref(), &role, true)?;
                } else {
                    keygen::run(key_path.as_deref(), &role)?;
                }
            }
            common::crypto::SignatureScheme::HybridEd25519MlDsa44V1 => {
                if mnemonic || restore {
                    anyhow::bail!(
                        "BIP-39 mnemonic derivation is Ed25519-only; hybrid keys generate independent OS-random Ed25519 and ML-DSA-44 seeds (ADR-017 §4)."
                    );
                }
                if rotate {
                    anyhow::bail!("--rotate is not yet supported for --scheme hybrid");
                }
                keygen::run_hybrid(key_path.as_deref(), &role)?;
            }
        },
        Commands::Lockfile { clear, dir, diff } => {
            let d = dir.unwrap_or_else(|| {
                std::env::current_dir().expect("cannot determine current directory")
            });
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
        Commands::Witness { command } => match command {
            WitnessCommands::Run {
                node_url,
                log_id,
                log_pubkey,
                witness_id,
                key_path,
                state_path,
                out_path,
                once,
                interval_secs,
            } => {
                let node_url = node_url
                    .or(cli.node_url)
                    .or_else(|| std::env::var("CREG_NODE_URL").ok())
                    .unwrap_or_else(|| "http://127.0.0.1:8080".into());
                witness::run(witness::WitnessRunArgs {
                    node_url,
                    log_id,
                    log_public_key: log_pubkey,
                    witness_id,
                    key_path: key_path.unwrap_or_else(witness::default_witness_key_path),
                    state_path: state_path.unwrap_or_else(witness::default_state_path),
                    out_path: out_path.unwrap_or_else(witness::default_out_path),
                    once,
                    interval_secs,
                })
                .await?;
            }
        },
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
            token_addr,
            rpc_url,
            key,
        } => {
            use stake::{parse_amount, StakeRole};
            let tokens = parse_amount(&amount)?;
            let r = if role == "validator" {
                StakeRole::Validator
            } else {
                StakeRole::Publisher
            };
            stake::run(
                tokens,
                r,
                key.as_deref(),
                rpc_url.as_deref(),
                staking_addr.as_deref(),
                token_addr.as_deref(),
            )
            .await?;
        }
        Commands::Console => {
            explorer_tui::run(cli.node_url.as_deref()).await?;
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

        // ── New commands ──────────────────────────────────────────────────────
        Commands::Doctor {
            testnet,
            faucet_url,
            eth_rpc_url,
            explorer_url,
            skip_explorer,
            skip_drip,
            recipient,
        } => {
            doctor::run(doctor::DoctorOptions {
                node_url: cli.node_url.as_deref(),
                json: json_out,
                testnet,
                faucet_url: faucet_url.as_deref(),
                eth_rpc_url: eth_rpc_url.as_deref(),
                explorer_url: explorer_url.as_deref(),
                skip_explorer,
                skip_drip,
                recipient: recipient.as_deref(),
            })
            .await?;
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
            workspace,
            lockfile,
        } => {
            let fmt: sbom::SbomFormat = format.parse()?;
            sbom::run(
                package.as_deref(),
                ecosystem.as_deref(),
                fmt,
                save.as_deref(),
                cli.node_url.as_deref(),
                workspace,
                lockfile.as_deref(),
            )
            .await?;
        }
        Commands::SupplyChain { command } => match command {
            SupplyChainCommands::Check { lockfile, policy } => {
                supply_chain::run_check(lockfile.as_deref(), policy.as_deref())?;
            }
        },
        Commands::Update {
            check,
            yes,
            version,
        } => {
            update::run(cli.node_url.as_deref(), check, yes, version.as_deref()).await?;
        }
        Commands::Multisig { command } => match command {
            MultisigCommands::Init {
                tarball,
                threshold,
                session_out,
                publisher_address,
            } => {
                multisig::init(
                    &tarball,
                    threshold,
                    &publisher_address,
                    cli.node_url.as_deref(),
                    &session_out,
                )
                .await?;
            }
            MultisigCommands::Sign { session, key } => {
                let key_content = std::fs::read_to_string(&key)
                    .with_context(|| format!("Cannot read key file: {}", key.display()))?;
                multisig::sign(&session, key_content.trim())?;
            }
            MultisigCommands::Submit { session, manifest } => {
                multisig::submit(&session, manifest.as_deref(), cli.node_url.as_deref()).await?;
            }
        },
        Commands::Init => {
            wizard::run().await?;
        }
        Commands::ChainSpec { command } => {
            chain_spec::run(command).await?;
        }
        Commands::Testnet { command } => match command {
            TestnetCommands::Start {
                nodes,
                port_base,
                data_dir,
                background,
            } => {
                testnet::start_cluster(nodes, port_base, data_dir, background).await?;
            }
            TestnetCommands::Stop { data_dir } => {
                testnet::stop_cluster(data_dir).await?;
            }
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
                key_file,
                key_ref,
                key,
                rpc_url,
            } => {
                let amount_wei = secret_input::parse_token_amount_wei(&amount)?;
                let key = secret_input::staking_key(
                    key_file.as_deref(),
                    key_ref.as_deref(),
                    key.as_deref(),
                )?;
                testnet::stake_publisher(amount_wei, &key, rpc_url.as_deref()).await?;
            }
            TestnetCommands::StakeValidator {
                amount,
                key_file,
                key_ref,
                key,
                rpc_url,
            } => {
                let amount_wei = secret_input::parse_token_amount_wei(&amount)?;
                let key = secret_input::staking_key(
                    key_file.as_deref(),
                    key_ref.as_deref(),
                    key.as_deref(),
                )?;
                testnet::stake_validator(amount_wei, &key, rpc_url.as_deref()).await?;
            }
            TestnetCommands::Docs => {
                testnet::docs();
            }
            TestnetCommands::Reset { data_dir } => {
                testnet::reset(data_dir)?;
            }
        },
        Commands::Observer { command } => match command {
            ObserverCommands::Status { lb_url, replicas } => {
                observer::status(&lb_url, &replicas).await?;
            }
            ObserverCommands::Probe { lb_url, count } => {
                observer::probe_drill(&lb_url, count).await?;
            }
        },
    }
    Ok(())
}
