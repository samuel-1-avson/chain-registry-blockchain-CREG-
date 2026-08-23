// crates/cli/src/supply_chain.rs
// SUPPLY-CHAIN-CI (in-repo slice): Cargo.lock inventory, deny policy, and a
// lockfile-bound provenance statement for this workspace.
//
// This is not SLSA, not Sigstore, and not a claim about submitted registry
// packages. Unsigned/incomplete package SBOMs stay incomplete. Independent
// builder identity remains out of scope.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const DEFAULT_POLICY_REL: &str = "config/supply-chain/policy.toml";

#[derive(Debug, Clone, Deserialize)]
pub struct SupplyChainPolicy {
    pub schema_version: u32,
    pub sources: SourcePolicy,
    pub checksums: ChecksumPolicy,
    #[serde(default)]
    pub bans: BanPolicy,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourcePolicy {
    pub allow_registry_index: Vec<String>,
    #[serde(default)]
    pub allow_path: bool,
    #[serde(default)]
    pub allow_git: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChecksumPolicy {
    pub require_for_registry: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BanPolicy {
    #[serde(default)]
    pub crate_names: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CargoLockFile {
    #[serde(default)]
    package: Vec<LockPackage>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct LockPackage {
    pub name: String,
    pub version: String,
    pub source: Option<String>,
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyViolation {
    pub crate_name: String,
    pub version: String,
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceProvenance {
    pub schema: &'static str,
    pub slsa: bool,
    pub note: &'static str,
    pub lockfile_sha256: String,
    pub package_count: usize,
    pub generated_at: String,
}

impl WorkspaceProvenance {
    pub fn schema_id() -> &'static str {
        "creg-workspace-provenance-v1"
    }
}

pub fn parse_cargo_lock(text: &str) -> Result<Vec<LockPackage>> {
    let parsed: CargoLockFile = toml::from_str(text).context("Cargo.lock is not valid TOML")?;
    if parsed.package.is_empty() {
        bail!("Cargo.lock contains no [[package]] entries");
    }
    Ok(parsed.package)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub fn load_policy_file(path: &Path) -> Result<SupplyChainPolicy> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read supply-chain policy {}", path.display()))?;
    let policy: SupplyChainPolicy =
        toml::from_str(&text).context("supply-chain policy is not valid TOML")?;
    if policy.schema_version != 1 {
        bail!(
            "unsupported supply-chain policy schema_version {}",
            policy.schema_version
        );
    }
    Ok(policy)
}

pub fn default_policy() -> SupplyChainPolicy {
    SupplyChainPolicy {
        schema_version: 1,
        sources: SourcePolicy {
            allow_registry_index: vec!["https://github.com/rust-lang/crates.io-index".into()],
            allow_path: true,
            allow_git: false,
        },
        checksums: ChecksumPolicy {
            require_for_registry: true,
        },
        bans: BanPolicy {
            crate_names: Vec::new(),
        },
    }
}

pub fn check_lock_policy(
    packages: &[LockPackage],
    policy: &SupplyChainPolicy,
) -> Vec<PolicyViolation> {
    let banned: BTreeSet<String> = policy
        .bans
        .crate_names
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let mut violations = Vec::new();
    for package in packages {
        if banned.contains(&package.name.to_ascii_lowercase()) {
            violations.push(PolicyViolation {
                crate_name: package.name.clone(),
                version: package.version.clone(),
                code: "banned_crate",
                message: format!(
                    "{}@{} is banned by supply-chain policy",
                    package.name, package.version
                ),
            });
        }
        match package.source.as_deref() {
            None => {
                if !policy.sources.allow_path {
                    violations.push(PolicyViolation {
                        crate_name: package.name.clone(),
                        version: package.version.clone(),
                        code: "path_source_denied",
                        message: format!(
                            "{}@{} is a path/workspace crate, which policy forbids",
                            package.name, package.version
                        ),
                    });
                }
            }
            Some(source) if source.starts_with("registry+") => {
                let index = source.trim_start_matches("registry+");
                if !policy
                    .sources
                    .allow_registry_index
                    .iter()
                    .any(|allowed| allowed == index)
                {
                    violations.push(PolicyViolation {
                        crate_name: package.name.clone(),
                        version: package.version.clone(),
                        code: "registry_source_denied",
                        message: format!(
                            "{}@{} uses registry index '{}', which is not allowlisted",
                            package.name, package.version, index
                        ),
                    });
                }
                if policy.checksums.require_for_registry
                    && package
                        .checksum
                        .as_deref()
                        .map(str::trim)
                        .unwrap_or("")
                        .is_empty()
                {
                    violations.push(PolicyViolation {
                        crate_name: package.name.clone(),
                        version: package.version.clone(),
                        code: "missing_checksum",
                        message: format!(
                            "{}@{} is a registry crate without a Cargo.lock checksum",
                            package.name, package.version
                        ),
                    });
                }
            }
            Some(source) if source.starts_with("git+") => {
                if !policy.sources.allow_git {
                    violations.push(PolicyViolation {
                        crate_name: package.name.clone(),
                        version: package.version.clone(),
                        code: "git_source_denied",
                        message: format!(
                            "{}@{} is a git dependency, which policy forbids",
                            package.name, package.version
                        ),
                    });
                }
            }
            Some(source) => violations.push(PolicyViolation {
                crate_name: package.name.clone(),
                version: package.version.clone(),
                code: "unknown_source",
                message: format!(
                    "{}@{} has unsupported source '{source}'",
                    package.name, package.version
                ),
            }),
        }
    }
    violations
}

pub fn workspace_provenance(lock_bytes: &[u8], packages: &[LockPackage]) -> WorkspaceProvenance {
    WorkspaceProvenance {
        schema: WorkspaceProvenance::schema_id(),
        slsa: false,
        note: "Lockfile-binding local statement. Not SLSA provenance and not a builder identity.",
        lockfile_sha256: sha256_hex(lock_bytes),
        package_count: packages.len(),
        generated_at: Utc::now().to_rfc3339(),
    }
}

pub fn verify_workspace_provenance(
    lock_bytes: &[u8],
    statement: &WorkspaceProvenance,
) -> Result<()> {
    if statement.schema != WorkspaceProvenance::schema_id() {
        bail!("unsupported provenance schema {}", statement.schema);
    }
    if statement.slsa {
        bail!("workspace provenance must not claim SLSA");
    }
    let expected = sha256_hex(lock_bytes);
    if statement.lockfile_sha256 != expected {
        bail!(
            "provenance lockfile_sha256 {} does not match current Cargo.lock {}",
            statement.lockfile_sha256,
            expected
        );
    }
    Ok(())
}

pub fn cyclonedx_from_lock(packages: &[LockPackage], lock_sha256: &str) -> serde_json::Value {
    let components: Vec<serde_json::Value> = packages
        .iter()
        .map(|package| {
            let mut component = serde_json::json!({
                "type": if package.source.is_none() { "application" } else { "library" },
                "name": package.name,
                "version": package.version,
                "bom-ref": format!("pkg:cargo/{}@{}", package.name, package.version),
                "purl": format!("pkg:cargo/{}@{}", package.name, package.version),
            });
            if let Some(checksum) = package
                .checksum
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                component["hashes"] =
                    serde_json::json!([{ "alg": "SHA-256", "content": checksum }]);
            }
            if let Some(source) = &package.source {
                component["properties"] = serde_json::json!([
                    { "name": "creg:cargo-source", "value": source }
                ]);
            }
            component
        })
        .collect();
    serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "timestamp": Utc::now().to_rfc3339(),
            "tools": {
                "components": [{
                    "type": "application",
                    "name": "creg",
                    "version": env!("CARGO_PKG_VERSION")
                }]
            },
            "properties": [
                { "name": "creg:completeness", "value": "cargo-lock-inventory" },
                { "name": "creg:lockfile-sha256", "value": lock_sha256 },
                { "name": "creg:slsa", "value": "false" }
            ]
        },
        "components": components
    })
}

pub fn spdx_from_lock(packages: &[LockPackage], lock_sha256: &str) -> serde_json::Value {
    let spdx_packages: Vec<serde_json::Value> = packages
        .iter()
        .enumerate()
        .map(|(index, package)| {
            let mut entry = serde_json::json!({
                "SPDXID": format!("SPDXRef-Package-{index}"),
                "name": package.name,
                "versionInfo": package.version,
                "downloadLocation": package.source.clone().unwrap_or_else(|| "NOASSERTION".into()),
                "filesAnalyzed": false,
            });
            if let Some(checksum) = package
                .checksum
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                entry["checksums"] = serde_json::json!([{
                    "algorithm": "SHA256",
                    "checksumValue": checksum
                }]);
            }
            entry
        })
        .collect();
    let relationships: Vec<serde_json::Value> = (0..packages.len())
        .map(|index| {
            serde_json::json!({
                "spdxElementId": "SPDXRef-DOCUMENT",
                "relationshipType": "DESCRIBES",
                "relatedSpdxElement": format!("SPDXRef-Package-{index}")
            })
        })
        .collect();
    serde_json::json!({
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": "creg-workspace-sbom",
        "documentNamespace": format!("https://creg.dev/sbom/workspace/{lock_sha256}"),
        "creationInfo": {
            "created": Utc::now().to_rfc3339(),
            "creators": [format!("Tool: creg/{}", env!("CARGO_PKG_VERSION"))],
            "comment": "Generated from Cargo.lock. Not a claim about submitted registry packages."
        },
        "comment": format!("creg:completeness=cargo-lock-inventory; creg:lockfile-sha256={lock_sha256}; creg:slsa=false"),
        "packages": spdx_packages,
        "relationships": relationships
    })
}

pub fn find_workspace_lock(start: &Path) -> Result<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join("Cargo.lock");
        if candidate.is_file() {
            return Ok(candidate);
        }
        if !dir.pop() {
            bail!("Cargo.lock not found from {}", start.display());
        }
    }
}

pub fn resolve_policy_path(lockfile: &Path, explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    lockfile
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(DEFAULT_POLICY_REL)
}

pub fn check_workspace(lockfile: &Path, policy_path: Option<&Path>) -> Result<(usize, String)> {
    let lock_bytes =
        std::fs::read(lockfile).with_context(|| format!("read {}", lockfile.display()))?;
    let lock_text = String::from_utf8(lock_bytes.clone()).context("Cargo.lock is not UTF-8")?;
    let packages = parse_cargo_lock(&lock_text)?;
    let policy_file = resolve_policy_path(lockfile, policy_path);
    let policy = if policy_file.is_file() {
        load_policy_file(&policy_file)?
    } else {
        default_policy()
    };
    let violations = check_lock_policy(&packages, &policy);
    if !violations.is_empty() {
        let details = violations
            .iter()
            .map(|v| format!("{}: {}", v.code, v.message))
            .collect::<Vec<_>>()
            .join("; ");
        bail!("supply-chain policy failed: {details}");
    }
    let provenance = workspace_provenance(&lock_bytes, &packages);
    verify_workspace_provenance(&lock_bytes, &provenance)?;
    Ok((packages.len(), provenance.lockfile_sha256))
}

pub fn emit_workspace_sbom(
    lockfile: &Path,
    format: crate::sbom::SbomFormat,
) -> Result<serde_json::Value> {
    let lock_bytes =
        std::fs::read(lockfile).with_context(|| format!("read {}", lockfile.display()))?;
    let lock_text = String::from_utf8(lock_bytes.clone()).context("Cargo.lock is not UTF-8")?;
    let packages = parse_cargo_lock(&lock_text)?;
    let digest = sha256_hex(&lock_bytes);
    Ok(match format {
        crate::sbom::SbomFormat::Spdx => spdx_from_lock(&packages, &digest),
        crate::sbom::SbomFormat::CycloneDx => cyclonedx_from_lock(&packages, &digest),
    })
}

pub fn run_check(lockfile: Option<&Path>, policy: Option<&Path>) -> Result<()> {
    let start = std::env::current_dir().context("current directory")?;
    let lock = match lockfile {
        Some(path) => path.to_path_buf(),
        None => find_workspace_lock(&start)?,
    };
    let (count, digest) = check_workspace(&lock, policy)?;
    println!(
        "supply-chain: {} crates in {} (sha256:{digest}) passed policy",
        count,
        lock.display()
    );
    Ok(())
}

pub fn run_workspace_sbom(
    format: crate::sbom::SbomFormat,
    output: Option<&Path>,
    lockfile: Option<&Path>,
) -> Result<()> {
    let start = std::env::current_dir().context("current directory")?;
    let lock = match lockfile {
        Some(path) => path.to_path_buf(),
        None => find_workspace_lock(&start)?,
    };
    let doc = emit_workspace_sbom(&lock, format)?;
    let pretty = serde_json::to_string_pretty(&doc)?;
    if let Some(path) = output {
        std::fs::write(path, &pretty).with_context(|| format!("write SBOM {}", path.display()))?;
        println!("workspace SBOM saved to {}", path.display());
    } else {
        println!("{pretty}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
version = 3

[[package]]
name = "workspace-pkg"
version = "0.1.0"

[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"#;

    fn serde_pkg() -> LockPackage {
        LockPackage {
            name: "serde".into(),
            version: "1.0.0".into(),
            source: Some("registry+https://github.com/rust-lang/crates.io-index".into()),
            checksum: Some("aa".repeat(32)),
        }
    }

    #[test]
    fn lockfile_parser_keeps_workspace_and_registry_crates() {
        let packages = parse_cargo_lock(FIXTURE).expect("parse");
        assert_eq!(packages.len(), 2);
        assert!(packages[0].source.is_none());
        assert_eq!(packages[1].name, "serde");
        assert!(packages[1].checksum.as_ref().unwrap().len() == 64);
    }

    #[test]
    fn empty_lockfile_fails_closed() {
        let err = parse_cargo_lock("version = 3\n").unwrap_err();
        assert!(err.to_string().contains("no [[package]]"));
    }

    #[test]
    fn registry_crate_without_checksum_is_rejected() {
        let packages = vec![LockPackage {
            name: "evil".into(),
            version: "1.0.0".into(),
            source: Some("registry+https://github.com/rust-lang/crates.io-index".into()),
            checksum: None,
        }];
        let hits = check_lock_policy(&packages, &default_policy());
        assert!(hits.iter().any(|v| v.code == "missing_checksum"));
    }

    #[test]
    fn git_dependency_is_rejected_by_default() {
        let packages = vec![LockPackage {
            name: "fork".into(),
            version: "1.0.0".into(),
            source: Some("git+https://example.com/fork#deadbeef".into()),
            checksum: None,
        }];
        let hits = check_lock_policy(&packages, &default_policy());
        assert!(hits.iter().any(|v| v.code == "git_source_denied"));
    }

    #[test]
    fn banned_crate_is_rejected() {
        let mut policy = default_policy();
        policy.bans.crate_names.push("serde".into());
        let hits = check_lock_policy(&[serde_pkg()], &policy);
        assert!(hits.iter().any(|v| v.code == "banned_crate"));
    }

    #[test]
    fn unknown_registry_index_is_rejected() {
        let packages = vec![LockPackage {
            name: "mirror".into(),
            version: "1.0.0".into(),
            source: Some("registry+https://evil.example/index".into()),
            checksum: Some("bb".repeat(32)),
        }];
        let hits = check_lock_policy(&packages, &default_policy());
        assert!(hits.iter().any(|v| v.code == "registry_source_denied"));
    }

    #[test]
    fn fixture_satisfies_default_policy() {
        let packages = parse_cargo_lock(FIXTURE).unwrap();
        assert!(check_lock_policy(&packages, &default_policy()).is_empty());
    }

    #[test]
    fn provenance_binds_lockfile_bytes_and_refuses_slsa_claim() {
        let packages = parse_cargo_lock(FIXTURE).unwrap();
        let mut statement = workspace_provenance(FIXTURE.as_bytes(), &packages);
        verify_workspace_provenance(FIXTURE.as_bytes(), &statement).unwrap();
        statement.lockfile_sha256 = "00".repeat(32);
        assert!(verify_workspace_provenance(FIXTURE.as_bytes(), &statement).is_err());
        statement = workspace_provenance(FIXTURE.as_bytes(), &packages);
        statement.slsa = true;
        assert!(verify_workspace_provenance(FIXTURE.as_bytes(), &statement).is_err());
        assert!(verify_workspace_provenance(
            b"tampered",
            &workspace_provenance(FIXTURE.as_bytes(), &packages)
        )
        .is_err());
    }

    #[test]
    fn cyclonedx_inventory_lists_every_lock_entry() {
        let packages = parse_cargo_lock(FIXTURE).unwrap();
        let bom = cyclonedx_from_lock(&packages, "abcd");
        assert_eq!(bom["specVersion"], "1.5");
        assert_eq!(bom["components"].as_array().unwrap().len(), 2);
        assert_eq!(
            bom["metadata"]["properties"][0]["value"],
            "cargo-lock-inventory"
        );
        assert_eq!(bom["metadata"]["properties"][2]["value"], "false");
    }

    #[test]
    fn workspace_lock_and_committed_policy_pass() {
        let lock = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
        let policy =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/supply-chain/policy.toml");
        let (count, digest) = check_workspace(&lock, Some(&policy)).expect("workspace check");
        assert!(
            count > 50,
            "expected a full Cargo.lock inventory, got {count}"
        );
        assert_eq!(digest.len(), 64);
        let bom = emit_workspace_sbom(&lock, crate::sbom::SbomFormat::CycloneDx).unwrap();
        assert_eq!(bom["components"].as_array().unwrap().len(), count);
    }
}
