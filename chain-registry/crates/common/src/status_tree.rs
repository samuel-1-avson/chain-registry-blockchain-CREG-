//! Current-status commitment for `BlockHeader.status_root` (ADR-019 decision
//! #1, `ECA-10`): a two-level sparse Merkle tree over
//! `(package, version) -> StatusLeafV1`.
//!
//! * The **version tree** of one package maps `sha256(version)` to the hash of
//!   that version's [`StatusLeafV1`].
//! * The **package tree** maps `sha256(package)` to the root of that package's
//!   version tree. Its root is `status_root`.
//!
//! Both are compact sparse Merkle trees over 256-bit keys: an empty subtree
//! hashes to 32 zero bytes, a subtree holding exactly one entry collapses to
//! that entry's leaf hash, and any other subtree is
//! `sha256(0x01 || left || right)`. Leaves are `sha256(0x00 || key || value)`.
//! A proof lists the sibling hashes from the root down to where the key's
//! path becomes unique, which gives membership and non-membership proofs of
//! at most 256 siblings (typically `log2(n)`).

use crate::committee::CregFindingCategoryV1;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub type Hash32 = [u8; 32];

/// Hash of an empty subtree.
pub const EMPTY_HASH: Hash32 = [0u8; 32];

const LEAF_PREFIX: u8 = 0x00;
const NODE_PREFIX: u8 = 0x01;
const PACKAGE_KEY_DOMAIN: &[u8] = b"creg-status-package-v2";
const VERSION_KEY_DOMAIN: &[u8] = b"creg-status-version-v2";
const LEAF_VALUE_DOMAIN: &[u8] = b"creg-status-leaf-v1";

fn sha256(parts: &[&[u8]]) -> Hash32 {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn leaf_hash(key: &Hash32, value: &Hash32) -> Hash32 {
    sha256(&[&[LEAF_PREFIX], key, value])
}

fn node_hash(left: &Hash32, right: &Hash32) -> Hash32 {
    if left == &EMPTY_HASH && right == &EMPTY_HASH {
        EMPTY_HASH
    } else {
        sha256(&[&[NODE_PREFIX], left, right])
    }
}

/// Bit `index` (0 = most significant) of `key`.
fn bit(key: &Hash32, index: usize) -> bool {
    (key[index / 8] >> (7 - (index % 8))) & 1 == 1
}

/// What terminates a proof path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SmtTerminal {
    /// The key is present with this value hash.
    Member,
    /// The path ends in an empty subtree: the key is absent.
    Empty,
    /// The path ends in another key's leaf: the key is absent.
    OtherLeaf {
        #[serde(with = "hex32")]
        key: Hash32,
        #[serde(with = "hex32")]
        value: Hash32,
    },
}

/// Sibling hashes from the root downward (`None` = empty subtree).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtProof {
    #[serde(with = "hex32_opt_vec")]
    pub siblings: Vec<Option<Hash32>>,
    pub terminal: SmtTerminal,
}

/// Compact sparse Merkle tree over 256-bit keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SparseMerkleTree {
    entries: BTreeMap<Hash32, Hash32>,
}

impl SparseMerkleTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn insert(&mut self, key: Hash32, value: Hash32) {
        self.entries.insert(key, value);
    }

    pub fn remove(&mut self, key: &Hash32) -> Option<Hash32> {
        self.entries.remove(key)
    }

    pub fn get(&self, key: &Hash32) -> Option<&Hash32> {
        self.entries.get(key)
    }

    pub fn root(&self) -> Hash32 {
        let entries: Vec<(&Hash32, &Hash32)> = self.entries.iter().collect();
        subtree_hash(&entries, 0)
    }

    pub fn prove(&self, key: &Hash32) -> SmtProof {
        let mut entries: Vec<(&Hash32, &Hash32)> = self.entries.iter().collect();
        let mut siblings = Vec::new();
        let mut depth = 0usize;
        loop {
            match entries.len() {
                0 => {
                    return SmtProof {
                        siblings,
                        terminal: SmtTerminal::Empty,
                    }
                }
                1 => {
                    let (only_key, only_value) = entries[0];
                    let terminal = if only_key == key {
                        SmtTerminal::Member
                    } else {
                        SmtTerminal::OtherLeaf {
                            key: *only_key,
                            value: *only_value,
                        }
                    };
                    return SmtProof { siblings, terminal };
                }
                _ => {}
            }
            // Entries are sorted, so the split by bit `depth` is a prefix/suffix.
            let split = entries.partition_point(|(entry_key, _)| !bit(entry_key, depth));
            let (left, right) = entries.split_at(split);
            let (ours, theirs) = if bit(key, depth) {
                (right, left)
            } else {
                (left, right)
            };
            let sibling = subtree_hash(theirs, depth + 1);
            siblings.push((sibling != EMPTY_HASH).then_some(sibling));
            entries = ours.to_vec();
            depth += 1;
        }
    }
}

fn subtree_hash(entries: &[(&Hash32, &Hash32)], depth: usize) -> Hash32 {
    match entries {
        [] => EMPTY_HASH,
        [(key, value)] => leaf_hash(key, value),
        _ => {
            let split = entries.partition_point(|(entry_key, _)| !bit(entry_key, depth));
            let (left, right) = entries.split_at(split);
            node_hash(
                &subtree_hash(left, depth + 1),
                &subtree_hash(right, depth + 1),
            )
        }
    }
}

/// Verify `proof` for `key` against `root`. `value` is `Some(hash)` to prove
/// membership with that value, `None` to prove absence.
pub fn verify_smt_proof(
    root: &Hash32,
    key: &Hash32,
    value: Option<&Hash32>,
    proof: &SmtProof,
) -> bool {
    if proof.siblings.len() > 256 {
        return false;
    }
    let depth = proof.siblings.len();
    let mut current = match (&proof.terminal, value) {
        (SmtTerminal::Member, Some(value)) => leaf_hash(key, value),
        (SmtTerminal::Empty, None) => EMPTY_HASH,
        (
            SmtTerminal::OtherLeaf {
                key: other_key,
                value: other_value,
            },
            None,
        ) => {
            // The other leaf must sit on the same path but be a different key.
            if other_key == key || (0..depth).any(|index| bit(other_key, index) != bit(key, index))
            {
                return false;
            }
            leaf_hash(other_key, other_value)
        }
        _ => return false,
    };
    for index in (0..depth).rev() {
        let sibling = proof.siblings[index].unwrap_or(EMPTY_HASH);
        current = if bit(key, index) {
            node_hash(&sibling, &current)
        } else {
            node_hash(&current, &sibling)
        };
    }
    &current == root
}

/// Committed lifecycle state of one package version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum StatusLeafStateV1 {
    Verified,
    Quarantined {
        expires_at_unix: i64,
        reason_code: CregFindingCategoryV1,
    },
    Revoked {
        /// True when the revocation carried a committee certificate or was a
        /// self-revocation; false for legacy single-validator revocations.
        certified: bool,
    },
}

/// Leaf of the status tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusLeafV1 {
    /// `<ecosystem>/<name>` (canonical without the version).
    pub package: String,
    pub version: String,
    pub content_hash: String,
    pub status: StatusLeafStateV1,
    /// Height of the block that produced this state.
    pub height: u64,
}

impl StatusLeafV1 {
    /// Value hash stored in the version tree.
    pub fn value_hash(&self) -> Hash32 {
        let bytes = crate::consensus_json_bytes(self).unwrap_or_default();
        sha256(&[LEAF_VALUE_DOMAIN, &bytes])
    }

    /// Value hash parameterized by a chain spec's `consensus_encoding` so
    /// standalone clients without `phase-c-canonical-transactions` can verify
    /// `StatusProofV2` against Phase C roots (`ECA-08`).
    pub fn value_hash_for_encoding(&self, encoding: Option<&str>) -> anyhow::Result<Hash32> {
        let bytes = crate::consensus_json_bytes_for_encoding(self, encoding)?;
        Ok(sha256(&[LEAF_VALUE_DOMAIN, &bytes]))
    }
}

pub fn package_key(package: &str) -> Hash32 {
    sha256(&[PACKAGE_KEY_DOMAIN, &[0], package.trim().as_bytes()])
}

pub fn version_key(version: &str) -> Hash32 {
    sha256(&[VERSION_KEY_DOMAIN, &[0], version.trim().as_bytes()])
}

/// Split a canonical `ecosystem/name@version` id at the last `@` that is not
/// the leading `@` of a scoped npm name.
pub fn split_canonical(canonical: &str) -> Option<(&str, &str)> {
    let at = canonical.rfind('@')?;
    let (package, version) = (&canonical[..at], &canonical[at + 1..]);
    if package.is_empty() || version.is_empty() || package.ends_with('/') {
        return None;
    }
    Some((package, version))
}

/// Proof that a `(package, version)` has a given leaf (or none) under a
/// `status_root`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusProofV2 {
    pub package_proof: SmtProof,
    /// Root of the package's version tree; absent when the package is absent.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "hex32_opt")]
    pub version_root: Option<Hash32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_proof: Option<SmtProof>,
}

impl StatusProofV2 {
    /// Verify against `status_root`: `Some(leaf)` proves that exact leaf is
    /// current, `None` proves `(package, version)` has no committed status.
    pub fn verify(
        &self,
        status_root: &Hash32,
        package: &str,
        version: &str,
        leaf: Option<&StatusLeafV1>,
    ) -> bool {
        let leaf_value = leaf.map(StatusLeafV1::value_hash);
        self.verify_with_leaf_value(status_root, package, version, leaf, leaf_value.as_ref())
    }

    /// Verify against `status_root` using the signed chain spec's explicit
    /// `consensus_encoding`.
    pub fn verify_for_encoding(
        &self,
        status_root: &Hash32,
        package: &str,
        version: &str,
        leaf: Option<&StatusLeafV1>,
        encoding: Option<&str>,
    ) -> bool {
        let leaf_value = match leaf {
            Some(leaf) => match leaf.value_hash_for_encoding(encoding) {
                Ok(hash) => Some(hash),
                Err(_) => return false,
            },
            None => None,
        };
        self.verify_with_leaf_value(status_root, package, version, leaf, leaf_value.as_ref())
    }

    fn verify_with_leaf_value(
        &self,
        status_root: &Hash32,
        package: &str,
        version: &str,
        leaf: Option<&StatusLeafV1>,
        leaf_value: Option<&Hash32>,
    ) -> bool {
        if let Some(leaf) = leaf {
            if leaf.package.trim() != package.trim() || leaf.version.trim() != version.trim() {
                return false;
            }
        }
        let package_key = package_key(package);
        match (&self.version_root, &self.version_proof) {
            (None, None) => {
                // Package absent: only valid for an absence claim.
                leaf.is_none()
                    && verify_smt_proof(status_root, &package_key, None, &self.package_proof)
            }
            (Some(version_root), Some(version_proof)) => {
                verify_smt_proof(
                    status_root,
                    &package_key,
                    Some(version_root),
                    &self.package_proof,
                ) && verify_smt_proof(
                    version_root,
                    &version_key(version),
                    leaf_value,
                    version_proof,
                )
            }
            _ => false,
        }
    }
}

/// Status tree over every package version with a committed status.
#[derive(Debug, Clone, Default)]
pub struct StatusTree {
    packages: BTreeMap<String, BTreeMap<String, StatusLeafV1>>,
}

impl StatusTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a leaf.
    pub fn upsert(&mut self, leaf: StatusLeafV1) {
        self.packages
            .entry(leaf.package.trim().to_string())
            .or_default()
            .insert(leaf.version.trim().to_string(), leaf);
    }

    pub fn get(&self, package: &str, version: &str) -> Option<&StatusLeafV1> {
        self.packages.get(package.trim())?.get(version.trim())
    }

    pub fn len(&self) -> usize {
        self.packages.values().map(BTreeMap::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    fn version_tree(
        versions: &BTreeMap<String, StatusLeafV1>,
        explicit_encoding: Option<Option<&str>>,
    ) -> anyhow::Result<SparseMerkleTree> {
        let mut tree = SparseMerkleTree::new();
        for (version, leaf) in versions {
            let value = match explicit_encoding {
                Some(encoding) => leaf.value_hash_for_encoding(encoding)?,
                None => leaf.value_hash(),
            };
            tree.insert(version_key(version), value);
        }
        Ok(tree)
    }

    fn package_tree(
        &self,
        explicit_encoding: Option<Option<&str>>,
    ) -> anyhow::Result<SparseMerkleTree> {
        let mut tree = SparseMerkleTree::new();
        for (package, versions) in &self.packages {
            tree.insert(
                package_key(package),
                Self::version_tree(versions, explicit_encoding)?.root(),
            );
        }
        Ok(tree)
    }

    /// `BlockHeader.status_root`.
    pub fn root(&self) -> Hash32 {
        self.package_tree(None)
            .expect("default encoding leaf serialization cannot fail")
            .root()
    }

    /// `BlockHeader.status_root` under an explicit spec `consensus_encoding`.
    pub fn root_for_encoding(&self, encoding: Option<&str>) -> anyhow::Result<Hash32> {
        Ok(self.package_tree(Some(encoding))?.root())
    }

    pub fn prove(&self, package: &str, version: &str) -> StatusProofV2 {
        self.prove_inner(package, version, None)
            .expect("default encoding leaf serialization cannot fail")
    }

    /// Build a membership or absence proof under an explicit spec `consensus_encoding`.
    pub fn prove_for_encoding(
        &self,
        package: &str,
        version: &str,
        encoding: Option<&str>,
    ) -> anyhow::Result<StatusProofV2> {
        self.prove_inner(package, version, Some(encoding))
    }

    fn prove_inner(
        &self,
        package: &str,
        version: &str,
        explicit_encoding: Option<Option<&str>>,
    ) -> anyhow::Result<StatusProofV2> {
        let package_proof = self
            .package_tree(explicit_encoding)?
            .prove(&package_key(package));
        match self.packages.get(package.trim()) {
            None => Ok(StatusProofV2 {
                package_proof,
                version_root: None,
                version_proof: None,
            }),
            Some(versions) => {
                let versions_tree = Self::version_tree(versions, explicit_encoding)?;
                Ok(StatusProofV2 {
                    package_proof,
                    version_root: Some(versions_tree.root()),
                    version_proof: Some(versions_tree.prove(&version_key(version))),
                })
            }
        }
    }
}

/// Lowercase hex form of a 32-byte hash (the wire form of `status_root`).
pub fn hex32_to_string(hash: &Hash32) -> String {
    hex::encode(hash)
}

/// Serde helpers: 32-byte hashes as 64-char lowercase hex.
pub mod hex32 {
    use super::Hash32;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Hash32, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Hash32, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse(&text).map_err(serde::de::Error::custom)
    }

    pub fn parse(text: &str) -> Result<Hash32, String> {
        let text = text.trim().trim_start_matches("0x");
        if text.len() != 64 {
            return Err("expected 64 hex chars".to_string());
        }
        let bytes = hex::decode(text).map_err(|error| error.to_string())?;
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(out)
    }
}

/// `Option<Hash32>` as hex or absent.
pub mod hex32_opt {
    use super::Hash32;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<Hash32>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(hash) => serializer.serialize_some(&hex::encode(hash)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Hash32>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|text| super::hex32::parse(&text).map_err(serde::de::Error::custom))
            .transpose()
    }
}

mod hex32_opt_vec {
    use super::Hash32;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        value: &[Option<Hash32>],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let encoded: Vec<Option<String>> = value.iter().map(|hash| hash.map(hex::encode)).collect();
        encoded.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Option<Hash32>>, D::Error> {
        let raw = Vec::<Option<String>>::deserialize(deserializer)?;
        if raw.len() > 256 {
            return Err(serde::de::Error::custom("proof has more than 256 siblings"));
        }
        raw.into_iter()
            .map(|entry| {
                entry
                    .map(|text| super::hex32::parse(&text).map_err(serde::de::Error::custom))
                    .transpose()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(package: &str, version: &str, status: StatusLeafStateV1, height: u64) -> StatusLeafV1 {
        StatusLeafV1 {
            package: package.to_string(),
            version: version.to_string(),
            content_hash: "ab".repeat(32),
            status,
            height,
        }
    }

    #[test]
    fn smt_membership_and_absence_proofs_verify() {
        let mut tree = SparseMerkleTree::new();
        assert_eq!(tree.root(), EMPTY_HASH);
        let keys: Vec<Hash32> = (0u8..40).map(|i| sha256(&[&[i]])).collect();
        for (index, key) in keys.iter().enumerate() {
            tree.insert(*key, sha256(&[b"value", &[index as u8]]));
        }
        let root = tree.root();
        for key in &keys {
            let proof = tree.prove(key);
            let value = tree.get(key).copied().unwrap();
            assert!(verify_smt_proof(&root, key, Some(&value), &proof));
            // Wrong value or an absence claim fails.
            assert!(!verify_smt_proof(&root, key, Some(&EMPTY_HASH), &proof));
            assert!(!verify_smt_proof(&root, key, None, &proof));
            assert!(proof.siblings.len() < 20, "compact paths are short");
        }
        for absent in (100u8..140).map(|i| sha256(&[&[i]])) {
            let proof = tree.prove(&absent);
            assert!(verify_smt_proof(&root, &absent, None, &proof));
            assert!(!verify_smt_proof(
                &root,
                &absent,
                Some(&sha256(&[b"x"])),
                &proof
            ));
        }
        // A present key cannot be proven absent with another key's proof.
        let other = tree.prove(&sha256(&[&[200]]));
        assert!(!verify_smt_proof(&root, &keys[0], None, &other));
    }

    #[test]
    fn single_entry_and_tampered_proofs() {
        let mut tree = SparseMerkleTree::new();
        let key = sha256(&[b"only"]);
        tree.insert(key, sha256(&[b"v"]));
        let proof = tree.prove(&key);
        assert!(proof.siblings.is_empty());
        assert_eq!(tree.root(), leaf_hash(&key, &sha256(&[b"v"])));
        assert!(verify_smt_proof(
            &tree.root(),
            &key,
            Some(&sha256(&[b"v"])),
            &proof
        ));

        let mut big = SparseMerkleTree::new();
        for i in 0u8..10 {
            big.insert(sha256(&[&[i]]), sha256(&[&[i, i]]));
        }
        let target = sha256(&[&[3]]);
        let mut tampered = big.prove(&target);
        tampered.siblings[0] = Some(sha256(&[b"forged"]));
        assert!(!verify_smt_proof(
            &big.root(),
            &target,
            Some(&sha256(&[&[3, 3]])),
            &tampered
        ));
        let mut too_long = big.prove(&target);
        too_long.siblings = vec![None; 257];
        assert!(!verify_smt_proof(&big.root(), &target, None, &too_long));
    }

    #[test]
    fn root_is_order_independent_and_changes_with_state() {
        let mut first = StatusTree::new();
        let mut second = StatusTree::new();
        let leaves = vec![
            leaf("npm/react", "19.0.0", StatusLeafStateV1::Verified, 10),
            leaf("npm/@scope/pkg", "1.0.0", StatusLeafStateV1::Verified, 11),
            leaf(
                "npm/left-padx",
                "1.4.2",
                StatusLeafStateV1::Quarantined {
                    expires_at_unix: 1_800_000_000,
                    reason_code: CregFindingCategoryV1::CredentialExfiltration,
                },
                12,
            ),
        ];
        for leaf in &leaves {
            first.upsert(leaf.clone());
        }
        for leaf in leaves.iter().rev() {
            second.upsert(leaf.clone());
        }
        assert_eq!(first.root(), second.root());
        let before = first.root();
        first.upsert(leaf(
            "npm/left-padx",
            "1.4.2",
            StatusLeafStateV1::Revoked { certified: true },
            13,
        ));
        assert_ne!(first.root(), before);
    }

    #[test]
    fn eco_adv_03_status_proofs_bind_the_current_state() {
        let mut tree = StatusTree::new();
        tree.upsert(leaf(
            "npm/express",
            "4.17.1",
            StatusLeafStateV1::Revoked { certified: true },
            50,
        ));
        tree.upsert(leaf(
            "npm/express",
            "5.0.0",
            StatusLeafStateV1::Verified,
            60,
        ));
        tree.upsert(leaf(
            "pypi/requests",
            "2.31.0",
            StatusLeafStateV1::Verified,
            61,
        ));
        let root = tree.root();

        let revoked = tree.get("npm/express", "4.17.1").cloned().unwrap();
        let proof = tree.prove("npm/express", "4.17.1");
        assert!(proof.verify(&root, "npm/express", "4.17.1", Some(&revoked)));

        // A lying node claims the revoked version is verified: rejected.
        let mut lie = revoked.clone();
        lie.status = StatusLeafStateV1::Verified;
        assert!(!proof.verify(&root, "npm/express", "4.17.1", Some(&lie)));
        // Nor can it claim the version has no status.
        assert!(!proof.verify(&root, "npm/express", "4.17.1", None));
        // Nor reuse the 5.0.0 proof for 4.17.1.
        let other = tree.prove("npm/express", "5.0.0");
        assert!(!other.verify(&root, "npm/express", "4.17.1", Some(&revoked)));

        // Absent version of a present package, and an absent package.
        let missing_version = tree.prove("npm/express", "9.9.9");
        assert!(missing_version.verify(&root, "npm/express", "9.9.9", None));
        let missing_package = tree.prove("npm/never-published", "1.0.0");
        assert!(missing_package.version_root.is_none());
        assert!(missing_package.verify(&root, "npm/never-published", "1.0.0", None));

        // Proofs survive JSON round trips.
        let json = serde_json::to_string(&proof).unwrap();
        let back: StatusProofV2 = serde_json::from_str(&json).unwrap();
        assert!(back.verify(&root, "npm/express", "4.17.1", Some(&revoked)));
    }

    #[test]
    fn canonical_ids_split_at_the_version_separator() {
        assert_eq!(
            split_canonical("npm/react@19.0.0"),
            Some(("npm/react", "19.0.0"))
        );
        assert_eq!(
            split_canonical("npm/@scope/pkg@1.0.0"),
            Some(("npm/@scope/pkg", "1.0.0"))
        );
        assert_eq!(split_canonical("npm/@scope/pkg"), None);
        assert_eq!(split_canonical("no-version"), None);
        assert_eq!(split_canonical("npm/x@"), None);
    }

    #[test]
    fn leaf_json_is_tagged_and_strict() {
        let value = serde_json::to_value(leaf(
            "npm/a",
            "1.0.0",
            StatusLeafStateV1::Revoked { certified: false },
            7,
        ))
        .unwrap();
        assert_eq!(value["status"]["state"], "revoked");
        assert_eq!(value["status"]["certified"], false);
        assert_eq!(value["height"], 7);
    }

    #[test]
    fn explicit_phase_c_encoding_proofs_work_without_compile_feature() {
        let entry = leaf("npm/left-pad", "1.3.0", StatusLeafStateV1::Verified, 9);
        let phase_c = Some(crate::PHASE_C_CONSENSUS_ENCODING);
        assert_ne!(
            entry.value_hash_for_encoding(phase_c).unwrap(),
            entry.value_hash_for_encoding(None).unwrap()
        );
        let mut tree = StatusTree::new();
        tree.upsert(entry.clone());
        let root = tree.root_for_encoding(phase_c).unwrap();
        let proof = tree
            .prove_for_encoding("npm/left-pad", "1.3.0", phase_c)
            .unwrap();
        assert!(proof.verify_for_encoding(&root, "npm/left-pad", "1.3.0", Some(&entry), phase_c));
        assert!(!proof.verify_for_encoding(&root, "npm/left-pad", "1.3.0", Some(&entry), None));
    }
}
