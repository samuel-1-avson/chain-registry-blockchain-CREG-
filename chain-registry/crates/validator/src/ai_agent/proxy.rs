//! Credential-isolated Universal LLM Proxy (`creg-llm-proxy`, ADR-018 Layer 4).
//!
//! Connects any validator-configured local GPU server (`Ollama`, `vLLM`,
//! `llama.cpp`, `SGLang`, `LM Studio`, generic OpenAI-compatible) or cloud
//! API (`Gemini`, `Anthropic`, `OpenAI`, `OpenRouter`) with:
//! - Strict `crates/secrets` `SecretString` custody (`VNA-32`, keys never enter agent prompts)
//! - SSRF IP/port blocklist (`169.254.0.0/16`, `metadata.google.internal`, node RPC/gRPC ports)
//! - Atomic UTC daily USD spend circuit breaker + per-provider concurrency semaphore

use super::config::{ProviderKind, ProviderProfile, ValidatorAiConfig};
use anyhow::{bail, Context, Result};
use chain_registry_secrets::{SecretString, SecretsProvider};
use chrono::Datelike;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{RwLock, Semaphore};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub provider_name: String,
    pub profile: ProviderProfile,
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmResponse {
    pub content: String,
    pub model: String,
    pub model_family: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub trait LlmTransport: Send + Sync {
    fn complete<'a>(
        &'a self,
        request: &'a LlmRequest,
        api_key: Option<&'a SecretString>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse>> + Send + 'a>>;
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
) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw_url.trim())
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

fn current_utc_day_key() -> u32 {
    let now = chrono::Utc::now();
    (now.year() as u32).saturating_mul(400) + now.ordinal()
}

/// Atomic UTC daily USD spend limiter & circuit breaker.
pub struct BudgetGuard {
    day_key: Mutex<u32>,
    spent_micros: AtomicU64,
    max_daily_micros: u64,
}

impl BudgetGuard {
    pub fn from_max_daily_usd(max_usd: f64) -> Self {
        let max_daily_micros = if !max_usd.is_finite() || max_usd <= 0.0 {
            0
        } else {
            (max_usd * 1_000_000.0).min(u64::MAX as f64) as u64
        };
        Self {
            day_key: Mutex::new(current_utc_day_key()),
            spent_micros: AtomicU64::new(0),
            max_daily_micros,
        }
    }

    fn sync_day(&self) {
        let today = current_utc_day_key();
        if let Ok(mut guard) = self.day_key.lock() {
            if *guard != today {
                *guard = today;
                self.spent_micros.store(0, Ordering::Release);
            }
        }
    }

    pub fn check_budget(&self, profile: &ProviderProfile) -> Result<()> {
        self.sync_day();
        let is_free = profile.cost_per_1k_input_usd <= 0.0 && profile.cost_per_1k_output_usd <= 0.0;
        if is_free {
            return Ok(());
        }
        let spent = self.spent_micros.load(Ordering::Acquire);
        if self.max_daily_micros == 0 || spent >= self.max_daily_micros {
            bail!(
                "daily LLM API spend limit exceeded (spent=${:.4}, limit=${:.4})",
                spent as f64 / 1_000_000.0,
                self.max_daily_micros as f64 / 1_000_000.0
            );
        }
        Ok(())
    }

    pub fn record_usage(&self, profile: &ProviderProfile, input_tokens: u64, output_tokens: u64) {
        self.sync_day();
        let usd = (input_tokens as f64 / 1000.0) * profile.cost_per_1k_input_usd.max(0.0)
            + (output_tokens as f64 / 1000.0) * profile.cost_per_1k_output_usd.max(0.0);
        if usd.is_finite() && usd > 0.0 {
            let add_micros = (usd * 1_000_000.0).round().max(1.0) as u64;
            self.spent_micros.fetch_add(add_micros, Ordering::AcqRel);
        }
    }

    pub fn spent_usd(&self) -> f64 {
        self.sync_day();
        self.spent_micros.load(Ordering::Acquire) as f64 / 1_000_000.0
    }
}

pub struct HttpLlmTransport;

pub fn gemini_endpoint(base_url: &str, model: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!("{base}/v1beta/models/{model}:generateContent")
}

pub fn build_gemini_request(req: &LlmRequest) -> serde_json::Value {
    let contents: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            let role = match m.role {
                ChatRole::Assistant => "model",
                ChatRole::System | ChatRole::User => "user",
            };
            json!({
                "role": role,
                "parts": [{"text": m.content}]
            })
        })
        .collect();
    json!({
        "system_instruction": {
            "parts": [{"text": req.system_prompt}]
        },
        "contents": contents,
        "generationConfig": {
            "temperature": 0.0,
            "maxOutputTokens": req.max_tokens,
            "responseMimeType": "application/json"
        }
    })
}

pub fn parse_gemini_response(
    raw: &serde_json::Value,
    model: &str,
    model_family: &str,
) -> Result<LlmResponse> {
    let content = raw["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .context("Gemini response missing candidates[0].content.parts[0].text")?
        .to_string();
    let input_tokens = raw["usageMetadata"]["promptTokenCount"]
        .as_u64()
        .unwrap_or(0);
    let output_tokens = raw["usageMetadata"]["candidatesTokenCount"]
        .as_u64()
        .unwrap_or(0);
    Ok(LlmResponse {
        content,
        model: model.to_string(),
        model_family: model_family.to_string(),
        input_tokens,
        output_tokens,
    })
}

pub fn anthropic_endpoint(base_url: &str) -> String {
    format!("{}/v1/messages", base_url.trim_end_matches('/'))
}

pub fn build_anthropic_request(req: &LlmRequest) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter(|m| m.role != ChatRole::System)
        .map(|m| {
            let role = match m.role {
                ChatRole::Assistant => "assistant",
                ChatRole::User | ChatRole::System => "user",
            };
            json!({ "role": role, "content": m.content })
        })
        .collect();
    json!({
        "model": req.profile.model,
        "max_tokens": req.max_tokens,
        "temperature": 0.0,
        "system": [{
            "type": "text",
            "text": req.system_prompt,
            "cache_control": { "type": "ephemeral" }
        }],
        "messages": messages,
    })
}

pub fn parse_anthropic_response(
    raw: &serde_json::Value,
    model: &str,
    model_family: &str,
) -> Result<LlmResponse> {
    let content = raw["content"][0]["text"]
        .as_str()
        .context("Anthropic response missing content[0].text")?
        .to_string();
    let input_tokens = raw["usage"]["input_tokens"].as_u64().unwrap_or(0);
    let output_tokens = raw["usage"]["output_tokens"].as_u64().unwrap_or(0);
    Ok(LlmResponse {
        content,
        model: model.to_string(),
        model_family: model_family.to_string(),
        input_tokens,
        output_tokens,
    })
}

pub fn openai_compat_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

pub fn build_openai_compat_request(req: &LlmRequest) -> serde_json::Value {
    let mut messages = vec![json!({ "role": "system", "content": req.system_prompt })];
    for m in &req.messages {
        let role = match m.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        };
        messages.push(json!({ "role": role, "content": m.content }));
    }
    json!({
        "model": req.profile.model,
        "messages": messages,
        "max_tokens": req.max_tokens,
        "temperature": 0.0,
        "response_format": { "type": "json_object" }
    })
}

pub fn parse_openai_compat_response(
    raw: &serde_json::Value,
    model: &str,
    model_family: &str,
) -> Result<LlmResponse> {
    let content = raw["choices"][0]["message"]["content"]
        .as_str()
        .context("OpenAI-compatible response missing choices[0].message.content")?
        .to_string();
    let input_tokens = raw["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
    let output_tokens = raw["usage"]["completion_tokens"].as_u64().unwrap_or(0);
    Ok(LlmResponse {
        content,
        model: raw["model"].as_str().unwrap_or(model).to_string(),
        model_family: model_family.to_string(),
        input_tokens,
        output_tokens,
    })
}

pub fn ollama_endpoint(base_url: &str) -> String {
    format!("{}/api/chat", base_url.trim_end_matches('/'))
}

pub fn build_ollama_request(req: &LlmRequest) -> serde_json::Value {
    let mut messages = vec![json!({ "role": "system", "content": req.system_prompt })];
    for m in &req.messages {
        let role = match m.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        };
        messages.push(json!({ "role": role, "content": m.content }));
    }
    json!({
        "model": req.profile.model,
        "messages": messages,
        "stream": false,
        "format": "json",
        "options": {
            "temperature": 0.0,
            "num_predict": req.max_tokens
        }
    })
}

pub fn parse_ollama_response(
    raw: &serde_json::Value,
    model: &str,
    model_family: &str,
) -> Result<LlmResponse> {
    let content = raw["message"]["content"]
        .as_str()
        .context("Ollama response missing message.content")?
        .to_string();
    let input_tokens = raw["prompt_eval_count"].as_u64().unwrap_or(0);
    let output_tokens = raw["eval_count"].as_u64().unwrap_or(0);
    Ok(LlmResponse {
        content,
        model: raw["model"].as_str().unwrap_or(model).to_string(),
        model_family: model_family.to_string(),
        input_tokens,
        output_tokens,
    })
}

impl LlmTransport for HttpLlmTransport {
    fn complete<'a>(
        &'a self,
        req: &'a LlmRequest,
        api_key: Option<&'a SecretString>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse>> + Send + 'a>> {
        Box::pin(async move {
            let client = Client::builder()
                .timeout(Duration::from_secs(req.profile.timeout_secs.max(1)))
                .build()
                .context("build HTTP client")?;
            let base = req.profile.resolved_base_url();
            let family = req.profile.resolved_model_family();
            let model = req.profile.model.as_str();

            match req.profile.kind {
                ProviderKind::Gemini => {
                    let key = api_key.context("Gemini API key missing")?;
                    let url = gemini_endpoint(base, model);
                    let body = build_gemini_request(req);
                    let resp = client
                        .post(&url)
                        .header("x-goog-api-key", key.expose_secret())
                        .json(&body)
                        .send()
                        .await
                        .context("Gemini HTTP request failed")?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        bail!("Gemini HTTP {status}: {}", &text[..text.len().min(200)]);
                    }
                    let raw: serde_json::Value = resp.json().await?;
                    parse_gemini_response(&raw, model, &family)
                }
                ProviderKind::Anthropic => {
                    let key = api_key.context("Anthropic API key missing")?;
                    let url = anthropic_endpoint(base);
                    let body = build_anthropic_request(req);
                    let resp = client
                        .post(&url)
                        .header("x-api-key", key.expose_secret())
                        .header("anthropic-version", "2023-06-01")
                        .json(&body)
                        .send()
                        .await
                        .context("Anthropic HTTP request failed")?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        bail!("Anthropic HTTP {status}: {}", &text[..text.len().min(200)]);
                    }
                    let raw: serde_json::Value = resp.json().await?;
                    parse_anthropic_response(&raw, model, &family)
                }
                ProviderKind::Ollama => {
                    let url = ollama_endpoint(base);
                    let body = build_ollama_request(req);
                    let resp = client
                        .post(&url)
                        .json(&body)
                        .send()
                        .await
                        .context("Ollama HTTP request failed")?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        bail!("Ollama HTTP {status}: {}", &text[..text.len().min(200)]);
                    }
                    let raw: serde_json::Value = resp.json().await?;
                    parse_ollama_response(&raw, model, &family)
                }
                ProviderKind::OpenAi
                | ProviderKind::OpenRouter
                | ProviderKind::Vllm
                | ProviderKind::LlamaCpp
                | ProviderKind::Sglang
                | ProviderKind::LmStudio
                | ProviderKind::OpenAiCompatible => {
                    let url = openai_compat_endpoint(base);
                    let body = build_openai_compat_request(req);
                    let mut builder = client.post(&url).json(&body);
                    if let Some(key) = api_key {
                        builder = builder
                            .header("Authorization", format!("Bearer {}", key.expose_secret()));
                    }
                    let resp = builder
                        .send()
                        .await
                        .context("OpenAI-compatible HTTP request failed")?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        bail!(
                            "OpenAI-compat HTTP {status}: {}",
                            &text[..text.len().min(200)]
                        );
                    }
                    let raw: serde_json::Value = resp.json().await?;
                    parse_openai_compat_response(&raw, model, &family)
                }
            }
        })
    }
}

/// Credential-isolated proxy mediating all agent LLM calls (`INV-06`, `VNA-32`).
pub struct LlmProxy {
    config: ValidatorAiConfig,
    transport: Arc<dyn LlmTransport>,
    secrets: Option<SecretsProvider>,
    resolved_keys: RwLock<BTreeMap<String, SecretString>>,
    semaphores: BTreeMap<String, Arc<Semaphore>>,
    budget: BudgetGuard,
}

impl LlmProxy {
    pub fn new(config: ValidatorAiConfig, transport: Arc<dyn LlmTransport>) -> Self {
        let semaphores = config
            .providers
            .iter()
            .map(|(name, profile)| {
                (
                    name.clone(),
                    Arc::new(Semaphore::new(profile.max_concurrent_requests.max(1))),
                )
            })
            .collect();
        let secrets = SecretsProvider::from_env().ok();
        let budget = BudgetGuard::from_max_daily_usd(config.agent.max_daily_api_spend_usd);
        Self {
            config,
            transport,
            secrets,
            resolved_keys: RwLock::new(BTreeMap::new()),
            semaphores,
            budget,
        }
    }

    pub fn new_http(config: ValidatorAiConfig) -> Self {
        Self::new(config, Arc::new(HttpLlmTransport))
    }

    pub fn config(&self) -> &ValidatorAiConfig {
        &self.config
    }

    pub fn spent_today_usd(&self) -> f64 {
        self.budget.spent_usd()
    }

    async fn resolve_key_for(
        &self,
        name: &str,
        profile: &ProviderProfile,
    ) -> Result<Option<SecretString>> {
        let Some(ref secret_ref) = profile.api_key_secret_ref else {
            return Ok(None);
        };
        if let Some(cached) = self.resolved_keys.read().await.get(name) {
            return Ok(Some(cached.clone()));
        }
        let secrets = self
            .secrets
            .as_ref()
            .context("SecretsProvider unavailable to resolve api_key_secret_ref")?;
        let key = secrets.resolve_secret_ref(secret_ref).await?;
        self.resolved_keys
            .write()
            .await
            .insert(name.to_string(), key.clone());
        Ok(Some(key))
    }

    pub async fn complete(
        &self,
        provider_name: &str,
        system_prompt: &str,
        messages: Vec<ChatMessage>,
        max_tokens: u32,
    ) -> Result<LlmResponse> {
        let profile = self
            .config
            .providers
            .get(provider_name)
            .with_context(|| format!("unknown LLM provider '{provider_name}'"))?;
        self.budget.check_budget(profile)?;

        let semaphore = self
            .semaphores
            .get(provider_name)
            .context("missing semaphore for provider")?;
        let _permit = semaphore
            .acquire()
            .await
            .context("provider semaphore closed")?;

        let api_key = self.resolve_key_for(provider_name, profile).await?;
        let req = LlmRequest {
            provider_name: provider_name.to_string(),
            profile: profile.clone(),
            system_prompt: system_prompt.to_string(),
            messages,
            max_tokens,
        };
        let resp = self.transport.complete(&req, api_key.as_ref()).await?;
        self.budget
            .record_usage(profile, resp.input_tokens, resp.output_tokens);
        Ok(resp)
    }
}
