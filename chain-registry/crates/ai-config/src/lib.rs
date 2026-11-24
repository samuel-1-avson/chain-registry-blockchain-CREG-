//! Dependency-minimal Validator AI configuration (`validator-ai.toml`),
//! secret-reference validation (`env:`, `vault:`, `file:`), SSRF URL guard,
//! and offline configuration diagnostics (`ADR-019` `ECA-16`, §4.3).

pub mod check;
pub mod secret;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use std::path::Path;

pub use check::{
    check_ai_config, AiConfigCheckReportV1, AiProviderCheckV1, RECOMMENDED_MIN_MODEL_FAMILIES,
};
pub use secret::{
    check_secret_file, check_secret_file_non_empty, parse_secret_ref, read_secret_file_zeroized,
    validate_secret_file_path, SecretRefKind, MAX_SECRET_FILE_BYTES,
};

pub const DEFAULT_AI_CONFIG_PATH: &str = "/etc/creg/validator-ai.toml";
pub const DEFAULT_MAX_DAILY_API_SPEND_USD: f64 = 25.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    /// Deterministic-only validator (`CREG_LLM_ENABLED=false`, default).
    #[default]
    Off,
    /// Single fast triage turn using `tier_fast`.
    TriageOnly,
    /// Run `tier_fast` and escalate suspicious packages to multi-step `tier_deep`.
    HybridEscalate,
    /// Run multi-step `tier_deep` on all submitted packages.
    DeepAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Ollama,
    Vllm,
    LlamaCpp,
    Sglang,
    LmStudio,
    OpenAiCompatible,
    Gemini,
    Anthropic,
    OpenAi,
    OpenRouter,
}

impl ProviderKind {
    pub fn is_local(self) -> bool {
        matches!(
            self,
            Self::Ollama
                | Self::Vllm
                | Self::LlamaCpp
                | Self::Sglang
                | Self::LmStudio
                | Self::OpenAiCompatible
        )
    }

    pub fn requires_api_key(self) -> bool {
        matches!(
            self,
            Self::Gemini | Self::Anthropic | Self::OpenAi | Self::OpenRouter
        )
    }

    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Ollama => "http://127.0.0.1:11434",
            Self::Vllm | Self::OpenAiCompatible => "http://127.0.0.1:8000/v1",
            Self::LlamaCpp => "http://127.0.0.1:8081/v1",
            Self::Sglang => "http://127.0.0.1:30000/v1",
            Self::LmStudio => "http://127.0.0.1:1234/v1",
            Self::Gemini => "https://generativelanguage.googleapis.com",
            Self::Anthropic => "https://api.anthropic.com",
            Self::OpenAi => "https://api.openai.com/v1",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
        }
    }
}

/// Network boundary classification for an AI provider endpoint URL (`ADR-019 §4.3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointClass {
    Loopback,
    Lan,
    Api,
}

impl EndpointClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loopback => "loopback",
            Self::Lan => "lan",
            Self::Api => "api",
        }
    }
}

impl fmt::Display for EndpointClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn default_true() -> bool {
    true
}
fn default_triage_threshold() -> u8 {
    20
}
fn default_max_steps() -> usize {
    8
}
fn default_max_replays() -> usize {
    3
}
fn default_max_tokens() -> u64 {
    32_000
}
fn default_wall_timeout() -> u64 {
    45
}
fn default_daily_spend() -> f64 {
    DEFAULT_MAX_DAILY_API_SPEND_USD
}
fn default_timeout_secs() -> u64 {
    25
}
fn default_max_concurrent() -> usize {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPolicyConfig {
    #[serde(default)]
    pub mode: AgentMode,
    /// Increment 5 runs agentic verdicts as advisory shadow telemetry beside
    /// deterministic consensus rules (`VNA-20`, `INV-02`).
    #[serde(default = "default_true")]
    pub shadow_mode: bool,
    #[serde(default = "default_triage_threshold")]
    pub triage_score_threshold: u8,
    #[serde(default = "default_max_steps")]
    pub max_steps: usize,
    #[serde(default = "default_max_replays")]
    pub max_sandbox_replays: usize,
    #[serde(default = "default_max_tokens")]
    pub max_total_tokens: u64,
    #[serde(default = "default_wall_timeout")]
    pub wall_timeout_secs: u64,
    #[serde(default = "default_daily_spend")]
    pub max_daily_api_spend_usd: f64,
}

impl Default for AgentPolicyConfig {
    fn default() -> Self {
        Self {
            mode: AgentMode::Off,
            shadow_mode: true,
            triage_score_threshold: default_triage_threshold(),
            max_steps: default_max_steps(),
            max_sandbox_replays: default_max_replays(),
            max_total_tokens: default_max_tokens(),
            wall_timeout_secs: default_wall_timeout(),
            max_daily_api_spend_usd: default_daily_spend(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderProfile {
    pub kind: ProviderKind,
    #[serde(default)]
    pub base_url: Option<String>,
    pub model: String,
    #[serde(default)]
    pub model_family: String,
    /// Must use `env:<VAR_NAME>`, `vault:<path>#<field>`, or `file:<abs_path>`.
    /// Inline keys are always rejected.
    #[serde(default)]
    pub api_key_secret_ref: Option<String>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_requests: usize,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub cost_per_1k_input_usd: f64,
    #[serde(default)]
    pub cost_per_1k_output_usd: f64,
    #[serde(default)]
    pub allow_private_network: bool,
}

impl ProviderProfile {
    pub fn resolved_base_url(&self) -> &str {
        self.base_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| self.kind.default_base_url())
    }

    pub fn resolved_model_family(&self) -> String {
        if !self.model_family.trim().is_empty() {
            return self.model_family.trim().to_ascii_lowercase();
        }
        let lower = self.model.to_ascii_lowercase();
        if lower.contains("gemini") {
            "gemini".into()
        } else if lower.contains("claude") {
            "claude".into()
        } else if lower.contains("qwen") {
            "qwen".into()
        } else if lower.contains("llama") {
            "llama".into()
        } else if lower.contains("gpt") || lower.contains("o3") || lower.contains("o4") {
            "openai".into()
        } else {
            lower
                .split([':', '/', '-'])
                .next()
                .unwrap_or("unknown")
                .to_string()
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingConfig {
    #[serde(default)]
    pub tier_fast: Option<String>,
    #[serde(default)]
    pub tier_deep: Option<String>,
    #[serde(default)]
    pub tier_crosscheck: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidatorAiConfig {
    #[serde(default)]
    pub agent: AgentPolicyConfig,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderProfile>,
    #[serde(default)]
    pub routing: RoutingConfig,
}

fn parse_env_bool(key: &str) -> bool {
    std::env::var(key).ok().is_some_and(|raw| {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn env_usize(key: &str) -> Option<usize> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}

impl ValidatorAiConfig {
    pub fn from_toml_str(toml_str: &str) -> Result<Self> {
        let cfg: Self = toml::from_str(toml_str).context("parse validator-ai.toml")?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read AI config at {}", path.display()))?;
        Self::from_toml_str(&raw)
    }

    /// Parse TOML without rejecting test HTTP URLs when `allow_test_http = true`.
    pub fn from_file_with_options(path: &Path, allow_test_http: bool) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read AI config at {}", path.display()))?;
        let cfg: Self = toml::from_str(&raw).context("parse validator-ai.toml")?;
        cfg.validate_with_options(allow_test_http)?;
        Ok(cfg)
    }

    /// Load `validator-ai.toml` from `CREG_AI_CONFIG`, or synthesize backward-compatible
    /// config from existing environment variables (`CREG_LLM_ENABLED`, `CREG_LLM_PROVIDERS`, etc.).
    pub fn from_env_or_file() -> Result<Self> {
        if let Ok(path) = std::env::var("CREG_AI_CONFIG") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return Self::from_file(Path::new(trimmed));
            }
        }
        Self::from_env()
    }

    pub fn from_env() -> Result<Self> {
        let mut cfg = Self::default();
        if !parse_env_bool("CREG_LLM_ENABLED") {
            cfg.agent.mode = AgentMode::Off;
            return Ok(cfg);
        }

        cfg.agent.mode = match std::env::var("CREG_AI_MODE")
            .unwrap_or_else(|_| "hybrid_escalate".into())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "off" => AgentMode::Off,
            "triage_only" => AgentMode::TriageOnly,
            "deep_all" => AgentMode::DeepAll,
            _ => AgentMode::HybridEscalate,
        };

        if let Some(steps) = env_usize("CREG_AI_MAX_STEPS") {
            cfg.agent.max_steps = steps.clamp(1, 12);
        }
        if let Some(replays) = env_usize("CREG_AI_MAX_REPLAYS") {
            cfg.agent.max_sandbox_replays = replays.min(3);
        }
        if let Some(tokens) = env_usize("CREG_AI_MAX_TOKENS") {
            cfg.agent.max_total_tokens = (tokens as u64).clamp(1024, 256_000);
        }

        let order = std::env::var("CREG_LLM_PROVIDERS")
            .unwrap_or_else(|_| "ollama,gemini,anthropic,openai,openrouter".into());
        let mut ordered_names = Vec::new();

        for raw in order.split(',') {
            match raw.trim().to_ascii_lowercase().as_str() {
                "ollama" => {
                    let base = std::env::var("CREG_OLLAMA_URL")
                        .ok()
                        .filter(|v| !v.trim().is_empty());
                    let model = std::env::var("CREG_OLLAMA_MODEL")
                        .unwrap_or_else(|_| "qwen2.5-coder:14b".into());
                    cfg.providers.insert(
                        "local_ollama".into(),
                        ProviderProfile {
                            kind: ProviderKind::Ollama,
                            base_url: Some(
                                base.unwrap_or_else(|| {
                                    ProviderKind::Ollama.default_base_url().into()
                                }),
                            ),
                            model,
                            model_family: "qwen".into(),
                            api_key_secret_ref: None,
                            max_concurrent_requests: 2,
                            timeout_secs: 25,
                            cost_per_1k_input_usd: 0.0,
                            cost_per_1k_output_usd: 0.0,
                            allow_private_network: false,
                        },
                    );
                    ordered_names.push("local_ollama".to_string());
                }
                "vllm" => {
                    if let Ok(base) = std::env::var("CREG_VLLM_URL") {
                        if !base.trim().is_empty() {
                            let model = std::env::var("CREG_VLLM_MODEL")
                                .unwrap_or_else(|_| "Qwen/Qwen2.5-Coder-32B-Instruct-AWQ".into());
                            cfg.providers.insert(
                                "local_vllm".into(),
                                ProviderProfile {
                                    kind: ProviderKind::Vllm,
                                    base_url: Some(base),
                                    model,
                                    model_family: "qwen".into(),
                                    api_key_secret_ref: None,
                                    max_concurrent_requests: 4,
                                    timeout_secs: 25,
                                    cost_per_1k_input_usd: 0.0,
                                    cost_per_1k_output_usd: 0.0,
                                    allow_private_network: false,
                                },
                            );
                            ordered_names.push("local_vllm".to_string());
                        }
                    }
                }
                "gemini" => {
                    if std::env::var("GEMINI_API_KEY").is_ok_and(|k| !k.trim().is_empty()) {
                        let model = std::env::var("CREG_GEMINI_MODEL")
                            .unwrap_or_else(|_| "gemini-2.5-pro".into());
                        cfg.providers.insert(
                            "gemini_api".into(),
                            ProviderProfile {
                                kind: ProviderKind::Gemini,
                                base_url: None,
                                model,
                                model_family: "gemini".into(),
                                api_key_secret_ref: Some("env:GEMINI_API_KEY".into()),
                                max_concurrent_requests: 8,
                                timeout_secs: 30,
                                cost_per_1k_input_usd: 0.00125,
                                cost_per_1k_output_usd: 0.005,
                                allow_private_network: false,
                            },
                        );
                        ordered_names.push("gemini_api".to_string());
                    }
                }
                "anthropic" | "claude" => {
                    if std::env::var("ANTHROPIC_API_KEY").is_ok_and(|k| !k.trim().is_empty()) {
                        let model = std::env::var("CREG_ANTHROPIC_MODEL")
                            .unwrap_or_else(|_| "claude-sonnet-4-6".into());
                        cfg.providers.insert(
                            "anthropic_api".into(),
                            ProviderProfile {
                                kind: ProviderKind::Anthropic,
                                base_url: None,
                                model,
                                model_family: "claude".into(),
                                api_key_secret_ref: Some("env:ANTHROPIC_API_KEY".into()),
                                max_concurrent_requests: 8,
                                timeout_secs: 30,
                                cost_per_1k_input_usd: 0.003,
                                cost_per_1k_output_usd: 0.015,
                                allow_private_network: false,
                            },
                        );
                        ordered_names.push("anthropic_api".to_string());
                    }
                }
                "openai" | "gpt" => {
                    if std::env::var("OPENAI_API_KEY").is_ok_and(|k| !k.trim().is_empty()) {
                        let model =
                            std::env::var("CREG_OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".into());
                        cfg.providers.insert(
                            "openai_api".into(),
                            ProviderProfile {
                                kind: ProviderKind::OpenAi,
                                base_url: None,
                                model,
                                model_family: "openai".into(),
                                api_key_secret_ref: Some("env:OPENAI_API_KEY".into()),
                                max_concurrent_requests: 8,
                                timeout_secs: 30,
                                cost_per_1k_input_usd: 0.0025,
                                cost_per_1k_output_usd: 0.010,
                                allow_private_network: false,
                            },
                        );
                        ordered_names.push("openai_api".to_string());
                    }
                }
                "openrouter"
                    if std::env::var("OPENROUTER_API_KEY").is_ok_and(|k| !k.trim().is_empty()) =>
                {
                    let model = std::env::var("CREG_LLM_MODEL")
                        .unwrap_or_else(|_| "anthropic/claude-sonnet-4-6".into());
                    let base_url = std::env::var("CREG_LLM_API_URL")
                        .ok()
                        .filter(|v| !v.trim().is_empty());
                    cfg.providers.insert(
                        "openrouter_api".into(),
                        ProviderProfile {
                            kind: ProviderKind::OpenRouter,
                            base_url,
                            model,
                            model_family: "openrouter".into(),
                            api_key_secret_ref: Some("env:OPENROUTER_API_KEY".into()),
                            max_concurrent_requests: 8,
                            timeout_secs: 30,
                            cost_per_1k_input_usd: 0.003,
                            cost_per_1k_output_usd: 0.015,
                            allow_private_network: false,
                        },
                    );
                    ordered_names.push("openrouter_api".to_string());
                }
                _ => {}
            }
        }

        if let Some(first) = ordered_names.first() {
            cfg.routing.tier_fast = Some(first.clone());
            cfg.routing.tier_deep = Some(
                ordered_names
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| first.clone()),
            );
        } else {
            cfg.agent.mode = AgentMode::Off;
        }
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_with_options(false)
    }

    pub fn validate_with_options(&self, allow_test_http: bool) -> Result<()> {
        if !(1..=12).contains(&self.agent.max_steps) {
            bail!("agent.max_steps must be between 1 and 12");
        }
        if self.agent.max_sandbox_replays > 3 {
            bail!("agent.max_sandbox_replays cannot exceed 3");
        }
        if self.agent.max_total_tokens == 0 {
            bail!("agent.max_total_tokens must be > 0");
        }
        if !self.agent.max_daily_api_spend_usd.is_finite()
            || self.agent.max_daily_api_spend_usd < 0.0
        {
            bail!("agent.max_daily_api_spend_usd must be finite and non-negative");
        }

        for (name, profile) in &self.providers {
            if profile.model.trim().is_empty() {
                bail!("provider '{name}' must specify a non-empty model");
            }
            if profile.max_concurrent_requests == 0 || profile.timeout_secs == 0 {
                bail!("provider '{name}' concurrency and timeout must be > 0");
            }
            if profile.kind.requires_api_key() && profile.api_key_secret_ref.is_none() {
                bail!(
                    "cloud provider '{name}' must configure api_key_secret_ref (env:..., vault:..., or file:...)"
                );
            }
            if let Some(ref secret_ref) = profile.api_key_secret_ref {
                parse_secret_ref(secret_ref)
                    .with_context(|| format!("invalid api_key_secret_ref for provider '{name}'"))?;
            }
            validate_provider_url(
                profile.resolved_base_url(),
                profile.kind,
                profile.allow_private_network,
                allow_test_http,
            )
            .with_context(|| format!("invalid base_url for provider '{name}'"))?;
        }

        for (tier_label, target) in [
            ("routing.tier_fast", self.routing.tier_fast.as_deref()),
            ("routing.tier_deep", self.routing.tier_deep.as_deref()),
            (
                "routing.tier_crosscheck",
                self.routing.tier_crosscheck.as_deref(),
            ),
        ] {
            if let Some(name) = target {
                if !self.providers.contains_key(name) {
                    bail!("{tier_label} references undefined provider '{name}'");
                }
            }
        }

        if self.agent.mode != AgentMode::Off
            && self.routing.tier_fast.is_none()
            && self.routing.tier_deep.is_none()
        {
            bail!(
                "agent.mode {:?} requires at least routing.tier_fast or routing.tier_deep",
                self.agent.mode
            );
        }

        Ok(())
    }
}

fn is_metadata_host(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    matches!(
        h.as_str(),
        "metadata.google.internal" | "metadata.goog" | "metadata" | "instance-data"
    )
}

fn is_metadata_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 169 && o[1] == 254
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                let o = v4.octets();
                return o[0] == 169 && o[1] == 254;
            }
            let s = v6.segments();
            // Link-local fe80::/10 or AWS IPv6 metadata fd00:ec2::254
            (s[0] & 0xffc0) == 0xfe80 || s == [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254]
        }
    }
}

fn is_private_or_special_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || (o[0] == 100 && (64..=127).contains(&o[1]))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_multicast()
                    || (v4.octets()[0] == 100 && (64..=127).contains(&v4.octets()[1]));
            }
            v6.is_unspecified() || v6.is_multicast() || ((v6.segments()[0] & 0xfe00) == 0xfc00)
        }
    }
}

fn is_internal_creg_port(port: u16) -> bool {
    if port == 50051 {
        return true;
    }
    for env_key in ["CREG_PORT", "CREG_ADMIN_PORT", "CREG_GRPC_PORT"] {
        if let Ok(raw) = std::env::var(env_key) {
            if raw.trim().parse::<u16>().ok() == Some(port) {
                return true;
            }
        }
    }
    false
}

/// Strict SSRF guard for operator-configured LLM endpoints.
pub fn validate_provider_url(
    raw_url: &str,
    kind: ProviderKind,
    allow_private_network: bool,
    allow_test_http: bool,
) -> Result<url::Url> {
    let url = url::Url::parse(raw_url.trim())
        .with_context(|| format!("invalid provider base_url '{raw_url}'"))?;
    if !url.username().is_empty() || url.password().is_some() {
        bail!("provider base_url must not embed credentials");
    }
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        bail!("unsupported provider URL scheme '{scheme}'");
    }
    let host_raw = url
        .host_str()
        .context("provider base_url missing host")?
        .trim_start_matches('[')
        .trim_end_matches(']');

    if is_metadata_host(host_raw) {
        bail!("provider base_url targets blocked cloud metadata host '{host_raw}'");
    }

    let port = url.port_or_known_default().unwrap_or(0);
    if is_internal_creg_port(port) {
        bail!("provider base_url targets blocked internal Chain Registry port {port}");
    }

    let is_localhost_name = host_raw.eq_ignore_ascii_case("localhost")
        || host_raw.to_ascii_lowercase().ends_with(".localhost");

    if let Ok(ip) = host_raw.parse::<IpAddr>() {
        if is_metadata_ip(ip) {
            bail!("provider base_url targets blocked link-local/metadata IP {ip}");
        }
        let is_loopback = ip.is_loopback();
        if !kind.is_local() && !allow_test_http && (is_loopback || is_private_or_special_ip(ip)) {
            bail!("cloud provider base_url cannot target private or loopback IP {ip}");
        }
        if kind.is_local() && !is_loopback && is_private_or_special_ip(ip) && !allow_private_network
        {
            bail!(
                "local provider base_url targeting RFC1918/private IP {ip} requires allow_private_network = true"
            );
        }
    } else if !kind.is_local() && !allow_test_http && is_localhost_name {
        bail!("cloud provider base_url cannot target localhost");
    }

    if !kind.is_local() && !allow_test_http && scheme != "https" {
        bail!("cloud LLM provider requires HTTPS base_url");
    }

    Ok(url)
}

/// Classify a validated provider endpoint URL as `Loopback`, `Lan`, or `Api`.
pub fn classify_endpoint(url: &url::Url) -> EndpointClass {
    let host_raw = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']');
    if host_raw.eq_ignore_ascii_case("localhost")
        || host_raw.to_ascii_lowercase().ends_with(".localhost")
    {
        return EndpointClass::Loopback;
    }
    if let Ok(ip) = host_raw.parse::<IpAddr>() {
        if ip.is_loopback() {
            return EndpointClass::Loopback;
        }
        if is_private_or_special_ip(ip) {
            return EndpointClass::Lan;
        }
        return EndpointClass::Api;
    }
    let lower = host_raw.trim_end_matches('.').to_ascii_lowercase();
    if lower.ends_with(".local") || lower.ends_with(".internal") || lower.ends_with(".lan") {
        EndpointClass::Lan
    } else {
        EndpointClass::Api
    }
}

/// Validate a provider URL against SSRF policy and return its [`EndpointClass`].
pub fn validate_and_classify_provider_url(
    raw_url: &str,
    kind: ProviderKind,
    allow_private_network: bool,
    allow_test_http: bool,
) -> Result<(url::Url, EndpointClass)> {
    let url = validate_provider_url(raw_url, kind, allow_private_network, allow_test_http)?;
    let class = classify_endpoint(&url);
    Ok((url, class))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn secret_ref_accepts_env_vault_file_and_rejects_inline() {
        assert!(matches!(
            parse_secret_ref("env:GEMINI_API_KEY").unwrap(),
            SecretRefKind::Env(v) if v == "GEMINI_API_KEY"
        ));
        assert!(matches!(
            parse_secret_ref("vault:secret/data/creg/ai#api_key").unwrap(),
            SecretRefKind::Vault { path, field } if path == "secret/data/creg/ai" && field == "api_key"
        ));
        let abs = std::env::temp_dir().join("creg-test-secret.key");
        let file_ref = format!("file:{}", abs.display());
        assert!(matches!(
            parse_secret_ref(&file_ref),
            Ok(SecretRefKind::File(_))
        ));

        // Reject inline keys, relative paths, and parent-dir traversal.
        assert!(parse_secret_ref("AIzaInlineNotAllowed").is_err());
        assert!(parse_secret_ref("file:relative/secret.key").is_err());
        let traversal = std::env::temp_dir().join("..").join("secret.key");
        assert!(parse_secret_ref(&format!("file:{}", traversal.display())).is_err());
    }

    #[test]
    fn file_secret_enforces_size_nonempty_and_unix_mode() {
        let dir = tempfile::tempdir().unwrap();
        let ok_path = dir.path().join("api-key.txt");
        {
            let mut f = std::fs::File::create(&ok_path).unwrap();
            writeln!(f, "  sk-super-secret-value  ").unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ok_path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(check_secret_file(&ok_path).is_err());
            std::fs::set_permissions(&ok_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let secret = read_secret_file_zeroized(&ok_path).unwrap();
        assert_eq!(secret.as_str(), "sk-super-secret-value");

        let oversized = dir.path().join("too-big.txt");
        std::fs::write(&oversized, vec![b'a'; (MAX_SECRET_FILE_BYTES as usize) + 1]).unwrap();
        assert!(check_secret_file(&oversized).is_err());

        let empty = dir.path().join("empty.txt");
        std::fs::write(&empty, b"   \n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&empty, std::fs::Permissions::from_mode(0o600));
        }
        assert!(check_secret_file_non_empty(&empty).is_err());

        #[cfg(unix)]
        {
            let link_path = dir.path().join("sym-key.txt");
            std::os::unix::fs::symlink(&ok_path, &link_path).unwrap();
            assert!(check_secret_file(&link_path).is_err());
        }
    }

    #[test]
    fn ssrf_guard_and_endpoint_classifier_work_as_specified() {
        let (_, cls_loop) = validate_and_classify_provider_url(
            "http://127.0.0.1:11434",
            ProviderKind::Ollama,
            false,
            false,
        )
        .unwrap();
        assert_eq!(cls_loop, EndpointClass::Loopback);

        let (_, cls_lan) = validate_and_classify_provider_url(
            "http://10.0.0.5:11434",
            ProviderKind::Ollama,
            true,
            false,
        )
        .unwrap();
        assert_eq!(cls_lan, EndpointClass::Lan);

        let (_, cls_api) = validate_and_classify_provider_url(
            "https://generativelanguage.googleapis.com",
            ProviderKind::Gemini,
            false,
            false,
        )
        .unwrap();
        assert_eq!(cls_api, EndpointClass::Api);

        assert!(validate_provider_url(
            "http://169.254.169.254/latest",
            ProviderKind::Ollama,
            true,
            false
        )
        .is_err());
        assert!(validate_provider_url(
            "http://127.0.0.1:50051/v1",
            ProviderKind::Ollama,
            true,
            false
        )
        .is_err());
        assert!(validate_provider_url(
            "http://generativelanguage.googleapis.com",
            ProviderKind::Gemini,
            false,
            false
        )
        .is_err());
    }

    #[test]
    fn check_ai_config_reports_diversity_advisory_and_never_leaks_secret_values() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let secret_file = dir.path().join("gemini.key");
        let sentinel_secret = "AIzaSySentinelSecretMustNotAppearInReport";
        std::fs::write(&secret_file, sentinel_secret).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&secret_file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }

        let single_family_toml = format!(
            r#"
[agent]
mode = "hybrid_escalate"

[providers.gemini]
kind = "gemini"
model = "gemini-2.5-pro"
api_key_secret_ref = "file:{}"

[routing]
tier_fast = "gemini"
tier_deep = "gemini"
"#,
            secret_file.display().to_string().replace('\\', "\\\\")
        );
        let cfg = ValidatorAiConfig::from_toml_str(&single_family_toml).unwrap();
        let report = check_ai_config(&cfg, Some("validator-ai.toml".into()), false);
        assert!(report.ok);
        assert_eq!(report.distinct_model_families, 1);
        assert!(report.diversity_advisory.is_some());
        assert_eq!(report.providers[0].endpoint_class, Some(EndpointClass::Api));
        assert_eq!(report.providers[0].secret_scheme.as_deref(), Some("file"));
        assert!(report.providers[0].secret_ok);

        let serialized = serde_json::to_string(&report).unwrap();
        assert!(!serialized.contains(sentinel_secret));

        let multi_family_toml = r#"
[agent]
mode = "hybrid_escalate"

[providers.local_triage]
kind = "ollama"
model = "qwen2.5-coder:14b"

[providers.gemini_deep]
kind = "gemini"
model = "gemini-2.5-pro"
api_key_secret_ref = "env:CREG_AI_CONFIG_TEST_KEY"

[routing]
tier_fast = "local_triage"
tier_deep = "gemini_deep"
"#;
        std::env::set_var("CREG_AI_CONFIG_TEST_KEY", "test-key-value");
        let multi_cfg = ValidatorAiConfig::from_toml_str(multi_family_toml).unwrap();
        let multi_report = check_ai_config(&multi_cfg, None, false);
        std::env::remove_var("CREG_AI_CONFIG_TEST_KEY");
        assert!(multi_report.ok);
        assert_eq!(multi_report.distinct_model_families, 2);
        assert!(multi_report.diversity_advisory.is_none());
    }
}
