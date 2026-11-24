// crates/cli/src/ai_ops.rs
// Operator diagnostics for validator AI configuration and Zone 4 live probing
// (`creg ai check` / `creg ai probe`, ADR-019 §4.3).

use ai_config::{
    check_ai_config, AgentMode, AiConfigCheckReportV1, ProviderKind, ValidatorAiConfig,
};
use anyhow::{bail, Context, Result};
use colored::Colorize;
use std::path::{Path, PathBuf};
use zone_ipc::{
    OperatorIpcKeyBundleV1, ProviderProbeReportV1, ZoneId, ZoneRequest, ZoneResponse,
    OPERATOR_KEY_FILE_ENV,
};

const SINGLE_SHOT_IPC_REQUEST_ID: u64 = 1;

/// Resolve the default owner-only operator IPC key bundle location matching
/// `creg-node-supervisor` (`CREG_OPERATOR_IPC_KEY_FILE`, `$CREG_DATA_DIR/operator-ipc-keys.json`,
/// `/var/lib/creg/operator-ipc-keys.json` if present, or `~/.creg/operator-ipc-keys.json`).
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
    let system_default = PathBuf::from("/var/lib/creg/operator-ipc-keys.json");
    if system_default.exists() {
        return system_default;
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".creg").join("operator-ipc-keys.json");
    }
    #[cfg(unix)]
    {
        system_default
    }
    #[cfg(not(unix))]
    {
        PathBuf::from("operator-ipc-keys.json")
    }
}

pub fn resolve_operator_key_file(override_path: Option<&Path>) -> PathBuf {
    match override_path {
        Some(path) => path.to_path_buf(),
        None => default_operator_key_file(),
    }
}

pub const fn agent_mode_label(mode: AgentMode) -> &'static str {
    match mode {
        AgentMode::Off => "off",
        AgentMode::TriageOnly => "triage_only",
        AgentMode::HybridEscalate => "hybrid_escalate",
        AgentMode::DeepAll => "deep_all",
    }
}

pub const fn provider_kind_label(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Ollama => "ollama",
        ProviderKind::Vllm => "vllm",
        ProviderKind::LlamaCpp => "llama_cpp",
        ProviderKind::Sglang => "sglang",
        ProviderKind::LmStudio => "lm_studio",
        ProviderKind::OpenAiCompatible => "openai_compatible",
        ProviderKind::Gemini => "gemini",
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::OpenAi => "openai",
        ProviderKind::OpenRouter => "openrouter",
    }
}

/// Load and statically validate a `ValidatorAiConfig` without exposing any secret values.
pub fn check_config_report(config_path: Option<&Path>) -> Result<AiConfigCheckReportV1> {
    let explicit_or_env = match config_path {
        Some(path) => Some(path.to_path_buf()),
        None => std::env::var("CREG_AI_CONFIG")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from),
    };

    let (cfg, label) = match explicit_or_env {
        Some(ref path) => {
            let cfg = ValidatorAiConfig::from_file(path)?;
            (cfg, Some(path.display().to_string()))
        }
        None => {
            let cfg = ValidatorAiConfig::from_env()?;
            (cfg, None)
        }
    };

    Ok(check_ai_config(&cfg, label, false))
}

pub fn format_check_report_human(report: &AiConfigCheckReportV1) -> String {
    let mut out = String::new();
    let header = if report.ok {
        "CREG AI Config Check: PASS".green().bold().to_string()
    } else {
        "CREG AI Config Check: FAIL".red().bold().to_string()
    };
    out.push_str(&format!("{header}\n"));
    out.push_str(&format!(
        "  config:   {}\n",
        report
            .config_path
            .as_deref()
            .unwrap_or("<environment fallback>")
    ));
    out.push_str(&format!("  mode:     {}\n", agent_mode_label(report.mode)));
    out.push_str(&format!(
        "  families: {} distinct model_family\n",
        report.distinct_model_families
    ));
    if let Some(advisory) = &report.diversity_advisory {
        out.push_str(&format!("  {} {}\n", "advisory:".yellow().bold(), advisory));
    }
    if report.providers.is_empty() {
        out.push_str("  providers: (none configured)\n");
    } else {
        out.push_str("  providers:\n");
        for p in &report.providers {
            let status = if p.url_ok && p.secret_ok && p.issues.is_empty() {
                "ok".green().to_string()
            } else {
                "fail".red().to_string()
            };
            let class_str = p.endpoint_class.map(|c| c.as_str()).unwrap_or("invalid");
            let secret_str = p.secret_scheme.as_deref().unwrap_or("none");
            out.push_str(&format!(
                "    - {} [{}] kind={} model={} family={} endpoint={}({}) secret={} ok={}\n",
                p.name,
                status,
                provider_kind_label(p.kind),
                p.model,
                p.model_family,
                p.base_url,
                class_str,
                secret_str,
                p.secret_ok
            ));
            for issue in &p.issues {
                out.push_str(&format!("        issue: {issue}\n"));
            }
        }
    }
    for warning in &report.warnings {
        out.push_str(&format!("  {} {}\n", "warn:".yellow(), warning));
    }
    for err in &report.errors {
        out.push_str(&format!("  {} {}\n", "error:".red(), err));
    }
    out
}

pub fn run_check(config_path: Option<&Path>, json: bool) -> Result<()> {
    match check_config_report(config_path) {
        Ok(report) => {
            if json {
                let encoded = serde_json::to_string_pretty(&report)
                    .context("serialize AiConfigCheckReportV1")?;
                println!("{encoded}");
            } else {
                print!("{}", format_check_report_human(&report));
            }
            if !report.ok {
                bail!("AI configuration check failed");
            }
            Ok(())
        }
        Err(err) => {
            if json {
                let err_report = AiConfigCheckReportV1 {
                    schema_version: 1,
                    config_path: config_path.map(|p| p.display().to_string()),
                    mode: AgentMode::Off,
                    ok: false,
                    distinct_model_families: 0,
                    diversity_advisory: None,
                    providers: Vec::new(),
                    errors: vec![format!("{err:#}")],
                    warnings: Vec::new(),
                };
                if let Ok(encoded) = serde_json::to_string_pretty(&err_report) {
                    println!("{encoded}");
                }
            }
            Err(err)
        }
    }
}

/// Delegate live AI provider canary probing to Zone 4 (`creg-agent-runner`)
/// over the owner-only `ZoneId::Operator` IPC link.
pub async fn probe_providers_report(
    bundle_path: &Path,
    provider: Option<&str>,
) -> Result<ProviderProbeReportV1> {
    let bundle = OperatorIpcKeyBundleV1::read_owner_only(bundle_path)
        .with_context(|| format!("load operator IPC bundle from {}", bundle_path.display()))?;
    let client = bundle
        .client_for(ZoneId::AgentRunner)
        .context("build operator IPC client for agent_runner zone")?;
    let resp = client
        .call(
            SINGLE_SHOT_IPC_REQUEST_ID,
            &ZoneRequest::ProbeProvider {
                provider: provider.map(str::to_string),
            },
        )
        .await
        .context("send ProbeProvider over operator IPC link to agent_runner")?;

    match resp {
        ZoneResponse::ProviderProbeResult { report_json } => {
            serde_json::from_str::<ProviderProbeReportV1>(&report_json)
                .context("decode ProviderProbeReportV1 from agent_runner")
        }
        ZoneResponse::Error { code, message } => {
            bail!("agent_runner rejected ProbeProvider (code {code}): {message}")
        }
        other => bail!("unexpected ZoneResponse from agent_runner: {other:?}"),
    }
}

pub fn format_probe_report_human(report: &ProviderProbeReportV1) -> String {
    let mut out = String::new();
    let header = if report.ok {
        "CREG AI Provider Probe: PASS".green().bold().to_string()
    } else {
        "CREG AI Provider Probe: FAIL".red().bold().to_string()
    };
    out.push_str(&format!("{header}\n"));
    if report.providers.is_empty() {
        out.push_str("  providers: (none probed)\n");
        return out;
    }
    for entry in &report.providers {
        let status = if entry.ok && entry.valid_tool_json {
            "ok".green().to_string()
        } else {
            "fail".red().to_string()
        };
        out.push_str(&format!(
            "  - {} [{}] kind={} model={} family={} endpoint_class={} latency={}ms tool_json={}\n",
            entry.provider,
            status,
            entry.kind,
            entry.model,
            entry.model_family,
            entry.endpoint_class,
            entry.latency_ms,
            entry.valid_tool_json
        ));
        if let Some(err) = &entry.error {
            out.push_str(&format!("      error: {err}\n"));
        }
    }
    out
}

pub async fn run_probe(
    operator_key_file: Option<&Path>,
    provider: Option<&str>,
    json: bool,
) -> Result<()> {
    let bundle_path = resolve_operator_key_file(operator_key_file);
    let report = probe_providers_report(&bundle_path, provider).await?;
    if json {
        let encoded =
            serde_json::to_string_pretty(&report).context("serialize ProviderProbeReportV1")?;
        println!("{encoded}");
    } else {
        print!("{}", format_probe_report_human(&report));
    }
    if !report.ok {
        bail!("one or more AI provider probes failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;
    use zone_ipc::{
        OperatorZoneEndpointKeyV1, ProviderProbeEntryV1, ZoneEndpoint, ZoneIpcServer, ZoneIpcToken,
    };

    fn write_owner_only_secret(path: &Path, secret_value: &str) {
        let mut f = std::fs::File::create(path).unwrap();
        writeln!(f, "{secret_value}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[test]
    fn ai_check_reports_single_family_advisory_and_never_leaks_secrets() {
        let dir = TempDir::new().unwrap();
        let secret_path = dir.path().join("gemini-key.txt");
        let sentinel = "SUPER_SECRET_SENTINEL_XYZ_987654";
        write_owner_only_secret(&secret_path, sentinel);

        let toml_path = dir.path().join("validator-ai.toml");
        let toml = format!(
            "[agent]\n\
             mode = \"hybrid_escalate\"\n\
             shadow_mode = true\n\
             max_steps = 8\n\
             max_sandbox_replays = 3\n\
             max_daily_api_spend_usd = 25.0\n\
             \n\
             [providers.primary]\n\
             kind = \"gemini\"\n\
             model = \"gemini-2.5-flash\"\n\
             model_family = \"gemini\"\n\
             api_key_secret_ref = 'file:{}'\n\
             \n\
             [routing]\n\
             tier_fast = \"primary\"\n\
             tier_deep = \"primary\"\n",
            secret_path.display()
        );
        std::fs::write(&toml_path, toml).unwrap();

        let report = check_config_report(Some(&toml_path)).unwrap();
        assert!(report.ok);
        assert_eq!(report.mode, AgentMode::HybridEscalate);
        assert_eq!(report.distinct_model_families, 1);
        assert!(report.diversity_advisory.is_some());
        assert_eq!(report.providers.len(), 1);
        assert_eq!(report.providers[0].secret_scheme.as_deref(), Some("file"));
        assert!(report.providers[0].secret_ok);

        let serialized = serde_json::to_string_pretty(&report).unwrap();
        let rendered = format_check_report_human(&report);
        assert!(!serialized.contains(sentinel));
        assert!(!rendered.contains(sentinel));
    }

    #[test]
    fn ai_check_fails_on_inline_secret_or_invalid_toml() {
        let dir = TempDir::new().unwrap();

        let inline_secret_path = dir.path().join("inline-secret.toml");
        std::fs::write(
            &inline_secret_path,
            "[agent]\n\
             mode = \"hybrid_escalate\"\n\
             \n\
             [providers.primary]\n\
             kind = \"openai\"\n\
             model = \"gpt-4o-mini\"\n\
             api_key_secret_ref = \"sk-plaintext-inline-secret\"\n\
             \n\
             [routing]\n\
             tier_fast = \"primary\"\n",
        )
        .unwrap();
        assert!(check_config_report(Some(&inline_secret_path)).is_err());

        let bad_toml_path = dir.path().join("broken.toml");
        std::fs::write(&bad_toml_path, "[agent\nmode = 123\n").unwrap();
        assert!(check_config_report(Some(&bad_toml_path)).is_err());
    }

    #[tokio::test]
    async fn ai_probe_delegates_over_operator_zone_ipc() {
        let dir = TempDir::new().unwrap();
        let master = ZoneIpcToken::generate();

        let server = ZoneIpcServer::bind(
            ZoneId::AgentRunner,
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

        let server_task = tokio::spawn(async move {
            let _ = server
                .serve(|sender, req| {
                    Box::pin(async move {
                        assert_eq!(sender, ZoneId::Operator);
                        match req {
                            ZoneRequest::ProbeProvider { provider } => {
                                let target_name = provider.unwrap_or_else(|| "primary".to_string());
                                let report = ProviderProbeReportV1 {
                                    schema_version: 1,
                                    ok: true,
                                    providers: vec![ProviderProbeEntryV1 {
                                        provider: target_name,
                                        kind: "gemini".to_string(),
                                        model: "gemini-2.5-flash".to_string(),
                                        model_family: "gemini".to_string(),
                                        endpoint_class: "api".to_string(),
                                        ok: true,
                                        latency_ms: 42,
                                        valid_tool_json: true,
                                        error: None,
                                    }],
                                };
                                ZoneResponse::ProviderProbeResult {
                                    report_json: serde_json::to_string(&report).unwrap(),
                                }
                            }
                            other => ZoneResponse::Error {
                                code: 400,
                                message: format!("unexpected request: {other:?}"),
                            },
                        }
                    })
                })
                .await;
        });

        let link_key_hex = master
            .derive_link_key(ZoneId::Operator, ZoneId::AgentRunner)
            .to_hex();
        let bundle = OperatorIpcKeyBundleV1 {
            schema_version: 1,
            gateway: OperatorZoneEndpointKeyV1 {
                endpoint: "tcp://127.0.0.1:1".to_string(),
                link_key_hex: "00".repeat(32),
            },
            consensus: OperatorZoneEndpointKeyV1 {
                endpoint: "tcp://127.0.0.1:1".to_string(),
                link_key_hex: "00".repeat(32),
            },
            sandbox: OperatorZoneEndpointKeyV1 {
                endpoint: "tcp://127.0.0.1:1".to_string(),
                link_key_hex: "00".repeat(32),
            },
            agent_runner: OperatorZoneEndpointKeyV1 {
                endpoint,
                link_key_hex,
            },
        };
        let bundle_path = dir.path().join("operator-ipc-keys.json");
        bundle.write_owner_only(&bundle_path).unwrap();

        let probe = probe_providers_report(&bundle_path, Some("canary"))
            .await
            .unwrap();
        assert!(probe.ok);
        assert_eq!(probe.providers.len(), 1);
        assert_eq!(probe.providers[0].provider, "canary");
        assert!(probe.providers[0].valid_tool_json);
        assert_eq!(probe.providers[0].latency_ms, 42);

        server_task.abort();
    }
}
