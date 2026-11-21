// crates/common/src/crypto.rs
//! Signature schemes behind one interface (ADR-017, increment 1).
//!
//! Every CREG-native signature is created and checked through this module,
//! so the hybrid Ed25519 + ML-DSA-44 scheme can later be added in one place.
//! Today the only scheme is Ed25519, and each function calls exactly the
//! `ed25519-dalek` routine that its call sites used before this module
//! existed. Signatures, public keys and verification results are therefore
//! byte-for-byte unchanged.
//!
//! Hex parsing, length checks with custom wording and error messages stay at
//! the call sites: they differ per protocol message, and some are visible to
//! API clients.
//!
//! [`SignatureScheme`] is deliberately not `#[non_exhaustive]`: adding the
//! hybrid scheme must break every `match` in the workspace, so each call site
//! is revisited.

use std::fmt;

/// A signature scheme CREG can sign and verify with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignatureScheme {
    /// RFC 8032 Ed25519, implemented by `ed25519-dalek`.
    Ed25519,
}

impl SignatureScheme {
    /// Stable identifier, as written in the chain spec's `signature_algorithm`.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Ed25519 => "ed25519",
        }
    }

    /// Parse an identifier produced by [`SignatureScheme::id`]. Exact match only.
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "ed25519" => Some(Self::Ed25519),
            _ => None,
        }
    }

    /// Encoded public-key length in bytes.
    pub const fn public_key_len(self) -> usize {
        match self {
            Self::Ed25519 => ed25519_dalek::PUBLIC_KEY_LENGTH,
        }
    }

    /// Encoded signature length in bytes.
    pub const fn signature_len(self) -> usize {
        match self {
            Self::Ed25519 => ed25519_dalek::SIGNATURE_LENGTH,
        }
    }

    /// Secret seed length in bytes.
    pub const fn seed_len(self) -> usize {
        match self {
            Self::Ed25519 => ed25519_dalek::SECRET_KEY_LENGTH,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    /// `VerifyingKey::verify_strict`: additionally rejects small-order public
    /// keys and signature points. Honest signers never produce them.
    Strict,
    /// `Verifier::verify`: the cofactorless RFC 8032 check as `ed25519-dalek`
    /// implements it. Accepts small-order keys.
    Permissive,
}

/// A key, signature or verification rejected by the underlying library.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// Rejected by `ed25519-dalek`. Displays the library's own message, so
    /// call sites that format the error produce the same text as before.
    #[error(transparent)]
    Ed25519(#[from] ed25519_dalek::SignatureError),
}

#[derive(Clone, PartialEq, Eq)]
enum PublicKeyInner {
    Ed25519(ed25519_dalek::VerifyingKey),
}

/// A public verification key.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicKey(PublicKeyInner);

impl PublicKey {
    /// Decode a public key. For Ed25519 this is
    /// `VerifyingKey::try_from(&[u8])`: exactly 32 bytes that decompress to a
    /// curve point. Small-order points are accepted here; strict verification
    /// rejects them later.
    pub fn from_bytes(scheme: SignatureScheme, bytes: &[u8]) -> Result<Self, CryptoError> {
        match scheme {
            SignatureScheme::Ed25519 => Ok(Self(PublicKeyInner::Ed25519(
                ed25519_dalek::VerifyingKey::try_from(bytes)?,
            ))),
        }
    }

    pub fn scheme(&self) -> SignatureScheme {
        match &self.0 {
            PublicKeyInner::Ed25519(_) => SignatureScheme::Ed25519,
        }
    }

    /// Canonical encoding.
    pub fn to_bytes(&self) -> Vec<u8> {
        match &self.0 {
            PublicKeyInner::Ed25519(key) => key.as_bytes().to_vec(),
        }
    }

    /// Lowercase hex of [`PublicKey::to_bytes`], without a `0x` prefix.
    pub fn to_hex(&self) -> String {
        hex::encode(self.to_bytes())
    }

    /// Check `signature` over `message`.
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
        }
        Ok(())
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({}:{})", self.scheme(), self.to_hex())
    }
}

#[derive(Clone, PartialEq, Eq)]
enum SignatureInner {
    Ed25519(ed25519_dalek::Signature),
}

/// A detached signature.
#[derive(Clone, PartialEq, Eq)]
pub struct Signature(SignatureInner);

impl Signature {
    /// Decode a signature. For Ed25519 this is `Signature::try_from(&[u8])`,
    /// which checks only the length (64 bytes); the scalar and point are
    /// checked during verification.
    pub fn from_bytes(scheme: SignatureScheme, bytes: &[u8]) -> Result<Self, CryptoError> {
        match scheme {
            SignatureScheme::Ed25519 => Ok(Self(SignatureInner::Ed25519(
                ed25519_dalek::Signature::try_from(bytes)?,
            ))),
        }
    }

    pub fn scheme(&self) -> SignatureScheme {
        match &self.0 {
            SignatureInner::Ed25519(_) => SignatureScheme::Ed25519,
        }
    }

    /// Canonical encoding.
    pub fn to_bytes(&self) -> Vec<u8> {
        match &self.0 {
            SignatureInner::Ed25519(signature) => signature.to_bytes().to_vec(),
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
}

/// A secret signing key. `Debug` shows only the public key.
#[derive(Clone)]
pub struct SigningKey(SigningKeyInner);

impl SigningKey {
    /// Build a key from its secret seed. For Ed25519 this is
    /// `SigningKey::try_from(&[u8])`: exactly 32 bytes.
    pub fn from_seed(scheme: SignatureScheme, seed: &[u8]) -> Result<Self, CryptoError> {
        match scheme {
            SignatureScheme::Ed25519 => Ok(Self(SigningKeyInner::Ed25519(
                ed25519_dalek::SigningKey::try_from(seed)?,
            ))),
        }
    }

    /// Build an Ed25519 key from a 32-byte seed (`SigningKey::from_bytes`).
    pub fn from_ed25519_seed(seed: &[u8; 32]) -> Self {
        Self(SigningKeyInner::Ed25519(
            ed25519_dalek::SigningKey::from_bytes(seed),
        ))
    }

    /// Generate a fresh key from a cryptographically secure RNG.
    pub fn generate<R>(scheme: SignatureScheme, rng: &mut R) -> Self
    where
        R: rand::RngCore + rand::CryptoRng,
    {
        match scheme {
            SignatureScheme::Ed25519 => Self(SigningKeyInner::Ed25519(
                ed25519_dalek::SigningKey::generate(rng),
            )),
        }
    }

    pub fn scheme(&self) -> SignatureScheme {
        match &self.0 {
            SigningKeyInner::Ed25519(_) => SignatureScheme::Ed25519,
        }
    }

    pub fn public_key(&self) -> PublicKey {
        match &self.0 {
            SigningKeyInner::Ed25519(key) => {
                PublicKey(PublicKeyInner::Ed25519(key.verifying_key()))
            }
        }
    }

    /// Sign `message`. Ed25519 signing is deterministic (RFC 8032).
    pub fn sign(&self, message: &[u8]) -> Signature {
        match &self.0 {
            SigningKeyInner::Ed25519(key) => {
                use ed25519_dalek::Signer;
                Signature(SignatureInner::Ed25519(key.sign(message)))
            }
        }
    }

    /// The 32-byte Ed25519 secret seed. Treat the result as secret and do
    /// not log it. Once the hybrid scheme exists this returns its Ed25519
    /// component, which the VRF keeps using (ADR-017 section 2).
    pub fn ed25519_seed(&self) -> [u8; 32] {
        match &self.0 {
            SigningKeyInner::Ed25519(key) => key.to_bytes(),
        }
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningKey")
            .field("scheme", &self.scheme())
            .field("public_key", &self.public_key().to_hex())
            .finish_non_exhaustive()
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
}
