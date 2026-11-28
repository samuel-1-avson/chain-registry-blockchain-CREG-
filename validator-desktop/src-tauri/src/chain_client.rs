use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use registry_view::index::{
    CommitteeDiversityV1, IndexEvidenceHistoryV1, QuarantineQueueEntryV1, QuarantineQueueV1,
};
use registry_view::{PackageViewV4, QuarantineListV1, SseEnvelopeV2, WitnessBundleV1};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

const HTTP_TIMEOUT_SECS: u64 = 10;
const MAX_SSE_FRAME_BYTES: usize = 256 * 1024;

#[derive(Clone, Default)]
pub struct SseStreamHandle {
    pub task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOverview {
    pub ok: bool,
    pub chain_id: Option<String>,
    pub tip_height: Option<u64>,
    pub finalized_height: Option<u64>,
    pub active_validators: Option<u64>,
    pub package_count: Option<u64>,
    pub status_text: String,
}

#[derive(Debug, Serialize)]
pub struct R7NodeHealth {
    pub is_validator: bool,
    pub chain_healthy: bool,
    pub chain_lag_blocks: u64,
    pub tip_height: u64,
    pub finalized_height: u64,
    pub peer_count: u64,
    pub sepolia_sync_state: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct R7CertifiedIdentity {
    pub node_id: String,
    pub evm_address: String,
    pub ed25519_pubkey: String,
}

pub(crate) async fn fetch_r7_certified_identity(
    port: u16,
    node_id: &str,
) -> Option<R7CertifiedIdentity> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let profile = client
        .get(format!(
            "http://127.0.0.1:{port}/v1/public/validators/{}",
            encode_segment(node_id)
        ))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()?;
    let identity: R7CertifiedIdentity = if let Some(certified) = profile.get("certified_membership")
    {
        serde_json::from_value(certified.clone()).ok()?
    } else {
        // The deployed v4 endpoint exposes the authoritative identity in
        // these top-level fields, separately from its registration record.
        if profile["in_active_set"].as_bool() != Some(true)
            || !matches!(
                profile["status"].as_str(),
                Some("active" | "online" | "self")
            )
        {
            return None;
        }
        R7CertifiedIdentity {
            node_id: profile["node_id"].as_str()?.to_owned(),
            evm_address: profile["eth_address"].as_str()?.to_owned(),
            ed25519_pubkey: profile["pubkey"].as_str()?.to_owned(),
        }
    };
    (identity.node_id == node_id).then_some(identity)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct R7AdmissionSummary {
    pub stage: String,
    pub stage_reason: String,
    pub certified_member: bool,
    pub registration_conflict: bool,
    pub l1_approvals_collected: u64,
    pub l1_approvals_required: u64,
    pub transition_approvals_collected: u64,
    pub transition_approvals_required: u64,
    pub l1_tx_status: String,
    pub l1_tx_hash: Option<String>,
    pub l1_confirmed_block: Option<u64>,
    pub voting_activation_height: Option<u64>,
    pub first_finalized_vote_height: Option<u64>,
    pub missing_signer_setup: Vec<String>,
    pub unavailable_reason: Option<String>,
}

#[derive(Deserialize)]
struct AdmissionStatusResponse {
    #[serde(default)]
    local_node: AdmissionLocalNodeResponse,
    #[serde(default)]
    applicants: Vec<AdmissionApplicantResponse>,
}

#[derive(Deserialize, Default)]
struct AdmissionLocalNodeResponse {
    #[serde(default)]
    is_validator: bool,
    #[serde(default)]
    admission_signer_configured: bool,
    #[serde(default)]
    admission_signer_issue: Option<String>,
}

#[derive(Deserialize, Default)]
struct AdmissionApplicantResponse {
    #[serde(default)]
    stage: String,
    #[serde(default)]
    stage_reason: String,
    #[serde(default)]
    certified_member: Option<bool>,
    #[serde(default)]
    registration_conflict: bool,
    #[serde(default)]
    l1_approvals_collected: u64,
    #[serde(default)]
    l1_approvals_required: u64,
    #[serde(default)]
    transition_approvals_collected: u64,
    #[serde(default)]
    transition_approvals_required: u64,
    #[serde(default)]
    l1_tx_status: String,
    #[serde(default)]
    l1_tx_hash: Option<String>,
    #[serde(default)]
    l1_confirmed_block: Option<u64>,
    #[serde(default)]
    voting_activation_height: Option<u64>,
    #[serde(default)]
    first_finalized_vote_height: Option<u64>,
    #[serde(default)]
    missing_signer_setup: Vec<String>,
}

enum AdmissionFetchOutcome {
    Applicant(R7AdmissionSummary),
    NotFound,
    Unavailable,
}

async fn fetch_admission_endpoint(
    client: &reqwest::Client,
    url: &str,
) -> AdmissionFetchOutcome {
    let Ok(resp) = client.get(url).send().await else {
        return AdmissionFetchOutcome::Unavailable;
    };
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return AdmissionFetchOutcome::NotFound;
    }
    if !resp.status().is_success() {
        return AdmissionFetchOutcome::Unavailable;
    }
    let Ok(parsed) = resp.json::<AdmissionStatusResponse>().await else {
        return AdmissionFetchOutcome::Unavailable;
    };
    let Some(app) = parsed.applicants.into_iter().next() else {
        return AdmissionFetchOutcome::Unavailable;
    };
    let mut missing = app.missing_signer_setup;
    if parsed.local_node.is_validator && !parsed.local_node.admission_signer_configured {
        if let Some(issue) = parsed.local_node.admission_signer_issue {
            if !missing.contains(&issue) {
                missing.push(issue);
            }
        }
    }
    AdmissionFetchOutcome::Applicant(R7AdmissionSummary {
        certified_member: app.certified_member.unwrap_or(matches!(
            app.stage.as_str(),
            "active_voting" | "activated_awaiting_first_vote"
        )),
        registration_conflict: app.registration_conflict,
        stage: app.stage,
        stage_reason: app.stage_reason,
        l1_approvals_collected: app.l1_approvals_collected,
        l1_approvals_required: app.l1_approvals_required,
        transition_approvals_collected: app.transition_approvals_collected,
        transition_approvals_required: app.transition_approvals_required,
        l1_tx_status: app.l1_tx_status,
        l1_tx_hash: app.l1_tx_hash,
        l1_confirmed_block: app.l1_confirmed_block,
        voting_activation_height: app.voting_activation_height,
        first_finalized_vote_height: app.first_finalized_vote_height,
        missing_signer_setup: missing,
        unavailable_reason: None,
    })
}

fn merge_admission_summaries(
    local: Option<R7AdmissionSummary>,
    public: Option<R7AdmissionSummary>,
) -> Option<R7AdmissionSummary> {
    match (local, public) {
        (Some(loc), Some(pub_)) => {
            let prefer_public_stage = (pub_.certified_member && !loc.certified_member)
                || (pub_.stage == "active_voting" && loc.stage != "active_voting");
            let (stage, stage_reason) = if prefer_public_stage {
                (pub_.stage, pub_.stage_reason)
            } else {
                (loc.stage, loc.stage_reason)
            };

            let l1_tx_hash = loc.l1_tx_hash.or(pub_.l1_tx_hash);
            let l1_confirmed_block = loc.l1_confirmed_block.or(pub_.l1_confirmed_block);
            let l1_tx_status = if l1_tx_hash.is_some()
                && (loc.l1_tx_status == "confirmed_on_chain"
                    || loc.l1_tx_status == "unknown"
                    || loc.l1_tx_status.is_empty())
                && !pub_.l1_tx_status.is_empty()
                && pub_.l1_tx_status != "unknown"
            {
                pub_.l1_tx_status
            } else if loc.l1_tx_status == "unknown" || loc.l1_tx_status.is_empty() {
                pub_.l1_tx_status
            } else {
                loc.l1_tx_status
            };

            let mut missing_signer_setup = loc.missing_signer_setup;
            for item in pub_.missing_signer_setup {
                if !missing_signer_setup.contains(&item) {
                    missing_signer_setup.push(item);
                }
            }

            Some(R7AdmissionSummary {
                stage,
                stage_reason,
                certified_member: loc.certified_member || pub_.certified_member,
                registration_conflict: loc.registration_conflict || pub_.registration_conflict,
                l1_approvals_collected: loc.l1_approvals_collected.max(pub_.l1_approvals_collected),
                l1_approvals_required: loc.l1_approvals_required.max(pub_.l1_approvals_required),
                transition_approvals_collected: loc
                    .transition_approvals_collected
                    .max(pub_.transition_approvals_collected),
                transition_approvals_required: loc
                    .transition_approvals_required
                    .max(pub_.transition_approvals_required),
                l1_tx_status,
                l1_tx_hash,
                l1_confirmed_block,
                voting_activation_height: match (
                    loc.voting_activation_height,
                    pub_.voting_activation_height,
                ) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (Some(a), None) | (None, Some(a)) => Some(a),
                    (None, None) => None,
                },
                first_finalized_vote_height: match (
                    loc.first_finalized_vote_height,
                    pub_.first_finalized_vote_height,
                ) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (Some(a), None) | (None, Some(a)) => Some(a),
                    (None, None) => None,
                },
                missing_signer_setup,
                unavailable_reason: loc.unavailable_reason.and(pub_.unavailable_reason),
            })
        }
        (Some(loc), None) => Some(loc),
        (None, Some(pub_)) => Some(pub_),
        (None, None) => None,
    }
}

pub(crate) async fn fetch_r7_admission_status(
    local_port: Option<u16>,
    node_id: &str,
) -> Option<R7AdmissionSummary> {
    let local_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let public_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;

    let encoded = encode_segment(node_id);
    let public_url =
        format!("https://api.testnet.cregnet.dev/v1/public/validator/admission-status?node_id={encoded}");
    let local_fut = async {
        match local_port {
            Some(port) => {
                let local_url = format!(
                    "http://127.0.0.1:{port}/v1/public/validator/admission-status?node_id={encoded}"
                );
                fetch_admission_endpoint(&local_client, &local_url).await
            }
            None => AdmissionFetchOutcome::Unavailable,
        }
    };
    let public_fut = fetch_admission_endpoint(&public_client, &public_url);
    let (local_outcome, public_outcome) = tokio::join!(local_fut, public_fut);

    let saw_not_found = matches!(local_outcome, AdmissionFetchOutcome::NotFound)
        || matches!(public_outcome, AdmissionFetchOutcome::NotFound);
    let local_summary = match local_outcome {
        AdmissionFetchOutcome::Applicant(summary) => Some(summary),
        _ => None,
    };
    let public_summary = match public_outcome {
        AdmissionFetchOutcome::Applicant(summary) => Some(summary),
        _ => None,
    };

    if let Some(merged) = merge_admission_summaries(local_summary, public_summary) {
        return Some(merged);
    }

    saw_not_found.then(|| R7AdmissionSummary {
        stage: "rollout_pending".to_string(),
        certified_member: false,
        registration_conflict: false,
        stage_reason:
            "R7 node does not yet expose /v1/public/validator/admission-status (HTTP 404); validator fleet rollout is pending"
                .to_string(),
        l1_approvals_collected: 0,
        l1_approvals_required: 0,
        transition_approvals_collected: 0,
        transition_approvals_required: 0,
        l1_tx_status: "unknown".to_string(),
        l1_tx_hash: None,
        l1_confirmed_block: None,
        voting_activation_height: None,
        first_finalized_vote_height: None,
        missing_signer_setup: Vec::new(),
        unavailable_reason: Some(
            "Admission telemetry endpoint returned HTTP 404 (pending fleet rollout)".to_string(),
        ),
    })
}

#[derive(Deserialize)]
struct R7HealthResponse {
    status: String,
    node_id: String,
    is_validator: bool,
    sync: R7SyncResponse,
}

#[derive(Deserialize)]
struct R7SyncResponse {
    lag_blocks: u64,
    healthy: bool,
}

#[derive(Deserialize)]
struct R7StatsResponse {
    chain_id: String,
    spec_signature_verified: bool,
    tip_height: u64,
    finalized_height: u64,
    validator_set_sync_state: String,
    peer_count: u64,
}

pub(crate) async fn fetch_r7_health(
    port: u16,
    node_id: &str,
    chain_id: &str,
) -> Result<R7NodeHealth> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let base = format!("http://127.0.0.1:{port}");
    let (health, stats) = tokio::try_join!(
        client.get(format!("{base}/v1/health")).send(),
        client.get(format!("{base}/v1/public/chain/stats")).send(),
    )?;
    let (health, stats) = tokio::try_join!(
        health.error_for_status()?.json::<R7HealthResponse>(),
        stats.error_for_status()?.json::<R7StatsResponse>(),
    )?;
    anyhow::ensure!(
        health.status == "ok"
            && health.node_id == node_id
            && stats.chain_id == chain_id
            && stats.spec_signature_verified,
        "R7 health check did not confirm the expected node and signed chain"
    );
    Ok(R7NodeHealth {
        is_validator: health.is_validator,
        chain_healthy: health.sync.healthy,
        chain_lag_blocks: health.sync.lag_blocks,
        tip_height: stats.tip_height,
        finalized_height: stats.finalized_height,
        peer_count: stats.peer_count,
        sepolia_sync_state: stats.validator_set_sync_state,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEvidenceBundle {
    pub canonical: String,
    pub status_v4: Option<PackageViewV4>,
    pub witnesses: Option<WitnessBundleV1>,
    pub history: Option<IndexEvidenceHistoryV1>,
    pub warnings: Vec<String>,
}

pub fn validate_http_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("URL must not be empty");
    }
    let parsed = Url::parse(trimmed).with_context(|| format!("invalid URL '{trimmed}'"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("only http:// or https:// URLs are permitted");
    }
    if parsed.host_str().is_none() {
        bail!("URL must contain a host");
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

pub fn split_canonical(canonical: &str) -> Result<(String, String)> {
    let trimmed = canonical.trim();
    let Some((name, version)) = trimmed.rsplit_once('@') else {
        bail!("canonical package identifier must include '@version'");
    };
    if name.is_empty() || version.is_empty() {
        bail!("canonical package identifier must include non-empty name and version");
    }
    Ok((name.to_string(), version.to_string()))
}

fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
        .context("failed to build HTTP client")
}

fn encode_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(*byte));
            }
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub async fn fetch_node_overview(node_url: &str) -> Result<NodeOverview> {
    let base = validate_http_url(node_url)?;
    let client = build_client()?;
    let (health_res, stats_res) = tokio::join!(
        client.get(format!("{base}/v1/public/health")).send(),
        client.get(format!("{base}/v1/public/chain/stats")).send()
    );

    let health_ok = health_res
        .as_ref()
        .is_ok_and(|resp| resp.status().is_success());
    let stats_json: Option<serde_json::Value> = match stats_res {
        Ok(resp) if resp.status().is_success() => resp.json().await.ok(),
        Ok(resp) => {
            return Ok(NodeOverview {
                ok: false,
                chain_id: None,
                tip_height: None,
                finalized_height: None,
                active_validators: None,
                package_count: None,
                status_text: format!("HTTP {}", resp.status()),
            });
        }
        Err(err) => {
            return Ok(NodeOverview {
                ok: false,
                chain_id: None,
                tip_height: None,
                finalized_height: None,
                active_validators: None,
                package_count: None,
                status_text: format!("Unreachable: {err}"),
            });
        }
    };

    let stats = stats_json.as_ref();
    Ok(NodeOverview {
        ok: health_ok || stats.is_some(),
        chain_id: stats
            .and_then(|v| v.get("chain_id"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        tip_height: stats.and_then(|v| {
            v.get("tip_height")
                .or_else(|| v.get("current_height"))
                .and_then(|n| n.as_u64())
        }),
        finalized_height: stats.and_then(|v| v.get("finalized_height").and_then(|n| n.as_u64())),
        active_validators: stats.and_then(|v| {
            v.get("active_validators")
                .or_else(|| v.get("validator_count"))
                .and_then(|n| n.as_u64())
        }),
        package_count: stats.and_then(|v| v.get("package_count").and_then(|n| n.as_u64())),
        status_text: if health_ok {
            "online".to_string()
        } else {
            "degraded".to_string()
        },
    })
}

pub async fn fetch_quarantine_queue(
    node_url: &str,
    indexer_url: Option<&str>,
) -> Result<QuarantineQueueV1> {
    let client = build_client()?;
    if let Some(idx) = indexer_url.filter(|s| !s.trim().is_empty()) {
        let base = validate_http_url(idx)?;
        if let Ok(resp) = client
            .get(format!("{base}/v1/index/quarantine-queue?limit=100"))
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(parsed) = resp.json::<QuarantineQueueV1>().await {
                    return Ok(parsed);
                }
            }
        }
    }

    let node_base = validate_http_url(node_url)?;
    let resp = client
        .get(format!("{node_base}/v1/public/v4/quarantine?limit=100"))
        .send()
        .await
        .context("failed to query quarantine list from node")?;
    if !resp.status().is_success() {
        bail!("quarantine endpoint returned HTTP {}", resp.status());
    }
    let list: QuarantineListV1 = resp
        .json()
        .await
        .context("invalid QuarantineListV1 JSON from node")?;

    let mut entries = Vec::with_capacity(list.packages.len());
    for pkg in list.packages {
        if let registry_view::PackageStatusViewV1::Quarantined {
            height,
            expires_at_unix,
            reason_code,
            committee,
        } = pkg.status
        {
            entries.push(QuarantineQueueEntryV1 {
                canonical: pkg.canonical,
                content_hash: pkg.content_hash,
                reason_code,
                opened_height: height,
                opened_at_unix: 0,
                expires_at_unix,
                committee: committee.or(pkg.committee).unwrap_or_default(),
            });
        }
    }
    Ok(QuarantineQueueV1 {
        server_time_unix: chrono::Utc::now().timestamp(),
        entries,
        truncated: list.truncated,
    })
}

pub async fn fetch_committee_diversity(
    indexer_url: &str,
    epoch: u64,
) -> Result<CommitteeDiversityV1> {
    let base = validate_http_url(indexer_url)?;
    let client = build_client()?;
    let resp = client
        .get(format!("{base}/v1/index/committee-diversity?epoch={epoch}"))
        .send()
        .await
        .context("failed to reach indexer committee-diversity endpoint")?;
    if !resp.status().is_success() {
        bail!("committee-diversity returned HTTP {}", resp.status());
    }
    resp.json::<CommitteeDiversityV1>()
        .await
        .context("invalid CommitteeDiversityV1 payload")
}

pub async fn fetch_package_evidence(
    node_url: &str,
    indexer_url: Option<&str>,
    canonical: &str,
) -> Result<PackageEvidenceBundle> {
    let node_base = validate_http_url(node_url)?;
    let (pkg, ver) = split_canonical(canonical)?;
    let seg_pkg = encode_segment(&pkg);
    let seg_ver = encode_segment(&ver);
    let seg_canonical = encode_segment(canonical.trim());
    let client = build_client()?;

    let status_url = format!("{node_base}/v1/public/v4/packages/{seg_pkg}/{seg_ver}/status");
    let witness_url = format!("{node_base}/v1/public/v4/packages/{seg_pkg}/{seg_ver}/witnesses");

    let idx_url = indexer_url
        .filter(|s| !s.trim().is_empty())
        .map(validate_http_url)
        .transpose()?
        .map(|b| format!("{b}/v1/index/packages/{seg_canonical}/evidence-history?limit=50"));

    let (status_res, witness_res, history_res) = tokio::join!(
        client.get(&status_url).send(),
        client.get(&witness_url).send(),
        async {
            match idx_url {
                Some(ref u) => Some(client.get(u).send().await),
                None => None,
            }
        }
    );

    let mut warnings = Vec::new();
    let status_v4 = match status_res {
        Ok(resp) if resp.status().is_success() => match resp.json::<PackageViewV4>().await {
            Ok(v) => Some(v),
            Err(e) => {
                warnings.push(format!("v4 status decode error: {e}"));
                None
            }
        },
        Ok(resp) => {
            warnings.push(format!("v4 status HTTP {}", resp.status()));
            None
        }
        Err(e) => {
            warnings.push(format!("v4 status unreachable: {e}"));
            None
        }
    };

    let witnesses = match witness_res {
        Ok(resp) if resp.status().is_success() => match resp.json::<WitnessBundleV1>().await {
            Ok(w) => Some(w),
            Err(e) => {
                warnings.push(format!("witness bundle decode error: {e}"));
                None
            }
        },
        Ok(resp) => {
            warnings.push(format!("witnesses HTTP {}", resp.status()));
            None
        }
        Err(e) => {
            warnings.push(format!("witnesses unreachable: {e}"));
            None
        }
    };

    let history = match history_res {
        Some(Ok(resp)) if resp.status().is_success() => {
            match resp.json::<IndexEvidenceHistoryV1>().await {
                Ok(h) => Some(h),
                Err(e) => {
                    warnings.push(format!("evidence history decode error: {e}"));
                    None
                }
            }
        }
        Some(Ok(resp)) => {
            warnings.push(format!("evidence history HTTP {}", resp.status()));
            None
        }
        Some(Err(e)) => {
            warnings.push(format!("evidence history unreachable: {e}"));
            None
        }
        None => None,
    };

    Ok(PackageEvidenceBundle {
        canonical: canonical.trim().to_string(),
        status_v4,
        witnesses,
        history,
        warnings,
    })
}

pub fn parse_sse_v2_frame(raw_frame: &str) -> Option<(Option<String>, SseEnvelopeV2)> {
    let mut id: Option<String> = None;
    let mut data_lines = Vec::new();
    for line in raw_frame.lines() {
        if let Some(rest) = line.strip_prefix("id:") {
            id = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start());
        }
    }
    if data_lines.is_empty() {
        return None;
    }
    let joined = data_lines.join("\n");
    let envelope = serde_json::from_str::<SseEnvelopeV2>(&joined).ok()?;
    Some((id.or_else(|| Some(envelope.id.clone())), envelope))
}

pub async fn start_sse_stream(
    app: AppHandle,
    handle: &SseStreamHandle,
    stream_url: &str,
) -> Result<()> {
    let base_stream = validate_http_url(stream_url)?;
    stop_sse_stream(handle).await;

    let task = tokio::spawn(async move {
        let client = match reqwest::Client::builder().build() {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut last_event_id: Option<String> = None;
        let mut backoff_secs = 1u64;

        loop {
            let _ = app.emit("creg://sse-state", "connecting");
            let mut req = client.get(&base_stream);
            if let Some(id) = &last_event_id {
                req = req.header("Last-Event-ID", id);
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    backoff_secs = 1;
                    let _ = app.emit("creg://sse-state", "live");
                    let mut stream = resp.bytes_stream();
                    let mut buf = String::new();
                    while let Some(chunk) = stream.next().await {
                        let Ok(bytes) = chunk else {
                            break;
                        };
                        if buf.len() + bytes.len() > MAX_SSE_FRAME_BYTES {
                            let _ = app.emit("creg://sse-state", "error");
                            break;
                        }
                        buf.push_str(&String::from_utf8_lossy(&bytes).replace("\r\n", "\n"));
                        while let Some(pos) = buf.find("\n\n") {
                            let frame = buf[..pos].to_string();
                            buf.drain(..pos + 2);
                            if let Some((eid, env)) = parse_sse_v2_frame(&frame) {
                                if let Some(id) = eid {
                                    last_event_id = Some(id);
                                }
                                let _ = app.emit("creg://sse-event", env);
                            }
                        }
                    }
                }
                _ => {
                    let _ = app.emit("creg://sse-state", "error");
                }
            }
            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(30);
        }
    });

    let mut guard = handle.task.lock().await;
    *guard = Some(task);
    Ok(())
}

pub async fn stop_sse_stream(handle: &SseStreamHandle) {
    let mut guard = handle.task.lock().await;
    if let Some(prev) = guard.take() {
        prev.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_canonical_handles_scoped_npm_packages() {
        let (pkg, ver) = split_canonical("npm:@scope/pkg@1.2.3").expect("scoped parse");
        assert_eq!(pkg, "npm:@scope/pkg");
        assert_eq!(ver, "1.2.3");
        assert!(split_canonical("npm:no-version").is_err());
    }

    #[test]
    fn validate_http_url_rejects_non_http_schemes() {
        assert!(validate_http_url("http://127.0.0.1:8080/").is_ok());
        assert!(validate_http_url("https://api.testnet.cregnet.dev").is_ok());
        assert!(validate_http_url("file:///etc/passwd").is_err());
        assert!(validate_http_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn parse_sse_v2_frame_extracts_id_and_envelope() {
        let frame = "id: 12.0\nevent: package_quarantined\ndata: {\"id\":\"12.0\",\"kind\":\"package_quarantined\",\"height\":12,\"tx_index\":0,\"canonical\":\"npm:evil@1.0.0\",\"data\":{\"reason\":\"hold\"}}";
        let (id, env) = parse_sse_v2_frame(frame).expect("valid frame");
        assert_eq!(id.as_deref(), Some("12.0"));
        assert_eq!(env.height, 12);
    }

    #[test]
    fn merge_admission_summaries_fills_public_sepolia_tx_into_local_active_status() {
        let local = R7AdmissionSummary {
            stage: "active_voting".to_string(),
            certified_member: true,
            registration_conflict: true,
            stage_reason: "Certified at block #22; first finalized vote recorded at block #22"
                .to_string(),
            l1_approvals_collected: 4,
            l1_approvals_required: 4,
            transition_approvals_collected: 4,
            transition_approvals_required: 4,
            l1_tx_status: "confirmed_on_chain".to_string(),
            l1_tx_hash: None,
            l1_confirmed_block: None,
            voting_activation_height: Some(22),
            first_finalized_vote_height: Some(22),
            missing_signer_setup: vec!["Validator admission signer not configured".to_string()],
            unavailable_reason: None,
        };
        let public = R7AdmissionSummary {
            stage: "active_voting".to_string(),
            certified_member: true,
            registration_conflict: false,
            stage_reason: "Certified at block #22; first finalized vote recorded at block #22"
                .to_string(),
            l1_approvals_collected: 4,
            l1_approvals_required: 4,
            transition_approvals_collected: 4,
            transition_approvals_required: 4,
            l1_tx_status: "confirmed".to_string(),
            l1_tx_hash: Some("0xb0e823c0829dd0076cb86a2a2828d1d968b4d23c9f21aa090e71c6987c192225".to_string()),
            l1_confirmed_block: Some(12_219_746),
            voting_activation_height: Some(22),
            first_finalized_vote_height: Some(22),
            missing_signer_setup: Vec::new(),
            unavailable_reason: None,
        };

        let merged = merge_admission_summaries(Some(local), Some(public)).expect("merged");
        assert!(merged.certified_member && merged.registration_conflict);
        assert_eq!(merged.l1_tx_status, "confirmed");
        assert_eq!(
            merged.l1_tx_hash.as_deref(),
            Some("0xb0e823c0829dd0076cb86a2a2828d1d968b4d23c9f21aa090e71c6987c192225")
        );
        assert_eq!(merged.l1_confirmed_block, Some(12_219_746));
        assert_eq!(merged.missing_signer_setup.len(), 1);
    }
}
