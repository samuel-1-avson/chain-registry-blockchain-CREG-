//! Zero-Knowledge Proof Validation for Chain Registry
//!
//! This crate provides ZK-SNARK based validation for packages, allowing
//! validators to verify package safety without re-executing the sandbox.
//!
//! # Architecture
//!
//! 1. **Circuit Definition**: Defines the R1CS constraints for package validation
//! 2. **Proof Generation**: Creates ZK proofs locally (publisher side)
//! 3. **Proof Verification**: Fast verification of proofs (validator side)
//!
//! # Example
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use zk_validator::{PackageInputs, ZkValidator};
//!
//! let validator = ZkValidator::new()?;
//! let inputs = PackageInputs::new([1u8; 32], [2u8; 32], 95, true);
//! let proof = validator.generate_proof(&inputs)?;
//! let is_valid = validator.verify_proof(&proof, &inputs.public_inputs())?;
//! assert!(is_valid);
//! # Ok(())
//! # }
//! ```

use ark_bn254::{Bn254, Fr};
use ark_ff::PrimeField;
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_relations::r1cs::SynthesisError;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_snark::SNARK;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, info, instrument, warn};

pub mod batch;
pub mod circuits;
pub mod constraints;
pub mod slashing;

pub use batch::{BatchInputs, BatchStateTransitionValidator};
pub use circuits::{BatchStateTransitionCircuit, PackageValidationCircuit};
pub use slashing::*;

/// Errors that can occur during ZK validation
#[derive(Error, Debug)]
pub enum ZkError {
    #[error("Circuit synthesis error: {0}")]
    CircuitError(#[from] SynthesisError),

    #[error("Proof generation failed: {0}")]
    ProofGenerationError(String),

    #[error("Proof verification failed: {0}")]
    VerificationError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),
}

/// Type aliases for Bn254 curve
pub type ZkProof = Proof<Bn254>;
pub type ZkProvingKey = ProvingKey<Bn254>;
pub type ZkVerifyingKey = VerifyingKey<Bn254>;

/// Split a 32-byte hash into BN254 field limbs (low/high 128-bit halves).
pub fn hash32_to_fr_limbs(hash: &[u8; 32]) -> (Fr, Fr) {
    let mut lo = [0u8; 16];
    let mut hi = [0u8; 16];
    lo.copy_from_slice(&hash[..16]);
    hi.copy_from_slice(&hash[16..]);
    (
        Fr::from_le_bytes_mod_order(&lo),
        Fr::from_le_bytes_mod_order(&hi),
    )
}

/// Input data for package validation
#[derive(Clone, Debug)]
pub struct PackageInputs {
    /// SHA256 hash of the package tarball
    pub content_hash: [u8; 32],
    /// Hash of the package manifest
    pub manifest_hash: [u8; 32],
    /// Result of static analysis (0-100 safety score)
    pub static_analysis_score: u8,
    /// Sandbox execution result (true = safe)
    pub sandbox_safe: bool,
    /// Whether the package uses known vulnerable dependencies
    pub no_vulnerable_deps: bool,
    /// Code complexity score
    pub complexity_score: u8,
}

impl PackageInputs {
    /// Create new package inputs
    pub fn new(
        content_hash: [u8; 32],
        manifest_hash: [u8; 32],
        static_analysis_score: u8,
        sandbox_safe: bool,
    ) -> Self {
        Self {
            content_hash,
            manifest_hash,
            static_analysis_score,
            sandbox_safe,
            no_vulnerable_deps: true,
            complexity_score: 50,
        }
    }

    /// Public inputs for Groth16 verification.
    ///
    /// Must match the order of `new_input` allocations in
    /// [`PackageValidationCircuit::generate_constraints`].
    pub fn public_inputs(&self) -> Vec<Fr> {
        let (content_lo, content_hi) = hash32_to_fr_limbs(&self.content_hash);
        let (manifest_lo, manifest_hi) = hash32_to_fr_limbs(&self.manifest_hash);
        vec![
            content_lo,
            content_hi,
            manifest_lo,
            manifest_hi,
            Fr::from(self.static_analysis_score as u64),
            Fr::from(if self.sandbox_safe { 1u64 } else { 0 }),
            Fr::from(if self.no_vulnerable_deps { 1u64 } else { 0 }),
        ]
    }

    /// Convert to field elements for circuit
    pub fn to_field_elements(&self) -> Vec<Fr> {
        let mut inputs = self.public_inputs();
        // Add private inputs
        inputs.push(Fr::from(self.complexity_score as u64));
        inputs
    }
}

/// ZK Validator for package verification
pub struct ZkValidator {
    proving_key: Arc<ZkProvingKey>,
    verifying_key: Arc<ZkVerifyingKey>,
}

/// Returns true when `CREG_PRODUCTION` is `1` or `true` (case-insensitive).
pub fn production_guard_active() -> bool {
    std::env::var("CREG_PRODUCTION")
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false)
}

impl ZkValidator {
    /// Default file names for trusted setup keys (bump suffix when public inputs change).
    pub const PROVING_KEY_FILE: &'static str = "proving_key_package_v2.bin";
    pub const VERIFYING_KEY_FILE: &'static str = "verifying_key_package_v2.bin";

    /// Initialize the ZK validator.
    ///
    /// Attempts to load keys from the `circuits/` directory (or `CREG_ZK_KEYS_DIR`).
    /// If key files are not found, generates fresh keys and saves them for subsequent
    /// runs. A warning is emitted because the generated keys are NOT from a trusted
    /// ceremony — they are only suitable for development/testing.
    ///
    /// Production (`CREG_PRODUCTION=true`) never generates ephemeral keys: missing or
    /// incompatible files return [`ZkError`] instead of panicking.
    pub fn new() -> Result<Self, ZkError> {
        Self::from_keys_dir(Self::keys_dir(), production_guard_active())
    }

    /// Resolve the key directory from `CREG_ZK_KEYS_DIR` or `<crate>/circuits`.
    pub fn keys_dir() -> PathBuf {
        std::env::var("CREG_ZK_KEYS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("circuits"))
    }

    /// True when both package-circuit key files exist in `dir`.
    pub fn package_key_files_present(dir: &Path) -> bool {
        dir.join(Self::PROVING_KEY_FILE).is_file() && dir.join(Self::VERIFYING_KEY_FILE).is_file()
    }

    /// Load or generate keys from an explicit directory. Used by tests so
    /// production-guard coverage does not mutate process-wide env.
    pub fn from_keys_dir(keys_dir: PathBuf, production: bool) -> Result<Self, ZkError> {
        let pk_path = keys_dir.join(Self::PROVING_KEY_FILE);
        let vk_path = keys_dir.join(Self::VERIFYING_KEY_FILE);

        if pk_path.exists() && vk_path.exists() {
            info!("Loading ZK keys from {}", keys_dir.display());
            let validator = Self::from_key_files(&pk_path, &vk_path)?;
            if validator.smoke_test_keys().is_ok() {
                return Ok(validator);
            }
            if production {
                return Err(ZkError::InvalidInput(format!(
                    "PRODUCTION GUARD: ZK key files in '{}' failed smoke verification. \
                     Refusing to generate ephemeral keys on a production node. \
                     Replace {} and {} (CREG_ZK_KEYS_DIR) or keep zk_validation disabled \
                     in the signed chain spec.",
                    keys_dir.display(),
                    Self::PROVING_KEY_FILE,
                    Self::VERIFYING_KEY_FILE,
                )));
            }
            warn!(
                "Stale or incompatible ZK keys in {} — regenerating for the current circuit",
                keys_dir.display()
            );
        }

        // Production guard: refuse to proceed with ephemeral keys when the
        // CREG_PRODUCTION env var is set. A missing key directory in production
        // indicates a deployment error; silently falling back to ephemeral keys
        // would invalidate all existing ZK proofs and open the door to proof
        // forgery (since the new ephemeral key is unknown to the network).
        if production {
            return Err(ZkError::InvalidInput(format!(
                "PRODUCTION GUARD: ZK trusted setup key files not found in '{}'. \
                 Refusing to generate ephemeral keys on a production node. \
                 Run `creg advanced zk-setup` to generate certified keys, or \
                 set CREG_ZK_KEYS_DIR to the correct key directory. \
                 Do not enable zk_validation without those files.",
                keys_dir.display()
            )));
        }

        warn!(
            "ZK trusted setup key files not found in {} — generating ephemeral keys. \
             These keys are NOT from a trusted ceremony and must not be used in production. \
             Set CREG_PRODUCTION=true to make this a hard error. \
             Run `creg advanced zk-setup` to generate and persist proper keys.",
            keys_dir.display()
        );

        let circuit = PackageValidationCircuit::default();
        let mut rng = rand::thread_rng();
        let (proving_key, verifying_key) =
            Groth16::<Bn254>::circuit_specific_setup(circuit, &mut rng)
                .map_err(|e| ZkError::ProofGenerationError(e.to_string()))?;

        let validator = Self {
            proving_key: Arc::new(proving_key),
            verifying_key: Arc::new(verifying_key),
        };

        // Best-effort save so the next restart reuses the same keys.
        if let Err(e) = validator.save_keys(&keys_dir) {
            warn!("Could not persist generated ZK keys: {}", e);
        }

        info!("ZK validator initialized with generated keys");
        Ok(validator)
    }

    /// Startup-only message when production mode would otherwise mint ephemeral
    /// package-circuit keys. Empty when ZK is disabled or key files are present.
    pub fn production_missing_package_keys_error(
        zk_validation_enabled: bool,
        production: bool,
        keys_dir: &Path,
    ) -> Option<String> {
        if !zk_validation_enabled || !production {
            return None;
        }
        if Self::package_key_files_present(keys_dir) {
            return None;
        }
        Some(format!(
            "CREG_PRODUCTION=true but ZK package key files are missing in '{}'. \
             Place {} and {} there (CREG_ZK_KEYS_DIR), or keep zk_validation false \
             in the signed chain spec. Ephemeral keys are never generated in production.",
            keys_dir.display(),
            Self::PROVING_KEY_FILE,
            Self::VERIFYING_KEY_FILE,
        ))
    }

    /// Prove + verify once to ensure on-disk keys match the current circuit layout.
    fn smoke_test_keys(&self) -> Result<(), ZkError> {
        let inputs = PackageInputs::new([0u8; 32], [0u8; 32], 95, true);
        let proof = self.generate_proof(&inputs)?;
        let ok = self.verify_proof(&proof, &inputs.public_inputs())?;
        if ok {
            Ok(())
        } else {
            Err(ZkError::VerificationError(
                "loaded ZK keys failed smoke verification".into(),
            ))
        }
    }

    /// Load validator from key files on disk.
    pub fn from_key_files(
        proving_key_path: &Path,
        verifying_key_path: &Path,
    ) -> Result<Self, ZkError> {
        let pk_bytes = std::fs::read(proving_key_path)
            .map_err(|e| ZkError::SerializationError(format!("Read proving key: {}", e)))?;
        let vk_bytes = std::fs::read(verifying_key_path)
            .map_err(|e| ZkError::SerializationError(format!("Read verifying key: {}", e)))?;

        Self::from_keys(&pk_bytes, &vk_bytes)
    }

    /// Save current keys to disk.
    pub fn save_keys(&self, dir: &Path) -> Result<(), ZkError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| ZkError::SerializationError(format!("Create key dir: {}", e)))?;

        let mut pk_bytes = Vec::new();
        self.proving_key
            .serialize_uncompressed(&mut pk_bytes)
            .map_err(|e| ZkError::SerializationError(e.to_string()))?;

        let mut vk_bytes = Vec::new();
        self.verifying_key
            .serialize_uncompressed(&mut vk_bytes)
            .map_err(|e| ZkError::SerializationError(e.to_string()))?;

        std::fs::write(dir.join(Self::PROVING_KEY_FILE), &pk_bytes)
            .map_err(|e| ZkError::SerializationError(format!("Write proving key: {}", e)))?;
        std::fs::write(dir.join(Self::VERIFYING_KEY_FILE), &vk_bytes)
            .map_err(|e| ZkError::SerializationError(format!("Write verifying key: {}", e)))?;

        info!("ZK keys saved to {}", dir.display());
        Ok(())
    }

    /// Load validator from existing keys
    pub fn from_keys(
        proving_key_bytes: &[u8],
        verifying_key_bytes: &[u8],
    ) -> Result<Self, ZkError> {
        let proving_key = ZkProvingKey::deserialize_uncompressed(proving_key_bytes)
            .map_err(|e| ZkError::SerializationError(e.to_string()))?;

        let verifying_key = ZkVerifyingKey::deserialize_uncompressed(verifying_key_bytes)
            .map_err(|e| ZkError::SerializationError(e.to_string()))?;

        Ok(Self {
            proving_key: Arc::new(proving_key),
            verifying_key: Arc::new(verifying_key),
        })
    }

    /// Generate a ZK proof for package validation
    ///
    /// This is run by the publisher to prove their package is safe
    #[instrument(skip(self, inputs), level = "debug")]
    pub fn generate_proof(&self, inputs: &PackageInputs) -> Result<ZkProof, ZkError> {
        debug!("Generating ZK proof for package validation");

        let circuit = PackageValidationCircuit::from_inputs(inputs)?;

        let mut rng = rand::thread_rng();
        let proof = Groth16::<Bn254>::prove(&self.proving_key, circuit, &mut rng)
            .map_err(|e| ZkError::ProofGenerationError(e.to_string()))?;

        info!("ZK proof generated successfully");
        Ok(proof)
    }

    /// Verify a ZK proof
    ///
    /// This is run by validators to quickly verify package safety
    #[instrument(skip(self, proof), level = "debug")]
    pub fn verify_proof(&self, proof: &ZkProof, public_inputs: &[Fr]) -> Result<bool, ZkError> {
        debug!("Verifying ZK proof");

        let is_valid = Groth16::<Bn254>::verify(&self.verifying_key, public_inputs, proof)
            .map_err(|e| ZkError::VerificationError(e.to_string()))?;

        if is_valid {
            debug!("ZK proof verified successfully");
        } else {
            debug!("ZK proof verification failed");
        }

        Ok(is_valid)
    }

    /// Serialize a proof to bytes
    pub fn serialize_proof(proof: &ZkProof) -> Result<Vec<u8>, ZkError> {
        let mut bytes = Vec::new();
        proof
            .serialize_uncompressed(&mut bytes)
            .map_err(|e| ZkError::SerializationError(e.to_string()))?;
        Ok(bytes)
    }

    /// Deserialize a proof from bytes
    pub fn deserialize_proof(bytes: &[u8]) -> Result<ZkProof, ZkError> {
        ZkProof::deserialize_uncompressed(bytes)
            .map_err(|e| ZkError::SerializationError(e.to_string()))
    }

    /// Serialize verifying key to bytes
    pub fn serialize_vk(&self) -> Result<Vec<u8>, ZkError> {
        let mut bytes = Vec::new();
        self.verifying_key
            .serialize_uncompressed(&mut bytes)
            .map_err(|e| ZkError::SerializationError(e.to_string()))?;
        Ok(bytes)
    }

    /// Batch verify multiple proofs (optimized)
    ///
    /// This is significantly faster than individual verification
    pub fn batch_verify(
        &self,
        proofs: &[ZkProof],
        public_inputs: &[Vec<Fr>],
    ) -> Result<Vec<bool>, ZkError> {
        debug!("Batch verifying {} proofs", proofs.len());

        let results: Result<Vec<_>, _> = proofs
            .iter()
            .zip(public_inputs.iter())
            .map(|(proof, inputs)| self.verify_proof(proof, inputs))
            .collect();

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_public_inputs_bind_hashes() {
        let inputs_a = PackageInputs::new([1u8; 32], [2u8; 32], 95, true);
        let mut inputs_b = inputs_a.clone();
        inputs_b.content_hash[0] = 9;

        assert_eq!(inputs_a.public_inputs().len(), 7);
        assert_ne!(inputs_a.public_inputs(), inputs_b.public_inputs());
    }

    #[test]
    fn test_zk_proof_lifecycle() {
        // Setup
        let validator = ZkValidator::new().unwrap();

        // Create test inputs
        let inputs = PackageInputs::new(
            [1u8; 32], // content hash
            [2u8; 32], // manifest hash
            95,        // high safety score
            true,      // sandbox passed
        );

        // Generate proof
        let proof = validator.generate_proof(&inputs).unwrap();

        // Verify proof
        let public_inputs = inputs.public_inputs();
        let is_valid = validator.verify_proof(&proof, &public_inputs).unwrap();

        assert!(is_valid, "Proof should be valid");
    }

    #[test]
    fn test_serialization() {
        let validator = ZkValidator::new().unwrap();
        let inputs = PackageInputs::new([1u8; 32], [2u8; 32], 95, true);

        let proof = validator.generate_proof(&inputs).unwrap();
        let serialized = ZkValidator::serialize_proof(&proof).unwrap();
        let deserialized = ZkValidator::deserialize_proof(&serialized).unwrap();

        // Verify deserialized proof
        let is_valid = validator
            .verify_proof(&deserialized, &inputs.public_inputs())
            .unwrap();
        assert!(is_valid);
    }

    #[test]
    fn production_missing_keys_returns_err_instead_of_panic() {
        let dir = std::env::temp_dir().join(format!(
            "creg-zk-issue-005-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp keys dir");
        let message = match ZkValidator::from_keys_dir(dir.clone(), true) {
            Ok(_) => panic!("production missing keys must return Err, not mint ephemeral keys"),
            Err(err) => err.to_string(),
        };
        assert!(
            message.contains("PRODUCTION GUARD"),
            "expected production guard error, got {message}"
        );
        assert!(
            !ZkValidator::package_key_files_present(&dir),
            "empty dir must not look like provisioned keys"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn production_startup_error_only_when_zk_validation_enabled() {
        let dir = std::path::Path::new("/nonexistent-creg-zk-keys");
        assert!(ZkValidator::production_missing_package_keys_error(false, true, dir).is_none());
        let err = ZkValidator::production_missing_package_keys_error(true, true, dir)
            .expect("missing keys must fail closed when zk_validation is on");
        assert!(err.contains("CREG_PRODUCTION=true but ZK package key files are missing"));
        assert!(ZkValidator::production_missing_package_keys_error(true, false, dir).is_none());
    }
}
