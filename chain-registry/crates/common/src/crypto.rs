// crates/common/src/crypto.rs
//! Signature schemes behind one interface (ADR-017, increments 1–2).
//!
//! Every CREG-native signature is created and checked through this module.
//! Increment 1 introduced the interface over Ed25519; Increment 2 adds
//! [`SignatureScheme::HybridEd25519MlDsa44V1`] (Ed25519 + pure FIPS 204
//! ML-DSA-44) behind the abstraction without changing any caller, consensus
//! message, or wire format.
//!
//! Hex parsing, length checks with custom wording and error messages stay at
//! the call sites: they differ per protocol message, and some are visible to
//! API clients.
//!
//! [`SignatureScheme`] is deliberately not `#[non_exhaustive]`: adding the
//! hybrid scheme forces every `match` in the workspace to be revisited.

use sha2::{Digest, Sha256};
use std::fmt;

/// Encoded Ed25519 public-key length in bytes.
pub const ED25519_PUBLIC_KEY_LEN: usize = ed25519_dalek::PUBLIC_KEY_LENGTH;
/// Encoded Ed25519 signature length in bytes.
pub const ED25519_SIGNATURE_LEN: usize = ed25519_dalek::SIGNATURE_LENGTH;
/// Ed25519 secret seed length in bytes.
pub const ED25519_SEED_LEN: usize = ed25519_dalek::SECRET_KEY_LENGTH;

/// Encoded FIPS 204 ML-DSA-44 public-key length in bytes.
pub const MLDSA44_PUBLIC_KEY_LEN: usize = 1_312;
/// Encoded FIPS 204 ML-DSA-44 signature length in bytes.
pub const MLDSA44_SIGNATURE_LEN: usize = 2_420;
/// FIPS 204 ML-DSA-44 secret seed (`ξ`) length in bytes.
pub const MLDSA44_SEED_LEN: usize = 32;

/// Concatenated hybrid public-key length in bytes (`32 + 1312 = 1344`).
pub const HYBRID_PUBLIC_KEY_LEN: usize = ED25519_PUBLIC_KEY_LEN + MLDSA44_PUBLIC_KEY_LEN;
/// Concatenated hybrid signature length in bytes (`64 + 2420 = 2484`).
pub const HYBRID_SIGNATURE_LEN: usize = ED25519_SIGNATURE_LEN + MLDSA44_SIGNATURE_LEN;
/// Concatenated hybrid seed length in bytes (`32 + 32 = 64`).
pub const HYBRID_SEED_LEN: usize = ED25519_SEED_LEN + MLDSA44_SEED_LEN;

/// Domain-separation prefix prepended (with a `0x00` byte) to protocol messages
/// before signing or verifying with [`SignatureScheme::HybridEd25519MlDsa44V1`].
pub const HYBRID_DOMAIN_PREFIX: &[u8] = b"creg-hybrid-sig-v1";

/// Pure FIPS 204 ML-DSA-44 context string used by
/// [`SignatureScheme::HybridEd25519MlDsa44V1`].
pub const HYBRID_MLDSA_CONTEXT: &[u8] = b"creg-hybrid-sig-v1";

/// Domain-separation prefix used to compute [`PublicKey::key_id`].
pub const KEY_ID_DOMAIN_PREFIX: &[u8] = b"creg-key-id-v1";

/// `M' = "creg-hybrid-sig-v1" || 0x00 || message`: the bytes both halves of a
/// [`SignatureScheme::HybridEd25519MlDsa44V1`] signature cover.
pub fn hybrid_message(message: &[u8]) -> Vec<u8> {
    let mut m_prime = Vec::with_capacity(HYBRID_DOMAIN_PREFIX.len() + 1 + message.len());
    m_prime.extend_from_slice(HYBRID_DOMAIN_PREFIX);
    m_prime.push(0x00);
    m_prime.extend_from_slice(message);
    m_prime
}

struct OsRngBridge;

#[derive(Debug)]
struct OsRngError(rand::Error);

impl fmt::Display for OsRngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for OsRngError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl ml_dsa::signature::rand_core::TryRng for OsRngBridge {
    type Error = OsRngError;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        let mut buf = [0u8; 4];
        self.try_fill_bytes(&mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        let mut buf = [0u8; 8];
        self.try_fill_bytes(&mut buf)?;
        Ok(u64::from_le_bytes(buf))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        <rand::rngs::OsRng as rand::RngCore>::try_fill_bytes(&mut rand::rngs::OsRng, dst)
            .map_err(OsRngError)
    }
}

impl ml_dsa::signature::rand_core::TryCryptoRng for OsRngBridge {}

/// A signature scheme CREG can sign and verify with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignatureScheme {
    /// RFC 8032 Ed25519, implemented by `ed25519-dalek`.
    Ed25519,
    /// Hybrid Ed25519 + pure FIPS 204 ML-DSA-44 (`hybrid-ed25519-mldsa44-v1`).
    HybridEd25519MlDsa44V1,
}

impl SignatureScheme {
    /// Stable identifier, as written in the chain spec's `signature_algorithm`.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Ed25519 => "ed25519",
            Self::HybridEd25519MlDsa44V1 => "hybrid-ed25519-mldsa44-v1",
        }
    }

    /// Parse an identifier produced by [`SignatureScheme::id`]. Exact match only.
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "ed25519" => Some(Self::Ed25519),
            "hybrid-ed25519-mldsa44-v1" => Some(Self::HybridEd25519MlDsa44V1),
            _ => None,
        }
    }

    /// Encoded public-key length in bytes.
    ///
    /// For [`SignatureScheme::HybridEd25519MlDsa44V1`], this returns the
    /// concatenated encoding size (`1344 = 32 + 1312`) used for test vectors
    /// and [`PublicKey::key_id`]; protocol wire structs carry separate
    /// fields (ADR-017 §3, Increment 4).
    pub const fn public_key_len(self) -> usize {
        match self {
            Self::Ed25519 => ED25519_PUBLIC_KEY_LEN,
            Self::HybridEd25519MlDsa44V1 => HYBRID_PUBLIC_KEY_LEN,
        }
    }

    /// Encoded signature length in bytes.
    ///
    /// For [`SignatureScheme::HybridEd25519MlDsa44V1`], this returns the
    /// concatenated encoding size (`2484 = 64 + 2420`) used for test vectors;
    /// protocol wire structs carry separate optional fields (ADR-017 §3,
    /// Increment 4).
    pub const fn signature_len(self) -> usize {
        match self {
            Self::Ed25519 => ED25519_SIGNATURE_LEN,
            Self::HybridEd25519MlDsa44V1 => HYBRID_SIGNATURE_LEN,
        }
    }

    /// Secret seed length in bytes.
    ///
    /// For [`SignatureScheme::HybridEd25519MlDsa44V1`], this returns the
    /// concatenated independent seed length (`64 = 32 + 32`).
    pub const fn seed_len(self) -> usize {
        match self {
            Self::Ed25519 => ED25519_SEED_LEN,
            Self::HybridEd25519MlDsa44V1 => HYBRID_SEED_LEN,
        }
    }
}

impl fmt::Display for SignatureScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// How strictly an Ed25519 signature is checked.
///
/// Every call site keeps the mode it used before ADR-017. Moving a site from
/// [`VerifyMode::Permissive`] to [`VerifyMode::Strict`] can reject a
/// signature that is already recorded on chain or in a log, so it is a
/// consensus change and needs its own review.
///
/// [`SignatureScheme::HybridEd25519MlDsa44V1`] always verifies its Ed25519
/// component with `verify_strict` (ignoring [`VerifyMode::Permissive`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    /// `VerifyingKey::verify_strict`: additionally rejects small-order public
    /// keys and signature points. Honest signers never produce them.
    Strict,
    /// `Verifier::verify`: the cofactorless RFC 8032 check as `ed25519-dalek`
    /// implements it. Accepts small-order keys. Ignored by hybrid verification,
    /// which always verifies strictly.
    Permissive,
}

/// A key, signature or verification rejected by the underlying library.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// Rejected by `ed25519-dalek`. Displays the library's own message, so
    /// call sites that format the error produce the same text as before.
    #[error(transparent)]
    Ed25519(#[from] ed25519_dalek::SignatureError),
    /// Rejected by FIPS 204 ML-DSA-44 decoding or verification.
    #[error("ML-DSA-44 error: {0}")]
    MlDsa44(String),
    /// Public key and signature use different [`SignatureScheme`] variants.
    #[error("signature scheme mismatch: key is {key}, signature is {signature}")]
    SchemeMismatch {
        key: SignatureScheme,
        signature: SignatureScheme,
    },
    /// Input slice did not match the exact expected byte length.
    #[error("invalid {kind} length for {scheme}: expected {expected} bytes, got {actual}")]
    InvalidLength {
        scheme: SignatureScheme,
        kind: &'static str,
        expected: usize,
        actual: usize,
    },
}

#[derive(Clone)]
enum PublicKeyInner {
    Ed25519(ed25519_dalek::VerifyingKey),
    HybridEd25519MlDsa44V1 {
        ed25519: ed25519_dalek::VerifyingKey,
        mldsa44: Box<ml_dsa::VerifyingKey<ml_dsa::MlDsa44>>,
    },
}

impl PartialEq for PublicKeyInner {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ed25519(a), Self::Ed25519(b)) => a == b,
            (
                Self::HybridEd25519MlDsa44V1 {
                    ed25519: a_ed,
                    mldsa44: a_pq,
                },
                Self::HybridEd25519MlDsa44V1 {
                    ed25519: b_ed,
                    mldsa44: b_pq,
                },
            ) => a_ed == b_ed && a_pq == b_pq,
            _ => false,
        }
    }
}

impl Eq for PublicKeyInner {}

/// A public verification key.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicKey(PublicKeyInner);

impl PublicKey {
    /// Decode a public key. For Ed25519 this is
    /// `VerifyingKey::try_from(&[u8])`: exactly 32 bytes that decompress to a
    /// curve point. Small-order points are accepted here; strict verification
    /// rejects them later. For [`SignatureScheme::HybridEd25519MlDsa44V1`],
    /// `bytes` must be the exact 1,344-byte concatenated form
    /// `ed25519_pk (32) || mldsa44_pk (1312)`.
    pub fn from_bytes(scheme: SignatureScheme, bytes: &[u8]) -> Result<Self, CryptoError> {
        match scheme {
            SignatureScheme::Ed25519 => Ok(Self(PublicKeyInner::Ed25519(
                ed25519_dalek::VerifyingKey::try_from(bytes)?,
            ))),
            SignatureScheme::HybridEd25519MlDsa44V1 => {
                if bytes.len() != HYBRID_PUBLIC_KEY_LEN {
                    return Err(CryptoError::InvalidLength {
                        scheme,
                        kind: "public key",
                        expected: HYBRID_PUBLIC_KEY_LEN,
                        actual: bytes.len(),
                    });
                }
                let (ed, pq) = bytes.split_at(ED25519_PUBLIC_KEY_LEN);
                Self::hybrid_from_parts(ed, pq)
            }
        }
    }

    /// Decode a [`SignatureScheme::HybridEd25519MlDsa44V1`] public key from its
    /// separate Ed25519 (`32` bytes) and ML-DSA-44 (`1312` bytes) components.
    pub fn hybrid_from_parts(ed25519: &[u8], mldsa44: &[u8]) -> Result<Self, CryptoError> {
        if ed25519.len() != ED25519_PUBLIC_KEY_LEN {
            return Err(CryptoError::InvalidLength {
                scheme: SignatureScheme::HybridEd25519MlDsa44V1,
                kind: "Ed25519 public key",
                expected: ED25519_PUBLIC_KEY_LEN,
                actual: ed25519.len(),
            });
        }
        if mldsa44.len() != MLDSA44_PUBLIC_KEY_LEN {
            return Err(CryptoError::InvalidLength {
                scheme: SignatureScheme::HybridEd25519MlDsa44V1,
                kind: "ML-DSA-44 public key",
                expected: MLDSA44_PUBLIC_KEY_LEN,
                actual: mldsa44.len(),
            });
        }

        let ed_key = ed25519_dalek::VerifyingKey::try_from(ed25519)?;
        let pq_enc = ml_dsa::EncodedVerifyingKey::<ml_dsa::MlDsa44>::try_from(mldsa44)
            .map_err(|_| CryptoError::MlDsa44("invalid ML-DSA-44 public key length".to_string()))?;
        let pq_key = ml_dsa::VerifyingKey::<ml_dsa::MlDsa44>::decode(&pq_enc);

        Ok(Self(PublicKeyInner::HybridEd25519MlDsa44V1 {
            ed25519: ed_key,
            mldsa44: Box::new(pq_key),
        }))
    }

    pub fn scheme(&self) -> SignatureScheme {
        match &self.0 {
            PublicKeyInner::Ed25519(_) => SignatureScheme::Ed25519,
            PublicKeyInner::HybridEd25519MlDsa44V1 { .. } => {
                SignatureScheme::HybridEd25519MlDsa44V1
            }
        }
    }

    /// Raw 32-byte Ed25519 public key (the whole key for Ed25519, or the
    /// Ed25519 component for hybrid).
    pub fn ed25519_bytes(&self) -> [u8; ED25519_PUBLIC_KEY_LEN] {
        match &self.0 {
            PublicKeyInner::Ed25519(key) => key.to_bytes(),
            PublicKeyInner::HybridEd25519MlDsa44V1 { ed25519, .. } => ed25519.to_bytes(),
        }
    }

    /// Raw 1,312-byte ML-DSA-44 public key, or `None` for Ed25519.
    pub fn mldsa44_bytes(&self) -> Option<Vec<u8>> {
        match &self.0 {
            PublicKeyInner::Ed25519(_) => None,
            PublicKeyInner::HybridEd25519MlDsa44V1 { mldsa44, .. } => {
                Some(mldsa44.encode().as_slice().to_vec())
            }
        }
    }

    /// Separate `(ed25519_pk, mldsa44_pk)` components, or `None` for Ed25519.
    pub fn hybrid_parts(&self) -> Option<([u8; ED25519_PUBLIC_KEY_LEN], Vec<u8>)> {
        match &self.0 {
            PublicKeyInner::Ed25519(_) => None,
            PublicKeyInner::HybridEd25519MlDsa44V1 { ed25519, mldsa44 } => {
                Some((ed25519.to_bytes(), mldsa44.encode().as_slice().to_vec()))
            }
        }
    }

    /// Canonical encoding (concatenated `ed25519_pk || mldsa44_pk` for hybrid).
    pub fn to_bytes(&self) -> Vec<u8> {
        match &self.0 {
            PublicKeyInner::Ed25519(key) => key.as_bytes().to_vec(),
            PublicKeyInner::HybridEd25519MlDsa44V1 { ed25519, mldsa44 } => {
                let mut out = Vec::with_capacity(HYBRID_PUBLIC_KEY_LEN);
                out.extend_from_slice(ed25519.as_bytes());
                out.extend_from_slice(mldsa44.encode().as_slice());
                out
            }
        }
    }

    /// Lowercase hex of [`PublicKey::to_bytes`], without a `0x` prefix.
    pub fn to_hex(&self) -> String {
        hex::encode(self.to_bytes())
    }

    /// Key identifier per ADR-017 §3:
    /// `sha256("creg-key-id-v1" || 0x00 || public_key_bytes)` as lowercase hex
    /// (for hybrid: `sha256("creg-key-id-v1" || 0x00 || ed25519_pk || mldsa44_pk)`).
    pub fn key_id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(KEY_ID_DOMAIN_PREFIX);
        hasher.update([0x00]);
        hasher.update(self.to_bytes());
        hex::encode(hasher.finalize())
    }

    /// Check `signature` over `message`.
    ///
    /// Hybrid keys always verify their Ed25519 component with `verify_strict`
    /// over `M' = "creg-hybrid-sig-v1" || 0x00 || message`, ignoring
    /// [`VerifyMode::Permissive`], and verify the ML-DSA-44 component over `M'`
    /// with context `"creg-hybrid-sig-v1"`.
    pub fn verify(
        &self,
        message: &[u8],
        signature: &Signature,
        mode: VerifyMode,
    ) -> Result<(), CryptoError> {
        match (&self.0, &signature.0) {
            (PublicKeyInner::Ed25519(key), SignatureInner::Ed25519(signature)) => match mode {
                VerifyMode::Strict => key.verify_strict(message, signature)?,
                VerifyMode::Permissive => {
                    use ed25519_dalek::Verifier;
                    key.verify(message, signature)?
                }
            },
            (
                PublicKeyInner::HybridEd25519MlDsa44V1 {
                    ed25519: ed_key,
                    mldsa44: pq_key,
                },
                SignatureInner::HybridEd25519MlDsa44V1 {
                    ed25519: ed_sig,
                    mldsa44: pq_sig,
                },
            ) => {
                let m_prime = hybrid_message(message);
                ed_key.verify_strict(&m_prime, ed_sig)?;
                if !pq_key.verify_with_context(&m_prime, HYBRID_MLDSA_CONTEXT, pq_sig) {
                    return Err(CryptoError::MlDsa44(
                        "signature verification failed".to_string(),
                    ));
                }
            }
            _ => {
                return Err(CryptoError::SchemeMismatch {
                    key: self.scheme(),
                    signature: signature.scheme(),
                });
            }
        }
        Ok(())
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            PublicKeyInner::Ed25519(_) => {
                write!(f, "PublicKey({}:{})", self.scheme(), self.to_hex())
            }
            PublicKeyInner::HybridEd25519MlDsa44V1 { .. } => {
                write!(f, "PublicKey({}:key_id={})", self.scheme(), self.key_id())
            }
        }
    }
}

#[derive(Clone)]
enum SignatureInner {
    Ed25519(ed25519_dalek::Signature),
    HybridEd25519MlDsa44V1 {
        ed25519: ed25519_dalek::Signature,
        mldsa44: Box<ml_dsa::Signature<ml_dsa::MlDsa44>>,
    },
}

impl PartialEq for SignatureInner {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ed25519(a), Self::Ed25519(b)) => a == b,
            (
                Self::HybridEd25519MlDsa44V1 {
                    ed25519: a_ed,
                    mldsa44: a_pq,
                },
                Self::HybridEd25519MlDsa44V1 {
                    ed25519: b_ed,
                    mldsa44: b_pq,
                },
            ) => a_ed == b_ed && a_pq == b_pq,
            _ => false,
        }
    }
}

impl Eq for SignatureInner {}

/// A detached signature.
#[derive(Clone, PartialEq, Eq)]
pub struct Signature(SignatureInner);

impl Signature {
    /// Decode a signature. For Ed25519 this is `Signature::try_from(&[u8])`,
    /// which checks only the length (64 bytes); the scalar and point are
    /// checked during verification. For [`SignatureScheme::HybridEd25519MlDsa44V1`],
    /// `bytes` must be exactly 2,484 bytes (`ed25519 (64) || mldsa44 (2420)`),
    /// and ML-DSA-44 hint monotonicity/uniqueness and `z` norm bounds are
    /// validated on decode.
    pub fn from_bytes(scheme: SignatureScheme, bytes: &[u8]) -> Result<Self, CryptoError> {
        match scheme {
            SignatureScheme::Ed25519 => Ok(Self(SignatureInner::Ed25519(
                ed25519_dalek::Signature::try_from(bytes)?,
            ))),
            SignatureScheme::HybridEd25519MlDsa44V1 => {
                if bytes.len() != HYBRID_SIGNATURE_LEN {
                    return Err(CryptoError::InvalidLength {
                        scheme,
                        kind: "signature",
                        expected: HYBRID_SIGNATURE_LEN,
                        actual: bytes.len(),
                    });
                }
                let (ed, pq) = bytes.split_at(ED25519_SIGNATURE_LEN);
                Self::hybrid_from_parts(ed, pq)
            }
        }
    }

    /// Decode a [`SignatureScheme::HybridEd25519MlDsa44V1`] signature from its
    /// separate Ed25519 (`64` bytes) and ML-DSA-44 (`2420` bytes) components.
    pub fn hybrid_from_parts(ed25519: &[u8], mldsa44: &[u8]) -> Result<Self, CryptoError> {
        if ed25519.len() != ED25519_SIGNATURE_LEN {
            return Err(CryptoError::InvalidLength {
                scheme: SignatureScheme::HybridEd25519MlDsa44V1,
                kind: "Ed25519 signature",
                expected: ED25519_SIGNATURE_LEN,
                actual: ed25519.len(),
            });
        }
        if mldsa44.len() != MLDSA44_SIGNATURE_LEN {
            return Err(CryptoError::InvalidLength {
                scheme: SignatureScheme::HybridEd25519MlDsa44V1,
                kind: "ML-DSA-44 signature",
                expected: MLDSA44_SIGNATURE_LEN,
                actual: mldsa44.len(),
            });
        }

        let ed_sig = ed25519_dalek::Signature::try_from(ed25519)?;
        let pq_sig = ml_dsa::Signature::<ml_dsa::MlDsa44>::try_from(mldsa44).map_err(|_| {
            CryptoError::MlDsa44("invalid ML-DSA-44 signature encoding".to_string())
        })?;

        Ok(Self(SignatureInner::HybridEd25519MlDsa44V1 {
            ed25519: ed_sig,
            mldsa44: Box::new(pq_sig),
        }))
    }

    pub fn scheme(&self) -> SignatureScheme {
        match &self.0 {
            SignatureInner::Ed25519(_) => SignatureScheme::Ed25519,
            SignatureInner::HybridEd25519MlDsa44V1 { .. } => {
                SignatureScheme::HybridEd25519MlDsa44V1
            }
        }
    }

    /// Raw 64-byte Ed25519 signature (the whole signature for Ed25519, or the
    /// Ed25519 component for hybrid).
    pub fn ed25519_bytes(&self) -> [u8; ED25519_SIGNATURE_LEN] {
        match &self.0 {
            SignatureInner::Ed25519(signature) => signature.to_bytes(),
            SignatureInner::HybridEd25519MlDsa44V1 { ed25519, .. } => ed25519.to_bytes(),
        }
    }

    /// Raw 2,420-byte ML-DSA-44 signature, or `None` for Ed25519.
    pub fn mldsa44_bytes(&self) -> Option<Vec<u8>> {
        match &self.0 {
            SignatureInner::Ed25519(_) => None,
            SignatureInner::HybridEd25519MlDsa44V1 { mldsa44, .. } => {
                Some(mldsa44.encode().as_slice().to_vec())
            }
        }
    }

    /// Separate `(ed25519_sig, mldsa44_sig)` components, or `None` for Ed25519.
    pub fn hybrid_parts(&self) -> Option<([u8; ED25519_SIGNATURE_LEN], Vec<u8>)> {
        match &self.0 {
            SignatureInner::Ed25519(_) => None,
            SignatureInner::HybridEd25519MlDsa44V1 { ed25519, mldsa44 } => {
                Some((ed25519.to_bytes(), mldsa44.encode().as_slice().to_vec()))
            }
        }
    }

    /// Canonical encoding (`ed25519_sig || mldsa44_sig` for hybrid).
    pub fn to_bytes(&self) -> Vec<u8> {
        match &self.0 {
            SignatureInner::Ed25519(signature) => signature.to_bytes().to_vec(),
            SignatureInner::HybridEd25519MlDsa44V1 { ed25519, mldsa44 } => {
                let mut out = Vec::with_capacity(HYBRID_SIGNATURE_LEN);
                out.extend_from_slice(&ed25519.to_bytes());
                out.extend_from_slice(mldsa44.encode().as_slice());
                out
            }
        }
    }

    /// Lowercase hex of [`Signature::to_bytes`], without a `0x` prefix.
    pub fn to_hex(&self) -> String {
        hex::encode(self.to_bytes())
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({}:{})", self.scheme(), self.to_hex())
    }
}

#[derive(Clone)]
enum SigningKeyInner {
    // `ed25519_dalek::SigningKey` zeroizes its secret on drop.
    Ed25519(ed25519_dalek::SigningKey),
    // Both `ed25519_dalek::SigningKey` and `ml_dsa::SigningKey` (with the
    // `zeroize` feature enabled) zeroize their secret material on drop.
    HybridEd25519MlDsa44V1 {
        ed25519: ed25519_dalek::SigningKey,
        mldsa44: Box<ml_dsa::SigningKey<ml_dsa::MlDsa44>>,
    },
}

/// A secret signing key. `Debug` never prints secret material.
#[derive(Clone)]
pub struct SigningKey(SigningKeyInner);

impl SigningKey {
    /// Build a key from its secret seed. For Ed25519 this is
    /// `SigningKey::try_from(&[u8])`: exactly 32 bytes. For
    /// [`SignatureScheme::HybridEd25519MlDsa44V1`], `seed` must be 64 bytes
    /// (`ed25519_seed (32) || mldsa44_seed (32)`).
    pub fn from_seed(scheme: SignatureScheme, seed: &[u8]) -> Result<Self, CryptoError> {
        match scheme {
            SignatureScheme::Ed25519 => Ok(Self(SigningKeyInner::Ed25519(
                ed25519_dalek::SigningKey::try_from(seed)?,
            ))),
            SignatureScheme::HybridEd25519MlDsa44V1 => {
                if seed.len() != HYBRID_SEED_LEN {
                    return Err(CryptoError::InvalidLength {
                        scheme,
                        kind: "seed",
                        expected: HYBRID_SEED_LEN,
                        actual: seed.len(),
                    });
                }
                let mut ed_seed = zeroize::Zeroizing::new([0u8; ED25519_SEED_LEN]);
                let mut pq_seed = zeroize::Zeroizing::new([0u8; MLDSA44_SEED_LEN]);
                ed_seed.copy_from_slice(&seed[..ED25519_SEED_LEN]);
                pq_seed.copy_from_slice(&seed[ED25519_SEED_LEN..]);
                Ok(Self::hybrid_from_seeds(&ed_seed, &pq_seed))
            }
        }
    }

    /// Build an Ed25519 key from a 32-byte seed (`SigningKey::from_bytes`).
    pub fn from_ed25519_seed(seed: &[u8; 32]) -> Self {
        Self(SigningKeyInner::Ed25519(
            ed25519_dalek::SigningKey::from_bytes(seed),
        ))
    }

    /// Build a [`SignatureScheme::HybridEd25519MlDsa44V1`] key from two
    /// independently generated 32-byte seeds.
    pub fn hybrid_from_seeds(ed25519_seed: &[u8; 32], mldsa44_seed: &[u8; 32]) -> Self {
        let ed25519 = ed25519_dalek::SigningKey::from_bytes(ed25519_seed);
        let pq_seed = zeroize::Zeroizing::new(ml_dsa::Seed::from(*mldsa44_seed));
        let mldsa44 = ml_dsa::SigningKey::<ml_dsa::MlDsa44>::from_seed(&pq_seed);
        Self(SigningKeyInner::HybridEd25519MlDsa44V1 {
            ed25519,
            mldsa44: Box::new(mldsa44),
        })
    }

    /// Generate a fresh key from a cryptographically secure RNG.
    ///
    /// For [`SignatureScheme::HybridEd25519MlDsa44V1`], draws two independent
    /// 32-byte seeds from `rng` (ADR-017 §4).
    pub fn generate<R>(scheme: SignatureScheme, rng: &mut R) -> Self
    where
        R: rand::RngCore + rand::CryptoRng,
    {
        match scheme {
            SignatureScheme::Ed25519 => Self(SigningKeyInner::Ed25519(
                ed25519_dalek::SigningKey::generate(rng),
            )),
            SignatureScheme::HybridEd25519MlDsa44V1 => {
                let mut ed_seed = zeroize::Zeroizing::new([0u8; ED25519_SEED_LEN]);
                let mut pq_seed = zeroize::Zeroizing::new([0u8; MLDSA44_SEED_LEN]);
                rng.fill_bytes(&mut *ed_seed);
                rng.fill_bytes(&mut *pq_seed);
                Self::hybrid_from_seeds(&ed_seed, &pq_seed)
            }
        }
    }

    pub fn scheme(&self) -> SignatureScheme {
        match &self.0 {
            SigningKeyInner::Ed25519(_) => SignatureScheme::Ed25519,
            SigningKeyInner::HybridEd25519MlDsa44V1 { .. } => {
                SignatureScheme::HybridEd25519MlDsa44V1
            }
        }
    }

    pub fn public_key(&self) -> PublicKey {
        match &self.0 {
            SigningKeyInner::Ed25519(key) => {
                PublicKey(PublicKeyInner::Ed25519(key.verifying_key()))
            }
            SigningKeyInner::HybridEd25519MlDsa44V1 { ed25519, mldsa44 } => {
                let pq_vk: &ml_dsa::VerifyingKey<ml_dsa::MlDsa44> = mldsa44.as_ref().as_ref();
                PublicKey(PublicKeyInner::HybridEd25519MlDsa44V1 {
                    ed25519: ed25519.verifying_key(),
                    mldsa44: Box::new(pq_vk.clone()),
                })
            }
        }
    }

    /// Key identifier matching [`PublicKey::key_id`].
    pub fn key_id(&self) -> String {
        self.public_key().key_id()
    }

    /// Sign `message`.
    ///
    /// Ed25519 signing is deterministic (RFC 8032). Hybrid signing signs
    /// `M' = "creg-hybrid-sig-v1" || 0x00 || message` using Ed25519 and
    /// hedged (randomized) FIPS 204 ML-DSA-44 with context
    /// `"creg-hybrid-sig-v1"`.
    pub fn sign(&self, message: &[u8]) -> Signature {
        match &self.0 {
            SigningKeyInner::Ed25519(key) => {
                use ed25519_dalek::Signer;
                Signature(SignatureInner::Ed25519(key.sign(message)))
            }
            SigningKeyInner::HybridEd25519MlDsa44V1 { ed25519, mldsa44 } => {
                use ed25519_dalek::Signer;
                let m_prime = hybrid_message(message);
                let ed_sig = ed25519.sign(&m_prime);
                let pq_sig = mldsa44
                    .expanded_key()
                    .sign_randomized(&m_prime, HYBRID_MLDSA_CONTEXT, &mut OsRngBridge)
                    .expect("OS RNG and ML-DSA-44 hedged signing succeed");
                Signature(SignatureInner::HybridEd25519MlDsa44V1 {
                    ed25519: ed_sig,
                    mldsa44: Box::new(pq_sig),
                })
            }
        }
    }

    /// Deterministic signing path for NIST ACVP and golden-vector tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn sign_deterministic(&self, message: &[u8]) -> Signature {
        match &self.0 {
            SigningKeyInner::Ed25519(key) => {
                use ed25519_dalek::Signer;
                Signature(SignatureInner::Ed25519(key.sign(message)))
            }
            SigningKeyInner::HybridEd25519MlDsa44V1 { ed25519, mldsa44 } => {
                use ed25519_dalek::Signer;
                let m_prime = hybrid_message(message);
                let ed_sig = ed25519.sign(&m_prime);
                let pq_sig = mldsa44
                    .expanded_key()
                    .sign_deterministic(&m_prime, HYBRID_MLDSA_CONTEXT)
                    .expect("ML-DSA-44 deterministic signing succeeds");
                Signature(SignatureInner::HybridEd25519MlDsa44V1 {
                    ed25519: ed_sig,
                    mldsa44: Box::new(pq_sig),
                })
            }
        }
    }

    /// The 32-byte Ed25519 secret seed. Treat the result as secret and do
    /// not log it. For [`SignatureScheme::HybridEd25519MlDsa44V1`], this returns
    /// its Ed25519 component, which the VRF keeps using (ADR-017 §2).
    pub fn ed25519_seed(&self) -> [u8; 32] {
        match &self.0 {
            SigningKeyInner::Ed25519(key) => key.to_bytes(),
            SigningKeyInner::HybridEd25519MlDsa44V1 { ed25519, .. } => ed25519.to_bytes(),
        }
    }

    /// The 32-byte ML-DSA-44 secret seed (`ξ`), wrapped in
    /// [`zeroize::Zeroizing`], or `None` for Ed25519. Treat as secret and do
    /// not log it.
    pub fn mldsa44_seed(&self) -> Option<zeroize::Zeroizing<[u8; MLDSA44_SEED_LEN]>> {
        match &self.0 {
            SigningKeyInner::Ed25519(_) => None,
            SigningKeyInner::HybridEd25519MlDsa44V1 { mldsa44, .. } => {
                let mut out = zeroize::Zeroizing::new([0u8; MLDSA44_SEED_LEN]);
                out.copy_from_slice(mldsa44.as_seed().as_slice());
                Some(out)
            }
        }
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            SigningKeyInner::Ed25519(_) => f
                .debug_struct("SigningKey")
                .field("scheme", &self.scheme())
                .field("public_key", &self.public_key().to_hex())
                .finish_non_exhaustive(),
            SigningKeyInner::HybridEd25519MlDsa44V1 { .. } => f
                .debug_struct("SigningKey")
                .field("scheme", &self.scheme().id())
                .field("key_id", &self.public_key().key_id())
                .finish_non_exhaustive(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ED: SignatureScheme = SignatureScheme::Ed25519;

    fn unhex(value: &str) -> Vec<u8> {
        hex::decode(value).unwrap()
    }

    /// RFC 8032 section 7.1, TEST 1 (empty message).
    #[test]
    fn rfc8032_test_vector_1() {
        let seed = unhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let key = SigningKey::from_seed(ED, &seed).unwrap();
        assert_eq!(
            key.public_key().to_hex(),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        let signature = key.sign(b"");
        assert_eq!(
            signature.to_hex(),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
        for mode in [VerifyMode::Strict, VerifyMode::Permissive] {
            key.public_key().verify(b"", &signature, mode).unwrap();
            assert!(key.public_key().verify(b"x", &signature, mode).is_err());
        }
    }

    /// The module must produce exactly what direct `ed25519-dalek` calls
    /// produced before ADR-017, and accept or reject exactly the same inputs.
    #[test]
    fn matches_direct_ed25519_dalek_calls() {
        use ed25519_dalek::{Signer, Verifier};

        for seed_byte in [0u8, 1, 7, 42, 255] {
            let seed = [seed_byte; 32];
            let ours = SigningKey::from_ed25519_seed(&seed);
            let theirs = ed25519_dalek::SigningKey::from_bytes(&seed);
            assert_eq!(
                ours.public_key().to_bytes(),
                theirs.verifying_key().as_bytes()
            );
            assert_eq!(ours.ed25519_seed(), theirs.to_bytes());
            assert_eq!(
                SigningKey::from_seed(ED, &seed).unwrap().public_key(),
                ours.public_key()
            );

            let messages: [&[u8]; 3] = [b"", b"creg-pbft-v1:commit:abc", &[0u8; 300]];
            for message in messages {
                let signature = ours.sign(message);
                assert_eq!(signature.to_bytes(), theirs.sign(message).to_bytes());

                let parsed_key = PublicKey::from_bytes(ED, &ours.public_key().to_bytes()).unwrap();
                let parsed_signature = Signature::from_bytes(ED, &signature.to_bytes()).unwrap();
                let dalek_key = theirs.verifying_key();
                let dalek_signature =
                    ed25519_dalek::Signature::from_slice(&signature.to_bytes()).unwrap();

                let mut tampered = signature.to_bytes();
                tampered[10] ^= 1;
                let tampered_ours = Signature::from_bytes(ED, &tampered).unwrap();
                let tampered_dalek = ed25519_dalek::Signature::from_slice(&tampered).unwrap();

                assert_eq!(
                    parsed_key
                        .verify(message, &parsed_signature, VerifyMode::Strict)
                        .is_ok(),
                    dalek_key.verify_strict(message, &dalek_signature).is_ok()
                );
                assert_eq!(
                    parsed_key
                        .verify(message, &parsed_signature, VerifyMode::Permissive)
                        .is_ok(),
                    dalek_key.verify(message, &dalek_signature).is_ok()
                );
                assert_eq!(
                    parsed_key
                        .verify(message, &tampered_ours, VerifyMode::Strict)
                        .is_ok(),
                    dalek_key.verify_strict(message, &tampered_dalek).is_ok()
                );
                assert_eq!(
                    parsed_key
                        .verify(message, &tampered_ours, VerifyMode::Permissive)
                        .is_ok(),
                    dalek_key.verify(message, &tampered_dalek).is_ok()
                );
            }
        }
    }

    /// Why each call site keeps its mode: with a small-order public key (the
    /// identity point) and the all-identity signature, permissive
    /// verification accepts any message and strict verification rejects it.
    #[test]
    fn strict_and_permissive_modes_differ_on_small_order_keys() {
        let mut identity = [0u8; 32];
        identity[0] = 1;
        let key = PublicKey::from_bytes(ED, &identity).expect("the identity point decodes");
        let mut signature_bytes = [0u8; 64];
        signature_bytes[0] = 1; // R = identity, S = 0
        let signature = Signature::from_bytes(ED, &signature_bytes).unwrap();

        assert!(key
            .verify(b"any message", &signature, VerifyMode::Permissive)
            .is_ok());
        assert!(key
            .verify(b"any message", &signature, VerifyMode::Strict)
            .is_err());
    }

    #[test]
    fn decoding_rejects_wrong_lengths_with_the_library_message() {
        for length in [0usize, 31, 33, 64] {
            let error = PublicKey::from_bytes(ED, &vec![1u8; length]).unwrap_err();
            let dalek =
                ed25519_dalek::VerifyingKey::try_from(vec![1u8; length].as_slice()).unwrap_err();
            assert_eq!(error.to_string(), dalek.to_string());
        }
        for length in [0usize, 63, 65] {
            assert!(Signature::from_bytes(ED, &vec![0u8; length]).is_err());
        }
        for length in [0usize, 31, 33, 64] {
            assert!(SigningKey::from_seed(ED, &vec![0u8; length]).is_err());
        }
    }

    #[test]
    fn scheme_identifiers_and_lengths() {
        assert_eq!(ED.id(), "ed25519");
        assert_eq!(ED.to_string(), "ed25519");
        assert_eq!(SignatureScheme::from_id("ed25519"), Some(ED));
        assert_eq!(SignatureScheme::from_id("Ed25519"), None);
        assert_eq!(SignatureScheme::from_id(""), None);
        assert_eq!(
            (ED.public_key_len(), ED.signature_len(), ED.seed_len()),
            (32, 64, 32)
        );
    }

    #[test]
    fn generated_keys_sign_and_verify() {
        let key = SigningKey::generate(ED, &mut rand::rngs::OsRng);
        assert_eq!(key.scheme(), ED);
        let signature = key.sign(b"message");
        key.public_key()
            .verify(b"message", &signature, VerifyMode::Strict)
            .unwrap();
    }

    #[test]
    fn debug_output_never_contains_the_secret() {
        let seed = [0x5au8; 32];
        let key = SigningKey::from_ed25519_seed(&seed);
        let debug = format!("{key:?}");
        assert!(!debug.contains(&hex::encode(seed)));
        assert!(debug.contains(&key.public_key().to_hex()));
    }

    const HYBRID: SignatureScheme = SignatureScheme::HybridEd25519MlDsa44V1;

    #[test]
    fn hybrid_scheme_identifiers_lengths_roundtrips_and_key_id() {
        assert_eq!(HYBRID.id(), "hybrid-ed25519-mldsa44-v1");
        assert_eq!(HYBRID.to_string(), "hybrid-ed25519-mldsa44-v1");
        assert_eq!(
            SignatureScheme::from_id("hybrid-ed25519-mldsa44-v1"),
            Some(HYBRID)
        );
        assert_eq!(SignatureScheme::from_id("Hybrid-Ed25519-MlDsa44-V1"), None);
        assert_eq!(
            (
                HYBRID.public_key_len(),
                HYBRID.signature_len(),
                HYBRID.seed_len()
            ),
            (1344, 2484, 64)
        );

        let ed_seed = [0x11u8; 32];
        let pq_seed = [0x22u8; 32];
        let mut concat_seed = [0u8; 64];
        concat_seed[..32].copy_from_slice(&ed_seed);
        concat_seed[32..].copy_from_slice(&pq_seed);

        let sk_parts = SigningKey::hybrid_from_seeds(&ed_seed, &pq_seed);
        let sk_concat = SigningKey::from_seed(HYBRID, &concat_seed).unwrap();
        assert_eq!(sk_parts.scheme(), HYBRID);
        assert_eq!(sk_parts.public_key(), sk_concat.public_key());
        assert_eq!(sk_parts.ed25519_seed(), ed_seed);
        assert_eq!(*sk_parts.mldsa44_seed().unwrap(), pq_seed);

        let pk = sk_parts.public_key();
        assert_eq!(pk.scheme(), HYBRID);
        let pk_bytes = pk.to_bytes();
        assert_eq!(pk_bytes.len(), 1344);
        assert_eq!(PublicKey::from_bytes(HYBRID, &pk_bytes).unwrap(), pk);

        let (pk_ed, pk_pq) = pk.hybrid_parts().unwrap();
        assert_eq!(pk_ed, pk.ed25519_bytes());
        assert_eq!(pk_pq, pk.mldsa44_bytes().unwrap());
        assert_eq!(pk_pq.len(), 1312);
        assert_eq!(PublicKey::hybrid_from_parts(&pk_ed, &pk_pq).unwrap(), pk);

        let mut expected_id_input = Vec::from(b"creg-key-id-v1\x00".as_slice());
        expected_id_input.extend_from_slice(&pk_ed);
        expected_id_input.extend_from_slice(&pk_pq);
        let expected_key_id = hex::encode(Sha256::digest(&expected_id_input));
        assert_eq!(pk.key_id(), expected_key_id);
        assert_eq!(sk_parts.key_id(), expected_key_id);

        let sig = sk_parts.sign(b"roundtrip-payload");
        assert_eq!(sig.scheme(), HYBRID);
        let sig_bytes = sig.to_bytes();
        assert_eq!(sig_bytes.len(), 2484);
        assert_eq!(Signature::from_bytes(HYBRID, &sig_bytes).unwrap(), sig);
        let (sig_ed, sig_pq) = sig.hybrid_parts().unwrap();
        assert_eq!(sig_ed, sig.ed25519_bytes());
        assert_eq!(sig_pq, sig.mldsa44_bytes().unwrap());
        assert_eq!(sig_pq.len(), 2420);
        assert_eq!(Signature::hybrid_from_parts(&sig_ed, &sig_pq).unwrap(), sig);

        // Plain Ed25519 returns None for ML-DSA-44 accessors.
        let ed_sk = SigningKey::from_ed25519_seed(&ed_seed);
        let ed_pk = ed_sk.public_key();
        let ed_sig = ed_sk.sign(b"plain");
        assert!(ed_sk.mldsa44_seed().is_none());
        assert!(ed_pk.mldsa44_bytes().is_none());
        assert!(ed_pk.hybrid_parts().is_none());
        assert!(ed_sig.mldsa44_bytes().is_none());
        assert!(ed_sig.hybrid_parts().is_none());
    }

    #[test]
    fn hybrid_malformed_corpus_rejected() {
        let sk = SigningKey::hybrid_from_seeds(&[0x31u8; 32], &[0x42u8; 32]);
        let pk = sk.public_key();
        let (pk_ed, pk_pq) = pk.hybrid_parts().unwrap();
        let msg = b"malformed-corpus-message";
        let sig = sk.sign_deterministic(msg);
        let (sig_ed, sig_pq) = sig.hybrid_parts().unwrap();

        // Wrong lengths rejected before any parsing.
        for len in [0usize, 32, 1312, 1343, 1345] {
            let err = PublicKey::from_bytes(HYBRID, &vec![0u8; len]).unwrap_err();
            assert!(matches!(err, CryptoError::InvalidLength { .. }));
        }
        assert!(matches!(
            PublicKey::hybrid_from_parts(&pk_ed[..31], &pk_pq).unwrap_err(),
            CryptoError::InvalidLength { .. }
        ));
        assert!(matches!(
            PublicKey::hybrid_from_parts(&pk_ed, &pk_pq[..1311]).unwrap_err(),
            CryptoError::InvalidLength { .. }
        ));

        for len in [0usize, 64, 2420, 2483, 2485] {
            let err = Signature::from_bytes(HYBRID, &vec![0u8; len]).unwrap_err();
            assert!(matches!(err, CryptoError::InvalidLength { .. }));
        }
        assert!(matches!(
            Signature::hybrid_from_parts(&sig_ed[..63], &sig_pq).unwrap_err(),
            CryptoError::InvalidLength { .. }
        ));
        assert!(matches!(
            Signature::hybrid_from_parts(&sig_ed, &sig_pq[..2419]).unwrap_err(),
            CryptoError::InvalidLength { .. }
        ));

        for len in [0usize, 32, 63, 65] {
            let err = SigningKey::from_seed(HYBRID, &vec![0u8; len]).unwrap_err();
            assert!(matches!(err, CryptoError::InvalidLength { .. }));
        }

        // ML-DSA-44 signature layout: c_tilde[0..32], z[32..2336], hint[2336..2420]
        // where hint = indices[0..80] || cuts[80..84].
        const HINT_OFF: usize = 2336;
        const CUTS_OFF: usize = 2336 + 80;

        // 1. CVE-2026-24850: repeated hint indices in the same polynomial.
        let mut repeated_hint = sig_pq.clone();
        repeated_hint[HINT_OFF..].fill(0);
        repeated_hint[HINT_OFF] = 5;
        repeated_hint[HINT_OFF + 1] = 5;
        repeated_hint[CUTS_OFF..].copy_from_slice(&[2, 2, 2, 2]);
        assert!(matches!(
            Signature::hybrid_from_parts(&sig_ed, &repeated_hint).unwrap_err(),
            CryptoError::MlDsa44(_)
        ));

        // 2. Out-of-order hint indices.
        let mut unordered_hint = sig_pq.clone();
        unordered_hint[HINT_OFF..].fill(0);
        unordered_hint[HINT_OFF] = 10;
        unordered_hint[HINT_OFF + 1] = 5;
        unordered_hint[CUTS_OFF..].copy_from_slice(&[2, 2, 2, 2]);
        assert!(matches!(
            Signature::hybrid_from_parts(&sig_ed, &unordered_hint).unwrap_err(),
            CryptoError::MlDsa44(_)
        ));

        // 3. Non-zero padding after the last cut.
        let mut nonzero_pad_hint = sig_pq.clone();
        nonzero_pad_hint[HINT_OFF..].fill(0);
        nonzero_pad_hint[HINT_OFF] = 5;
        nonzero_pad_hint[HINT_OFF + 1] = 1;
        nonzero_pad_hint[CUTS_OFF..].copy_from_slice(&[1, 1, 1, 1]);
        assert!(matches!(
            Signature::hybrid_from_parts(&sig_ed, &nonzero_pad_hint).unwrap_err(),
            CryptoError::MlDsa44(_)
        ));

        // 4. Non-monotonic cuts and cut exceeding omega (80).
        let mut non_mono_cuts = sig_pq.clone();
        non_mono_cuts[HINT_OFF..].fill(0);
        non_mono_cuts[HINT_OFF] = 1;
        non_mono_cuts[HINT_OFF + 1] = 2;
        non_mono_cuts[CUTS_OFF..].copy_from_slice(&[2, 1, 2, 2]);
        assert!(matches!(
            Signature::hybrid_from_parts(&sig_ed, &non_mono_cuts).unwrap_err(),
            CryptoError::MlDsa44(_)
        ));

        let mut over_omega_cuts = sig_pq.clone();
        over_omega_cuts[HINT_OFF..].fill(0);
        over_omega_cuts[CUTS_OFF..].copy_from_slice(&[81, 81, 81, 81]);
        assert!(matches!(
            Signature::hybrid_from_parts(&sig_ed, &over_omega_cuts).unwrap_err(),
            CryptoError::MlDsa44(_)
        ));

        // 5. Out-of-range z: zeroing the first 18-bit packed coefficient sets z_0 = gamma1 >= gamma1 - beta.
        let mut out_of_range_z = sig_pq.clone();
        out_of_range_z[32] = 0;
        out_of_range_z[33] = 0;
        out_of_range_z[34] &= 0b1111_1100;
        assert!(matches!(
            Signature::hybrid_from_parts(&sig_ed, &out_of_range_z).unwrap_err(),
            CryptoError::MlDsa44(_)
        ));

        // 6. Wrong context string and empty context where "creg-hybrid-sig-v1" is required.
        let pq_sk = ml_dsa::SigningKey::<ml_dsa::MlDsa44>::from_seed(&ml_dsa::Seed::from(
            *sk.mldsa44_seed().unwrap(),
        ));
        let m_prime = hybrid_message(msg);
        for bad_ctx in [b"wrong-context".as_slice(), b"".as_slice()] {
            let bad_pq_sig = pq_sk
                .expanded_key()
                .sign_deterministic(&m_prime, bad_ctx)
                .unwrap();
            let bad_sig =
                Signature::hybrid_from_parts(&sig_ed, bad_pq_sig.encode().as_slice()).unwrap();
            assert!(matches!(
                pk.verify(msg, &bad_sig, VerifyMode::Strict).unwrap_err(),
                CryptoError::MlDsa44(_)
            ));
        }
    }

    #[test]
    fn hybrid_non_separability() {
        let sk1 = SigningKey::hybrid_from_seeds(&[0x01u8; 32], &[0x02u8; 32]);
        let sk2 = SigningKey::hybrid_from_seeds(&[0x03u8; 32], &[0x04u8; 32]);
        let pk1 = sk1.public_key();
        let pk2 = sk2.public_key();

        let msg1 = b"creg-pbft-v1:commit:block-1";
        let msg2 = b"creg-pbft-v1:commit:block-2";
        let sig1 = sk1.sign_deterministic(msg1);
        let sig2 = sk1.sign_deterministic(msg2);
        let (ed1, pq1) = sig1.hybrid_parts().unwrap();
        let (ed2, pq2) = sig2.hybrid_parts().unwrap();

        // Stripped halves cannot decode as hybrid and plain Ed25519 signature cannot verify under hybrid key.
        assert!(matches!(
            Signature::from_bytes(HYBRID, &ed1).unwrap_err(),
            CryptoError::InvalidLength { .. }
        ));
        assert!(matches!(
            Signature::from_bytes(HYBRID, &pq1).unwrap_err(),
            CryptoError::InvalidLength { .. }
        ));
        let plain_ed_sk = SigningKey::from_ed25519_seed(&sk1.ed25519_seed());
        let plain_ed_pk = plain_ed_sk.public_key();
        let plain_ed_sig = plain_ed_sk.sign(msg1);
        assert!(matches!(
            pk1.verify(msg1, &plain_ed_sig, VerifyMode::Strict)
                .unwrap_err(),
            CryptoError::SchemeMismatch { .. }
        ));

        // Flipping a bit in either half breaks verification.
        let mut flipped_ed = ed1;
        flipped_ed[0] ^= 1;
        let bad_ed = Signature::hybrid_from_parts(&flipped_ed, &pq1).unwrap();
        assert!(pk1.verify(msg1, &bad_ed, VerifyMode::Strict).is_err());

        let mut flipped_pq = pq1.clone();
        flipped_pq[0] ^= 1;
        let bad_pq = Signature::hybrid_from_parts(&ed1, &flipped_pq).unwrap();
        assert!(pk1.verify(msg1, &bad_pq, VerifyMode::Strict).is_err());

        // Swapped components between two messages fail for both messages.
        let swapped_1_2 = Signature::hybrid_from_parts(&ed1, &pq2).unwrap();
        let swapped_2_1 = Signature::hybrid_from_parts(&ed2, &pq1).unwrap();
        for msg in [msg1.as_slice(), msg2.as_slice()] {
            assert!(pk1.verify(msg, &swapped_1_2, VerifyMode::Strict).is_err());
            assert!(pk1.verify(msg, &swapped_2_1, VerifyMode::Strict).is_err());
        }

        // Hybrid Ed25519 component must not verify as a plain Ed25519 signature over M,
        // only over M' = "creg-hybrid-sig-v1" || 0x00 || M.
        let hybrid_ed_as_plain = Signature::from_bytes(ED, &ed1).unwrap();
        for mode in [VerifyMode::Strict, VerifyMode::Permissive] {
            assert!(plain_ed_pk.verify(msg1, &hybrid_ed_as_plain, mode).is_err());
            plain_ed_pk
                .verify(&hybrid_message(msg1), &hybrid_ed_as_plain, mode)
                .unwrap();
        }

        // Conversely, a plain Ed25519 signature over M paired with the valid ML-DSA half over M' fails.
        let downgrade_mix = Signature::hybrid_from_parts(&plain_ed_sig.to_bytes(), &pq1).unwrap();
        assert!(pk1
            .verify(msg1, &downgrade_mix, VerifyMode::Strict)
            .is_err());

        // Substituting either key component from another signer fails.
        let (pk1_ed, pk1_pq) = pk1.hybrid_parts().unwrap();
        let (pk2_ed, pk2_pq) = pk2.hybrid_parts().unwrap();
        let mixed_pk_a = PublicKey::hybrid_from_parts(&pk1_ed, &pk2_pq).unwrap();
        let mixed_pk_b = PublicKey::hybrid_from_parts(&pk2_ed, &pk1_pq).unwrap();
        assert!(mixed_pk_a.verify(msg1, &sig1, VerifyMode::Strict).is_err());
        assert!(mixed_pk_b.verify(msg1, &sig1, VerifyMode::Strict).is_err());
    }

    #[test]
    fn hybrid_always_enforces_strict_ed25519_verification() {
        let mut identity = [0u8; 32];
        identity[0] = 1;
        let mut forged_ed_sig = [0u8; 64];
        forged_ed_sig[0] = 1; // R = identity, S = 0

        let pq_seed = [0x55u8; 32];
        let sk_valid = SigningKey::hybrid_from_seeds(&[0x12u8; 32], &pq_seed);
        let pq_pk = sk_valid.public_key().mldsa44_bytes().unwrap();

        let msg = b"strict-only-hybrid-check";
        let genuine_sig = sk_valid.sign_deterministic(msg);
        let pq_sig = genuine_sig.mldsa44_bytes().unwrap();

        let weak_hybrid_pk = PublicKey::hybrid_from_parts(&identity, &pq_pk).unwrap();
        let forged_hybrid_sig = Signature::hybrid_from_parts(&forged_ed_sig, &pq_sig).unwrap();

        assert!(weak_hybrid_pk
            .verify(msg, &forged_hybrid_sig, VerifyMode::Strict)
            .is_err());
        assert!(weak_hybrid_pk
            .verify(msg, &forged_hybrid_sig, VerifyMode::Permissive)
            .is_err());
    }

    #[test]
    fn hybrid_seed_independence() {
        let sk1 = SigningKey::generate(HYBRID, &mut rand::rngs::OsRng);
        let sk2 = SigningKey::generate(HYBRID, &mut rand::rngs::OsRng);

        let ed1 = sk1.ed25519_seed();
        let pq1 = sk1.mldsa44_seed().unwrap();
        let ed2 = sk2.ed25519_seed();
        let pq2 = sk2.mldsa44_seed().unwrap();

        assert_ne!(ed1, ed2);
        assert_ne!(*pq1, *pq2);
        assert_ne!(ed1, *pq1);
        assert_ne!(ed2, *pq2);

        // Changing only the ML-DSA seed leaves the Ed25519 public key unchanged and changes key_id.
        let fixed_ed = [0x77u8; 32];
        let a = SigningKey::hybrid_from_seeds(&fixed_ed, &[0x01u8; 32]);
        let b = SigningKey::hybrid_from_seeds(&fixed_ed, &[0x02u8; 32]);
        assert_eq!(a.ed25519_seed(), fixed_ed);
        assert_eq!(*a.mldsa44_seed().unwrap(), [0x01u8; 32]);
        assert_eq!(*b.mldsa44_seed().unwrap(), [0x02u8; 32]);
        assert_eq!(
            a.public_key().ed25519_bytes(),
            b.public_key().ed25519_bytes()
        );
        assert_ne!(
            a.public_key().mldsa44_bytes().unwrap(),
            b.public_key().mldsa44_bytes().unwrap()
        );
        assert_ne!(a.key_id(), b.key_id());
    }

    #[test]
    fn hybrid_hedged_differs_and_deterministic_is_stable() {
        let sk = SigningKey::hybrid_from_seeds(&[0x99u8; 32], &[0xaau8; 32]);
        let pk = sk.public_key();
        let msg = b"hedged-vs-deterministic";

        let h1 = sk.sign(msg);
        let h2 = sk.sign(msg);
        assert_eq!(h1.ed25519_bytes(), h2.ed25519_bytes());
        assert_ne!(h1.mldsa44_bytes().unwrap(), h2.mldsa44_bytes().unwrap());
        pk.verify(msg, &h1, VerifyMode::Strict).unwrap();
        pk.verify(msg, &h2, VerifyMode::Strict).unwrap();

        let d1 = sk.sign_deterministic(msg);
        let d2 = sk.sign_deterministic(msg);
        assert_eq!(d1.to_bytes(), d2.to_bytes());
        pk.verify(msg, &d1, VerifyMode::Strict).unwrap();
    }

    #[test]
    fn hybrid_debug_output_never_contains_secrets_and_shows_scheme_and_key_id() {
        let ed_seed = [0x5au8; 32];
        let pq_seed = [0xa5u8; 32];
        let sk = SigningKey::hybrid_from_seeds(&ed_seed, &pq_seed);
        let pk = sk.public_key();
        let key_id = pk.key_id();

        let sk_debug = format!("{sk:?}");
        let pk_debug = format!("{pk:?}");
        for s in [&sk_debug, &pk_debug] {
            assert!(!s.contains(&hex::encode(ed_seed)));
            assert!(!s.contains(&hex::encode(pq_seed)));
            assert!(s.contains("hybrid-ed25519-mldsa44-v1"));
            assert!(s.contains(&key_id));
        }
    }
}
