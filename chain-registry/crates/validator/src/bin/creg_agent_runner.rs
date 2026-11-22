//! Zone 4 Layer 4 Agentic Orchestrator Daemon (`creg-agent-runner`, `ADR-018` Increment 6).
//!
//! Isolated from consensus signing keys and Vault tokens (`INV-01`). All kernel
//! sandbox replay tool requests (`request_sandbox_replay`) are forwarded over
//! authenticated `zone-ipc` to Zone 3 (`creg-sandboxd`) rather than executing
//! untrusted package payloads in Zone 4.

use anyhow::{bail, Context, Result};
use common::{Finding, PackageId, PackageManifest};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use validator::ai_agent::{
    AgentInvestigationInput, AgentRunner, LlmProxy, SandboxReplayExecutor, ValidatorAiConfig,
};
use validator::kernel_sandbox::{KernelBehaviorGraphV1, TriggerSpecV1};
use validator::sandbox::SandboxResult;
use zone_ipc::{
    decode_b64, encode_b64, enforce_zone_forbidden_env, ZoneEndpoint, ZoneId, ZoneIpcClient,
    ZoneIpcServer, ZoneIpcToken, ZoneRequest, ZoneResponse, AGENT_RUNNER_FORBIDDEN_ENV,
    ZONE_IPC_TOKEN_ENV,
};

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

    let token = ZoneIpcToken::from_env(ZONE_IPC_TOKEN_ENV)?;
    let bind_addr =
        std::env::var("CREG_AGENT_IPC_ADDR").unwrap_or_else(|_| "127.0.0.1:19004".into());
    let sandbox_addr =
        std::env::var("CREG_SANDBOX_IPC_ADDR").unwrap_or_else(|_| "127.0.0.1:19003".into());

    let endpoint = ZoneEndpoint::parse(&bind_addr)?;
    let sandbox_ep = ZoneEndpoint::parse(&sandbox_addr)?;

    let ai_cfg = ValidatorAiConfig::from_env_or_file()?;
    let proxy = Arc::new(LlmProxy::new_http(ai_cfg));
    let sandbox_client = ZoneIpcClient::new(
        ZoneId::AgentRunner,
        ZoneId::Sandbox,
        sandbox_ep,
        token.clone(),
    );
    let replay_exec = Arc::new(ZoneIpcReplayExecutor::new(sandbox_client));
    let runner = Arc::new(AgentRunner::with_replay_executor(proxy, replay_exec));

    let server = ZoneIpcServer::bind(ZoneId::AgentRunner, endpoint, token).await?;
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
    use std::net::SocketAddr;
    use validator::sandbox::SandboxMetrics;
    use zone_ipc::check_forbidden_env_map;

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
}
