// crates/node/src/config.rs

use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum NodeMode {
    /// Full mode: Store everything (500GB+, 16GB RAM)
    Full,
    /// Pruned mode: Store last 30 days (200GB, 8GB RAM)
    Pruned,
    /// Light mode: Current state only (100GB, 4-8GB RAM)
    Light,
}

impl Default for NodeMode {
    fn default() -> Self {
        NodeMode::Pruned
    }
}

#[derive(Debug, Clone)]
pub struct PruningConfig {
    /// Keep packages for X days, then archive to IPFS
    pub package_retention_days: u32,
    /// Keep full block history or just headers
    pub keep_full_blocks: bool,
    /// Prune interval (every X blocks)
    pub prune_interval: u64,
    /// Max database size before forced pruning (GB)
    pub max_db_size_gb: u32,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            package_retention_days: 30,
            keep_full_blocks: false,
            prune_interval: 1000,
            max_db_size_gb: 150,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NodeConfig {
    /// HTTP bind address for the REST API.
    pub listen_addr: String,
    /// Persistent data directory (chain + pending pool).
    pub data_dir: PathBuf,
    /// P2P listen address (Multiaddr format).
    pub p2p_listen: String,
    /// Bootstrap peers for Kademlia discovery.
    pub p2p_seeds: Vec<String>,
    /// Ethereum RPC URL for the bridge.
    pub eth_rpc_url: String,
    /// Registry contract address on Ethereum.
    pub registry_addr: String,
    /// Governance contract address used to execute privileged registry actions.
    pub governance_addr: String,
    /// Test token contract address used by wallet and faucet flows.
    pub token_addr: String,
    /// Staking contract address used by publisher and validator staking.
    pub staking_addr: String,
    /// Unique ID for this node (hex-encoded public key in production).
    pub node_id: String,
    /// This node's Ed25519 private key (hex). Used to sign validator votes.
    pub validator_privkey: Option<String>,
    /// Separate secp256k1 private key for Ethereum bridge operations.
    /// If unset, falls back to `validator_privkey` (legacy single-key mode).
    /// Setting a dedicated bridge key reduces blast radius: compromise of
    /// one key does not affect the other. (I4 improvement)
    pub bridge_privkey: Option<String>,
    /// Whether this node is a validator (votes on packages).
    pub is_validator: bool,
    /// Peer node URLs for gossip and consensus message forwarding.
    pub peers: Vec<String>,
    /// How often the block producer ticks (seconds).
    pub block_interval_secs: u64,
    /// IPFS API base URL.
    pub ipfs_url: String,
    /// PostgreSQL connection URL for the sync worker.
    pub pg_url: String,
    /// The set of active validators (JSON-encoded).
    pub validator_set: common::ValidatorSet,
    /// Node operation mode (Full/Pruned/Light)
    pub mode: NodeMode,
    /// Pruning configuration
    pub pruning: PruningConfig,
    /// Max peers for low-bandwidth environments
    pub max_peers: usize,
    /// Testnet mode: allows multiple nodes per machine.
    /// Mainnet (false) enforces a single node per data directory via PID lock.
    pub is_testnet: bool,
}

impl NodeConfig {
    pub fn from_env() -> Self {
        let mode = match env("CREG_NODE_MODE", "pruned").as_str() {
            "full" => NodeMode::Full,
            "light" => NodeMode::Light,
            _ => NodeMode::Pruned,
        };

        let pruning = PruningConfig {
            package_retention_days: env("CREG_PACKAGE_RETENTION_DAYS", "30")
                .parse()
                .unwrap_or(30),
            keep_full_blocks: env("CREG_KEEP_FULL_BLOCKS", "false") == "true",
            prune_interval: env("CREG_PRUNE_INTERVAL", "1000").parse().unwrap_or(1000),
            max_db_size_gb: env("CREG_MAX_DB_SIZE_GB", "150").parse().unwrap_or(150),
        };

        let max_peers = env("CREG_MAX_PEERS", "15").parse().unwrap_or(15);

        Self {
            listen_addr: env("CREG_LISTEN", "0.0.0.0:8080"),
            data_dir: PathBuf::from(env("CREG_DATA_DIR", "./data")),
            node_id: env("CREG_NODE_ID", &Uuid::new_v4().to_string()),
            validator_privkey: std::env::var("CREG_VALIDATOR_KEY").ok(),
            bridge_privkey: std::env::var("CREG_BRIDGE_KEY").ok(),
            is_validator: env("CREG_IS_VALIDATOR", "false") == "true",
            peers: std::env::var("CREG_PEERS")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
            p2p_listen: env("CREG_P2P_LISTEN", "/ip4/0.0.0.0/tcp/4001"),
            p2p_seeds: std::env::var("CREG_P2P_SEEDS")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
            eth_rpc_url: env("CREG_ETH_RPC", "http://127.0.0.1:8545"),
            registry_addr: env(
                "CREG_REGISTRY_ADDR",
                "0x0000000000000000000000000000000000000000",
            ),
            governance_addr: env(
                "CREG_GOVERNANCE_ADDR",
                "0x0000000000000000000000000000000000000000",
            ),
            token_addr: env(
                "CREG_TOKEN_ADDR",
                "0x0000000000000000000000000000000000000000",
            ),
            staking_addr: env(
                "CREG_STAKING_ADDR",
                "0x0000000000000000000000000000000000000000",
            ),
            block_interval_secs: env("CREG_BLOCK_INTERVAL", "5").parse().unwrap_or(5),
            ipfs_url: env("CREG_IPFS_URL", "http://127.0.0.1:5001"),
            pg_url: env("CREG_PG_URL", ""),
            validator_set: serde_json::from_str(&env("CREG_VALIDATOR_SET", "{\"validators\":[]}"))
                .unwrap_or_else(|_| common::ValidatorSet::new(vec![])),
            mode,
            pruning,
            max_peers,
            is_testnet: env("CREG_TESTNET", "false") == "true",
        }
    }

    /// Validate the configuration and return a list of human-readable errors.
    /// Call this at startup before opening any resources.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        let is_zero_like = |value: &str| {
            let trimmed = value.trim();
            trimmed.is_empty()
                || trimmed.eq_ignore_ascii_case("0x0000000000000000000000000000000000000000")
        };

        // A validator node must have a signing key.
        if self.is_validator && self.validator_privkey.is_none() {
            errors.push(
                "CREG_IS_VALIDATOR=true but CREG_VALIDATOR_KEY is not set. \
                 Generate a key with `creg keygen` and set CREG_VALIDATOR_KEY."
                    .into(),
            );
        }

        // Validate the key is proper hex if set.
        if let Some(key) = &self.validator_privkey {
            match hex::decode(key) {
                Ok(bytes) if bytes.len() == 32 => {}
                Ok(bytes) => errors.push(format!(
                    "CREG_VALIDATOR_KEY must be 32 bytes (64 hex chars), got {} bytes",
                    bytes.len()
                )),
                Err(_) => errors.push("CREG_VALIDATOR_KEY is not valid hex".into()),
            }
        }

        // Validate the dedicated bridge key if set (I4).
        if let Some(key) = &self.bridge_privkey {
            match hex::decode(key) {
                Ok(bytes) if bytes.len() == 32 => {}
                Ok(bytes) => errors.push(format!(
                    "CREG_BRIDGE_KEY must be 32 bytes (64 hex chars), got {} bytes",
                    bytes.len()
                )),
                Err(_) => errors.push("CREG_BRIDGE_KEY is not valid hex".into()),
            }
        }

        // Warn if using the null registry address (bridge will not work).
        if is_zero_like(&self.registry_addr) {
            errors.push(
                "CREG_REGISTRY_ADDR is the zero address. \
                 Deploy Registry.sol and set CREG_REGISTRY_ADDR for Ethereum bridging."
                    .into(),
            );
        }

        if self.bridge_privkey.is_some() && is_zero_like(&self.governance_addr) {
            errors.push(
                "CREG_GOVERNANCE_ADDR is the zero address. \
                 Set CREG_GOVERNANCE_ADDR so the bridge can execute rollup settlement via Governance.sol."
                    .into(),
            );
        }

        if is_zero_like(&self.token_addr) {
            errors.push(
                "CREG_TOKEN_ADDR is not set. Wallet balances, faucet wiring, and staking UI will be unavailable until testnet artifacts are synced."
                    .into(),
            );
        }

        if is_zero_like(&self.staking_addr) {
            errors.push(
                "CREG_STAKING_ADDR is not set. Validator and publisher staking flows will be unavailable until testnet artifacts are synced."
                    .into(),
            );
        }

        // Block interval sanity check.
        if self.block_interval_secs == 0 {
            errors.push("CREG_BLOCK_INTERVAL must be > 0 seconds".into());
        }

        errors
    }
}

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
