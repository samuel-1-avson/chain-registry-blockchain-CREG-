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
}
