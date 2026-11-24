//! Zone 3 Isolated Sandbox Execution Daemon (`creg-sandboxd`, `ADR-018` Increment 6).
//!
//! Runs with ZERO consensus keys, ZERO Vault secrets, ZERO LLM API keys (`INV-01`),
//! and NO outbound network access in container deployments (`network_mode: none`).

use anyhow::{bail, Context, Result};
use common::{PackageId, PackageManifest};
use std::time::Duration;
use validator::kernel_sandbox::TriggerSpecV1;
use zone_ipc::{
    decode_b64, enforce_zone_forbidden_env, ZoneCapabilitiesV1, ZoneCapabilityReportV1,
    ZoneEndpoint, ZoneId, ZoneIpcServer, ZoneKeyring, ZoneRequest, ZoneResponse,
    SANDBOX_FORBIDDEN_ENV,
};

pub fn parse_package_canonical(raw: &str) -> Result<PackageId> {
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

pub async fn handle_sandbox_request(sender: ZoneId, req: ZoneRequest) -> ZoneResponse {
    match req {
        ZoneRequest::Health {} => ZoneResponse::Health {
            zone: ZoneId::Sandbox,
            status: "ok".into(),
        },
        ZoneRequest::Capabilities {} => {
            let status = validator::sandbox::engine_status().await;
            let mut available_engines = Vec::new();
            if validator::kernel_sandbox::detect_firecracker_ready().await {
                available_engines.push("firecracker".into());
            }
            if validator::kernel_sandbox::detect_nsjail_ready(None, None).await {
                available_engines.push("nsjail".into());
            }
            if status.dev_bypass {
                available_engines.push("dev-bypass".into());
            }
            let rootfs_configured = std::env::var("CREG_SANDBOX_ROOTFS")
                .ok()
                .is_some_and(|v| !v.trim().is_empty());
            let report = ZoneCapabilityReportV1 {
                schema_version: 1,
                zone: ZoneId::Sandbox,
                status: "ok".into(),
                capabilities: ZoneCapabilitiesV1::Sandbox {
                    selected_engine: status.engine,
                    available_engines,
                    fallback_active: !status.isolated,
                    rootfs_configured,
                    max_timeout_secs: 10,
                },
            };
            match serde_json::to_string(&report) {
                Ok(report_json) => ZoneResponse::Capabilities { report_json },
                Err(err) => ZoneResponse::Error {
                    code: 500,
                    message: format!("serialize sandbox capability report failed: {err}"),
                },
            }
        }
        ZoneRequest::RunSandboxReplay {
            package_canonical,
            content_hash: _,
            tarball_b64,
            manifest_json,
            trigger_spec_json,
            timeout_secs,
        } => {
            if !matches!(sender, ZoneId::Consensus | ZoneId::AgentRunner) {
                return ZoneResponse::Error {
                    code: 403,
                    message: format!("zone '{sender}' is not authorized to invoke sandbox replay"),
                };
            }
            let tarball = match decode_b64(&tarball_b64) {
                Ok(b) => b,
                Err(e) => {
                    return ZoneResponse::Error {
                        code: 400,
                        message: format!("invalid tarball_b64: {e}"),
                    }
                }
            };
            let pkg_id = match parse_package_canonical(&package_canonical) {
                Ok(id) => id,
                Err(e) => {
                    return ZoneResponse::Error {
                        code: 400,
                        message: e.to_string(),
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
            let trigger_spec: TriggerSpecV1 = match serde_json::from_str(&trigger_spec_json) {
                Ok(t) => t,
                Err(e) => {
                    return ZoneResponse::Error {
                        code: 400,
                        message: format!("invalid trigger_spec_json: {e}"),
                    }
                }
            };
            let capped = Duration::from_secs(timeout_secs.clamp(1, 10));
            let res = tokio::time::timeout(
                capped.saturating_add(Duration::from_secs(2)),
                validator::sandbox::run_with_trigger(&pkg_id, &tarball, &manifest, &trigger_spec),
            )
            .await;

            match res {
                Ok(Ok(outcome)) => match serde_json::to_string(&outcome) {
                    Ok(sandbox_result_json) => ZoneResponse::SandboxReplayResult {
                        sandbox_result_json,
                    },
                    Err(e) => ZoneResponse::Error {
                        code: 500,
                        message: format!("serialize SandboxResult failed: {e}"),
                    },
                },
                Ok(Err(e)) => ZoneResponse::Error {
                    code: 500,
                    message: format!("sandbox replay failed: {e}"),
                },
                Err(_) => ZoneResponse::Error {
                    code: 504,
                    message: format!("sandbox replay timed out after {}s", capped.as_secs()),
                },
            }
        }
        _ => ZoneResponse::Error {
            code: 403,
            message: "unsupported zone request in creg-sandboxd".into(),
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

    enforce_zone_forbidden_env(ZoneId::Sandbox, SANDBOX_FORBIDDEN_ENV)?;

    let keyring = ZoneKeyring::from_env_for_zone(ZoneId::Sandbox)?;
    let bind_addr =
        std::env::var("CREG_SANDBOX_IPC_ADDR").unwrap_or_else(|_| "127.0.0.1:19003".into());
    let endpoint = ZoneEndpoint::parse(&bind_addr)?;
    let server = ZoneIpcServer::bind_with_keyring(ZoneId::Sandbox, endpoint, keyring).await?;

    tracing::info!(ipc = %bind_addr, "Zone 3 creg-sandboxd online (INV-01 enforced)");
    server
        .serve(|sender, req| Box::pin(handle_sandbox_request(sender, req)))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use zone_ipc::check_forbidden_env_map;

    #[test]
    fn parse_package_canonical_handles_scoped_and_invalid() {
        let scoped = parse_package_canonical("npm:@scope/pkg@1.2.3").unwrap();
        assert_eq!(scoped.ecosystem, "npm");
        assert_eq!(scoped.name, "@scope/pkg");
        assert_eq!(scoped.version, "1.2.3");

        let pypi = parse_package_canonical("pypi:requests@2.31.0").unwrap();
        assert_eq!(pypi.canonical(), "pypi:requests@2.31.0");

        assert!(parse_package_canonical("invalid").is_err());
        assert!(parse_package_canonical("npm:@scope/").is_err());
    }

    #[test]
    fn inv_01_sandbox_guard_blocks_all_forbidden_keys() {
        let clean = [("CREG_SANDBOX_ENGINE", "nsjail")];
        assert!(check_forbidden_env_map(ZoneId::Sandbox, &clean, SANDBOX_FORBIDDEN_ENV).is_ok());

        for &key in SANDBOX_FORBIDDEN_ENV {
            let poisoned = [(key, "secret-value")];
            assert!(
                check_forbidden_env_map(ZoneId::Sandbox, &poisoned, SANDBOX_FORBIDDEN_ENV).is_err()
            );
        }
    }

    #[tokio::test]
    async fn sandbox_dispatch_blocks_gateway_and_answers_health() {
        let ok = handle_sandbox_request(ZoneId::AgentRunner, ZoneRequest::Health {}).await;
        assert_eq!(
            ok,
            ZoneResponse::Health {
                zone: ZoneId::Sandbox,
                status: "ok".into(),
            }
        );

        let denied = handle_sandbox_request(
            ZoneId::Gateway,
            ZoneRequest::RunSandboxReplay {
                package_canonical: "npm:foo@1.0.0".into(),
                content_hash: "00".repeat(32),
                tarball_b64: String::new(),
                manifest_json: "{}".into(),
                trigger_spec_json: "{}".into(),
                timeout_secs: 5,
            },
        )
        .await;
        assert!(matches!(denied, ZoneResponse::Error { code: 403, .. }));

        let cap = handle_sandbox_request(ZoneId::Operator, ZoneRequest::Capabilities {}).await;
        match cap {
            ZoneResponse::Capabilities { report_json } => {
                let parsed: ZoneCapabilityReportV1 = serde_json::from_str(&report_json).unwrap();
                assert_eq!(parsed.zone, ZoneId::Sandbox);
                assert!(matches!(
                    parsed.capabilities,
                    ZoneCapabilitiesV1::Sandbox {
                        max_timeout_secs: 10,
                        ..
                    }
                ));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }
}
