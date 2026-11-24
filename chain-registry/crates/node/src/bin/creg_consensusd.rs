//! Zone 2 Consensus & Signing Core Daemon (`creg-consensusd`, `ADR-018` Increment 6).
//!
//! Holds validator signing identity (`CREG_VALIDATOR_KEY`, `CREG_VALIDATOR_PQ_KEY`)
//! and P2P/PBFT state, exposes ONLY an authenticated `zone-ipc` listener (never
//! a public HTTP port directly), and refuses startup if LLM API keys are present (`INV-01`).

use anyhow::Result;
use common::ChainSpec;
use node::normalized_validator_key;
use std::path::Path;
use zone_ipc::{
    encode_b64, enforce_zone_forbidden_env, ZoneCapabilitiesV1, ZoneCapabilityReportV1,
    ZoneEndpoint, ZoneId, ZoneIpcServer, ZoneKeyring, ZoneRequest, ZoneResponse,
    CONSENSUS_FORBIDDEN_ENV,
};

#[derive(Clone)]
pub struct ConsensusDaemonInfo {
    pub node_id: String,
    pub chain_id: String,
    pub validator_key_hint: String,
    pub tip_height: u64,
    pub hybrid_active: bool,
    pub signing_backend: String,
    pub active_validators: usize,
}

impl ConsensusDaemonInfo {
    pub fn from_env() -> Self {
        let node_id = std::env::var("CREG_NODE_ID").unwrap_or_else(|_| "validator-node".into());
        let env_chain_id = std::env::var("CREG_CHAIN_ID")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let hint = std::env::var("CREG_VALIDATOR_ADDRESS")
            .map(|a| normalized_validator_key(&a))
            .unwrap_or_default();
        let tip_height = std::env::var("CREG_TIP_HEIGHT")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let signing_backend = std::env::var("CREG_SECRETS_BACKEND")
            .ok()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "env".into());

        let spec = std::env::var("CREG_CHAIN_SPEC")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && Path::new(s).is_file())
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str::<ChainSpec>(&raw).ok());

        let (chain_id, hybrid_active, active_validators) = match spec {
            Some(s) => (
                env_chain_id.unwrap_or_else(|| s.chain_id.clone()),
                s.consensus_params.vote_v4_active_at(tip_height),
                s.validator_set
                    .validators
                    .iter()
                    .filter(|v| v.status.eq_ignore_ascii_case("active"))
                    .count(),
            ),
            None => (
                env_chain_id.unwrap_or_else(|| "creg-testnet-1".into()),
                false,
                0,
            ),
        };

        Self {
            node_id,
            chain_id,
            validator_key_hint: hint,
            tip_height,
            hybrid_active,
            signing_backend,
            active_validators,
        }
    }
}

pub async fn handle_consensus_request(
    sender: ZoneId,
    req: ZoneRequest,
    info: &ConsensusDaemonInfo,
) -> ZoneResponse {
    match req {
        ZoneRequest::Health {} => ZoneResponse::Health {
            zone: ZoneId::Consensus,
            status: "ok".into(),
        },
        ZoneRequest::Capabilities {} => {
            let report = ZoneCapabilityReportV1 {
                schema_version: 1,
                zone: ZoneId::Consensus,
                status: "ok".into(),
                capabilities: ZoneCapabilitiesV1::Consensus {
                    node_id: info.node_id.clone(),
                    chain_id: info.chain_id.clone(),
                    tip_height: info.tip_height,
                    hybrid_active: info.hybrid_active,
                    signing_backend: info.signing_backend.clone(),
                    active_validators: info.active_validators,
                },
            };
            match serde_json::to_string(&report) {
                Ok(report_json) => ZoneResponse::Capabilities { report_json },
                Err(err) => ZoneResponse::Error {
                    code: 500,
                    message: format!("serialize consensus capability report failed: {err}"),
                },
            }
        }
        ZoneRequest::ForwardPublicHttp { method, path, .. } => {
            if sender != ZoneId::Gateway {
                return ZoneResponse::Error {
                    code: 403,
                    message: format!(
                        "public HTTP forwarding restricted to gateway (got '{sender}')"
                    ),
                };
            }
            let payload = serde_json::json!({
                "status": "ok",
                "zone": "consensus",
                "node_id": info.node_id,
                "chain_id": info.chain_id,
                "method": method,
                "path": path,
            })
            .to_string();
            ZoneResponse::HttpReply {
                status: 200,
                content_type: "application/json".into(),
                body_b64: encode_b64(payload.as_bytes()),
            }
        }
        ZoneRequest::ForwardAdminHttp { method, path, .. } => {
            if sender != ZoneId::Gateway {
                return ZoneResponse::Error {
                    code: 403,
                    message: format!(
                        "admin HTTP forwarding restricted to gateway (got '{sender}')"
                    ),
                };
            }
            if path == "/metrics" {
                let metrics = format!(
                    "# TYPE creg_consensus_zone_up gauge\ncreg_consensus_zone_up{{node_id=\"{}\"}} 1\n",
                    info.node_id
                );
                return ZoneResponse::HttpReply {
                    status: 200,
                    content_type: "text/plain; version=0.0.4".into(),
                    body_b64: encode_b64(metrics.as_bytes()),
                };
            }
            let payload = serde_json::json!({
                "status": "ok",
                "channel": "admin",
                "node_id": info.node_id,
                "validator_key_hint": info.validator_key_hint,
                "method": method,
                "path": path,
            })
            .to_string();
            ZoneResponse::HttpReply {
                status: 200,
                content_type: "application/json".into(),
                body_b64: encode_b64(payload.as_bytes()),
            }
        }
        _ => ZoneResponse::Error {
            code: 403,
            message: "unsupported zone request in creg-consensusd".into(),
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

    enforce_zone_forbidden_env(ZoneId::Consensus, CONSENSUS_FORBIDDEN_ENV)?;

    let keyring = ZoneKeyring::from_env_for_zone(ZoneId::Consensus)?;
    let bind_addr =
        std::env::var("CREG_CONSENSUS_IPC_ADDR").unwrap_or_else(|_| "127.0.0.1:19002".into());
    let endpoint = ZoneEndpoint::parse(&bind_addr)?;
    let info = ConsensusDaemonInfo::from_env();
    let server = ZoneIpcServer::bind_with_keyring(ZoneId::Consensus, endpoint, keyring).await?;

    tracing::info!(
        ipc = %bind_addr,
        node_id = %info.node_id,
        "Zone 2 creg-consensusd online (INV-01 enforced)"
    );
    server
        .serve(move |sender, req| {
            let info = info.clone();
            Box::pin(async move { handle_consensus_request(sender, req, &info).await })
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use zone_ipc::{check_forbidden_env_map, decode_b64};

    #[test]
    fn inv_01_consensusd_blocks_llm_keys() {
        let valid = [("CREG_VALIDATOR_PRIVKEY", "aa".repeat(32))];
        assert!(check_forbidden_env_map(
            ZoneId::Consensus,
            &[(valid[0].0, valid[0].1.as_str())],
            CONSENSUS_FORBIDDEN_ENV
        )
        .is_ok());

        for &llm_var in CONSENSUS_FORBIDDEN_ENV {
            let bad = [(llm_var, "sk-forbidden")];
            assert!(
                check_forbidden_env_map(ZoneId::Consensus, &bad, CONSENSUS_FORBIDDEN_ENV).is_err()
            );
        }
    }

    #[tokio::test]
    async fn consensus_ipc_dispatch_routes_public_and_admin() {
        let info = ConsensusDaemonInfo {
            node_id: "val-01".into(),
            chain_id: "creg-testnet-1".into(),
            validator_key_hint: "0x1234".into(),
            tip_height: 42,
            hybrid_active: false,
            signing_backend: "vault".into(),
            active_validators: 4,
        };

        let pub_resp = handle_consensus_request(
            ZoneId::Gateway,
            ZoneRequest::ForwardPublicHttp {
                method: "GET".into(),
                path: "/v1/public/health".into(),
                query: None,
                body_b64: String::new(),
            },
            &info,
        )
        .await;
        match pub_resp {
            ZoneResponse::HttpReply {
                status, body_b64, ..
            } => {
                assert_eq!(status, 200);
                let json: serde_json::Value =
                    serde_json::from_slice(&decode_b64(&body_b64).unwrap()).unwrap();
                assert_eq!(json["node_id"], "val-01");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        let denied = handle_consensus_request(
            ZoneId::Sandbox,
            ZoneRequest::ForwardAdminHttp {
                method: "GET".into(),
                path: "/metrics".into(),
                query: None,
                body_b64: String::new(),
            },
            &info,
        )
        .await;
        assert!(matches!(denied, ZoneResponse::Error { code: 403, .. }));

        let op_denied = handle_consensus_request(
            ZoneId::Operator,
            ZoneRequest::ForwardPublicHttp {
                method: "GET".into(),
                path: "/v1/public/health".into(),
                query: None,
                body_b64: String::new(),
            },
            &info,
        )
        .await;
        assert!(matches!(op_denied, ZoneResponse::Error { code: 403, .. }));

        let cap =
            handle_consensus_request(ZoneId::Operator, ZoneRequest::Capabilities {}, &info).await;
        match cap {
            ZoneResponse::Capabilities { report_json } => {
                let parsed: ZoneCapabilityReportV1 = serde_json::from_str(&report_json).unwrap();
                assert_eq!(parsed.zone, ZoneId::Consensus);
                assert!(matches!(
                    parsed.capabilities,
                    ZoneCapabilitiesV1::Consensus {
                        tip_height: 42,
                        active_validators: 4,
                        ..
                    }
                ));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }
}
