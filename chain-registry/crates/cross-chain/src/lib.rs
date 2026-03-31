//! Cross-Chain Package Verification
//!
//! This crate provides multi-chain support for Chain Registry,
//! enabling package verification to be shared across multiple L1/L2 chains.
//!
//! # Features
//!
//! - Multi-chain registry client
//! - Bridge message encoding/decoding
//! - Cross-chain transaction monitoring
//! - Chain selection and fallback
//!
//! # Example
//!
//! ```rust,no_run
//! use cross_chain::{MultiChainClient, ChainConfig};
//!
//! let client = MultiChainClient::new(vec![
//!     ChainConfig::arbitrum(),
//!     ChainConfig::optimism(),
//! ]);
//!
//! // Sync package verification across chains
//! client.sync_verification("npm:package@1.0.0").await?;
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Chain configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    /// Chain name
    pub name: String,
    /// Chain ID
    pub chain_id: u64,
    /// LayerZero chain ID
    pub layerzero_id: u16,
    /// RPC URLs
    pub rpc_urls: Vec<String>,
    /// Explorer URL
    pub explorer: String,
    /// Contract addresses
    pub contracts: ContractAddresses,
}

/// Contract addresses for a chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractAddresses {
    /// Registry contract
    pub registry: String,
    /// Cross-chain registry
    pub cross_chain: String,
    /// ZK verifier
    pub zk_verifier: Option<String>,
}

/// Multi-chain client
pub struct MultiChainClient {
    chains: HashMap<String, ChainConfig>,
}

impl MultiChainClient {
    /// Create new multi-chain client
    pub fn new(configs: Vec<ChainConfig>) -> Self {
        let chains = configs
            .into_iter()
            .map(|c| (c.name.clone(), c))
            .collect();
        
        Self { chains }
    }
    
    /// Get chain config by name
    pub fn get_chain(&self, name: &str) -> Option<&ChainConfig> {
        self.chains.get(name)
    }
    
    /// List all supported chains
    pub fn list_chains(&self) -> Vec<&String> {
        self.chains.keys().collect()
    }
    
    /// Arbitrum configuration
    pub fn arbitrum() -> ChainConfig {
        ChainConfig {
            name: "arbitrum".to_string(),
            chain_id: 42161,
            layerzero_id: 110,
            rpc_urls: vec![
                "https://arb1.arbitrum.io/rpc".to_string(),
            ],
            explorer: "https://arbiscan.io".to_string(),
            contracts: ContractAddresses {
                registry: "".to_string(),
                cross_chain: "".to_string(),
                zk_verifier: None,
            },
        }
    }
    
    /// Optimism configuration
    pub fn optimism() -> ChainConfig {
        ChainConfig {
            name: "optimism".to_string(),
            chain_id: 10,
            layerzero_id: 111,
            rpc_urls: vec![
                "https://mainnet.optimism.io".to_string(),
            ],
            explorer: "https://optimistic.etherscan.io".to_string(),
            contracts: ContractAddresses {
                registry: "".to_string(),
                cross_chain: "".to_string(),
                zk_verifier: None,
            },
        }
    }
    
    /// Polygon configuration
    pub fn polygon() -> ChainConfig {
        ChainConfig {
            name: "polygon".to_string(),
            chain_id: 137,
            layerzero_id: 109,
            rpc_urls: vec![
                "https://polygon-rpc.com".to_string(),
            ],
            explorer: "https://polygonscan.com".to_string(),
            contracts: ContractAddresses {
                registry: "".to_string(),
                cross_chain: "".to_string(),
                zk_verifier: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_config() {
        let arbitrum = MultiChainClient::arbitrum();
        assert_eq!(arbitrum.chain_id, 42161);
        assert_eq!(arbitrum.layerzero_id, 110);
    }

    #[test]
    fn test_multi_chain_client() {
        let client = MultiChainClient::new(vec![
            MultiChainClient::arbitrum(),
            MultiChainClient::optimism(),
        ]);
        
        assert_eq!(client.list_chains().len(), 2);
        assert!(client.get_chain("arbitrum").is_some());
        assert!(client.get_chain("optimism").is_some());
    }
}
