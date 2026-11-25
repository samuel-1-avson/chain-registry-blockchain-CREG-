//! Rate-limited read-only HTTP views and durable SSE v2 replay.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use common::{CommitteeQuorumCertificateV1, DisplayText};
use registry_view::{
    index::{
        BountyAwardV1, BountyLeaderboardV1, BountyProverV1, CommitteeDiversityV1,
        IndexEvidenceEntryV1, IndexEvidenceHistoryV1, ModelFamilyCountV1, QuarantineQueueEntryV1,
        QuarantineQueueV1,
    },
    SseEnvelopeV2, MAX_REASON_CHARS,
};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use sqlx::{types::Json as PgJson, PgPool};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAX_PAGE: i64 = 200;
const RATE_WINDOW: Duration = Duration::from_secs(60);
const REQUESTS_PER_WINDOW: u32 = 120;
const MAX_TRACKED_CLIENTS: usize = 4096;

#[derive(Clone)]
pub struct ApiState {
    pool: PgPool,
    chain_ready: Arc<AtomicBool>,
    bounty_ready: Arc<AtomicBool>,
    limiter: Arc<Mutex<RateWindow>>,
    streams: Arc<Semaphore>,
}

struct RateWindow {
    started: Instant,
    clients: HashMap<IpAddr, u32>,
    overflow: u32,
}

impl RateWindow {
    fn allow(&mut self, ip: IpAddr) -> bool {
        if self.started.elapsed() >= RATE_WINDOW {
            self.started = Instant::now();
            self.clients.clear();
            self.overflow = 0;
        }
        // Use a shared bounded bucket under high address churn. IPv6 clients
        // are grouped by /64, and forwarded headers do not determine identity.
        let ip = match ip {
            IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or(
                IpAddr::V6((u128::from(ip) & (u128::MAX << 64)).into()),
                IpAddr::V4,
            ),
            _ => ip,
        };
        let count = if self.clients.len() < MAX_TRACKED_CLIENTS || self.clients.contains_key(&ip) {
            self.clients.entry(ip).or_default()
        } else {
            &mut self.overflow
        };
        if *count >= REQUESTS_PER_WINDOW {
            return false;
        }
        *count += 1;
        true
    }
}

impl ApiState {
    pub fn new(pool: PgPool, chain_ready: Arc<AtomicBool>, bounty_ready: Arc<AtomicBool>) -> Self {
        Self {
            pool,
            chain_ready,
            bounty_ready,
            limiter: Arc::new(Mutex::new(RateWindow {
                started: Instant::now(),
                clients: HashMap::new(),
                overflow: 0,
            })),
            streams: Arc::new(Semaphore::new(32)),
        }
    }

    pub fn router(self) -> Router {
        Router::new()
            .route("/v1/index/quarantine-queue", get(quarantine_queue))
            .route(
                "/v1/index/packages/:canonical/evidence-history",
                get(evidence_history),
            )
            .route("/v1/index/committee-diversity", get(committee_diversity))
            .route("/v1/index/bounties/leaderboard", get(leaderboard))
            .route("/v1/index/events", get(events))
            .route_layer(middleware::from_fn_with_state(self.clone(), limit_requests))
            .route("/health", get(health))
            .with_state(self)
    }
}

async fn limit_requests(State(state): State<ApiState>, request: Request, next: Next) -> Response {
    let ip = request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::V4(Ipv4Addr::LOCALHOST), |connection| {
            connection.0.ip()
        });
    let allowed = state
        .limiter
        .lock()
        .is_ok_and(|mut limiter| limiter.allow(ip));
    if !allowed {
        return (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "60")]).into_response();
    }
    if !state.chain_ready.load(Ordering::Acquire) {
        return ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "chain projection unavailable",
        )
        .into_response();
    }
    next.run(request).await
}

async fn health(State(state): State<ApiState>) -> Response {
    let chain = state.chain_ready.load(Ordering::Acquire);
    let bounty = state.bounty_ready.load(Ordering::Acquire);
    let status = if chain && bounty {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({ "chain_ready": chain, "bounty_ready": bounty })),
    )
        .into_response()
}

#[derive(Debug)]
struct ApiError(StatusCode, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(error: E) -> Self {
        tracing::error!(error = %error.into(), "indexer read failed");
        Self(StatusCode::SERVICE_UNAVAILABLE, "indexer read unavailable")
    }
}

#[derive(Deserialize)]
struct PageQuery {
    limit: Option<u16>,
    before: Option<String>,
}

impl PageQuery {
    fn limit(&self) -> i64 {
        i64::from(self.limit.unwrap_or(50)).clamp(1, MAX_PAGE)
    }
}

fn decode_enum<T: DeserializeOwned>(value: String) -> Result<T> {
    Ok(serde_json::from_value(Value::String(value))?)
}

#[derive(sqlx::FromRow)]
struct QueueRow {
    canonical: String,
    content_hash: String,
    reason_code: String,
    opened_height: i64,
    opened_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    committee_summary: PgJson<registry_view::CommitteeSummaryV1>,
}

async fn quarantine_queue(
    State(state): State<ApiState>,
    Query(page): Query<PageQuery>,
) -> Result<Json<QuarantineQueueV1>, ApiError> {
    let limit = page.limit();
    let rows: Vec<QueueRow> = sqlx::query_as(
        "SELECT canonical,content_hash,reason_code,opened_height,opened_at,expires_at,committee_summary
         FROM v_quarantine_queue ORDER BY expires_at,canonical LIMIT $1",
    )
    .bind(limit + 1).fetch_all(&state.pool).await?;
    let truncated = rows.len() > limit as usize;
    let entries = rows
        .into_iter()
        .take(limit as usize)
        .map(|row| {
            Ok(QuarantineQueueEntryV1 {
                canonical: DisplayText::bounded(&row.canonical, 512),
                content_hash: row.content_hash,
                reason_code: decode_enum(row.reason_code)?,
                opened_height: u64::try_from(row.opened_height)?,
                opened_at_unix: row.opened_at.timestamp(),
                expires_at_unix: row.expires_at.timestamp(),
                committee: row.committee_summary.0,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(QuarantineQueueV1 {
        server_time_unix: Utc::now().timestamp(),
        entries,
        truncated,
    }))
}

#[derive(sqlx::FromRow)]
struct EvidenceRow {
    height: i64,
    tx_index: i32,
    source_tx_type: String,
    content_hash: Option<String>,
    status: Option<String>,
    reason_code: Option<String>,
    reason: Option<String>,
    quarantine_expires_at: Option<DateTime<Utc>>,
    committee_summary: Option<PgJson<registry_view::CommitteeSummaryV1>>,
    authority: Option<String>,
    evidence_hash: Option<String>,
    witness_digest: Option<String>,
    quorum_certificate: Option<PgJson<CommitteeQuorumCertificateV1>>,
    certificate_digest: Option<String>,
}

async fn evidence_history(
    State(state): State<ApiState>,
    Path(canonical): Path<String>,
    Query(page): Query<PageQuery>,
) -> Result<Json<IndexEvidenceHistoryV1>, ApiError> {
    if canonical.len() > 512 || common::status_tree::split_canonical(&canonical).is_none() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "invalid canonical package identifier",
        ));
    }
    let limit = page.limit();
    let before = page
        .before
        .as_deref()
        .map(|id| {
            id.split_once('.')
                .and_then(|(height, index)| {
                    Some((height.parse::<i64>().ok()?, index.parse::<i32>().ok()?))
                })
                .filter(|(height, index)| *height >= 0 && *index >= -1)
                .ok_or(ApiError(StatusCode::BAD_REQUEST, "invalid evidence cursor"))
        })
        .transpose()?;
    let (height, index) = before.unwrap_or((i64::MAX, i32::MAX));
    let rows: Vec<EvidenceRow> = sqlx::query_as(
        "SELECT COALESCE(h.height,r.height) AS height, COALESCE(h.tx_index,r.tx_index) AS tx_index,
                COALESCE(h.source_tx_type,r.tx_type) AS source_tx_type, h.content_hash,
                h.status,h.reason_code,COALESCE(h.reason,r.reason) AS reason,h.quarantine_expires_at,
                h.committee_summary,r.authority,r.evidence_hash,r.witness_digest,r.quorum_certificate,h.certificate_digest
         FROM (SELECT * FROM package_status_history
               WHERE canonical = $1 AND (height,tx_index) < ($2,$3)) h
         FULL JOIN (SELECT * FROM revocations
                    WHERE canonical = $1 AND (height,tx_index) < ($2,$3)) r
              USING (canonical,height,tx_index)
         ORDER BY height DESC,tx_index DESC LIMIT $4",
    )
    .bind(&canonical).bind(height).bind(index).bind(limit + 1).fetch_all(&state.pool).await?;
    let next_before = (rows.len() > limit as usize).then(|| {
        let row = &rows[limit as usize - 1];
        format!("{}.{}", row.height, row.tx_index)
    });
    let entries = rows
        .into_iter()
        .take(limit as usize)
        .map(|row| {
            let authority = row
                .authority
                .map(|value| {
                    if value == "ttl_expiry" {
                        Ok(registry_view::RevocationAuthorityV1::QuarantineExpiry)
                    } else {
                        decode_enum(value)
                    }
                })
                .transpose()?;
            Ok(IndexEvidenceEntryV1 {
                height: u64::try_from(row.height)?,
                tx_index: row.tx_index,
                source_tx_type: row.source_tx_type,
                content_hash: row.content_hash,
                status: row.status.map(decode_enum).transpose()?,
                reason_code: row.reason_code.map(decode_enum).transpose()?,
                reason: row
                    .reason
                    .map(|reason| DisplayText::bounded(&reason, MAX_REASON_CHARS)),
                quarantine_expires_at_unix: row.quarantine_expires_at.map(|date| date.timestamp()),
                committee: row.committee_summary.map(|summary| summary.0),
                revocation_authority: authority,
                evidence_hash: row.evidence_hash,
                witness_digest: row.witness_digest,
                certificate_digest: row
                    .certificate_digest
                    .or_else(|| row.quorum_certificate.map(|cert| cert.0.digest())),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(IndexEvidenceHistoryV1 {
        canonical: DisplayText::bounded(&canonical, 512),
        entries,
        next_before,
    }))
}

#[derive(Deserialize)]
struct EpochQuery {
    epoch: i64,
}

#[derive(sqlx::FromRow)]
struct DiversityRow {
    observations: i64,
    approvals: i64,
    quarantine_reviews: i64,
    rejections: i64,
    abstentions: i64,
    hybrid_signers: i64,
    classical_signers: i64,
}

async fn committee_diversity(
    State(state): State<ApiState>,
    Query(query): Query<EpochQuery>,
) -> Result<Json<CommitteeDiversityV1>, ApiError> {
    let epoch = u64::try_from(query.epoch)
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "invalid epoch"))?;
    let mut transaction = state.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *transaction)
        .await?;
    let validator_set_hash: String = sqlx::query_scalar(
        "SELECT validator_set_hash FROM blocks WHERE committee_epoch = $1 AND height > 0
         AND height <= (SELECT last_height FROM sync_state WHERE id = 1)
         ORDER BY height LIMIT 1",
    )
    .bind(query.epoch)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(ApiError(StatusCode::NOT_FOUND, "unknown committee epoch"))?;
    let row: DiversityRow = sqlx::query_as(
        "SELECT count(*) AS observations,
             COALESCE(sum((h.committee_summary->>'approvals')::BIGINT),0)::BIGINT AS approvals,
             COALESCE(sum((h.committee_summary->>'quarantine_reviews')::BIGINT),0)::BIGINT AS quarantine_reviews,
             COALESCE(sum((h.committee_summary->>'rejections')::BIGINT),0)::BIGINT AS rejections,
             COALESCE(sum((h.committee_summary->>'abstentions')::BIGINT),0)::BIGINT AS abstentions,
             COALESCE(sum((h.committee_summary->>'hybrid_signers')::BIGINT),0)::BIGINT AS hybrid_signers,
             COALESCE(sum((h.committee_summary->>'classical_signers')::BIGINT),0)::BIGINT AS classical_signers
         FROM package_status_history h JOIN blocks b USING (height)
         WHERE b.committee_epoch = $1 AND h.source_tx_type IN ('publish','publish_authorized_v1')",
    )
    .bind(query.epoch).fetch_one(&mut *transaction).await?;
    let families: Vec<(String, i64)> = sqlx::query_as(
        "SELECT family.key,sum(family.value::BIGINT)::BIGINT AS votes
         FROM package_status_history h JOIN blocks b USING (height)
         CROSS JOIN LATERAL jsonb_each_text(h.committee_summary->'model_families') AS family
         WHERE b.committee_epoch = $1 AND h.source_tx_type IN ('publish','publish_authorized_v1') GROUP BY family.key ORDER BY votes DESC,family.key LIMIT $2",
    )
    .bind(query.epoch)
    .bind(MAX_PAGE + 1)
    .fetch_all(&mut *transaction)
    .await?;
    transaction.commit().await?;
    let model_families_truncated = families.len() > MAX_PAGE as usize;
    let model_families = families
        .into_iter()
        .take(MAX_PAGE as usize)
        .map(|(family, votes)| ModelFamilyCountV1 {
            model_family: DisplayText::bounded(&family, 64),
            votes: votes as u64,
        })
        .collect();
    Ok(Json(CommitteeDiversityV1 {
        epoch,
        validator_set_hash,
        observations: row.observations as u64,
        approvals: row.approvals as u64,
        quarantine_reviews: row.quarantine_reviews as u64,
        rejections: row.rejections as u64,
        abstentions: row.abstentions as u64,
        hybrid_signers: row.hybrid_signers as u64,
        classical_signers: row.classical_signers as u64,
        model_families,
        model_families_truncated,
    }))
}

#[derive(sqlx::FromRow)]
struct LeaderRow {
    l1_chain_id: i64,
    prover_address: String,
    awards: i64,
    matched_witnesses: i64,
    bonus: String,
    reserved: String,
    distributed: String,
    l1_block_number: i64,
    l1_block_hash: String,
    l1_tx_hash: String,
    log_index: i64,
    witness_digest: String,
    package_hash: String,
    canonical: Option<String>,
    witness_height: Option<i64>,
}

async fn leaderboard(
    State(state): State<ApiState>,
    Query(page): Query<PageQuery>,
) -> Result<Json<BountyLeaderboardV1>, ApiError> {
    let limit = page.limit();
    let rows: Vec<LeaderRow> = sqlx::query_as(
        "WITH leaders AS (
             SELECT l1_chain_id,prover_address,count(*) AS awards,count(canonical) AS matched_witnesses,
                    sum(prover_bonus_wei) AS bonus,
                    COALESCE(sum(committee_share_wei) FILTER (WHERE committee_share_status = 'reserved'),0) AS reserved,
                    COALESCE(sum(committee_share_wei) FILTER (WHERE committee_share_status = 'distributed'),0) AS distributed
             FROM v_discovery_bounty_witnesses
             GROUP BY l1_chain_id,prover_address ORDER BY bonus DESC,l1_chain_id,prover_address LIMIT $1
         )
         SELECT l.l1_chain_id,l.prover_address,l.awards,l.matched_witnesses,
                l.bonus::TEXT AS bonus,l.reserved::TEXT AS reserved,l.distributed::TEXT AS distributed,
                a.l1_block_number,a.l1_block_hash,a.l1_tx_hash,a.log_index,a.witness_digest,
                a.package_hash,a.canonical,a.witness_height
         FROM leaders l CROSS JOIN LATERAL (
             SELECT * FROM v_discovery_bounty_witnesses a
             WHERE a.l1_chain_id = l.l1_chain_id AND a.prover_address = l.prover_address
             ORDER BY a.l1_block_number DESC,a.log_index DESC,a.l1_tx_hash LIMIT 1
         ) a ORDER BY l.bonus DESC,l.l1_chain_id,l.prover_address",
    )
    .bind(limit + 1).fetch_all(&state.pool).await?;
    let truncated = rows.len() > limit as usize;
    let entries = rows
        .into_iter()
        .take(limit as usize)
        .map(|row| {
            Ok(BountyProverV1 {
                l1_chain_id: u64::try_from(row.l1_chain_id)?,
                prover_address: row.prover_address,
                awards: u64::try_from(row.awards)?,
                matched_witnesses: u64::try_from(row.matched_witnesses)?,
                prover_bonus_wei: row.bonus,
                committee_reserved_wei: row.reserved,
                committee_distributed_wei: row.distributed,
                latest_award: BountyAwardV1 {
                    l1_block_number: u64::try_from(row.l1_block_number)?,
                    l1_block_hash: row.l1_block_hash,
                    l1_tx_hash: row.l1_tx_hash,
                    log_index: u64::try_from(row.log_index)?,
                    witness_digest: row.witness_digest,
                    package_hash: row.package_hash,
                    canonical: row.canonical.map(|name| DisplayText::bounded(&name, 512)),
                    witness_height: row.witness_height.map(u64::try_from).transpose()?,
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(BountyLeaderboardV1 { entries, truncated }))
}

#[derive(Deserialize)]
struct EventQuery {
    after: Option<String>,
}

struct Replay {
    pool: PgPool,
    cursor: (i64, i64),
    buffered: std::vec::IntoIter<PgJson<SseEnvelopeV2>>,
    _permit: OwnedSemaphorePermit,
}

async fn replay_next(mut state: Replay) -> Result<Option<(Event, Replay)>> {
    loop {
        if let Some(PgJson(envelope)) = state.buffered.next() {
            state.cursor = (
                i64::try_from(envelope.height)?,
                i64::from(envelope.tx_index),
            );
            let event = Event::default()
                .id(&envelope.id)
                .event(envelope.kind.as_str())
                .json_data(envelope)?;
            return Ok(Some((event, state)));
        }
        let batch: Vec<PgJson<SseEnvelopeV2>> = sqlx::query_scalar(
            "SELECT envelope FROM index_events WHERE (height,tx_index) > ($1,$2)
             ORDER BY height,tx_index LIMIT 128",
        )
        .bind(state.cursor.0)
        .bind(state.cursor.1)
        .fetch_all(&state.pool)
        .await?;
        if batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(1)).await;
        } else {
            state.buffered = batch.into_iter();
        }
    }
}

async fn events(
    State(state): State<ApiState>,
    Query(query): Query<EventQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let header = headers
        .get("last-event-id")
        .map(|value| value.to_str())
        .transpose()
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "invalid Last-Event-ID"))?;
    let cursor = match header.or(query.after.as_deref()) {
        Some(id) => {
            let (height, index) = SseEnvelopeV2::parse_id(id)
                .ok_or(ApiError(StatusCode::BAD_REQUEST, "invalid event cursor"))?;
            (
                i64::try_from(height)
                    .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "invalid event height"))?,
                i64::from(index),
            )
        }
        None => sqlx::query_as::<_, (i64, i64)>(
            "SELECT height,tx_index FROM index_events ORDER BY height DESC,tx_index DESC LIMIT 1",
        )
        .fetch_optional(&state.pool)
        .await?
        .unwrap_or((-1, -1)),
    };
    let permit = state
        .streams
        .try_acquire_owned()
        .map_err(|_| ApiError(StatusCode::TOO_MANY_REQUESTS, "too many event streams"))?;
    let replay = Replay {
        pool: state.pool,
        cursor,
        buffered: Vec::new().into_iter(),
        _permit: permit,
    };
    let stream = futures::stream::try_unfold(replay, |state| async {
        replay_next(state)
            .await
            .context("replay index events")
            .inspect_err(|error| {
                tracing::warn!(%error, "index event stream ended");
            })
    });
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}
