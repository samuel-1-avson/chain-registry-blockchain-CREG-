// crates/resolver/src/chain_client.rs
// REST client for a chain node's package endpoint.
//
// ECA-11 (ADR-019 Increment 1): the former gRPC "fast path" was deleted. It
// rewrote `https://` node URLs to plaintext `http://<host>:50051` and answered
// with the *latest* verified version's content hash for whatever version was
// requested, so a revoked `pkg@1` read as VERIFIED whenever `pkg@2` was.
// Verdicts now come only from the per-version REST record.

use anyhow::{Context, Result};
use chrono::Utc;
use common::{DisplayText, PackageId, TrustVerdict, VerdictSource, VerdictStatus};

/// Reason shown when the node reports `quarantined` without one.
const DEFAULT_QUARANTINE_REASON: &str = "held for Class C committee review";

/// Wire shape of `GET /v1/public/packages/:canonical` (node `PackageResp`).
#[derive(serde::Deserialize, Default)]
pub(crate) struct PackageApiResp {
    pub status: String,
    pub block_hash: Option<String>,
    pub content_hash: Option<String>,
    pub ipfs_cid: Option<String>,
    pub publisher: Option<String>,
    pub revocation_reason: Option<String>,
    #[serde(default)]
    pub quarantine_reason: Option<String>,
    #[serde(default)]
    pub quarantine_expires_at_unix: Option<i64>,
    pub deterministic_risk: Option<common::DeterministicRiskSummary>,
    pub evidence_phase: Option<String>,
    pub block_phase: Option<String>,
    pub chain_status: Option<String>,
}

/// Map the node's status string onto a client verdict, failing closed.
///
/// Only `verified`, `revoked`, `quarantined`, `pending` and `unverified` are
/// understood. Anything else is held as an indefinite quarantine, because a
/// status this client does not know (the way `quarantined` was once mapped to
/// the break-glass-installable `Unverified`, ECA-07) must never become
/// installable through `--unverified`.
pub(crate) fn status_from_record(record: &PackageApiResp) -> VerdictStatus {
    let content_hash = record.content_hash.clone().unwrap_or_default();
    match record.status.as_str() {
        "verified" => VerdictStatus::Verified {
            block_hash: record.block_hash.clone().unwrap_or_default(),
            content_hash,
            ipfs_cid: record.ipfs_cid.clone().unwrap_or_default(),
            findings: vec![], // Findings are not included in the lightweight REST response
        },
        "revoked" => VerdictStatus::Revoked {
            reason: record
                .revocation_reason
                .clone()
                .unwrap_or_else(|| "Revoked".into()),
            findings: vec![],
        },
        "quarantined" => VerdictStatus::Quarantined {
            reason: record
                .quarantine_reason
                .clone()
                .filter(|reason| !reason.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_QUARANTINE_REASON.into()),
            content_hash,
            expires_at_unix: record.quarantine_expires_at_unix,
        },
        "pending" | "unverified" => VerdictStatus::Unverified,
        other => VerdictStatus::Quarantined {
            reason: format!(
                "node reported an unrecognised package status '{}'; held fail-closed",
                DisplayText::bounded(other, 32)
            ),
            content_hash,
            expires_at_unix: None,
        },
    }
}

/// Resolve a verdict from the node's per-version REST record.
pub async fn fetch_verdict(id: &PackageId, node_url: &str) -> Result<TrustVerdict> {
    let canonical = id.canonical();
    let encoded_canonical = urlencoding::encode(&canonical).into_owned();
    let grouped_url = format!(
        "{}/v1/public/packages/{}",
        node_url.trim_end_matches('/'),
        encoded_canonical
    );
    let legacy_url = format!(
        "{}/v1/packages/{}",
        node_url.trim_end_matches('/'),
        encoded_canonical
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    let resp = client
        .get(&grouped_url)
        .header("Accept", "application/json")
        .send()
        .await
        .with_context(|| format!("Failed to reach chain node at {}", node_url))?;

    let resp = if matches!(
        resp.status(),
        reqwest::StatusCode::NOT_FOUND
            | reqwest::StatusCode::METHOD_NOT_ALLOWED
            | reqwest::StatusCode::NOT_IMPLEMENTED
    ) {
        client
            .get(&legacy_url)
            .header("Accept", "application/json")
            .send()
            .await
            .with_context(|| format!("Failed to reach legacy package endpoint at {}", node_url))?
    } else {
        resp
    };

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(TrustVerdict {
            package: id.clone(),
            status: VerdictStatus::Unknown,
            resolved_at: Utc::now(),
            source: VerdictSource::Chain {
                node_url: node_url.to_string(),
            },
            deterministic_risk: None,
            evidence_phase: None,
            block_phase: None,
            chain_status: None,
            publisher: None,
        });
    }

    let record: PackageApiResp = resp
        .error_for_status()
        .context("Chain node returned an error")?
        .json()
        .await
        .context("Invalid JSON from chain node (expected package response)")?;

    let status = status_from_record(&record);

    Ok(TrustVerdict {
        package: id.clone(),
        status,
        resolved_at: Utc::now(),
        source: VerdictSource::Chain {
            node_url: node_url.to_string(),
        },
        deterministic_risk: record.deterministic_risk,
        evidence_phase: record.evidence_phase,
        block_phase: record.block_phase,
        chain_status: record.chain_status,
        publisher: record
            .publisher
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(status: &str) -> PackageApiResp {
        PackageApiResp {
            status: status.into(),
            content_hash: Some("ab".repeat(32)),
            ..Default::default()
        }
    }

    #[test]
    fn quarantined_maps_to_quarantined_not_unverified() {
        let mut rec = record("quarantined");
        rec.quarantine_reason = Some("credential exfiltration".into());
        rec.quarantine_expires_at_unix = Some(1_800_000_000);
        match status_from_record(&rec) {
            VerdictStatus::Quarantined {
                reason,
                content_hash,
                expires_at_unix,
            } => {
                assert_eq!(reason, "credential exfiltration");
                assert_eq!(content_hash, "ab".repeat(32));
                assert_eq!(expires_at_unix, Some(1_800_000_000));
            }
            other => panic!("expected quarantined, got {other:?}"),
        }
    }

    #[test]
    fn quarantined_without_reason_gets_default() {
        match status_from_record(&record("quarantined")) {
            VerdictStatus::Quarantined { reason, .. } => {
                assert_eq!(reason, DEFAULT_QUARANTINE_REASON)
            }
            other => panic!("expected quarantined, got {other:?}"),
        }
    }

    #[test]
    fn unknown_status_strings_fail_closed() {
        let status = status_from_record(&record("quarantined_v2\u{1b}]0;x\u{07}"));
        assert!(status.is_blocked(), "{status:?}");
        if let VerdictStatus::Quarantined { reason, .. } = status {
            assert!(!reason.contains('\u{1b}'));
        }
    }

    #[test]
    fn pending_is_unverified_and_known_statuses_round_trip() {
        assert_eq!(
            status_from_record(&record("pending")),
            VerdictStatus::Unverified
        );
        assert!(status_from_record(&record("verified")).is_safe());
        assert!(matches!(
            status_from_record(&record("revoked")),
            VerdictStatus::Revoked { .. }
        ));
    }
}
