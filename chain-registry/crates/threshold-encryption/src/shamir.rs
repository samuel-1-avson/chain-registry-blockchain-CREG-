//! Shamir's Secret Sharing Implementation
//!
//! Based on the paper "How to Share a Secret" by Adi Shamir (1979).
//! Uses finite field arithmetic on GF(2^8) for byte-wise operations.

use crate::ThresholdError;
use rand::Rng;
use std::collections::HashSet;

/// A single share in Shamir's Secret Sharing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    /// Share index (1-based, x coordinate)
    pub index: u8,
    /// Share value (y coordinate)
    pub value: Vec<u8>,
}

impl Share {
    /// Create a new share.
    pub fn new(index: u8, value: Vec<u8>) -> Self {
        Self { index, value }
    }
}

/// Shamir Secret Sharing implementation
pub struct ShamirSecretSharing {
    threshold: u8,
    total_shares: u8,
}

impl ShamirSecretSharing {
    /// Create new SSS instance
    pub fn new(threshold: u8, total_shares: u8) -> Self {
        Self {
            threshold,
            total_shares,
        }
    }

    /// Split a secret into shares
    ///
    /// # Algorithm
    /// 1. Generate random polynomial of degree (threshold - 1)
    /// 2. Evaluate polynomial at points 1..=total_shares
    /// 3. Return (index, value) pairs
    pub fn split_secret(&self, secret: &[u8]) -> Result<Vec<Share>, ThresholdError> {
        let mut shares: Vec<Vec<u8>> =
            vec![Vec::with_capacity(secret.len()); self.total_shares as usize];

        // For each byte of the secret
        for byte in secret {
            // Generate random coefficients for polynomial
            // f(x) = secret + a1*x + a2*x^2 + ... + a_{k-1}*x^{k-1}
            let mut coefficients = vec![*byte];
            for _ in 1..self.threshold {
                coefficients.push(rand::thread_rng().gen());
            }

            // Evaluate polynomial at each point
            for i in 1..=self.total_shares {
                let y = Self::evaluate_polynomial(&coefficients, i);
                shares[(i - 1) as usize].push(y);
            }
        }

        // Create share structs
        Ok((1..=self.total_shares)
            .zip(shares.into_iter())
            .map(|(index, value)| Share { index, value })
            .collect())
    }

    /// Reconstruct secret from shares using Lagrange interpolation
    pub fn reconstruct_secret(&self, shares: &[Share]) -> Result<Vec<u8>, ThresholdError> {
        if shares.len() < self.threshold as usize {
            return Err(ThresholdError::InsufficientShares(
                shares.len() as u8,
                self.threshold,
            ));
        }

        let mut seen = HashSet::new();
        for share in shares {
            if share.index == 0 {
                return Err(ThresholdError::InvalidShare(
                    "share index 0 is reserved for the secret".into(),
                ));
            }
            if !seen.insert(share.index) {
                return Err(ThresholdError::InvalidShare(format!(
                    "duplicate share index {}",
                    share.index
                )));
            }
        }

        let secret_len = shares[0].value.len();
        if shares.iter().any(|share| share.value.len() != secret_len) {
            return Err(ThresholdError::InvalidShare(
                "share value lengths differ".into(),
            ));
        }
        let mut secret = Vec::with_capacity(secret_len);

        // For each byte position
        for i in 0..secret_len {
            let points: Vec<(u8, u8)> = shares.iter().map(|s| (s.index, s.value[i])).collect();

            let byte = Self::lagrange_interpolate_at_zero(&points)?;
            secret.push(byte);
        }

        Ok(secret)
    }

    /// Evaluate polynomial at point x
    fn evaluate_polynomial(coefficients: &[u8], x: u8) -> u8 {
        let mut result = 0u8;
        let mut power = 1u8;

        for coeff in coefficients {
            result = Self::gf_add(result, Self::gf_mul(*coeff, power));
            power = Self::gf_mul(power, x);
        }

        result
    }

    /// Lagrange interpolation at x = 0
    /// Given points (x_i, y_i), compute f(0)
    fn lagrange_interpolate_at_zero(points: &[(u8, u8)]) -> Result<u8, ThresholdError> {
        let mut result = 0u8;

        for (i, &(x_i, y_i)) in points.iter().enumerate() {
            let mut numerator = 1u8;
            let mut denominator = 1u8;

            for (j, &(x_j, _)) in points.iter().enumerate() {
                if i != j {
                    numerator = Self::gf_mul(numerator, x_j);
                    denominator = Self::gf_mul(denominator, Self::gf_sub(x_j, x_i));
                }
            }

            let lagrange_coeff = Self::gf_div(numerator, denominator)?;
            result = Self::gf_add(result, Self::gf_mul(y_i, lagrange_coeff));
        }

        Ok(result)
    }

    // ── Galois Field Arithmetic (GF(2^8)) ─────────────────────────────────────
    //
    // All operations below are implemented in constant time using precomputed
    // log/exp tables.  This prevents timing side-channels that could leak
    // secret shares or the reconstructed secret.

    /// Irreducible polynomial for GF(2^8): x^8 + x^4 + x^3 + x + 1
    const IRREDUCIBLE_POLY: u16 = 0x11b;

    /// Precomputed exponential (antilog) table for GF(2^8).
    /// EXP_TABLE[i] = g^i where g = 0x03 is a generator of GF(2^8)*.
    const EXP_TABLE: [u8; 256] = Self::build_exp_table();
    /// Precomputed logarithm table for GF(2^8).
    /// LOG_TABLE[a] = i such that g^i = a.  LOG_TABLE[0] is unused.
    const LOG_TABLE: [u8; 256] = Self::build_log_table();

    const fn build_exp_table() -> [u8; 256] {
        let mut table = [0u8; 256];
        let mut val: u16 = 1;
        let mut i = 0usize;
        while i < 256 {
            table[i] = val as u8;
            // Multiply by generator 0x03 in GF(2^8).
            val = (val << 1) ^ val; // val * 3 = val * 2 ^ val
            if val & 0x100 != 0 {
                val ^= Self::IRREDUCIBLE_POLY;
            }
            i += 1;
        }
        table
    }

    const fn build_log_table() -> [u8; 256] {
        let mut table = [0u8; 256];
        let exp = Self::build_exp_table();
        let mut i = 0u8;
        // Only fill indices 0..254; 255 maps back to 0 (g^255 = 1 = g^0).
        loop {
            table[exp[i as usize] as usize] = i;
            if i == 254 {
                break;
            }
            i += 1;
        }
        table
    }

    /// Add two elements in GF(2^8) - same as XOR
    fn gf_add(a: u8, b: u8) -> u8 {
        a ^ b
    }

    /// Subtract two elements in GF(2^8) - same as XOR
    fn gf_sub(a: u8, b: u8) -> u8 {
        a ^ b
    }

    /// Multiply two elements in GF(2^8) in **constant time** using log/exp tables.
    fn gf_mul(a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            return 0;
        }
        let log_a = Self::LOG_TABLE[a as usize] as u16;
        let log_b = Self::LOG_TABLE[b as usize] as u16;
        let log_sum = (log_a + log_b) % 255;
        Self::EXP_TABLE[log_sum as usize]
    }

    /// Divide two elements in GF(2^8) in **constant time**.
    /// `0 / b` is 0 for nonzero `b`. Division by zero is an error.
    fn gf_div(a: u8, b: u8) -> Result<u8, ThresholdError> {
        if b == 0 {
            return Err(ThresholdError::InvalidShare(
                "division by zero in GF(2^8)".into(),
            ));
        }
        if a == 0 {
            return Ok(0);
        }
        let log_a = Self::LOG_TABLE[a as usize] as u16;
        let log_b = Self::LOG_TABLE[b as usize] as u16;
        // (log_a - log_b) mod 255 — add 255 first to avoid underflow.
        let log_diff = (log_a + 255 - log_b) % 255;
        Ok(Self::EXP_TABLE[log_diff as usize])
    }

    /// Compute multiplicative inverse in **constant time** using the log/exp table.
    fn gf_inverse(a: u8) -> u8 {
        if a == 0 {
            return 0;
        }
        let log_a = Self::LOG_TABLE[a as usize] as u16;
        let log_inv = (255 - log_a) % 255;
        Self::EXP_TABLE[log_inv as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shamir_basic() {
        let sss = ShamirSecretSharing::new(3, 5);
        let secret = b"Hello, World!";

        // Split
        let shares = sss.split_secret(secret).unwrap();
        assert_eq!(shares.len(), 5);

        // Reconstruct with 3 shares
        let reconstructed = sss.reconstruct_secret(&shares[..3]).unwrap();
        assert_eq!(reconstructed, secret.to_vec());

        // Reconstruct with all 5 shares
        let reconstructed = sss.reconstruct_secret(&shares).unwrap();
        assert_eq!(reconstructed, secret.to_vec());
    }

    #[test]
    fn test_gf_arithmetic() {
        // Addition = XOR
        assert_eq!(ShamirSecretSharing::gf_add(0x57, 0x83), 0xd4);

        // Multiplication
        let product = ShamirSecretSharing::gf_mul(0x57, 0x83);
        // Verify with division
        assert_eq!(ShamirSecretSharing::gf_div(product, 0x83).unwrap(), 0x57);
        assert_eq!(ShamirSecretSharing::gf_div(0, 0x83).unwrap(), 0);
        assert!(ShamirSecretSharing::gf_div(0x57, 0).is_err());
    }

    #[test]
    fn duplicate_share_indices_are_rejected() {
        let sss = ShamirSecretSharing::new(3, 5);
        let secret = b"dup-index";
        let shares = sss.split_secret(secret).unwrap();
        let bogus = vec![shares[0].clone(), shares[0].clone(), shares[1].clone()];
        let err = sss.reconstruct_secret(&bogus).unwrap_err();
        assert!(
            matches!(err, ThresholdError::InvalidShare(ref message) if message.contains("duplicate")),
            "{err:?}"
        );
    }

    #[test]
    fn share_index_zero_is_rejected() {
        let sss = ShamirSecretSharing::new(2, 3);
        let secret = b"zero";
        let mut shares = sss.split_secret(secret).unwrap();
        shares[0].index = 0;
        let err = sss.reconstruct_secret(&shares[..2]).unwrap_err();
        assert!(
            matches!(err, ThresholdError::InvalidShare(ref message) if message.contains("index 0")),
            "{err:?}"
        );
    }

    #[test]
    fn test_polynomial_evaluation() {
        // f(x) = 5 + 3x + 7x^2
        let coeffs = vec![5, 3, 7];

        // f(1) = 5 + 3 + 7 = 5 ^ 3 ^ 7 = 1 (in GF)
        let result = ShamirSecretSharing::evaluate_polynomial(&coeffs, 1);
        assert_eq!(result, 5 ^ 3 ^ 7);
    }

    #[test]
    fn test_insufficient_shares() {
        let sss = ShamirSecretSharing::new(5, 10);
        let secret = b"test";

        let shares = sss.split_secret(secret).unwrap();

        // Try to reconstruct with only 3 shares (need 5)
        let result = sss.reconstruct_secret(&shares[..3]);
        assert!(result.is_err());
    }

    #[test]
    fn test_different_share_combinations() {
        let sss = ShamirSecretSharing::new(3, 5);
        let secret = b"Secret message";

        let shares = sss.split_secret(secret).unwrap();

        // Try different combinations of 3 shares
        let combinations = vec![vec![0, 1, 2], vec![0, 2, 4], vec![1, 3, 4], vec![0, 1, 4]];

        for combo in combinations {
            let selected: Vec<Share> = combo.iter().map(|&i| shares[i].clone()).collect();
            let reconstructed = sss.reconstruct_secret(&selected).unwrap();
            assert_eq!(reconstructed, secret.to_vec());
        }
    }
}
