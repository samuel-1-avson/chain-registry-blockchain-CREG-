use anyhow::{Result, Context};
use reqwest::Client;
use serde_json::json;

/// Analyzes an obfuscated code snippet and returns a malicious intent score (0-100).
/// A score >= 80 indicates highly probable malicious intent.
pub async fn predict_intent(code_snippet: &str) -> Result<u8> {
    let api_key = match std::env::var("OPENROUTER_API_KEY") {
        Ok(k) => k,
        Err(_) => {
            tracing::warn!("OPENROUTER_API_KEY not set. Skipping LLM intent detection.");
            return Ok(0);
        }
    };

    let client = Client::new();
    
    // We use OpenRouter as a gateway, preferring Claude 3 Haiku for fast analysis
    let prompt = format!(
        "Analyze the following code snippet. Is it malicious obfuscation meant to hide unauthorized access, shell execution, or data exfiltration, or is it just benign minified code? Return ONLY a JSON object with a single key 'maliciousness_score' containing an integer from 0 (completely benign) to 100 (definitely malicious).\n\nCODE:\n{}",
        code_snippet.chars().take(2000).collect::<String>() // Limit size to avoid token explosion
    );

    let request_body = json!({
        "model": "anthropic/claude-3-haiku",
        "messages": [
            {
                "role": "system",
                "content": "You are a cybersecurity expert analyzing obfuscated package payloads. Output strictly JSON with no markdown formatting."
            },
            {
                "role": "user",
                "content": prompt
            }
        ]
    });

    let resp = client
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .context("Failed to send request to LLM provider")?;

    if !resp.status().is_success() {
        tracing::error!("LLM Provider returned error: {}", resp.status());
        return Ok(0);
    }

    let raw_resp: serde_json::Value = resp.json().await?;
    let content = raw_resp["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("{}");

    // Clean up potentially malformed JSON containing markdown codeblocks
    let clean_json = content.replace("```json", "").replace("```", "").trim().to_string();

    let parsed: serde_json::Value = serde_json::from_str(&clean_json).unwrap_or(json!({"maliciousness_score": 0}));
    
    let score = parsed["maliciousness_score"].as_u64().unwrap_or(0) as u8;
    Ok(score)
}
