use ai_config::{check_ai_config, AiConfigCheckReportV1, ValidatorAiConfig};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use zone_ipc::{
    OperatorIpcKeyBundleV1, ProviderProbeReportV1, ZoneCapabilitiesV1, ZoneCapabilityReportV1,
    ZoneId, ZoneRequest, ZoneResponse, OPERATOR_KEY_FILE_ENV,
};

const ZONE_IPC_TIMEOUT_SECS: u64 = 5;
const PROBE_IPC_TIMEOUT_SECS: u64 = 20;
const MAX_AI_CONFIG_BYTES: u64 = 256 * 1024;

pub const VALIDATOR_ZONES_ORDER: [ZoneId; 4] = [
    ZoneId::Gateway,
    ZoneId::Consensus,
    ZoneId::Sandbox,
    ZoneId::AgentRunner,
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneProbeStatus {
    pub zone: ZoneId,
    pub endpoint: String,
    pub reachable: bool,
    pub healthy: bool,
    pub status: Option<String>,
    pub capabilities: Option<ZoneCapabilityReportV1>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesktopDoctorReport {
    pub schema_version: u16,
    pub ok: bool,
    pub operator_key_file: Option<String>,
    pub zones: Vec<ZoneProbeStatus>,
    pub ai_config: Option<AiConfigCheckReportV1>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn default_operator_key_file() -> PathBuf {
    if let Ok(path) = std::env::var(OPERATOR_KEY_FILE_ENV) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Ok(data_dir) = std::env::var("CREG_DATA_DIR") {
        let trimmed = data_dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("operator-ipc-keys.json");
        }
    }
    if let Some(home) = dirs::home_dir() {
        let candidate = home.join(".creg").join("operator-ipc-keys.json");
        if candidate.exists() {
            return candidate;
        }
    }
    let system_default = PathBuf::from("/var/lib/creg/operator-ipc-keys.json");
    if system_default.exists() {
        return system_default;
    }
    dirs::home_dir()
        .map(|h| h.join(".creg").join("operator-ipc-keys.json"))
        .unwrap_or_else(|| PathBuf::from("operator-ipc-keys.json"))
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

async fn probe_single_zone(bundle: &OperatorIpcKeyBundleV1, zone: ZoneId) -> ZoneProbeStatus {
    let endpoint = bundle
        .endpoint_and_key(zone)
        .map(|entry| entry.endpoint.clone())
        .unwrap_or_default();

    let client = match bundle.client_for(zone) {
        Ok(c) => c,
        Err(err) => {
            return ZoneProbeStatus {
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

    let call = client.call(1, &ZoneRequest::Capabilities {});
    let response =
        match tokio::time::timeout(Duration::from_secs(ZONE_IPC_TIMEOUT_SECS), call).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(err)) => {
                return ZoneProbeStatus {
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
                return ZoneProbeStatus {
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
                        return ZoneProbeStatus {
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
                        return ZoneProbeStatus {
                            zone,
                            endpoint,
                            reachable: true,
                            healthy: false,
                            status: Some(report.status.clone()),
                            capabilities: Some(report.clone()),
                            error: Some(format!(
                                "capability payload kind does not match zone {zone}"
                            )),
                        };
                    }
                    let normalized = report.status.trim().to_ascii_lowercase();
                    let healthy = matches!(normalized.as_str(), "ok" | "healthy" | "ready");
                    let error = if healthy {
                        None
                    } else {
                        Some(format!(
                            "zone {zone} reported non-healthy status '{}'",
                            report.status
                        ))
                    };
                    ZoneProbeStatus {
                        zone,
                        endpoint,
                        reachable: true,
                        healthy,
                        status: Some(report.status.clone()),
                        capabilities: Some(report),
                        error,
                    }
                }
                Err(err) => ZoneProbeStatus {
                    zone,
                    endpoint,
                    reachable: true,
                    healthy: false,
                    status: None,
                    capabilities: None,
                    error: Some(format!("invalid ZoneCapabilityReportV1 payload: {err}")),
                },
            }
        }
        ZoneResponse::Error { code, message } => ZoneProbeStatus {
            zone,
            endpoint,
            reachable: true,
            healthy: false,
            status: Some("error".to_string()),
            capabilities: None,
            error: Some(format!("zone error {code}: {message}")),
        },
        other => ZoneProbeStatus {
            zone,
            endpoint,
            reachable: true,
            healthy: false,
            status: None,
            capabilities: None,
            error: Some(format!("unexpected IPC response variant: {other:?}")),
        },
    }
}

pub async fn probe_all_zones(
    operator_key_file: Option<&Path>,
    ai_config_path: Option<&Path>,
) -> DesktopDoctorReport {
    let key_path = operator_key_file
        .map(Path::to_path_buf)
        .unwrap_or_else(default_operator_key_file);
    let key_path_display = key_path.display().to_string();

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut zones = Vec::with_capacity(4);

    match OperatorIpcKeyBundleV1::read_owner_only(&key_path) {
        Ok(bundle) => {
            for zone in VALIDATOR_ZONES_ORDER {
                let probe = probe_single_zone(&bundle, zone).await;
                if let Some(err) = &probe.error {
                    errors.push(format!("{}: {}", probe.zone, err));
                }
                zones.push(probe);
            }
        }
        Err(err) => {
            errors.push(format!(
                "Failed to load operator IPC key bundle '{}': {err:#}",
                key_path.display()
            ));
            for zone in VALIDATOR_ZONES_ORDER {
                zones.push(ZoneProbeStatus {
                    zone,
                    endpoint: String::new(),
                    reachable: false,
                    healthy: false,
                    status: None,
                    capabilities: None,
                    error: Some("operator IPC key bundle unavailable".to_string()),
                });
            }
        }
    }

    let ai_config = match check_ai_config_file(ai_config_path) {
        Ok(report) => {
            if let Some(adv) = &report.diversity_advisory {
                warnings.push(adv.clone());
            }
            for e in &report.errors {
                errors.push(format!("ai-config: {e}"));
            }
            for w in &report.warnings {
                warnings.push(format!("ai-config: {w}"));
            }
            Some(report)
        }
        Err(err) => {
            warnings.push(format!("AI config check skipped: {err:#}"));
            None
        }
    };

    let all_zones_healthy = zones.iter().all(|z| z.healthy);
    let ok = all_zones_healthy && errors.is_empty();

    DesktopDoctorReport {
        schema_version: 1,
        ok,
        operator_key_file: Some(key_path_display),
        zones,
        ai_config,
        errors,
        warnings,
    }
}

pub fn check_ai_config_file(config_path: Option<&Path>) -> Result<AiConfigCheckReportV1> {
    let resolved = config_path.map(Path::to_path_buf).or_else(|| {
        std::env::var("CREG_AI_CONFIG")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    });

    let (cfg, label) = match resolved {
        Some(path) => (
            ValidatorAiConfig::from_file(&path)?,
            Some(path.display().to_string()),
        ),
        None => (ValidatorAiConfig::from_env()?, None),
    };

    Ok(check_ai_config(&cfg, label, false))
}

pub fn check_ai_config_toml_str(
    toml_text: &str,
    label: Option<String>,
) -> Result<AiConfigCheckReportV1> {
    let cfg = ValidatorAiConfig::from_toml_str(toml_text)?;
    Ok(check_ai_config(&cfg, label, false))
}

pub fn read_ai_config_text(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path)
        .with_context(|| format!("failed to stat AI config file '{}'", path.display()))?;
    if meta.len() > MAX_AI_CONFIG_BYTES {
        bail!(
            "AI config file exceeds {} byte safety cap",
            MAX_AI_CONFIG_BYTES
        );
    }
    std::fs::read_to_string(path)
        .with_context(|| format!("failed to read AI config file '{}'", path.display()))
}

pub fn save_ai_config_text(path: &Path, toml_text: &str) -> Result<AiConfigCheckReportV1> {
    let cfg = ValidatorAiConfig::from_toml_str(toml_text)
        .context("validator-ai.toml failed schema/validation checks")?;
    let report = check_ai_config(&cfg, Some(path.display().to_string()), false);
    if !report.ok {
        bail!(
            "refusing to write invalid AI config: {}",
            report.errors.join("; ")
        );
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create parent directory '{}'", parent.display())
            })?;
        }
    }

    let tmp_path = path.with_extension("toml.tmp");
    let normalized = toml_text.replace("\r\n", "\n");
    std::fs::write(&tmp_path, normalized.as_bytes()).with_context(|| {
        format!(
            "failed to write temporary AI config '{}'",
            tmp_path.display()
        )
    })?;
    if let Err(err) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err).with_context(|| format!("failed to replace '{}'", path.display()));
    }

    Ok(report)
}

pub async fn probe_ai_providers(
    operator_key_file: Option<&Path>,
    provider: Option<String>,
) -> Result<ProviderProbeReportV1> {
    let key_path = operator_key_file
        .map(Path::to_path_buf)
        .unwrap_or_else(default_operator_key_file);
    let bundle = OperatorIpcKeyBundleV1::read_owner_only(&key_path).with_context(|| {
        format!(
            "failed to load operator IPC key bundle '{}'",
            key_path.display()
        )
    })?;
    let client = bundle
        .client_for(ZoneId::AgentRunner)
        .context("operator key bundle has no route for agent_runner")?;

    let req = ZoneRequest::ProbeProvider { provider };
    let resp = tokio::time::timeout(
        Duration::from_secs(PROBE_IPC_TIMEOUT_SECS),
        client.call(1, &req),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!("AgentRunner ProbeProvider timed out after {PROBE_IPC_TIMEOUT_SECS}s")
    })?
    .context("ZoneIPC ProbeProvider call failed")?;

    match resp {
        ZoneResponse::ProviderProbeResult { report_json } => {
            serde_json::from_str::<ProviderProbeReportV1>(&report_json)
                .context("failed to parse ProviderProbeReportV1")
        }
        ZoneResponse::Error { code, message } => {
            bail!("agent_runner returned error {code}: {message}");
        }
        other => bail!("unexpected response from agent_runner: {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probe_all_zones_fails_gracefully_when_key_bundle_missing() {
        let missing = PathBuf::from("/nonexistent/creg/operator-ipc-keys.json");
        let report = probe_all_zones(Some(&missing), None).await;
        assert!(!report.ok);
        assert_eq!(report.zones.len(), 4);
        assert!(report.zones.iter().all(|z| !z.reachable && !z.healthy));
        assert!(!report.errors.is_empty());
    }

    #[test]
    fn save_ai_config_text_rejects_inline_secret_and_writes_valid_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("validator-ai.toml");

        let inline_secret_toml = r#"
[agent]
mode = "triage_only"

[providers.cloud]
kind = "openai"
model = "gpt-4o-mini"
api_key_secret_ref = "sk-live-secret-not-a-ref"

[routing]
tier_fast = "cloud"
"#;
        assert!(save_ai_config_text(&path, inline_secret_toml).is_err());
        assert!(!path.exists());

        let valid_toml = r#"
[agent]
mode = "triage_only"

[providers.local_ollama]
kind = "ollama"
model = "qwen2.5:7b"
base_url = "http://127.0.0.1:11434"

[routing]
tier_fast = "local_ollama"
"#;
        let report = save_ai_config_text(&path, valid_toml).expect("valid config saves cleanly");
        assert!(report.ok);
        assert!(path.exists());
        let reread = read_ai_config_text(&path).expect("reread saved config");
        assert!(reread.contains("local_ollama"));
    }
}
