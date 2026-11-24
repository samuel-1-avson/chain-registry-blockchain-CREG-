// crates/resolver/src/install_gate.rs
// Shared install admission for `creg install` and PATH shims (SHIM-301 +
// CLIENT-FRESHNESS).
//
// Default is fail-closed: unverified, unknown, revoked, quarantined, stale
// cache, high-severity findings, an unreachable node, revocation rollback,
// freeze, and (when a witness policy is configured) unsigned/stale/inconsistent
// tree heads all refuse. `--unverified` or CREG_ALLOW_UNVERIFIED is the only
// break-glass for unqualified packages; it cannot override revocation,
// quarantine (ECA-07, ADR-019 open decision #2) or anti-rollback.

use crate::chain_client;
use crate::light_client::{VerifiedProofBundle, VerifyOptions};
use crate::{cache, parse_pkg};
use anyhow::{bail, Result};
use chrono::{DateTime, Duration, Utc};
use common::{
    DisplayText, Finding, FindingSeverity, PackageId, TrustVerdict, VerdictSource, VerdictStatus,
};

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

/// Client install-decision lattice (`ECO-INV-02`):
/// `Revoked` ⊐ `Quarantined` ⊐ {`Unproven`, `Pending`, `Unknown`} ⊐ `Verified`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LatticeOutcome {
    Verified {
        findings: Vec<Finding>,
    },
    Pending,
    Unknown,
    Unproven {
        reason: String,
    },
    Quarantined {
        reason: String,
        content_hash: String,
        expires_at_unix: Option<i64>,
        certified: bool,
        height: u64,
    },
    Revoked {
        reason: String,
        certified: bool,
        height: u64,
    },
}

impl LatticeOutcome {
    /// Fail-closed dominance level (`Revoked` or expired `Quarantined` = 4,
    /// active `Quarantined` = 3, `{Unproven, Pending, Unknown}` = 2,
    /// `Verified` = 1).
    pub fn severity(&self, now_unix: i64) -> u8 {
        match self {
            Self::Revoked { .. } => 4,
            Self::Quarantined {
                expires_at_unix: Some(deadline),
                ..
            } if now_unix >= *deadline => 4,
            Self::Quarantined { .. } => 3,
            Self::Unproven { .. } | Self::Pending | Self::Unknown => 2,
            Self::Verified { .. } => 1,
        }
    }

    fn is_certified(&self) -> bool {
        match self {
            Self::Quarantined { certified, .. } | Self::Revoked { certified, .. } => *certified,
            Self::Verified { .. } => true,
            Self::Pending | Self::Unknown | Self::Unproven { .. } => false,
        }
    }

    /// Least upper bound on the fail-closed status lattice (`ECO-INV-02`).
    pub fn join(self, other: Self, now_unix: i64) -> Self {
        match self.severity(now_unix).cmp(&other.severity(now_unix)) {
            std::cmp::Ordering::Greater => self,
            std::cmp::Ordering::Less => other,
            std::cmp::Ordering::Equal => match (&self, &other) {
                (Self::Unproven { .. }, _) => self,
                (_, Self::Unproven { .. }) => other,
                (Self::Pending, Self::Unknown) => self,
                (Self::Unknown, Self::Pending) => other,
                _ if !self.is_certified() && other.is_certified() => other,
                _ => self,
            },
        }
    }

    /// Project a resolved [`TrustVerdict`] onto the lattice.
    pub fn from_verdict(verdict: &TrustVerdict, certified: bool, height: u64) -> Self {
        match &verdict.status {
            VerdictStatus::Verified { findings, .. } => Self::Verified {
                findings: findings.clone(),
            },
            VerdictStatus::Unverified => Self::Pending,
            VerdictStatus::Unknown => Self::Unknown,
            VerdictStatus::Quarantined {
                reason,
                content_hash,
                expires_at_unix,
            } => Self::Quarantined {
                reason: reason.clone(),
                content_hash: content_hash.clone(),
                expires_at_unix: *expires_at_unix,
                certified,
                height,
            },
            VerdictStatus::Revoked { reason, .. } => Self::Revoked {
                reason: reason.clone(),
                certified,
                height,
            },
        }
    }

    /// Project a cryptographically verified [`VerifiedProofBundle`] onto the
    /// lattice.
    pub fn from_verified_proof(proof: &VerifiedProofBundle) -> Self {
        match &proof.status {
            VerdictStatus::Verified { findings, .. } => Self::Verified {
                findings: findings.clone(),
            },
            VerdictStatus::Unverified => Self::Pending,
            VerdictStatus::Unknown => Self::Unknown,
            VerdictStatus::Quarantined {
                reason,
                content_hash,
                expires_at_unix,
            } => Self::Quarantined {
                reason: reason.clone(),
                content_hash: content_hash.clone(),
                expires_at_unix: *expires_at_unix,
                certified: proof.certified,
                height: proof.status_height,
            },
            VerdictStatus::Revoked { reason, .. } => Self::Revoked {
                reason: reason.clone(),
                certified: proof.certified,
                height: proof.status_height,
            },
        }
    }
}

/// Reconcile an untrusted REST verdict with local cryptographic proof
/// verification (`ECO-INV-01`, `ECO-INV-02`, `eco-adv-03`).
///
/// An unproven claim of `Verified` is never left installable (not even under
/// `--unverified`); it becomes [`LatticeOutcome::Unproven`].
pub fn reconcile_verdict_with_proof(
    rest_verdict: &TrustVerdict,
    proof: std::result::Result<&VerifiedProofBundle, &str>,
    now: DateTime<Utc>,
) -> LatticeOutcome {
    match proof {
        Ok(verified) => LatticeOutcome::from_verdict(rest_verdict, false, 0).join(
            LatticeOutcome::from_verified_proof(verified),
            now.timestamp(),
        ),
        Err(reason) => {
            if rest_verdict.status.is_safe() {
                LatticeOutcome::Unproven {
                    reason: reason.to_string(),
                }
            } else {
                LatticeOutcome::from_verdict(rest_verdict, false, 0)
            }
        }
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
    // A cached revocation, or a cached quarantine whose deadline has passed,
    // can only ever block, so it stays usable past the freshness window.
    if matches!(verdict.status, VerdictStatus::Revoked { .. })
        || verdict.status.quarantine_expired(now.timestamp())
    {
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

/// Render a unix timestamp for block messages.
pub(crate) fn format_unix(ts: Option<i64>) -> String {
    match ts.and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0)) {
        Some(at) => at.format("%Y-%m-%d %H:%M UTC").to_string(),
        None => "an unreported time".into(),
    }
}

pub fn evaluate_install(verdict: &TrustVerdict, allow_unverified: bool) -> InstallDecision {
    evaluate_install_at(verdict, allow_unverified, Utc::now())
}

/// Evaluate an [`ECO-INV-02`](LatticeOutcome) decision at `now`.
pub fn evaluate_lattice_at(
    canonical: &str,
    outcome: &LatticeOutcome,
    allow_unverified: bool,
    now: DateTime<Utc>,
) -> InstallDecision {
    match outcome {
        LatticeOutcome::Revoked {
            reason, certified, ..
        } => InstallDecision::Block {
            code: if *certified {
                "revoked_certified"
            } else {
                "revoked"
            },
            message: format!(
                "Package '{}' is REVOKED and cannot be installed. Reason: {}",
                canonical,
                DisplayText::for_terminal(reason)
            ),
        },
        LatticeOutcome::Quarantined {
            reason,
            expires_at_unix,
            ..
        } => {
            if expires_at_unix.is_some_and(|deadline| now.timestamp() >= deadline) {
                InstallDecision::Block {
                    code: "quarantine_expired",
                    message: format!(
                        "Package '{}' was QUARANTINED and the hold expired at {}; it fails closed to REVOKED and cannot be installed. Reason: {}",
                        canonical,
                        format_unix(*expires_at_unix),
                        DisplayText::for_terminal(reason)
                    ),
                }
            } else {
                let deadline = match expires_at_unix {
                    Some(_) => format!(
                        "fails closed to REVOKED at {} unless the committee releases it",
                        format_unix(*expires_at_unix)
                    ),
                    None => "no release deadline was reported".into(),
                };
                InstallDecision::Block {
                    code: "quarantined",
                    message: format!(
                        "Package '{}' is QUARANTINED for committee review and cannot be installed; --unverified and {ALLOW_UNVERIFIED_ENV} do not override a quarantine ({deadline}). Reason: {}",
                        canonical,
                        DisplayText::for_terminal(reason)
                    ),
                }
            }
        }
        LatticeOutcome::Unproven { reason } => InstallDecision::Block {
            code: "unproven_status",
            message: format!(
                "Package '{}' has an UNPROVEN status and cannot be installed; --unverified and {ALLOW_UNVERIFIED_ENV} do not bypass proof verification. Reason: {}",
                canonical,
                DisplayText::for_terminal(reason)
            ),
        },
        LatticeOutcome::Verified { findings } => {
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
                        canonical
                    ),
                }
            } else if has_severe {
                InstallDecision::AllowUnverified
            } else {
                InstallDecision::Allow
            }
        }
        LatticeOutcome::Pending => {
            if allow_unverified {
                InstallDecision::AllowUnverified
            } else {
                InstallDecision::Block {
                    code: "unverified",
                    message: format!(
                        "Package '{}' is not qualified. Refusing installation; use --unverified or {ALLOW_UNVERIFIED_ENV}=true only as an explicit break-glass exception.",
                        canonical
                    ),
                }
            }
        }
        LatticeOutcome::Unknown => {
            if allow_unverified {
                InstallDecision::AllowUnverified
            } else {
                InstallDecision::Block {
                    code: "unknown",
                    message: format!(
                        "Package '{}' is unknown to the chain registry. Use --unverified or {ALLOW_UNVERIFIED_ENV}=true to install from the original registry.",
                        canonical
                    ),
                }
            }
        }
    }
}

/// SHIM-301 status rules evaluated at `now`.
pub fn evaluate_install_at(
    verdict: &TrustVerdict,
    allow_unverified: bool,
    now: DateTime<Utc>,
) -> InstallDecision {
    let outcome = LatticeOutcome::from_verdict(verdict, false, 0);
    evaluate_lattice_at(
        &verdict.package.canonical(),
        &outcome,
        allow_unverified,
        now,
    )
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
    resolve_id_for_install_with_options(id, node_url, &VerifyOptions::from_env()).await
}

/// Proof-carrying install-grade resolution parameterized by [`VerifyOptions`].
pub async fn resolve_id_for_install_with_options(
    id: &PackageId,
    node_url: Option<&str>,
    options: &VerifyOptions,
) -> Result<TrustVerdict> {
    if let Some(cached) = cache::get(id)? {
        if install_cache_usable(&cached, Utc::now()) && cached.status.is_blocked() {
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
        .unwrap_or_else(crate::default_node_url);

    let verdict = chain_client::fetch_verdict(id, &url).await?;
    let (verdict, proof_opt) =
        crate::light_client::verify_verdict_detailed(verdict, &url, None, options).await?;
    if let Some(proof) = proof_opt {
        let store = crate::freshness::FreshnessStore::open_default()?;
        let last_tip = store.last_tip_height()?;
        if let Some(InstallDecision::Block { message, .. }) =
            crate::freshness::check_tip_height(last_tip, proof.tip_height)
        {
            bail!("{message}");
        }
        store.record_tip_height(proof.tip_height)?;
    }
    cache::set(id, &verdict)?;
    Ok(verdict)
}

/// Apply SHIM-301 status rules plus CLIENT-FRESHNESS anti-rollback and, when a
/// witness policy is configured, witnessed tree-head inclusion.
pub async fn admit_install_live(
    verdict: &TrustVerdict,
    allow_unverified: bool,
    node_url: Option<&str>,
) -> Result<InstallDecision> {
    use crate::freshness::{
        admit_with_freshness, evaluate_package_freshness, fetch_witnessed_evidence,
        load_witness_policy_from_env, FreshnessStore, WitnessPolicyLoad,
        DEFAULT_MAX_FUTURE_SKEW_SECONDS,
    };

    let store = FreshnessStore::open_default()?;
    let now = Utc::now();
    let policy = load_witness_policy_from_env();
    let max_skew = Duration::seconds(
        match &policy {
            WitnessPolicyLoad::Active(config) => config.policy.maximum_future_skew_seconds,
            _ => DEFAULT_MAX_FUTURE_SKEW_SECONDS,
        }
        .max(0),
    );
    if let Some(blocked) = evaluate_package_freshness(
        store.get_package(&verdict.package.canonical())?.as_ref(),
        verdict,
        now,
        max_skew,
    ) {
        return Ok(blocked);
    }

    let preview = evaluate_install(verdict, allow_unverified);
    let fetched = match (&policy, &preview) {
        (WitnessPolicyLoad::Active(config), InstallDecision::Allow) => {
            let url = node_url
                .map(String::from)
                .unwrap_or_else(crate::default_node_url);
            let previous = store.accepted_head()?;
            match fetch_witnessed_evidence(
                config,
                &verdict.package.canonical(),
                &url,
                previous.as_ref(),
            )
            .await
            {
                Ok(bundle) => Some(bundle),
                Err(blocked) => return Ok(blocked),
            }
        }
        _ => None,
    };

    Ok(admit_with_freshness(
        verdict,
        allow_unverified,
        &store,
        now,
        &policy,
        fetched.as_ref(),
        evaluate_install,
    ))
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
        Ok(verdict) => admit_install_live(&verdict, allow_unverified, node_url).await,
        Err(err) => {
            if allow_unverified {
                // Break-glass covers an unreachable node, not a package this
                // client already journaled as revoked or quarantined.
                let (name, version) = parse_pkg(raw);
                let id = PackageId::new(ecosystem, name, version.as_deref().unwrap_or("latest"));
                if let Some(blocked) = offline_journal_block(&id)? {
                    return Ok(blocked);
                }
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

/// Anti-rollback check used when the node cannot be reached: replays the
/// journal against an `Unknown` observation so a revoked or quarantined
/// package stays blocked even under break-glass.
fn offline_journal_block(id: &PackageId) -> Result<Option<InstallDecision>> {
    let store = crate::freshness::FreshnessStore::open_default()?;
    let previous = store.get_package(&id.canonical())?;
    Ok(offline_journal_decision(previous.as_ref(), id, Utc::now()))
}

fn offline_journal_decision(
    previous: Option<&crate::freshness::PackageFreshnessRecord>,
    id: &PackageId,
    now: DateTime<Utc>,
) -> Option<InstallDecision> {
    let probe = TrustVerdict {
        package: id.clone(),
        status: VerdictStatus::Unknown,
        resolved_at: now,
        source: VerdictSource::Chain {
            node_url: "unreachable".into(),
        },
        deterministic_risk: None,
        evidence_phase: None,
        block_phase: None,
        chain_status: None,
        publisher: None,
    };
    crate::freshness::evaluate_package_freshness(
        previous,
        &probe,
        now,
        Duration::seconds(crate::freshness::DEFAULT_MAX_FUTURE_SKEW_SECONDS),
    )
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

    fn quarantined(expires_at_unix: Option<i64>) -> VerdictStatus {
        VerdictStatus::Quarantined {
            reason: "credential exfiltration\u{1b}]52;c;ZXZpbA==\u{07}".into(),
            content_hash: "hash".into(),
            expires_at_unix,
        }
    }

    /// eco-adv-01: node reports `quarantined`; break-glass must not install.
    #[test]
    fn eco_adv_01_quarantine_blocks_even_with_break_glass() {
        let now = Utc::now();
        let v = verdict(
            quarantined(Some(now.timestamp() + 3_600)),
            VerdictSource::Chain {
                node_url: "http://127.0.0.1:8080".into(),
            },
            now,
        );
        for allow in [false, true] {
            match evaluate_install_at(&v, allow, now) {
                InstallDecision::Block { code, message } => {
                    assert_eq!(code, "quarantined");
                    assert!(!message.contains('\u{1b}'), "{message}");
                    assert!(!message.contains('\u{07}'), "{message}");
                }
                other => panic!("expected quarantine block, got {other:?}"),
            }
        }
    }

    #[test]
    fn expired_quarantine_fails_closed_as_revoked() {
        let now = Utc::now();
        let v = verdict(
            quarantined(Some(now.timestamp() - 1)),
            VerdictSource::Chain {
                node_url: "http://127.0.0.1:8080".into(),
            },
            now,
        );
        match evaluate_install_at(&v, true, now) {
            InstallDecision::Block { code, .. } => assert_eq!(code, "quarantine_expired"),
            other => panic!("expected expiry block, got {other:?}"),
        }
        // An expired hold behaves like a cached revocation.
        let cached = verdict(
            quarantined(Some(now.timestamp() - 1)),
            VerdictSource::Cache {
                expires_at: now + Duration::minutes(1),
            },
            now - Duration::hours(6),
        );
        assert!(install_cache_usable(&cached, now));
    }

    #[test]
    fn quarantine_without_deadline_is_still_blocked() {
        let now = Utc::now();
        let v = verdict(
            quarantined(None),
            VerdictSource::Chain {
                node_url: "http://127.0.0.1:8080".into(),
            },
            now,
        );
        assert!(evaluate_install_at(&v, true, now).is_blocked());
    }

    #[test]
    fn revoked_reason_is_terminal_sanitized() {
        let v = verdict(
            VerdictStatus::Revoked {
                reason: "ok\rREJECTED\u{202E}".into(),
                findings: vec![],
            },
            VerdictSource::Chain {
                node_url: "http://127.0.0.1:8080".into(),
            },
            Utc::now(),
        );
        match evaluate_install(&v, false) {
            InstallDecision::Block { message, .. } => {
                assert!(!message.contains('\r'));
                assert!(!message.contains('\u{202E}'));
            }
            other => panic!("expected block, got {other:?}"),
        }
    }

    #[test]
    fn offline_break_glass_respects_journaled_revocation_and_quarantine() {
        use crate::freshness::{PackageFreshnessKind, PackageFreshnessRecord};
        let now = Utc::now();
        let record = |kind| PackageFreshnessRecord {
            canonical: pkg().canonical(),
            kind,
            resolved_at: now - Duration::minutes(10),
            content_hash: None,
            certified: true,
            height: 0,
        };

        assert!(offline_journal_decision(None, &pkg(), now).is_none());

        let revoked = record(PackageFreshnessKind::Revoked);
        assert!(offline_journal_decision(Some(&revoked), &pkg(), now)
            .is_some_and(|decision| decision.is_blocked()));

        let held = record(PackageFreshnessKind::Quarantined {
            expires_at_unix: Some(now.timestamp() + 3_600),
        });
        match offline_journal_decision(Some(&held), &pkg(), now) {
            Some(InstallDecision::Block { code, .. }) => assert_eq!(code, "quarantined"),
            other => panic!("expected quarantine block, got {other:?}"),
        }

        let lapsed = record(PackageFreshnessKind::Quarantined {
            expires_at_unix: Some(now.timestamp() - 1),
        });
        match offline_journal_decision(Some(&lapsed), &pkg(), now) {
            Some(InstallDecision::Block { code, .. }) => assert_eq!(code, "quarantine_expired"),
            other => panic!("expected expiry block, got {other:?}"),
        }

        let verified = record(PackageFreshnessKind::Verified);
        assert!(offline_journal_decision(Some(&verified), &pkg(), now).is_none());
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

    #[test]
    fn eco_adv_03_rest_verified_with_revoked_or_missing_proof_blocks_fail_closed() {
        let now = Utc::now();
        let lying_rest = verdict(
            verified(),
            VerdictSource::Chain {
                node_url: "https://n.example".into(),
            },
            now,
        );
        let unproven =
            reconcile_verdict_with_proof(&lying_rest, Err("status SMT proof invalid"), now);
        assert!(matches!(unproven, LatticeOutcome::Unproven { .. }));
        // Break-glass cannot override Unproven (ECO-INV-02).
        match evaluate_lattice_at(&pkg().canonical(), &unproven, true, now) {
            InstallDecision::Block { code, .. } => assert_eq!(code, "unproven_status"),
            other => panic!("expected unproven_status block, got {other:?}"),
        }

        let revoked_proof = VerifiedProofBundle {
            canonical: pkg().canonical(),
            tip_height: 12,
            tip_block_hash: "ab".repeat(32),
            tip_timestamp_unix: now.timestamp(),
            publication_height: 10,
            publication_block_hash: "cd".repeat(32),
            record: common::ChainRecord {
                id: pkg(),
                content_hash: "hash".into(),
                ..Default::default()
            },
            status: VerdictStatus::Revoked {
                reason: "committee revoked".into(),
                findings: vec![],
            },
            status_view: registry_view::PackageStatusViewV1::Revoked {
                height: 12,
                authority: registry_view::RevocationAuthorityV1::CommitteeQuorum,
                reason: DisplayText::bounded("committee revoked", 128),
                witness_digest: None,
                certificate_digest: Some("ef".repeat(32)),
            },
            quorum: registry_view::QuorumLabelV1::V4Quorum,
            certified: true,
            status_height: 12,
            status_proven_by_smt: true,
        };
        let joined = reconcile_verdict_with_proof(&lying_rest, Ok(&revoked_proof), now);
        match evaluate_lattice_at(&pkg().canonical(), &joined, true, now) {
            InstallDecision::Block { code, .. } => assert_eq!(code, "revoked_certified"),
            other => panic!("expected revoked_certified block, got {other:?}"),
        }
    }

    #[test]
    fn eco_inv_02_lattice_ordering_and_break_glass_rules() {
        let now = Utc::now();
        let canonical = pkg().canonical();
        let verified_l = LatticeOutcome::Verified { findings: vec![] };
        let pending_l = LatticeOutcome::Pending;
        let unknown_l = LatticeOutcome::Unknown;
        let unproven_l = LatticeOutcome::Unproven {
            reason: "missing proof".into(),
        };
        let active_q = LatticeOutcome::Quarantined {
            reason: "hold".into(),
            content_hash: "hh".into(),
            expires_at_unix: Some(now.timestamp() + 3600),
            certified: true,
            height: 5,
        };
        let expired_q = LatticeOutcome::Quarantined {
            reason: "hold".into(),
            content_hash: "hh".into(),
            expires_at_unix: Some(now.timestamp() - 1),
            certified: true,
            height: 5,
        };
        let revoked_l = LatticeOutcome::Revoked {
            reason: "yank".into(),
            certified: true,
            height: 6,
        };

        assert_eq!(
            verified_l.clone().join(pending_l.clone(), now.timestamp()),
            pending_l
        );
        assert_eq!(
            pending_l.clone().join(unproven_l.clone(), now.timestamp()),
            unproven_l
        );
        assert_eq!(
            unproven_l.clone().join(active_q.clone(), now.timestamp()),
            active_q
        );
        assert_eq!(
            active_q.clone().join(revoked_l.clone(), now.timestamp()),
            revoked_l
        );
        assert_eq!(expired_q.severity(now.timestamp()), 4);

        assert_eq!(
            evaluate_lattice_at(&canonical, &pending_l, true, now),
            InstallDecision::AllowUnverified
        );
        assert_eq!(
            evaluate_lattice_at(&canonical, &unknown_l, true, now),
            InstallDecision::AllowUnverified
        );
        assert!(evaluate_lattice_at(&canonical, &unproven_l, true, now).is_blocked());
        assert!(evaluate_lattice_at(&canonical, &active_q, true, now).is_blocked());
        assert!(evaluate_lattice_at(&canonical, &expired_q, true, now).is_blocked());
        assert!(evaluate_lattice_at(&canonical, &revoked_l, true, now).is_blocked());
    }

    #[test]
    fn eco_adv_05_uncertified_revocation_is_not_sticky_and_tip_rollback_blocks() {
        use crate::freshness::{admit_with_freshness_certified, FreshnessStore, WitnessPolicyLoad};

        let store = FreshnessStore::memory();
        let now = Utc::now();
        let src = VerdictSource::Chain {
            node_url: "https://n.example".into(),
        };

        // Uncertified revocation blocks itself, but is NOT sticky in Journal v2 (ECO-INV-04).
        let uncert_rev = verdict(
            VerdictStatus::Revoked {
                reason: "single validator".into(),
                findings: vec![],
            },
            src.clone(),
            now,
        );
        let d1 = admit_with_freshness_certified(
            &uncert_rev,
            true,
            &store,
            now,
            &WitnessPolicyLoad::Inactive,
            None,
            false,
            5,
            Some(5),
            evaluate_install,
        );
        assert!(d1.is_blocked());

        let ver_later = verdict(verified(), src.clone(), now + Duration::seconds(10));
        let d2 = admit_with_freshness_certified(
            &ver_later,
            false,
            &store,
            now + Duration::seconds(10),
            &WitnessPolicyLoad::Inactive,
            None,
            true,
            6,
            Some(6),
            evaluate_install,
        );
        assert_eq!(d2, InstallDecision::Allow);

        // Tip rollback (< persisted tip 6) blocks cleanly with freshness_freeze.
        let d_rollback = admit_with_freshness_certified(
            &ver_later,
            false,
            &store,
            now + Duration::seconds(20),
            &WitnessPolicyLoad::Inactive,
            None,
            true,
            4,
            Some(4),
            evaluate_install,
        );
        match d_rollback {
            InstallDecision::Block { code, .. } => assert_eq!(code, "freshness_freeze"),
            other => panic!("expected freshness_freeze on tip rollback, got {other:?}"),
        }

        // Certified revocation IS sticky and blocks subsequent Verified permanently.
        let cert_rev = verdict(
            VerdictStatus::Revoked {
                reason: "committee qc".into(),
                findings: vec![],
            },
            src,
            now + Duration::seconds(30),
        );
        let d3 = admit_with_freshness_certified(
            &cert_rev,
            true,
            &store,
            now + Duration::seconds(30),
            &WitnessPolicyLoad::Inactive,
            None,
            true,
            7,
            Some(7),
            evaluate_install,
        );
        assert!(d3.is_blocked());
        let d4 = admit_with_freshness_certified(
            &ver_later,
            false,
            &store,
            now + Duration::seconds(40),
            &WitnessPolicyLoad::Inactive,
            None,
            true,
            8,
            Some(8),
            evaluate_install,
        );
        match d4 {
            InstallDecision::Block { code, .. } => assert_eq!(code, "freshness_rollback"),
            other => {
                panic!("expected freshness_rollback after certified revocation, got {other:?}")
            }
        }
    }
}
