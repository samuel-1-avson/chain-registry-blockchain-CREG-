// crates/cli/src/validator_ops.rs
// Operator diagnostics across the four validator zones (`creg validator doctor`,
// ADR-019 §4.3, ECA-04).

use crate::ai_ops;
use ai_config::AiConfigCheckReportV1;
use anyhow::{bail, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use zone_ipc::{
    OperatorIpcKeyBundleV1, ZoneCapabilitiesV1, ZoneCapabilityReportV1, ZoneId, ZoneRequest,
    ZoneResponse,
};

const ZONE_IPC_TIMEOUT_SECS: u64 = 5;
const DOCTOR_IPC_REQUEST_ID: u64 = 1;

pub const VALIDATOR_ZONES_ORDER: [ZoneId; 4] = [
    ZoneId::Gateway,
    ZoneId::Consensus,
    ZoneId::Sandbox,
    ZoneId::AgentRunner,
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidatorZoneProbeV1 {
    pub zone: ZoneId,
    pub endpoint: String,
    pub reachable: bool,
    pub healthy: bool,
    pub status: Option<String>,
    pub capabilities: Option<ZoneCapabilityReportV1>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidatorDoctorReportV1 {
    pub schema_version: u16,
    pub ok: bool,
    pub operator_key_file: Option<String>,
    pub zones: Vec<ValidatorZoneProbeV1>,
    pub ai_config: Option<AiConfigCheckReportV1>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

fn zone_capabilities_match_target(zone: ZoneId, caps: &ZoneCapabilitiesV1) -> bool {
    matches!(
        (zone, caps),
        (ZoneId::Gateway, ZoneCapabilitiesV1::Gateway { .. })
            | (ZoneId::Consensus, ZoneCapabilitiesV1::Consensus { .. })
            | (ZoneId::Sandbox, ZoneCapabilitiesV1::Sandbox { .. })
            | (ZoneId::AgentRunner, ZoneCapabilitiesV1::AgentRunner { .. })
    )
}

async fn probe_single_zone(bundle: &OperatorIpcKeyBundleV1, zone: ZoneId) -> ValidatorZoneProbeV1 {
    let endpoint = bundle
        .endpoint_and_key(zone)
        .map(|entry| entry.endpoint.clone())
        .unwrap_or_default();

    let client = match bundle.client_for(zone) {
        Ok(client) => client,
        Err(err) => {
            return ValidatorZoneProbeV1 {
                zone,
                endpoint,
                reachable: false,
                healthy: false,
                status: None,
                capabilities: None,
                error: Some(format!("{err:#}")),
            };
        }
    };

    let call_future = client.call(DOCTOR_IPC_REQUEST_ID, &ZoneRequest::Capabilities {});
    let response =
        match tokio::time::timeout(Duration::from_secs(ZONE_IPC_TIMEOUT_SECS), call_future).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(err)) => {
                return ValidatorZoneProbeV1 {
                    zone,
                    endpoint,
                    reachable: false,
                    healthy: false,
                    status: None,
                    capabilities: None,
                    error: Some(format!("{err:#}")),
                };
            }
            Err(_) => {
                return ValidatorZoneProbeV1 {
                    zone,
                    endpoint,
                    reachable: false,
                    healthy: false,
                    status: None,
                    capabilities: None,
                    error: Some(format!(
                        "IPC request timed out after {ZONE_IPC_TIMEOUT_SECS}s"
                    )),
                };
            }
        };

    match response {
        ZoneResponse::Capabilities { report_json } => {
            match serde_json::from_str::<ZoneCapabilityReportV1>(&report_json) {
                Ok(report) => {
                    if report.zone != zone {
                        return ValidatorZoneProbeV1 {
                            zone,
                            endpoint,
                            reachable: true,
                            healthy: false,
                            status: Some(report.status.clone()),
                            capabilities: Some(report.clone()),
                            error: Some(format!(
                                "capability zone mismatch: expected {}, got {}",
                                zone, report.zone
                            )),
                        };
                    }
                    if !zone_capabilities_match_target(zone, &report.capabilities) {
                        return ValidatorZoneProbeV1 {
                            zone,
                            endpoint,
                            reachable: true,
                            healthy: false,
                            status: Some(report.status.clone()),
                            capabilities: Some(report.clone()),
                            error: Some(format!(
                                "capability payload kind does not match zone {}",
                                zone
                            )),
                        };
                    }
                    let normalized = report.status.trim().to_ascii_lowercase();
                    let healthy = matches!(normalized.as_str(), "ok" | "healthy" | "ready");
                    let error = if healthy {
                        None
                    } else {
                        Some(format!(
                            "zone {} reported non-healthy status '{}'",
                            zone, report.status
                        ))
                    };
                    ValidatorZoneProbeV1 {
                        zone,
                        endpoint,
                        reachable: true,
                        healthy,
                        status: Some(report.status.clone()),
                        capabilities: Some(report),
                        error,
                    }
                }
                Err(err) => ValidatorZoneProbeV1 {
                    zone,
                    endpoint,
                    reachable: true,
                    healthy: false,
                    status: None,
                    capabilities: None,
                    error: Some(format!("decode ZoneCapabilityReportV1 failed: {err}")),
                },
            }
        }
        ZoneResponse::Error { code, message } => ValidatorZoneProbeV1 {
            zone,
            endpoint,
            reachable: true,
            healthy: false,
            status: Some("error".to_string()),
            capabilities: None,
            error: Some(format!("zone error {code}: {message}")),
        },
        other => ValidatorZoneProbeV1 {
            zone,
            endpoint,
            reachable: true,
            healthy: false,
            status: None,
            capabilities: None,
            error: Some(format!("unexpected ZoneResponse variant: {other:?}")),
        },
    }
}

/// Probe all four validator zones in fixed order (`gateway`, `consensus`,
/// `sandbox`, `agent_runner`) over the read-only `ZoneId::Operator` IPC link
/// without aborting when an individual zone is unreachable.
pub async fn collect_validator_doctor_report(
    bundle_path: &Path,
    ai_config_path: Option<&Path>,
) -> ValidatorDoctorReportV1 {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let zones = match OperatorIpcKeyBundleV1::read_owner_only(bundle_path) {
        Ok(bundle) => {
            let mut results = Vec::with_capacity(VALIDATOR_ZONES_ORDER.len());
            for zone in VALIDATOR_ZONES_ORDER {
                let probe = probe_single_zone(&bundle, zone).await;
                if let Some(err) = &probe.error {
                    errors.push(format!("zone {}: {err}", probe.zone));
                }
                if let Some(ZoneCapabilityReportV1 {
                    capabilities:
                        ZoneCapabilitiesV1::AgentRunner {
                            diversity_warning: Some(warn),
                            ..
                        },
                    ..
                }) = &probe.capabilities
                {
                    warnings.push(warn.clone());
                }
                if let Some(ZoneCapabilityReportV1 {
                    capabilities:
                        ZoneCapabilitiesV1::Sandbox {
                            fallback_active: true,
                            selected_engine,
                            ..
                        },
                    ..
                }) = &probe.capabilities
                {
                    warnings.push(format!(
                        "sandbox zone is running fallback engine '{selected_engine}'"
                    ));
                }
                results.push(probe);
            }
            results
        }
        Err(err) => {
            let msg = format!(
                "load operator IPC key bundle '{}': {err:#}",
                bundle_path.display()
            );
            errors.push(msg.clone());
            VALIDATOR_ZONES_ORDER
                .iter()
                .map(|&zone| ValidatorZoneProbeV1 {
                    zone,
                    endpoint: String::new(),
                    reachable: false,
                    healthy: false,
                    status: None,
                    capabilities: None,
                    error: Some(msg.clone()),
                })
                .collect()
        }
    };

    let ai_config = match ai_ops::check_config_report(ai_config_path) {
        Ok(report) => {
            for warn in &report.warnings {
                if !warnings.contains(warn) {
                    warnings.push(warn.clone());
                }
            }
            if !report.ok {
                errors.push("validator AI configuration check failed".to_string());
            }
            Some(report)
        }
        Err(err) => {
            if ai_config_path.is_some()
                || std::env::var("CREG_AI_CONFIG")
                    .ok()
                    .is_some_and(|v| !v.trim().is_empty())
            {
                errors.push(format!("validator AI configuration invalid: {err:#}"));
            } else {
                warnings.push(format!("AI config inspection skipped: {err:#}"));
            }
            None
        }
    };

    let all_zones_healthy = !zones.is_empty() && zones.iter().all(|z| z.reachable && z.healthy);
    let ai_ok = ai_config.as_ref().is_none_or(|c| c.ok);
    let ok = all_zones_healthy && ai_ok && errors.is_empty();

    ValidatorDoctorReportV1 {
        schema_version: 1,
        ok,
        operator_key_file: Some(bundle_path.display().to_string()),
        zones,
        ai_config,
        errors,
        warnings,
    }
}

fn format_zone_capabilities_summary(caps: &ZoneCapabilitiesV1) -> String {
    match caps {
        ZoneCapabilitiesV1::Gateway {
            public_listen,
            admin_listen,
            ipc_listen,
            rate_limit_max,
            rate_limit_window_secs,
            admin_token_configured,
            consensus_ipc_reachable,
        } => format!(
            "public={public_listen} admin={admin_listen} ipc={ipc_listen} rate_limit={rate_limit_max}/{rate_limit_window_secs}s admin_token={admin_token_configured} consensus_ipc={consensus_ipc_reachable}"
        ),
        ZoneCapabilitiesV1::Consensus {
            node_id,
            chain_id,
            tip_height,
            hybrid_active,
            signing_backend,
            active_validators,
        } => format!(
            "node_id={node_id} chain_id={chain_id} tip={tip_height} hybrid_active={hybrid_active} signing={signing_backend} validators={active_validators}"
        ),
        ZoneCapabilitiesV1::Sandbox {
            selected_engine,
            available_engines,
            fallback_active,
            rootfs_configured,
            max_timeout_secs,
        } => format!(
            "engine={selected_engine} available=[{}] fallback={fallback_active} rootfs={rootfs_configured} max_timeout={max_timeout_secs}s",
            available_engines.join(",")
        ),
        ZoneCapabilitiesV1::AgentRunner {
            ai_mode,
            configured_providers,
            distinct_model_families,
            diversity_warning: _,
            daily_budget_usd,
            spent_today_usd,
            ipfs_configured,
            sandbox_ipc_configured,
        } => format!(
            "ai_mode={ai_mode} providers=[{}] families={distinct_model_families} spend=${spent_today_usd:.2}/${daily_budget_usd:.2} ipfs={ipfs_configured} sandbox_ipc={sandbox_ipc_configured}",
            configured_providers.join(",")
        ),
    }
}

pub fn format_doctor_report_human(report: &ValidatorDoctorReportV1, show_details: bool) -> String {
    let mut out = String::new();
    let header = if report.ok {
        "CREG Validator Doctor: PASS".green().bold().to_string()
    } else {
        "CREG Validator Doctor: FAIL".red().bold().to_string()
    };
    out.push_str(&format!("{header}\n"));
    if let Some(key_file) = &report.operator_key_file {
        out.push_str(&format!("  operator_key_file: {key_file}\n"));
    }
    out.push_str("  zones:\n");
    for z in &report.zones {
        let state = if z.reachable && z.healthy {
            "PASS".green().to_string()
        } else {
            "FAIL".red().to_string()
        };
        let ep = if z.endpoint.is_empty() {
            "<unconfigured>"
        } else {
            z.endpoint.as_str()
        };
        out.push_str(&format!(
            "    - {:<12} [{}] endpoint={} status={}\n",
            z.zone.as_str(),
            state,
            ep,
            z.status.as_deref().unwrap_or("unreachable")
        ));
        if let Some(cap_report) = &z.capabilities {
            if show_details || !(z.reachable && z.healthy) {
                out.push_str(&format!(
                    "        capabilities: {}\n",
                    format_zone_capabilities_summary(&cap_report.capabilities)
                ));
            }
        }
        if let Some(err) = &z.error {
            out.push_str(&format!("        error: {err}\n"));
        }
    }
    if let Some(ai) = &report.ai_config {
        out.push_str(&format!(
            "  ai_config: ok={} mode={} families={}\n",
            ai.ok,
            ai_ops::agent_mode_label(ai.mode),
            ai.distinct_model_families
        ));
    }
    for warn in &report.warnings {
        out.push_str(&format!("  {} {warn}\n", "warn:".yellow()));
    }
    for err in &report.errors {
        out.push_str(&format!("  {} {err}\n", "error:".red()));
    }
    out
}

pub async fn run_doctor(
    operator_key_file: Option<&Path>,
    config_path: Option<&Path>,
    zones: bool,
    json: bool,
) -> Result<()> {
    let bundle_path = ai_ops::resolve_operator_key_file(operator_key_file);
    let report = collect_validator_doctor_report(&bundle_path, config_path).await;
    if json {
        let encoded =
            serde_json::to_string_pretty(&report).context("serialize ValidatorDoctorReportV1")?;
        println!("{encoded}");
    } else {
        print!("{}", format_doctor_report_human(&report, zones));
    }
    if !report.ok {
        bail!("validator doctor reported one or more unhealthy or unreachable checks");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::net::TcpListener;
    use tempfile::TempDir;
    use zone_ipc::{
        OperatorZoneEndpointKeyV1, ZoneCapabilitiesV1, ZoneCapabilityReportV1, ZoneEndpoint,
        ZoneIpcServer, ZoneIpcToken,
    };

    fn unused_loopback_endpoint() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("tcp://{addr}")
    }

    fn make_mock_capabilities(zone: ZoneId) -> ZoneCapabilitiesV1 {
        match zone {
            ZoneId::Gateway => ZoneCapabilitiesV1::Gateway {
                public_listen: "0.0.0.0:8080".to_string(),
                admin_listen: "127.0.0.1:18080".to_string(),
                ipc_listen: "127.0.0.1:19001".to_string(),
                rate_limit_max: 120,
                rate_limit_window_secs: 60,
                admin_token_configured: true,
                consensus_ipc_reachable: true,
            },
            ZoneId::Consensus => ZoneCapabilitiesV1::Consensus {
                node_id: "validator-01".to_string(),
                chain_id: "creg-testnet-1".to_string(),
                tip_height: 42,
                hybrid_active: false,
                signing_backend: "env".to_string(),
                active_validators: 7,
            },
            ZoneId::Sandbox => ZoneCapabilitiesV1::Sandbox {
                selected_engine: "nsjail".to_string(),
                available_engines: vec!["nsjail".to_string(), "wasmtime".to_string()],
                fallback_active: false,
                rootfs_configured: true,
                max_timeout_secs: 30,
            },
            ZoneId::AgentRunner => ZoneCapabilitiesV1::AgentRunner {
                ai_mode: "hybrid_escalate".to_string(),
                configured_providers: vec!["fast".to_string(), "deep".to_string()],
                distinct_model_families: 2,
                diversity_warning: None,
                daily_budget_usd: 25.0,
                spent_today_usd: 1.5,
                ipfs_configured: true,
                sandbox_ipc_configured: true,
            },
            ZoneId::Operator => unreachable!("operator is not a target zone"),
        }
    }

    async fn spawn_mock_zone(
        zone: ZoneId,
        master: &ZoneIpcToken,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let server = ZoneIpcServer::bind(
            zone,
            ZoneEndpoint::Tcp("127.0.0.1:0".parse().unwrap()),
            master.clone(),
        )
        .await
        .unwrap();
        let endpoint = match server.local_endpoint().unwrap() {
            ZoneEndpoint::Tcp(addr) => format!("tcp://{addr}"),
            #[cfg(unix)]
            ZoneEndpoint::Unix(p) => format!("unix://{}", p.display()),
        };
        let handle = tokio::spawn(async move {
            let _ = server
                .serve(move |sender, req| {
                    Box::pin(async move {
                        assert_eq!(sender, ZoneId::Operator);
                        match req {
                            ZoneRequest::Capabilities {} => {
                                let report = ZoneCapabilityReportV1 {
                                    schema_version: 1,
                                    zone,
                                    status: "ok".to_string(),
                                    capabilities: make_mock_capabilities(zone),
                                };
                                ZoneResponse::Capabilities {
                                    report_json: serde_json::to_string(&report).unwrap(),
                                }
                            }
                            other => ZoneResponse::Error {
                                code: 400,
                                message: format!("unexpected op: {other:?}"),
                            },
                        }
                    })
                })
                .await;
        });
        (endpoint, handle)
    }

    fn write_sample_ai_config(dir: &Path) -> std::path::PathBuf {
        let key_path = dir.join("api-key.txt");
        let mut f = std::fs::File::create(&key_path).unwrap();
        writeln!(f, "test-dummy-token").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let cfg_path = dir.join("validator-ai.toml");
        std::fs::write(
            &cfg_path,
            format!(
                "[agent]\n\
                 mode = \"hybrid_escalate\"\n\
                 shadow_mode = true\n\
                 \n\
                 [providers.fast]\n\
                 kind = \"ollama\"\n\
                 model = \"qwen2.5-coder:7b\"\n\
                 model_family = \"qwen\"\n\
                 \n\
                 [providers.deep]\n\
                 kind = \"gemini\"\n\
                 model = \"gemini-2.5-pro\"\n\
                 model_family = \"gemini\"\n\
                 api_key_secret_ref = 'file:{}'\n\
                 \n\
                 [routing]\n\
                 tier_fast = \"fast\"\n\
                 tier_deep = \"deep\"\n",
                key_path.display()
            ),
        )
        .unwrap();
        cfg_path
    }

    #[tokio::test]
    async fn validator_doctor_probes_all_four_zones_and_matches_json_schema_snapshot() {
        let dir = TempDir::new().unwrap();
        let master = ZoneIpcToken::generate();

        let (gw_ep, gw_h) = spawn_mock_zone(ZoneId::Gateway, &master).await;
        let (co_ep, co_h) = spawn_mock_zone(ZoneId::Consensus, &master).await;
        let (sb_ep, sb_h) = spawn_mock_zone(ZoneId::Sandbox, &master).await;
        let (ag_ep, ag_h) = spawn_mock_zone(ZoneId::AgentRunner, &master).await;

        let bundle = OperatorIpcKeyBundleV1 {
            schema_version: 1,
            gateway: OperatorZoneEndpointKeyV1 {
                endpoint: gw_ep,
                link_key_hex: master
                    .derive_link_key(ZoneId::Operator, ZoneId::Gateway)
                    .to_hex(),
            },
            consensus: OperatorZoneEndpointKeyV1 {
                endpoint: co_ep,
                link_key_hex: master
                    .derive_link_key(ZoneId::Operator, ZoneId::Consensus)
                    .to_hex(),
            },
            sandbox: OperatorZoneEndpointKeyV1 {
                endpoint: sb_ep,
                link_key_hex: master
                    .derive_link_key(ZoneId::Operator, ZoneId::Sandbox)
                    .to_hex(),
            },
            agent_runner: OperatorZoneEndpointKeyV1 {
                endpoint: ag_ep,
                link_key_hex: master
                    .derive_link_key(ZoneId::Operator, ZoneId::AgentRunner)
                    .to_hex(),
            },
        };
        let bundle_path = dir.path().join("operator-ipc-keys.json");
        bundle.write_owner_only(&bundle_path).unwrap();
        let cfg_path = write_sample_ai_config(dir.path());

        let report = collect_validator_doctor_report(&bundle_path, Some(&cfg_path)).await;
        assert!(report.ok);
        assert!(report.errors.is_empty());
        assert_eq!(report.zones.len(), 4);

        let value: serde_json::Value = serde_json::to_value(&report).unwrap();
        let obj = value.as_object().unwrap();
        let mut top_keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        top_keys.sort_unstable();
        assert_eq!(
            top_keys,
            vec![
                "ai_config",
                "errors",
                "ok",
                "operator_key_file",
                "schema_version",
                "warnings",
                "zones"
            ]
        );
        assert_eq!(obj["schema_version"], 1);
        assert_eq!(obj["ok"], true);
        assert_eq!(obj["ai_config"]["distinct_model_families"], 2);
        assert_eq!(obj["ai_config"]["ok"], true);

        let zones_arr = obj["zones"].as_array().unwrap();
        let expected_zones = [
            ("gateway", "gateway"),
            ("consensus", "consensus"),
            ("sandbox", "sandbox"),
            ("agent_runner", "agent_runner"),
        ];
        for (idx, (expected_zone, expected_kind)) in expected_zones.iter().enumerate() {
            let zobj = zones_arr[idx].as_object().unwrap();
            let mut zkeys: Vec<&str> = zobj.keys().map(String::as_str).collect();
            zkeys.sort_unstable();
            assert_eq!(
                zkeys,
                vec![
                    "capabilities",
                    "endpoint",
                    "error",
                    "healthy",
                    "reachable",
                    "status",
                    "zone",
                ]
            );
            assert_eq!(zobj["zone"], *expected_zone);
            assert_eq!(zobj["reachable"], true);
            assert_eq!(zobj["healthy"], true);
            assert_eq!(zobj["status"], "ok");
            assert_eq!(zobj["capabilities"]["capabilities"]["kind"], *expected_kind);
        }

        // Verify per-zone capability keys match the ADR-019 §4.3 schema snapshot.
        let mut gw_caps: Vec<&str> = zones_arr[0]["capabilities"]["capabilities"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        gw_caps.sort_unstable();
        assert_eq!(
            gw_caps,
            vec![
                "admin_listen",
                "admin_token_configured",
                "consensus_ipc_reachable",
                "ipc_listen",
                "kind",
                "public_listen",
                "rate_limit_max",
                "rate_limit_window_secs",
            ]
        );
        let mut co_caps: Vec<&str> = zones_arr[1]["capabilities"]["capabilities"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        co_caps.sort_unstable();
        assert_eq!(
            co_caps,
            vec![
                "active_validators",
                "chain_id",
                "hybrid_active",
                "kind",
                "node_id",
                "signing_backend",
                "tip_height",
            ]
        );
        let mut sb_caps: Vec<&str> = zones_arr[2]["capabilities"]["capabilities"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        sb_caps.sort_unstable();
        assert_eq!(
            sb_caps,
            vec![
                "available_engines",
                "fallback_active",
                "kind",
                "max_timeout_secs",
                "rootfs_configured",
                "selected_engine",
            ]
        );
        let mut ag_caps: Vec<&str> = zones_arr[3]["capabilities"]["capabilities"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        ag_caps.sort_unstable();
        assert_eq!(
            ag_caps,
            vec![
                "ai_mode",
                "configured_providers",
                "daily_budget_usd",
                "distinct_model_families",
                "diversity_warning",
                "ipfs_configured",
                "kind",
                "sandbox_ipc_configured",
                "spent_today_usd",
            ]
        );

        let roundtrip: BTreeMap<String, serde_json::Value> = serde_json::from_value(value).unwrap();
        assert!(roundtrip.contains_key("zones"));

        gw_h.abort();
        co_h.abort();
        sb_h.abort();
        ag_h.abort();
    }

    #[tokio::test]
    async fn validator_doctor_marks_unreachable_zone_without_aborting_others() {
        let dir = TempDir::new().unwrap();
        let master = ZoneIpcToken::generate();

        let (gw_ep, gw_h) = spawn_mock_zone(ZoneId::Gateway, &master).await;
        let (co_ep, co_h) = spawn_mock_zone(ZoneId::Consensus, &master).await;
        let dead_sandbox_ep = unused_loopback_endpoint();
        let (ag_ep, ag_h) = spawn_mock_zone(ZoneId::AgentRunner, &master).await;

        let bundle = OperatorIpcKeyBundleV1 {
            schema_version: 1,
            gateway: OperatorZoneEndpointKeyV1 {
                endpoint: gw_ep,
                link_key_hex: master
                    .derive_link_key(ZoneId::Operator, ZoneId::Gateway)
                    .to_hex(),
            },
            consensus: OperatorZoneEndpointKeyV1 {
                endpoint: co_ep,
                link_key_hex: master
                    .derive_link_key(ZoneId::Operator, ZoneId::Consensus)
                    .to_hex(),
            },
            sandbox: OperatorZoneEndpointKeyV1 {
                endpoint: dead_sandbox_ep,
                link_key_hex: master
                    .derive_link_key(ZoneId::Operator, ZoneId::Sandbox)
                    .to_hex(),
            },
            agent_runner: OperatorZoneEndpointKeyV1 {
                endpoint: ag_ep,
                link_key_hex: master
                    .derive_link_key(ZoneId::Operator, ZoneId::AgentRunner)
                    .to_hex(),
            },
        };
        let bundle_path = dir.path().join("operator-ipc-keys.json");
        bundle.write_owner_only(&bundle_path).unwrap();
        let cfg_path = write_sample_ai_config(dir.path());

        let report = collect_validator_doctor_report(&bundle_path, Some(&cfg_path)).await;
        assert!(!report.ok);
        assert_eq!(report.zones.len(), 4);

        assert!(report.zones[0].reachable && report.zones[0].healthy);
        assert!(report.zones[1].reachable && report.zones[1].healthy);
        assert!(!report.zones[2].reachable && !report.zones[2].healthy);
        assert!(report.zones[2].error.is_some());
        assert!(report.zones[3].reachable && report.zones[3].healthy);

        gw_h.abort();
        co_h.abort();
        ag_h.abort();
    }
}
