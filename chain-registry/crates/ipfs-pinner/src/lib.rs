//! IPFS Pinning Rewards System
//!
//! This crate provides:
//! - Automatic pinning of verified packages
//! - Verification of content availability
//! - Rewards tracking and claiming
//! - Integration with the PinningRewards.sol contract

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

pub mod contract;
pub mod pinner;
pub mod verifier;

pub use contract::{PinningContract, PinningRewardsClient};
pub use pinner::{IpfsPinner, PinnerConfig};
pub use verifier::{VerificationResult, Verifier};

/// Configuration for the pinning rewards system
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PinningConfig {
    /// IPFS node RPC endpoint
    pub ipfs_url: String,
    /// Ethereum RPC endpoint
    pub eth_rpc: String,
    /// PinningRewards contract address
    pub contract_address: String,
    /// Node operator's Ethereum private key
    pub operator_key: String,
    /// Minimum stake required (in CREG wei)
    pub min_stake: u128,
    /// Auto-register as pinner on startup
    pub auto_register: bool,
    /// Verification interval (seconds)
    pub verification_interval: u64,
    /// Max pins to track per node
    pub max_pins: usize,
}

impl Default for PinningConfig {
    fn default() -> Self {
        Self {
            ipfs_url: "http://localhost:5001".to_string(),
            eth_rpc: "http://localhost:8545".to_string(),
            contract_address: "0x0000000000000000000000000000000000000000".to_string(),
            operator_key: String::new(),
            min_stake: 1000e18 as u128, // 1000 CREG
            auto_register: false,
            verification_interval: 3600, // 1 hour
            max_pins: 10000,
        }
    }
}

/// Information about a pinned CID
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinInfo {
    /// The content identifier
    pub cid: String,
    /// Content size in bytes
    pub size: u64,
    /// When first pinned
    pub pinned_at: DateTime<Utc>,
    /// Last successful verification
    pub last_verified: Option<DateTime<Utc>>,
    /// Number of times content was served
    pub access_count: u64,
    /// Whether currently active
    pub is_active: bool,
    /// Local file path (if cached)
    pub local_path: Option<PathBuf>,
}

/// Pinner statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PinnerStats {
    /// Total CIDs pinned
    pub total_pins: usize,
    /// Total size pinned (bytes)
    pub total_size: u64,
    /// Successful verifications
    pub successful_verifications: u64,
    /// Failed verifications
    pub failed_verifications: u64,
    /// Cumulative rewards earned (CREG wei)
    pub cumulative_rewards: u128,
    /// Pending rewards (CREG wei)
    pub pending_rewards: u128,
    /// Current stake (CREG wei)
    pub current_stake: u128,
}

/// The main pinning rewards manager
pub struct PinningManager {
    config: PinningConfig,
    pinner: Arc<dyn IpfsPinner>,
    contract: Arc<dyn PinningContract>,
    verifier: Arc<dyn Verifier>,
    /// Tracked pins
    pins: Arc<RwLock<HashMap<String, PinInfo>>>,
    /// Runtime statistics
    stats: Arc<RwLock<PinnerStats>>,
}

impl PinningManager {
    /// Create a new pinning manager
    pub async fn new(
        config: PinningConfig,
        pinner: Arc<dyn IpfsPinner>,
        contract: Arc<dyn PinningContract>,
        verifier: Arc<dyn Verifier>,
    ) -> Result<Self> {
        let manager = Self {
            config,
            pinner,
            contract,
            verifier,
            pins: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(PinnerStats::default())),
        };

        // Auto-register if configured
        if manager.config.auto_register {
            manager.ensure_registered().await?;
        }

        // Load existing pins
        manager.load_existing_pins().await?;

        Ok(manager)
    }

    /// Start the background tasks
    pub async fn start(&self) -> Result<()> {
        info!("Starting IPFS pinning manager");

        // Spawn verification loop
        let pins = Arc::clone(&self.pins);
        let verifier = Arc::clone(&self.verifier);
        let contract = Arc::clone(&self.contract);
        let interval = self.config.verification_interval;

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(interval));
            loop {
                ticker.tick().await;
                if let Err(e) = Self::run_verification(&pins, &verifier, &contract).await {
                    warn!("Verification error: {}", e);
                }
            }
        });

        // Spawn rewards claiming loop
        let contract_claim = Arc::clone(&self.contract);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(86400)); // Daily
            loop {
                ticker.tick().await;
                if let Err(e) = contract_claim.claim_rewards().await {
                    warn!("Rewards claim error: {}", e);
                }
            }
        });

        info!("Pinning manager started successfully");
        Ok(())
    }

    /// Pin a new package CID
    pub async fn pin_package(&self, cid: &str, size: u64) -> Result<()> {
        debug!("Pinning package: {} ({} bytes)", cid, size);

        // Check if already pinned
        {
            let pins = self.pins.read().await;
            if pins.contains_key(cid) {
                debug!("CID {} already pinned", cid);
                return Ok(());
            }
        }

        // Pin to IPFS
        self.pinner.pin(cid).await.context("IPFS pin failed")?;

        // Register on-chain
        let cid_hash = cid_to_bytes32(cid)?;
        self.contract
            .register_pin(cid_hash, size)
            .await
            .context("Contract registration failed")?;

        // Track locally
        let pin_info = PinInfo {
            cid: cid.to_string(),
            size,
            pinned_at: Utc::now(),
            last_verified: None,
            access_count: 0,
            is_active: true,
            local_path: None,
        };

        {
            let mut pins = self.pins.write().await;
            pins.insert(cid.to_string(), pin_info);
        }

        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.total_pins += 1;
            stats.total_size += size;
        }

        info!("Successfully pinned {} ({} bytes)", cid, size);
        Ok(())
    }

    /// Unpin a package
    pub async fn unpin_package(&self, cid: &str) -> Result<()> {
        debug!("Unpinning package: {}", cid);

        // Unpin from IPFS
        self.pinner.unpin(cid).await?;

        // Unregister on-chain
        let cid_hash = cid_to_bytes32(cid)?;
        self.contract.unregister_pin(cid_hash).await?;

        // Update local tracking
        {
            let mut pins = self.pins.write().await;
            if let Some(pin) = pins.get_mut(cid) {
                pin.is_active = false;
            }
        }

        info!("Successfully unpinned {}", cid);
        Ok(())
    }

    /// Get current statistics
    pub async fn get_stats(&self) -> PinnerStats {
        self.stats.read().await.clone()
    }

    /// Get list of pinned CIDs
    pub async fn get_pins(&self) -> Vec<PinInfo> {
        let pins = self.pins.read().await;
        pins.values().cloned().collect()
    }

    /// Calculate pending rewards
    pub async fn calculate_pending_rewards(&self) -> Result<u128> {
        self.contract.calculate_rewards().await
    }

    /// Manually claim rewards
    pub async fn claim_rewards(&self) -> Result<u128> {
        self.contract.claim_rewards().await
    }

    // ============ Private Methods ============

    async fn ensure_registered(&self) -> Result<()> {
        let is_registered = self.contract.is_registered().await?;
        
        if !is_registered {
            info!("Registering as pinner with stake: {}", self.config.min_stake);
            self.contract
                .register_pinner(self.config.min_stake)
                .await
                .context("Pinner registration failed")?;
            info!("Successfully registered as pinner");
        }

        Ok(())
    }

    async fn load_existing_pins(&self) -> Result<()> {
        // Query contract for existing pins
        let cids = self.contract.get_pinner_cids().await?;
        
        for cid_hash in cids {
            let cid = bytes32_to_cid(&cid_hash)?;
            let pin_info = self.contract.get_pin_info(cid_hash).await?;
            
            let pin = PinInfo {
                cid: cid.clone(),
                size: pin_info.size,
                pinned_at: DateTime::from_timestamp(pin_info.pinned_at as i64, 0)
                    .unwrap_or_else(|| Utc::now()),
                last_verified: pin_info.last_verified.map(|t| {
                    DateTime::from_timestamp(t as i64, 0).unwrap_or_else(|| Utc::now())
                }),
                access_count: pin_info.access_count,
                is_active: pin_info.is_active,
                local_path: None,
            };

            let mut pins = self.pins.write().await;
            pins.insert(cid, pin);
        }

        info!("Loaded {} existing pins from contract", cids.len());
        Ok(())
    }

    async fn run_verification(
        pins: &Arc<RwLock<HashMap<String, PinInfo>>>,
        verifier: &Arc<dyn Verifier>,
        contract: &Arc<dyn PinningContract>,
    ) -> Result<()> {
        let pins_to_verify: Vec<String> = {
            let pins = pins.read().await;
            pins.values()
                .filter(|p| p.is_active)
                .map(|p| p.cid.clone())
                .collect()
        };

        for cid in pins_to_verify {
            match verifier.verify(&cid).await {
                Ok(result) => {
                    let cid_hash = cid_to_bytes32(&cid)?;
                    let success = result.is_available;
                    let proof_hash = result.proof_hash;

                    // Submit to contract
                    if let Err(e) = contract
                        .submit_verification(cid_hash, success, proof_hash)
                        .await
                    {
                        warn!("Failed to submit verification for {}: {}", cid, e);
                    }

                    // Update local state
                    if success {
                        let mut pins = pins.write().await;
                        if let Some(pin) = pins.get_mut(&cid) {
                            pin.last_verified = Some(Utc::now());
                        }
                    }
                }
                Err(e) => {
                    warn!("Verification failed for {}: {}", cid, e);
                }
            }
        }

        Ok(())
    }
}

/// Convert CID string to bytes32
fn cid_to_bytes32(cid: &str) -> Result<[u8; 32]> {
    // In production, this would use proper CID encoding
    // For now, we hash the CID string
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    let mut hasher = DefaultHasher::new();
    cid.hash(&mut hasher);
    let hash = hasher.finish();
    
    let mut result = [0u8; 32];
    result[0..8].copy_from_slice(&hash.to_le_bytes());
    Ok(result)
}

/// Convert bytes32 to CID string (best effort)
fn bytes32_to_cid(_bytes: &[u8; 32]) -> Result<String> {
    // In production, this would use proper CID decoding
    // For now, return a placeholder
    Ok("QmPlaceholder".to_string())
}
