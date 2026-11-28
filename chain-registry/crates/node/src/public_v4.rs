//! ADR-019 Increment 2, part 3a: the v4 public read surface (`ECA-05`,
//! `ECA-10`, `ECO-INV-02`, `ECO-INV-08`).
//!
//! Read-only, gateway-public routes, all derived from committed chain state:
//!
//! | Route | Body |
//! | --- | --- |
//! | `GET /v1/public/v4/packages/:name/:version/status` | [`PackageViewV4`] |
//! | `GET /v1/public/v4/packages/:name/:version/proof-bundle` | [`ProofBundleV2`] |
//! | `GET /v1/public/v4/packages/:name/:version/witnesses` | [`WitnessBundleV1`] |
//! | `GET /v1/public/v4/quarantine?limit=` | [`QuarantineListV1`] |
//! | `GET /v1/public/v4/events` | SSE v2 ([`SseEnvelopeV2`] frames) |
//!
//! `:name` is the package part of the canonical id (`npm:left-pad`,
//! `npm:@scope/pkg`) percent-encoded as one path segment. Views are display
//! data, never evidence: clients decide from [`ProofBundleV2`] with the
//! consensus verifiers (`ECO-INV-01`).

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use axum::extract::{ConnectInfo, Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use common::status_tree::split_canonical;
use common::{
    Block, ChainRecord, DisplayText, PackageStatus, QuarantineOutcomeV1, StatusLeafStateV1,
    StatusLeafV1, Transaction,
};
use futures::stream::{self, Stream};
use registry_view::{
    CommitteeSummaryV1, MerkleStepV1, PackageStatusViewV1, PackageViewV4, ProofBundleV2,
    QuarantineListV1, QuorumLabelV1, RevocationAuthorityV1, SseEnvelopeV2, WitnessBundleV1,
    WitnessVerificationV1, WitnessViewV1, MAX_REASON_CHARS, PROOF_BUNDLE_SCHEMA,
    QUARANTINE_LIST_SCHEMA, WITNESS_BUNDLE_SCHEMA,
};
use serde::Deserialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::chain_store::ChainStore;
use crate::rate_limit::RateLimiter;
use crate::state::SharedState;

/// SSE v2 subscriber caps (ADR-019 §3.3).
pub const MAX_SSE_V2_SUBSCRIBERS: usize = 2_048;
pub const MAX_SSE_V2_SUBSCRIBERS_PER_IP: usize = 8;
/// `Last-Event-ID` resume reaches back at most this many blocks.
pub const SSE_V2_REPLAY_WINDOW_BLOCKS: u64 = 1_024;
/// Blocks read per wake-up of one SSE v2 stream.
const SSE_V2_BLOCKS_PER_POLL: u64 = 64;
const SSE_V2_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub const DEFAULT_QUARANTINE_LIMIT: usize = 100;
pub const MAX_QUARANTINE_LIMIT: usize = 500;

const MAX_PACKAGE_NAME_CHARS: usize = 256;
const MAX_VERSION_CHARS: usize = 128;
/// Attempts to read a consistent (tip, status tree) pair while blocks land.
const PROOF_BUNDLE_ATTEMPTS: usize = 3;

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route(
            "/v1/public/v4/packages/:name/:version/status",
            get(get_status),
        )
        .route(
            "/v1/public/v4/packages/:name/:version/proof-bundle",
            get(get_proof_bundle),
        )
        .route(
            "/v1/public/v4/packages/:name/:version/witnesses",
            get(get_witnesses),
        )
        .route("/v1/public/v4/quarantine", get(get_quarantine))
        .route("/v1/public/v4/events", get(get_events))
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

/// `name@version` from the two path segments, bounded and control-free.
pub fn canonical_from_path(name: &str, version: &str) -> std::result::Result<String, String> {
    let (name, version) = (name.trim(), version.trim());
    let valid = |value: &str, max: usize| {
        !value.is_empty()
            && value.chars().count() <= max
            && !value.chars().any(|c| c.is_control() || c.is_whitespace())
    };
    if !valid(name, MAX_PACKAGE_NAME_CHARS) || name.ends_with('@') {
        return Err("invalid package name".into());
    }
    if !valid(version, MAX_VERSION_CHARS) || version.contains('@') {
        return Err("invalid package version".into());
    }
    Ok(format!("{name}@{version}"))
}

/// Block and index of a record's publication transaction, when canonical.
fn publication(chain: &ChainStore, record: &ChainRecord) -> Result<Option<(Block, usize)>> {
    let Some(block) = chain.get_block_by_hash(&record.block_hash)? else {
        return Ok(None);
    };
    let canonical_hash = chain
        .get_block_by_height(block.header.height)?
        .map(|stored| stored.hash());
    if canonical_hash.as_deref() != Some(block.hash().as_str()) {
        return Ok(None);
    }
    let canonical = record.id.canonical();
    let index = block.transactions.iter().position(|tx| {
        tx.published_record()
            .is_some_and(|published| published.id.canonical() == canonical)
    });
    Ok(index.map(|index| (block, index)))
}

struct RevocationDetail {
    authority: RevocationAuthorityV1,
    reason: String,
    witness_digest: Option<String>,
    certificate_digest: Option<String>,
}

/// Who revoked `canonical` at `leaf.height`, read from that block.
fn revocation_detail(
    chain: &ChainStore,
    canonical: &str,
    leaf: &StatusLeafV1,
    certified: bool,
    record: Option<&ChainRecord>,
) -> Result<RevocationDetail> {
    let record_reason = match record.map(|record| &record.status) {
        Some(PackageStatus::Revoked { reason }) => reason.clone(),
        _ => String::new(),
    };
    if let Some(block) = chain.get_block_by_height(leaf.height)? {
        for tx in block.transactions.iter().rev() {
            match tx {
                Transaction::RevokeV2 {
                    request,
                    witness,
                    quorum_certificate,
                    ..
                } if request.package_canonical.trim() == canonical => {
                    let authority = if quorum_certificate.is_some() {
                        RevocationAuthorityV1::CommitteeQuorum
                    } else if record.is_some_and(|record| {
                        record
                            .publisher_pubkey
                            .eq_ignore_ascii_case(&request.revoker_pubkey)
                    }) {
                        RevocationAuthorityV1::Owner
                    } else if certified {
                        RevocationAuthorityV1::Controller
                    } else {
                        RevocationAuthorityV1::LegacySingleValidator
                    };
                    return Ok(RevocationDetail {
                        authority,
                        reason: request.reason.clone(),
                        witness_digest: witness.as_ref().map(|w| w.canonical_digest()),
                        certificate_digest: quorum_certificate.as_ref().map(|qc| qc.digest()),
                    });
                }
                Transaction::Revoke {
                    package_canonical,
                    reason,
                    ..
                } if package_canonical.trim() == canonical => {
                    return Ok(RevocationDetail {
                        authority: RevocationAuthorityV1::LegacySingleValidator,
                        reason: reason.clone(),
                        witness_digest: None,
                        certificate_digest: None,
                    });
                }
                Transaction::QuarantineResolutionV1 {
                    package_canonical,
                    outcome: QuarantineOutcomeV1::Reject,
                    quorum_certificate,
                    ..
                } if package_canonical.trim() == canonical => {
                    return Ok(RevocationDetail {
                        authority: RevocationAuthorityV1::CommitteeQuorum,
                        reason: record_reason,
                        witness_digest: None,
                        certificate_digest: Some(quorum_certificate.digest()),
                    });
                }
                _ => {}
            }
        }
    }
    // No transaction at that height: the deterministic quarantine expiry.
    Ok(RevocationDetail {
        authority: RevocationAuthorityV1::QuarantineExpiry,
        reason: record_reason,
        witness_digest: None,
        certificate_digest: None,
    })
}

fn status_from_leaf(
    chain: &ChainStore,
    canonical: &str,
    leaf: &StatusLeafV1,
    record: Option<&ChainRecord>,
) -> Result<PackageStatusViewV1> {
    Ok(match leaf.status {
        StatusLeafStateV1::Verified => PackageStatusViewV1::Verified {
            height: leaf.height,
            block_hash: chain
                .get_block_by_height(leaf.height)?
                .map(|block| block.hash())
                .unwrap_or_default(),
        },
        StatusLeafStateV1::Quarantined {
            expires_at_unix,
            reason_code,
        } => PackageStatusViewV1::Quarantined {
            height: leaf.height,
            expires_at_unix,
            reason_code,
            committee: record.map(CommitteeSummaryV1::for_record),
        },
        StatusLeafStateV1::Revoked { certified } => {
            let detail = revocation_detail(chain, canonical, leaf, certified, record)?;
            PackageStatusViewV1::Revoked {
                height: leaf.height,
                authority: detail.authority,
                reason: DisplayText::bounded(&detail.reason, MAX_REASON_CHARS),
                witness_digest: detail.witness_digest,
                certificate_digest: detail.certificate_digest,
            }
        }
    })
}

/// Current view of one version, projected to `now_unix` (a quarantine past
/// its deadline shows as revoked, `ECO-INV-02`). `pending_content_hash` is
/// the pending-pool entry for a version not yet on chain.
pub fn package_view(
    chain: &ChainStore,
    canonical: &str,
    now_unix: i64,
    pending_content_hash: Option<&str>,
) -> Result<Option<PackageViewV4>> {
    let leaf = chain.status_leaf(canonical)?;
    let record = chain.get_package(canonical)?;
    let (status, content_hash) = match (&leaf, &record) {
        (Some(leaf), _) => (
            status_from_leaf(chain, canonical, leaf, record.as_ref())?,
            leaf.content_hash.clone(),
        ),
        (None, Some(record)) => (PackageStatusViewV1::Unknown, record.content_hash.clone()),
        (None, None) => match pending_content_hash {
            Some(hash) => (PackageStatusViewV1::Pending, hash.to_string()),
            None => return Ok(None),
        },
    };
    let quorum = match &record {
        Some(record) => publication(chain, record)?
            .map(|(block, index)| QuorumLabelV1::for_publication(&block.transactions[index]))
            .unwrap_or(QuorumLabelV1::LegacyV3Quorum),
        None => QuorumLabelV1::LegacyV3Quorum,
    };
    let mut view = PackageViewV4::new(canonical, &content_hash, status.at(now_unix), quorum);
    view.committee = record.as_ref().map(CommitteeSummaryV1::for_record);
    view.witness_digests = chain
        .witnesses_for(canonical)?
        .into_iter()
        .map(|entry| entry.witness_digest)
        .collect();
    Ok(Some(view))
}

fn status_transaction_for_leaf(
    chain: &ChainStore,
    canonical: &str,
    leaf: &StatusLeafV1,
    publication_height: u64,
) -> Result<Option<Transaction>> {
    match leaf.status {
        StatusLeafStateV1::Verified => {
            if leaf.height == publication_height {
                return Ok(None);
            }
            let Some(block) = chain.get_block_by_height(leaf.height)? else {
                return Ok(None);
            };
            Ok(block
                .transactions
                .iter()
                .rev()
                .find(|tx| {
                    matches!(
                        tx,
                        Transaction::QuarantineResolutionV1 {
                            package_canonical,
                            outcome: QuarantineOutcomeV1::Release,
                            ..
                        } if package_canonical.trim() == canonical
                    )
                })
                .cloned())
        }
        StatusLeafStateV1::Quarantined { .. } => {
            let Some(block) = chain.get_block_by_height(leaf.height)? else {
                return Ok(None);
            };
            Ok(block
                .transactions
                .iter()
                .rev()
                .find(|tx| {
                    matches!(
                        tx,
                        Transaction::QuarantineV1 { record, .. }
                            if record.id.canonical() == canonical
                    )
                })
                .cloned())
        }
        StatusLeafStateV1::Revoked { .. } => {
            let Some(block) = chain.get_block_by_height(leaf.height)? else {
                return Ok(None);
            };
            Ok(block
                .transactions
                .iter()
                .rev()
                .find(|tx| match tx {
                    Transaction::RevokeV2 { request, .. } => {
                        request.package_canonical.trim() == canonical
                    }
                    Transaction::Revoke {
                        package_canonical, ..
                    } => package_canonical.trim() == canonical,
                    Transaction::QuarantineResolutionV1 {
                        package_canonical,
                        outcome: QuarantineOutcomeV1::Reject,
                        ..
                    } => package_canonical.trim() == canonical,
                    _ => false,
                })
                .cloned())
        }
    }
}

/// Proof bundle against the current tip: tip header and commit signatures,
/// authenticated validator-set transition history, the tip's `status_root`
/// with the version's leaf, proof, and optional status-transition transaction,
/// and the publication's block, header, index, transaction and Merkle path.
pub fn proof_bundle(chain: &ChainStore, canonical: &str) -> Result<Option<ProofBundleV2>> {
    let Some(record) = chain.get_package(canonical)? else {
        return Ok(None);
    };
    let Some((publication_block, tx_index)) = publication(chain, &record)? else {
        return Ok(None);
    };
    let publication_transaction = publication_block.transactions[tx_index].clone();
    let merkle = resolver::light_client::build_merkle_proof(&publication_block, tx_index)?;
    let (package, version) =
        split_canonical(canonical).context("package id cannot be split into name and version")?;
    for _ in 0..PROOF_BUNDLE_ATTEMPTS {
        let tip_height = chain.tip_height()?;
        let tip = chain
            .get_block_by_height(tip_height)?
            .context("tip block is missing")?;
        let (status_leaf, status_proof) = match tip.header.status_root {
            Some(root) => {
                let tree = chain.status_tree()?;
                let leaf = tree.get(package, version).cloned();
                let proof = tree.prove(package, version);
                // A block landing between the reads changes the tree; only a
                // proof that verifies against this tip is served.
                if chain.tip_height()? != tip_height
                    || !proof.verify(&root, package, version, leaf.as_ref())
                {
                    continue;
                }
                (leaf, Some(proof))
            }
            None => (None, None),
        };
        let committed_leaf = match &status_leaf {
            Some(leaf) => Some(leaf.clone()),
            None => chain.status_leaf(canonical)?,
        };
        let status_transaction = match committed_leaf.as_ref() {
            Some(leaf) => status_transaction_for_leaf(
                chain,
                canonical,
                leaf,
                publication_block.header.height,
            )?,
            None => None,
        };
        let authority_history: Vec<common::AuthenticatedValidatorSetSnapshot> =
            crate::validator_set_history::load_checked(chain.data_dir())?
                .into_iter()
                .filter(|entry| entry.effective_from_height <= tip_height)
                .filter_map(|entry| entry.certificate)
                .collect();
        if chain.tip_height()? != tip_height {
            continue;
        }
        return Ok(Some(ProofBundleV2 {
            schema: PROOF_BUNDLE_SCHEMA.to_string(),
            canonical: DisplayText::bounded(canonical, 512),
            tip_height,
            tip_block_hash: tip.hash(),
            tip_timestamp_unix: tip.header.timestamp.timestamp(),
            tip_header: tip.header.clone(),
            tip_commit_signatures: tip.pbft_signatures.clone(),
            authority_history,
            status_root: ProofBundleV2::status_root_from(tip.header.status_root.as_ref()),
            status_leaf,
            status_proof,
            status_transaction,
            publication_height: publication_block.header.height,
            publication_block_hash: publication_block.hash(),
            publication_header: publication_block.header.clone(),
            publication_tx_index: u32::try_from(tx_index).unwrap_or(u32::MAX),
            publication_tx_hash: merkle.tx_hash.clone(),
            publication_transaction,
            publication_merkle_path: merkle
                .path
                .iter()
                .map(|step| MerkleStepV1 {
                    sibling_hash: step.sibling_hash.clone(),
                    is_right: step.is_right,
                })
                .collect(),
            publication_commit_signatures: publication_block.pbft_signatures.clone(),
            quorum: QuorumLabelV1::for_publication(&publication_block.transactions[tx_index]),
        }));
    }
    bail!("the chain advanced while the proof bundle was built; retry")
}

/// Witnesses committed for one version (`witness:<package>:<version>`).
pub fn witness_bundle(chain: &ChainStore, canonical: &str) -> Result<WitnessBundleV1> {
    let witnesses = chain
        .witnesses_for(canonical)?
        .into_iter()
        .map(|entry| {
            let verification = if entry.qc_digest.is_some() {
                WitnessVerificationV1::OnChainCertified {
                    height: entry.height,
                }
            } else {
                WitnessVerificationV1::OnChainUncertified {
                    height: entry.height,
                }
            };
            WitnessViewV1::from_witness(&entry.witness, verification)
        })
        .collect();
    Ok(WitnessBundleV1 {
        schema: WITNESS_BUNDLE_SCHEMA.to_string(),
        canonical: DisplayText::bounded(canonical, 512),
        witnesses,
    })
}

/// Versions still quarantined at `now_unix`, earliest deadline first.
pub fn quarantine_list(
    chain: &ChainStore,
    limit: usize,
    now_unix: i64,
) -> Result<QuarantineListV1> {
    let mut packages = Vec::new();
    let mut truncated = false;
    for (_, canonical) in chain.quarantine_expiries_up_to(i64::MAX)? {
        let Some(view) = package_view(chain, &canonical, now_unix, None)? else {
            continue;
        };
        if !matches!(view.status, PackageStatusViewV1::Quarantined { .. }) {
            continue;
        }
        if packages.len() == limit {
            truncated = true;
            break;
        }
        packages.push(view);
    }
    Ok(QuarantineListV1 {
        schema: QUARANTINE_LIST_SCHEMA.to_string(),
        tip_height: chain.tip_height()?,
        packages,
        truncated,
    })
}

// ── HTTP handlers ────────────────────────────────────────────────────────────

async fn chain_of(state: &SharedState) -> ChainStore {
    state.read().await.chain.clone()
}

async fn get_status(
    State(state): State<SharedState>,
    Path((name, version)): Path<(String, String)>,
) -> Response {
    let canonical = match canonical_from_path(&name, &version) {
        Ok(canonical) => canonical,
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    let (chain, pending) = {
        let s = state.read().await;
        (
            s.chain.clone(),
            s.pending_pool
                .get(&canonical)
                .map(|entry| entry.request.content_hash.clone()),
        )
    };
    let now = chrono::Utc::now().timestamp();
    match package_view(&chain, &canonical, now, pending.as_deref()) {
        Ok(Some(view)) => Json(view).into_response(),
        Ok(None) => error(
            StatusCode::NOT_FOUND,
            format!("unknown package {canonical}"),
        ),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

async fn get_proof_bundle(
    State(state): State<SharedState>,
    Path((name, version)): Path<(String, String)>,
) -> Response {
    let canonical = match canonical_from_path(&name, &version) {
        Ok(canonical) => canonical,
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    let chain = chain_of(&state).await;
    match proof_bundle(&chain, &canonical) {
        Ok(Some(bundle)) => Json(bundle).into_response(),
        Ok(None) => error(
            StatusCode::NOT_FOUND,
            format!("no committed publication for {canonical}"),
        ),
        Err(e) => error(StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")),
    }
}

async fn get_witnesses(
    State(state): State<SharedState>,
    Path((name, version)): Path<(String, String)>,
) -> Response {
    let canonical = match canonical_from_path(&name, &version) {
        Ok(canonical) => canonical,
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    let chain = chain_of(&state).await;
    match witness_bundle(&chain, &canonical) {
        Ok(bundle) => Json(bundle).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

#[derive(Debug, Deserialize)]
struct QuarantineQuery {
    limit: Option<usize>,
}

async fn get_quarantine(
    State(state): State<SharedState>,
    Query(query): Query<QuarantineQuery>,
) -> Response {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_QUARANTINE_LIMIT)
        .clamp(1, MAX_QUARANTINE_LIMIT);
    let chain = chain_of(&state).await;
    match quarantine_list(&chain, limit, chrono::Utc::now().timestamp()) {
        Ok(list) => Json(list).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

// ── SSE v2 ───────────────────────────────────────────────────────────────────

/// Bounded subscriber registry: a global semaphore plus a per-IP count.
pub struct SseSubscribers {
    global: Arc<Semaphore>,
    per_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
    max_per_ip: usize,
}

/// Held by one open stream; releases both slots when the stream drops.
pub struct SseSlot {
    _global: OwnedSemaphorePermit,
    ip: IpAddr,
    per_ip: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

impl Drop for SseSlot {
    fn drop(&mut self) {
        if let Ok(mut counts) = self.per_ip.lock() {
            if let Some(count) = counts.get_mut(&self.ip) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    counts.remove(&self.ip);
                }
            }
        }
    }
}

impl SseSubscribers {
    pub fn new(max_total: usize, max_per_ip: usize) -> Self {
        Self {
            global: Arc::new(Semaphore::new(max_total)),
            per_ip: Arc::new(Mutex::new(HashMap::new())),
            max_per_ip,
        }
    }

    /// A slot for `ip`, or `None` when either cap is reached.
    pub fn acquire(&self, ip: IpAddr) -> Option<SseSlot> {
        let mut counts = self.per_ip.lock().ok()?;
        let current = counts.get(&ip).copied().unwrap_or(0);
        if current >= self.max_per_ip {
            return None;
        }
        let permit = Arc::clone(&self.global).try_acquire_owned().ok()?;
        counts.insert(ip, current + 1);
        Some(SseSlot {
            _global: permit,
            ip,
            per_ip: Arc::clone(&self.per_ip),
        })
    }
}

fn sse_subscribers() -> &'static SseSubscribers {
    static SUBSCRIBERS: OnceLock<SseSubscribers> = OnceLock::new();
    SUBSCRIBERS
        .get_or_init(|| SseSubscribers::new(MAX_SSE_V2_SUBSCRIBERS, MAX_SSE_V2_SUBSCRIBERS_PER_IP))
}

/// First block to stream and the last `tx_index` already delivered in it.
/// Without `Last-Event-ID` the stream starts after the current tip; a resume
/// point older than the replay window restarts at the window's start.
pub fn resume_point(tip_height: u64, last_event_id: Option<(u64, u32)>) -> (u64, Option<u32>) {
    match last_event_id {
        Some((height, tx_index)) => {
            let floor = tip_height.saturating_sub(SSE_V2_REPLAY_WINDOW_BLOCKS);
            if height < floor {
                (floor, None)
            } else {
                (height, Some(tx_index))
            }
        }
        None => (tip_height.saturating_add(1), None),
    }
}

struct SseCursor {
    state: SharedState,
    next_height: u64,
    delivered_through: Option<u32>,
    queue: VecDeque<Event>,
    _slot: SseSlot,
}

impl SseCursor {
    /// Queue events of up to `SSE_V2_BLOCKS_PER_POLL` new blocks; false when
    /// there is nothing new yet.
    async fn fill(&mut self) -> Result<bool> {
        let chain = chain_of(&self.state).await;
        let tip = chain.tip_height()?;
        if self.next_height > tip {
            return Ok(false);
        }
        let last = tip.min(
            self.next_height
                .saturating_add(SSE_V2_BLOCKS_PER_POLL)
                .saturating_sub(1),
        );
        for height in self.next_height..=last {
            let Some(block) = chain.get_block_by_height(height)? else {
                break;
            };
            for envelope in registry_view::block_events(&block) {
                if self
                    .delivered_through
                    .is_some_and(|delivered| envelope.tx_index <= delivered)
                {
                    continue;
                }
                self.queue.push_back(
                    Event::default()
                        .id(envelope.id.clone())
                        .event(envelope.kind.as_str())
                        .data(serde_json::to_string(&envelope)?),
                );
            }
            self.delivered_through = None;
            self.next_height = height.saturating_add(1);
        }
        Ok(true)
    }
}

fn event_stream(cursor: SseCursor) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    stream::unfold(cursor, |mut cursor| async move {
        loop {
            if let Some(event) = cursor.queue.pop_front() {
                return Some((Ok(event), cursor));
            }
            match cursor.fill().await {
                Ok(true) => {}
                Ok(false) => tokio::time::sleep(SSE_V2_POLL_INTERVAL).await,
                Err(e) => {
                    tracing::warn!("SSE v2 stream read failed: {e:#}");
                    tokio::time::sleep(SSE_V2_POLL_INTERVAL).await;
                }
            }
        }
    })
}

async fn get_events(
    State(state): State<SharedState>,
    limiter: Option<Extension<RateLimiter>>,
    connect: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Response {
    let peer = connect
        .map(|ConnectInfo(addr)| addr.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let ip = match &limiter {
        Some(Extension(limiter)) => {
            crate::rate_limit::extract_client_ip(peer, &headers, limiter.trusted_proxies())
        }
        None => peer,
    };
    let Some(slot) = sse_subscribers().acquire(ip) else {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "too many SSE v2 subscribers; retry later",
        );
    };
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(SseEnvelopeV2::parse_id);
    let tip = chain_of(&state).await.tip_height().unwrap_or(0);
    let (next_height, delivered_through) = resume_point(tip, last_event_id);
    let cursor = SseCursor {
        state,
        next_height,
        delivered_through,
        queue: VecDeque::new(),
        _slot: slot,
    };
    Sse::new(event_stream(cursor))
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_status::test_fixture::*;
    use common::{CregFindingCategoryV1, QUARANTINE_QC_DOMAIN, REVOKE_QC_DOMAIN};

    #[test]
    fn path_segments_form_a_bounded_canonical_id() {
        assert_eq!(
            canonical_from_path("npm:@scope/pkg", "1.2.3").unwrap(),
            "npm:@scope/pkg@1.2.3"
        );
        assert!(canonical_from_path("", "1.0.0").is_err());
        assert!(canonical_from_path("npm:x", "1.0.0@2").is_err());
        assert!(canonical_from_path("npm:x\u{1b}[31m", "1.0.0").is_err());
        assert!(canonical_from_path(&"a".repeat(MAX_PACKAGE_NAME_CHARS + 1), "1").is_err());
    }

    #[test]
    fn views_follow_the_committed_status_lattice() {
        let fx = fixture(Some(1));
        let rec = record("viewpkg", "publisher");
        let canonical = rec.id.canonical();
        fx.commit(fx.block(10, vec![Transaction::Publish(rec.clone())]))
            .expect("publish");
        let now = fx.start.timestamp() + 10;

        let view = package_view(&fx.store, &canonical, now, None)
            .expect("view")
            .expect("known");
        assert!(matches!(
            view.status,
            PackageStatusViewV1::Verified { height: 1, .. }
        ));
        assert_eq!(view.quorum, QuorumLabelV1::LegacyV3Quorum);
        assert!(package_view(&fx.store, "npm:absent@1.0.0", now, None)
            .expect("view")
            .is_none());
        assert!(matches!(
            package_view(&fx.store, "npm:absent@1.0.0", now, Some("ab"))
                .expect("view")
                .expect("pending")
                .status,
            PackageStatusViewV1::Pending
        ));

        // Quarantine: listed, and shown revoked once the deadline passes.
        let hold = fx.qc(QUARANTINE_QC_DOMAIN, &canonical, &rec.content_hash, None, 3);
        let held = fx
            .commit(fx.block(
                20,
                vec![Transaction::QuarantineV1 {
                    record: rec.clone(),
                    reason_code: CregFindingCategoryV1::Typosquat,
                    quorum_certificate: hold,
                }],
            ))
            .expect("quarantine");
        let deadline = held.header.timestamp.timestamp() + common::CLASS_C_QUARANTINE_TTL_SECS;
        let list = quarantine_list(&fx.store, 10, now).expect("list");
        assert_eq!(list.packages.len(), 1);
        assert!(!list.truncated);
        assert!(matches!(
            list.packages[0].status,
            PackageStatusViewV1::Quarantined { expires_at_unix, .. } if expires_at_unix == deadline
        ));
        assert!(quarantine_list(&fx.store, 10, deadline)
            .expect("list")
            .packages
            .is_empty());
        assert!(matches!(
            package_view(&fx.store, &canonical, deadline, None)
                .expect("view")
                .expect("known")
                .status,
            PackageStatusViewV1::Revoked {
                authority: RevocationAuthorityV1::QuarantineExpiry,
                ..
            }
        ));

        // A certified committee revocation names its authority and QC.
        let validator_key = &fx.members[0].ed;
        let revoke_qc = fx.qc(REVOKE_QC_DOMAIN, &canonical, &rec.content_hash, None, 3);
        let certificate_digest = revoke_qc.digest();
        fx.commit(fx.block(
            30,
            vec![revoke_v2(
                revocation(validator_key, &canonical),
                Some(revoke_qc),
            )],
        ))
        .expect("certified revocation");
        let view = package_view(&fx.store, &canonical, now, None)
            .expect("view")
            .expect("known");
        match view.status {
            PackageStatusViewV1::Revoked {
                authority,
                certificate_digest: digest,
                reason,
                ..
            } => {
                assert_eq!(authority, RevocationAuthorityV1::CommitteeQuorum);
                assert_eq!(digest.as_deref(), Some(certificate_digest.as_str()));
                assert_eq!(reason.as_str(), "malicious install hook");
            }
            other => panic!("expected revoked, got {other:?}"),
        }
    }

    #[test]
    fn proof_bundle_verifies_against_the_tip_status_root() {
        let fx = fixture(Some(1));
        let rec = record("proofpkg", "publisher");
        let canonical = rec.id.canonical();
        fx.commit(fx.block(10, vec![Transaction::Publish(rec.clone())]))
            .expect("publish");
        fx.commit(fx.block(20, vec![])).expect("empty block");

        let bundle = proof_bundle(&fx.store, &canonical)
            .expect("bundle")
            .expect("published");
        assert_eq!(bundle.tip_height, 2);
        assert_eq!(bundle.publication_height, 1);
        assert_eq!(bundle.publication_tx_index, 0);
        assert_eq!(
            Block {
                header: bundle.tip_header.clone(),
                transactions: vec![],
                pbft_signatures: vec![],
            }
            .hash(),
            bundle.tip_block_hash
        );
        assert_eq!(
            Block {
                header: bundle.publication_header.clone(),
                transactions: vec![],
                pbft_signatures: vec![],
            }
            .hash(),
            bundle.publication_block_hash
        );
        assert_eq!(
            common::transaction_hash(&bundle.publication_transaction),
            bundle.publication_tx_hash
        );
        assert!(bundle.status_transaction.is_none());
        assert!(bundle.authority_history.is_empty());
        let root = common::status_tree::hex32::parse(bundle.status_root.as_deref().expect("root"))
            .expect("hex root");
        let leaf = bundle.status_leaf.as_ref().expect("leaf");
        assert_eq!(leaf.status, StatusLeafStateV1::Verified);
        assert!(bundle.status_proof.as_ref().expect("proof").verify(
            &root,
            "npm:proofpkg",
            "1.0.0",
            Some(leaf)
        ));
        // The proof binds the leaf: a forged state does not verify.
        let mut forged = leaf.clone();
        forged.status = StatusLeafStateV1::Revoked { certified: true };
        assert!(!bundle.status_proof.as_ref().expect("proof").verify(
            &root,
            "npm:proofpkg",
            "1.0.0",
            Some(&forged)
        ));
        assert!(proof_bundle(&fx.store, "npm:absent@1.0.0")
            .expect("bundle")
            .is_none());
    }

    #[test]
    fn proof_bundle_carries_status_transactions_and_authority_history() {
        use common::{
            AuthenticatedValidatorSetSnapshot, ValidatorSet, ValidatorSetTransitionSignature,
            QUARANTINE_RESOLUTION_QC_DOMAIN, VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
        };

        let fx = fixture(Some(1));
        let rec_q = record("held-then-released", "publisher");
        let can_q = rec_q.id.canonical();
        let hold_qc = fx.qc(QUARANTINE_QC_DOMAIN, &can_q, &rec_q.content_hash, None, 3);
        let b1 = fx
            .commit(fx.block(
                10,
                vec![
                    Transaction::Publish(rec_q.clone()),
                    Transaction::QuarantineV1 {
                        record: rec_q.clone(),
                        reason_code: CregFindingCategoryV1::CredentialExfiltration,
                        quorum_certificate: hold_qc,
                    },
                ],
            ))
            .expect("publish + hold");
        let b1_bundle = proof_bundle(&fx.store, &can_q)
            .expect("bundle")
            .expect("held");
        assert!(matches!(
            b1_bundle.status_transaction,
            Some(Transaction::QuarantineV1 { .. })
        ));

        // Record certified authority transitions at height 2 (<= tip) and 999 (> tip).
        let bootstrap = ValidatorSet::new(fx.members.iter().map(|m| m.validator.clone()).collect());
        let bootstrap_hash = common::canonical_validator_set_hash(&bootstrap);
        let make_transition = |effective_from_height: u64, previous_block_hash: String| {
            let mut snap = AuthenticatedValidatorSetSnapshot {
                schema_version: VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
                effective_from_height,
                previous_block_hash,
                previous_validator_set_hash: bootstrap_hash.clone(),
                validators: bootstrap.clone(),
                validator_set_hash: bootstrap_hash.clone(),
                transition_signatures: vec![],
            };
            let msg = common::validator_set_transition_message(&snap);
            for m in fx.members.iter().take(3) {
                snap.transition_signatures
                    .push(ValidatorSetTransitionSignature {
                        validator_id: m.validator.id.clone(),
                        validator_pubkey: m.validator.pubkey.clone(),
                        signature: m.ed.sign(msg.as_bytes()).to_hex(),
                        pq_signature: None,
                    });
            }
            snap
        };
        crate::validator_set_history::record_authenticated(
            fx.store.data_dir(),
            make_transition(2, b1.hash()),
            &bootstrap,
            None,
        )
        .expect("record height 2 transition");
        crate::validator_set_history::record_authenticated(
            fx.store.data_dir(),
            make_transition(999, "ff".repeat(32)),
            &bootstrap,
            None,
        )
        .expect("record future transition");

        // Block 2: release pkg-q and publish pkg-r.
        let rec_r = record("revoked-pkg", "publisher");
        let can_r = rec_r.id.canonical();
        let release_qc = fx.qc(
            QUARANTINE_RESOLUTION_QC_DOMAIN,
            &can_q,
            &rec_q.content_hash,
            Some(QuarantineOutcomeV1::Release),
            3,
        );
        fx.commit(fx.block(
            20,
            vec![
                Transaction::QuarantineResolutionV1 {
                    package_canonical: can_q.clone(),
                    content_hash: rec_q.content_hash.clone(),
                    outcome: QuarantineOutcomeV1::Release,
                    quorum_certificate: release_qc,
                },
                Transaction::Publish(rec_r.clone()),
            ],
        ))
        .expect("release + publish r");
        let released_bundle = proof_bundle(&fx.store, &can_q)
            .expect("bundle")
            .expect("released");
        assert!(matches!(
            released_bundle.status_transaction,
            Some(Transaction::QuarantineResolutionV1 {
                outcome: QuarantineOutcomeV1::Release,
                ..
            })
        ));
        assert_eq!(released_bundle.authority_history.len(), 1);
        assert_eq!(
            released_bundle.authority_history[0].effective_from_height,
            2
        );

        // Block 3: certified RevokeV2 on pkg-r, and publish + QuarantineV1 on pkg-exp.
        let rec_exp = record("expired-pkg", "publisher");
        let can_exp = rec_exp.id.canonical();
        let revoke_qc = fx.qc(REVOKE_QC_DOMAIN, &can_r, &rec_r.content_hash, None, 3);
        let exp_qc = fx.qc(
            QUARANTINE_QC_DOMAIN,
            &can_exp,
            &rec_exp.content_hash,
            None,
            3,
        );
        fx.commit(fx.block(
            30,
            vec![
                revoke_v2(revocation(&fx.members[0].ed, &can_r), Some(revoke_qc)),
                Transaction::Publish(rec_exp.clone()),
                Transaction::QuarantineV1 {
                    record: rec_exp.clone(),
                    reason_code: CregFindingCategoryV1::Backdoor,
                    quorum_certificate: exp_qc,
                },
            ],
        ))
        .expect("revoke + hold exp");
        let revoked_bundle = proof_bundle(&fx.store, &can_r)
            .expect("bundle")
            .expect("revoked");
        assert!(matches!(
            revoked_bundle.status_transaction,
            Some(Transaction::RevokeV2 {
                quorum_certificate: Some(_),
                ..
            })
        ));

        // Block 4: advance past quarantine TTL -> deterministic expiry leaves status_transaction = None.
        fx.commit(fx.block(30 + common::CLASS_C_QUARANTINE_TTL_SECS + 1, vec![]))
            .expect("expiry block");
        let expired_bundle = proof_bundle(&fx.store, &can_exp)
            .expect("bundle")
            .expect("expired");
        assert_eq!(
            expired_bundle.status_leaf.as_ref().expect("leaf").status,
            StatusLeafStateV1::Revoked { certified: true }
        );
        assert!(expired_bundle.status_transaction.is_none());
    }

    #[test]
    fn pre_activation_proof_bundle_carries_no_status_proof() {
        let fx = fixture(None);
        let rec = record("legacypkg", "publisher");
        fx.commit(fx.block(10, vec![Transaction::Publish(rec.clone())]))
            .expect("publish");
        let bundle = proof_bundle(&fx.store, &rec.id.canonical())
            .expect("bundle")
            .expect("published");
        assert!(bundle.status_root.is_none());
        assert!(bundle.status_leaf.is_none() && bundle.status_proof.is_none());
        assert!(!bundle.publication_tx_hash.is_empty());
    }

    #[test]
    fn block_events_have_unique_ordered_ids_ending_with_the_block() {
        let fx = fixture(Some(1));
        let rec = record("eventpkg", "publisher");
        let block = fx
            .commit(fx.block(10, vec![Transaction::Publish(rec.clone())]))
            .expect("publish");
        let events = registry_view::block_events(&block);
        let ids: Vec<&str> = events.iter().map(|event| event.id.as_str()).collect();
        assert_eq!(ids, vec!["1.0", "1.1"]);
        assert_eq!(
            events[0].kind,
            registry_view::SseEventKindV2::PackageVerified
        );
        assert_eq!(
            events[1].kind,
            registry_view::SseEventKindV2::BlockCommitted
        );
        assert!(events[1].data["status_root"].is_string());
        let frame = events[0].to_sse_frame();
        assert!(frame.starts_with("id: 1.0\nevent: package_verified\ndata: {"));
        assert_eq!(frame.matches('\n').count(), 4, "single-line payload");
    }

    #[test]
    fn resume_points_and_subscriber_caps_are_bounded() {
        assert_eq!(resume_point(10, None), (11, None));
        assert_eq!(resume_point(10, Some((7, 2))), (7, Some(2)));
        let tip = 5_000;
        assert_eq!(
            resume_point(tip, Some((3, 0))),
            (tip - SSE_V2_REPLAY_WINDOW_BLOCKS, None)
        );

        let subscribers = SseSubscribers::new(3, 2);
        let a: IpAddr = "198.51.100.1".parse().unwrap();
        let b: IpAddr = "198.51.100.2".parse().unwrap();
        let first = subscribers.acquire(a).expect("first");
        let _second = subscribers.acquire(a).expect("second");
        assert!(subscribers.acquire(a).is_none(), "per-IP cap");
        let _third = subscribers.acquire(b).expect("other ip");
        assert!(subscribers.acquire(b).is_none(), "global cap");
        drop(first);
        assert!(subscribers.acquire(a).is_some(), "slot released on drop");
    }
}
