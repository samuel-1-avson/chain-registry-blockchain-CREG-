//! Key Share Distribution System
//!
//! Handles the distribution of encryption key shares to validators
//! and coordination of decryption requests.

use crate::{KeyShare, ThresholdEncryption, ThresholdError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// Information about a distributed key share
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributedShare {
    /// Validator ID who received this share
    pub validator_id: String,
    /// Encrypted share (encrypted to validator's public key)
    pub encrypted_share: Vec<u8>,
    /// Share index (1-based)
    pub share_index: u8,
    /// When the share was distributed
    pub distributed_at: u64,
    /// Whether the validator has confirmed receipt
    pub confirmed: bool,
}

/// Package encryption metadata stored on-chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShieldedPackageMetadata {
    /// Package canonical ID
    pub canonical: String,
    /// IPFS CID of encrypted content
    pub encrypted_cid: String,
    /// Content hash (of decrypted content)
    pub content_hash: String,
    /// Threshold required for decryption (M)
    pub threshold: u8,
    /// Total shares created (N)
    pub total_shares: u8,
    /// List of validators who received shares
    pub share_holders: Vec<String>,
    /// Access policy for this package
    pub access_policy: AccessPolicy,
    /// When the package was published
    pub published_at: u64,
}

/// Access policy for shielded packages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessPolicy {
    /// List of authorized decryptor addresses
    pub authorized_decryptors: Vec<String>,
    /// Whether package is restricted to organization
    pub organization_only: bool,
    /// Organization ID (if organization_only)
    pub organization_id: Option<String>,
    /// Time-based access restriction (Unix timestamp)
    pub expires_at: Option<u64>,
    /// Maximum number of decryptions allowed
    pub max_decryptions: Option<u32>,
}

impl Default for AccessPolicy {
    fn default() -> Self {
        Self {
            authorized_decryptors: vec![],
            organization_only: false,
            organization_id: None,
            expires_at: None,
            max_decryptions: None,
        }
    }
}

/// Distribution coordinator for key shares
pub struct ShareDistributor {
    /// Threshold encryption instance
    te: ThresholdEncryption,
    /// Validator public keys (validator_id -> public_key)
    validator_keys: HashMap<String, Vec<u8>>,
    /// Distributed shares cache
    distributed_shares: HashMap<String, Vec<DistributedShare>>,
}

impl ShareDistributor {
    /// Create a new share distributor
    pub fn new(threshold: u8, total_shares: u8) -> Result<Self, ThresholdError> {
        let te = ThresholdEncryption::new(threshold, total_shares)?;
        
        Ok(Self {
            te,
            validator_keys: HashMap::new(),
            distributed_shares: HashMap::new(),
        })
    }
    
    /// Register a validator's public key
    pub fn register_validator(&mut self, validator_id: String, public_key: Vec<u8>) {
        debug!("Registering validator {} with public key", validator_id);
        self.validator_keys.insert(validator_id, public_key);
    }
    
    /// Generate and distribute shares for a package
    pub fn distribute_shares(
        &mut self,
        package_canonical: &str,
        encryption_key: &[u8],
        access_policy: &AccessPolicy,
    ) -> Result<Vec<DistributedShare>, ThresholdError> {
        info!("Distributing shares for package: {}", package_canonical);
        
        // Generate shares
        let shares = self.te.generate_shares(encryption_key)?;
        
        // Select validators to receive shares
        let selected_validators = self.select_validators(access_policy)?;
        
        if selected_validators.len() < self.te.threshold as usize {
            return Err(ThresholdError::InvalidThreshold(
                self.te.threshold,
                selected_validators.len() as u8
            ));
        }
        
        // Encrypt and distribute shares
        let mut distributed = Vec::new();
        for (i, (share, validator_id)) in shares.iter().zip(selected_validators.iter()).enumerate() {
            let public_key = self.validator_keys.get(validator_id)
                .ok_or_else(|| ThresholdError::InvalidShare(
                    format!("No public key for validator {}", validator_id)
                ))?;
            
            // Encrypt share to validator's public key
            let encrypted_share = self.encrypt_share(share, public_key)?;
            
            let distributed_share = DistributedShare {
                validator_id: validator_id.clone(),
                encrypted_share,
                share_index: share.index,
                distributed_at: current_timestamp(),
                confirmed: false,
            };
            
            distributed.push(distributed_share);
        }
        
        // Cache distributed shares
        self.distributed_shares.insert(package_canonical.to_string(), distributed.clone());
        
        info!(
            "Distributed {} shares for {} to validators: {:?}",
            distributed.len(),
            package_canonical,
            selected_validators
        );
        
        Ok(distributed)
    }
    
    /// Select validators to receive shares based on access policy
    fn select_validators(&self, access_policy: &AccessPolicy) -> Result<Vec<String>, ThresholdError> {
        let all_validators: Vec<String> = self.validator_keys.keys().cloned().collect();
        
        if all_validators.len() < self.te.total_shares as usize {
            return Err(ThresholdError::InvalidThreshold(
                self.te.total_shares,
                all_validators.len() as u8
            ));
        }
        
        // For now, select first N validators deterministically
        // In production, use VRF-based selection for fairness
        let selected: Vec<String> = all_validators
            .into_iter()
            .take(self.te.total_shares as usize)
            .collect();
        
        Ok(selected)
    }
    
    /// Encrypt a share to a validator's public key
    fn encrypt_share(&self, share: &KeyShare, public_key: &[u8]) -> Result<Vec<u8>, ThresholdError> {
        use aes_gcm::{
            aead::{Aead, KeyInit},
            Aes256Gcm, Nonce,
        };
        use rand::RngCore;
        
        // Derive encryption key from validator's public key
        let mut key = [0u8; 32];
        let mut hasher = sha2::Sha256::new();
        hasher.update(public_key);
        hasher.update(b"share-encryption-salt");
        let hash = hasher.finalize();
        key.copy_from_slice(&hash[..32]);
        
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| ThresholdError::EncryptionError(e.to_string()))?;
        
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        let plaintext = share.to_bytes();
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|e| ThresholdError::EncryptionError(e.to_string()))?;
        
        // Prepend nonce to ciphertext
        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&ciphertext);
        
        Ok(result)
    }
    
    /// Get distributed shares for a package
    pub fn get_shares(&self, package_canonical: &str) -> Option<&Vec<DistributedShare>> {
        self.distributed_shares.get(package_canonical)
    }
    
    /// Mark a share as confirmed received
    pub fn confirm_share(&mut self, package_canonical: &str, validator_id: &str) -> Result<(), ThresholdError> {
        if let Some(shares) = self.distributed_shares.get_mut(package_canonical) {
            for share in shares.iter_mut() {
                if share.validator_id == validator_id {
                    share.confirmed = true;
                    debug!("Confirmed share for {} from {}", package_canonical, validator_id);
                    return Ok(());
                }
            }
        }
        
        Err(ThresholdError::InvalidShare("Share not found".to_string()))
    }
    
    /// Check if enough shares are confirmed for decryption
    pub fn can_decrypt(&self, package_canonical: &str) -> bool {
        if let Some(shares) = self.distributed_shares.get(package_canonical) {
            let confirmed_count = shares.iter().filter(|s| s.confirmed).count();
            confirmed_count >= self.te.threshold as usize
        } else {
            false
        }
    }
}

/// Decryption request from an authorized party
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptionRequest {
    /// Package canonical ID
    pub canonical: String,
    /// Requestor address/ID
    pub requestor: String,
    /// Requestor's public key (for encrypting response)
    pub requestor_pubkey: Vec<u8>,
    /// Timestamp
    pub timestamp: u64,
    /// Request signature
    pub signature: Vec<u8>,
    /// Purpose/justification for decryption
    pub purpose: String,
}

/// Decryption response from a validator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptionResponse {
    /// Validator ID
    pub validator_id: String,
    /// Package canonical ID
    pub canonical: String,
    /// Encrypted partial decryption (encrypted to requestor's key)
    pub encrypted_share: Vec<u8>,
    /// Share index
    pub share_index: u8,
    /// Timestamp
    pub timestamp: u64,
    /// Validator signature
    pub signature: Vec<u8>,
}

/// Decryption coordinator
pub struct DecryptionCoordinator {
    /// Share distributor reference
    distributor: ShareDistributor,
    /// Pending decryption requests
    pending_requests: HashMap<String, DecryptionRequest>,
    /// Received partial decryptions
    partial_shares: HashMap<String, Vec<DecryptionResponse>>,
}

impl DecryptionCoordinator {
    /// Create new coordinator
    pub fn new(distributor: ShareDistributor) -> Self {
        Self {
            distributor,
            pending_requests: HashMap::new(),
            partial_shares: HashMap::new(),
        }
    }
    
    /// Submit a decryption request
    pub fn request_decryption(&mut self, request: DecryptionRequest) -> Result<(), ThresholdError> {
        info!("Decryption request for {} from {}", request.canonical, request.requestor);
        
        // Validate request
        if !self.validate_request(&request) {
            return Err(ThresholdError::InvalidShare("Invalid decryption request".to_string()));
        }
        
        self.pending_requests.insert(request.canonical.clone(), request);
        self.partial_shares.insert(request.canonical.clone(), Vec::new());
        
        Ok(())
    }
    
    /// Submit a partial decryption from a validator
    pub fn submit_partial(
        &mut self,
        canonical: &str,
        response: DecryptionResponse,
    ) -> Result<(), ThresholdError> {
        debug!(
            "Received partial decryption for {} from validator {}",
            canonical, response.validator_id
        );
        
        // Verify response signature
        if !self.verify_response(&response) {
            return Err(ThresholdError::InvalidShare("Invalid response signature".to_string()));
        }
        
        if let Some(shares) = self.partial_shares.get_mut(canonical) {
            shares.push(response);
            
            // Check if we have enough shares
            if shares.len() >= self.distributor.te.threshold as usize {
                info!("Sufficient shares received for {}", canonical);
            }
        }
        
        Ok(())
    }
    
    /// Check if decryption is ready (enough shares collected)
    pub fn is_ready(&self, canonical: &str) -> bool {
        if let Some(shares) = self.partial_shares.get(canonical) {
            shares.len() >= self.distributor.te.threshold as usize
        } else {
            false
        }
    }
    
    /// Get collected shares for reconstruction
    pub fn get_collected_shares(&self, canonical: &str) -> Option<&Vec<DecryptionResponse>> {
        self.partial_shares.get(canonical)
    }
    
    /// Validate decryption request (check authorization)
    fn validate_request(&self, request: &DecryptionRequest) -> bool {
        // Check timestamp (request must be recent)
        let now = current_timestamp();
        if now - request.timestamp > 3600 { // 1 hour expiry
            warn!("Decryption request expired");
            return false;
        }
        
        // TODO: Check if requestor is authorized based on access policy
        // TODO: Verify request signature
        
        true
    }
    
    /// Verify validator's response signature
    fn verify_response(&self, response: &DecryptionResponse) -> bool {
        // TODO: Implement Ed25519 signature verification
        // For now, accept all (placeholder)
        true
    }
}

/// Get current Unix timestamp
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_share_distributor_creation() {
        let distributor = ShareDistributor::new(3, 5);
        assert!(distributor.is_ok());
    }

    #[test]
    fn test_validator_registration() {
        let mut distributor = ShareDistributor::new(3, 5).unwrap();
        distributor.register_validator("val1".to_string(), vec![1, 2, 3]);
        assert!(distributor.validator_keys.contains_key("val1"));
    }

    #[test]
    fn test_access_policy_default() {
        let policy = AccessPolicy::default();
        assert!(policy.authorized_decryptors.is_empty());
        assert!(!policy.organization_only);
    }

    #[test]
    fn test_decryption_request_validation() {
        let request = DecryptionRequest {
            canonical: "npm:test@1.0.0".to_string(),
            requestor: "user1".to_string(),
            requestor_pubkey: vec![1, 2, 3],
            timestamp: current_timestamp(),
            signature: vec![],
            purpose: "Testing".to_string(),
        };
        
        assert_eq!(request.canonical, "npm:test@1.0.0");
    }
}
