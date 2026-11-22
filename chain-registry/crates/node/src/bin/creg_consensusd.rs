//! Zone 2 Consensus & Signing Core Daemon (`creg-consensusd`, `ADR-018` Increment 6).
//!
//! Holds validator signing identity (`CREG_VALIDATOR_PRIVKEY`, `CREG_VALIDATOR_PQ_KEY`)
//! and P2P/PBFT state, exposes ONLY an authenticated `zone-ipc` listener (never
//! a public HTTP port directly), and refuses startup if LLM API keys are present (`INV-01`).

use anyhow::Result;
use node::normalized_validator_key;
use zone_ipc::{
    encode_b64, enforce_zone_forbidden_env, ZoneEndpoint, ZoneId, ZoneIpcServer, ZoneIpcToken,
    ZoneRequest, ZoneResponse, CONSENSUS_FORBIDDEN_ENV, ZONE_IPC_TOKEN_ENV,
};

#[derive(Clone)]
pub struct ConsensusDaemonInfo {
    pub node_id: String,
    pub chain_id: String,
    pub validator_key_hint: String,
}

impl ConsensusDaemonInfo {
    pub fn from_env() -> Self {
        let node_id = std::env::var("CREG_NODE_ID").unwrap_or_else(|_| "validator-node".into());
        let chain_id = std::env::var("CREG_CHAIN_ID").unwrap_or_else(|_| "creg-testnet-1".into());
        let hint = std::env::var("CREG_VALIDATOR_ADDRESS")
            .map(|a| normalized_validator_key(&a))
            .unwrap_or_default();
        Self {
            node_id,
            chain_id,
            validator_key_hint: hint,
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

    let token = ZoneIpcToken::from_env(ZONE_IPC_TOKEN_ENV)?;
    let bind_addr =
        std::env::var("CREG_CONSENSUS_IPC_ADDR").unwrap_or_else(|_| "127.0.0.1:19002".into());
    let endpoint = ZoneEndpoint::parse(&bind_addr)?;
    let info = ConsensusDaemonInfo::from_env();
    let server = ZoneIpcServer::bind(ZoneId::Consensus, endpoint, token).await?;

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
    }
}
