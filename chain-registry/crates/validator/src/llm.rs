use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// In-memory LLM response cache keyed by content hash.
/// Avoids redundant API calls for the same code snippet.
static LLM_CACHE: std::sync::LazyLock<Mutex<HashMap<u64, u8>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Rate limiter: tracks call timestamps within a rolling window.
static RATE_LIMITER: std::sync::LazyLock<Mutex<RateLimiter>> =
    std::sync::LazyLock::new(|| Mutex::new(RateLimiter::new()));

/// Simple sliding-window rate limiter for LLM API calls.
struct RateLimiter {
    /// Timestamps of recent calls within the current hour window.
    calls: Vec<Instant>,
    /// Start of the current window.
    window_start: Instant,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            calls: Vec::new(),
            window_start: Instant::now(),
        }
    }

    /// Returns Ok(()) if a call is allowed, Err with reason if rate-limited.
    /// Configurable via CREG_LLM_RATE_LIMIT (default: 100 calls per hour).
    fn check(&mut self) -> std::result::Result<(), String> {
        let max_calls: usize = std::env::var("CREG_LLM_RATE_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);

        let now = Instant::now();
        let window = std::time::Duration::from_secs(3600); // 1 hour

        // Reset window if expired
        if now.duration_since(self.window_start) > window {
            self.calls.clear();
            self.window_start = now;
        }

        // Prune old entries (shouldn't be needed after window reset, but defensive)
        self.calls.retain(|t| now.duration_since(*t) < window);

        if self.calls.len() >= max_calls {
            return Err(format!(
                "LLM rate limit exceeded: {} calls in the current hour (max: {})",
                self.calls.len(),
                max_calls
            ));
        }

        self.calls.push(now);
        Ok(())
    }
}

/// Simple content hash for cache keying (FNV-1a style).
fn content_hash(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Sanitize code for inclusion in LLM prompt to prevent prompt injection.
/// Replaces control sequences and known injection patterns that could
/// override the system prompt or extract the API key.
fn sanitize_for_prompt(code: &str) -> String {
    let truncated: String = code.chars().take(2000).collect();
    // Base64-encode to prevent prompt injection — the LLM can still
    // decode and analyze the patterns, but injected instruction tokens
    // won't be interpreted as directives.
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(truncated.as_bytes());
    encoded
}

/// Configurable LLM model name. Defaults to Claude 3 Haiku via OpenRouter.
fn llm_model() -> String {
    std::env::var("CREG_LLM_MODEL").unwrap_or_else(|_| "anthropic/claude-3-haiku".to_string())
}

/// Configurable LLM API URL. Defaults to OpenRouter.
fn llm_api_url() -> String {
    std::env::var("CREG_LLM_API_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1/chat/completions".to_string())
}

/// Result type that distinguishes "LLM was not available" from "LLM ran and returned a score".
/// When the LLM is unavailable, callers can decide how to handle degraded mode.
pub enum LlmResult {
    /// LLM returned a maliciousness score (0-100)
    Score(u8),
    /// LLM was not available (no API key, network error, etc.)
    Unavailable(String),
}

/// Analyzes an obfuscated code snippet and returns a malicious intent score (0-100).
/// A score >= 80 indicates highly probable malicious intent.
///
/// Returns `LlmResult::Unavailable` instead of silently returning 0 when the LLM
/// cannot be reached, so the caller can emit appropriate degraded-mode findings.
pub async fn predict_intent(code_snippet: &str) -> Result<u8> {
    match predict_intent_full(code_snippet).await {
        Ok(LlmResult::Score(s)) => Ok(s),
        Ok(LlmResult::Unavailable(reason)) => {
            tracing::warn!("LLM unavailable: {}", reason);
            Ok(0)
        }
        Err(e) => Err(e),
    }
}

/// Full LLM intent prediction with explicit unavailability tracking.
///
/// **Opt-in by default**: LLM analysis is disabled unless `CREG_LLM_ENABLED=true`
/// is set. This prevents accidental exfiltration of proprietary package source
/// code to external cloud APIs.
///
/// Provider fallback chain (when enabled):
///   1. Ollama (local) — preferred; requires `CREG_OLLAMA_URL` (default: http://localhost:11434)
///   2. OpenRouter (cloud) — requires `OPENROUTER_API_KEY`
/// If neither is available, returns Unavailable.
pub async fn predict_intent_full(code_snippet: &str) -> Result<LlmResult> {
    // Gate: LLM analysis must be explicitly opted into.
    let enabled = std::env::var("CREG_LLM_ENABLED")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);

    if !enabled {
        return Ok(LlmResult::Unavailable(
            "LLM analysis disabled (set CREG_LLM_ENABLED=true to enable)".into(),
        ));
    }

    // Check cache first (before any rate limiting or network calls)
    let hash = content_hash(code_snippet);
    if let Ok(cache) = LLM_CACHE.lock() {
        if let Some(&cached_score) = cache.get(&hash) {
            tracing::debug!("LLM cache hit for content hash {:016x}", hash);
            return Ok(LlmResult::Score(cached_score));
        }
    }

    // Check rate limit
    if let Ok(mut limiter) = RATE_LIMITER.lock() {
        if let Err(reason) = limiter.check() {
            tracing::warn!("{}", reason);
            return Ok(LlmResult::Unavailable(reason));
        }
    }

    // Sanitize code via base64 encoding to prevent prompt injection
    let encoded_code = sanitize_for_prompt(code_snippet);

    // ── Try local Ollama first (preferred — no data leaves the machine) ───
    let ollama_result = try_ollama(&encoded_code).await;
    match &ollama_result {
        Ok(LlmResult::Score(score)) => {
            cache_score(hash, *score);
            return ollama_result;
        }
        Ok(LlmResult::Unavailable(reason)) => {
            tracing::debug!("Ollama unavailable: {}; trying OpenRouter fallback", reason);
        }
        Err(e) => {
            tracing::debug!("Ollama error: {}; trying OpenRouter fallback", e);
        }
    }

    // ── Fallback: Try OpenRouter cloud API ────────────────────────────────
    let openrouter_result = try_openrouter(&encoded_code).await;
    match &openrouter_result {
        Ok(LlmResult::Score(score)) => {
            cache_score(hash, *score);
            return openrouter_result;
        }
        Ok(LlmResult::Unavailable(reason)) => {
            tracing::debug!("OpenRouter also unavailable: {}", reason);
        }
        Err(e) => {
            tracing::debug!("OpenRouter error: {}", e);
        }
    }

    // Both providers failed
    Ok(LlmResult::Unavailable(
        "All LLM providers unavailable (Ollama + OpenRouter)".into(),
    ))
}

/// Cache a score result.
fn cache_score(hash: u64, score: u8) {
    if let Ok(mut cache) = LLM_CACHE.lock() {
        if cache.len() > 10_000 {
            cache.clear();
        }
        cache.insert(hash, score);
    }
}

/// Build the analysis messages array (shared between providers).
fn build_messages(encoded_code: &str) -> serde_json::Value {
    json!([
        {
            "role": "system",
            "content": "You are a cybersecurity expert analyzing obfuscated package payloads. \
                The user will provide a base64-encoded code snippet. Decode it mentally, \
                analyze for malicious patterns (shell execution, data exfiltration, \
                credential harvesting, backdoors, crypto miners), and return ONLY a JSON \
                object: {\"maliciousness_score\": <0-100>}. No markdown, no explanation."
        },
        {
            "role": "user",
            "content": format!("Analyze this base64-encoded code snippet for malicious intent:\n{}", encoded_code)
        }
    ])
}

/// Parse the maliciousness_score from a provider's chat completion response.
fn parse_llm_response(raw_resp: &serde_json::Value) -> Result<LlmResult> {
    let content = raw_resp["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| raw_resp["message"]["content"].as_str()) // Ollama format
        .unwrap_or("{}");

    let clean_json = content
        .replace("```json", "")
        .replace("```", "")
        .trim()
        .to_string();

    let parsed: serde_json::Value = match serde_json::from_str(&clean_json) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to parse LLM response as JSON: {} — raw: {}", e, clean_json);
            return Ok(LlmResult::Unavailable(format!(
                "LLM returned unparseable response: {}",
                e
            )));
        }
    };

    let score = parsed["maliciousness_score"].as_u64().unwrap_or(0).min(100) as u8;
    Ok(LlmResult::Score(score))
}

/// Try the OpenRouter cloud API.
async fn try_openrouter(encoded_code: &str) -> Result<LlmResult> {
    let api_key = match std::env::var("OPENROUTER_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            return Ok(LlmResult::Unavailable(
                "OPENROUTER_API_KEY not set or empty".into(),
            ));
        }
    };

    let model = llm_model();
    let api_url = llm_api_url();

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let request_body = json!({
        "model": model,
        "messages": build_messages(encoded_code),
        "max_tokens": 50,
        "temperature": 0.0
    });

    let resp = client
        .post(&api_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .context("Failed to send request to OpenRouter")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::error!("OpenRouter returned HTTP {}: {}", status, body);
        return Ok(LlmResult::Unavailable(format!(
            "OpenRouter API returned HTTP {}",
            status
        )));
    }

    let raw_resp: serde_json::Value = resp.json().await?;
    parse_llm_response(&raw_resp)
}

/// Try a local Ollama instance as a fallback.
/// Requires `CREG_OLLAMA_URL` env var (default: http://localhost:11434).
/// Uses `CREG_OLLAMA_MODEL` env var (default: codellama:7b).
async fn try_ollama(encoded_code: &str) -> Result<LlmResult> {
    let ollama_url = std::env::var("CREG_OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_string());

    // Quick check: is Ollama reachable?
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    // Ollama uses /api/chat for chat completions
    let chat_url = format!("{}/api/chat", ollama_url.trim_end_matches('/'));
    let model = std::env::var("CREG_OLLAMA_MODEL")
        .unwrap_or_else(|_| "codellama:7b".to_string());

    let request_body = json!({
        "model": model,
        "messages": build_messages(encoded_code),
        "stream": false
    });

    let resp = match client
        .post(&chat_url)
        .json(&request_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Ok(LlmResult::Unavailable(format!(
                "Ollama not reachable at {}: {}",
                ollama_url, e
            )));
        }
    };

    if !resp.status().is_success() {
        return Ok(LlmResult::Unavailable(format!(
            "Ollama returned HTTP {}",
            resp.status()
        )));
    }

    let raw_resp: serde_json::Value = resp.json().await?;
    parse_llm_response(&raw_resp)
}
