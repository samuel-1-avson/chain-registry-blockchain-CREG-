// crates/resolver/src/install_gate.rs
// Shared install admission for `creg install` and PATH shims (SHIM-301).
//
// Default is fail-closed: unverified, unknown, revoked, stale cache, high-
// severity findings, and an unreachable node all refuse. `--unverified` or
// CREG_ALLOW_UNVERIFIED is the only break-glass, and it still cannot override
// an explicit revocation.

use crate::chain_client;
use crate::{cache, parse_pkg};
use anyhow::{bail, Result};
use chrono::{DateTime, Duration, Utc};
use common::{FindingSeverity, PackageId, TrustVerdict, VerdictSource, VerdictStatus};

pub const ALLOW_UNVERIFIED_ENV: &str = "CREG_ALLOW_UNVERIFIED";
pub const INSTALL_CACHE_MAX_AGE: Duration = Duration::minutes(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallDecision {
    Allow,
    AllowUnverified,
    Block { code: &'static str, message: String },
}

impl InstallDecision {
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Block { .. })
    }
}

pub fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            let trimmed = value.trim();
            trimmed == "1"
                || trimmed.eq_ignore_ascii_case("true")
                || trimmed.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

pub fn env_allow_unverified() -> bool {
    env_flag(ALLOW_UNVERIFIED_ENV)
}

pub fn args_allow_unverified(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--unverified")
}

pub fn shim_allows_unverified(args: &[String]) -> bool {
    args_allow_unverified(args) || env_allow_unverified()
}

pub fn requirement_file_requested(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg == "-r"
            || arg == "--requirement"
            || arg == "--requirements"
            || arg.starts_with("--requirement=")
            || arg.starts_with("--requirements=")
    })
}

pub fn strip_unverified_flag(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|arg| *arg != "--unverified")
        .cloned()
        .collect()
}

pub fn install_cache_usable(verdict: &TrustVerdict, now: DateTime<Utc>) -> bool {
    if matches!(verdict.status, VerdictStatus::Revoked { .. }) {
        return true;
    }
    let age = now.signed_duration_since(verdict.resolved_at);
    if age > INSTALL_CACHE_MAX_AGE || age < Duration::zero() {
        return false;
    }
    match &verdict.source {
        VerdictSource::Cache { expires_at } => now <= *expires_at,
        VerdictSource::Chain { .. } => true,
    }
}

pub fn evaluate_install(verdict: &TrustVerdict, allow_unverified: bool) -> InstallDecision {
    match &verdict.status {
        VerdictStatus::Revoked { reason, .. } => InstallDecision::Block {
            code: "revoked",
            message: format!(
                "Package '{}' is REVOKED and cannot be installed. Reason: {reason}",
                verdict.package.canonical()
            ),
        },
        VerdictStatus::Verified { findings, .. } => {
            let has_severe = findings.iter().any(|finding| {
                matches!(
                    finding.severity,
                    FindingSeverity::Critical | FindingSeverity::High
                )
            });
            if has_severe && !allow_unverified {
                InstallDecision::Block {
                    code: "high_severity",
                    message: format!(
                        "Package '{}' has high-severity security findings. Refusing installation; use --unverified or {ALLOW_UNVERIFIED_ENV}=true only as an explicit break-glass exception.",
                        verdict.package.canonical()
                    ),
                }
            } else if has_severe {
                InstallDecision::AllowUnverified
            } else {
                InstallDecision::Allow
            }
        }
        VerdictStatus::Unverified => {
            if allow_unverified {
                InstallDecision::AllowUnverified
            } else {
                InstallDecision::Block {
                    code: "unverified",
                    message: format!(
                        "Package '{}' is not qualified. Refusing installation; use --unverified or {ALLOW_UNVERIFIED_ENV}=true only as an explicit break-glass exception.",
                        verdict.package.canonical()
                    ),
                }
            }
        }
        VerdictStatus::Unknown => {
            if allow_unverified {
                InstallDecision::AllowUnverified
            } else {
                InstallDecision::Block {
                    code: "unknown",
                    message: format!(
                        "Package '{}' is unknown to the chain registry. Use --unverified or {ALLOW_UNVERIFIED_ENV}=true to install from the original registry.",
                        verdict.package.canonical()
                    ),
                }
            }
        }
    }
}

/// Resolve for an install/shim decision: honor cached revocations, ignore
/// verified/unknown cache older than five minutes, and fail closed on RPC
/// errors instead of synthesizing Unknown.
pub async fn resolve_for_install(
    raw: &str,
    ecosystem: Option<&str>,
    node_url: Option<&str>,
) -> Result<TrustVerdict> {
    let (name, version) = parse_pkg(raw);
    let eco = ecosystem.unwrap_or("npm");
    let id = PackageId::new(eco, name, version.as_deref().unwrap_or("latest"));
    resolve_id_for_install(&id, node_url).await
}

pub async fn resolve_id_for_install(
    id: &PackageId,
    node_url: Option<&str>,
) -> Result<TrustVerdict> {
    if let Some(cached) = cache::get(id)? {
        if install_cache_usable(&cached, Utc::now()) {
            tracing::debug!("install cache hit for {}", id.canonical());
            return Ok(cached);
        }
        tracing::debug!(
            "install cache stale for {} — querying live node",
            id.canonical()
        );
    }

    let url = node_url
        .map(String::from)
        .unwrap_or_else(|| crate::default_node_url());

    let verdict = chain_client::fetch_verdict(id, &url).await?;
    cache::set(id, &verdict)?;
    Ok(verdict)
}

/// Gate one package the way `creg install` does. RPC failure is blocked unless
/// break-glass unverified is set.
pub async fn gate_shim_package(
    raw: &str,
    ecosystem: &str,
    allow_unverified: bool,
    node_url: Option<&str>,
) -> Result<InstallDecision> {
    match resolve_for_install(raw, Some(ecosystem), node_url).await {
        Ok(verdict) => Ok(evaluate_install(&verdict, allow_unverified)),
        Err(err) => {
            if allow_unverified {
                Ok(InstallDecision::AllowUnverified)
            } else {
                bail!(
                    "chain registry unreachable ({err}). Refusing to install '{raw}'. \
                     Set {ALLOW_UNVERIFIED_ENV}=true or pass --unverified only as break-glass."
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{Finding, PackageId};

    fn pkg() -> PackageId {
        PackageId::new("npm", "left-pad", "1.0.0")
    }

    fn verdict(
        status: VerdictStatus,
        source: VerdictSource,
        resolved_at: DateTime<Utc>,
    ) -> TrustVerdict {
        TrustVerdict {
            package: pkg(),
            status,
            resolved_at,
            source,
            deterministic_risk: None,
            evidence_phase: None,
            block_phase: None,
            chain_status: None,
            publisher: None,
        }
    }

    fn verified() -> VerdictStatus {
        VerdictStatus::Verified {
            block_hash: "0xabc".into(),
            content_hash: "hash".into(),
            ipfs_cid: "bafy".into(),
            findings: vec![],
        }
    }

    fn high_finding() -> Finding {
        Finding {
            id: "MAL-001".into(),
            title: "malware".into(),
            severity: FindingSeverity::High,
            description: "bad".into(),
            file: "index.js".into(),
            line: Some(1),
        }
    }

    #[test]
    fn revoked_always_blocks_even_with_unverified_flag() {
        let v = verdict(
            VerdictStatus::Revoked {
                reason: "malicious".into(),
                findings: vec![],
            },
            VerdictSource::Chain {
                node_url: "http://127.0.0.1:8080".into(),
            },
            Utc::now(),
        );
        assert!(evaluate_install(&v, false).is_blocked());
        assert!(evaluate_install(&v, true).is_blocked());
        if let InstallDecision::Block { code, .. } = evaluate_install(&v, true) {
            assert_eq!(code, "revoked");
        }
    }

    #[test]
    fn unverified_and_unknown_fail_closed_without_break_glass() {
        let now = Utc::now();
        let source = VerdictSource::Chain {
            node_url: "http://127.0.0.1:8080".into(),
        };
        let unverified = verdict(VerdictStatus::Unverified, source.clone(), now);
        let unknown = verdict(VerdictStatus::Unknown, source, now);
        match evaluate_install(&unverified, false) {
            InstallDecision::Block { code, .. } => assert_eq!(code, "unverified"),
            other => panic!("expected block, got {other:?}"),
        }
        match evaluate_install(&unknown, false) {
            InstallDecision::Block { code, .. } => assert_eq!(code, "unknown"),
            other => panic!("expected block, got {other:?}"),
        }
        assert_eq!(
            evaluate_install(&unverified, true),
            InstallDecision::AllowUnverified
        );
        assert_eq!(
            evaluate_install(&unknown, true),
            InstallDecision::AllowUnverified
        );
    }

    #[test]
    fn verified_without_high_findings_is_allowed() {
        let v = verdict(
            verified(),
            VerdictSource::Chain {
                node_url: "http://127.0.0.1:8080".into(),
            },
            Utc::now(),
        );
        assert_eq!(evaluate_install(&v, false), InstallDecision::Allow);
    }

    #[test]
    fn high_severity_verified_requires_break_glass() {
        let status = VerdictStatus::Verified {
            block_hash: "0xabc".into(),
            content_hash: "hash".into(),
            ipfs_cid: "bafy".into(),
            findings: vec![high_finding()],
        };
        let v = verdict(
            status,
            VerdictSource::Chain {
                node_url: "http://127.0.0.1:8080".into(),
            },
            Utc::now(),
        );
        match evaluate_install(&v, false) {
            InstallDecision::Block { code, .. } => assert_eq!(code, "high_severity"),
            other => panic!("expected block, got {other:?}"),
        }
        assert_eq!(evaluate_install(&v, true), InstallDecision::AllowUnverified);
    }

    #[test]
    fn stale_verified_cache_is_not_usable_for_install() {
        let now = Utc::now();
        let stale = verdict(
            verified(),
            VerdictSource::Cache {
                expires_at: now + Duration::hours(20),
            },
            now - Duration::hours(6),
        );
        assert!(!install_cache_usable(&stale, now));

        let fresh = verdict(
            verified(),
            VerdictSource::Cache {
                expires_at: now + Duration::minutes(4),
            },
            now - Duration::minutes(2),
        );
        assert!(install_cache_usable(&fresh, now));
    }

    #[test]
    fn cached_revocation_stays_usable_past_install_ttl() {
        let now = Utc::now();
        let revoked = verdict(
            VerdictStatus::Revoked {
                reason: "yanked".into(),
                findings: vec![],
            },
            VerdictSource::Cache {
                expires_at: now + Duration::hours(40),
            },
            now - Duration::hours(12),
        );
        assert!(install_cache_usable(&revoked, now));
    }

    #[test]
    fn shim_unverified_flag_and_env() {
        let args = vec!["npm".into(), "install".into(), "left-pad".into()];
        assert!(!args_allow_unverified(&args));
        let with_flag = vec![
            "npm".into(),
            "install".into(),
            "left-pad".into(),
            "--unverified".into(),
        ];
        assert!(args_allow_unverified(&with_flag));
        let stripped = strip_unverified_flag(&with_flag);
        assert_eq!(stripped, vec!["npm", "install", "left-pad"]);
        assert!(!stripped.iter().any(|a| a == "--unverified"));
        assert!(requirement_file_requested(&[
            "pip".into(),
            "install".into(),
            "-r".into(),
            "requirements.txt".into()
        ]));
        assert!(!requirement_file_requested(&[
            "pip".into(),
            "install".into(),
            "requests".into()
        ]));
    }

    struct EnvRestore {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvRestore {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn env_allow_unverified_accepts_true_1_yes() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let _clear = EnvRestore::remove(ALLOW_UNVERIFIED_ENV);
        assert!(!env_allow_unverified());
        let _one = EnvRestore::set(ALLOW_UNVERIFIED_ENV, "1");
        assert!(env_allow_unverified());
        drop(_one);
        let _true = EnvRestore::set(ALLOW_UNVERIFIED_ENV, "TRUE");
        assert!(env_allow_unverified());
        drop(_true);
        let _yes = EnvRestore::set(ALLOW_UNVERIFIED_ENV, "yes");
        assert!(env_allow_unverified());
        drop(_yes);
        let _no = EnvRestore::set(ALLOW_UNVERIFIED_ENV, "false");
        assert!(!env_allow_unverified());
    }
}
