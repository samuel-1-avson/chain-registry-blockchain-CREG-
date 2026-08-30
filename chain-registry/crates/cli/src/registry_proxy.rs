//! Developer Registry Proxy Server
//!
//! Provides a local drop-in HTTP registry proxy for `npm`, `pip`, and `cargo` that
//! intercepts package queries, checks on-chain trust verdicts, and blocks malicious
//! or revoked packages in real time before they reach the developer machine.

use anyhow::{bail, Result};
use common::{PackageId, TrustVerdict, VerdictStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for local registry proxy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub port: u16,
    pub upstream_npm: String,
    pub upstream_pypi: String,
    pub upstream_crates: String,
    pub node_url: String,
    pub allow_unverified: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            upstream_npm: "https://registry.npmjs.org".into(),
            upstream_pypi: "https://pypi.org".into(),
            upstream_crates: "https://crates.io".into(),
            node_url: "http://127.0.0.1:8545".into(),
            allow_unverified: false,
        }
    }
}

/// Evaluation outcome for a package request passing through the proxy
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyVerdictOutcome {
    /// Safe to proxy through upstream
    Forward {
        canonical: String,
        risk_score: u32,
        upstream_url: String,
    },
    /// Block package with security diagnostics
    Block {
        canonical: String,
        reason: String,
        status_code: u16,
    },
}

/// Registry proxy router and request dispatcher
pub struct RegistryProxy {
    config: ProxyConfig,
}

impl RegistryProxy {
    pub fn new(config: ProxyConfig) -> Self {
        Self { config }
    }

    /// Match an incoming HTTP request path to an ecosystem and package
    pub fn parse_request_path(&self, path: &str) -> Option<(PackageId, String)> {
        let trimmed = path.trim_start_matches('/');

        // npm pattern: "/@scope/pkg/-/@scope/pkg-1.0.0.tgz" or "/pkg/1.0.0" or "/pkg"
        if trimmed.starts_with("npm/") || (!trimmed.starts_with("pypi/") && !trimmed.starts_with("crates/")) {
            let pkg_path = trimmed.strip_prefix("npm/").unwrap_or(trimmed);
            let parts: Vec<&str> = pkg_path.split('/').collect();
            if !parts.is_empty() {
                let name = parts[0];
                let version = if parts.len() > 1 && !parts[1].starts_with('-') {
                    parts[1]
                } else {
                    "latest"
                };
                let id = PackageId::new("npm", name, version);
                let upstream = format!("{}/{}", self.config.upstream_npm, pkg_path);
                return Some((id, upstream));
            }
        }

        // PyPI pattern: "/pypi/requests/json" or "/simple/requests/"
        if trimmed.starts_with("pypi/") || trimmed.starts_with("simple/") {
            let pkg_path = trimmed.strip_prefix("pypi/").or_else(|| trimmed.strip_prefix("simple/")).unwrap_or(trimmed);
            let parts: Vec<&str> = pkg_path.split('/').collect();
            if !parts.is_empty() {
                let name = parts[0];
                let id = PackageId::new("pypi", name, "latest");
                let upstream = format!("{}/{}", self.config.upstream_pypi, pkg_path);
                return Some((id, upstream));
            }
        }

        // Crates.io pattern: "/crates/serde/1.0.0/download"
        if trimmed.starts_with("crates/") {
            let pkg_path = trimmed.strip_prefix("crates/").unwrap_or(trimmed);
            let parts: Vec<&str> = pkg_path.split('/').collect();
            if parts.len() >= 2 {
                let name = parts[0];
                let version = parts[1];
                let id = PackageId::new("cargo", name, version);
                let upstream = format!("{}/api/v1/crates/{}/{}/download", self.config.upstream_crates, name, version);
                return Some((id, upstream));
            }
        }

        None
    }

    /// Evaluate trust verdict for an intercepted package request
    pub fn evaluate_verdict(
        &self,
        verdict: &TrustVerdict,
        upstream_url: &str,
    ) -> ProxyVerdictOutcome {
        let canonical = verdict.package.canonical();

        match &verdict.status {
            VerdictStatus::Revoked { reason, .. } => ProxyVerdictOutcome::Block {
                canonical: canonical.clone(),
                reason: format!("Package is REVOKED on-chain: {}", reason),
                status_code: 403,
            },
            VerdictStatus::Verified { findings, .. } => {
                let has_critical = findings.iter().any(|f| matches!(f.severity, common::FindingSeverity::Critical));
                if has_critical && !self.config.allow_unverified {
                    ProxyVerdictOutcome::Block {
                        canonical: canonical.clone(),
                        reason: "Package contains CRITICAL security findings detected during sandbox analysis".into(),
                        status_code: 403,
                    }
                } else {
                    ProxyVerdictOutcome::Forward {
                        canonical: canonical.clone(),
                        risk_score: 95,
                        upstream_url: upstream_url.to_string(),
                    }
                }
            }
            VerdictStatus::Unverified | VerdictStatus::Unknown => {
                if self.config.allow_unverified {
                    ProxyVerdictOutcome::Forward {
                        canonical: canonical.clone(),
                        risk_score: 50,
                        upstream_url: upstream_url.to_string(),
                    }
                } else {
                    ProxyVerdictOutcome::Block {
                        canonical: canonical.clone(),
                        reason: "Package is UNVERIFIED on CREG blockchain ledger".into(),
                        status_code: 403,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use common::{VerdictSource, FindingSeverity};

    #[test]
    fn test_proxy_route_parsing() {
        let proxy = RegistryProxy::new(ProxyConfig::default());

        // npm route
        let (id, upstream) = proxy.parse_request_path("/express/4.18.2").expect("npm parse");
        assert_eq!(id.ecosystem, "npm");
        assert_eq!(id.name, "express");
        assert_eq!(id.version, "4.18.2");
        assert!(upstream.contains("registry.npmjs.org"));

        // PyPI route
        let (id2, upstream2) = proxy.parse_request_path("/pypi/requests/json").expect("pypi parse");
        assert_eq!(id2.ecosystem, "pypi");
        assert_eq!(id2.name, "requests");
        assert!(upstream2.contains("pypi.org"));

        // Crates route
        let (id3, upstream3) = proxy.parse_request_path("/crates/serde/1.0.197/download").expect("crates parse");
        assert_eq!(id3.ecosystem, "cargo");
        assert_eq!(id3.name, "serde");
        assert_eq!(id3.version, "1.0.197");
        assert!(upstream3.contains("crates.io"));
    }

    #[test]
    fn test_proxy_verdict_blocking() {
        let proxy = RegistryProxy::new(ProxyConfig::default());
        let id = PackageId::new("npm", "malicious-pkg", "1.0.0");

        // Revoked package is blocked
        let revoked_verdict = TrustVerdict {
            package: id.clone(),
            status: VerdictStatus::Revoked {
                reason: "Typosquat malware detected".into(),
                findings: vec![],
            },
            resolved_at: Utc::now(),
            source: VerdictSource::Chain { node_url: "http://localhost".into() },
            deterministic_risk: None,
            evidence_phase: None,
            block_phase: None,
            chain_status: None,
            publisher: None,
        };

        let res = proxy.evaluate_verdict(&revoked_verdict, "https://registry.npmjs.org/malicious-pkg");
        assert!(matches!(res, ProxyVerdictOutcome::Block { status_code: 403, .. }));
    }
}
