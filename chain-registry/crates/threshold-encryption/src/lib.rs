//! Threshold Encryption for Private Registries
//!
//! This crate provides threshold encryption using Shamir's Secret Sharing (SSS)
//! for enterprise private registries. It allows packages to be encrypted such
//! that M-of-N validators are required to decrypt them.
//!
//! # Example
//!
//! ```rust
//! use threshold_encryption::{ThresholdEncryption, KeyShare};
//!
//! // Create encryption for 3-of-5 threshold
//! let te = ThresholdEncryption::new(3, 5).unwrap();
//!
//! // Generate shares
//! let shares = te.generate_shares(b"secret encryption key").unwrap();
//!
//! // Encrypt package
//! let encrypted = te.encrypt_package(b"package content", &shares[0]).unwrap();
//!
//! // Decrypt with 3 shares
//! let decrypted = te.decrypt_with_shares(&encrypted, &shares[..3]).unwrap();
//! ```

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{debug, info, instrument};

pub mod access_control;
pub mod distribution;
pub mod service;
pub mod shamir;

pub use access_control::AccessPolicy;
pub use distribution::{
    AccessPolicy as ShieldedAccessPolicy, DecryptionRequest, DecryptionResponse,
    ShieldedPackageMetadata,
};
pub use service::{DecryptionService, ServiceConfig};
pub use shamir::{ShamirSecretSharing, Share};

/// Errors that can occur in threshold encryption
#[derive(Error, Debug)]
pub enum ThresholdError {
    #[error("Invalid threshold: {0} of {1}")]
    InvalidThreshold(u8, u8),

    #[error("Share reconstruction failed: {0}")]
    ReconstructionFailed(String),

    #[error("Encryption error: {0}")]
    EncryptionError(String),

    #[error("Decryption error: {0}")]
    DecryptionError(String),

    #[error("Invalid share: {0}")]
    InvalidShare(String),

    #[error("Insufficient shares: got {0}, need {1}")]
    InsufficientShares(u8, u8),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// A single key share for threshold decryption
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyShare {
    /// Share index (1-based)
    pub index: u8,
    /// Share value
    pub value: Vec<u8>,
    /// Public key for this share
    pub public_key: Vec<u8>,
}

impl KeyShare {
    /// Create a new key share
    pub fn new(index: u8, value: Vec<u8>, public_key: Vec<u8>) -> Self {
        Self {
            index,
            value,
            public_key,
        }
    }

    /// Serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![self.index];
        bytes.extend_from_slice(&(self.value.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&self.value);
        bytes.extend_from_slice(&self.public_key);
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ThresholdError> {
        if bytes.len() < 5 {
            return Err(ThresholdError::InvalidShare("Too short".to_string()));
        }

        let index = bytes[0];
        let value_len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;

        if bytes.len() < 5 + value_len {
            return Err(ThresholdError::InvalidShare("Invalid length".to_string()));
        }

        let value = bytes[5..5 + value_len].to_vec();
        let public_key = bytes[5 + value_len..].to_vec();

        Ok(Self {
            index,
            value,
            public_key,
        })
    }
}

/// Encrypted package with metadata
#[derive(Debug, Clone)]
pub struct EncryptedPackage {
    /// Encrypted content (AES-256-GCM)
    pub ciphertext: Vec<u8>,
    /// Nonce for AES-GCM
    pub nonce: [u8; 12],
    /// Content hash for verification
    pub content_hash: [u8; 32],
    /// Number of shares required
    pub threshold: u8,
    /// Total number of shares
    pub total_shares: u8,
    /// Encrypted shares for each validator
    pub encrypted_shares: HashMap<u8, Vec<u8>>,
}

impl EncryptedPackage {
    /// Serialize to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ThresholdError> {
        serde_json::from_slice(bytes).map_err(|e| ThresholdError::InvalidShare(e.to_string()))
    }
}

/// Threshold encryption manager
pub struct ThresholdEncryption {
    /// Minimum shares needed (M)
    threshold: u8,
    /// Total shares (N)
    total_shares: u8,
    /// Shamir secret sharing instance
    sss: ShamirSecretSharing,
}

impl ThresholdEncryption {
    /// Create new threshold encryption instance
    ///
    /// # Arguments
    /// * `threshold` - Minimum shares needed (M)
    /// * `total_shares` - Total shares to generate (N)
    pub fn new(threshold: u8, total_shares: u8) -> Result<Self, ThresholdError> {
        if threshold == 0 || threshold > total_shares {
            return Err(ThresholdError::InvalidThreshold(threshold, total_shares));
        }
        if total_shares > 255 {
            return Err(ThresholdError::InvalidThreshold(threshold, total_shares));
        }

        info!(
            "Creating threshold encryption: {} of {}",
            threshold, total_shares
        );

        Ok(Self {
            threshold,
            total_shares,
            sss: ShamirSecretSharing::new(threshold, total_shares),
        })
    }

    /// Generate key shares from a master secret
    #[instrument(skip(self, secret), level = "debug")]
    pub fn generate_shares(&self, secret: &[u8]) -> Result<Vec<KeyShare>, ThresholdError> {
        debug!(
            "Generating {} shares with threshold {}",
            self.total_shares, self.threshold
        );

        // Generate random shares using Shamir's Secret Sharing
        let shares = self.sss.split_secret(secret)?;

        // Convert to KeyShare format
        let key_shares: Vec<KeyShare> = shares
            .into_iter()
            .map(|share| {
                let public_key = Self::derive_public_key(&share.value);
                KeyShare::new(share.index, share.value, public_key)
            })
            .collect();

        info!("Generated {} key shares", key_shares.len());
        Ok(key_shares)
    }

    /// Encrypt a package using threshold encryption
    ///
    /// # Process
    /// 1. Generate random encryption key
    /// 2. Split key into shares
    /// 3. Encrypt content with key
    /// 4. Encrypt each share with validator's public key
    #[instrument(skip(self, content, validator_keys), level = "debug")]
    pub fn encrypt_package(
        &self,
        content: &[u8],
        validator_keys: &[Vec<u8>],
    ) -> Result<EncryptedPackage, ThresholdError> {
        debug!("Encrypting package: {} bytes", content.len());

        if validator_keys.len() != self.total_shares as usize {
            return Err(ThresholdError::InvalidShare(format!(
                "Expected {} validator keys, got {}",
                self.total_shares,
                validator_keys.len()
            )));
        }

        // Generate random encryption key
        let mut encryption_key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut encryption_key);

        // Generate shares of the encryption key
        let shares = self.generate_shares(&encryption_key)?;

        // Encrypt content with AES-256-GCM
        let nonce_bytes = Self::generate_nonce();
        let cipher = Aes256Gcm::new_from_slice(&encryption_key)
            .map_err(|e| ThresholdError::EncryptionError(e.to_string()))?;

        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), content)
            .map_err(|e| ThresholdError::EncryptionError(e.to_string()))?;

        // Encrypt each share with corresponding validator's public key
        let mut encrypted_shares = HashMap::new();
        for (i, share) in shares.iter().enumerate() {
            let validator_key = &validator_keys[i];
            let encrypted_share = self.encrypt_share(share, validator_key)?;
            encrypted_shares.insert(share.index, encrypted_share);
        }

        let content_hash = Self::compute_hash(content);

        info!("Package encrypted successfully");

        Ok(EncryptedPackage {
            ciphertext,
            nonce: nonce_bytes,
            content_hash,
            threshold: self.threshold,
            total_shares: self.total_shares,
            encrypted_shares,
        })
    }

    /// Decrypt a package using shares
    ///
    /// # Arguments
    /// * `package` - The encrypted package
    /// * `shares` - At least `threshold` shares from validators
    #[instrument(skip(self, package, shares), level = "debug")]
    pub fn decrypt_with_shares(
        &self,
        package: &EncryptedPackage,
        shares: &[KeyShare],
    ) -> Result<Vec<u8>, ThresholdError> {
        debug!("Decrypting package with {} shares", shares.len());

        if shares.len() < self.threshold as usize {
            return Err(ThresholdError::InsufficientShares(
                shares.len() as u8,
                self.threshold,
            ));
        }

        // Reconstruct encryption key from shares
        let encryption_key = self.reconstruct_key(&shares[..self.threshold as usize])?;

        // Decrypt content
        let cipher = Aes256Gcm::new_from_slice(&encryption_key)
            .map_err(|e| ThresholdError::DecryptionError(e.to_string()))?;

        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&package.nonce),
                package.ciphertext.as_ref(),
            )
            .map_err(|e| ThresholdError::DecryptionError(e.to_string()))?;

        // Verify content hash
        let computed_hash = Self::compute_hash(&plaintext);
        if computed_hash != package.content_hash {
            return Err(ThresholdError::DecryptionError(
                "Content hash mismatch - possible tampering".to_string(),
            ));
        }

        info!("Package decrypted successfully, {} bytes", plaintext.len());

        Ok(plaintext)
    }

    /// Reconstruct master key from shares
    fn reconstruct_key(&self, shares: &[KeyShare]) -> Result<[u8; 32], ThresholdError> {
        let shamir_shares: Vec<Share> = shares
            .iter()
            .map(|ks| Share {
                index: ks.index,
                value: ks.value.clone(),
            })
            .collect();

        let secret = self.sss.reconstruct_secret(&shamir_shares)?;

        if secret.len() != 32 {
            return Err(ThresholdError::ReconstructionFailed(
                "Invalid key length".to_string(),
            ));
        }

        let mut key = [0u8; 32];
        key.copy_from_slice(&secret);
        Ok(key)
    }

    /// Encrypt a share with a validator's public key
    fn encrypt_share(
        &self,
        share: &KeyShare,
        validator_key: &[u8],
    ) -> Result<Vec<u8>, ThresholdError> {
        // Simplified: XOR with hash of validator key
        // In production, use proper ECIES or similar
        let key_hash = Self::compute_hash(validator_key);
        let mut encrypted = Vec::with_capacity(share.value.len());

        for (i, byte) in share.value.iter().enumerate() {
            encrypted.push(byte ^ key_hash[i % 32]);
        }

        // Prepend share index
        let mut result = vec![share.index];
        result.extend(encrypted);

        Ok(result)
    }

    /// Decrypt a share with validator's private key
    pub fn decrypt_share(
        &self,
        encrypted_share: &[u8],
        validator_private_key: &[u8],
    ) -> Result<KeyShare, ThresholdError> {
        if encrypted_share.len() < 2 {
            return Err(ThresholdError::InvalidShare("Too short".to_string()));
        }

        let index = encrypted_share[0];
        let encrypted_value = &encrypted_share[1..];

        // Decrypt
        let key_hash = Self::compute_hash(validator_private_key);
        let mut value = Vec::with_capacity(encrypted_value.len());

        for (i, byte) in encrypted_value.iter().enumerate() {
            value.push(byte ^ key_hash[i % 32]);
        }

        let public_key = Self::derive_public_key(&value);

        Ok(KeyShare::new(index, value, public_key))
    }

    /// Generate random nonce for AES-GCM
    fn generate_nonce() -> [u8; 12] {
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        nonce
    }

    /// Derive public key from share (simplified)
    fn derive_public_key(share_value: &[u8]) -> Vec<u8> {
        let hash = Self::compute_hash(share_value);
        hash.to_vec()
    }

    /// Compute SHA-256 hash
    fn compute_hash(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    /// Get threshold
    pub fn threshold(&self) -> u8 {
        self.threshold
    }

    /// Get total shares
    pub fn total_shares(&self) -> u8 {
        self.total_shares
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_threshold_encryption_lifecycle() {
        // Create 3-of-5 threshold encryption
        let te = ThresholdEncryption::new(3, 5).unwrap();

        // Generate validator keys
        let validator_keys: Vec<Vec<u8>> = (0..5).map(|i| vec![i as u8; 32]).collect();

        // Encrypt package
        let content = b"Secret package content";
        let encrypted = te.encrypt_package(content, &validator_keys).unwrap();

        // Verify encrypted package structure
        assert_eq!(encrypted.threshold, 3);
        assert_eq!(encrypted.total_shares, 5);
        assert_eq!(encrypted.encrypted_shares.len(), 5);

        // Decrypt shares (simulate validators)
        let mut shares = Vec::new();
        for (i, (_, encrypted_share)) in encrypted.encrypted_shares.iter().enumerate() {
            let share = te
                .decrypt_share(encrypted_share, &validator_keys[i])
                .unwrap();
            shares.push(share);
        }

        // Decrypt with 3 shares
        let decrypted = te.decrypt_with_shares(&encrypted, &shares[..3]).unwrap();
        assert_eq!(decrypted, content);
    }

    #[test]
    fn test_insufficient_shares() {
        let te = ThresholdEncryption::new(3, 5).unwrap();

        let validator_keys: Vec<Vec<u8>> = (0..5).map(|i| vec![i as u8; 32]).collect();

        let content = b"test";
        let encrypted = te.encrypt_package(content, &validator_keys).unwrap();

        // Generate some shares
        let mut shares = Vec::new();
        for (i, (_, encrypted_share)) in encrypted.encrypted_shares.iter().enumerate().take(2) {
            let share = te
                .decrypt_share(encrypted_share, &validator_keys[i])
                .unwrap();
            shares.push(share);
        }

        // Try to decrypt with only 2 shares (need 3)
        let result = te.decrypt_with_shares(&encrypted, &shares);
        assert!(matches!(
            result,
            Err(ThresholdError::InsufficientShares(2, 3))
        ));
    }

    #[test]
    fn test_key_share_serialization() {
        let share = KeyShare::new(1, vec![1, 2, 3, 4], vec![5, 6, 7, 8]);

        let bytes = share.to_bytes();
        let deserialized = KeyShare::from_bytes(&bytes).unwrap();

        assert_eq!(share.index, deserialized.index);
        assert_eq!(share.value, deserialized.value);
    }

    #[test]
    fn test_invalid_threshold() {
        // Threshold 0
        assert!(ThresholdEncryption::new(0, 5).is_err());

        // Threshold > total
        assert!(ThresholdEncryption::new(6, 5).is_err());
    }
}
