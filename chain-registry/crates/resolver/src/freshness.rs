// crates/resolver/src/freshness.rs
// CLIENT-FRESHNESS: anti-rollback journal, expiry/skew, and optional witnessed
// tree-head admission for `creg install` and PATH shims.
//
// Unsigned GET /v1/transparency/.../head responses are monitoring data only.
// They are never install-eligible. Witnessed inclusion is required only when
// a log public key and witness policy are configured (or
// CREG_REQUIRE_WITNESSED_HEAD is set). Independent witness operations remain
// out of this crate.

use crate::file_store::{atomic_write, quarantine_corrupt_file, FileLockGuard};
use crate::install_gate::{format_unix, InstallDecision};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use common::{
    verify_witnessed_inclusion, ConsistencyProof, InclusionProof, PackageId, TransparencyEvent,
    TransparencyVerificationPolicy, TreeHead, TrustVerdict, VerdictStatus, WitnessPolicy,
    WitnessedTreeHead,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const JOURNAL_SCHEMA_VERSION: u16 = 2;
pub const MAX_JOURNAL_FILE_BYTES: u64 = 32 * 1024 * 1024;

static PROCESS_JOURNAL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn process_journal_guard() -> std::sync::MutexGuard<'static, ()> {
    PROCESS_JOURNAL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub const FRESHNESS_PATH_ENV: &str = "CREG_FRESHNESS_PATH";
pub const FRESHNESS_RESET_ENV: &str = "CREG_FRESHNESS_RESET";
pub const REQUIRE_WITNESSED_HEAD_ENV: &str = "CREG_REQUIRE_WITNESSED_HEAD";
pub const TRANSPARENCY_LOG_PUBKEY_ENV: &str = "CREG_TRANSPARENCY_LOG_PUBKEY";
pub const TRANSPARENCY_LOG_ID_ENV: &str = "CREG_TRANSPARENCY_LOG_ID";
pub const TRANSPARENCY_WITNESS_KEYS_ENV: &str = "CREG_TRANSPARENCY_WITNESS_KEYS";
pub const TRANSPARENCY_MIN_WITNESSES_ENV: &str = "CREG_TRANSPARENCY_MIN_WITNESSES";
pub const TRANSPARENCY_MAX_HEAD_AGE_ENV: &str = "CREG_TRANSPARENCY_MAX_HEAD_AGE_SECONDS";
pub const TRANSPARENCY_MAX_SKEW_ENV: &str = "CREG_TRANSPARENCY_MAX_FUTURE_SKEW_SECONDS";
pub const WITNESSED_EVIDENCE_PATH_ENV: &str = "CREG_WITNESSED_EVIDENCE_PATH";
/// Comma-separated URLs of independent `WitnessedTreeHead` JSON (one per witness).
pub const WITNESS_HEAD_URLS_ENV: &str = "CREG_WITNESS_HEAD_URLS";

pub const DEFAULT_MAX_HEAD_AGE_SECONDS: i64 = 300;
pub const DEFAULT_MAX_FUTURE_SKEW_SECONDS: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PackageFreshnessKind {
    Verified,
    Revoked,
    /// A committee hold was observed (ECA-07). It may only end in `Revoked`,
    /// or be released to `Verified` before `expires_at_unix`. After the
    /// deadline it is treated as `Revoked` permanently. Certified releases
    /// (`QuarantineResolutionV1`) replace the deadline rule in ADR-019 Inc 3.
    Quarantined {
        expires_at_unix: Option<i64>,
    },
}

impl PackageFreshnessKind {
    /// `true` for a quarantine whose deadline has passed at `now`.
    pub fn quarantine_expired_at(&self, now: DateTime<Utc>) -> bool {
        matches!(
            self,
            PackageFreshnessKind::Quarantined {
                expires_at_unix: Some(expires_at),
            } if now.timestamp() >= *expires_at
        )
    }
}

fn default_certified_true() -> bool {
    true
}

fn default_journal_version() -> u16 {
    JOURNAL_SCHEMA_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageFreshnessRecord {
    pub canonical: String,
    pub kind: PackageFreshnessKind,
    pub resolved_at: DateTime<Utc>,
    #[serde(default)]
    pub content_hash: Option<String>,
    /// ECO-INV-04: only certified revocations and quarantines are sticky.
    /// Defaults to `true` when decoding legacy v1 journal files so existing
    /// entries remain fail-closed.
    #[serde(default = "default_certified_true")]
    pub certified: bool,
    /// Block height where this status was proven (0 for legacy/unproven records).
    #[serde(default)]
    pub height: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FreshnessState {
    #[serde(default = "default_journal_version")]
    schema_version: u16,
    #[serde(default)]
    packages: BTreeMap<String, PackageFreshnessRecord>,
    #[serde(default)]
    accepted_head: Option<TreeHead>,
    #[serde(default)]
    last_tip_height: u64,
}

impl Default for FreshnessState {
    fn default() -> Self {
        Self {
            schema_version: JOURNAL_SCHEMA_VERSION,
            packages: BTreeMap::new(),
            accepted_head: None,
            last_tip_height: 0,
        }
    }
}

enum StoreInner {
    Memory(Mutex<FreshnessState>),
    File { path: PathBuf, lock: Mutex<()> },
}

pub struct FreshnessStore {
    inner: StoreInner,
}

impl FreshnessStore {
    pub fn memory() -> Self {
        Self {
            inner: StoreInner::Memory(Mutex::new(FreshnessState::default())),
        }
    }

    pub fn open_path(path: PathBuf) -> Self {
        Self {
            inner: StoreInner::File {
                path,
                lock: Mutex::new(()),
            },
        }
    }

    pub fn open_default() -> Result<Self> {
        if let Ok(path) = std::env::var(FRESHNESS_PATH_ENV) {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return Ok(Self::open_path(PathBuf::from(trimmed)));
            }
        }
        let dir = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("chain-registry");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create freshness dir {}", dir.display()))?;
        Ok(Self::open_path(dir.join("freshness-state.json")))
    }

    pub fn get_package(&self, canonical: &str) -> Result<Option<PackageFreshnessRecord>> {
        Ok(self.load()?.packages.get(canonical).cloned())
    }

    pub fn accepted_head(&self) -> Result<Option<TreeHead>> {
        if env_truthy(FRESHNESS_RESET_ENV) {
            return Ok(None);
        }
        Ok(self.load()?.accepted_head)
    }

    pub fn last_tip_height(&self) -> Result<u64> {
        if env_truthy(FRESHNESS_RESET_ENV) {
            return Ok(0);
        }
        Ok(self.load()?.last_tip_height)
    }

    pub fn record_tip_height(&self, height: u64) -> Result<u64> {
        let reset = env_truthy(FRESHNESS_RESET_ENV);
        self.update_state(|state| {
            let previous = if reset { 0 } else { state.last_tip_height };
            if height < previous {
                bail!(
                    "tip height {} is behind previously accepted height {} (freeze/rollback)",
                    height,
                    previous
                );
            }
            if !reset && height == state.last_tip_height {
                return Ok((height, false));
            }
            state.last_tip_height = height;
            Ok((height, true))
        })
    }

    pub fn persist_package(&self, record: PackageFreshnessRecord) -> Result<()> {
        self.persist_package_at(record, Utc::now())
    }

    /// Persist `record` under the journal's monotonic rules (`ECA-12`,
    /// `ECO-INV-04`):
    /// - uncertified revocations and quarantines are never stored as sticky;
    /// - first-seen non-empty `content_hash` is pinned permanently;
    /// - certified `Revoked` is sticky forever;
    /// - certified `Quarantined` keeps its first-seen deadline, may be cleared
    ///   to `Verified` only by a certified release before that deadline, and
    ///   becomes `Revoked` once the deadline passes.
    pub fn persist_package_at(
        &self,
        mut record: PackageFreshnessRecord,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if !record.certified
            && matches!(
                record.kind,
                PackageFreshnessKind::Revoked | PackageFreshnessKind::Quarantined { .. }
            )
        {
            return Ok(());
        }
        record.content_hash = record
            .content_hash
            .as_deref()
            .map(str::trim)
            .filter(|hash| !hash.is_empty())
            .map(str::to_string);

        self.update_state(|state| {
            if let Some(existing) = state.packages.get(&record.canonical) {
                let existing_hash = existing
                    .content_hash
                    .as_deref()
                    .map(str::trim)
                    .filter(|hash| !hash.is_empty())
                    .map(str::to_string);
                match (&existing_hash, &record.content_hash) {
                    (Some(prev), Some(next)) if !prev.eq_ignore_ascii_case(next) => {
                        bail!(
                            "refusing to overwrite pinned content_hash for '{}': {} != {}",
                            record.canonical,
                            prev,
                            next
                        );
                    }
                    (Some(prev), None) => {
                        record.content_hash = Some(prev.clone());
                    }
                    _ => {}
                }

                if existing.certified {
                    match (existing.kind, record.kind) {
                        (PackageFreshnessKind::Revoked, PackageFreshnessKind::Revoked) => {
                            if record.height <= existing.height
                                && record.content_hash == existing.content_hash
                            {
                                return Ok(((), false));
                            }
                            record.certified = true;
                            record.height = record.height.max(existing.height);
                        }
                        (PackageFreshnessKind::Revoked, _) => return Ok(((), false)),
                        (held @ PackageFreshnessKind::Quarantined { .. }, _)
                            if held.quarantine_expired_at(now) =>
                        {
                            record.kind = PackageFreshnessKind::Revoked;
                            record.certified = true;
                            record.height = record.height.max(existing.height);
                        }
                        (
                            PackageFreshnessKind::Quarantined { .. },
                            PackageFreshnessKind::Revoked,
                        ) => {
                            record.height = record.height.max(existing.height);
                        }
                        (
                            PackageFreshnessKind::Quarantined { .. },
                            PackageFreshnessKind::Quarantined { .. },
                        ) => return Ok(((), false)),
                        (
                            PackageFreshnessKind::Quarantined { .. },
                            PackageFreshnessKind::Verified,
                        ) if !record.certified => return Ok(((), false)),
                        _ => {
                            record.height = record.height.max(existing.height);
                        }
                    }
                } else {
                    record.height = record.height.max(existing.height);
                }
            }
            state.packages.insert(record.canonical.clone(), record);
            Ok(((), true))
        })
    }

    pub fn persist_accepted_head(&self, head: TreeHead) -> Result<()> {
        self.update_state(|state| {
            state.accepted_head = Some(head);
            Ok(((), true))
        })
    }

    pub fn clear(&self) -> Result<()> {
        self.save(&FreshnessState::default())
    }

    fn load(&self) -> Result<FreshnessState> {
        match &self.inner {
            StoreInner::Memory(lock) => Ok(lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()),
            StoreInner::File { path, lock } => {
                let _process_guard = process_journal_guard();
                let _instance_guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let _file_guard = FileLockGuard::acquire(path)?;
                load_file(path)
            }
        }
    }

    fn save(&self, state: &FreshnessState) -> Result<()> {
        match &self.inner {
            StoreInner::Memory(lock) => {
                *lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = state.clone();
                Ok(())
            }
            StoreInner::File { path, lock } => {
                let _process_guard = process_journal_guard();
                let _instance_guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let _file_guard = FileLockGuard::acquire(path)?;
                save_file(path, state)
            }
        }
    }

    fn update_state<R>(
        &self,
        f: impl FnOnce(&mut FreshnessState) -> Result<(R, bool)>,
    ) -> Result<R> {
        match &self.inner {
            StoreInner::Memory(lock) => {
                let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut next = guard.clone();
                let (result, changed) = f(&mut next)?;
                if changed {
                    *guard = next;
                }
                Ok(result)
            }
            StoreInner::File { path, lock } => {
                let _process_guard = process_journal_guard();
                let _instance_guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let _file_guard = FileLockGuard::acquire(path)?;
                let mut state = load_file(path)?;
                let (result, changed) = f(&mut state)?;
                if changed {
                    save_file(path, &state)?;
                }
                Ok(result)
            }
        }
    }
}

fn load_file(path: &Path) -> Result<FreshnessState> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FreshnessState::default());
        }
        Err(err) => {
            return Err(err).with_context(|| format!("stat freshness state {}", path.display()));
        }
    };
    if meta.len() > MAX_JOURNAL_FILE_BYTES {
        quarantine_corrupt_file(path);
        bail!(
            "freshness state {} exceeded max size ({} bytes); quarantined",
            path.display(),
            MAX_JOURNAL_FILE_BYTES
        );
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FreshnessState::default());
        }
        Err(err) => {
            return Err(err).with_context(|| format!("read freshness state {}", path.display()));
        }
    };
    let state: FreshnessState = match serde_json::from_slice(&bytes) {
        Ok(state) => state,
        Err(err) => {
            quarantine_corrupt_file(path);
            return Err(err).context("decode freshness state; quarantined corrupt file");
        }
    };
    if state.schema_version == 0 || state.schema_version > JOURNAL_SCHEMA_VERSION {
        quarantine_corrupt_file(path);
        bail!(
            "unsupported freshness state schema version {}; quarantined",
            state.schema_version
        );
    }
    Ok(state)
}

fn save_file(path: &Path, state: &FreshnessState) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(state).context("encode freshness state")?;
    atomic_write(path, &bytes)
}

pub fn clear_default_store() -> Result<()> {
    FreshnessStore::open_default()?.clear()
}

fn env_truthy(name: &str) -> bool {
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

fn env_trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn record_from_verdict(verdict: &TrustVerdict) -> Option<PackageFreshnessRecord> {
    record_from_verdict_at(verdict, Utc::now())
}

/// Journal record for `verdict` as observed at `now`. An already-expired
/// quarantine is recorded as `Revoked`.
pub fn record_from_verdict_at(
    verdict: &TrustVerdict,
    now: DateTime<Utc>,
) -> Option<PackageFreshnessRecord> {
    record_from_verdict_certified(verdict, now, true, 0)
}

/// Journal record for `verdict` with explicit certification provenance
/// (`ECO-INV-04`) and proven block `height`.
pub fn record_from_verdict_certified(
    verdict: &TrustVerdict,
    now: DateTime<Utc>,
    certified: bool,
    height: u64,
) -> Option<PackageFreshnessRecord> {
    let (kind, content_hash) = match &verdict.status {
        VerdictStatus::Verified { content_hash, .. } => (
            PackageFreshnessKind::Verified,
            Some(content_hash.trim().to_string()).filter(|hash| !hash.is_empty()),
        ),
        VerdictStatus::Revoked { .. } => (PackageFreshnessKind::Revoked, None),
        status @ VerdictStatus::Quarantined {
            content_hash,
            expires_at_unix,
            ..
        } => {
            let hash = Some(content_hash.trim().to_string()).filter(|hash| !hash.is_empty());
            if status.quarantine_expired(now.timestamp()) {
                (PackageFreshnessKind::Revoked, hash)
            } else {
                (
                    PackageFreshnessKind::Quarantined {
                        expires_at_unix: *expires_at_unix,
                    },
                    hash,
                )
            }
        }
        VerdictStatus::Unverified | VerdictStatus::Unknown => return None,
    };
    Some(PackageFreshnessRecord {
        canonical: verdict.package.canonical(),
        kind,
        resolved_at: verdict.resolved_at,
        content_hash,
        certified,
        height,
    })
}

pub fn check_tip_height(last_tip_height: u64, incoming_tip_height: u64) -> Option<InstallDecision> {
    if incoming_tip_height < last_tip_height {
        Some(block(
            "freshness_freeze",
            format!(
                "Node tip height {} is behind previously accepted tip height {} (freeze/rollback).",
                incoming_tip_height, last_tip_height
            ),
        ))
    } else {
        None
    }
}

fn status_content_hash(status: &VerdictStatus) -> Option<&str> {
    match status {
        VerdictStatus::Verified { content_hash, .. }
        | VerdictStatus::Quarantined { content_hash, .. } => {
            let trimmed = content_hash.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        }
        _ => None,
    }
}

pub fn evaluate_package_freshness(
    previous: Option<&PackageFreshnessRecord>,
    verdict: &TrustVerdict,
    now: DateTime<Utc>,
    max_future_skew: Duration,
) -> Option<InstallDecision> {
    if verdict.resolved_at > now + max_future_skew {
        return Some(block(
            "freshness_future",
            format!(
                "Package '{}' metadata timestamp is beyond permitted clock skew.",
                verdict.package.canonical()
            ),
        ));
    }
    let previous = previous?;
    let incoming_revoked = matches!(verdict.status, VerdictStatus::Revoked { .. });
    if previous.certified {
        if let PackageFreshnessKind::Quarantined { expires_at_unix } = previous.kind {
            if !incoming_revoked {
                if previous.kind.quarantine_expired_at(now) {
                    return Some(block(
                        "quarantine_expired",
                        format!(
                            "Package '{}' was QUARANTINED and the hold expired at {}; it fails closed to REVOKED and cannot be installed.",
                            verdict.package.canonical(),
                            format_unix(expires_at_unix)
                        ),
                    ));
                }
                if matches!(
                    verdict.status,
                    VerdictStatus::Unverified | VerdictStatus::Unknown
                ) {
                    return Some(block(
                        "quarantined",
                        format!(
                            "Package '{}' was previously observed as QUARANTINED; a hold ends only in revocation or a verified release, never in an unverified install (anti-rollback).",
                            verdict.package.canonical()
                        ),
                    ));
                }
            }
        }
        if previous.kind == PackageFreshnessKind::Revoked && !incoming_revoked {
            return Some(block(
                "freshness_rollback",
                format!(
                    "Package '{}' was previously observed as REVOKED; refusing a later non-revoked status (anti-rollback).",
                    verdict.package.canonical()
                ),
            ));
        }
    }
    if matches!(verdict.status, VerdictStatus::Verified { .. })
        && verdict.resolved_at < previous.resolved_at
    {
        return Some(block(
            "freshness_freeze",
            format!(
                "Package '{}' offered older qualified metadata than the last accepted record (freeze/rollback).",
                verdict.package.canonical()
            ),
        ));
    }
    if let (Some(previous_hash), Some(incoming_hash)) = (
        previous
            .content_hash
            .as_deref()
            .map(str::trim)
            .filter(|hash| !hash.is_empty()),
        status_content_hash(&verdict.status),
    ) {
        if !previous_hash.eq_ignore_ascii_case(incoming_hash) {
            return Some(block(
                "freshness_mix_and_match",
                format!(
                    "Package '{}' mixed a previously accepted digest ({}) with a different digest ({}).",
                    verdict.package.canonical(),
                    previous_hash,
                    incoming_hash
                ),
            ));
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct WitnessPolicyConfig {
    pub log_id: String,
    pub log_public_key: String,
    pub policy: TransparencyVerificationPolicy,
}

#[derive(Debug, Clone)]
pub enum WitnessPolicyLoad {
    Inactive,
    Active(WitnessPolicyConfig),
    Incomplete(String),
}

pub fn load_witness_policy_from_env() -> WitnessPolicyLoad {
    let require = env_truthy(REQUIRE_WITNESSED_HEAD_ENV);
    let pubkey = env_trimmed(TRANSPARENCY_LOG_PUBKEY_ENV);
    let witness_keys_raw = env_trimmed(TRANSPARENCY_WITNESS_KEYS_ENV);
    let any_configured = require || pubkey.is_some() || witness_keys_raw.is_some();
    if !any_configured {
        return WitnessPolicyLoad::Inactive;
    }

    let Some(log_public_key) = pubkey else {
        return WitnessPolicyLoad::Incomplete(
            "CREG_TRANSPARENCY_LOG_PUBKEY is required when witnessed heads are configured".into(),
        );
    };
    let trusted_witness_keys: BTreeMap<String, String> = match witness_keys_raw {
        Some(raw) => match serde_json::from_str(&raw) {
            Ok(map) => map,
            Err(err) => {
                return WitnessPolicyLoad::Incomplete(format!(
                    "CREG_TRANSPARENCY_WITNESS_KEYS must be a JSON object of witness_id → hex pubkey: {err}"
                ));
            }
        },
        None => BTreeMap::new(),
    };
    if trusted_witness_keys.is_empty() {
        return WitnessPolicyLoad::Incomplete(
            "CREG_TRANSPARENCY_WITNESS_KEYS must include at least one trusted witness".into(),
        );
    }
    let minimum_witnesses = env_trimmed(TRANSPARENCY_MIN_WITNESSES_ENV)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    if minimum_witnesses == 0 {
        return WitnessPolicyLoad::Incomplete(
            "CREG_TRANSPARENCY_MIN_WITNESSES must be at least 1".into(),
        );
    }
    let maximum_head_age_seconds = env_trimmed(TRANSPARENCY_MAX_HEAD_AGE_ENV)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(DEFAULT_MAX_HEAD_AGE_SECONDS);
    let maximum_future_skew_seconds = env_trimmed(TRANSPARENCY_MAX_SKEW_ENV)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(DEFAULT_MAX_FUTURE_SKEW_SECONDS);
    if maximum_head_age_seconds < 0 || maximum_future_skew_seconds < 0 {
        return WitnessPolicyLoad::Incomplete(
            "transparency freshness bounds must be non-negative".into(),
        );
    }

    WitnessPolicyLoad::Active(WitnessPolicyConfig {
        log_id: env_trimmed(TRANSPARENCY_LOG_ID_ENV).unwrap_or_else(|| "creg".into()),
        log_public_key,
        policy: TransparencyVerificationPolicy {
            maximum_head_age_seconds,
            maximum_future_skew_seconds,
            witness_policy: WitnessPolicy {
                minimum_witnesses,
                trusted_witness_keys,
            },
        },
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WitnessedInclusionEvidence {
    pub event: TransparencyEvent,
    pub inclusion: InclusionProof,
    pub witnessed_head: WitnessedTreeHead,
    #[serde(default)]
    pub consistency: Option<ConsistencyProof>,
}

pub enum WitnessedAdmission<'a> {
    NotRequired,
    Required(&'a WitnessedInclusionEvidence),
    Missing { code: &'static str, message: String },
}

pub fn verify_install_inclusion(
    package: &PackageId,
    verdict: &TrustVerdict,
    config: &WitnessPolicyConfig,
    evidence: &WitnessedInclusionEvidence,
    previous_head: Option<&TreeHead>,
    now: DateTime<Utc>,
) -> Result<(), InstallDecision> {
    let head = &evidence.witnessed_head.signed_head.head;
    if head.log_id != config.log_id {
        return Err(block(
            "freshness_log_mismatch",
            format!(
                "Witnessed tree head log_id '{}' does not match configured '{}'.",
                head.log_id, config.log_id
            ),
        ));
    }
    if evidence.event.subject != package.canonical() {
        return Err(block(
            "freshness_mix_and_match",
            format!(
                "Transparency event subject '{}' does not match package '{}'.",
                evidence.event.subject,
                package.canonical()
            ),
        ));
    }
    if matches!(verdict.status, VerdictStatus::Verified { .. })
        && evidence.event.kind == "revocation"
    {
        return Err(block(
            "freshness_rollback",
            format!(
                "Package '{}' has a later transparency revocation event; refusing a qualified install.",
                package.canonical()
            ),
        ));
    }
    if let VerdictStatus::Verified { content_hash, .. } = &verdict.status {
        if !content_hash.is_empty() && evidence.event.payload_hash != *content_hash {
            return Err(block(
                "freshness_mix_and_match",
                format!(
                    "Package '{}' content hash does not match the witnessed transparency payload.",
                    package.canonical()
                ),
            ));
        }
    }

    verify_witnessed_inclusion(
        &evidence.event,
        &evidence.inclusion,
        &evidence.witnessed_head,
        &config.log_public_key,
        &config.policy,
        previous_head,
        evidence.consistency.as_ref(),
        now,
    )
    .map_err(|err| map_witness_error(package, err))
}

fn map_witness_error(package: &PackageId, err: anyhow::Error) -> InstallDecision {
    let message = err.to_string();
    let code = if message.contains("stale") {
        "freshness_stale_head"
    } else if message.contains("future clock skew") {
        "freshness_future"
    } else if message.contains("missing consistency") || message.contains("not consistent") {
        "freshness_inconsistent_head"
    } else if message.contains("not validly included") {
        "freshness_inclusion"
    } else if message.contains("unsigned") || message.contains("signature") {
        "unsigned_head"
    } else if message.contains("witness") {
        "freshness_witnesses"
    } else {
        "freshness_unwitnessed"
    };
    block(
        code,
        format!(
            "Package '{}' failed witnessed freshness verification: {message}",
            package.canonical()
        ),
    )
}

pub fn admit_with_freshness(
    verdict: &TrustVerdict,
    allow_unverified: bool,
    store: &FreshnessStore,
    now: DateTime<Utc>,
    policy: &WitnessPolicyLoad,
    evidence: Option<&WitnessedInclusionEvidence>,
    evaluate: impl FnOnce(&TrustVerdict, bool) -> InstallDecision,
) -> InstallDecision {
    admit_with_freshness_certified(
        verdict,
        allow_unverified,
        store,
        now,
        policy,
        evidence,
        true,
        0,
        None,
        evaluate,
    )
}

/// Apply install admission rules, proof certification provenance (`ECO-INV-04`),
/// tip-height rollback checks (`ECA-12`), and optional witnessed-head checks.
#[allow(clippy::too_many_arguments)]
pub fn admit_with_freshness_certified(
    verdict: &TrustVerdict,
    allow_unverified: bool,
    store: &FreshnessStore,
    now: DateTime<Utc>,
    policy: &WitnessPolicyLoad,
    evidence: Option<&WitnessedInclusionEvidence>,
    certified: bool,
    status_height: u64,
    tip_height: Option<u64>,
    evaluate: impl FnOnce(&TrustVerdict, bool) -> InstallDecision,
) -> InstallDecision {
    if let Some(incoming_tip) = tip_height {
        let last_tip = match store.last_tip_height() {
            Ok(height) => height,
            Err(err) => {
                return block(
                    "freshness_unwitnessed",
                    format!("failed to read persisted tip height: {err}"),
                );
            }
        };
        if let Some(blocked) = check_tip_height(last_tip, incoming_tip) {
            return blocked;
        }
    }

    let max_skew = Duration::seconds(
        match policy {
            WitnessPolicyLoad::Active(config) => config.policy.maximum_future_skew_seconds,
            _ => DEFAULT_MAX_FUTURE_SKEW_SECONDS,
        }
        .max(0),
    );
    if let Ok(previous) = store.get_package(&verdict.package.canonical()) {
        if let Some(blocked) = evaluate_package_freshness(previous.as_ref(), verdict, now, max_skew)
        {
            return blocked;
        }
    }

    let decision = evaluate(verdict, allow_unverified);
    if let InstallDecision::Block { .. } = &decision {
        if let Some(record) = record_from_verdict_certified(verdict, now, certified, status_height)
        {
            if matches!(
                record.kind,
                PackageFreshnessKind::Revoked | PackageFreshnessKind::Quarantined { .. }
            ) {
                let _ = store.persist_package_at(record, now);
            }
        }
        if let Some(incoming_tip) = tip_height {
            let _ = store.record_tip_height(incoming_tip);
        }
        return decision;
    }

    let witnessed = match (policy, evidence) {
        (WitnessPolicyLoad::Inactive, _) => WitnessedAdmission::NotRequired,
        (WitnessPolicyLoad::Incomplete(message), _) => WitnessedAdmission::Missing {
            code: "freshness_unwitnessed",
            message: message.clone(),
        },
        (WitnessPolicyLoad::Active(_), Some(bundle)) => WitnessedAdmission::Required(bundle),
        (WitnessPolicyLoad::Active(_), None) => WitnessedAdmission::Missing {
            code: "unsigned_head",
            message: format!(
                "Package '{}' requires a witnessed signed tree head; unsigned transparency heads are not install-eligible.",
                verdict.package.canonical()
            ),
        },
    };

    match (&decision, witnessed) {
        (_, WitnessedAdmission::NotRequired) => {
            persist_accepted_package_certified(
                store,
                verdict,
                &decision,
                now,
                certified,
                status_height,
            );
            if let Some(incoming_tip) = tip_height {
                let _ = store.record_tip_height(incoming_tip);
            }
            decision
        }
        (InstallDecision::AllowUnverified, WitnessedAdmission::Missing { .. }) => decision,
        (InstallDecision::AllowUnverified, WitnessedAdmission::Required(_)) => decision,
        (_, WitnessedAdmission::Missing { code, message }) => block(code, message),
        (InstallDecision::Allow, WitnessedAdmission::Required(bundle)) => {
            let WitnessPolicyLoad::Active(config) = policy else {
                return block(
                    "freshness_unwitnessed",
                    "internal: witnessed evidence without an active policy".into(),
                );
            };
            let previous = match store.accepted_head() {
                Ok(head) => head,
                Err(err) => {
                    return block(
                        "freshness_unwitnessed",
                        format!("failed to read persisted tree head: {err}"),
                    );
                }
            };
            if let Err(blocked) = verify_install_inclusion(
                &verdict.package,
                verdict,
                config,
                bundle,
                previous.as_ref(),
                now,
            ) {
                return blocked;
            }
            let _ = store.persist_accepted_head(bundle.witnessed_head.signed_head.head.clone());
            persist_accepted_package_certified(
                store,
                verdict,
                &decision,
                now,
                certified,
                status_height,
            );
            if let Some(incoming_tip) = tip_height {
                let _ = store.record_tip_height(incoming_tip);
            }
            decision
        }
        (other, _) => other.clone(),
    }
}

fn persist_accepted_package_certified(
    store: &FreshnessStore,
    verdict: &TrustVerdict,
    decision: &InstallDecision,
    now: DateTime<Utc>,
    certified: bool,
    status_height: u64,
) {
    if matches!(
        decision,
        InstallDecision::Allow | InstallDecision::AllowUnverified
    ) {
        if let Some(record) = record_from_verdict_certified(verdict, now, certified, status_height)
        {
            let _ = store.persist_package_at(record, now);
        }
    }
}

fn block(code: &'static str, message: String) -> InstallDecision {
    InstallDecision::Block { code, message }
}

#[derive(Debug)]
enum ParsedHead {
    Witnessed(WitnessedTreeHead),
    Unsigned,
}

fn parse_head_bytes(bytes: &[u8]) -> ParsedHead {
    if let Ok(witnessed) = serde_json::from_slice::<WitnessedTreeHead>(bytes) {
        if !witnessed.signed_head.signature.trim().is_empty()
            && !witnessed.witness_signatures.is_empty()
        {
            return ParsedHead::Witnessed(witnessed);
        }
    }
    ParsedHead::Unsigned
}

pub async fn fetch_witnessed_evidence(
    config: &WitnessPolicyConfig,
    subject: &str,
    node_url: &str,
    previous_head: Option<&TreeHead>,
) -> Result<WitnessedInclusionEvidence, InstallDecision> {
    if let Some(path) = env_trimmed(WITNESSED_EVIDENCE_PATH_ENV) {
        let bytes = std::fs::read(&path).map_err(|err| {
            block(
                "freshness_unwitnessed",
                format!("failed to read {WITNESSED_EVIDENCE_PATH_ENV} ({path}): {err}"),
            )
        })?;
        return serde_json::from_slice(&bytes).map_err(|err| {
            block(
                "freshness_unwitnessed",
                format!("invalid witnessed evidence file {path}: {err}"),
            )
        });
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|err| {
            block(
                "freshness_unwitnessed",
                format!("failed to build transparency client: {err}"),
            )
        })?;
    let base = node_url.trim_end_matches('/');
    let log_id = urlencoding::encode(&config.log_id);

    let witnessed_head = if let Some(raw) = env_trimmed(WITNESS_HEAD_URLS_ENV) {
        let mut heads = Vec::new();
        for url in raw.split(',').map(str::trim).filter(|url| !url.is_empty()) {
            let bytes = get_bytes(&client, url).await?;
            let head: WitnessedTreeHead = serde_json::from_slice(&bytes).map_err(|err| {
                block(
                    "freshness_unwitnessed",
                    format!("invalid WitnessedTreeHead from {url}: {err}"),
                )
            })?;
            heads.push(head);
        }
        WitnessedTreeHead::merge_from(&heads).map_err(|err| {
            block(
                "freshness_unwitnessed",
                format!("failed to merge independent witness heads: {err}"),
            )
        })?
    } else {
        let head_url = format!("{base}/v1/public/transparency/{log_id}/head");
        let head_bytes = get_bytes(&client, &head_url).await?;
        match parse_head_bytes(&head_bytes) {
            ParsedHead::Witnessed(head) => head,
            ParsedHead::Unsigned => {
                return Err(block(
                    "unsigned_head",
                    format!(
                        "Node at {head_url} returned an unsigned transparency head. Unsigned heads are not install-eligible. Independent witnesses publish WitnessedTreeHead JSON; set {WITNESS_HEAD_URLS_ENV}."
                    ),
                ));
            }
        }
    };

    let latest_url = format!(
        "{base}/v1/public/transparency/{log_id}/latest-event?subject={}",
        urlencoding::encode(subject)
    );
    #[derive(Deserialize)]
    struct LatestEventResp {
        event: TransparencyEvent,
        inclusion: InclusionProof,
    }
    let latest: LatestEventResp = get_json(&client, &latest_url).await?;
    let mut evidence = WitnessedInclusionEvidence {
        event: latest.event,
        inclusion: latest.inclusion,
        witnessed_head,
        consistency: None,
    };
    if let Some(previous) = previous_head {
        let consistency_url = format!(
            "{base}/v1/public/transparency/{log_id}/consistency/{}",
            previous.tree_size
        );
        evidence.consistency = Some(get_json(&client, &consistency_url).await?);
    }
    Ok(evidence)
}

async fn get_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, InstallDecision> {
    let response = client.get(url).send().await.map_err(|err| {
        block(
            "freshness_unwitnessed",
            format!("failed to fetch {url}: {err}"),
        )
    })?;
    if !response.status().is_success() {
        return Err(block(
            "freshness_unwitnessed",
            format!("{url} returned HTTP {}", response.status()),
        ));
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|err| {
            block(
                "freshness_unwitnessed",
                format!("failed to read {url}: {err}"),
            )
        })
}

async fn get_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T, InstallDecision> {
    let bytes = get_bytes(client, url).await?;
    serde_json::from_slice(&bytes).map_err(|err| {
        block(
            "freshness_unwitnessed",
            format!("invalid JSON from {url}: {err}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use common::crypto::SigningKey;
    use common::{
        consistency_proof, inclusion_proof, sign_tree_head, sign_witnessed_tree_head, tree_root,
        Finding, FindingSeverity, PackageId, VerdictSource, TRANSPARENCY_LOG_VERSION,
    };

    fn pkg() -> PackageId {
        PackageId::new("npm", "left-pad", "1.0.0")
    }

    fn verdict(status: VerdictStatus, resolved_at: DateTime<Utc>) -> TrustVerdict {
        TrustVerdict {
            package: pkg(),
            status,
            resolved_at,
            source: VerdictSource::Chain {
                node_url: "http://127.0.0.1:8080".into(),
            },
            deterministic_risk: None,
            evidence_phase: None,
            block_phase: None,
            chain_status: None,
            publisher: None,
        }
    }

    fn verified_hash(hash: &str) -> VerdictStatus {
        VerdictStatus::Verified {
            block_hash: "0xabc".into(),
            content_hash: hash.into(),
            ipfs_cid: "bafy".into(),
            findings: vec![],
        }
    }

    fn revoked() -> VerdictStatus {
        VerdictStatus::Revoked {
            reason: "malicious".into(),
            findings: vec![],
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 23, 12, 0, 0).unwrap()
    }

    fn evaluate_stub(verdict: &TrustVerdict, allow_unverified: bool) -> InstallDecision {
        crate::install_gate::evaluate_install(verdict, allow_unverified)
    }

    #[test]
    fn revoked_then_verified_is_rollback() {
        let store = FreshnessStore::memory();
        let t0 = now();
        store
            .persist_package(PackageFreshnessRecord {
                canonical: pkg().canonical(),
                kind: PackageFreshnessKind::Revoked,
                resolved_at: t0,
                content_hash: None,
                certified: true,
                height: 0,
            })
            .unwrap();
        let incoming = verdict(verified_hash("aa"), t0 + Duration::seconds(5));
        match evaluate_package_freshness(
            store.get_package(&pkg().canonical()).unwrap().as_ref(),
            &incoming,
            t0 + Duration::seconds(10),
            Duration::seconds(60),
        ) {
            Some(InstallDecision::Block { code, .. }) => assert_eq!(code, "freshness_rollback"),
            other => panic!("expected rollback, got {other:?}"),
        }
    }

    #[test]
    fn older_verified_after_newer_verified_is_freeze() {
        let previous = PackageFreshnessRecord {
            canonical: pkg().canonical(),
            kind: PackageFreshnessKind::Verified,
            resolved_at: now(),
            content_hash: Some("aa".into()),
            certified: true,
            height: 0,
        };
        let incoming = verdict(verified_hash("aa"), now() - Duration::hours(1));
        match evaluate_package_freshness(Some(&previous), &incoming, now(), Duration::seconds(60)) {
            Some(InstallDecision::Block { code, .. }) => assert_eq!(code, "freshness_freeze"),
            other => panic!("expected freeze, got {other:?}"),
        }
    }

    #[test]
    fn future_timestamp_is_rejected() {
        let incoming = verdict(verified_hash("aa"), now() + Duration::hours(2));
        match evaluate_package_freshness(None, &incoming, now(), Duration::seconds(60)) {
            Some(InstallDecision::Block { code, .. }) => assert_eq!(code, "freshness_future"),
            other => panic!("expected future, got {other:?}"),
        }
    }

    #[test]
    fn mix_and_match_same_timestamp_different_hash_is_rejected() {
        let previous = PackageFreshnessRecord {
            canonical: pkg().canonical(),
            kind: PackageFreshnessKind::Verified,
            resolved_at: now(),
            content_hash: Some("aa".into()),
            certified: true,
            height: 0,
        };
        let incoming = verdict(verified_hash("bb"), now());
        match evaluate_package_freshness(Some(&previous), &incoming, now(), Duration::seconds(60)) {
            Some(InstallDecision::Block { code, .. }) => {
                assert_eq!(code, "freshness_mix_and_match")
            }
            other => panic!("expected mix-and-match, got {other:?}"),
        }
    }

    #[test]
    fn verified_then_revoked_is_accepted_and_persisted() {
        let store = FreshnessStore::memory();
        let t0 = now();
        let allow = admit_with_freshness(
            &verdict(verified_hash("aa"), t0),
            false,
            &store,
            t0,
            &WitnessPolicyLoad::Inactive,
            None,
            evaluate_stub,
        );
        assert_eq!(allow, InstallDecision::Allow);
        let blocked = admit_with_freshness(
            &verdict(revoked(), t0 + Duration::seconds(1)),
            true,
            &store,
            t0 + Duration::seconds(1),
            &WitnessPolicyLoad::Inactive,
            None,
            evaluate_stub,
        );
        match blocked {
            InstallDecision::Block { code, .. } => assert_eq!(code, "revoked"),
            other => panic!("expected revoked block, got {other:?}"),
        }
        assert_eq!(
            store.get_package(&pkg().canonical()).unwrap().unwrap().kind,
            PackageFreshnessKind::Revoked
        );
    }

    #[test]
    fn break_glass_cannot_override_revocation_rollback() {
        let store = FreshnessStore::memory();
        let t0 = now();
        store
            .persist_package(PackageFreshnessRecord {
                canonical: pkg().canonical(),
                kind: PackageFreshnessKind::Revoked,
                resolved_at: t0,
                content_hash: None,
                certified: true,
                height: 0,
            })
            .unwrap();
        match admit_with_freshness(
            &verdict(verified_hash("aa"), t0 + Duration::seconds(2)),
            true,
            &store,
            t0 + Duration::seconds(2),
            &WitnessPolicyLoad::Inactive,
            None,
            evaluate_stub,
        ) {
            InstallDecision::Block { code, .. } => assert_eq!(code, "freshness_rollback"),
            other => panic!("expected rollback, got {other:?}"),
        }
    }

    #[test]
    fn unsigned_head_blocks_verified_when_policy_active() {
        let store = FreshnessStore::memory();
        let config = sample_policy();
        match admit_with_freshness(
            &verdict(verified_hash("aa"), now()),
            false,
            &store,
            now(),
            &WitnessPolicyLoad::Active(config),
            None,
            evaluate_stub,
        ) {
            InstallDecision::Block { code, .. } => assert_eq!(code, "unsigned_head"),
            other => panic!("expected unsigned_head, got {other:?}"),
        }
    }

    #[test]
    fn break_glass_unverified_skips_missing_witnessed_head() {
        let store = FreshnessStore::memory();
        let config = sample_policy();
        let unknown = verdict(VerdictStatus::Unknown, now());
        assert_eq!(
            admit_with_freshness(
                &unknown,
                true,
                &store,
                now(),
                &WitnessPolicyLoad::Active(config),
                None,
                evaluate_stub,
            ),
            InstallDecision::AllowUnverified
        );
    }

    fn sample_keys() -> (SigningKey, SigningKey, String, BTreeMap<String, String>) {
        let log_key = SigningKey::from_ed25519_seed(&[11u8; 32]);
        let witness_key = SigningKey::from_ed25519_seed(&[12u8; 32]);
        let mut keys = BTreeMap::new();
        keys.insert("witness-a".into(), witness_key.public_key().to_hex());
        (
            log_key.clone(),
            witness_key,
            log_key.public_key().to_hex(),
            keys,
        )
    }

    fn sample_policy() -> WitnessPolicyConfig {
        let (_, _, pubkey, keys) = sample_keys();
        WitnessPolicyConfig {
            log_id: "test-log".into(),
            log_public_key: pubkey,
            policy: TransparencyVerificationPolicy {
                maximum_head_age_seconds: 60,
                maximum_future_skew_seconds: 5,
                witness_policy: WitnessPolicy {
                    minimum_witnesses: 1,
                    trusted_witness_keys: keys,
                },
            },
        }
    }

    fn events_and_leaves(payload: &str) -> (Vec<TransparencyEvent>, Vec<String>) {
        let events: Vec<TransparencyEvent> = (0..2)
            .map(|sequence| TransparencyEvent {
                version: TRANSPARENCY_LOG_VERSION,
                sequence,
                kind: "package-finalized".into(),
                subject: if sequence == 1 {
                    pkg().canonical()
                } else {
                    "npm:other@1.0.0".into()
                },
                payload_hash: if sequence == 1 {
                    payload.into()
                } else {
                    common::sha256_hex(b"other")
                },
                source: "test".into(),
                occurred_at: "2026-08-23T12:00:00Z".into(),
            })
            .collect();
        let leaves = events.iter().map(TransparencyEvent::leaf_hash).collect();
        (events, leaves)
    }

    fn witnessed_bundle(
        payload: &str,
        issued_at: &str,
        log_key: &SigningKey,
        witness_key: &SigningKey,
        previous: bool,
    ) -> (WitnessedInclusionEvidence, TreeHead) {
        let (events, leaves) = events_and_leaves(payload);
        let previous_head = TreeHead {
            version: TRANSPARENCY_LOG_VERSION,
            log_id: "test-log".into(),
            tree_size: 1,
            root_hash: tree_root(&leaves[..1]).unwrap(),
            issued_at: "2026-08-23T11:59:00Z".into(),
        };
        let current = TreeHead {
            version: TRANSPARENCY_LOG_VERSION,
            log_id: "test-log".into(),
            tree_size: 2,
            root_hash: tree_root(&leaves).unwrap(),
            issued_at: issued_at.into(),
        };
        let witnessed = sign_witnessed_tree_head(
            sign_tree_head(current, "log-operator", log_key),
            "witness-a",
            witness_key,
        );
        let evidence = WitnessedInclusionEvidence {
            event: events[1].clone(),
            inclusion: inclusion_proof("test-log", &leaves, 1).unwrap(),
            witnessed_head: witnessed,
            consistency: if previous {
                Some(consistency_proof("test-log", &leaves, 1).unwrap())
            } else {
                None
            },
        };
        (evidence, previous_head)
    }

    #[test]
    fn witnessed_inclusion_allows_consistent_fresh_head() {
        let (log_key, witness_key, pubkey, keys) = sample_keys();
        let store = FreshnessStore::memory();
        let (evidence, previous) =
            witnessed_bundle("aa", "2026-08-23T12:00:00Z", &log_key, &witness_key, true);
        store.persist_accepted_head(previous).unwrap();
        let config = WitnessPolicyConfig {
            log_id: "test-log".into(),
            log_public_key: pubkey,
            policy: TransparencyVerificationPolicy {
                maximum_head_age_seconds: 60,
                maximum_future_skew_seconds: 5,
                witness_policy: WitnessPolicy {
                    minimum_witnesses: 1,
                    trusted_witness_keys: keys,
                },
            },
        };
        let incoming = verdict(verified_hash("aa"), now());
        assert_eq!(
            admit_with_freshness(
                &incoming,
                false,
                &store,
                now(),
                &WitnessPolicyLoad::Active(config),
                Some(&evidence),
                evaluate_stub,
            ),
            InstallDecision::Allow
        );
        assert_eq!(
            store.accepted_head().unwrap().unwrap().tree_size,
            2,
            "accepted head must advance after a consistent witnessed update"
        );
    }

    #[test]
    fn stale_witnessed_head_is_rejected() {
        let (log_key, witness_key, pubkey, keys) = sample_keys();
        let store = FreshnessStore::memory();
        let (evidence, previous) =
            witnessed_bundle("aa", "2026-08-23T12:00:00Z", &log_key, &witness_key, true);
        store.persist_accepted_head(previous).unwrap();
        let config = WitnessPolicyConfig {
            log_id: "test-log".into(),
            log_public_key: pubkey,
            policy: TransparencyVerificationPolicy {
                maximum_head_age_seconds: 60,
                maximum_future_skew_seconds: 5,
                witness_policy: WitnessPolicy {
                    minimum_witnesses: 1,
                    trusted_witness_keys: keys,
                },
            },
        };
        match admit_with_freshness(
            &verdict(verified_hash("aa"), now()),
            false,
            &store,
            now() + Duration::minutes(5),
            &WitnessPolicyLoad::Active(config),
            Some(&evidence),
            evaluate_stub,
        ) {
            InstallDecision::Block { code, .. } => assert_eq!(code, "freshness_stale_head"),
            other => panic!("expected stale head, got {other:?}"),
        }
    }

    #[test]
    fn missing_consistency_from_persisted_head_is_rejected() {
        let (log_key, witness_key, pubkey, keys) = sample_keys();
        let store = FreshnessStore::memory();
        let (mut evidence, previous) =
            witnessed_bundle("aa", "2026-08-23T12:00:00Z", &log_key, &witness_key, true);
        evidence.consistency = None;
        store.persist_accepted_head(previous).unwrap();
        let config = WitnessPolicyConfig {
            log_id: "test-log".into(),
            log_public_key: pubkey,
            policy: TransparencyVerificationPolicy {
                maximum_head_age_seconds: 60,
                maximum_future_skew_seconds: 5,
                witness_policy: WitnessPolicy {
                    minimum_witnesses: 1,
                    trusted_witness_keys: keys,
                },
            },
        };
        match admit_with_freshness(
            &verdict(verified_hash("aa"), now()),
            false,
            &store,
            now(),
            &WitnessPolicyLoad::Active(config),
            Some(&evidence),
            evaluate_stub,
        ) {
            InstallDecision::Block { code, .. } => assert_eq!(code, "freshness_inconsistent_head"),
            other => panic!("expected missing consistency, got {other:?}"),
        }
    }

    #[test]
    fn rollback_to_smaller_tree_without_consistency_is_rejected() {
        let (log_key, witness_key, pubkey, keys) = sample_keys();
        let store = FreshnessStore::memory();
        let (events, leaves) = events_and_leaves("aa");
        let large = TreeHead {
            version: TRANSPARENCY_LOG_VERSION,
            log_id: "test-log".into(),
            tree_size: 2,
            root_hash: tree_root(&leaves).unwrap(),
            issued_at: "2026-08-23T12:00:00Z".into(),
        };
        store.persist_accepted_head(large).unwrap();
        let small_leaves = vec![leaves[0].clone()];
        let small = TreeHead {
            version: TRANSPARENCY_LOG_VERSION,
            log_id: "test-log".into(),
            tree_size: 1,
            root_hash: tree_root(&small_leaves).unwrap(),
            issued_at: "2026-08-23T12:00:01Z".into(),
        };
        let evidence = WitnessedInclusionEvidence {
            event: events[0].clone(),
            inclusion: inclusion_proof("test-log", &small_leaves, 0).unwrap(),
            witnessed_head: sign_witnessed_tree_head(
                sign_tree_head(small, "log-operator", &log_key),
                "witness-a",
                &witness_key,
            ),
            consistency: None,
        };
        let config = WitnessPolicyConfig {
            log_id: "test-log".into(),
            log_public_key: pubkey,
            policy: TransparencyVerificationPolicy {
                maximum_head_age_seconds: 60,
                maximum_future_skew_seconds: 5,
                witness_policy: WitnessPolicy {
                    minimum_witnesses: 1,
                    trusted_witness_keys: keys,
                },
            },
        };
        match admit_with_freshness(
            &verdict(verified_hash("aa"), now()),
            false,
            &store,
            now(),
            &WitnessPolicyLoad::Active(config),
            Some(&evidence),
            evaluate_stub,
        ) {
            InstallDecision::Block { code, .. } => {
                assert!(
                    code == "freshness_inconsistent_head"
                        || code == "freshness_mix_and_match"
                        || code == "freshness_inclusion",
                    "unexpected code {code}"
                );
            }
            other => panic!("expected rollback block, got {other:?}"),
        }
    }

    #[test]
    fn high_severity_still_blocks_before_witness_persist() {
        let store = FreshnessStore::memory();
        let status = VerdictStatus::Verified {
            block_hash: "0xabc".into(),
            content_hash: "aa".into(),
            ipfs_cid: "bafy".into(),
            findings: vec![Finding {
                id: "MAL-001".into(),
                title: "malware".into(),
                severity: FindingSeverity::High,
                description: "bad".into(),
                file: "index.js".into(),
                line: Some(1),
            }],
        };
        match admit_with_freshness(
            &verdict(status, now()),
            false,
            &store,
            now(),
            &WitnessPolicyLoad::Inactive,
            None,
            evaluate_stub,
        ) {
            InstallDecision::Block { code, .. } => assert_eq!(code, "high_severity"),
            other => panic!("expected high_severity, got {other:?}"),
        }
        assert!(store.get_package(&pkg().canonical()).unwrap().is_none());
    }

    #[test]
    fn file_store_round_trips_revocation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("freshness-state.json");
        let store = FreshnessStore::open_path(path.clone());
        store
            .persist_package(PackageFreshnessRecord {
                canonical: pkg().canonical(),
                kind: PackageFreshnessKind::Revoked,
                resolved_at: now(),
                content_hash: None,
                certified: true,
                height: 0,
            })
            .unwrap();
        let reopened = FreshnessStore::open_path(path);
        assert_eq!(
            reopened
                .get_package(&pkg().canonical())
                .unwrap()
                .unwrap()
                .kind,
            PackageFreshnessKind::Revoked
        );
    }

    #[test]
    fn unsigned_json_tree_head_is_not_parsed_as_witnessed() {
        let unsigned = serde_json::json!({
            "version": 1,
            "log_id": "creg",
            "tree_size": 1,
            "root_hash": "00".repeat(32),
            "issued_at": "2026-08-23T12:00:00Z"
        });
        match parse_head_bytes(unsigned.to_string().as_bytes()) {
            ParsedHead::Unsigned => {}
            other => panic!("expected unsigned, got {other:?}"),
        }
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
    fn require_witnessed_head_without_keys_is_incomplete() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let _require = EnvRestore::set(REQUIRE_WITNESSED_HEAD_ENV, "true");
        let _pubkey = EnvRestore::remove(TRANSPARENCY_LOG_PUBKEY_ENV);
        let _keys = EnvRestore::remove(TRANSPARENCY_WITNESS_KEYS_ENV);
        match load_witness_policy_from_env() {
            WitnessPolicyLoad::Incomplete(_) => {}
            other => panic!("expected incomplete, got {other:?}"),
        }
    }

    #[test]
    fn eco_adv_06_different_content_hash_at_later_timestamp_blocks_as_mix_and_match() {
        let store = FreshnessStore::memory();
        let t0 = now();
        let hash_a = "aa".repeat(32);
        let hash_b = "bb".repeat(32);
        let first = verdict(verified_hash(&hash_a), t0);
        let rec = record_from_verdict_certified(&first, t0, true, 10).unwrap();
        store.persist_package_at(rec, t0).unwrap();

        let later = verdict(verified_hash(&hash_b), t0 + Duration::hours(1));
        match evaluate_package_freshness(
            store.get_package(&pkg().canonical()).unwrap().as_ref(),
            &later,
            t0 + Duration::hours(1),
            Duration::seconds(60),
        ) {
            Some(InstallDecision::Block { code, .. }) => {
                assert_eq!(code, "freshness_mix_and_match")
            }
            other => panic!("expected freshness_mix_and_match, got {other:?}"),
        }

        // First-seen content hash cannot be overwritten even at a newer height.
        let overwrite =
            record_from_verdict_certified(&later, t0 + Duration::hours(1), true, 11).unwrap();
        assert!(store
            .persist_package_at(overwrite, t0 + Duration::hours(1))
            .is_err());
    }

    #[test]
    fn tip_height_monotonicity_rejects_rollback() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let _reset = EnvRestore::remove(FRESHNESS_RESET_ENV);
        let dir = tempfile::tempdir().unwrap();
        let store = FreshnessStore::open_path(dir.path().join("freshness-state.json"));
        assert_eq!(store.last_tip_height().unwrap(), 0);
        assert_eq!(store.record_tip_height(10).unwrap(), 10);
        assert_eq!(store.record_tip_height(12).unwrap(), 12);
        assert!(store.record_tip_height(11).is_err());
        assert_eq!(store.last_tip_height().unwrap(), 12);

        match check_tip_height(12, 11) {
            Some(InstallDecision::Block { code, .. }) => assert_eq!(code, "freshness_freeze"),
            other => panic!("expected freshness_freeze, got {other:?}"),
        }
        assert!(check_tip_height(12, 12).is_none());
    }

    #[test]
    fn uncertified_revocation_and_quarantine_are_not_sticky() {
        let store = FreshnessStore::memory();
        let t0 = now();
        let hash = "aa".repeat(32);

        // On an unseen package, uncertified revocations/quarantines are ignored.
        let uncert_rev =
            record_from_verdict_certified(&verdict(revoked(), t0), t0, false, 5).unwrap();
        store.persist_package_at(uncert_rev, t0).unwrap();
        assert!(store.get_package(&pkg().canonical()).unwrap().is_none());

        let uncert_q = record_from_verdict_certified(
            &verdict(
                VerdictStatus::Quarantined {
                    reason: "unproven".into(),
                    content_hash: hash.clone(),
                    expires_at_unix: Some((t0 + Duration::hours(72)).timestamp()),
                },
                t0,
            ),
            t0,
            false,
            6,
        )
        .unwrap();
        store.persist_package_at(uncert_q, t0).unwrap();
        assert!(store.get_package(&pkg().canonical()).unwrap().is_none());

        // Certified hold is sticky, cannot be cleared by uncertified Verified,
        // and CAN be cleared before expiry by certified Verified.
        let cert_q = record_from_verdict_certified(
            &verdict(
                VerdictStatus::Quarantined {
                    reason: "class c hold".into(),
                    content_hash: hash.clone(),
                    expires_at_unix: Some((t0 + Duration::hours(72)).timestamp()),
                },
                t0,
            ),
            t0,
            true,
            7,
        )
        .unwrap();
        store.persist_package_at(cert_q, t0).unwrap();
        assert!(matches!(
            store.get_package(&pkg().canonical()).unwrap().unwrap().kind,
            PackageFreshnessKind::Quarantined { .. }
        ));

        let uncert_release = record_from_verdict_certified(
            &verdict(verified_hash(&hash), t0 + Duration::minutes(1)),
            t0 + Duration::minutes(1),
            false,
            8,
        )
        .unwrap();
        store
            .persist_package_at(uncert_release, t0 + Duration::minutes(1))
            .unwrap();
        assert!(matches!(
            store.get_package(&pkg().canonical()).unwrap().unwrap().kind,
            PackageFreshnessKind::Quarantined { .. }
        ));

        let cert_release = record_from_verdict_certified(
            &verdict(verified_hash(&hash), t0 + Duration::minutes(2)),
            t0 + Duration::minutes(2),
            true,
            9,
        )
        .unwrap();
        store
            .persist_package_at(cert_release, t0 + Duration::minutes(2))
            .unwrap();
        assert_eq!(
            store.get_package(&pkg().canonical()).unwrap().unwrap().kind,
            PackageFreshnessKind::Verified
        );
    }

    #[test]
    fn legacy_v1_journal_deserializes_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("freshness-state.json");
        let t0 = now();
        let legacy_json = serde_json::json!({
            "packages": {
                pkg().canonical(): {
                    "canonical": pkg().canonical(),
                    "kind": "revoked",
                    "resolved_at": t0,
                    "content_hash": null
                }
            },
            "accepted_head": null
        });
        std::fs::write(&path, serde_json::to_vec(&legacy_json).unwrap()).unwrap();

        let store = FreshnessStore::open_path(path);
        assert_eq!(store.last_tip_height().unwrap(), 0);
        let rec = store.get_package(&pkg().canonical()).unwrap().unwrap();
        assert!(
            rec.certified,
            "legacy records must default to certified=true"
        );
        assert_eq!(rec.height, 0);
        match evaluate_package_freshness(
            Some(&rec),
            &verdict(verified_hash("aa"), t0 + Duration::seconds(10)),
            t0 + Duration::seconds(10),
            Duration::seconds(60),
        ) {
            Some(InstallDecision::Block { code, .. }) => assert_eq!(code, "freshness_rollback"),
            other => panic!("expected freshness_rollback, got {other:?}"),
        }
    }

    #[test]
    fn corrupt_journal_is_quarantined_and_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("freshness-state.json");
        std::fs::write(&path, b"{truncated").unwrap();
        let store = FreshnessStore::open_path(path.clone());

        assert!(store.get_package(&pkg().canonical()).is_err());
        assert!(!path.exists(), "corrupt journal must be quarantined");
        let quarantined: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("freshness-state.json.corrupt-")
            })
            .collect();
        assert_eq!(quarantined.len(), 1);

        std::fs::write(
            &path,
            format!(
                r#"{{"schema_version":{},"packages":{{}}}}"#,
                JOURNAL_SCHEMA_VERSION + 1
            ),
        )
        .unwrap();
        assert!(store.get_package(&pkg().canonical()).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn concurrent_journal_writers_preserve_all_updates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("freshness-state.json");
        let t0 = now();
        let handles: Vec<_> = (0..8)
            .map(|worker| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let store = FreshnessStore::open_path(path);
                    for i in 0..5 {
                        let canonical = format!("npm:pkg-{worker}-{i}@1.0.0");
                        store
                            .persist_package_at(
                                PackageFreshnessRecord {
                                    canonical,
                                    kind: PackageFreshnessKind::Verified,
                                    resolved_at: t0,
                                    content_hash: Some(format!("{worker:02x}{i:02x}").repeat(16)),
                                    certified: true,
                                    height: (worker * 5 + i + 1) as u64,
                                },
                                t0,
                            )
                            .unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let store = FreshnessStore::open_path(path);
        for worker in 0..8 {
            for i in 0..5 {
                let canonical = format!("npm:pkg-{worker}-{i}@1.0.0");
                assert!(
                    store.get_package(&canonical).unwrap().is_some(),
                    "missing {canonical} after concurrent writes"
                );
            }
        }
    }
}
