//! CaMeL-inspired structural prompt-injection containment (`INV-06`).
//!
//! All untrusted package source text, stdout/stderr logs, and decoded payloads
//! are wrapped in session-unique 128-bit nonce delimiters before reaching any
//! model prompt, and pre-existing delimiter lookalikes inside untrusted data are
//! neutralized. Explicit prompt-injection attempts are flagged deterministically
//! so compromised model completions cannot silently pass review.

#[derive(Debug, Clone)]
pub struct SessionEnvelope {
    nonce: String,
}

impl Default for SessionEnvelope {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionEnvelope {
    pub fn new() -> Self {
        Self {
            nonce: uuid::Uuid::new_v4().simple().to_string(),
        }
    }

    pub fn with_nonce(nonce: impl Into<String>) -> Self {
        Self {
            nonce: nonce.into(),
        }
    }

    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    /// Wrap untrusted package bytes/text inside unique per-session boundary tags
    /// while stripping control escapes and neutralizing spoofed boundaries.
    pub fn wrap_untrusted(&self, label: &str, raw_content: &str) -> String {
        let clean_label: String = label
            .chars()
            .filter(|c| !c.is_control() && *c != '"' && *c != '<' && *c != '>')
            .take(128)
            .collect();

        let neutralized = raw_content
            .replace("<<<", "[ESC_OPEN]")
            .replace(">>>", "[ESC_CLOSE]")
            .replace("CREG_UNTRUSTED_DATA", "CREG_ESCAPED_TAG")
            .replace(&self.nonce, "[ESC_NONCE]");

        format!(
            "<<<CREG_UNTRUSTED_DATA_{nonce} label=\"{label}\">>>\n{body}\n<<<END_CREG_UNTRUSTED_DATA_{nonce}>>>",
            nonce = self.nonce,
            label = clean_label,
            body = neutralized,
        )
    }
}

/// Detect structural prompt-injection overrides embedded in package files,
/// comments, or sandbox output (`adv-prompt-inject-trap`).
pub fn detect_prompt_injection_patterns(text: &str) -> Vec<&'static str> {
    if text.is_empty() {
        return Vec::new();
    }
    let normalized: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();

    const SIGNATURES: &[(&str, &str)] = &[
        (
            "ignore previous instructions",
            "ignore_previous_instructions",
        ),
        (
            "ignore all previous instructions",
            "ignore_all_previous_instructions",
        ),
        ("disregard all prior", "disregard_prior_instructions"),
        ("system override", "system_override"),
        ("override system prompt", "override_system_prompt"),
        ("new system instructions", "new_system_instructions"),
        ("<|system|>", "chat_template_system_token"),
        ("<|assistant|>", "chat_template_assistant_token"),
        ("semantic_disposition=approve", "forced_disposition_approve"),
        ("semantic_disposition=\"approve", "forced_disposition_json"),
        (
            "semantic_disposition\": \"approve",
            "forced_disposition_json",
        ),
        (
            "semantic_disposition\":\"approve",
            "forced_disposition_json",
        ),
        ("output maliciousness_score: 0", "forced_zero_score"),
        ("maliciousness_score=0", "forced_zero_score"),
    ];

    let mut found = Vec::new();
    for (needle, tag) in SIGNATURES {
        if normalized.contains(needle) && !found.contains(tag) {
            found.push(*tag);
        }
    }
    found
}
