//! Authoritative package-manifest audit & lifecycle hook extractor (`VNA-10`).
//!
//! Resolves package identity authoritatively from the signed [`PackageId`] when
//! supplied (never letting arbitrary tarball iteration order switch ecosystems),
//! flags foreign/conflicting manifests (`ST-ECOSYSTEM-SPOOF`), and audits npm,
//! Cargo, PyPI, Maven, and RubyGems build/install hooks across raw bytes.

use common::{Finding, FindingSeverity, PackageId, PackageManifest};
use creg_unpack::{normalize_ecosystem, UnpackedArchive, UnpackedFile};
use std::collections::BTreeSet;

/// Output of deterministic manifest auditing across an unpacked archive.
#[derive(Debug, Clone, Default)]
pub struct ManifestAuditResult {
    pub pkg_name: String,
    pub pkg_version: String,
    pub ecosystem: String,
    pub hook_files: Vec<String>,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestKind {
    NpmPackageJson,
    CargoToml,
    PyProjectToml,
    PySetupPy,
    PySetupCfg,
    MavenPomXml,
    RubyGemspec,
}

impl ManifestKind {
    fn ecosystem(self) -> &'static str {
        match self {
            Self::NpmPackageJson => "npm",
            Self::CargoToml => "cargo",
            Self::PyProjectToml | Self::PySetupPy | Self::PySetupCfg => "pypi",
            Self::MavenPomXml => "maven",
            Self::RubyGemspec => "rubygems",
        }
    }
}

fn classify_manifest(path: &str) -> Option<(ManifestKind, usize)> {
    let depth = path.split('/').count();
    let lower = path.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or("");
    let kind = match base {
        "package.json" => ManifestKind::NpmPackageJson,
        "cargo.toml" => ManifestKind::CargoToml,
        "pyproject.toml" => ManifestKind::PyProjectToml,
        "setup.py" => ManifestKind::PySetupPy,
        "setup.cfg" => ManifestKind::PySetupCfg,
        "pom.xml" => ManifestKind::MavenPomXml,
        _ if base.ends_with(".gemspec") => ManifestKind::RubyGemspec,
        _ => return None,
    };
    Some((kind, depth))
}

/// Audits manifests and lifecycle hooks inside `archive`.
///
/// When `authoritative_id` is `Some(id)` (consensus validation), `ecosystem` is
/// bound strictly to `id.ecosystem`, and any shallow foreign-ecosystem manifest
/// or conflicting package name/version triggers `ST-ECOSYSTEM-SPOOF`.
pub fn audit(
    archive: &UnpackedArchive,
    authoritative_id: Option<&PackageId>,
    declared_manifest: &PackageManifest,
) -> ManifestAuditResult {
    let mut manifests: Vec<(ManifestKind, usize, &UnpackedFile)> = Vec::new();
    for file in &archive.files {
        if let Some((kind, depth)) = classify_manifest(&file.path) {
            manifests.push((kind, depth, file));
        }
    }
    // Canonical depth-first, lexicographical tie-break ordering (never raw tar order).
    manifests.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.path.cmp(&b.2.path)));

    let mut findings = Vec::new();
    let mut hook_files = BTreeSet::new();

    let auth_eco = authoritative_id
        .map(|id| normalize_ecosystem(&id.ecosystem))
        .filter(|&eco| eco != "unknown");

    let (mut pkg_name, mut pkg_version, mut ecosystem) = match authoritative_id {
        Some(id) => (
            id.name.clone(),
            id.version.clone(),
            normalize_ecosystem(&id.ecosystem).to_string(),
        ),
        None => (String::new(), String::new(), String::new()),
    };

    // Flag conflicting shallow manifests when an authoritative ecosystem is known.
    if let Some(expected_eco) = auth_eco {
        for &(kind, depth, file) in &manifests {
            if depth <= 2 && kind.ecosystem() != expected_eco {
                findings.push(Finding {
                    id: "ST-ECOSYSTEM-SPOOF".into(),
                    title: "Conflicting foreign ecosystem manifest in package".into(),
                    severity: FindingSeverity::High,
                    description: format!(
                        "Submission claims ecosystem '{}' but contains shallow '{}' manifest at '{}' (potential ecosystem spoofing).",
                        expected_eco,
                        kind.ecosystem(),
                        file.path
                    ),
                    file: file.path.clone(),
                    line: None,
                });
            }
        }
    }

    // Extract identity & run ecosystem-specific manifest audits.
    for &(kind, depth, file) in &manifests {
        let text = file.text_lossy();
        match kind {
            ManifestKind::NpmPackageJson => {
                if depth <= 2 {
                    audit_package_json(
                        &file.path,
                        &text,
                        authoritative_id,
                        declared_manifest,
                        &mut pkg_name,
                        &mut pkg_version,
                        &mut ecosystem,
                        &mut hook_files,
                        &mut findings,
                    );
                }
            }
            ManifestKind::CargoToml => {
                if depth <= 2 {
                    audit_cargo_toml(
                        &file.path,
                        &text,
                        authoritative_id,
                        archive,
                        &mut pkg_name,
                        &mut pkg_version,
                        &mut ecosystem,
                        &mut hook_files,
                        &mut findings,
                    );
                }
            }
            ManifestKind::PyProjectToml => {
                if depth <= 2 {
                    audit_pyproject_toml(
                        &file.path,
                        &text,
                        authoritative_id,
                        &mut pkg_name,
                        &mut pkg_version,
                        &mut ecosystem,
                        &mut findings,
                    );
                }
            }
            ManifestKind::PySetupPy | ManifestKind::PySetupCfg => {
                if depth <= 2 {
                    audit_setup_py_or_cfg(
                        &file.path,
                        kind,
                        &text,
                        &mut pkg_name,
                        &mut pkg_version,
                        &mut ecosystem,
                        &mut hook_files,
                        &mut findings,
                    );
                }
            }
            ManifestKind::MavenPomXml => {
                if depth <= 2 {
                    if ecosystem.is_empty() {
                        ecosystem = "maven".to_string();
                    }
                    let lower = text.to_ascii_lowercase();
                    if lower.contains("exec-maven-plugin") || lower.contains("maven-antrun-plugin")
                    {
                        findings.push(Finding {
                            id: "ST-MANIFEST-HOOK".into(),
                            title: "Maven POM configures arbitrary command plugin".into(),
                            severity: FindingSeverity::High,
                            description: format!(
                                "Maven manifest '{}' declares exec/antrun execution plugin.",
                                file.path
                            ),
                            file: file.path.clone(),
                            line: None,
                        });
                    }
                }
            }
            ManifestKind::RubyGemspec => {
                if depth <= 2 {
                    if ecosystem.is_empty() {
                        ecosystem = "rubygems".to_string();
                    }
                    let lower = text.to_ascii_lowercase();
                    if lower.contains(".extensions") {
                        findings.push(Finding {
                            id: "ST-MANIFEST-HOOK".into(),
                            title: "RubyGems gemspec declares native build extensions".into(),
                            severity: FindingSeverity::High,
                            description: format!(
                                "Gemspec '{}' configures build-time native extensions.",
                                file.path
                            ),
                            file: file.path.clone(),
                            line: None,
                        });
                    }
                }
            }
        }
    }

    // Flag Python `.pth` auto-execution files anywhere in Python packages (or any package).
    for file in &archive.files {
        let depth = file.path.split('/').count();
        let lower = file.path.to_ascii_lowercase();
        if lower.ends_with(".pth") {
            hook_files.insert(file.path.clone());
            findings.push(Finding {
                id: "ST-MANIFEST-PTH-HOOK".into(),
                title: "Python .pth startup execution hook present".into(),
                severity: FindingSeverity::High,
                description: format!(
                    "Archive includes '{}' which Python automatically processes/executes on interpreter startup.",
                    file.path
                ),
                file: file.path.clone(),
                line: None,
            });
        } else if depth <= 2 && lower.ends_with("build.rs") {
            hook_files.insert(file.path.clone());
        }
    }

    if ecosystem == "unknown" || ecosystem.is_empty() {
        ecosystem = manifests
            .first()
            .map(|m| m.0.ecosystem().to_string())
            .unwrap_or_default();
    }

    ManifestAuditResult {
        pkg_name,
        pkg_version,
        ecosystem,
        hook_files: hook_files.into_iter().collect(),
        findings,
    }
}

fn parent_dir(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(parent, _)| parent)
}

fn resolve_relative_target(manifest_path: &str, token: &str) -> String {
    let clean = token.trim_matches(|c| c == '"' || c == '\'' || c == ';');
    let trimmed = clean.trim_start_matches("./");
    match parent_dir(manifest_path) {
        Some(dir) if !trimmed.starts_with(&format!("{dir}/")) => format!("{dir}/{trimmed}"),
        _ => trimmed.to_string(),
    }
}

fn has_dangerous_shell_hook(cmd_lower: &str) -> bool {
    const DANGEROUS_HOOK_PATTERNS: &[&str] = &[
        "curl ",
        "wget ",
        "eval ",
        "eval(",
        "bash -c",
        "sh -c",
        "powershell",
        "pwsh ",
        "nc -e",
        "/dev/tcp/",
        "base64 -d",
        "base64 --decode",
        "node -e",
        "python -c",
        "python3 -c",
        "chmod +x",
    ];
    DANGEROUS_HOOK_PATTERNS
        .iter()
        .any(|pat| cmd_lower.contains(pat))
}

#[allow(clippy::too_many_arguments)]
fn audit_package_json(
    path: &str,
    content: &str,
    authoritative_id: Option<&PackageId>,
    declared_manifest: &PackageManifest,
    pkg_name: &mut String,
    pkg_version: &mut String,
    ecosystem: &mut String,
    hook_files: &mut BTreeSet<String>,
    findings: &mut Vec<Finding>,
) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(content) else {
        findings.push(Finding {
            id: "ST-MANIFEST-CORRUPT".into(),
            title: "Unparseable package.json manifest".into(),
            severity: FindingSeverity::High,
            description: format!("Failed to parse '{}' as valid JSON.", path),
            file: path.to_string(),
            line: None,
        });
        return;
    };

    let parsed_name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").trim();
    let parsed_ver = v
        .get("version")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();

    if authoritative_id.is_none() && pkg_name.is_empty() && !parsed_name.is_empty() {
        *pkg_name = parsed_name.to_string();
        *pkg_version = if parsed_ver.is_empty() {
            "0.0.0".to_string()
        } else {
            parsed_ver.to_string()
        };
        *ecosystem = "npm".to_string();
    } else if let Some(auth) = authoritative_id {
        if normalize_ecosystem(&auth.ecosystem) == "npm" && !parsed_name.is_empty() {
            if !parsed_name.eq_ignore_ascii_case(&auth.name) {
                findings.push(Finding {
                    id: "ST-ECOSYSTEM-SPOOF".into(),
                    title: "package.json name does not match submitted PackageId".into(),
                    severity: FindingSeverity::High,
                    description: format!(
                        "Manifest '{}' declares name '{}' which disagrees with submitted '{}'.",
                        path, parsed_name, auth.name
                    ),
                    file: path.to_string(),
                    line: None,
                });
            }
            if !parsed_ver.is_empty()
                && !auth.version.trim().is_empty()
                && parsed_ver != auth.version
            {
                findings.push(Finding {
                    id: "ST-ECOSYSTEM-SPOOF".into(),
                    title: "package.json version does not match submitted PackageId".into(),
                    severity: FindingSeverity::High,
                    description: format!(
                        "Manifest '{}' declares version '{}' which disagrees with submitted '{}'.",
                        path, parsed_ver, auth.version
                    ),
                    file: path.to_string(),
                    line: None,
                });
            }
        }
    }

    // Lifecycle scripts (`scripts`)
    const LIFECYCLE_KEYS: &[&str] = &[
        "preinstall",
        "install",
        "postinstall",
        "prepare",
        "prepublish",
        "prepublishOnly",
        "preuninstall",
        "postuninstall",
    ];
    if let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) {
        for key in LIFECYCLE_KEYS {
            if let Some(cmd) = scripts.get(*key).and_then(|val| val.as_str()) {
                for tok in cmd.split_whitespace() {
                    let clean = tok.trim_matches(|c| c == '"' || c == '\'' || c == ';');
                    if matches!(
                        std::path::Path::new(clean)
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.to_ascii_lowercase())
                            .as_deref(),
                        Some("js" | "mjs" | "cjs" | "ts" | "sh" | "bash" | "py")
                    ) {
                        hook_files.insert(resolve_relative_target(path, clean));
                    }
                }
                let lower = cmd.to_ascii_lowercase();
                if has_dangerous_shell_hook(&lower) {
                    findings.push(Finding {
                        id: "ST-MANIFEST-HOOK-RCE".into(),
                        title: format!("Dangerous command in npm '{}' lifecycle script", key),
                        severity: FindingSeverity::Critical,
                        description: format!(
                            "package.json script '{}' contains dangerous execution/download command: '{}'.",
                            key, cmd
                        ),
                        file: path.to_string(),
                        line: None,
                    });
                } else if !declared_manifest.spawns_processes {
                    findings.push(Finding {
                        id: "ST-MANIFEST-HOOK".into(),
                        title: format!("Undeclared npm '{}' lifecycle script", key),
                        severity: FindingSeverity::High,
                        description: format!(
                            "package.json defines '{}' hook ('{}') without spawns_processes permission.",
                            key, cmd
                        ),
                        file: path.to_string(),
                        line: None,
                    });
                }
            }
        }
    }

    // Remote/git URL dependencies
    for dep_section in ["dependencies", "optionalDependencies", "devDependencies"] {
        if let Some(deps) = v.get(dep_section).and_then(|d| d.as_object()) {
            for (dep_name, dep_spec) in deps {
                if let Some(spec) = dep_spec.as_str() {
                    let lower = spec.to_ascii_lowercase();
                    if lower.starts_with("http://")
                        || lower.starts_with("https://")
                        || lower.starts_with("git://")
                        || lower.starts_with("git+http")
                        || lower.starts_with("git+ssh://")
                    {
                        findings.push(Finding {
                            id: "ST-MANIFEST-REMOTE-DEP".into(),
                            title: "Remote URL dependency bypasses registry hashing".into(),
                            severity: FindingSeverity::High,
                            description: format!(
                                "package.json {} entry '{}' resolves external URL '{}'.",
                                dep_section, dep_name, spec
                            ),
                            file: path.to_string(),
                            line: None,
                        });
                    }
                }
            }
        }
    }

    // Binary shadow hijack checks (`bin`)
    const SENSITIVE_BIN_NAMES: &[&str] = &[
        "sh", "bash", "zsh", "sudo", "su", "ssh", "curl", "wget", "git", "python", "python3",
        "node", "npm", "cargo", "rustc", "docker", "kubectl",
    ];
    if let Some(bin_obj) = v.get("bin").and_then(|b| b.as_object()) {
        for bin_name in bin_obj.keys() {
            if SENSITIVE_BIN_NAMES.contains(&bin_name.to_ascii_lowercase().as_str()) {
                findings.push(Finding {
                    id: "ST-MANIFEST-BIN-HIJACK".into(),
                    title: format!("Package bin exposes shadowed system binary '{}'", bin_name),
                    severity: FindingSeverity::Critical,
                    description: format!(
                        "package.json 'bin' maps sensitive system command '{}'.",
                        bin_name
                    ),
                    file: path.to_string(),
                    line: None,
                });
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn audit_cargo_toml(
    path: &str,
    content: &str,
    authoritative_id: Option<&PackageId>,
    archive: &UnpackedArchive,
    pkg_name: &mut String,
    pkg_version: &mut String,
    ecosystem: &mut String,
    hook_files: &mut BTreeSet<String>,
    findings: &mut Vec<Finding>,
) {
    let Ok(val) = toml::from_str::<toml::Value>(content) else {
        findings.push(Finding {
            id: "ST-MANIFEST-CORRUPT".into(),
            title: "Unparseable Cargo.toml manifest".into(),
            severity: FindingSeverity::High,
            description: format!("Failed to parse '{}' as valid TOML.", path),
            file: path.to_string(),
            line: None,
        });
        return;
    };

    if let Some(pkg_tbl) = val.get("package").and_then(|v| v.as_table()) {
        let parsed_name = pkg_tbl
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let parsed_ver = pkg_tbl
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        if authoritative_id.is_none() && pkg_name.is_empty() && !parsed_name.is_empty() {
            *pkg_name = parsed_name.to_string();
            *pkg_version = if parsed_ver.is_empty() {
                "0.0.0".to_string()
            } else {
                parsed_ver.to_string()
            };
            *ecosystem = "cargo".to_string();
        } else if let Some(auth) = authoritative_id {
            if normalize_ecosystem(&auth.ecosystem) == "cargo"
                && !parsed_name.is_empty()
                && !parsed_name.eq_ignore_ascii_case(&auth.name)
            {
                findings.push(Finding {
                    id: "ST-ECOSYSTEM-SPOOF".into(),
                    title: "Cargo.toml package.name disagrees with PackageId".into(),
                    severity: FindingSeverity::High,
                    description: format!(
                        "Cargo.toml '{}' declares name '{}' != '{}'.",
                        path, parsed_name, auth.name
                    ),
                    file: path.to_string(),
                    line: None,
                });
            }
        }

        if let Some(custom_build) = pkg_tbl.get("build").and_then(|v| v.as_str()) {
            let resolved = resolve_relative_target(path, custom_build);
            hook_files.insert(resolved);
        }
    }

    let sibling_build = resolve_relative_target(path, "build.rs");
    if archive.files.iter().any(|f| f.path == sibling_build) {
        hook_files.insert(sibling_build);
    }

    if val
        .get("lib")
        .and_then(|l| l.get("proc-macro"))
        .and_then(|b| b.as_bool())
        == Some(true)
    {
        findings.push(Finding {
            id: "ST-MANIFEST-PROCMACRO".into(),
            title: "Cargo proc-macro executes arbitrary compile-time code".into(),
            severity: FindingSeverity::Medium,
            description: format!("Cargo.toml '{}' sets [lib] proc-macro = true.", path),
            file: path.to_string(),
            line: None,
        });
    }

    for section in ["dependencies", "build-dependencies", "dev-dependencies"] {
        if let Some(tbl) = val.get(section).and_then(|d| d.as_table()) {
            for (dep_name, dep_val) in tbl {
                if let Some(dep_tbl) = dep_val.as_table() {
                    if let Some(p) = dep_tbl.get("path").and_then(|v| v.as_str()) {
                        if p.contains("..") || p.starts_with('/') {
                            findings.push(Finding {
                                id: "ST-MANIFEST-REMOTE-DEP".into(),
                                title: "Cargo dependency escapes package root path".into(),
                                severity: FindingSeverity::High,
                                description: format!(
                                    "Dependency '{}' in '{}' specifies suspicious path '{}'.",
                                    dep_name, path, p
                                ),
                                file: path.to_string(),
                                line: None,
                            });
                        }
                    }
                    if let Some(git) = dep_tbl.get("git").and_then(|v| v.as_str()) {
                        if git.to_ascii_lowercase().starts_with("http://") {
                            findings.push(Finding {
                                id: "ST-MANIFEST-REMOTE-DEP".into(),
                                title: "Insecure plaintext HTTP git dependency".into(),
                                severity: FindingSeverity::High,
                                description: format!(
                                    "Dependency '{}' in '{}' uses plaintext git URL '{}'.",
                                    dep_name, path, git
                                ),
                                file: path.to_string(),
                                line: None,
                            });
                        }
                    }
                }
            }
        }
    }
}

fn audit_pyproject_toml(
    path: &str,
    content: &str,
    authoritative_id: Option<&PackageId>,
    pkg_name: &mut String,
    pkg_version: &mut String,
    ecosystem: &mut String,
    findings: &mut Vec<Finding>,
) {
    let Ok(val) = toml::from_str::<toml::Value>(content) else {
        return;
    };

    if let Some(proj) = val.get("project").and_then(|p| p.as_table()) {
        let name = proj
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .trim();
        let ver = proj
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if authoritative_id.is_none() && pkg_name.is_empty() && !name.is_empty() {
            *pkg_name = name.to_string();
            *pkg_version = if ver.is_empty() {
                "0.0.0".to_string()
            } else {
                ver.to_string()
            };
            *ecosystem = "pypi".to_string();
        }
    }

    if let Some(backend) = val
        .get("build-system")
        .and_then(|b| b.get("build-backend"))
        .and_then(|b| b.as_str())
    {
        const KNOWN_BACKENDS: &[&str] = &[
            "setuptools.build_meta",
            "setuptools.build_meta:__legacy__",
            "flit_core.buildapi",
            "poetry.core.masonry.api",
            "hatchling.build",
            "maturin",
            "pdm.backend",
            "scikit_build_core.build",
            "mesonpy",
            "whey",
        ];
        if !backend.trim().is_empty() && !KNOWN_BACKENDS.contains(&backend.trim()) {
            findings.push(Finding {
                id: "ST-MANIFEST-HOOK".into(),
                title: "Non-standard Python PEP 517 build-backend declared".into(),
                severity: FindingSeverity::Medium,
                description: format!(
                    "pyproject.toml '{}' configures custom build-backend '{}'.",
                    path, backend
                ),
                file: path.to_string(),
                line: None,
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn audit_setup_py_or_cfg(
    path: &str,
    kind: ManifestKind,
    content: &str,
    pkg_name: &mut String,
    pkg_version: &mut String,
    ecosystem: &mut String,
    hook_files: &mut BTreeSet<String>,
    findings: &mut Vec<Finding>,
) {
    if pkg_name.is_empty() {
        for line in content.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("name") {
                let n = trimmed
                    .splitn(2, '=')
                    .nth(1)
                    .unwrap_or("")
                    .trim()
                    .trim_matches(['"', '\'', ',', ' ']);
                if !n.is_empty() {
                    *pkg_name = n.to_string();
                }
            } else if trimmed.starts_with("version") && pkg_version.is_empty() {
                let v = trimmed
                    .splitn(2, '=')
                    .nth(1)
                    .unwrap_or("")
                    .trim()
                    .trim_matches(['"', '\'', ',', ' ']);
                if !v.is_empty() {
                    *pkg_version = v.to_string();
                }
            }
        }
        if !pkg_name.is_empty() {
            if pkg_version.is_empty() {
                *pkg_version = "0.0.0".to_string();
            }
            *ecosystem = "pypi".to_string();
        }
    }

    if kind == ManifestKind::PySetupPy {
        hook_files.insert(path.to_string());
        let lower = content.to_ascii_lowercase();
        if lower.contains("cmdclass")
            || lower.contains("os.system")
            || lower.contains("subprocess")
            || lower.contains("eval(")
            || lower.contains("exec(")
            || lower.contains("urllib.")
            || lower.contains("requests.")
        {
            findings.push(Finding {
                id: "ST-MANIFEST-HOOK-RCE".into(),
                title: "Suspicious execution or install-time command inside setup.py".into(),
                severity: FindingSeverity::Critical,
                description: format!(
                    "Python build script '{}' uses custom cmdclass or dynamic OS/network execution.",
                    path
                ),
                file: path.to_string(),
                line: None,
            });
        }
    }
}
