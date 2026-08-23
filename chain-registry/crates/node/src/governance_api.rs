// crates/node/src/governance_api.rs
// Read-only mirror of Governance.sol proposals for the explorer and public API.
//
// This is not token voting and does not submit or vote. Missing L1 config is an
// honest empty list (HTTP 200). RPC failures are HTTP 503, never HTTP 501.

use alloy::primitives::{Address, B256, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::Filter;
use alloy::sol;
use alloy::sol_types::SolEvent;
use alloy::transports::http::{Client, Http};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

use crate::SharedState;

const MAX_PROPOSALS: usize = 100;
const DEFAULT_LIMIT: usize = 50;
const L1_TIMEOUT: Duration = Duration::from_secs(12);
const PROPOSER_LOG_LOOKBACK_BLOCKS: u64 = 10_000;

sol!(
    #[sol(rpc)]
    interface IGovernance {
        function proposalCount() external view returns (uint256 _0);
        function threshold() external view returns (uint256 _0);
        function getProposal(uint256 id) external view returns (
            address target,
            string description,
            uint256 submittedAt,
            uint8 status,
            uint256 approvalCount,
            uint256 rejectionCount
        );
        event ProposalSubmitted(uint256 indexed id, address indexed proposer, string description);
    }
);

#[derive(Debug, Default, Deserialize)]
pub struct GovernanceProposalsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GovernanceProposalView {
    pub id: u64,
    pub title: String,
    pub description: String,
    pub status: String,
    pub target: String,
    pub proposer: String,
    pub votes_yea: u64,
    pub votes_nay: u64,
    pub quorum: u64,
    pub submitted_at: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GovernanceProposalsBody {
    pub proposals: Vec<GovernanceProposalView>,
    pub total: usize,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub fn proposal_status_label(status: u8) -> &'static str {
    match status {
        0 => "active",
        1 => "executed",
        2 => "cancelled",
        _ => "unknown",
    }
}

pub fn proposal_title(description: &str) -> String {
    let line = description.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        "Untitled proposal".to_string()
    } else if line.chars().count() > 120 {
        let truncated: String = line.chars().take(117).collect();
        format!("{truncated}...")
    } else {
        line.to_string()
    }
}

pub fn rfc3339_from_unix(seconds: u64) -> String {
    chrono::DateTime::from_timestamp(seconds as i64, 0)
        .map(|ts| ts.to_rfc3339())
        .unwrap_or_default()
}

fn is_configured(rpc_url: &str, governance_addr: &str) -> bool {
    let rpc = rpc_url.trim();
    let addr = governance_addr.trim();
    if rpc.is_empty() || addr.is_empty() {
        return false;
    }
    let normalized = addr.to_ascii_lowercase();
    normalized != "0x0000000000000000000000000000000000000000" && normalized != "0x0"
}

fn unconfigured_body() -> GovernanceProposalsBody {
    GovernanceProposalsBody {
        proposals: Vec::new(),
        total: 0,
        source: "unconfigured".into(),
        error: None,
        message: Some(
            "Set CREG_ETH_RPC and CREG_GOVERNANCE_ADDR to mirror Governance.sol proposals.".into(),
        ),
    }
}

fn unavailable_body(message: String) -> GovernanceProposalsBody {
    GovernanceProposalsBody {
        proposals: Vec::new(),
        total: 0,
        source: "l1".into(),
        error: Some("governance_l1_unavailable".into()),
        message: Some(message),
    }
}

pub async fn handle(state: SharedState, query: GovernanceProposalsQuery) -> Response {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_PROPOSALS);
    let (rpc_url, governance_addr) = {
        let s = state.read().await;
        (
            s.config.eth_rpc_url.clone(),
            s.config.governance_addr.clone(),
        )
    };

    if !is_configured(&rpc_url, &governance_addr) {
        return (StatusCode::OK, Json(unconfigured_body())).into_response();
    }

    match tokio::time::timeout(
        L1_TIMEOUT,
        fetch_l1_proposals(&rpc_url, &governance_addr, limit),
    )
    .await
    {
        Ok(Ok(body)) => (StatusCode::OK, Json(body)).into_response(),
        Ok(Err(err)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(unavailable_body(err.to_string())),
        )
            .into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(unavailable_body(format!(
                "Governance L1 RPC timed out after {}s",
                L1_TIMEOUT.as_secs()
            ))),
        )
            .into_response(),
    }
}

async fn fetch_l1_proposals(
    rpc_url: &str,
    governance_addr: &str,
    limit: usize,
) -> anyhow::Result<GovernanceProposalsBody> {
    let gov_addr: Address = governance_addr
        .trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid CREG_GOVERNANCE_ADDR: {e}"))?;
    let provider = ProviderBuilder::new().on_http(rpc_url.trim().parse()?);
    let contract = IGovernance::new(gov_addr, &provider);

    let count = contract.proposalCount().call().await?._0.to::<u64>();
    let quorum = contract.threshold().call().await?._0.to::<u64>();
    let proposers = proposal_proposers(&provider, gov_addr).await;

    if count == 0 {
        return Ok(GovernanceProposalsBody {
            proposals: Vec::new(),
            total: 0,
            source: "l1".into(),
            error: None,
            message: None,
        });
    }

    let mut ids = Vec::new();
    let mut id = count - 1;
    loop {
        ids.push(id);
        if ids.len() >= limit || id == 0 {
            break;
        }
        id -= 1;
    }

    let mut proposals = Vec::with_capacity(ids.len());
    for id in ids {
        let p = contract.getProposal(U256::from(id)).call().await?;
        let submitted_at = p.submittedAt.to::<u64>();
        proposals.push(GovernanceProposalView {
            id,
            title: proposal_title(&p.description),
            description: p.description,
            status: proposal_status_label(p.status).to_string(),
            target: format!("{:#x}", p.target),
            proposer: proposers.get(&id).cloned().unwrap_or_default(),
            votes_yea: p.approvalCount.to::<u64>(),
            votes_nay: p.rejectionCount.to::<u64>(),
            quorum,
            submitted_at,
            created_at: rfc3339_from_unix(submitted_at),
        });
    }

    let total = proposals.len();
    Ok(GovernanceProposalsBody {
        proposals,
        total,
        source: "l1".into(),
        error: None,
        message: None,
    })
}

async fn proposal_proposers(
    provider: &impl Provider<Http<Client>>,
    gov_addr: Address,
) -> HashMap<u64, String> {
    let mut proposers = HashMap::new();
    let Ok(latest) = provider.get_block_number().await else {
        return proposers;
    };
    let from = latest.saturating_sub(PROPOSER_LOG_LOOKBACK_BLOCKS);
    let filter = Filter::new()
        .address(gov_addr)
        .event_signature(IGovernance::ProposalSubmitted::SIGNATURE_HASH)
        .from_block(from)
        .to_block(latest);
    let Ok(logs) = provider.get_logs(&filter).await else {
        return proposers;
    };
    for log in logs {
        if let Some((id, proposer)) = decode_submitted_proposer(log.topics()) {
            proposers.insert(id, proposer);
        }
    }
    proposers
}

fn decode_submitted_proposer(topics: &[B256]) -> Option<(u64, String)> {
    if topics.len() < 3 {
        return None;
    }
    let id = u64::try_from(U256::from_be_slice(topics[1].as_slice())).ok()?;
    let proposer = Address::from_word(topics[2]);
    Some((id, format!("{proposer:#x}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_on_chain_status_to_explorer_labels() {
        assert_eq!(proposal_status_label(0), "active");
        assert_eq!(proposal_status_label(1), "executed");
        assert_eq!(proposal_status_label(2), "cancelled");
        assert_eq!(proposal_status_label(9), "unknown");
    }

    #[test]
    fn title_uses_first_description_line() {
        assert_eq!(
            proposal_title("Upgrade Registry\ncalldata details"),
            "Upgrade Registry"
        );
        assert_eq!(proposal_title("   \n"), "Untitled proposal");
    }

    #[test]
    fn zero_governance_address_is_unconfigured() {
        assert!(!is_configured(
            "http://127.0.0.1:8545",
            "0x0000000000000000000000000000000000000000"
        ));
        assert!(!is_configured("http://127.0.0.1:8545", ""));
        assert!(!is_configured(
            "",
            "0x1111111111111111111111111111111111111111"
        ));
        assert!(is_configured(
            "http://127.0.0.1:8545",
            "0x1111111111111111111111111111111111111111"
        ));
    }

    #[test]
    fn decodes_indexed_proposer_topics() {
        let id = B256::from(U256::from(7));
        let proposer = Address::from_word(B256::with_last_byte(0xab));
        let topic2 = B256::from(proposer.into_word());
        let (decoded_id, decoded_addr) =
            decode_submitted_proposer(&[B256::ZERO, id, topic2]).expect("topics");
        assert_eq!(decoded_id, 7);
        assert_eq!(decoded_addr, format!("{proposer:#x}"));
    }
}
