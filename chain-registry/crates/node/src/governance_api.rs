// crates/node/src/governance_api.rs
// Public GET mirrors Governance.sol proposals. Writes (submit / vote / execute)
// are operator-authenticated and signed on L1 by CREG_GOVERNANCE_KEY.
//
// GET: missing L1 config is an honest empty list (HTTP 200). RPC failures are
// HTTP 503, never HTTP 501.
// POST: missing operator ACL is 401/503 at the private-API middleware. Missing
// CREG_GOVERNANCE_KEY is HTTP 503 (not CREG_BRIDGE_KEY). This node does not
// expose write routes on /v1/public or the legacy GET path.

use alloy::network::EthereumWallet;
use alloy::primitives::{Address, Bytes, B256, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::Filter;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use alloy::sol_types::SolEvent;
use alloy::transports::Transport;
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
const L1_WRITE_TIMEOUT: Duration = Duration::from_secs(45);
const PROPOSER_LOG_LOOKBACK_BLOCKS: u64 = 10_000;
const MAX_DESCRIPTION_CHARS: usize = 4096;
const MAX_CALLDATA_BYTES: usize = 24 * 1024;

pub const GOVERNANCE_KEY_ENV: &str = "CREG_GOVERNANCE_KEY";

sol!(
    #[sol(rpc)]
    interface IGovernance {
        function proposalCount() external view returns (uint256 _0);
        function threshold() external view returns (uint256 _0);
        function isSigner(address account) external view returns (bool _0);
        function getProposal(uint256 id) external view returns (
            address target,
            string description,
            uint256 submittedAt,
            uint8 status,
            uint256 approvalCount,
            uint256 rejectionCount
        );
        function submit(
            address target,
            bytes calldata callData,
            string calldata description
        ) external returns (uint256 id);
        function vote(uint256 id, bool approve) external;
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

#[derive(Debug, Clone, Deserialize)]
pub struct GovernanceSubmitBody {
    pub target: String,
    #[serde(default)]
    pub calldata: Option<String>,
    #[serde(default)]
    pub call_data: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GovernanceVoteBody {
    pub approve: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GovernanceWriteBody {
    pub action: String,
    pub signer: String,
    pub tx_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct GovernanceWriteErrorBody {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
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

fn write_error(status: StatusCode, error: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(GovernanceWriteErrorBody {
            error: error.to_string(),
            message: Some(message.into()),
        }),
    )
        .into_response()
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

pub async fn handle_submit(state: SharedState, body: GovernanceSubmitBody) -> Response {
    let parsed = match parse_submit_body(&body) {
        Ok(parsed) => parsed,
        Err((status, error, message)) => return write_error(status, error, message),
    };
    match prepare_write(&state).await {
        Ok(ctx) => dispatch_write(ctx, WriteAction::Submit(parsed)).await,
        Err(response) => response,
    }
}

pub async fn handle_vote(state: SharedState, id: u64, body: GovernanceVoteBody) -> Response {
    match prepare_write(&state).await {
        Ok(ctx) => {
            dispatch_write(
                ctx,
                WriteAction::Vote {
                    id,
                    approve: body.approve,
                },
            )
            .await
        }
        Err(response) => response,
    }
}

pub async fn handle_execute(state: SharedState, id: u64) -> Response {
    match prepare_write(&state).await {
        Ok(ctx) => dispatch_write(ctx, WriteAction::Execute { id }).await,
        Err(response) => response,
    }
}

struct SubmitArgs {
    target: Address,
    calldata: Bytes,
    description: String,
}

enum WriteAction {
    Submit(SubmitArgs),
    Vote { id: u64, approve: bool },
    Execute { id: u64 },
}

struct WriteContext {
    rpc_url: String,
    governance_addr: Address,
    signer: PrivateKeySigner,
}

#[allow(clippy::result_large_err)]
async fn prepare_write(state: &SharedState) -> Result<WriteContext, Response> {
    let (rpc_url, governance_addr) = {
        let s = state.read().await;
        (
            s.config.eth_rpc_url.clone(),
            s.config.governance_addr.clone(),
        )
    };
    if !is_configured(&rpc_url, &governance_addr) {
        return Err(write_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_l1_unconfigured",
            "Set CREG_ETH_RPC and CREG_GOVERNANCE_ADDR before submitting, voting, or executing.",
        ));
    }
    let gov_addr: Address = match governance_addr.trim().parse() {
        Ok(addr) => addr,
        Err(err) => {
            return Err(write_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "governance_l1_unconfigured",
                format!("invalid CREG_GOVERNANCE_ADDR: {err}"),
            ))
        }
    };
    let signer = match load_governance_signer().await {
        Ok(signer) => signer,
        Err(response) => return Err(response),
    };
    Ok(WriteContext {
        rpc_url,
        governance_addr: gov_addr,
        signer,
    })
}

#[allow(clippy::result_large_err)]
async fn load_governance_signer() -> Result<PrivateKeySigner, Response> {
    let secrets = match chain_registry_secrets::SecretsProvider::from_env() {
        Ok(provider) => provider,
        Err(err) => {
            return Err(write_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "governance_signer_unavailable",
                format!("Secrets backend failed for Governance.sol signer: {err}"),
            ))
        }
    };
    let vault_conflicts = chain_registry_secrets::validate_vault_exclusivity(secrets.backend());
    if !vault_conflicts.is_empty() {
        return Err(write_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_signer_unavailable",
            vault_conflicts.join("; "),
        ));
    }

    let secret = match secrets
        .try_signing_key_secret(chain_registry_secrets::HotKeyRole::Governance)
        .await
    {
        Ok(Some(value)) => value,
        Ok(None) => {
            return Err(write_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "governance_signer_unconfigured",
                format!(
                    "Set {GOVERNANCE_KEY_ENV} (or configure HashiCorp Vault role Governance) to a Governance.sol signer private key (32 bytes / 64 hex chars). \
                     CREG_BRIDGE_KEY and CREG_OPERATOR_API_KEY cannot submit or vote."
                ),
            ))
        }
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("not valid hex") || msg.contains("must be 32 bytes") {
                return Err(write_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_governance_key",
                    format!("{GOVERNANCE_KEY_ENV} must be 32 bytes (64 hex chars), optional 0x prefix."),
                ));
            }
            return Err(write_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "governance_signer_unavailable",
                format!("Failed to load {GOVERNANCE_KEY_ENV}: {err}"),
            ));
        }
    };

    secret
        .expose_secret()
        .parse::<PrivateKeySigner>()
        .map_err(|_| {
            write_error(
                StatusCode::BAD_REQUEST,
                "invalid_governance_key",
                format!("{GOVERNANCE_KEY_ENV} is not a valid secp256k1 private key."),
            )
        })
}

fn parse_submit_body(
    body: &GovernanceSubmitBody,
) -> Result<SubmitArgs, (StatusCode, &'static str, String)> {
    if body.description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid_governance_proposal",
            format!("description must be at most {MAX_DESCRIPTION_CHARS} characters"),
        ));
    }
    if body.description.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid_governance_proposal",
            "description must not be empty".into(),
        ));
    }
    let target: Address = body.target.trim().parse().map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            "invalid_governance_proposal",
            format!("invalid target address: {err}"),
        )
    })?;
    if target == Address::ZERO {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid_governance_proposal",
            "target must not be the zero address".into(),
        ));
    }
    let calldata_hex = body
        .calldata
        .as_deref()
        .or(body.call_data.as_deref())
        .unwrap_or("0x");
    let calldata = parse_calldata(calldata_hex)?;
    Ok(SubmitArgs {
        target,
        calldata,
        description: body.description.clone(),
    })
}

fn parse_calldata(raw: &str) -> Result<Bytes, (StatusCode, &'static str, String)> {
    let hex_part = raw.trim().strip_prefix("0x").unwrap_or(raw.trim());
    if hex_part.is_empty() {
        return Ok(Bytes::new());
    }
    if !hex_part.len().is_multiple_of(2) || !hex_part.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid_governance_proposal",
            "calldata must be even-length hex, optional 0x prefix".into(),
        ));
    }
    let bytes = hex::decode(hex_part).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            "invalid_governance_proposal",
            format!("invalid calldata hex: {err}"),
        )
    })?;
    if bytes.len() > MAX_CALLDATA_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid_governance_proposal",
            format!("calldata must be at most {MAX_CALLDATA_BYTES} bytes"),
        ));
    }
    Ok(Bytes::from(bytes))
}

async fn dispatch_write(ctx: WriteContext, action: WriteAction) -> Response {
    match tokio::time::timeout(L1_WRITE_TIMEOUT, send_governance_tx(ctx, action)).await {
        Ok(Ok(body)) => (StatusCode::OK, Json(body)).into_response(),
        Ok(Err(response)) => response,
        Err(_) => write_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_l1_unavailable",
            format!(
                "Governance L1 RPC timed out after {}s",
                L1_WRITE_TIMEOUT.as_secs()
            ),
        ),
    }
}

#[allow(clippy::result_large_err)]
async fn send_governance_tx(
    ctx: WriteContext,
    action: WriteAction,
) -> Result<GovernanceWriteBody, Response> {
    let signer_addr = ctx.signer.address();
    let wallet = EthereumWallet::from(ctx.signer);
    let base_provider =
        crate::rpc_failover::build_l1_provider(ctx.rpc_url.trim()).map_err(|err| {
            write_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "governance_l1_unavailable",
                format!("invalid CREG_ETH_RPC configuration: {err}"),
            )
        })?;
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_provider(base_provider);
    let contract = IGovernance::new(ctx.governance_addr, &provider);

    let is_signer = contract
        .isSigner(signer_addr)
        .call()
        .await
        .map_err(|err| {
            write_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "governance_l1_unavailable",
                err.to_string(),
            )
        })?
        ._0;
    if !is_signer {
        return Err(write_error(
            StatusCode::FORBIDDEN,
            "governance_signer_not_authorized",
            format!("{GOVERNANCE_KEY_ENV} address {signer_addr:#x} is not a Governance.sol signer"),
        ));
    }

    match action {
        WriteAction::Submit(args) => {
            let next_id = contract
                .proposalCount()
                .call()
                .await
                .map_err(l1_unavailable)?
                ._0
                .to::<u64>();
            let tx_hash = contract
                .submit(args.target, args.calldata, args.description)
                .send()
                .await
                .map_err(l1_rejected)?
                .watch()
                .await
                .map_err(l1_unavailable)?;
            Ok(GovernanceWriteBody {
                action: "submit".into(),
                signer: format!("{signer_addr:#x}"),
                tx_hash: format!("{tx_hash:#x}"),
                proposal_id: Some(next_id),
                approved: None,
                execution: None,
            })
        }
        WriteAction::Vote { id, approve } => {
            require_pending_proposal(&contract, id).await?;
            let tx_hash = contract
                .vote(U256::from(id), approve)
                .send()
                .await
                .map_err(l1_rejected)?
                .watch()
                .await
                .map_err(l1_unavailable)?;
            Ok(GovernanceWriteBody {
                action: "vote".into(),
                signer: format!("{signer_addr:#x}"),
                tx_hash: format!("{tx_hash:#x}"),
                proposal_id: Some(id),
                approved: Some(approve),
                execution: approve.then(|| "automatic_on_threshold".to_string()),
            })
        }
        WriteAction::Execute { id } => {
            require_pending_proposal(&contract, id).await?;
            let tx_hash = contract
                .vote(U256::from(id), true)
                .send()
                .await
                .map_err(l1_rejected)?
                .watch()
                .await
                .map_err(l1_unavailable)?;
            Ok(GovernanceWriteBody {
                action: "execute".into(),
                signer: format!("{signer_addr:#x}"),
                tx_hash: format!("{tx_hash:#x}"),
                proposal_id: Some(id),
                approved: Some(true),
                execution: Some(
                    "Governance.sol has no public execute(); this approval vote auto-executes when threshold is met."
                        .into(),
                ),
            })
        }
    }
}

#[allow(clippy::result_large_err)]
async fn require_pending_proposal<T, P>(
    contract: &IGovernance::IGovernanceInstance<T, P>,
    id: u64,
) -> Result<(), Response>
where
    T: Transport + Clone,
    P: Provider<T>,
{
    let count = contract
        .proposalCount()
        .call()
        .await
        .map_err(l1_unavailable)?
        ._0
        .to::<u64>();
    if id >= count {
        return Err(write_error(
            StatusCode::NOT_FOUND,
            "proposal_not_found",
            format!("proposal {id} does not exist"),
        ));
    }
    let proposal = contract
        .getProposal(U256::from(id))
        .call()
        .await
        .map_err(l1_unavailable)?;
    match proposal.status {
        0 => Ok(()),
        1 => Err(write_error(
            StatusCode::CONFLICT,
            "proposal_already_executed",
            format!("proposal {id} is already executed"),
        )),
        2 => Err(write_error(
            StatusCode::CONFLICT,
            "proposal_cancelled",
            format!("proposal {id} is cancelled"),
        )),
        other => Err(write_error(
            StatusCode::BAD_REQUEST,
            "proposal_not_pending",
            format!("proposal {id} has unknown status {other}"),
        )),
    }
}

fn l1_unavailable(err: impl ToString) -> Response {
    write_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "governance_l1_unavailable",
        err.to_string(),
    )
}

fn l1_rejected(err: impl ToString) -> Response {
    write_error(
        StatusCode::BAD_REQUEST,
        "governance_l1_rejected",
        err.to_string(),
    )
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
    let provider = crate::rpc_failover::build_l1_provider(rpc_url.trim())?;
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

async fn proposal_proposers<T: Transport + Clone>(
    provider: &impl Provider<T>,
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

    #[test]
    fn parses_calldata_hex_and_empty() {
        assert_eq!(parse_calldata("0x").unwrap(), Bytes::new());
        assert_eq!(parse_calldata("").unwrap(), Bytes::new());
        assert_eq!(
            parse_calldata("0xaabb").unwrap(),
            Bytes::from(vec![0xaa, 0xbb])
        );
        assert!(parse_calldata("0xzz").is_err());
        assert!(parse_calldata("0xabc").is_err());
    }

    #[test]
    fn submit_body_rejects_zero_target_and_empty_description() {
        let zero = parse_submit_body(&GovernanceSubmitBody {
            target: "0x0000000000000000000000000000000000000000".into(),
            calldata: Some("0x".into()),
            call_data: None,
            description: "ok".into(),
        });
        assert!(zero.is_err());

        let empty = parse_submit_body(&GovernanceSubmitBody {
            target: "0x1111111111111111111111111111111111111111".into(),
            calldata: None,
            call_data: None,
            description: "   ".into(),
        });
        assert!(empty.is_err());

        let ok = parse_submit_body(&GovernanceSubmitBody {
            target: "0x1111111111111111111111111111111111111111".into(),
            calldata: None,
            call_data: Some("0xdead".into()),
            description: "Pause registry".into(),
        })
        .expect("valid submit");
        assert_eq!(ok.calldata, Bytes::from(vec![0xde, 0xad]));
    }

    #[test]
    fn governance_key_must_be_64_hex_chars() {
        use chain_registry_secrets::{normalize_signing_key_secret, HotKeyRole};

        assert!(normalize_signing_key_secret("not-a-key", HotKeyRole::Governance).is_err());
        assert!(normalize_signing_key_secret(&"ab".repeat(31), HotKeyRole::Governance).is_err());
        let ok = normalize_signing_key_secret(&"ab".repeat(32), HotKeyRole::Governance)
            .expect("hex key");
        assert_eq!(ok.expose_secret(), format!("0x{}", "ab".repeat(32)));
        let prefixed =
            normalize_signing_key_secret(&format!("0x{}", "cd".repeat(32)), HotKeyRole::Governance)
                .expect("prefixed");
        assert_eq!(prefixed.expose_secret(), format!("0x{}", "cd".repeat(32)));
    }
}
