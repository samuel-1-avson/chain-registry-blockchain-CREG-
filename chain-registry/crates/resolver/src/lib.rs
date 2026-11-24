// crates/resolver/src/lib.rs
// Resolves a package name/ID to a TrustVerdict.
//
// ECA-13 (ADR-019 Increment 1): there is one resolution path. `resolve` /
// `resolve_id` (used by `creg audit`, `status`, `verify` and `batch`) run the
// same install-grade resolution as `creg install`: a cached positive verdict is
// reused for at most `install_gate::INSTALL_CACHE_MAX_AGE`, phase-C builds
// verify the publication proof, and an unreachable node is reported as
// `Unknown` without being cached.

pub mod attestation_verifier;
pub mod cache;
mod chain_client;
pub mod downloader;
mod file_store;
pub mod freshness;
pub mod install_gate;
pub mod npm_manifest;

pub use attestation_verifier::{SignedPackageAttestation, DEFAULT_MIN_ACCEPTABLE_SCORE};

use anyhow::Result;
use chrono::Utc;
use common::{PackageId, TrustVerdict, VerdictSource, VerdictStatus};

/// Resolve "name@version" or "name" in the given ecosystem using explicit
/// [`light_client::VerifyOptions`].
pub async fn resolve_with_options(
    raw: &str,
    ecosystem: Option<&str>,
    node_url: Option<&str>,
    options: &light_client::VerifyOptions,
) -> Result<TrustVerdict> {
    let (name, version) = parse_pkg(raw);
    let eco = ecosystem.unwrap_or("npm");
    let id = PackageId::new(eco, name, version.as_deref().unwrap_or("latest"));
    resolve_id_with_options(&id, node_url, options).await
}

/// Resolve "name@version" or "name" in the given ecosystem.
pub async fn resolve(
    raw: &str,
    ecosystem: Option<&str>,
    node_url: Option<&str>,
) -> Result<TrustVerdict> {
    resolve_with_options(
        raw,
        ecosystem,
        node_url,
        &light_client::VerifyOptions::from_env(),
    )
    .await
}

/// Resolve a fully-formed PackageId through the install-grade path using
/// explicit [`light_client::VerifyOptions`].
///
/// An unreachable node, or (in phase-C builds) a node that cannot prove a
/// verified record, yields `Unknown`; that observation is never cached, so a
/// transient failure cannot mask a later revocation.
pub async fn resolve_id_with_options(
    id: &PackageId,
    node_url: Option<&str>,
    options: &light_client::VerifyOptions,
) -> Result<TrustVerdict> {
    match install_gate::resolve_id_for_install_with_options(id, node_url, options).await {
        Ok(verdict) => Ok(verdict),
        Err(e) => {
            let url = node_url.map(String::from).unwrap_or_else(default_node_url);
            tracing::warn!(
                "install-grade resolution failed for {} ({}): returning Unknown",
                id.canonical(),
                e
            );
            Ok(TrustVerdict {
                package: id.clone(),
                status: VerdictStatus::Unknown,
                resolved_at: Utc::now(),
                source: VerdictSource::Chain { node_url: url },
                deterministic_risk: None,
                evidence_phase: None,
                block_phase: None,
                chain_status: None,
                publisher: None,
            })
        }
    }
}

/// Resolve a fully-formed PackageId through the install-grade path.
///
/// An unreachable node, or (in phase-C builds) a node that cannot prove a
/// verified record, yields `Unknown`; that observation is never cached, so a
/// transient failure cannot mask a later revocation.
pub async fn resolve_id(id: &PackageId, node_url: Option<&str>) -> Result<TrustVerdict> {
    resolve_id_with_options(id, node_url, &light_client::VerifyOptions::from_env()).await
}

pub(crate) fn parse_pkg(raw: &str) -> (String, Option<String>) {
    // Strip ecosystem prefix if present (e.g. "npm:express" -> "express")
    let mut clean = raw;
    for eco in &["npm:", "pypi:", "cargo:", "rubygems:", "maven:"] {
        if let Some(stripped) = raw.strip_prefix(eco) {
            clean = stripped;
            break;
        }
    }

    if let Some(rest) = clean.strip_prefix('@') {
        if let Some(idx) = rest.rfind('@') {
            return (
                format!("@{}", &rest[..idx]),
                Some(rest[idx + 1..].to_string()),
            );
        }
        return (clean.to_string(), None);
    }
    match clean.rfind('@') {
        Some(idx) => (clean[..idx].to_string(), Some(clean[idx + 1..].to_string())),
        None => (clean.to_string(), None),
    }
}

pub(crate) fn default_node_url() -> String {
    std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
}

pub mod light_client;

use light_client::Checkpoint;

/// Resolve a package using light-client SPV verification.
/// Provides stronger guarantees than trusting the node's verdict directly,
/// at the cost of one extra round-trip for the Merkle proof.
pub async fn resolve_verified(
    id: &common::PackageId,
    node_url: Option<&str>,
    checkpoint: Option<&Checkpoint>,
) -> anyhow::Result<common::TrustVerdict> {
    let url = node_url.map(String::from).unwrap_or_else(|| {
        std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
    });
    let verdict = resolve_id(id, Some(&url)).await?;
    light_client::verify_verdict(verdict, &url, checkpoint).await
}
