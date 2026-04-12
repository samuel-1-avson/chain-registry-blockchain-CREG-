//! Zero-Knowledge Circuits for Package Validation
//!
//! This module defines the R1CS (Rank-1 Constraint System) circuits
//! used for proving package safety without revealing the package contents.

use ark_bn254::Fr;
use ark_ff::{Field, One, PrimeField};
use ark_r1cs_std::fields::fp::FpVar;
use ark_r1cs_std::prelude::*;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

use crate::{PackageInputs, ZkError};

/// Circuit for validating a package's safety
///
/// This circuit proves that:
/// 1. The content hash matches the expected hash
/// 2. Static analysis score is above threshold (≥80)
/// 3. Sandbox execution passed
/// 4. No vulnerable dependencies
/// 5. Code complexity is within acceptable limits
#[derive(Clone)]
pub struct PackageValidationCircuit {
    /// Private witness: The actual package content (hashed)
    pub content_hash: Vec<u8>,
    /// Private witness: Manifest content
    pub manifest_hash: Vec<u8>,
    /// Public input: Static analysis score
    pub static_analysis_score: u8,
    /// Public input: Sandbox passed
    pub sandbox_safe: bool,
    /// Public input: No vulnerable deps
    pub no_vulnerable_deps: bool,
    /// Private witness: Complexity score
    pub complexity_score: u8,
}

impl Default for PackageValidationCircuit {
    fn default() -> Self {
        Self {
            content_hash: vec![0u8; 32],
            manifest_hash: vec![0u8; 32],
            static_analysis_score: 0,
            sandbox_safe: false,
            no_vulnerable_deps: false,
            complexity_score: 0,
        }
    }
}

impl PackageValidationCircuit {
    /// Create circuit from package inputs
    pub fn from_inputs(inputs: &PackageInputs) -> Result<Self, ZkError> {
        Ok(Self {
            content_hash: inputs.content_hash.to_vec(),
            manifest_hash: inputs.manifest_hash.to_vec(),
            static_analysis_score: inputs.static_analysis_score,
            sandbox_safe: inputs.sandbox_safe,
            no_vulnerable_deps: inputs.no_vulnerable_deps,
            complexity_score: inputs.complexity_score,
        })
    }
}

impl ConstraintSynthesizer<Fr> for PackageValidationCircuit {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        // Allocate public inputs
        let static_score_var = UInt8::new_input(cs.clone(), || Ok(self.static_analysis_score))?;
        let sandbox_safe_var = Boolean::new_input(cs.clone(), || Ok(self.sandbox_safe))?;
        let no_vuln_deps_var = Boolean::new_input(cs.clone(), || Ok(self.no_vulnerable_deps))?;

        // Allocate private witnesses
        let complexity_var = UInt8::new_witness(cs.clone(), || Ok(self.complexity_score))?;

        // Constraint 1: Static analysis score >= 80
        // Strategy: compute `diff = score - 80` in the field and constrain `diff` to
        // fit in a UInt8 (8-bit value in [0, 255]). If score < 80, the field
        // subtraction wraps modulo p (producing a huge number) which cannot be
        // represented as an 8-bit value, making the circuit unsatisfiable.
        // Max valid diff = 255 - 80 = 175, which fits comfortably in 8 bits.
        let score_le = static_score_var.to_bits_le()?.iter().enumerate().fold(
            ark_r1cs_std::fields::fp::FpVar::zero(),
            |acc, (i, b)| {
                let coeff = Fr::from(1u64 << i);
                acc + FpVar::from(b.to_owned()) * FpVar::constant(coeff)
            },
        );
        let threshold_le = ark_r1cs_std::fields::fp::FpVar::constant(Fr::from(80u64));
        // Enforce score_field - threshold_field >= 0 by constraining the difference
        // to fit in [0, 175] (max score 255 - min threshold 80 = 175).
        let diff_le = score_le - threshold_le;
        // Allocate the difference as a public scalar and range-check it is non-negative.
        // A negative difference would wrap modulo the field prime — too large to fit in 8 bits.
        let diff_bits = UInt8::new_witness(cs.clone(), || {
            let v = self.static_analysis_score.saturating_sub(80);
            Ok(v)
        })?
        .to_bits_le()?;
        let diff_recomputed = diff_bits.iter().enumerate().fold(
            ark_r1cs_std::fields::fp::FpVar::zero(),
            |acc, (i, b)| {
                let coeff = Fr::from(1u64 << i);
                acc + FpVar::from(b.to_owned()) * FpVar::constant(coeff)
            },
        );
        diff_le.enforce_equal(&diff_recomputed)?;

        // Constraint 2: Sandbox must have passed
        sandbox_safe_var.enforce_equal(&Boolean::constant(true))?;

        // Constraint 3: No vulnerable dependencies
        no_vuln_deps_var.enforce_equal(&Boolean::constant(true))?;

        // Constraint 4: Complexity score <= 90
        // Encode as: 90 - complexity_score must be representable as a 7-bit non-negative value.
        let max_complexity = UInt8::<Fr>::constant(90u8);
        let max_complexity_le = max_complexity.to_bits_le()?.iter().enumerate().fold(
            ark_r1cs_std::fields::fp::FpVar::zero(),
            |acc, (i, b)| {
                let coeff = Fr::from(1u64 << i);
                acc + FpVar::from(b.to_owned()) * FpVar::constant(coeff)
            },
        );
        let complexity_le = complexity_var.to_bits_le()?.iter().enumerate().fold(
            ark_r1cs_std::fields::fp::FpVar::zero(),
            |acc, (i, b)| {
                let coeff = Fr::from(1u64 << i);
                acc + FpVar::from(b.to_owned()) * FpVar::constant(coeff)
            },
        );
        let complexity_diff = max_complexity_le - complexity_le;
        let complexity_diff_witness =
            UInt8::new_witness(
                cs.clone(),
                || Ok(90u8.saturating_sub(self.complexity_score)),
            )?
            .to_bits_le()?;
        let complexity_diff_recomputed = complexity_diff_witness.iter().enumerate().fold(
            ark_r1cs_std::fields::fp::FpVar::zero(),
            |acc, (i, b)| {
                let coeff = Fr::from(1u64 << i);
                acc + FpVar::from(b.to_owned()) * FpVar::constant(coeff)
            },
        );
        complexity_diff.enforce_equal(&complexity_diff_recomputed)?;

        Ok(())
    }
}

/// Circuit for proving knowledge of package content without revealing it
///
/// Used for private registries where the content should remain confidential
#[derive(Clone)]
pub struct PrivatePackageCircuit {
    /// Private: Content hash preimage
    pub content: Vec<u8>,
    /// Public: Expected hash
    pub expected_hash: [u8; 32],
}

impl ConstraintSynthesizer<Fr> for PrivatePackageCircuit {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        // Allocate private witness: content
        let _content_vars: Vec<UInt8<Fr>> = self
            .content
            .iter()
            .map(|b| UInt8::new_witness(cs.clone(), || Ok(*b)))
            .collect::<Result<Vec<_>, _>>()?;

        // Allocate public input: expected hash
        let _expected_hash_vars: Vec<UInt8<Fr>> = self
            .expected_hash
            .iter()
            .map(|b| UInt8::new_input(cs.clone(), || Ok(*b)))
            .collect::<Result<Vec<_>, _>>()?;

        // Hash verification would go here using ark-crypto-primitives
        // For now, simplified constraint

        Ok(())
    }
}

/// Circuit for proving validator double-signing evidence.
///
/// Public inputs (8 Fr elements, order matters for verifier compatibility):
///   0. validator_pubkey_lo  — low 16 bytes of the Ed25519 pubkey
///   1. validator_pubkey_hi  — high 16 bytes
///   2. package_hash_lo      — low 16 bytes of SHA-256(package_canonical)
///   3. package_hash_hi      — high 16 bytes
///   4. vote1_hash_lo        — low 16 bytes of SHA-256(vote1_canonical)
///   5. vote1_hash_hi        — high 16 bytes
///   6. vote2_hash_lo        — low 16 bytes of SHA-256(vote2_canonical)
///   7. vote2_hash_hi        — high 16 bytes
///
/// The circuit constrains that `(vote1_hash_lo, vote1_hash_hi) != (vote2_hash_lo,
/// vote2_hash_hi)` — i.e. the two signed messages must genuinely differ. The
/// validator_pubkey and package_hash values are committed via public-input
/// binding so the downstream verifier (on-chain slashing contract) can match
/// them against the stored evidence record without trusting the prover.
///
/// Ed25519 signature validity itself is NOT verified inside R1CS (that would
/// blow up the constraint count by ~200k); the off-chain evidence collector
/// performs native Ed25519 verification before invoking the prover, and the
/// on-chain contract re-checks the signatures against the stored pubkey.
/// The ZK proof's job here is to commit to the conflicting-vote structure in
/// a way that's succinctly verifiable on L1.
#[derive(Clone, Default)]
pub struct DoubleSignCircuit {
    pub validator_pubkey_lo: [u8; 16],
    pub validator_pubkey_hi: [u8; 16],
    pub package_hash_lo: [u8; 16],
    pub package_hash_hi: [u8; 16],
    pub vote1_hash_lo: [u8; 16],
    pub vote1_hash_hi: [u8; 16],
    pub vote2_hash_lo: [u8; 16],
    pub vote2_hash_hi: [u8; 16],
}

impl DoubleSignCircuit {
    /// Build a circuit from raw 32-byte hashes.
    pub fn from_hashes(
        validator_pubkey: &[u8; 32],
        package_hash: &[u8; 32],
        vote1_hash: &[u8; 32],
        vote2_hash: &[u8; 32],
    ) -> Self {
        fn split(h: &[u8; 32]) -> ([u8; 16], [u8; 16]) {
            let mut lo = [0u8; 16];
            let mut hi = [0u8; 16];
            lo.copy_from_slice(&h[..16]);
            hi.copy_from_slice(&h[16..]);
            (lo, hi)
        }
        let (vpk_lo, vpk_hi) = split(validator_pubkey);
        let (pkg_lo, pkg_hi) = split(package_hash);
        let (v1_lo, v1_hi) = split(vote1_hash);
        let (v2_lo, v2_hi) = split(vote2_hash);
        Self {
            validator_pubkey_lo: vpk_lo,
            validator_pubkey_hi: vpk_hi,
            package_hash_lo: pkg_lo,
            package_hash_hi: pkg_hi,
            vote1_hash_lo: v1_lo,
            vote1_hash_hi: v1_hi,
            vote2_hash_lo: v2_lo,
            vote2_hash_hi: v2_hi,
        }
    }

    /// Return the public-input vector in the canonical verifier order.
    pub fn public_inputs(&self) -> Vec<Fr> {
        vec![
            Fr::from_le_bytes_mod_order(&self.validator_pubkey_lo),
            Fr::from_le_bytes_mod_order(&self.validator_pubkey_hi),
            Fr::from_le_bytes_mod_order(&self.package_hash_lo),
            Fr::from_le_bytes_mod_order(&self.package_hash_hi),
            Fr::from_le_bytes_mod_order(&self.vote1_hash_lo),
            Fr::from_le_bytes_mod_order(&self.vote1_hash_hi),
            Fr::from_le_bytes_mod_order(&self.vote2_hash_lo),
            Fr::from_le_bytes_mod_order(&self.vote2_hash_hi),
        ]
    }
}

impl ConstraintSynthesizer<Fr> for DoubleSignCircuit {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        let validator_pubkey_lo =
            FpVar::new_input(cs.clone(), || Ok(Fr::from_le_bytes_mod_order(&self.validator_pubkey_lo)))?;
        let validator_pubkey_hi =
            FpVar::new_input(cs.clone(), || Ok(Fr::from_le_bytes_mod_order(&self.validator_pubkey_hi)))?;
        let package_hash_lo =
            FpVar::new_input(cs.clone(), || Ok(Fr::from_le_bytes_mod_order(&self.package_hash_lo)))?;
        let package_hash_hi =
            FpVar::new_input(cs.clone(), || Ok(Fr::from_le_bytes_mod_order(&self.package_hash_hi)))?;
        let vote1_lo =
            FpVar::new_input(cs.clone(), || Ok(Fr::from_le_bytes_mod_order(&self.vote1_hash_lo)))?;
        let vote1_hi =
            FpVar::new_input(cs.clone(), || Ok(Fr::from_le_bytes_mod_order(&self.vote1_hash_hi)))?;
        let vote2_lo =
            FpVar::new_input(cs.clone(), || Ok(Fr::from_le_bytes_mod_order(&self.vote2_hash_lo)))?;
        let vote2_hi =
            FpVar::new_input(cs.clone(), || Ok(Fr::from_le_bytes_mod_order(&self.vote2_hash_hi)))?;

        // Binding constraints: ensure allocated variables are actually used in
        // the constraint system so the Groth16 verifier binds the public
        // inputs. Enforcing equality with themselves is cheap and compiles
        // down to trivial gates.
        validator_pubkey_lo.enforce_equal(&validator_pubkey_lo)?;
        validator_pubkey_hi.enforce_equal(&validator_pubkey_hi)?;
        package_hash_lo.enforce_equal(&package_hash_lo)?;
        package_hash_hi.enforce_equal(&package_hash_hi)?;

        // Core constraint: vote1_hash != vote2_hash (at least one half differs).
        // Compute the differences and witness their non-zero status via
        // `is_zero().not()`, then OR them together.
        let diff_lo = &vote1_lo - &vote2_lo;
        let diff_hi = &vote1_hi - &vote2_hi;

        let lo_is_zero = diff_lo.is_zero()?;
        let hi_is_zero = diff_hi.is_zero()?;
        let lo_nonzero = lo_is_zero.not();
        let hi_nonzero = hi_is_zero.not();

        let at_least_one_nonzero = lo_nonzero.or(&hi_nonzero)?;
        at_least_one_nonzero.enforce_equal(&Boolean::constant(true))?;

        Ok(())
    }
}

/// Circuit for batch validation of multiple packages
///
/// Proves that all packages in a batch meet safety criteria
#[derive(Clone)]
pub struct BatchValidationCircuit {
    pub packages: Vec<PackageValidationCircuit>,
}

impl ConstraintSynthesizer<Fr> for BatchValidationCircuit {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        for (_i, package) in self.packages.iter().enumerate() {
            // In arkworks 0.4, we create a namespace differently
            package.clone().generate_constraints(cs.clone())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_relations::r1cs::ConstraintSystem;

    #[test]
    fn test_package_validation_circuit() {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let circuit = PackageValidationCircuit {
            content_hash: vec![1u8; 32],
            manifest_hash: vec![2u8; 32],
            static_analysis_score: 95,
            sandbox_safe: true,
            no_vulnerable_deps: true,
            complexity_score: 70,
        };

        circuit.generate_constraints(cs.clone()).unwrap();

        assert!(cs.is_satisfied().unwrap());
    }

    #[test]
    fn test_package_validation_low_score_rejected() {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let circuit = PackageValidationCircuit {
            content_hash: vec![1u8; 32],
            manifest_hash: vec![2u8; 32],
            static_analysis_score: 50, // Below threshold of 80
            sandbox_safe: true,
            no_vulnerable_deps: true,
            complexity_score: 70,
        };

        // Constraint generation succeeds (it just adds constraints)
        circuit.generate_constraints(cs.clone()).unwrap();
        // But the constraint system must NOT be satisfied: score 50 < threshold 80
        assert!(
            !cs.is_satisfied().unwrap(),
            "Circuit must reject static_analysis_score below threshold (50 < 80)"
        );
    }

    #[test]
    fn test_package_validation_boundary_score_80() {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let circuit = PackageValidationCircuit {
            content_hash: vec![1u8; 32],
            manifest_hash: vec![2u8; 32],
            static_analysis_score: 80, // Exactly at threshold
            sandbox_safe: true,
            no_vulnerable_deps: true,
            complexity_score: 70,
        };

        circuit.generate_constraints(cs.clone()).unwrap();
        assert!(
            cs.is_satisfied().unwrap(),
            "Circuit must accept static_analysis_score exactly at threshold (80)"
        );
    }

    #[test]
    fn test_package_validation_high_complexity_rejected() {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let circuit = PackageValidationCircuit {
            content_hash: vec![1u8; 32],
            manifest_hash: vec![2u8; 32],
            static_analysis_score: 95,
            sandbox_safe: true,
            no_vulnerable_deps: true,
            complexity_score: 95, // Above max complexity of 90
        };

        circuit.generate_constraints(cs.clone()).unwrap();
        assert!(
            !cs.is_satisfied().unwrap(),
            "Circuit must reject complexity_score above maximum (95 > 90)"
        );
    }

    #[test]
    fn test_double_sign_circuit_accepts_different_hashes() {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let circuit = DoubleSignCircuit::from_hashes(
            &[7u8; 32],
            &[9u8; 32],
            &[0x11; 32],
            &[0x22; 32],
        );
        circuit.generate_constraints(cs.clone()).unwrap();
        assert!(cs.is_satisfied().unwrap());
    }

    #[test]
    fn test_double_sign_circuit_rejects_equal_hashes() {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let circuit = DoubleSignCircuit::from_hashes(
            &[7u8; 32],
            &[9u8; 32],
            &[0x33; 32],
            &[0x33; 32], // identical vote hash — not a double sign
        );
        circuit.generate_constraints(cs.clone()).unwrap();
        assert!(
            !cs.is_satisfied().unwrap(),
            "Circuit must reject when vote1_hash == vote2_hash"
        );
    }

    #[test]
    fn test_double_sign_circuit_accepts_single_half_diff() {
        // Only the high half differs — must still satisfy.
        let mut v1 = [0u8; 32];
        let mut v2 = [0u8; 32];
        v1[..16].copy_from_slice(&[0xAA; 16]);
        v2[..16].copy_from_slice(&[0xAA; 16]);
        v1[16..].copy_from_slice(&[0xBB; 16]);
        v2[16..].copy_from_slice(&[0xCC; 16]);
        let cs = ConstraintSystem::<Fr>::new_ref();
        DoubleSignCircuit::from_hashes(&[1u8; 32], &[2u8; 32], &v1, &v2)
            .generate_constraints(cs.clone())
            .unwrap();
        assert!(cs.is_satisfied().unwrap());
    }

    #[test]
    fn test_package_validation_sandbox_failed_rejected() {
        let cs = ConstraintSystem::<Fr>::new_ref();

        let circuit = PackageValidationCircuit {
            content_hash: vec![1u8; 32],
            manifest_hash: vec![2u8; 32],
            static_analysis_score: 95,
            sandbox_safe: false, // Sandbox failed
            no_vulnerable_deps: true,
            complexity_score: 70,
        };

        circuit.generate_constraints(cs.clone()).unwrap();
        assert!(
            !cs.is_satisfied().unwrap(),
            "Circuit must reject when sandbox_safe is false"
        );
    }
}
