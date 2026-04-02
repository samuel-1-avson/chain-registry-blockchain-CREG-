// crates/node/src/config.rs

use std::path::PathBuf;
use uuid::Uuid;

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
    /// Unique ID for this node (hex-encoded public key in production).
    pub node_id: String,
    /// This node's Ed25519 private key (hex). Used to sign validator votes.
    pub validator_privkey: Option<String>,
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
}

impl NodeConfig {
    pub fn from_env() -> Self {
        Self {
            listen_addr: env("CREG_LISTEN", "0.0.0.0:8080"),
            data_dir: PathBuf::from(env("CREG_DATA_DIR", "./data")),
            node_id: env("CREG_NODE_ID", &Uuid::new_v4().to_string()),
            validator_privkey: std::env::var("CREG_VALIDATOR_KEY").ok(),
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
            registry_addr: env("CREG_REGISTRY_ADDR", "0x0000000000000000000000000000000000000000"),
            block_interval_secs: env("CREG_BLOCK_INTERVAL", "5").parse().unwrap_or(5),
            ipfs_url: env("CREG_IPFS_URL", "http://127.0.0.1:5001"),
            pg_url: env("CREG_PG_URL", ""),
            validator_set: serde_json::from_str(&env("CREG_VALIDATOR_SET", "{\"validators\":[]}"))
                .unwrap_or_else(|_| common::ValidatorSet::new(vec![])),
        }
    }

    /// Validate the configuration and return a list of human-readable errors.
    /// Call this at startup before opening any resources.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

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
                Err(_) => errors.push(
                    "CREG_VALIDATOR_KEY is not valid hex".into()
                ),
            }
        }

        // Warn if using the null registry address (bridge will not work).
        if self.registry_addr == "0x0000000000000000000000000000000000000000" {
            errors.push(
                "CREG_REGISTRY_ADDR is the zero address. \
                 Deploy Registry.sol and set CREG_REGISTRY_ADDR for Ethereum bridging."
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
