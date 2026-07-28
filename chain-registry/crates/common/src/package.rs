// crates/common/src/package.rs

use crate::{CertifiedNamespaceAuthority, NamespaceAuthorityProof, NamespaceAuthorityPurpose};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const PUBLISHER_AUTHORIZATION_SCHEMA_VERSION: u16 = 1;
/// Maximum lifetime of a proof-retaining publisher authorization.
pub const MAX_PUBLISHER_AUTHORIZATION_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FindingSeverity {
    /// Package must be rejected — direct evidence of malice.
    Critical,
    /// Strongly suspicious — requires human appeal to override.
    High,
    /// Notable but possibly legitimate — shown as warning.
    Medium,
    /// Informational only.
    #[default]
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub severity: FindingSeverity,
    pub description: String,
    pub file: String,
    pub line: Option<usize>,
}

/// Uniquely identifies a package across all ecosystems.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct PackageId {
    /// Ecosystem: "npm" | "pypi" | "cargo" | "rubygems" | "maven"
    pub ecosystem: String,
    pub name: String,
    pub version: String,
}

impl PackageId {
    pub fn new(
        ecosystem: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            ecosystem: ecosystem.into(),
            name: name.into(),
            version: version.into(),
        }
    }

    /// Canonical string used as a cache key and chain identifier.
    pub fn canonical(&self) -> String {
        format!("{}:{}@{}", self.ecosystem, self.name, self.version)
    }
}

pub fn canonical_publisher_address(publisher_address: &str) -> String {
    let trimmed = publisher_address.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let stripped = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    format!("0x{}", stripped.to_ascii_lowercase())
}

/// Canonical namespace prefix shared by every version of a package.
///
/// The trailing `@` is deliberate: it prevents a namespace such as
/// `npm:foo` from also matching `npm:foobar` in prefix-indexed stores.
pub fn package_namespace_prefix(id: &PackageId) -> String {
    format!("{}:{}@", id.ecosystem.trim(), id.name.trim())
}

/// Canonical npm/PyPI namespace used by authoritative ownership statements.
/// Legacy registry identifiers remain byte-for-byte decodable; this stricter
/// form is used only by the activation-gated authority protocol so aliases
/// cannot acquire independent controllers.
pub fn authoritative_package_namespace(id: &PackageId) -> Result<String, &'static str> {
    let ecosystem = id.ecosystem.trim().to_ascii_lowercase();
    if ecosystem != id.ecosystem {
        return Err("authoritative ecosystem must be canonical lowercase");
    }
    let name = canonical_authoritative_package_name(&ecosystem, &id.name)?;
    if name != id.name {
        return Err("authoritative package name is not canonical");
    }
    Ok(format!("{}:{}@", ecosystem, name))
}

/// Validate that a serialized namespace is the one canonical spelling for
/// its upstream ecosystem.
pub fn validate_authoritative_namespace(prefix: &str) -> Result<(), &'static str> {
    validate_namespace_prefix(prefix)?;
    let (ecosystem, marked_name) = prefix
        .split_once(':')
        .ok_or("namespace prefix must contain an ecosystem separator")?;
    let name = marked_name
        .strip_suffix('@')
        .ok_or("namespace prefix must end with '@'")?;
    let canonical = canonical_authoritative_package_name(ecosystem, name)?;
    if ecosystem != ecosystem.to_ascii_lowercase() || name != canonical {
        return Err("authoritative namespace is not canonical");
    }
    Ok(())
}

fn canonical_authoritative_package_name(
    ecosystem: &str,
    name: &str,
) -> Result<String, &'static str> {
    if name.is_empty() || name != name.trim() || name.chars().any(char::is_control) {
        return Err("authoritative package name is invalid");
    }
    match ecosystem {
        "pypi" => {
            let mut normalized = String::with_capacity(name.len());
            let mut separator = false;
            for byte in name.bytes() {
                if byte.is_ascii_alphanumeric() {
                    normalized.push((byte as char).to_ascii_lowercase());
                    separator = false;
                } else if matches!(byte, b'-' | b'_' | b'.') {
                    if !separator {
                        normalized.push('-');
                        separator = true;
                    }
                } else {
                    return Err("PyPI package name contains an unsupported character");
                }
            }
            if normalized.is_empty() || normalized.starts_with('-') || normalized.ends_with('-') {
                return Err("PyPI package name is invalid");
            }
            Ok(normalized)
        }
        "npm" => {
            if name != name.to_ascii_lowercase() {
                return Err("npm package name must be lowercase");
            }
            let valid_component = |component: &str| {
                !component.is_empty()
                    && component.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'-' | b'_' | b'.' | b'~')
                    })
            };
            if let Some(scoped) = name.strip_prefix('@') {
                let Some((scope, package)) = scoped.split_once('/') else {
                    return Err("scoped npm package name must contain one '/' separator");
                };
                if package.contains('/') || !valid_component(scope) || !valid_component(package) {
                    return Err("scoped npm package name is invalid");
                }
            } else if name.contains('/') || !valid_component(name) {
                return Err("npm package name is invalid");
            }
            Ok(name.to_string())
        }
        _ => Err("authoritative ownership supports only npm and PyPI namespaces"),
    }
}

pub fn is_canonical_single_line(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && !value
            .chars()
            .any(|ch| ch.is_control() || ch == '\u{2028}' || ch == '\u{2029}')
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value == value.to_ascii_lowercase()
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Validate the serialized form accepted by namespace-transfer and key-rotation
/// events. This deliberately accepts ecosystem-specific characters in names
/// (for example npm's `@scope/name`) while rejecting ambiguous, whitespace, or
/// version-qualified prefixes.
pub fn validate_namespace_prefix(prefix: &str) -> Result<(), &'static str> {
    let value = prefix.trim();
    if value != prefix || value.len() < 4 || !value.ends_with('@') {
        return Err("namespace prefix must be trimmed and end with '@'");
    }
    let Some((ecosystem, name_with_marker)) = value.split_once(':') else {
        return Err("namespace prefix must contain an ecosystem separator");
    };
    let Some(name) = name_with_marker.strip_suffix('@') else {
        return Err("namespace prefix must end with '@'");
    };
    if ecosystem.is_empty() || name.is_empty() {
        return Err("namespace ecosystem and name are required");
    }
    if ecosystem.chars().any(char::is_whitespace) || name.chars().any(char::is_whitespace) {
        return Err("namespace prefix must not contain whitespace");
    }
    Ok(())
}

/// Domain-separated authorization payload for a namespace-scoped publisher key
/// rotation. Both keys sign this exact payload, preventing signatures from one
/// namespace, nonce, or CREG deployment from being replayed for another.
pub fn publisher_rotation_message(
    canonical_prefix: &str,
    old_pubkey: &str,
    new_pubkey: &str,
    nonce: u64,
) -> String {
    format!(
        "creg-publisher-key-rotation-v1\nservice:creg-package-ownership\nnamespace:{}\nold_ed25519_pubkey:{}\nnew_ed25519_pubkey:{}\nnonce:{}",
        canonical_prefix.trim(),
        old_pubkey.trim().to_ascii_lowercase(),
        new_pubkey.trim().to_ascii_lowercase(),
        nonce,
    )
}

pub fn publish_signature_message(
    id: &PackageId,
    content_hash: &str,
    publisher_address: &str,
) -> String {
    format!(
        "{}{}{}",
        id.canonical(),
        content_hash,
        canonical_publisher_address(publisher_address)
    )
}

/// Domain-separated publisher authorization payload. The legacy helper above
/// remains available for decoding old records, but new admission should use
/// this version so signatures cannot be replayed across services, chains,
/// contracts, packages, or validity windows.
pub fn publish_signature_message_v2(
    chain_id: &str,
    staking_contract: &str,
    id: &PackageId,
    content_hash: &str,
    source_hash: &str,
    manifest_hash: &str,
    publisher_pubkey: &str,
    publisher_address: &str,
    ownership_claim: &str,
    nonce: &str,
    issued_at: i64,
    expires_at: u64,
) -> String {
    format!(
        "creg-publisher-admission-v2\nservice:creg-package-admission\nchain_id:{}\nstaking_contract:{}\necosystem:{}\nname:{}\nversion:{}\ncontent_sha256:{}\nsource_sha256:{}\nmanifest_sha256:{}\ned25519_pubkey:{}\nevm_address:{}\nownership_claim:{}\nnonce:{}\nissued_at:{}\nexpires_at:{}",
        chain_id.trim(),
        canonical_publisher_address(staking_contract),
        id.ecosystem.trim(),
        id.name.trim(),
        id.version.trim(),
        content_hash.trim().to_ascii_lowercase(),
        source_hash.trim().to_ascii_lowercase(),
        manifest_hash.trim().to_ascii_lowercase(),
        publisher_pubkey.trim().to_ascii_lowercase(),
        canonical_publisher_address(publisher_address),
        ownership_claim.trim(),
        nonce.trim(),
        issued_at,
        expires_at,
    )
}

/// Version 3 of the publisher authorization payload. It retains all v2
/// bindings and additionally commits to the exact registry-authority evidence
/// bundle supplied with the request.
pub fn publish_signature_message_v3(
    chain_id: &str,
    staking_contract: &str,
    id: &PackageId,
    content_hash: &str,
    source_hash: &str,
    manifest_hash: &str,
    publisher_pubkey: &str,
    publisher_address: &str,
    ownership_claim: &str,
    nonce: &str,
    issued_at: i64,
    expires_at: u64,
    authority_proof_digest: &str,
) -> String {
    format!(
        "{}\nownership_proof_sha256:{}",
        publish_signature_message_v2(
            chain_id,
            staking_contract,
            id,
            content_hash,
            source_hash,
            manifest_hash,
            publisher_pubkey,
            publisher_address,
            ownership_claim,
            nonce,
            issued_at,
            expires_at,
        ),
        authority_proof_digest.trim().to_ascii_lowercase(),
    )
}

/// Activation-gated publisher authorization. Besides the v3 package and
/// authority-proof bindings, v4 commits to retrieval/encryption metadata and
/// the complete multisig policy so a proposer cannot downgrade a threshold
/// request or substitute an IPFS/key bundle while reusing valid signatures.
#[allow(clippy::too_many_arguments)]
pub fn publish_signature_message_v4(
    chain_id: &str,
    genesis_hash: &str,
    staking_contract: &str,
    id: &PackageId,
    content_hash: &str,
    source_hash: &str,
    manifest_hash: &str,
    ipfs_cid: &str,
    shielded: bool,
    key_bundle_sha256: &str,
    pgp_signature_sha256: &str,
    pgp_public_key_sha256: &str,
    publisher_pubkey: &str,
    publisher_address: &str,
    ownership_claim: &str,
    nonce: &str,
    issued_at: i64,
    expires_at: u64,
    threshold: usize,
    publisher_pubkeys: &[String],
    authority_proof_digest: &str,
    authority_certificate_digest: &str,
) -> String {
    let keyset = canonical_publisher_keyset(publisher_pubkey, publisher_pubkeys).join(",");
    format!(
        "creg-publisher-admission-v4\nservice:creg-package-admission\nchain_id:{}\ngenesis_hash:{}\nstaking_contract:{}\necosystem:{}\nname:{}\nversion:{}\ncontent_sha256:{}\nsource_sha256:{}\nmanifest_sha256:{}\nipfs_cid:{}\nshielded:{}\nkey_bundle_sha256:{}\npgp_signature_sha256:{}\npgp_public_key_sha256:{}\ned25519_pubkey:{}\nevm_address:{}\nownership_claim:{}\nnonce:{}\nissued_at:{}\nexpires_at:{}\nthreshold:{}\npublisher_pubkeys:{}\nownership_proof_sha256:{}\nauthority_certificate_sha256:{}",
        chain_id.trim(),
        genesis_hash.trim().to_ascii_lowercase(),
        canonical_publisher_address(staking_contract),
        id.ecosystem.trim(),
        id.name.trim(),
        id.version.trim(),
        content_hash.trim().to_ascii_lowercase(),
        source_hash.trim().to_ascii_lowercase(),
        manifest_hash.trim().to_ascii_lowercase(),
        ipfs_cid.trim(),
        shielded,
        key_bundle_sha256.trim().to_ascii_lowercase(),
        pgp_signature_sha256.trim().to_ascii_lowercase(),
        pgp_public_key_sha256.trim().to_ascii_lowercase(),
        publisher_pubkey.trim().to_ascii_lowercase(),
        canonical_publisher_address(publisher_address),
        ownership_claim.trim(),
        nonce.trim(),
        issued_at,
        expires_at,
        threshold,
        keyset,
        authority_proof_digest.trim().to_ascii_lowercase(),
        authority_certificate_digest.trim().to_ascii_lowercase(),
    )
}

pub fn canonical_publisher_keyset(primary: &str, additional: &[String]) -> Vec<String> {
    let mut keys = if additional.is_empty() {
        vec![primary.trim().to_ascii_lowercase()]
    } else {
        additional
            .iter()
            .map(|key| key.trim().to_ascii_lowercase())
            .collect()
    };
    keys.sort();
    keys.dedup();
    keys
}

impl std::fmt::Display for PackageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.canonical())
    }
}

/// Declared package behaviors submitted alongside the tarball.
/// Validators check *against* this manifest rather than blanket policy —
/// an HTTP client that declares outbound HTTPS calls is legitimate.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackageManifest {
    /// Allowed outbound hosts, e.g. ["api.example.com"]
    pub allowed_network_hosts: Vec<String>,
    /// Allowed filesystem paths the package may write to.
    pub allowed_fs_writes: Vec<String>,
    /// Whether the package spawns child processes.
    pub spawns_processes: bool,
    /// Fine-grained process spawn allowlist — binary names or full paths.
    /// Only checked when `spawns_processes` is true.  If empty and
    /// `spawns_processes` is true, all spawns are permitted (backwards compat).
    #[serde(default)]
    pub allowed_process_spawns: Vec<String>,
    /// Free-text description for human reviewers.
    pub description: Option<String>,
}

#[cfg(test)]
mod publisher_message_tests {
    use super::{
        authoritative_package_namespace, package_namespace_prefix, publish_signature_message_v2,
        publish_signature_message_v3, publish_signature_message_v4, publisher_rotation_message,
        validate_authoritative_namespace, validate_namespace_prefix, PackageId,
    };

    #[allow(clippy::too_many_arguments)]
    fn v4_message(
        id: &PackageId,
        genesis_hash: &str,
        ipfs_cid: &str,
        shielded: bool,
        key_bundle_sha256: &str,
        pgp_signature_sha256: &str,
        threshold: usize,
        publisher_pubkeys: &[String],
        authority_proof_digest: &str,
        authority_certificate_digest: &str,
    ) -> String {
        publish_signature_message_v4(
            "creg-testnet-1",
            genesis_hash,
            "0x1111111111111111111111111111111111111111",
            id,
            &"1".repeat(64),
            &"2".repeat(64),
            &"3".repeat(64),
            ipfs_cid,
            shielded,
            key_bundle_sha256,
            pgp_signature_sha256,
            &"6".repeat(64),
            &"a".repeat(64),
            "0x2222222222222222222222222222222222222222",
            "npm:pkg",
            "nonce-a",
            1_700_000_000,
            1_700_000_600,
            threshold,
            publisher_pubkeys,
            authority_proof_digest,
            authority_certificate_digest,
        )
    }

    #[test]
    fn publisher_payload_is_domain_separated_and_binds_manifest_and_nonce() {
        let id = PackageId::new("npm", "pkg", "1.0.0");
        let first = publish_signature_message_v2(
            "creg-testnet-1",
            "0x1111111111111111111111111111111111111111",
            &id,
            "aa",
            "bb",
            "cc",
            "dd",
            "0x2222222222222222222222222222222222222222",
            "npm:pkg",
            "nonce-a",
            1,
            2,
        );
        let second = publish_signature_message_v2(
            "creg-testnet-1",
            "0x1111111111111111111111111111111111111111",
            &id,
            "aa",
            "bb",
            "changed-manifest",
            "dd",
            "0x2222222222222222222222222222222222222222",
            "npm:pkg",
            "nonce-a",
            1,
            2,
        );
        assert!(first.starts_with("creg-publisher-admission-v2\nservice:creg-package-admission"));
        assert_ne!(first, second);

        let third = publish_signature_message_v3(
            "creg-testnet-1",
            "0x1111111111111111111111111111111111111111",
            &id,
            "aa",
            "bb",
            "cc",
            "dd",
            "0x2222222222222222222222222222222222222222",
            "npm:pkg",
            "nonce-a",
            1,
            2,
            &"e".repeat(64),
        );
        assert_ne!(first, third);
    }

    #[test]
    fn namespace_prefix_is_unambiguous_and_rotation_payload_is_scoped() {
        let id = PackageId::new("npm", "@creg/pkg", "1.0.0");
        let prefix = package_namespace_prefix(&id);
        assert_eq!(prefix, "npm:@creg/pkg@");
        assert!(validate_namespace_prefix(&prefix).is_ok());
        assert!(validate_namespace_prefix("npm:pkg").is_err());
        assert!(validate_namespace_prefix("npm:pkg@1.0.0@").is_ok());
        assert!(validate_namespace_prefix("npm:pkg @").is_err());

        let first = publisher_rotation_message("npm:pkg@", "AA", "BB", 7);
        let changed_namespace = publisher_rotation_message("npm:other@", "AA", "BB", 7);
        let changed_nonce = publisher_rotation_message("npm:pkg@", "AA", "BB", 8);
        assert_ne!(first, changed_namespace);
        assert_ne!(first, changed_nonce);
    }

    #[test]
    fn authoritative_namespaces_reject_npm_and_pypi_aliases() {
        assert_eq!(
            authoritative_package_namespace(&PackageId::new("npm", "@creg/pkg", "1.0.0")),
            Ok("npm:@creg/pkg@".into())
        );
        assert!(
            authoritative_package_namespace(&PackageId::new("npm", "@CREG/pkg", "1.0.0")).is_err()
        );
        assert!(authoritative_package_namespace(&PackageId::new(
            "npm",
            "@creg/pkg/other",
            "1.0.0"
        ))
        .is_err());

        assert_eq!(
            authoritative_package_namespace(&PackageId::new("pypi", "zope-interface", "6.0.0")),
            Ok("pypi:zope-interface@".into())
        );
        for alias in ["Zope.Interface", "zope_interface", "zope..interface"] {
            assert!(
                authoritative_package_namespace(&PackageId::new("pypi", alias, "6.0.0")).is_err()
            );
        }
        assert!(validate_authoritative_namespace("pypi:zope-interface@").is_ok());
        assert!(validate_authoritative_namespace("pypi:zope_interface@").is_err());
        assert!(validate_authoritative_namespace("cargo:serde@").is_err());
    }

    #[test]
    fn publisher_v4_message_binds_retrieval_multisig_and_authority_data() {
        let id = PackageId::new("npm", "pkg", "1.0.0");
        let keys = vec!["a".repeat(64), "b".repeat(64)];
        let base = v4_message(
            &id,
            &"4".repeat(64),
            "bafy-package",
            true,
            &"5".repeat(64),
            &"7".repeat(64),
            2,
            &keys,
            &"8".repeat(64),
            &"9".repeat(64),
        );
        assert!(base.starts_with("creg-publisher-admission-v4\nservice:creg-package-admission"));

        let mutations = [
            v4_message(
                &id,
                &"0".repeat(64),
                "bafy-package",
                true,
                &"5".repeat(64),
                &"7".repeat(64),
                2,
                &keys,
                &"8".repeat(64),
                &"9".repeat(64),
            ),
            v4_message(
                &id,
                &"4".repeat(64),
                "bafy-substituted",
                true,
                &"5".repeat(64),
                &"7".repeat(64),
                2,
                &keys,
                &"8".repeat(64),
                &"9".repeat(64),
            ),
            v4_message(
                &id,
                &"4".repeat(64),
                "bafy-package",
                false,
                &"5".repeat(64),
                &"7".repeat(64),
                2,
                &keys,
                &"8".repeat(64),
                &"9".repeat(64),
            ),
            v4_message(
                &id,
                &"4".repeat(64),
                "bafy-package",
                true,
                &"0".repeat(64),
                &"7".repeat(64),
                2,
                &keys,
                &"8".repeat(64),
                &"9".repeat(64),
            ),
            v4_message(
                &id,
                &"4".repeat(64),
                "bafy-package",
                true,
                &"5".repeat(64),
                &"0".repeat(64),
                2,
                &keys,
                &"8".repeat(64),
                &"9".repeat(64),
            ),
            v4_message(
                &id,
                &"4".repeat(64),
                "bafy-package",
                true,
                &"5".repeat(64),
                &"7".repeat(64),
                1,
                &keys,
                &"8".repeat(64),
                &"9".repeat(64),
            ),
            v4_message(
                &id,
                &"4".repeat(64),
                "bafy-package",
                true,
                &"5".repeat(64),
                &"7".repeat(64),
                2,
                &["a".repeat(64), "c".repeat(64)],
                &"8".repeat(64),
                &"9".repeat(64),
            ),
            v4_message(
                &id,
                &"4".repeat(64),
                "bafy-package",
                true,
                &"5".repeat(64),
                &"7".repeat(64),
                2,
                &keys,
                &"0".repeat(64),
                &"9".repeat(64),
            ),
            v4_message(
                &id,
                &"4".repeat(64),
                "bafy-package",
                true,
                &"5".repeat(64),
                &"7".repeat(64),
                2,
                &keys,
                &"8".repeat(64),
                &"0".repeat(64),
            ),
        ];
        for mutation in mutations {
            assert_ne!(base, mutation);
        }
    }
}

/// Submitted by a publisher to place a package in the pending pool.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PublishRequest {
    pub id: PackageId,
    /// SHA-256 of the tarball bytes.
    pub content_hash: String,
    /// IPFS CID where the tarball is already pinned.
    pub ipfs_cid: String,
    /// Publisher's staked EVM address (0x-prefixed). Production admission
    /// binds it to the Ed25519 key with the dual-domain proof below.
    #[serde(default)]
    pub publisher_address: String,
    /// Publisher's Ed25519 public key (hex-encoded).
    pub publisher_pubkey: String,
    /// Ed25519 signature over the versioned domain-separated payload (or the
    /// legacy payload for compatibility-only environments).
    pub signature: String,
    pub manifest: PackageManifest,
    pub submitted_at: DateTime<Utc>,
    /// Whether the tarball is encrypted (AES-256-GCM).
    pub shielded: bool,
    /// Ephemeral symmetric key encrypted for the validator set (ECIES bundle).
    pub key_bundle: Option<String>,
    /// Optional detached PGP signature for the tarball.
    pub pgp_signature: Option<String>,
    /// Optional PGP public key for verification.
    pub pgp_public_key: Option<String>,
    /// Multi-sig: minimum signatures required (default 2).
    #[serde(default)]
    pub threshold: usize,
    /// Multi-sig: list of publisher pubkeys (2-of-3 support).
    #[serde(default)]
    pub publisher_pubkeys: Vec<String>,
    /// Multi-sig: signatures corresponding to `publisher_pubkeys`.
    #[serde(default)]
    pub signatures: Vec<String>,
    /// Phase-1 dual-key publisher proof and replay/freshness metadata.
    /// Empty values identify legacy records that must not pass production
    /// admission; serde defaults preserve old queue records for migration.
    #[serde(default)]
    pub publisher_evm_signature: String,
    #[serde(default)]
    pub publisher_nonce: String,
    #[serde(default)]
    pub publisher_expires_at: u64,
    #[serde(default)]
    pub source_hash: String,
    #[serde(default)]
    pub manifest_hash: String,
    #[serde(default)]
    pub ownership_claim: String,
    #[serde(default)]
    pub publisher_chain_id: String,
    #[serde(default)]
    pub publisher_genesis_hash: String,
    #[serde(default)]
    pub publisher_staking_contract: String,
    /// `4` is required for proof-retaining post-activation transactions.
    /// Zero identifies historical v1-v3 admission messages.
    #[serde(default)]
    pub publisher_authorization_version: u16,
    /// Optional authoritative upstream ownership evidence. When present, its
    /// digest is bound into the v3 dual-controller signature payload.
    #[serde(default)]
    pub ownership_proof: Option<NamespaceAuthorityProof>,
    /// Quorum certificate produced after validators independently verify the
    /// upstream evidence. The raw proof alone is never authoritative.
    #[serde(default)]
    pub namespace_authority_certificate: Option<CertifiedNamespaceAuthority>,
}

/// Publisher authorization retained in authoritative publish transactions.
/// Unlike the legacy `Publish(ChainRecord)` variant, this contains everything
/// a peer or rebuilding node needs to reconstruct the signed publisher
/// message and verify the optional upstream namespace certificate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherAuthorizationV1 {
    pub schema_version: u16,
    pub chain_id: String,
    pub genesis_hash: String,
    pub staking_contract: String,
    pub publisher_address: String,
    pub publisher_ed25519_pubkey: String,
    pub publisher_ed25519_signature: String,
    pub publisher_evm_signature: String,
    pub nonce: String,
    pub issued_at: i64,
    pub expires_at: u64,
    pub source_hash: String,
    pub manifest_hash: String,
    pub ipfs_cid: String,
    pub shielded: bool,
    #[serde(default)]
    pub key_bundle_sha256: String,
    #[serde(default)]
    pub pgp_signature_sha256: String,
    #[serde(default)]
    pub pgp_public_key_sha256: String,
    pub ownership_claim: String,
    #[serde(default)]
    pub threshold: usize,
    #[serde(default)]
    pub publisher_pubkeys: Vec<String>,
    #[serde(default)]
    pub signatures: Vec<String>,
    #[serde(default)]
    pub ownership_proof: Option<NamespaceAuthorityProof>,
    #[serde(default)]
    pub authority_certificate: Option<CertifiedNamespaceAuthority>,
}

impl PublisherAuthorizationV1 {
    pub fn from_publish_request(request: &PublishRequest) -> Self {
        Self {
            schema_version: PUBLISHER_AUTHORIZATION_SCHEMA_VERSION,
            chain_id: request.publisher_chain_id.clone(),
            genesis_hash: request.publisher_genesis_hash.clone(),
            staking_contract: request.publisher_staking_contract.clone(),
            publisher_address: request.publisher_address.clone(),
            publisher_ed25519_pubkey: request.publisher_pubkey.clone(),
            publisher_ed25519_signature: request.signature.clone(),
            publisher_evm_signature: request.publisher_evm_signature.clone(),
            nonce: request.publisher_nonce.clone(),
            issued_at: request.submitted_at.timestamp(),
            expires_at: request.publisher_expires_at,
            source_hash: request.source_hash.clone(),
            manifest_hash: request.manifest_hash.clone(),
            ipfs_cid: request.ipfs_cid.clone(),
            shielded: request.shielded,
            key_bundle_sha256: request
                .key_bundle
                .as_ref()
                .map(|bundle| crate::sha256_hex(bundle.as_bytes()))
                .unwrap_or_default(),
            pgp_signature_sha256: request
                .pgp_signature
                .as_ref()
                .map(|value| crate::sha256_hex(value.as_bytes()))
                .unwrap_or_default(),
            pgp_public_key_sha256: request
                .pgp_public_key
                .as_ref()
                .map(|value| crate::sha256_hex(value.as_bytes()))
                .unwrap_or_default(),
            ownership_claim: request.ownership_claim.clone(),
            threshold: request.threshold,
            publisher_pubkeys: request.publisher_pubkeys.clone(),
            signatures: request.signatures.clone(),
            ownership_proof: request.ownership_proof.clone(),
            authority_certificate: request.namespace_authority_certificate.clone(),
        }
    }

    pub fn signing_message(&self, id: &PackageId, content_hash: &str) -> String {
        publish_signature_message_v4(
            &self.chain_id,
            &self.genesis_hash,
            &self.staking_contract,
            id,
            content_hash,
            &self.source_hash,
            &self.manifest_hash,
            &self.ipfs_cid,
            self.shielded,
            &self.key_bundle_sha256,
            &self.pgp_signature_sha256,
            &self.pgp_public_key_sha256,
            &self.publisher_ed25519_pubkey,
            &self.publisher_address,
            &self.ownership_claim,
            &self.nonce,
            self.issued_at,
            self.expires_at,
            self.threshold,
            &self.publisher_pubkeys,
            &self
                .ownership_proof
                .as_ref()
                .map(NamespaceAuthorityProof::digest)
                .unwrap_or_default(),
            &self
                .authority_certificate
                .as_ref()
                .map(CertifiedNamespaceAuthority::digest)
                .unwrap_or_default(),
        )
    }

    pub fn authorization_digest(&self, id: &PackageId, content_hash: &str) -> String {
        crate::sha256_hex(self.signing_message(id, content_hash).as_bytes())
    }

    pub fn validate_record_binding(&self, record: &ChainRecord) -> Result<(), &'static str> {
        if self.schema_version != PUBLISHER_AUTHORIZATION_SCHEMA_VERSION
            || !is_canonical_single_line(&self.chain_id)
            || !is_lower_hex_sha256(&self.genesis_hash)
            || !is_canonical_single_line(&self.nonce)
            || !is_canonical_single_line(&self.ownership_claim)
            || !is_canonical_single_line(&record.id.version)
            || self.nonce.len() > 128
            || !self
                .nonce
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || self.issued_at < 0
            || self.expires_at <= self.issued_at as u64
            || self.expires_at.saturating_sub(self.issued_at as u64)
                > MAX_PUBLISHER_AUTHORIZATION_TTL_SECS
        {
            return Err("publisher authorization metadata is incomplete");
        }
        let staking_contract = canonical_publisher_address(&self.staking_contract);
        let publisher_address = canonical_publisher_address(&self.publisher_address);
        if staking_contract != self.staking_contract
            || publisher_address != self.publisher_address
            || staking_contract.len() != 42
            || publisher_address.len() != 42
            || !staking_contract[2..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !publisher_address[2..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("publisher authorization addresses are not canonical");
        }
        let namespace = authoritative_package_namespace(&record.id)?;
        if self.ownership_claim != namespace.trim_end_matches('@') {
            return Err("publisher authorization ownership claim is not canonical");
        }
        if !self.publisher_ed25519_pubkey.eq(&record.publisher_pubkey)
            || self.publisher_ed25519_pubkey.len() != 64
            || self.publisher_ed25519_pubkey != self.publisher_ed25519_pubkey.to_ascii_lowercase()
            || !self
                .publisher_ed25519_pubkey
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("publisher authorization key does not match the record controller");
        }
        if self.ipfs_cid != record.ipfs_cid
            || self.shielded != record.shielded
            || self.key_bundle_sha256
                != record
                    .key_bundle
                    .as_ref()
                    .map(|bundle| crate::sha256_hex(bundle.as_bytes()))
                    .unwrap_or_default()
            || !is_canonical_single_line(&self.ipfs_cid)
            || (!self.key_bundle_sha256.is_empty() && !is_lower_hex_sha256(&self.key_bundle_sha256))
            || (self.shielded != !self.key_bundle_sha256.is_empty())
            || self.pgp_signature_sha256 != record.pgp_signature_sha256
            || self.pgp_public_key_sha256 != record.pgp_public_key_sha256
            || (!self.pgp_signature_sha256.is_empty()
                && !is_lower_hex_sha256(&self.pgp_signature_sha256))
            || (!self.pgp_public_key_sha256.is_empty()
                && !is_lower_hex_sha256(&self.pgp_public_key_sha256))
            || (self.pgp_signature_sha256.is_empty() != self.pgp_public_key_sha256.is_empty())
        {
            return Err("publisher authorization retrieval metadata does not match the record");
        }
        let keyset =
            canonical_publisher_keyset(&self.publisher_ed25519_pubkey, &self.publisher_pubkeys);
        let canonical_signature = |signature: &str| {
            signature.len() == 128
                && signature == signature.to_ascii_lowercase()
                && signature.bytes().all(|byte| byte.is_ascii_hexdigit())
        };
        if keyset.iter().any(|key| {
            key.len() != 64
                || key != &key.to_ascii_lowercase()
                || !key.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) || self.publisher_pubkeys.is_empty()
            || self.publisher_pubkeys != keyset
            || !keyset.contains(&self.publisher_ed25519_pubkey)
            || self.threshold == 0
            || self.threshold > keyset.len()
            || !self.publisher_ed25519_signature.is_empty()
            || self.signatures.len() != self.publisher_pubkeys.len()
            || self
                .signatures
                .iter()
                .any(|signature| !signature.is_empty() && !canonical_signature(signature))
            || record.threshold != self.threshold
            || record.publisher_pubkeys != self.publisher_pubkeys
        {
            return Err("publisher authorization multisig policy is invalid");
        }
        let manifest = record
            .manifest
            .as_ref()
            .ok_or("authorized publish record lacks its signed manifest")?;
        let manifest_hash = crate::sha256_hex(
            &serde_json::to_vec(manifest).map_err(|_| "package manifest does not serialize")?,
        );
        if !manifest_hash.eq_ignore_ascii_case(&self.manifest_hash) {
            return Err("authorized publish manifest digest does not match the record");
        }
        if self.source_hash.len() != 64
            || !self
                .source_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.source_hash != self.source_hash.to_ascii_lowercase()
            || self.manifest_hash.len() != 64
            || !self
                .manifest_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.manifest_hash != self.manifest_hash.to_ascii_lowercase()
            || record.content_hash.len() != 64
            || !record
                .content_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || record.content_hash != record.content_hash.to_ascii_lowercase()
        {
            return Err("authorized publish source/manifest digest is invalid");
        }

        if let Some(proof) = &self.ownership_proof {
            proof.validate_structure()?;
            if proof.namespace != namespace
                || proof.controller_ed25519_pubkey != self.publisher_ed25519_pubkey
                || proof.controller_evm_address != self.publisher_address
            {
                return Err("ownership proof does not bind the publisher authorization");
            }
        }

        if let Some(certificate) = &self.authority_certificate {
            certificate.statement.validate_structure()?;
            let proof = self
                .ownership_proof
                .as_ref()
                .ok_or("authority certificate requires its ownership proof")?;
            if !matches!(
                certificate.statement.purpose,
                NamespaceAuthorityPurpose::Claim
            ) || certificate.statement.chain_id != self.chain_id
                || certificate.statement.genesis_hash != self.genesis_hash
                || certificate.statement.namespace != namespace
                || !certificate
                    .statement
                    .next_controller_ed25519_pubkey
                    .eq_ignore_ascii_case(&self.publisher_ed25519_pubkey)
                || !canonical_publisher_address(&certificate.statement.next_controller_evm_address)
                    .eq_ignore_ascii_case(&canonical_publisher_address(&self.publisher_address))
                || certificate.statement.authority_proof.digest() != proof.digest()
                || certificate.statement.authority_intent_digest != proof.challenge_sha256
            {
                return Err("namespace authority certificate does not bind the publish record");
            }
        }
        Ok(())
    }
}

/// Shared references to the analysis bundles active when a validator formed a verdict.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisBundleRefs {
    #[serde(default)]
    pub policy_bundle_id: String,
    #[serde(default)]
    pub feature_schema_id: String,
    #[serde(default)]
    pub expert_bundle_id: String,
    #[serde(default)]
    pub embedding_model_id: String,
    #[serde(default)]
    pub index_epoch: String,
    #[serde(default)]
    pub threshold_profile_id: String,
    #[serde(default)]
    pub llm_prompt_profile_id: String,
    /// Pinned OSV snapshot epoch (`osv-off` when consensus OSV is disabled).
    #[serde(default)]
    pub osv_snapshot_epoch: String,
}

fn default_osv_snapshot_epoch() -> String {
    "osv-off".to_string()
}

impl Default for AnalysisBundleRefs {
    fn default() -> Self {
        Self {
            policy_bundle_id: String::new(),
            feature_schema_id: String::new(),
            expert_bundle_id: String::new(),
            embedding_model_id: String::new(),
            index_epoch: String::new(),
            threshold_profile_id: String::new(),
            llm_prompt_profile_id: String::new(),
            osv_snapshot_epoch: default_osv_snapshot_epoch(),
        }
    }
}

impl AnalysisBundleRefs {
    /// Effective OSV epoch for digests and completeness checks.
    pub fn effective_osv_snapshot_epoch(&self) -> String {
        let trimmed = self.osv_snapshot_epoch.trim();
        if trimmed.is_empty() {
            default_osv_snapshot_epoch()
        } else {
            trimmed.to_string()
        }
    }

    /// True when the vote names every deterministic scanner artifact needed
    /// to reproduce its consensus decision.
    pub fn is_consensus_complete(&self) -> bool {
        [
            self.policy_bundle_id.as_str(),
            self.feature_schema_id.as_str(),
            self.expert_bundle_id.as_str(),
            self.embedding_model_id.as_str(),
            self.index_epoch.as_str(),
            self.threshold_profile_id.as_str(),
            self.llm_prompt_profile_id.as_str(),
        ]
        .iter()
        .all(|value| !value.trim().is_empty())
            && !self.effective_osv_snapshot_epoch().trim().is_empty()
    }

    /// Stable, domain-separated digest of the scanner/profile references.
    pub fn scanner_profile_digest(&self, scanner_version: &str) -> String {
        let input = format!(
            "creg-scanner-profile-v1|scanner={}|policy={}|features={}|experts={}|embedding={}|index={}|thresholds={}|llm_prompt={}|osv={}",
            scanner_version.trim(),
            self.policy_bundle_id.trim(),
            self.feature_schema_id.trim(),
            self.expert_bundle_id.trim(),
            self.embedding_model_id.trim(),
            self.index_epoch.trim(),
            self.threshold_profile_id.trim(),
            self.llm_prompt_profile_id.trim(),
            self.effective_osv_snapshot_epoch().trim(),
        );
        crate::sha256_hex(input.as_bytes())
    }
}

pub fn scanner_profile_digest(
    scanner_version: &str,
    analysis_bundles: &AnalysisBundleRefs,
) -> String {
    analysis_bundles.scanner_profile_digest(scanner_version)
}

pub fn is_consensus_grade_vote(
    scanner_version: &str,
    analysis_bundles: &AnalysisBundleRefs,
    evidence_digest: &str,
) -> bool {
    !scanner_version.trim().starts_with("degraded")
        && analysis_bundles.is_consensus_complete()
        && !evidence_digest.trim().is_empty()
}

/// Compact deterministic risk data exposed on finalized records and validator votes.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DeterministicRiskSummary {
    #[serde(default)]
    pub score: u8,
    #[serde(default)]
    pub deterministic_score: u8,
    #[serde(default)]
    pub advisory_score: u8,
    #[serde(default)]
    pub band: String,
    #[serde(default)]
    pub disposition: String,
    #[serde(default)]
    pub deterministic_findings: usize,
    #[serde(default)]
    pub advisory_findings: usize,
    #[serde(default)]
    pub critical_findings: usize,
    #[serde(default)]
    pub high_findings: usize,
    #[serde(default)]
    pub medium_findings: usize,
    #[serde(default)]
    pub low_findings: usize,
    #[serde(default)]
    pub reasons: Vec<String>,
}

/// A single entry in the on-chain package index.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChainRecord {
    pub id: PackageId,
    pub content_hash: String,
    pub ipfs_cid: String,
    pub publisher_pubkey: String,
    /// Hex block hash of the block that included this record.
    pub block_hash: String,
    pub published_at: DateTime<Utc>,
    /// Signatures from the N-of-M validators that approved this package.
    pub validator_signatures: Vec<ValidatorSignature>,
    pub status: PackageStatus,
    /// Whether this record represents a private (encrypted) package.
    pub shielded: bool,
    /// Encrypted key bundle required for decryption (available to authorized nodes).
    pub key_bundle: Option<String>,
    /// Verified PGP fingerprint (if any).
    pub pgp_fingerprint: Option<String>,
    /// Hashes of the exact optional detached PGP inputs bound by publisher v4.
    #[serde(default)]
    pub pgp_signature_sha256: String,
    #[serde(default)]
    pub pgp_public_key_sha256: String,
    /// Security validation findings (Sandbox, Static, Diff).
    pub findings: Vec<Finding>,
    /// Versioned policy and feature bundles used while producing this record.
    #[serde(default)]
    pub analysis_bundles: AnalysisBundleRefs,
    /// Digest over deterministic evidence that shaped the final decision.
    #[serde(default)]
    pub evidence_digest: String,
    /// Compact deterministic risk summary captured when the record was finalized.
    #[serde(default)]
    pub deterministic_risk: DeterministicRiskSummary,
    /// Real-time access metrics (Kind Enhancement)
    pub access_count: u32,
    pub last_accessed: Option<DateTime<Utc>>,
    /// Multi-sig: minimum signatures required.
    #[serde(default)]
    pub threshold: usize,
    /// Multi-sig: list of publisher pubkeys.
    #[serde(default)]
    pub publisher_pubkeys: Vec<String>,
    /// Package manifest from the publisher (declared behaviors).
    /// Stored at finalization so that future versions can diff against it.
    #[serde(default)]
    pub manifest: Option<PackageManifest>,

    // === HYBRID ANALYSIS COMMITTEE FIELDS (mandatory + voluntary) ===
    #[serde(default)]
    pub analysis_committee_size: u32, // Target mandatory committee size (e.g. 100)
    #[serde(default)]
    pub analysis_selection_seed: String, // Seed used to deterministically order/select the committee
    #[serde(default)]
    pub analysis_effective_size: u32, // Actual size after any expansion
    #[serde(default)]
    pub analysis_mandatory_contributed: u32, // How many mandatory votes were in the final record

    /// Full committee metadata (including members list and performance_bias).
    /// Preferred over the scalar fields for new records. Stored for full auditability
    /// of how the mandatory set was chosen (stake/rep/perf-weighted VRF selection).
    #[serde(default)]
    pub analysis_committee: Option<AnalysisCommittee>,
}

impl ChainRecord {
    /// Returns the committee, preferring the full object if present.
    /// For legacy records (pre full committee storage), reconstructs a partial
    /// AnalysisCommittee from the scalar fields (note: members list will be empty,
    /// performance info unknown).
    pub fn get_analysis_committee(&self) -> Option<AnalysisCommittee> {
        if let Some(c) = &self.analysis_committee {
            return Some(c.clone());
        }
        if self.analysis_committee_size > 0 || !self.analysis_selection_seed.is_empty() {
            return Some(AnalysisCommittee {
                size: self.analysis_committee_size,
                members: vec![], // not stored in legacy scalars
                seed: self.analysis_selection_seed.clone(),
                effective_size: self.analysis_effective_size,
                performance_bias: false, // unknown for old records,
                ..Default::default()
            });
        }
        None
    }
}

/// Current lifecycle state of a package on the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PackageStatus {
    /// Accepted by consensus — safe to install.
    Verified,
    /// Submitted but not yet through consensus — pending pool only.
    #[default]
    Pending,
    /// Rejected by consensus or later found malicious.
    Revoked { reason: String },
}

/// A validator's signature over a package hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorSignature {
    pub validator_id: String,
    pub validator_pubkey: String,
    pub signature: String,
    pub vote: ValidatorVote,
    pub signed_at: DateTime<Utc>,
    /// ML model version used for deep scan (e.g., "codebert-v0.1.0" or "degraded-no-model").
    /// Allows consensus to verify validators used compatible model versions.
    #[serde(default)]
    pub ml_model_version: String,
    /// Versioned analysis bundles active when this vote was produced.
    #[serde(default)]
    pub analysis_bundles: AnalysisBundleRefs,
    /// Digest over deterministic evidence considered by the voting validator.
    #[serde(default)]
    pub evidence_digest: String,
    /// Compact deterministic risk summary captured when this vote was formed.
    #[serde(default)]
    pub deterministic_risk: DeterministicRiskSummary,

    // === HYBRID FIELDS ===
    #[serde(default)]
    pub is_mandatory: bool, // true if this validator was in the primary selected committee
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ValidatorVote {
    #[default]
    Approve,
    Reject {
        reason: String,
    },
}

/// Committee metadata for the hybrid mandatory + voluntary analysis model.
/// Stored in pending entries and finalized ChainRecord for verifiability.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnalysisCommittee {
    /// Target size of the primary mandatory committee (e.g. 64).
    pub size: u32,
    /// Deterministic ordered list of validator_ids for this package.
    /// First `size` are the primary mandatory set.
    pub members: Vec<String>,
    /// The seed used for deterministic selection (package + randomness source).
    pub seed: String,
    /// Effective size after any expansion due to timeouts.
    #[serde(default)]
    pub effective_size: u32,
    /// Whether performance_score (resources/reliability) was factored into selection ordering.
    #[serde(default)]
    pub performance_bias: bool,

    /// Snapshot of each member's performance_score (and contribution_count if desired)
    /// captured at finalization time. This is "at-finalization-time" data for auditability.
    /// Only populated for verified records going forward.
    #[serde(default)]
    pub member_performance_at_finalization: std::collections::HashMap<String, u32>,
    /// Canonical hash of the validator set used by the selector. Empty only
    /// for legacy records that predate snapshot-bound assignment proofs.
    #[serde(default)]
    pub validator_snapshot_hash: String,
    /// Height at which the validator snapshot became effective.
    #[serde(default)]
    pub validator_snapshot_height: u64,
    /// Final block hash under the preceding set. This prevents a committee
    /// assignment from being replayed onto a competing chain fork.
    #[serde(default)]
    pub validator_snapshot_anchor_block_hash: String,
    /// Witness-independent logical ID of the quorum-certified transition.
    /// The legacy field name is retained on wire. Empty marks a legacy/local
    /// assignment and must never be treated as an authenticated anchor.
    #[serde(default)]
    pub validator_snapshot_certificate_hash: String,
    /// Public randomness that was used to derive `seed`. Keeping it lets any
    /// node recompute the deterministic ordering instead of trusting a seed
    /// supplied by the admitting node.
    #[serde(default)]
    pub selection_randomness: String,
    /// SHA-256 commitment to subject, snapshot, seed, ordered members, and
    /// selector parameters. It makes any post-selection edit detectable.
    #[serde(default)]
    pub assignment_proof: String,
}

pub fn committee_assignment_proof(
    canonical: &str,
    content_hash: &str,
    committee: &AnalysisCommittee,
) -> String {
    let members = committee.members.join(",");
    crate::sha256_hex(
        format!(
            "creg-committee-assignment-v2\nservice:creg-consensus\nsubject:{}\ncontent_hash:{}\nvalidator_snapshot_hash:{}\nvalidator_snapshot_height:{}\nvalidator_snapshot_anchor_block_hash:{}\nvalidator_snapshot_certificate_hash:{}\nselection_randomness:{}\nseed:{}\nsize:{}\neffective_size:{}\nperformance_bias:{}\nmembers:{}",
            canonical,
            content_hash,
            committee.validator_snapshot_hash,
            committee.validator_snapshot_height,
            committee.validator_snapshot_anchor_block_hash,
            committee.validator_snapshot_certificate_hash,
            committee.selection_randomness,
            committee.seed,
            committee.size,
            committee.effective_size,
            committee.performance_bias,
            members,
        )
        .as_bytes(),
    )
}

impl AnalysisCommittee {
    pub fn assignment_proof_is_valid(&self, canonical: &str, content_hash: &str) -> bool {
        !self.validator_snapshot_hash.is_empty()
            && !self.selection_randomness.is_empty()
            && self.assignment_proof == committee_assignment_proof(canonical, content_hash, self)
    }
}

#[cfg(test)]
mod publisher_authorization_tests {
    use super::*;

    fn record_fixture() -> ChainRecord {
        ChainRecord {
            id: PackageId::new("npm", "owned-package", "1.0.0"),
            content_hash: "1".repeat(64),
            ipfs_cid: "bafy-owned-package".into(),
            publisher_pubkey: "a".repeat(64),
            shielded: false,
            threshold: 1,
            publisher_pubkeys: vec!["a".repeat(64)],
            manifest: Some(PackageManifest::default()),
            ..Default::default()
        }
    }

    fn authorization_fixture(record: &ChainRecord) -> PublisherAuthorizationV1 {
        let manifest = record.manifest.as_ref().expect("test manifest");
        PublisherAuthorizationV1 {
            schema_version: PUBLISHER_AUTHORIZATION_SCHEMA_VERSION,
            chain_id: "creg-testnet-1".into(),
            genesis_hash: "2".repeat(64),
            staking_contract: "0x1111111111111111111111111111111111111111".into(),
            publisher_address: "0x2222222222222222222222222222222222222222".into(),
            publisher_ed25519_pubkey: record.publisher_pubkey.clone(),
            publisher_ed25519_signature: String::new(),
            publisher_evm_signature: "c".repeat(130),
            nonce: "publisher-nonce-1".into(),
            issued_at: 1_700_000_000,
            expires_at: 1_700_000_600,
            source_hash: "3".repeat(64),
            manifest_hash: crate::sha256_hex(
                &serde_json::to_vec(manifest).expect("serialize test manifest"),
            ),
            ipfs_cid: record.ipfs_cid.clone(),
            shielded: record.shielded,
            key_bundle_sha256: String::new(),
            pgp_signature_sha256: String::new(),
            pgp_public_key_sha256: String::new(),
            ownership_claim: "npm:owned-package".into(),
            threshold: record.threshold,
            publisher_pubkeys: record.publisher_pubkeys.clone(),
            signatures: vec!["b".repeat(128)],
            ownership_proof: None,
            authority_certificate: None,
        }
    }

    #[test]
    fn publisher_authorization_accepts_exact_record_binding() {
        let record = record_fixture();
        let authorization = authorization_fixture(&record);
        assert!(authorization.validate_record_binding(&record).is_ok());
    }

    #[test]
    fn publisher_authorization_rejects_validity_over_twenty_four_hours() {
        let record = record_fixture();
        let mut authorization = authorization_fixture(&record);
        authorization.expires_at =
            authorization.issued_at as u64 + MAX_PUBLISHER_AUTHORIZATION_TTL_SECS;
        assert!(authorization.validate_record_binding(&record).is_ok());

        authorization.expires_at =
            authorization.issued_at as u64 + MAX_PUBLISHER_AUTHORIZATION_TTL_SECS + 1;
        assert_eq!(
            authorization.validate_record_binding(&record),
            Err("publisher authorization metadata is incomplete")
        );
    }

    #[test]
    fn publisher_authorization_rejects_record_metadata_substitution() {
        let record = record_fixture();
        let authorization = authorization_fixture(&record);

        let mut changed = record.clone();
        changed.ipfs_cid = "bafy-attacker-package".into();
        assert!(authorization.validate_record_binding(&changed).is_err());

        let mut changed = record.clone();
        changed.publisher_pubkey = "d".repeat(64);
        assert!(authorization.validate_record_binding(&changed).is_err());

        let mut changed = record.clone();
        changed.shielded = true;
        changed.key_bundle = Some("substituted-key-bundle".into());
        assert!(authorization.validate_record_binding(&changed).is_err());

        let mut changed = record;
        changed
            .manifest
            .as_mut()
            .expect("test manifest")
            .description = Some("post-signature edit".into());
        assert!(authorization.validate_record_binding(&changed).is_err());
    }

    #[test]
    fn publisher_authorization_rejects_multisig_downgrade_and_aliases() {
        let record = record_fixture();
        let authorization = authorization_fixture(&record);

        let mut downgraded = authorization.clone();
        downgraded.threshold = 0;
        assert!(downgraded.validate_record_binding(&record).is_err());

        let mut alternate_single_signer_encoding = authorization.clone();
        alternate_single_signer_encoding.publisher_pubkeys.clear();
        alternate_single_signer_encoding.signatures.clear();
        alternate_single_signer_encoding.publisher_ed25519_signature = "b".repeat(128);
        let mut alternate_record = record.clone();
        alternate_record.publisher_pubkeys.clear();
        assert!(alternate_single_signer_encoding
            .validate_record_binding(&alternate_record)
            .is_err());

        let mut ambiguous = authorization.clone();
        ambiguous.publisher_pubkeys = vec!["b".repeat(64), "a".repeat(64)];
        ambiguous.signatures = vec!["c".repeat(128), "d".repeat(128)];
        ambiguous.publisher_ed25519_signature.clear();
        ambiguous.threshold = 2;
        let mut ambiguous_record = record.clone();
        ambiguous_record.publisher_pubkeys = ambiguous.publisher_pubkeys.clone();
        ambiguous_record.threshold = 2;
        assert!(ambiguous
            .validate_record_binding(&ambiguous_record)
            .is_err());

        let mut aliased_record = record.clone();
        aliased_record.id.name = "Owned_Package".into();
        assert!(authorization
            .validate_record_binding(&aliased_record)
            .is_err());

        let mut malformed = authorization;
        malformed.source_hash = "A".repeat(64);
        assert!(malformed.validate_record_binding(&record).is_err());
    }

    #[test]
    fn publisher_authorization_rejects_unknown_json_fields() {
        let record = record_fixture();
        let authorization = authorization_fixture(&record);
        let mut value = serde_json::to_value(authorization).unwrap();
        value["unsigned_extension"] = serde_json::json!("ignored-by-old-readers");
        assert!(serde_json::from_value::<PublisherAuthorizationV1>(value).is_err());
    }
}

#[cfg(test)]
mod committee_assignment_tests {
    use super::*;

    #[test]
    fn assignment_proof_detects_snapshot_and_member_tampering() {
        let mut committee = AnalysisCommittee {
            size: 2,
            members: vec!["v1".into(), "v2".into()],
            seed: "a".repeat(64),
            effective_size: 2,
            performance_bias: true,
            validator_snapshot_hash: "b".repeat(64),
            validator_snapshot_height: 42,
            validator_snapshot_anchor_block_hash: "c".repeat(64),
            validator_snapshot_certificate_hash: "d".repeat(64),
            selection_randomness: "e".repeat(64),
            ..Default::default()
        };
        committee.assignment_proof = committee_assignment_proof("npm:pkg@1.0.0", "f", &committee);
        assert!(committee.assignment_proof_is_valid("npm:pkg@1.0.0", "f"));
        committee.members.swap(0, 1);
        assert!(!committee.assignment_proof_is_valid("npm:pkg@1.0.0", "f"));
    }
}
