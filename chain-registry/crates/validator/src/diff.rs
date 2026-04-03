// crates/validator/src/diff.rs
// Security-focused version diffing.
// detects "delta inflation" of permissions (e.g., version 1.0.1 adds network access).

use crate::sandbox::SandboxResult;
use common::PackageManifest;
use common::{Finding, FindingSeverity};

pub struct DiffResult {
    pub findings: Vec<Finding>,
    pub new_hosts: Vec<String>,
    pub new_paths: Vec<String>,
}

/// Compare current findings and sandbox observations against the previous verified version.
pub fn analyze(
    current_manifest: &PackageManifest,
    current_sandbox: &SandboxResult,
    prev_manifest: Option<&PackageManifest>,
    _prev_sandbox: Option<&SandboxResult>,
) -> DiffResult {
    let mut findings = Vec::new();
    let mut new_hosts = Vec::new();
    let mut new_paths = Vec::new();

    if let Some(prev) = prev_manifest {
        // Detect new network hosts.
        for host in &current_manifest.allowed_network_hosts {
            if !prev.allowed_network_hosts.contains(host) {
                new_hosts.push(host.clone());
                findings.push(Finding {
                    id: "DF001".into(),
                    title: "New network host".into(),
                    severity: FindingSeverity::Medium,
                    description: format!("New undeclared network host access: {}", host),
                    file: "manifest".into(),
                    line: None,
                });
            }
        }

        // Detect new filesystem write paths.
        for path in &current_manifest.allowed_fs_writes {
            if !prev.allowed_fs_writes.contains(path) {
                new_paths.push(path.clone());
                findings.push(Finding {
                    id: "DF002".into(),
                    title: "New fs write path".into(),
                    severity: FindingSeverity::Medium,
                    description: format!("New undeclared filesystem write path: {}", path),
                    file: "manifest".into(),
                    line: None,
                });
            }
        }

        // Detect change in child process spawning.
        if current_manifest.spawns_processes && !prev.spawns_processes {
            findings.push(Finding {
                id: "DF003".into(),
                title: "Permission escalation: process-spawn".into(),
                severity: FindingSeverity::High,
                description: "Package now requests child process execution (previously disabled)"
                    .into(),
                file: "manifest".into(),
                line: None,
            });
        }
    }

    // Also compare actual recorded observations from the sandbox.
    for host in &current_sandbox.observed_network_hosts {
        if !current_manifest.allowed_network_hosts.contains(host) {
            findings.push(Finding {
                id:          "DF004".into(),
                title:       "Sandbox violation: Undeclared host egress".into(),
                severity:    FindingSeverity::High,
                description: format!("Suspicious behavior: Real-world access to '{}' detected in sandbox but not in manifest", host),
                file:        "sandbox".into(),
                line:        None,
            });
        }
    }

    DiffResult {
        findings,
        new_hosts,
        new_paths,
    }
}
