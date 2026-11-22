//! Validator AI configuration (`validator-ai.toml`) supporting local LLM
//! runtimes (`Ollama`, `vLLM`, `llama.cpp`, `SGLang`, `LM Studio`) and cloud
//! API providers (`Gemini`, `Anthropic`, `OpenAI`, `OpenRouter`).

use super::proxy::validate_provider_url;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

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
    25.0
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
    /// Must use `env:<VAR_NAME>` or `vault:<path>#<field>`. Inline keys rejected.
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
        if !common::parse_env_bool("CREG_LLM_ENABLED") {
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
                "openrouter" => {
                    if std::env::var("OPENROUTER_API_KEY").is_ok_and(|k| !k.trim().is_empty()) {
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
                    "cloud provider '{name}' must configure api_key_secret_ref (env:... or vault:...)"
                );
            }
            if let Some(ref secret_ref) = profile.api_key_secret_ref {
                if !secret_ref.starts_with("env:") && !secret_ref.starts_with("vault:") {
                    bail!(
                        "provider '{name}' api_key_secret_ref must start with 'env:' or 'vault:' (inline secrets forbidden)"
                    );
                }
            }
            validate_provider_url(
                profile.resolved_base_url(),
                profile.kind,
                profile.allow_private_network,
                false,
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

fn env_usize(key: &str) -> Option<usize> {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok())
}
