//! Zone 4 Layer 4 Agentic Orchestrator Daemon (`creg-agent-runner`, `ADR-018` Increment 6).
//!
//! Isolated from consensus signing keys and Vault tokens (`INV-01`). All kernel
//! sandbox replay tool requests (`request_sandbox_replay`) are forwarded over
//! authenticated `zone-ipc` to Zone 3 (`creg-sandboxd`) rather than executing
//! untrusted package payloads in Zone 4.

use ai_config::{check_ai_config, validate_and_classify_provider_url};
use anyhow::{bail, Context, Result};
use common::{ChainRecord, Finding, PackageId, PackageIntelligenceReport, PackageManifest};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use validator::ai_agent::{
    AgentInvestigationInput, AgentRunner, ChatMessage, ChatRole, LlmProxy, SandboxReplayExecutor,
    ValidatorAiConfig,
};
use validator::kernel_sandbox::{KernelBehaviorGraphV1, TriggerSpecV1};
use validator::sandbox::SandboxResult;
use zone_ipc::{
    decode_b64, encode_b64, enforce_zone_forbidden_env, ProviderProbeEntryV1,
    ProviderProbeReportV1, ZoneCapabilitiesV1, ZoneCapabilityReportV1, ZoneEndpoint, ZoneId,
    ZoneIpcClient, ZoneIpcServer, ZoneKeyring, ZoneRequest, ZoneResponse,
    AGENT_RUNNER_FORBIDDEN_ENV,
};

const CANARY_SYSTEM_PROMPT: &str = "Connectivity probe. Respond with exact JSON only: {\"tool\":\"list_files\",\"args\":{\"max_entries\":1}}";
const CANARY_USER_PROMPT: &str = "Return the exact canary tool-call JSON.";

fn strip_markdown_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(after_open) = trimmed.strip_prefix("```") {
        let after_lang = after_open
            .find('\n')
            .map(|idx| &after_open[idx + 1..])
            .unwrap_or(after_open);
        after_lang
            .trim()
            .strip_suffix("```")
            .unwrap_or(after_lang)
            .trim()
    } else {
        trimmed
    }
}

fn is_valid_canary_tool_json(raw: &str) -> bool {
    let cleaned = strip_markdown_fences(raw);
    let Ok(val) = serde_json::from_str::<serde_json::Value>(cleaned) else {
        return false;
    };
    let direct_ok = val.get("tool").and_then(|v| v.as_str()) == Some("list_files")
        && val
            .get("args")
            .and_then(|a| a.get("max_entries"))
            .and_then(|n| n.as_u64())
            == Some(1);
    let envelope_ok = val.get("action").and_then(|v| v.as_str()) == Some("tool_call")
        && val["call"]["tool"].as_str() == Some("list_files")
        && val["call"]["args"]["max_entries"].as_u64() == Some(1);
    direct_ok || envelope_ok
}

fn parse_package_canonical(raw: &str) -> Result<PackageId> {
    let trimmed = raw.trim();
    let (ecosystem, rest) = trimmed
        .split_once(':')
        .context("canonical package id missing ecosystem ':'")?;
    let (name, version) = rest
        .rsplit_once('@')
        .context("canonical package id missing '@version'")?;
    if ecosystem.is_empty() || name.is_empty() || version.is_empty() {
        bail!("invalid canonical package id '{}'", raw);
    }
    Ok(PackageId::new(ecosystem, name, version))
}

/// Replay executor forwarding `TriggerSpecV1` requests over `zone-ipc` to Zone 3.
pub struct ZoneIpcReplayExecutor {
    client: ZoneIpcClient,
    next_request_id: AtomicU64,
}

impl ZoneIpcReplayExecutor {
    pub fn new(client: ZoneIpcClient) -> Self {
        Self {
            client,
            next_request_id: AtomicU64::new(1),
        }
    }
}

impl SandboxReplayExecutor for ZoneIpcReplayExecutor {
    fn replay<'a>(
        &'a self,
        pkg_id: &'a PackageId,
        tarball_bytes: &'a [u8],
        manifest: &'a PackageManifest,
        trigger_spec: &'a TriggerSpecV1,
        timeout_secs: u64,
    ) -> Pin<Box<dyn Future<Output = Result<SandboxResult>> + Send + 'a>> {
        Box::pin(async move {
            let content_hash = hex::encode(Sha256::digest(tarball_bytes));
            let manifest_json =
                serde_json::to_string(manifest).context("serialize PackageManifest")?;
            let trigger_spec_json =
                serde_json::to_string(trigger_spec).context("serialize TriggerSpecV1")?;
            let req = ZoneRequest::RunSandboxReplay {
                package_canonical: pkg_id.canonical(),
                content_hash,
                tarball_b64: encode_b64(tarball_bytes),
                manifest_json,
                trigger_spec_json,
                timeout_secs: timeout_secs.clamp(1, 10),
            };
            let req_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
            match self.client.call(req_id, &req).await? {
                ZoneResponse::SandboxReplayResult {
                    sandbox_result_json,
                } => serde_json::from_str::<SandboxResult>(&sandbox_result_json)
                    .context("decode SandboxResult from Zone 3"),
                ZoneResponse::Error { code, message } => {
                    bail!("Zone 3 sandbox replay returned error {code}: {message}")
                }
                other => bail!("unexpected Zone 3 response: {other:?}"),
            }
        })
    }
}

/// Lane C tarball bound (matches the former in-node worker).
pub const MAX_INTELLIGENCE_TARBALL_BYTES: u64 = 512 * 1024 * 1024;
pub const INTELLIGENCE_IPFS_TIMEOUT_SECS: u64 = 300;
/// IPFS API used for Lane C fetches.
pub const INTELLIGENCE_IPFS_URL_ENV: &str = "CREG_IPFS_URL";

/// Lane C (ADR-019 `ECA-03`): fetch the committed tarball with the shared
/// bounded fetcher, refuse bytes that do not match the record, and run the
/// non-binding intelligence report here in Zone 4 instead of in consensus.
pub async fn generate_intelligence_report(
    record: &ChainRecord,
    ipfs_url: Option<&str>,
) -> Result<PackageIntelligenceReport> {
    if record.shielded {
        bail!("shielded packages are not analysed by Lane C");
    }
    let ipfs_url = ipfs_url
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .with_context(|| format!("{INTELLIGENCE_IPFS_URL_ENV} is not set for creg-agent-runner"))?;
    let tarball = validator::ipfs_fetch::fetch_ipfs_bounded(
        &record.ipfs_cid,
        ipfs_url,
        MAX_INTELLIGENCE_TARBALL_BYTES,
        INTELLIGENCE_IPFS_TIMEOUT_SECS,
    )
    .await?;
    let actual = hex::encode(Sha256::digest(&tarball));
    if !actual.eq_ignore_ascii_case(record.content_hash.trim()) {
        bail!("fetched tarball does not match the record's content hash");
    }
    Ok(validator::intelligence::generate_report(record, &tarball).await)
}

pub async fn handle_agent_request(
    sender: ZoneId,
    req: ZoneRequest,
    runner: Arc<AgentRunner>,
) -> ZoneResponse {
    match req {
        ZoneRequest::Health {} => ZoneResponse::Health {
            zone: ZoneId::AgentRunner,
            status: "ok".into(),
        },
        ZoneRequest::Capabilities {} => {
            let cfg = runner.config();
            let check = check_ai_config(cfg, None, false);
            let configured_providers = cfg.providers.keys().cloned().collect();
            let ipfs_configured = std::env::var(INTELLIGENCE_IPFS_URL_ENV)
                .ok()
                .is_some_and(|v| !v.trim().is_empty());
            let sandbox_ipc_configured = std::env::var("CREG_SANDBOX_IPC_ADDR")
                .ok()
                .is_some_and(|v| !v.trim().is_empty());
            let report = ZoneCapabilityReportV1 {
                schema_version: 1,
                zone: ZoneId::AgentRunner,
                status: "ok".into(),
                capabilities: ZoneCapabilitiesV1::AgentRunner {
                    ai_mode: format!("{:?}", cfg.agent.mode),
                    configured_providers,
                    distinct_model_families: check.distinct_model_families,
                    diversity_warning: check.diversity_advisory,
                    daily_budget_usd: cfg.agent.max_daily_api_spend_usd,
                    spent_today_usd: runner.proxy().spent_today_usd(),
                    ipfs_configured,
                    sandbox_ipc_configured,
                },
            };
            match serde_json::to_string(&report) {
                Ok(report_json) => ZoneResponse::Capabilities { report_json },
                Err(err) => ZoneResponse::Error {
                    code: 500,
                    message: format!("serialize agent capability report failed: {err}"),
                },
            }
        }
        ZoneRequest::ProbeProvider { provider } => {
            if !matches!(sender, ZoneId::Operator | ZoneId::Consensus) {
                return ZoneResponse::Error {
                    code: 403,
                    message: format!(
                        "only operator or consensus zone may probe AI providers (got '{sender}')"
                    ),
                };
            }
            let cfg = runner.config();
            let targets: Vec<String> = match provider {
                Some(ref name) => {
                    if !cfg.providers.contains_key(name) {
                        return ZoneResponse::Error {
                            code: 404,
                            message: format!("unknown AI provider '{name}'"),
                        };
                    }
                    vec![name.clone()]
                }
                None => cfg.providers.keys().cloned().collect(),
            };

            let mut entries = Vec::with_capacity(targets.len());
            for name in targets {
                let Some(profile) = cfg.providers.get(&name) else {
                    continue;
                };
                let endpoint_class = validate_and_classify_provider_url(
                    profile.resolved_base_url(),
                    profile.kind,
                    profile.allow_private_network,
                    false,
                )
                .map(|(_u, cls)| cls.as_str().to_string())
                .unwrap_or_else(|_| "unknown".to_string());
                let kind_str = serde_json::to_value(profile.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_else(|| format!("{:?}", profile.kind));
                let model_family = profile.resolved_model_family();
                let started = Instant::now();
                let res = runner
                    .proxy()
                    .complete(
                        &name,
                        CANARY_SYSTEM_PROMPT,
                        vec![ChatMessage {
                            role: ChatRole::User,
                            content: CANARY_USER_PROMPT.to_string(),
                        }],
                        64,
                    )
                    .await;
                let latency_ms = started.elapsed().as_millis() as u64;
                match res {
                    Ok(resp) => {
                        let valid_tool_json = is_valid_canary_tool_json(&resp.content);
                        let error = if valid_tool_json {
                            None
                        } else {
                            Some("canary response was not expected list_files tool JSON".into())
                        };
                        entries.push(ProviderProbeEntryV1 {
                            provider: name,
                            kind: kind_str,
                            model: profile.model.clone(),
                            model_family,
                            endpoint_class,
                            ok: valid_tool_json,
                            latency_ms,
                            valid_tool_json,
                            error,
                        });
                    }
                    Err(err) => {
                        entries.push(ProviderProbeEntryV1 {
                            provider: name,
                            kind: kind_str,
                            model: profile.model.clone(),
                            model_family,
                            endpoint_class,
                            ok: false,
                            latency_ms,
                            valid_tool_json: false,
                            error: Some(format!("{err:#}")),
                        });
                    }
                }
            }

            let report = ProviderProbeReportV1 {
                schema_version: 1,
                ok: !entries.is_empty() && entries.iter().all(|e| e.ok && e.valid_tool_json),
                providers: entries,
            };
            match serde_json::to_string(&report) {
                Ok(report_json) => ZoneResponse::ProviderProbeResult { report_json },
                Err(err) => ZoneResponse::Error {
                    code: 500,
                    message: format!("serialize ProviderProbeReportV1 failed: {err}"),
                },
            }
        }
        ZoneRequest::GenerateIntelligenceReport { record_json } => {
            if sender != ZoneId::Consensus {
                return ZoneResponse::Error {
                    code: 403,
                    message: format!(
                        "only consensus zone may request intelligence reports (got '{sender}')"
                    ),
                };
            }
            let record: ChainRecord = match serde_json::from_str(&record_json) {
                Ok(record) => record,
                Err(e) => {
                    return ZoneResponse::Error {
                        code: 400,
                        message: format!("invalid record_json: {e}"),
                    }
                }
            };
            let ipfs_url = std::env::var(INTELLIGENCE_IPFS_URL_ENV).ok();
            match generate_intelligence_report(&record, ipfs_url.as_deref()).await {
                Ok(report) => match serde_json::to_string(&report) {
                    Ok(report_json) => ZoneResponse::IntelligenceReportResult { report_json },
                    Err(e) => ZoneResponse::Error {
                        code: 500,
                        message: format!("serialize PackageIntelligenceReport failed: {e}"),
                    },
                },
                Err(e) => ZoneResponse::Error {
                    code: 502,
                    message: format!("intelligence report failed: {e:#}"),
                },
            }
        }
        ZoneRequest::RunAgentInvestigation {
            package_canonical,
            content_hash,
            tarball_b64,
            manifest_json,
            static_findings_json,
            initial_kernel_graph_json,
        } => {
            if sender != ZoneId::Consensus {
                return ZoneResponse::Error {
                    code: 403,
                    message: format!(
                        "only consensus zone may request agent investigation (got '{sender}')"
                    ),
                };
            }
            let pkg_id = match parse_package_canonical(&package_canonical) {
                Ok(id) => id,
                Err(e) => {
                    return ZoneResponse::Error {
                        code: 400,
                        message: e.to_string(),
                    }
                }
            };
            let tarball = match decode_b64(&tarball_b64) {
                Ok(b) => b,
                Err(e) => {
                    return ZoneResponse::Error {
                        code: 400,
                        message: format!("invalid tarball_b64: {e}"),
                    }
                }
            };
            let manifest: PackageManifest = match serde_json::from_str(&manifest_json) {
                Ok(m) => m,
                Err(e) => {
                    return ZoneResponse::Error {
                        code: 400,
                        message: format!("invalid manifest_json: {e}"),
                    }
                }
            };
            let findings: Vec<Finding> = match serde_json::from_str(&static_findings_json) {
                Ok(f) => f,
                Err(e) => {
                    return ZoneResponse::Error {
                        code: 400,
                        message: format!("invalid static_findings_json: {e}"),
                    }
                }
            };
            let initial_graph: Option<KernelBehaviorGraphV1> = match initial_kernel_graph_json {
                Some(raw) => match serde_json::from_str(&raw) {
                    Ok(g) => Some(g),
                    Err(e) => {
                        return ZoneResponse::Error {
                            code: 400,
                            message: format!("invalid initial_kernel_graph_json: {e}"),
                        }
                    }
                },
                None => None,
            };

            let res = runner
                .investigate(AgentInvestigationInput {
                    pkg_id: &pkg_id,
                    content_hash: &content_hash,
                    tarball: &tarball,
                    manifest: &manifest,
                    report_findings: &findings,
                    ast_index: None,
                    binary_report: None,
                    initial_kernel_graph: initial_graph.as_ref(),
                    prev_manifest: None,
                    prev_sandbox: None,
                    policy_bundle_id: "creg-policy-v1",
                })
                .await;

            match res {
                Ok(report) => match serde_json::to_string(&report) {
                    Ok(report_json) => ZoneResponse::AgentInvestigationResult { report_json },
                    Err(e) => ZoneResponse::Error {
                        code: 500,
                        message: format!("serialize AgentInvestigationReport failed: {e}"),
                    },
                },
                Err(e) => ZoneResponse::Error {
                    code: 500,
                    message: format!("agent investigation failed: {e}"),
                },
            }
        }
        _ => ZoneResponse::Error {
            code: 403,
            message: "unsupported zone request in creg-agent-runner".into(),
        },
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    enforce_zone_forbidden_env(ZoneId::AgentRunner, AGENT_RUNNER_FORBIDDEN_ENV)?;

    let keyring = ZoneKeyring::from_env_for_zone(ZoneId::AgentRunner)?;
    let sandbox_link_key = keyring.outbound_key(ZoneId::Sandbox)?;
    let bind_addr =
        std::env::var("CREG_AGENT_IPC_ADDR").unwrap_or_else(|_| "127.0.0.1:19004".into());
    let sandbox_addr =
        std::env::var("CREG_SANDBOX_IPC_ADDR").unwrap_or_else(|_| "127.0.0.1:19003".into());

    let endpoint = ZoneEndpoint::parse(&bind_addr)?;
    let sandbox_ep = ZoneEndpoint::parse(&sandbox_addr)?;

    let ai_cfg = ValidatorAiConfig::from_env_or_file()?;
    let proxy = Arc::new(LlmProxy::new_http(ai_cfg));
    let sandbox_client = ZoneIpcClient::with_link_key(
        ZoneId::AgentRunner,
        ZoneId::Sandbox,
        sandbox_ep,
        sandbox_link_key,
    );
    let replay_exec = Arc::new(ZoneIpcReplayExecutor::new(sandbox_client));
    let runner = Arc::new(AgentRunner::with_replay_executor(proxy, replay_exec));

    let server = ZoneIpcServer::bind_with_keyring(ZoneId::AgentRunner, endpoint, keyring).await?;
    tracing::info!(
        ipc = %bind_addr,
        sandbox_ipc = %sandbox_addr,
        "Zone 4 creg-agent-runner online (INV-01 enforced)"
    );
    server
        .serve(move |sender, req| {
            let runner = Arc::clone(&runner);
            Box::pin(handle_agent_request(sender, req, runner))
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chain_registry_secrets::SecretString;
    use std::collections::VecDeque;
    use std::net::SocketAddr;
    use std::sync::Mutex;
    use validator::ai_agent::{LlmRequest, LlmResponse, LlmTransport};
    use validator::sandbox::SandboxMetrics;
    use zone_ipc::{check_forbidden_env_map, ZoneIpcToken};

    #[test]
    fn inv_01_agent_runner_blocks_consensus_keys() {
        let ok = [("OPENAI_API_KEY", "sk-allowed-in-zone4")];
        assert!(
            check_forbidden_env_map(ZoneId::AgentRunner, &ok, AGENT_RUNNER_FORBIDDEN_ENV).is_ok()
        );

        for &forbidden in AGENT_RUNNER_FORBIDDEN_ENV {
            let bad = [(forbidden, "deadbeef")];
            assert!(
                check_forbidden_env_map(ZoneId::AgentRunner, &bad, AGENT_RUNNER_FORBIDDEN_ENV)
                    .is_err()
            );
        }
    }

    fn test_runner() -> Arc<AgentRunner> {
        Arc::new(AgentRunner::new(Arc::new(LlmProxy::new_http(
            ValidatorAiConfig::default(),
        ))))
    }

    fn lane_c_record(shielded: bool) -> ChainRecord {
        ChainRecord {
            id: PackageId::new("npm", "lane-c", "1.0.0"),
            content_hash: hex::encode(Sha256::digest(b"tarball")),
            ipfs_cid: "bafylanec".into(),
            shielded,
            ..ChainRecord::default()
        }
    }

    #[tokio::test]
    async fn lane_c_reports_are_consensus_only_and_fail_closed() {
        let request = ZoneRequest::GenerateIntelligenceReport {
            record_json: serde_json::to_string(&lane_c_record(false)).unwrap(),
        };
        // Only the consensus zone may ask.
        let denied = handle_agent_request(ZoneId::Gateway, request.clone(), test_runner()).await;
        assert!(matches!(denied, ZoneResponse::Error { code: 403, .. }));
        // Malformed records are refused before any fetch.
        let malformed = handle_agent_request(
            ZoneId::Consensus,
            ZoneRequest::GenerateIntelligenceReport {
                record_json: "{".into(),
            },
            test_runner(),
        )
        .await;
        assert!(matches!(malformed, ZoneResponse::Error { code: 400, .. }));

        // Without an IPFS API there is no report, and shielded ciphertext
        // is never analysed.
        let error = generate_intelligence_report(&lane_c_record(false), None)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains(INTELLIGENCE_IPFS_URL_ENV));
        let error = generate_intelligence_report(&lane_c_record(true), Some("http://127.0.0.1:1"))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("shielded"));
    }

    #[tokio::test]
    async fn zone_ipc_replay_executor_forwards_to_sandbox_zone() {
        let token = ZoneIpcToken::generate();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let server = ZoneIpcServer::bind(ZoneId::Sandbox, ZoneEndpoint::Tcp(addr), token.clone())
            .await
            .unwrap();
        let endpoint = server.local_endpoint().unwrap();

        tokio::spawn(server.serve(|sender, req| {
            Box::pin(async move {
                assert_eq!(sender, ZoneId::AgentRunner);
                match req {
                    ZoneRequest::RunSandboxReplay {
                        package_canonical,
                        timeout_secs,
                        ..
                    } => {
                        assert_eq!(package_canonical, "npm:demo-pkg@1.0.0");
                        assert_eq!(timeout_secs, 5);
                        let mock = SandboxResult {
                            findings: Vec::new(),
                            observed_network_hosts: vec!["198.51.100.77".into()],
                            observed_fs_writes: Vec::new(),
                            observed_process_spawns: vec!["/usr/bin/curl".into()],
                            kernel_graph: None,
                            metrics: SandboxMetrics {
                                engine_used: "zone3-mock-firecracker".into(),
                                wall_time_ms: 15,
                                exit_code: 0,
                                observations_count: 1,
                                findings_count: 0,
                            },
                        };
                        ZoneResponse::SandboxReplayResult {
                            sandbox_result_json: serde_json::to_string(&mock).unwrap(),
                        }
                    }
                    _ => ZoneResponse::Error {
                        code: 400,
                        message: "unexpected op".into(),
                    },
                }
            })
        }));

        let client = ZoneIpcClient::new(ZoneId::AgentRunner, ZoneId::Sandbox, endpoint, token);
        let exec = ZoneIpcReplayExecutor::new(client);
        let pkg = PackageId::new("npm", "demo-pkg", "1.0.0");
        let manifest = PackageManifest::default();
        let spec = TriggerSpecV1::default();

        let res = exec
            .replay(&pkg, b"tarball-bytes", &manifest, &spec, 5)
            .await
            .unwrap();
        assert_eq!(res.metrics.engine_used, "zone3-mock-firecracker");
        assert_eq!(res.observed_network_hosts, vec!["198.51.100.77"]);
    }

    struct MockProbeTransport {
        responses: Mutex<VecDeque<Result<String>>>,
    }

    impl LlmTransport for MockProbeTransport {
        fn complete<'a>(
            &'a self,
            request: &'a LlmRequest,
            _api_key: Option<&'a SecretString>,
        ) -> Pin<Box<dyn Future<Output = Result<LlmResponse>> + Send + 'a>> {
            Box::pin(async move {
                let next = self
                    .responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| Err(anyhow::anyhow!("no mock response")));
                let content = next?;
                Ok(LlmResponse {
                    content,
                    model: request.profile.model.clone(),
                    model_family: request.profile.resolved_model_family(),
                    input_tokens: 12,
                    output_tokens: 8,
                })
            })
        }
    }

    #[tokio::test]
    async fn agent_capabilities_and_provider_probe_behave_as_specified() {
        let single_cfg = ValidatorAiConfig::from_toml_str(
            r#"
[agent]
mode = "hybrid_escalate"

[providers.local_qwen]
kind = "ollama"
model = "qwen2.5-coder:14b"

[routing]
tier_fast = "local_qwen"
"#,
        )
        .unwrap();
        let transport = Arc::new(MockProbeTransport {
            responses: Mutex::new(VecDeque::from([
                Ok(r#"{"tool":"list_files","args":{"max_entries":1}}"#.to_string()),
                Ok("not valid json".to_string()),
            ])),
        });
        let runner = Arc::new(AgentRunner::new(Arc::new(LlmProxy::new(
            single_cfg,
            transport.clone(),
        ))));

        // Capabilities reports 1 distinct model family + diversity warning.
        let cap = handle_agent_request(
            ZoneId::Operator,
            ZoneRequest::Capabilities {},
            Arc::clone(&runner),
        )
        .await;
        match cap {
            ZoneResponse::Capabilities { report_json } => {
                let parsed: ZoneCapabilityReportV1 = serde_json::from_str(&report_json).unwrap();
                assert_eq!(parsed.zone, ZoneId::AgentRunner);
                match parsed.capabilities {
                    ZoneCapabilitiesV1::AgentRunner {
                        distinct_model_families,
                        diversity_warning,
                        configured_providers,
                        ..
                    } => {
                        assert_eq!(distinct_model_families, 1);
                        assert!(diversity_warning.is_some());
                        assert_eq!(configured_providers, vec!["local_qwen"]);
                    }
                    other => panic!("unexpected capabilities: {other:?}"),
                }
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // Gateway cannot probe AI providers.
        let denied = handle_agent_request(
            ZoneId::Gateway,
            ZoneRequest::ProbeProvider { provider: None },
            Arc::clone(&runner),
        )
        .await;
        assert!(matches!(denied, ZoneResponse::Error { code: 403, .. }));

        // Unknown provider name yields 404.
        let missing = handle_agent_request(
            ZoneId::Operator,
            ZoneRequest::ProbeProvider {
                provider: Some("nonexistent".into()),
            },
            Arc::clone(&runner),
        )
        .await;
        assert!(matches!(missing, ZoneResponse::Error { code: 404, .. }));

        // 1st probe returns valid canary tool JSON -> ok: true.
        let ok_probe = handle_agent_request(
            ZoneId::Operator,
            ZoneRequest::ProbeProvider {
                provider: Some("local_qwen".into()),
            },
            Arc::clone(&runner),
        )
        .await;
        match ok_probe {
            ZoneResponse::ProviderProbeResult { report_json } => {
                let rep: ProviderProbeReportV1 = serde_json::from_str(&report_json).unwrap();
                assert!(rep.ok);
                assert_eq!(rep.providers.len(), 1);
                assert!(rep.providers[0].ok);
                assert!(rep.providers[0].valid_tool_json);
                assert_eq!(rep.providers[0].endpoint_class, "loopback");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // 2nd probe returns malformed output -> ok: false, valid_tool_json: false.
        let bad_probe = handle_agent_request(
            ZoneId::Operator,
            ZoneRequest::ProbeProvider {
                provider: Some("local_qwen".into()),
            },
            Arc::clone(&runner),
        )
        .await;
        match bad_probe {
            ZoneResponse::ProviderProbeResult { report_json } => {
                let rep: ProviderProbeReportV1 = serde_json::from_str(&report_json).unwrap();
                assert!(!rep.ok);
                assert_eq!(rep.providers.len(), 1);
                assert!(!rep.providers[0].ok);
                assert!(!rep.providers[0].valid_tool_json);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }
}
