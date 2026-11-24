//! Offline static diagnostics for `ValidatorAiConfig` (`creg ai check`, ADR-019 §4.3).

use crate::secret::{check_secret_file_non_empty, parse_secret_ref, SecretRefKind};
use crate::{validate_and_classify_provider_url, AgentMode, EndpointClass, ValidatorAiConfig};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Minimum recommended distinct model families when AI mode is active
/// (`ADR-019 §4.3`; emitted as a warning/advisory, never a hard failure).
pub const RECOMMENDED_MIN_MODEL_FAMILIES: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProviderCheckV1 {
    pub name: String,
    pub kind: crate::ProviderKind,
    pub model: String,
    pub model_family: String,
    pub base_url: String,
    pub endpoint_class: Option<EndpointClass>,
    pub url_ok: bool,
    pub secret_scheme: Option<String>,
    pub secret_ok: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConfigCheckReportV1 {
    pub schema_version: u16,
    pub config_path: Option<String>,
    pub mode: AgentMode,
    pub ok: bool,
    pub distinct_model_families: usize,
    pub diversity_advisory: Option<String>,
    pub providers: Vec<AiProviderCheckV1>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Run offline validation on a [`ValidatorAiConfig`]:
/// - Validates policy bounds, routing references, SSRF URL rules, and URL endpoint classification.
/// - Verifies `api_key_secret_ref` syntax, env presence, or owner-only `file:` readability without
///   ever placing secret values into the report.
/// - Emits a single-family diversity advisory (warning only) if fewer than 2 distinct model
///   families are configured while `agent.mode != Off`.
pub fn check_ai_config(
    cfg: &ValidatorAiConfig,
    config_path: Option<String>,
    allow_test_http: bool,
) -> AiConfigCheckReportV1 {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if let Err(err) = cfg.validate_with_options(allow_test_http) {
        errors.push(format!("{err:#}"));
    }

    let mut providers = Vec::with_capacity(cfg.providers.len());
    let mut model_families = BTreeSet::new();

    for (name, profile) in &cfg.providers {
        let model_family = profile.resolved_model_family();
        model_families.insert(model_family.clone());

        let base_url = profile.resolved_base_url().to_string();
        let mut issues = Vec::new();
        let (endpoint_class, url_ok) = match validate_and_classify_provider_url(
            &base_url,
            profile.kind,
            profile.allow_private_network,
            allow_test_http,
        ) {
            Ok((_url, class)) => (Some(class), true),
            Err(err) => {
                issues.push(format!("URL validation failed: {err}"));
                (None, false)
            }
        };

        let (secret_scheme, secret_ok) = match &profile.api_key_secret_ref {
            None => {
                if profile.kind.requires_api_key() {
                    issues.push(format!(
                        "cloud provider '{name}' requires api_key_secret_ref (env:/vault:/file:)"
                    ));
                    (None, false)
                } else {
                    (None, true)
                }
            }
            Some(raw_ref) => match parse_secret_ref(raw_ref) {
                Err(err) => {
                    issues.push(format!("secret reference invalid: {err}"));
                    (None, false)
                }
                Ok(parsed) => {
                    let scheme = Some(parsed.scheme().to_string());
                    match parsed {
                        SecretRefKind::Env(var_name) => {
                            let present = std::env::var(&var_name)
                                .ok()
                                .is_some_and(|v| !v.trim().is_empty());
                            if !present {
                                issues.push(format!(
                                    "environment variable '{var_name}' referenced by provider '{name}' is unset or empty"
                                ));
                            }
                            (scheme, present)
                        }
                        SecretRefKind::File(path) => match check_secret_file_non_empty(&path) {
                            Ok(()) => (scheme, true),
                            Err(err) => {
                                issues.push(format!("secret file unusable: {err}"));
                                (scheme, false)
                            }
                        },
                        SecretRefKind::Vault { .. } => {
                            let vault_backend =
                                std::env::var("CREG_SECRETS_BACKEND").ok().is_some_and(|v| {
                                    matches!(
                                        v.trim().to_ascii_lowercase().as_str(),
                                        "vault" | "hashicorp" | "hashicorp-vault"
                                    )
                                });
                            if !vault_backend {
                                warnings.push(format!(
                                    "provider '{name}' uses vault: secret_ref, but CREG_SECRETS_BACKEND is not set to 'vault'"
                                ));
                            }
                            if std::env::var("VAULT_ADDR")
                                .ok()
                                .is_none_or(|v| v.trim().is_empty())
                            {
                                warnings.push(format!(
                                    "provider '{name}' uses vault: secret_ref, but VAULT_ADDR is unset"
                                ));
                            }
                            (scheme, true)
                        }
                    }
                }
            },
        };

        providers.push(AiProviderCheckV1 {
            name: name.clone(),
            kind: profile.kind,
            model: profile.model.clone(),
            model_family,
            base_url,
            endpoint_class,
            url_ok,
            secret_scheme,
            secret_ok,
            issues,
        });
    }

    let distinct_model_families = model_families.len();
    let diversity_advisory = if cfg.agent.mode != AgentMode::Off
        && distinct_model_families < RECOMMENDED_MIN_MODEL_FAMILIES
    {
        let msg = format!(
            "Model-family diversity advisory: {} distinct model_family configured (recommended >= {} across fast/deep tiers)",
            distinct_model_families, RECOMMENDED_MIN_MODEL_FAMILIES
        );
        warnings.push(msg.clone());
        Some(msg)
    } else {
        None
    };

    let ok = errors.is_empty()
        && providers
            .iter()
            .all(|p| p.url_ok && p.secret_ok && p.issues.is_empty());

    AiConfigCheckReportV1 {
        schema_version: 1,
        config_path,
        mode: cfg.agent.mode,
        ok,
        distinct_model_families,
        diversity_advisory,
        providers,
        errors,
        warnings,
    }
}
