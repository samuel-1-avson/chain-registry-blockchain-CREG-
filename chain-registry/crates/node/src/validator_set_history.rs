// crates/node/src/validator_set_history.rs
//
// Height-indexed history of the active validator set (ISSUE-050 / ISSUE-016).
//
// Block signatures must be verified against the validator set that was active
// AT THE BLOCK'S HEIGHT, not the current tip set. Otherwise a joining or
// long-range-syncing node can accept a backdated block signed by today's
// validators (or by keys that were ejected after the claimed height).
//
// Security-sensitive readers use `verification_set_at` / `authenticated_at_checked`:
// authority is the certified transition chain rooted in the immutable
// deployment bootstrap, or in a configured weak-subjectivity checkpoint when
// `height` is at or after that checkpoint. Missing or broken history fails
// closed. `set_at` / `authenticated_at` remain observational only and must
// not be used for sync, committee, or admission decisions.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::Context;
use common::{AuthenticatedValidatorSetSnapshot, ValidatorSet};
use serde::{Deserialize, Serialize};

const HISTORY_FILE: &str = "validator-set-history.json";

#[cfg(test)]
tokio::task_local! {
    static TEST_CHECKPOINT_ENV_OVERRIDE: std::cell::RefCell<Option<String>>;
}

#[cfg(test)]
pub(crate) async fn with_checkpoint_env_override<T>(
    value: Option<String>,
    future: impl std::future::Future<Output = T>,
) -> T {
    TEST_CHECKPOINT_ENV_OVERRIDE
        .scope(std::cell::RefCell::new(value), future)
        .await
}

fn checkpoint_environment_value() -> Option<String> {
    #[cfg(test)]
    if let Ok(value) = TEST_CHECKPOINT_ENV_OVERRIDE.try_with(|current| current.borrow().clone()) {
        return value;
    }
    std::env::var("CREG_WEAK_SUBJECTIVITY_CHECKPOINT").ok()
}

/// Load a deployment-provisioned weak-subjectivity checkpoint. It is never
/// accepted from the network; operators must provision the same certificate
/// out of band for a new/rebuilding node.
pub fn checkpoint_from_env() -> Result<Option<AuthenticatedValidatorSetSnapshot>, String> {
    let Some(raw) = checkpoint_environment_value() else {
        return Ok(None);
    };
    let raw = raw.trim();
    // Deployment manifests deliberately pass an empty value until an
    // independently verified certificate is published. Empty means the
    // compatibility rollout state; malformed non-empty JSON remains fatal.
    if raw.is_empty() {
        return Ok(None);
    }
    let checkpoint: AuthenticatedValidatorSetSnapshot = serde_json::from_str(raw)
        .map_err(|error| format!("invalid CREG_WEAK_SUBJECTIVITY_CHECKPOINT: {error}"))?;
    checkpoint
        .validate_structure()
        .map_err(|error| format!("invalid CREG_WEAK_SUBJECTIVITY_CHECKPOINT: {error}"))?;
    Ok(Some(checkpoint))
}

/// Maximum compatibility snapshots retained when no configured checkpoint is
/// present. Once a weak-subjectivity checkpoint is configured, its complete
/// descendant chain is retained; pruning a middle certificate would silently
/// destroy the very ancestry proof the checkpoint is meant to provide.
const MAX_SNAPSHOTS: usize = 512;
static HISTORY_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn history_write_guard() -> anyhow::Result<std::sync::MutexGuard<'static, ()>> {
    HISTORY_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("validator-set history write lock is poisoned"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidatorSetSnapshot {
    /// First block height for which this set is authoritative.
    pub effective_from_height: u64,
    pub validators: ValidatorSet,
    /// Absent for pre-certificate compatibility entries. Such entries may be
    /// used only for legacy block verification, never as a committee-selection
    /// trust anchor.
    #[serde(default)]
    pub certificate: Option<AuthenticatedValidatorSetSnapshot>,
}

/// Classifies failures from the authenticated history writer without relying
/// on error-message matching at API or gossip boundaries.
#[derive(Debug, thiserror::Error)]
pub enum RecordAuthenticatedError {
    /// The received certificate itself is invalid or lacks a prior-set quorum.
    #[error("invalid validator-set transition: {0}")]
    InvalidCertificate(String),
    /// The certificate can be valid in isolation but cannot extend the local
    /// canonical transition history.
    #[error("{0}")]
    Conflict(String),
    /// The receiver could not safely read, retain, serialize, or durably
    /// replace its local history. This is not evidence of malicious gossip.
    #[error("validator-set history persistence failed: {source}")]
    Persistence {
        #[source]
        source: anyhow::Error,
    },
}

impl RecordAuthenticatedError {
    pub(crate) fn persistence(source: impl Into<anyhow::Error>) -> Self {
        Self::Persistence {
            source: source.into(),
        }
    }

    pub fn is_persistence(&self) -> bool {
        matches!(self, Self::Persistence { .. })
    }
}

fn history_path(data_dir: &Path) -> PathBuf {
    data_dir.join(HISTORY_FILE)
}

fn validate_history_bytes(bytes: &[u8]) -> anyhow::Result<()> {
    serde_json::from_slice::<Vec<ValidatorSetSnapshot>>(bytes)
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("invalid validator-set history snapshot: {error}"))
}

#[cfg(unix)]
fn sync_parent_directory(data_dir: &Path) -> anyhow::Result<()> {
    std::fs::File::open(data_dir)?.sync_all()?;
    Ok(())
}

#[cfg(all(not(unix), not(windows)))]
fn sync_parent_directory(_data_dir: &Path) -> anyhow::Result<()> {
    anyhow::bail!("durable validator-history directory sync is unsupported on this platform")
}

#[cfg(windows)]
fn durable_replace(from: &Path, to: &Path, _data_dir: &Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let from_wide: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to_wide: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let moved = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "durably replace validator history {} with {}",
                to.display(),
                from.display()
            )
        });
    }
    Ok(())
}

#[cfg(not(windows))]
fn durable_replace(from: &Path, to: &Path, data_dir: &Path) -> anyhow::Result<()> {
    std::fs::rename(from, to)?;
    // The rename is not committed until its parent metadata is durable.
    sync_parent_directory(data_dir)
}

fn durable_remove_history(data_dir: &Path) -> anyhow::Result<()> {
    let path = history_path(data_dir);
    if !path.exists() {
        return Ok(());
    }

    // A durably renamed tombstone makes the active history path absent before
    // the journal can be cleared. If the final unlink is lost in a crash, only
    // the ignored tombstone can reappear.
    let tombstone = path.with_extension("json.removed");
    durable_replace(&path, &tombstone, data_dir)?;
    match std::fs::remove_file(&tombstone) {
        Ok(()) => {
            #[cfg(not(windows))]
            sync_parent_directory(data_dir)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Persist a complete history replacement durably before reporting success.
/// Callers hold `history_write_guard`, so the fixed temporary path cannot race
/// another local history writer.
fn persist_history_bytes(data_dir: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    validate_history_bytes(bytes)?;
    atomic_write_bytes(&history_path(data_dir), bytes)
}

/// Durably write `bytes` to `path` via a sibling temporary file, `sync_all`,
/// and platform-specific atomic replacement.
pub(crate) fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state.json");
    let tmp = parent.join(format!(".{file_name}.tmp.{}", uuid::Uuid::new_v4()));
    let mut file = std::fs::File::create(&tmp)?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&tmp);
        return Err(error.into());
    }
    drop(file);
    if let Err(error) = durable_replace(&tmp, path, parent) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    Ok(())
}

/// Run `persist` while the exact pre-reorg history snapshot is protected from
/// concurrent local writers. The closure durably writes the RocksDB journal,
/// making that journal the transaction boundary for subsequent history edits.
#[allow(dead_code)]
pub(crate) fn with_reorg_snapshot<T>(
    data_dir: &Path,
    persist: impl FnOnce(Option<Vec<u8>>) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let _write_guard = history_write_guard()?;
    let snapshot = match std::fs::read(history_path(data_dir)) {
        Ok(bytes) => {
            validate_history_bytes(&bytes)?;
            Some(bytes)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    persist(snapshot)
}

/// Idempotently restore the history captured by an active reorg journal.
/// The journal remains present if this fails, so startup can retry safely.
pub(crate) fn restore_reorg_snapshot(
    data_dir: &Path,
    snapshot: Option<&[u8]>,
) -> anyhow::Result<()> {
    let _write_guard = history_write_guard()?;
    match snapshot {
        Some(bytes) => persist_history_bytes(data_dir, bytes),
        None => durable_remove_history(data_dir),
    }
}

/// Load and structurally validate the local history. This does not establish
/// authority: certificate quorum and prior-set continuity require an explicit
/// trust root and are checked by the strict authenticated accessors below.
pub fn load_checked(data_dir: &Path) -> anyhow::Result<Vec<ValidatorSetSnapshot>> {
    let mut snaps: Vec<ValidatorSetSnapshot> = match std::fs::read(history_path(data_dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("parse {}", history_path(data_dir).display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read {}", history_path(data_dir).display()));
        }
    };
    snaps.sort_by_key(|s| s.effective_from_height);
    for pair in snaps.windows(2) {
        if pair[0].effective_from_height == pair[1].effective_from_height {
            anyhow::bail!(
                "validator-set history contains duplicate effective height {}",
                pair[0].effective_from_height
            );
        }
    }
    for entry in &snaps {
        let Some(certificate) = entry.certificate.as_ref() else {
            continue;
        };
        certificate.validate_structure().map_err(|error| {
            anyhow::anyhow!(
                "validator-set history contains a malformed certificate at height {}: {}",
                entry.effective_from_height,
                error
            )
        })?;
        if entry.effective_from_height != certificate.effective_from_height
            || common::canonical_validator_set_hash(&entry.validators)
                != certificate.validator_set_hash
            || common::canonical_validator_set_hash(&certificate.validators)
                != certificate.validator_set_hash
        {
            anyhow::bail!(
                "validator-set history contains an inconsistent certificate at height {}",
                entry.effective_from_height
            );
        }
    }
    Ok(snaps)
}

/// Load all snapshots (ascending `effective_from_height`). Empty if missing or
/// unreadable. Security-sensitive writers use `load_checked` and fail closed;
/// this compatibility accessor retains the historical fallback contract for
/// callers that do not make a new authority decision.
pub fn load(data_dir: &Path) -> Vec<ValidatorSetSnapshot> {
    load_checked(data_dir).unwrap_or_default()
}

/// Sorted, lowercased pubkey list — the membership identity used to decide
/// whether the set actually changed (ignores ordering / status churn).
fn membership_key(set: &ValidatorSet) -> Vec<String> {
    let mut keys: Vec<String> = set
        .validators
        .iter()
        .map(|v| v.pubkey.to_ascii_lowercase())
        .collect();
    keys.sort();
    keys
}

fn enforce_retention_for_checkpoint_hash(
    snapshots: &mut Vec<ValidatorSetSnapshot>,
    checkpoint_hash: Option<&str>,
) {
    let configured_checkpoint_is_present = checkpoint_hash
        .map(|checkpoint_hash| {
            snapshots.iter().any(|entry| {
                entry
                    .certificate
                    .as_ref()
                    .map(common::validator_snapshot_certificate_hash)
                    .as_deref()
                    == Some(checkpoint_hash)
            })
        })
        .unwrap_or(false);
    if configured_checkpoint_is_present || snapshots.len() <= MAX_SNAPSHOTS {
        return;
    }
    let overflow = snapshots.len() - MAX_SNAPSHOTS;
    snapshots.drain(0..overflow);
}

fn enforce_retention(snapshots: &mut Vec<ValidatorSetSnapshot>) -> anyhow::Result<()> {
    let checkpoint_hash = checkpoint_from_env()
        .map_err(anyhow::Error::msg)?
        .map(|checkpoint| common::validator_snapshot_certificate_hash(&checkpoint));
    enforce_retention_for_checkpoint_hash(snapshots, checkpoint_hash.as_deref());
    Ok(())
}

/// Select the snapshot active at `height`: the one with the greatest
/// `effective_from_height <= height`. Pure; unit-tested.
pub fn select_at(snapshots: &[ValidatorSetSnapshot], height: u64) -> Option<&ValidatorSetSnapshot> {
    snapshots
        .iter()
        .filter(|s| s.effective_from_height <= height)
        .max_by_key(|s| s.effective_from_height)
}

/// Observational lookup of the validator set recorded at `height`.
///
/// This does not establish certificate ancestry. Security-sensitive callers
/// must use `verification_set_at` instead of treating `None` as "use the
/// current tip set".
pub fn set_at(data_dir: &Path, height: u64) -> Option<ValidatorSet> {
    set_at_checked(data_dir, height).ok().flatten()
}

pub fn set_at_checked(data_dir: &Path, height: u64) -> anyhow::Result<Option<ValidatorSet>> {
    let snapshots = load_checked(data_dir)?;
    let Some(snap) = select_at(&snapshots, height) else {
        return Ok(None);
    };
    if snap.validators.validators.is_empty() {
        Ok(None)
    } else {
        Ok(Some(snap.validators.clone()))
    }
}

/// Lossy observational lookup of the latest certificate at `height`.
///
/// This intentionally does not establish certificate ancestry and must never
/// be used to make a committee, consensus, or admission decision. Security
/// boundaries use `authenticated_at_checked` with an explicit trusted
/// bootstrap set, or the configured-checkpoint accessor below.
pub fn authenticated_at(data_dir: &Path, height: u64) -> Option<AuthenticatedValidatorSetSnapshot> {
    let snapshots = load(data_dir);
    select_at(&snapshots, height).and_then(|snapshot| snapshot.certificate.clone())
}

/// Return a certificate only after validating the complete persisted
/// certificate chain against the deployment-provisioned bootstrap set.
///
/// Unsigned compatibility entries may describe the currently authenticated
/// set, but may not change it. Every certified transition must continue the
/// preceding authenticated set and carry its quorum. The full retained chain
/// is checked even when `height` selects an earlier certificate, so a poisoned
/// tail cannot be silently ignored by a security-sensitive reader.
pub fn authenticated_at_checked(
    data_dir: &Path,
    height: u64,
    trusted_bootstrap: &ValidatorSet,
) -> anyhow::Result<Option<AuthenticatedValidatorSetSnapshot>> {
    let snapshots = load_checked(data_dir)?;
    let mut current_hash = common::canonical_validator_set_hash(trusted_bootstrap);
    let mut current_set = trusted_bootstrap.clone();
    let mut selected = None;
    let mut authority_started = false;

    for entry in &snapshots {
        let Some(certificate) = entry.certificate.as_ref() else {
            if authority_started {
                anyhow::bail!(
                    "validator-set history contains an unsigned entry after authenticated authority starts at height {}",
                    entry.effective_from_height
                );
            }
            if common::canonical_validator_set_hash(&entry.validators) != current_hash {
                anyhow::bail!(
                    "validator-set history contains an unsigned authority change at height {}",
                    entry.effective_from_height
                );
            }
            continue;
        };
        if certificate.previous_validator_set_hash != current_hash {
            anyhow::bail!(
                "validator-set history breaks trusted-bootstrap continuity at height {}",
                entry.effective_from_height
            );
        }
        certificate.validate_against(&current_set).map_err(|error| {
            anyhow::anyhow!(
                "validator-set history contains an invalid authenticated transition at height {}: {}",
                entry.effective_from_height,
                error
            )
        })?;
        current_hash = certificate.validator_set_hash.clone();
        current_set = certificate.validators.clone();
        authority_started = true;
        if entry.effective_from_height <= height {
            selected = Some(certificate.clone());
        }
    }

    Ok(selected)
}

/// Return a certificate only when the local history contains the configured
/// checkpoint and every subsequent certificate forms one continuous
/// validator-set chain. This prevents a rebuilt node from accepting a
/// self-consistent but long-range alternative history.
pub fn authenticated_at_from_checkpoint(
    data_dir: &Path,
    height: u64,
    checkpoint: &AuthenticatedValidatorSetSnapshot,
) -> Option<AuthenticatedValidatorSetSnapshot> {
    authenticated_at_from_checkpoint_checked(data_dir, height, checkpoint)
        .ok()
        .flatten()
}

pub fn authenticated_at_from_checkpoint_checked(
    data_dir: &Path,
    height: u64,
    checkpoint: &AuthenticatedValidatorSetSnapshot,
) -> anyhow::Result<Option<AuthenticatedValidatorSetSnapshot>> {
    let snapshots = load_checked(data_dir)?;
    let checkpoint_hash = common::validator_snapshot_certificate_hash(checkpoint);
    let mut current_hash = checkpoint.validator_set_hash.clone();
    let mut current_set = checkpoint.validators.clone();
    let mut found = false;
    for entry in &snapshots {
        if !found {
            // Compatibility/bootstrap entries may predate the configured
            // checkpoint and therefore legitimately have no certificate.
            // They are not authority anchors; skip them while searching for
            // the exact out-of-band checkpoint.
            if let Some(certificate) = entry.certificate.as_ref() {
                if common::validator_snapshot_certificate_hash(certificate) == checkpoint_hash {
                    if entry.effective_from_height != certificate.effective_from_height
                        || common::canonical_validator_set_hash(&entry.validators)
                            != certificate.validator_set_hash
                    {
                        anyhow::bail!(
                            "configured checkpoint has an inconsistent local history entry"
                        );
                    }
                    found = true;
                    current_hash = certificate.validator_set_hash.clone();
                    current_set = certificate.validators.clone();
                }
            }
            continue;
        }
        // Once anchored, every subsequent history entry must be certified.
        let certificate = entry.certificate.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "validator-set history contains an unsigned entry after the configured checkpoint"
            )
        })?;
        if entry.effective_from_height != certificate.effective_from_height
            || common::canonical_validator_set_hash(&entry.validators)
                != certificate.validator_set_hash
        {
            anyhow::bail!(
                "validator-set history contains an inconsistent checkpoint descendant at height {}",
                entry.effective_from_height
            );
        }
        if certificate.previous_validator_set_hash != current_hash {
            anyhow::bail!(
                "validator-set history breaks checkpoint continuity at height {}",
                entry.effective_from_height
            );
        }
        if let Err(error) = certificate.validate_against(&current_set) {
            anyhow::bail!(
                "validator-set history contains an invalid checkpoint descendant at height {}: {}",
                entry.effective_from_height,
                error
            );
        }
        current_hash = certificate.validator_set_hash.clone();
        current_set = certificate.validators.clone();
    }
    if !found {
        return Ok(None);
    }
    if height < checkpoint.effective_from_height {
        return Ok(None);
    }
    let selected = select_at(&snapshots, height).ok_or_else(|| {
        anyhow::anyhow!("configured checkpoint history has no entry at requested height {height}")
    })?;
    let certificate = selected.certificate.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "validator-set history selected an unsigned checkpoint descendant at height {}",
            selected.effective_from_height
        )
    })?;
    Ok(Some(certificate))
}

/// Resolve committee authority under the configured weak-subjectivity policy.
/// Malformed checkpoint configuration is an error, never a legacy fallback.
pub fn committee_authority_at(
    data_dir: &Path,
    height: u64,
    trusted_bootstrap: &ValidatorSet,
) -> Result<Option<AuthenticatedValidatorSetSnapshot>, String> {
    match checkpoint_from_env()? {
        Some(checkpoint) => authenticated_at_from_checkpoint_checked(data_dir, height, &checkpoint)
            .map_err(|error| format!("validator-set history unavailable: {error}")),
        None => authenticated_at_checked(data_dir, height, trusted_bootstrap)
            .map_err(|error| format!("validator-set history unavailable: {error}")),
    }
}

/// Resolve the validator set used for historical block verification. At and
/// after a configured checkpoint this is fail-closed; earlier/no-checkpoint
/// history is authenticated from the immutable deployment bootstrap.
pub fn verification_set_at(
    data_dir: &Path,
    height: u64,
    trusted_bootstrap: &ValidatorSet,
) -> Result<ValidatorSet, String> {
    let checkpoint = checkpoint_from_env()?;
    verification_set_at_with_checkpoint(data_dir, height, trusted_bootstrap, checkpoint.as_ref())
}

/// Return the highest-height certified validator-set transition in `data_dir`.
pub fn latest_certified_transition(
    data_dir: &Path,
) -> anyhow::Result<Option<AuthenticatedValidatorSetSnapshot>> {
    let snapshots = load_checked(data_dir)?;
    Ok(snapshots
        .into_iter()
        .filter_map(|entry| entry.certificate)
        .max_by_key(|cert| cert.effective_from_height))
}

/// Return the earliest certified transition height at which `(node_id, pubkey, evm_address)`
/// appears as an active member.
pub fn certified_activation_height_for(
    data_dir: &Path,
    node_id: &str,
    pubkey: &str,
    evm_address: &str,
) -> anyhow::Result<Option<u64>> {
    let snapshots = load_checked(data_dir)?;
    let normalized_pk = pubkey.trim().trim_start_matches("0x").to_ascii_lowercase();
    let normalized_evm = evm_address.trim().to_ascii_lowercase();
    Ok(snapshots
        .into_iter()
        .filter_map(|entry| entry.certificate)
        .find(|cert| {
            cert.validators.validators.iter().any(|validator| {
                validator.id == node_id
                    && validator.pubkey.to_ascii_lowercase() == normalized_pk
                    && validator.eth_address.to_ascii_lowercase() == normalized_evm
                    && common::validator_status_is_active(&validator.status)
            })
        })
        .map(|cert| cert.effective_from_height))
}

/// A new signing node must already have verified the chain block anchoring its
/// admission and the preceding validators' quorum certificate. L1 status and
/// unsigned history alone cannot enable a local signing key.
pub(crate) fn require_phase_c_signing_membership(
    data_dir: &Path,
    next_height: u64,
    trusted_bootstrap: &ValidatorSet,
    node_id: &str,
    pubkey: &str,
    canonical_block_hash: impl FnOnce(u64) -> anyhow::Result<Option<String>>,
) -> anyhow::Result<()> {
    let matches = |set: &ValidatorSet| {
        set.validators.iter().any(|validator| {
            validator.id == node_id
                && validator.pubkey == pubkey
                && common::validator_status_is_active(&validator.status)
        })
    };
    if matches(trusted_bootstrap) {
        return Ok(());
    }
    let Some(certificate) = authenticated_at_checked(data_dir, next_height, trusted_bootstrap)?
    else {
        anyhow::bail!("local Phase C signing identity has no certified validator admission");
    };
    if !matches(&certificate.validators) {
        anyhow::bail!("local Phase C signing identity is outside the certified validator set");
    }
    let parent_height = certificate
        .effective_from_height
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("validator admission certificate has no parent height"))?;
    if canonical_block_hash(parent_height)?.as_deref()
        != Some(certificate.previous_block_hash.as_str())
    {
        anyhow::bail!(
            "validator admission certificate does not anchor to the local canonical chain"
        );
    }
    let authority = verification_set_at(data_dir, next_height, trusted_bootstrap)
        .map_err(anyhow::Error::msg)?;
    if common::canonical_validator_set_hash(&authority) != certificate.validator_set_hash {
        anyhow::bail!("validator admission certificate conflicts with the configured authority");
    }
    Ok(())
}

pub(crate) fn verification_set_at_with_checkpoint(
    data_dir: &Path,
    height: u64,
    trusted_bootstrap: &ValidatorSet,
    checkpoint: Option<&AuthenticatedValidatorSetSnapshot>,
) -> Result<ValidatorSet, String> {
    match checkpoint {
        Some(checkpoint) if height >= checkpoint.effective_from_height => {
            authenticated_at_from_checkpoint_checked(data_dir, height, checkpoint)
                .map_err(|error| format!("validator-set history unavailable: {error}"))?
                .map(|snapshot| snapshot.validators)
                .ok_or_else(|| {
                    format!("no checkpoint-descended validator authority at height {height}")
                })
        }
        _ => Ok(
            authenticated_at_checked(data_dir, height, trusted_bootstrap)
                .map_err(|error| format!("validator-set history unavailable: {error}"))?
                .map(|snapshot| snapshot.validators)
                .unwrap_or_else(|| trusted_bootstrap.clone()),
        ),
    }
}

pub fn certified_committees_required() -> bool {
    std::env::var("CREG_REQUIRE_CERTIFIED_COMMITTEES")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Structural receiver gate shared by API/gossip tests: a live transition is
/// valid only for the next height and must name the receiver's canonical tip.
pub fn validate_transition_at_tip(
    snapshot: &AuthenticatedValidatorSetSnapshot,
    tip_height: u64,
    tip_hash: &str,
) -> anyhow::Result<()> {
    if snapshot.effective_from_height != tip_height.saturating_add(1) {
        anyhow::bail!("validator-set transition must become effective at the next block height");
    }
    if snapshot.previous_block_hash != tip_hash {
        anyhow::bail!("validator-set transition prior block hash does not match the local tip");
    }
    Ok(())
}

/// Record `set` as effective from `effective_from_height`, but only if its
/// membership differs from the most recent snapshot (dedup). Atomic write.
pub fn record(
    data_dir: &Path,
    effective_from_height: u64,
    set: &ValidatorSet,
) -> anyhow::Result<()> {
    let _write_guard = history_write_guard()?;
    if set.validators.is_empty() {
        return Ok(());
    }
    let mut snapshots = load_checked(data_dir)?;

    if snapshots.iter().any(|snapshot| {
        snapshot.effective_from_height == effective_from_height && snapshot.certificate.is_some()
    }) {
        anyhow::bail!("legacy validator-set update cannot replace a certified transition");
    }

    if let Some(last) = snapshots.last() {
        if membership_key(&last.validators) == membership_key(set) {
            return Ok(()); // unchanged membership — nothing to record
        }
    }

    // If a snapshot already exists for this exact height, replace it (a later
    // reconcile at the same tip supersedes the earlier one).
    snapshots.retain(|s| s.effective_from_height != effective_from_height);
    snapshots.push(ValidatorSetSnapshot {
        effective_from_height,
        validators: set.clone(),
        certificate: None,
    });
    snapshots.sort_by_key(|s| s.effective_from_height);
    enforce_retention(&mut snapshots)?;

    let bytes = serde_json::to_vec_pretty(&snapshots)?;
    persist_history_bytes(data_dir, &bytes)
}

/// Remove authority entries whose anchor lies on an abandoned branch. A
/// transition effective at `common_ancestor + 1` remains valid because it is
/// anchored to the common ancestor itself.
pub fn truncate_after_reorg(data_dir: &Path, common_ancestor: u64) -> anyhow::Result<()> {
    let _write_guard = history_write_guard()?;
    let mut snapshots = load_checked(data_dir)?;
    let max_effective_height = common_ancestor.saturating_add(1);
    let original_len = snapshots.len();
    snapshots.retain(|snapshot| snapshot.effective_from_height <= max_effective_height);
    if snapshots.len() == original_len {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(&snapshots)?;
    persist_history_bytes(data_dir, &bytes)
}

/// Persist a transition only after its certificate verifies against the prior
/// active set. This is the sole writer for security-sensitive snapshot anchors.
pub fn record_authenticated(
    data_dir: &Path,
    snapshot: AuthenticatedValidatorSetSnapshot,
    previous: &ValidatorSet,
) -> Result<(), RecordAuthenticatedError> {
    let _write_guard = history_write_guard().map_err(RecordAuthenticatedError::persistence)?;
    snapshot
        .validate_against(previous)
        .map_err(|error| RecordAuthenticatedError::InvalidCertificate(error.to_string()))?;
    let mut snapshots = load_checked(data_dir).map_err(RecordAuthenticatedError::persistence)?;
    if let Some(active) = select_at(&snapshots, snapshot.effective_from_height.saturating_sub(1)) {
        if common::canonical_validator_set_hash(&active.validators)
            != common::canonical_validator_set_hash(previous)
        {
            return Err(RecordAuthenticatedError::Conflict(
                "validator-set transition does not continue the persisted active set".into(),
            ));
        }
    }
    if let Some(existing) = snapshots
        .iter()
        .find(|existing| existing.effective_from_height == snapshot.effective_from_height)
    {
        if common::canonical_validator_set_hash(&existing.validators)
            != common::canonical_validator_set_hash(&snapshot.validators)
        {
            return Err(RecordAuthenticatedError::Conflict(
                "conflicting validator-set transition at the same effective height".into(),
            ));
        }
        if let Some(existing_certificate) = &existing.certificate {
            if common::validator_snapshot_certificate_hash(existing_certificate)
                == common::validator_snapshot_certificate_hash(&snapshot)
            {
                return Ok(()); // exact idempotent replay
            }
            return Err(RecordAuthenticatedError::Conflict(
                "conflicting validator-set certificate at the same effective height".into(),
            ));
        }
    }
    snapshots.retain(|existing| existing.effective_from_height != snapshot.effective_from_height);
    snapshots.push(ValidatorSetSnapshot {
        effective_from_height: snapshot.effective_from_height,
        validators: snapshot.validators.clone(),
        certificate: Some(snapshot),
    });
    snapshots.sort_by_key(|snapshot| snapshot.effective_from_height);
    enforce_retention(&mut snapshots).map_err(RecordAuthenticatedError::persistence)?;
    let bytes =
        serde_json::to_vec_pretty(&snapshots).map_err(RecordAuthenticatedError::persistence)?;
    persist_history_bytes(data_dir, &bytes).map_err(RecordAuthenticatedError::persistence)
}

/// Install the exact out-of-band weak-subjectivity checkpoint as a local trust
/// root when pre-checkpoint transition history has been pruned. The configured
/// certificate is persisted (never untrusted peer witness bytes), and every
/// later descendant still goes through `record_authenticated`.
pub(crate) fn record_configured_checkpoint(
    data_dir: &Path,
    proposed: &AuthenticatedValidatorSetSnapshot,
) -> Result<(), RecordAuthenticatedError> {
    let configured = checkpoint_from_env()
        .map_err(|error| RecordAuthenticatedError::InvalidCertificate(error.to_string()))?
        .ok_or_else(|| {
            RecordAuthenticatedError::Conflict(
                "cannot install validator checkpoint without configured weak-subjectivity trust root"
                    .into(),
            )
        })?;
    record_configured_checkpoint_against(data_dir, proposed, &configured)
}

fn record_configured_checkpoint_against(
    data_dir: &Path,
    proposed: &AuthenticatedValidatorSetSnapshot,
    configured: &AuthenticatedValidatorSetSnapshot,
) -> Result<(), RecordAuthenticatedError> {
    let configured_hash = common::validator_snapshot_certificate_hash(configured);
    if configured_hash != common::validator_snapshot_certificate_hash(proposed) {
        return Err(RecordAuthenticatedError::Conflict(
            "proposed validator checkpoint does not match the configured trust root".into(),
        ));
    }

    let _write_guard = history_write_guard().map_err(RecordAuthenticatedError::persistence)?;
    let snapshots = load_checked(data_dir).map_err(RecordAuthenticatedError::persistence)?;
    let mut reconciled: Vec<_> = snapshots
        .iter()
        .filter(|entry| entry.effective_from_height < configured.effective_from_height)
        .cloned()
        .collect();
    reconciled.push(ValidatorSetSnapshot {
        effective_from_height: configured.effective_from_height,
        validators: configured.validators.clone(),
        certificate: Some(configured.clone()),
    });

    let mut current_hash = configured.validator_set_hash.clone();
    let mut current_set = configured.validators.clone();
    for entry in snapshots
        .into_iter()
        .filter(|entry| entry.effective_from_height > configured.effective_from_height)
    {
        let Some(certificate) = entry.certificate.as_ref() else {
            // Compatibility writers may have raced ahead of checkpoint
            // installation. An unsigned entry is not a descendant and is
            // deliberately removed rather than promoted by local assertion.
            continue;
        };
        if certificate.previous_validator_set_hash != current_hash {
            // A structurally valid certificate on another local branch is not
            // descended from the configured trust root. Discard that branch;
            // any of its children will likewise fail continuity below.
            continue;
        }
        certificate.validate_against(&current_set).map_err(|error| {
            RecordAuthenticatedError::persistence(anyhow::anyhow!(
                "local validator-set history contains an invalid configured-checkpoint descendant at height {}: {}",
                entry.effective_from_height,
                error
            ))
        })?;
        current_hash = certificate.validator_set_hash.clone();
        current_set = certificate.validators.clone();
        reconciled.push(entry);
    }

    reconciled.sort_by_key(|entry| entry.effective_from_height);
    enforce_retention_for_checkpoint_hash(&mut reconciled, Some(&configured_hash));
    let bytes =
        serde_json::to_vec_pretty(&reconciled).map_err(RecordAuthenticatedError::persistence)?;
    persist_history_bytes(data_dir, &bytes).map_err(RecordAuthenticatedError::persistence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{Validator, VALIDATOR_SNAPSHOT_SCHEMA_VERSION};
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    fn validator(id: &str, pubkey: &str) -> Validator {
        Validator {
            id: id.into(),
            alias: id.into(),
            pubkey: pubkey.into(),
            eth_address: format!("0x{}", id),
            stake: 100,
            reputation: 100,
            status: "online".into(),
            performance_score: 500,
            contribution_count: 0,
            pq_pubkey: None,
        }
    }

    fn set(pubkeys: &[&str]) -> ValidatorSet {
        ValidatorSet::new(
            pubkeys
                .iter()
                .enumerate()
                .map(|(i, pk)| validator(&format!("n{i}"), pk))
                .collect(),
        )
    }

    fn keyed_set(keys: &[SigningKey]) -> ValidatorSet {
        ValidatorSet::new(
            keys.iter()
                .enumerate()
                .map(|(index, key)| {
                    validator(
                        &format!("v{index}"),
                        &hex::encode(key.verifying_key().as_bytes()),
                    )
                })
                .collect(),
        )
    }

    fn signed_transition(
        effective_from_height: u64,
        previous: &ValidatorSet,
        next: &ValidatorSet,
        signing_keys: &[SigningKey],
    ) -> AuthenticatedValidatorSetSnapshot {
        let mut snapshot = AuthenticatedValidatorSetSnapshot {
            schema_version: VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
            effective_from_height,
            previous_block_hash: format!("{:064x}", effective_from_height.saturating_sub(1)),
            previous_validator_set_hash: common::canonical_validator_set_hash(previous),
            validators: next.clone(),
            validator_set_hash: common::canonical_validator_set_hash(next),
            transition_signatures: Vec::new(),
        };
        let message = common::validator_set_transition_message(&snapshot);
        let quorum = (2 * previous.validators.len()) / 3 + 1;
        for (validator, key) in previous.validators.iter().zip(signing_keys).take(quorum) {
            snapshot
                .transition_signatures
                .push(common::ValidatorSetTransitionSignature {
                    validator_id: validator.id.clone(),
                    validator_pubkey: validator.pubkey.clone(),
                    signature: hex::encode(key.sign(message.as_bytes()).to_bytes()),
                });
        }
        snapshot
    }

    fn write_history(data_dir: &Path, snapshots: &[ValidatorSetSnapshot]) {
        std::fs::write(
            history_path(data_dir),
            serde_json::to_vec_pretty(snapshots).expect("serialize history"),
        )
        .expect("write history");
    }

    #[test]
    fn select_at_picks_latest_not_after_height() {
        let snaps = vec![
            ValidatorSetSnapshot {
                effective_from_height: 0,
                validators: set(&["aa"]),
                certificate: None,
            },
            ValidatorSetSnapshot {
                effective_from_height: 10,
                validators: set(&["aa", "bb"]),
                certificate: None,
            },
            ValidatorSetSnapshot {
                effective_from_height: 20,
                validators: set(&["bb", "cc"]),
                certificate: None,
            },
        ];
        assert_eq!(select_at(&snaps, 5).unwrap().effective_from_height, 0);
        assert_eq!(select_at(&snaps, 10).unwrap().effective_from_height, 10);
        assert_eq!(select_at(&snaps, 19).unwrap().effective_from_height, 10);
        assert_eq!(select_at(&snaps, 100).unwrap().effective_from_height, 20);
    }

    #[test]
    fn record_dedupes_unchanged_membership() {
        let dir = tempfile::tempdir().expect("tempdir");
        record(dir.path(), 0, &set(&["aa"])).expect("record 0");
        // Same membership at a later height → no new snapshot.
        record(dir.path(), 5, &set(&["aa"])).expect("record 5 same");
        assert_eq!(load(dir.path()).len(), 1);

        // Changed membership → new snapshot.
        record(dir.path(), 6, &set(&["aa", "bb"])).expect("record 6 changed");
        let snaps = load(dir.path());
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[1].effective_from_height, 6);
    }

    #[test]
    fn reorg_snapshot_absence_durably_removes_active_history_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        record(dir.path(), 0, &set(&["aa"])).expect("record history");
        assert!(history_path(dir.path()).exists());

        restore_reorg_snapshot(dir.path(), None).expect("remove history");
        assert!(!history_path(dir.path()).exists());
        assert!(load(dir.path()).is_empty());

        // A later legitimate bootstrap write must not be obstructed by a
        // tombstone left by the durable-removal protocol.
        record(dir.path(), 0, &set(&["bb"])).expect("record replacement history");
        assert_eq!(load(dir.path())[0].validators.validators[0].pubkey, "bb");
    }

    #[test]
    fn set_at_returns_none_when_empty_and_some_otherwise() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(set_at(dir.path(), 1).is_none());
        record(dir.path(), 0, &set(&["aa", "bb"])).expect("record");
        let got = set_at(dir.path(), 50).expect("set present");
        assert_eq!(got.validators.len(), 2);
    }

    #[test]
    fn external_signing_requires_a_certified_admission_on_the_local_chain() {
        let data = tempfile::tempdir().unwrap();
        let keys: Vec<_> = (0..5).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let bootstrap = keyed_set(&keys[..4]);
        let admitted = keyed_set(&keys);
        let candidate = &admitted.validators[4];
        let certificate = signed_transition(22, &bootstrap, &admitted, &keys);
        let require = |height, key: &str, block_hash: &str| {
            require_phase_c_signing_membership(
                data.path(),
                height,
                &bootstrap,
                &candidate.id,
                key,
                |parent_height| {
                    assert_eq!(parent_height, 21);
                    Ok(Some(block_hash.to_string()))
                },
            )
        };
        record(data.path(), 0, &bootstrap).unwrap();
        assert!(require(22, &candidate.pubkey, &certificate.previous_block_hash).is_err());
        record_authenticated(data.path(), certificate.clone(), &bootstrap).unwrap();
        assert!(require(21, &candidate.pubkey, &certificate.previous_block_hash).is_err());
        assert!(require(22, &candidate.pubkey, &"0".repeat(64)).is_err());
        assert!(require(
            22,
            &bootstrap.validators[0].pubkey,
            &certificate.previous_block_hash
        )
        .is_err());
        require(22, &candidate.pubkey, &certificate.previous_block_hash).unwrap();
    }

    #[test]
    fn authenticated_transition_persists_and_replays_idempotently() {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let previous = ValidatorSet::new(
            keys.iter()
                .enumerate()
                .map(|(index, key)| {
                    validator(
                        &format!("v{index}"),
                        &hex::encode(key.verifying_key().as_bytes()),
                    )
                })
                .collect(),
        );
        record(dir.path(), 0, &previous).expect("configured bootstrap history");
        let mut transition = AuthenticatedValidatorSetSnapshot {
            schema_version: common::VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
            effective_from_height: 7,
            previous_block_hash: "a".repeat(64),
            previous_validator_set_hash: common::canonical_validator_set_hash(&previous),
            validators: previous.clone(),
            validator_set_hash: common::canonical_validator_set_hash(&previous),
            transition_signatures: vec![],
        };
        let message = common::validator_set_transition_message(&transition);
        for (index, key) in keys.iter().take(3).enumerate() {
            transition
                .transition_signatures
                .push(common::ValidatorSetTransitionSignature {
                    validator_id: format!("v{index}"),
                    validator_pubkey: hex::encode(key.verifying_key().as_bytes()),
                    signature: hex::encode(key.sign(message.as_bytes()).to_bytes()),
                });
        }
        record_authenticated(dir.path(), transition.clone(), &previous).expect("certified write");
        record_authenticated(dir.path(), transition.clone(), &previous).expect("idempotent replay");
        let mut reordered = transition.clone();
        reordered.transition_signatures.reverse();
        assert_eq!(
            common::validator_snapshot_certificate_hash(&reordered),
            common::validator_snapshot_certificate_hash(&transition)
        );
        record_authenticated(dir.path(), reordered, &previous).expect("order-insensitive replay");

        let message = common::validator_set_transition_message(&transition);
        let mut alternate_certificate = transition.clone();
        alternate_certificate.transition_signatures.clear();
        for index in [1usize, 2, 3] {
            let key = &keys[index];
            alternate_certificate.transition_signatures.push(
                common::ValidatorSetTransitionSignature {
                    validator_id: format!("v{index}"),
                    validator_pubkey: hex::encode(key.verifying_key().as_bytes()),
                    signature: hex::encode(key.sign(message.as_bytes()).to_bytes()),
                },
            );
        }
        alternate_certificate
            .validate_against(&previous)
            .expect("alternate witness has an independent quorum");
        assert_eq!(
            common::validator_snapshot_certificate_hash(&alternate_certificate),
            common::validator_snapshot_certificate_hash(&transition)
        );
        assert_ne!(
            common::validator_snapshot_witness_hash(&alternate_certificate),
            common::validator_snapshot_witness_hash(&transition)
        );
        record_authenticated(dir.path(), alternate_certificate, &previous)
            .expect("alternate valid quorum converges idempotently");
        let retained = authenticated_at(dir.path(), 7).expect("certified snapshot");
        assert_eq!(retained.validator_set_hash, transition.validator_set_hash);
        assert_eq!(
            common::validator_snapshot_witness_hash(&retained),
            common::validator_snapshot_witness_hash(&transition),
            "the first independently validated witness remains the audit record"
        );
        assert!(authenticated_at_from_checkpoint(dir.path(), 7, &transition).is_some());
        let mut conflicting_checkpoint = transition.clone();
        conflicting_checkpoint.previous_block_hash = "b".repeat(64);
        assert!(authenticated_at_from_checkpoint(dir.path(), 7, &conflicting_checkpoint).is_none());

        assert!(validate_transition_at_tip(&transition, 6, &"a".repeat(64)).is_ok());
        assert!(validate_transition_at_tip(&transition, 5, &"a".repeat(64)).is_err());
        assert!(validate_transition_at_tip(&transition, 6, &"b".repeat(64)).is_err());

        let mut duplicate_signer = transition.clone();
        duplicate_signer
            .transition_signatures
            .push(duplicate_signer.transition_signatures[0].clone());
        assert!(duplicate_signer.validate_against(&previous).is_err());
    }

    #[test]
    fn authenticated_writer_classifies_certificate_conflict_and_local_history_failures() {
        let keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let previous = keyed_set(&keys);

        let invalid_dir = tempfile::tempdir().expect("invalid tempdir");
        record(invalid_dir.path(), 0, &previous).expect("bootstrap invalid history");
        let mut invalid = signed_transition(1, &previous, &previous, &keys);
        invalid.transition_signatures.truncate(2);
        assert!(matches!(
            record_authenticated(invalid_dir.path(), invalid, &previous),
            Err(RecordAuthenticatedError::InvalidCertificate(_))
        ));

        let conflict_dir = tempfile::tempdir().expect("conflict tempdir");
        record(conflict_dir.path(), 0, &previous).expect("bootstrap conflict history");
        let accepted = signed_transition(1, &previous, &previous, &keys);
        record_authenticated(conflict_dir.path(), accepted, &previous)
            .expect("initial certificate");
        let alternate_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let alternate = signed_transition(1, &previous, &keyed_set(&alternate_keys), &keys);
        assert!(matches!(
            record_authenticated(conflict_dir.path(), alternate, &previous),
            Err(RecordAuthenticatedError::Conflict(_))
        ));

        let persistence_dir = tempfile::tempdir().expect("persistence tempdir");
        std::fs::write(history_path(persistence_dir.path()), b"{not-json")
            .expect("corrupt local history");
        let valid = signed_transition(1, &previous, &previous, &keys);
        assert!(matches!(
            record_authenticated(persistence_dir.path(), valid, &previous),
            Err(RecordAuthenticatedError::Persistence { .. })
        ));
    }

    #[test]
    fn malformed_history_is_not_confused_with_absent_checkpoint_ancestry() {
        let keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let validators = keyed_set(&keys);
        let checkpoint = signed_transition(1, &validators, &validators, &keys);
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(history_path(dir.path()), b"{malformed-history")
            .expect("write malformed history");

        assert!(load_checked(dir.path()).is_err());
        assert!(authenticated_at_checked(dir.path(), 1, &validators).is_err());
        assert!(
            authenticated_at_from_checkpoint_checked(dir.path(), 1, &checkpoint).is_err(),
            "local corruption is an error, not absent ancestry"
        );
        assert!(
            authenticated_at_from_checkpoint(dir.path(), 1, &checkpoint).is_none(),
            "the explicitly lossy compatibility wrapper remains non-panicking"
        );
    }

    #[test]
    fn truncate_after_reorg_removes_abandoned_certificates_and_keeps_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let validators = keyed_set(&keys);
        record(dir.path(), 0, &validators).expect("bootstrap history");

        let checkpoint = signed_transition(5, &validators, &validators, &keys);
        let retained = signed_transition(6, &validators, &validators, &keys);
        let abandoned = signed_transition(7, &validators, &validators, &keys);
        for transition in [&checkpoint, &retained, &abandoned] {
            record_authenticated(dir.path(), transition.clone(), &validators)
                .expect("certified transition");
        }

        truncate_after_reorg(dir.path(), 5).expect("truncate abandoned branch");

        let heights: Vec<_> = load(dir.path())
            .into_iter()
            .map(|snapshot| snapshot.effective_from_height)
            .collect();
        assert_eq!(heights, vec![0, 5, 6]);
        let authority = authenticated_at_from_checkpoint(dir.path(), 99, &checkpoint)
            .expect("retained prefix remains checkpoint-descended");
        assert_eq!(
            common::validator_snapshot_certificate_hash(&authority),
            common::validator_snapshot_certificate_hash(&retained)
        );
        assert!(!load(dir.path()).iter().any(|snapshot| {
            snapshot
                .certificate
                .as_ref()
                .map(common::validator_snapshot_certificate_hash)
                == Some(common::validator_snapshot_certificate_hash(&abandoned))
        }));
    }

    #[test]
    fn checkpoint_ancestry_rejects_missing_conflicting_and_unsigned_descendants() {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let validators = keyed_set(&keys);
        record(dir.path(), 0, &validators).expect("bootstrap history");

        let checkpoint = signed_transition(5, &validators, &validators, &keys);
        assert!(authenticated_at_from_checkpoint(dir.path(), 5, &checkpoint).is_none());

        record_authenticated(dir.path(), checkpoint.clone(), &validators)
            .expect("checkpoint history");
        let child = signed_transition(6, &validators, &validators, &keys);
        record_authenticated(dir.path(), child.clone(), &validators).expect("valid descendant");
        assert_eq!(
            common::validator_snapshot_certificate_hash(
                &authenticated_at_from_checkpoint(dir.path(), 6, &checkpoint)
                    .expect("valid descendant accepted")
            ),
            common::validator_snapshot_certificate_hash(&child)
        );

        let valid_history = load(dir.path());
        let mut conflicting_history = valid_history.clone();
        let conflicting = conflicting_history
            .iter_mut()
            .find(|entry| entry.effective_from_height == 6)
            .and_then(|entry| entry.certificate.as_mut())
            .expect("child certificate");
        conflicting.previous_validator_set_hash = "f".repeat(64);
        write_history(dir.path(), &conflicting_history);
        assert!(authenticated_at_from_checkpoint(dir.path(), 6, &checkpoint).is_none());
        assert!(
            authenticated_at_from_checkpoint_checked(dir.path(), 5, &checkpoint).is_err(),
            "an invalid later descendant poisons the full retained checkpoint chain"
        );

        let mut unsigned_history = valid_history;
        unsigned_history
            .iter_mut()
            .find(|entry| entry.effective_from_height == 6)
            .and_then(|entry| entry.certificate.as_mut())
            .expect("child certificate")
            .transition_signatures
            .clear();
        write_history(dir.path(), &unsigned_history);
        assert!(authenticated_at_from_checkpoint(dir.path(), 6, &checkpoint).is_none());
        assert!(
            verification_set_at_with_checkpoint(dir.path(), 6, &validators, Some(&checkpoint))
                .is_err()
        );
    }

    #[test]
    fn authenticated_reader_requires_a_complete_chain_from_the_explicit_bootstrap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bootstrap_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let rotated_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let final_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let bootstrap = keyed_set(&bootstrap_keys);
        let rotated = keyed_set(&rotated_keys);
        let final_set = keyed_set(&final_keys);
        record(dir.path(), 0, &bootstrap).expect("trusted bootstrap history");

        let first = signed_transition(1, &bootstrap, &rotated, &bootstrap_keys);
        record_authenticated(dir.path(), first, &bootstrap).expect("first transition");
        let second = signed_transition(2, &rotated, &final_set, &rotated_keys);
        record_authenticated(dir.path(), second.clone(), &rotated).expect("second transition");
        assert_eq!(
            common::validator_snapshot_certificate_hash(
                &authenticated_at_checked(dir.path(), 2, &bootstrap)
                    .expect("strict authenticated read")
                    .expect("latest authenticated certificate")
            ),
            common::validator_snapshot_certificate_hash(&second)
        );

        let valid_history = load_checked(dir.path()).expect("valid history");
        let mut unsigned_same_set_history = valid_history.clone();
        unsigned_same_set_history.push(ValidatorSetSnapshot {
            effective_from_height: 3,
            validators: final_set.clone(),
            certificate: None,
        });
        write_history(dir.path(), &unsigned_same_set_history);
        assert!(
            authenticated_at_checked(dir.path(), 3, &bootstrap).is_err(),
            "no unsigned entry is allowed after authenticated authority starts"
        );

        let attacker_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let attacker_set = keyed_set(&attacker_keys);
        let forged = signed_transition(3, &attacker_set, &attacker_set, &attacker_keys);
        let mut forged_history = valid_history.clone();
        forged_history.push(ValidatorSetSnapshot {
            effective_from_height: forged.effective_from_height,
            validators: forged.validators.clone(),
            certificate: Some(forged.clone()),
        });
        write_history(dir.path(), &forged_history);

        assert_eq!(
            common::validator_snapshot_certificate_hash(
                &authenticated_at(dir.path(), 3)
                    .expect("lossy observation can still report the stored tail")
            ),
            common::validator_snapshot_certificate_hash(&forged)
        );
        assert!(
            authenticated_at_checked(dir.path(), 3, &bootstrap).is_err(),
            "a self-signed alternative tail must not become authority"
        );
        assert!(
            verification_set_at_with_checkpoint(dir.path(), 3, &bootstrap, None).is_err(),
            "historical verification must not trust the structurally valid alternative tail"
        );

        let mut unsigned_history = valid_history;
        unsigned_history.push(ValidatorSetSnapshot {
            effective_from_height: 3,
            validators: attacker_set,
            certificate: None,
        });
        write_history(dir.path(), &unsigned_history);
        assert!(
            authenticated_at_checked(dir.path(), 3, &bootstrap).is_err(),
            "an unsigned compatibility entry cannot change authenticated authority"
        );
    }

    #[test]
    fn checkpoint_install_reconciles_local_forks_and_keeps_only_valid_descendants() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bootstrap_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let checkpoint_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let child_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let grandchild_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let fork_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let bootstrap = keyed_set(&bootstrap_keys);
        let checkpoint_set = keyed_set(&checkpoint_keys);
        let child_set = keyed_set(&child_keys);
        let grandchild_set = keyed_set(&grandchild_keys);
        let fork_set = keyed_set(&fork_keys);
        let checkpoint = signed_transition(5, &bootstrap, &checkpoint_set, &bootstrap_keys);
        let child = signed_transition(6, &checkpoint_set, &child_set, &checkpoint_keys);
        let fork = signed_transition(8, &fork_set, &fork_set, &fork_keys);
        let grandchild = signed_transition(9, &child_set, &grandchild_set, &child_keys);

        write_history(
            dir.path(),
            &[
                ValidatorSetSnapshot {
                    effective_from_height: 0,
                    validators: bootstrap,
                    certificate: None,
                },
                ValidatorSetSnapshot {
                    effective_from_height: 5,
                    validators: fork_set.clone(),
                    certificate: None,
                },
                ValidatorSetSnapshot {
                    effective_from_height: 6,
                    validators: child.validators.clone(),
                    certificate: Some(child.clone()),
                },
                ValidatorSetSnapshot {
                    effective_from_height: 7,
                    validators: fork_set.clone(),
                    certificate: None,
                },
                ValidatorSetSnapshot {
                    effective_from_height: 8,
                    validators: fork.validators.clone(),
                    certificate: Some(fork),
                },
                ValidatorSetSnapshot {
                    effective_from_height: 9,
                    validators: grandchild.validators.clone(),
                    certificate: Some(grandchild.clone()),
                },
            ],
        );

        record_configured_checkpoint_against(dir.path(), &checkpoint, &checkpoint)
            .expect("install and reconcile configured checkpoint");
        let reconciled = load_checked(dir.path()).expect("reconciled history");
        assert_eq!(
            reconciled
                .iter()
                .map(|entry| entry.effective_from_height)
                .collect::<Vec<_>>(),
            vec![0, 5, 6, 9]
        );
        assert_eq!(
            common::validator_snapshot_certificate_hash(
                reconciled[1]
                    .certificate
                    .as_ref()
                    .expect("configured checkpoint retained")
            ),
            common::validator_snapshot_certificate_hash(&checkpoint)
        );
        assert_eq!(
            common::validator_snapshot_certificate_hash(
                &authenticated_at_from_checkpoint_checked(dir.path(), 99, &checkpoint)
                    .expect("validate reconciled checkpoint history")
                    .expect("grandchild authority")
            ),
            common::validator_snapshot_certificate_hash(&grandchild)
        );
    }

    #[test]
    fn checkpoint_install_fails_closed_without_rewriting_an_invalid_descendant() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bootstrap_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let checkpoint_keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let bootstrap = keyed_set(&bootstrap_keys);
        let checkpoint_set = keyed_set(&checkpoint_keys);
        let checkpoint = signed_transition(5, &bootstrap, &checkpoint_set, &bootstrap_keys);
        let mut invalid = signed_transition(6, &checkpoint_set, &checkpoint_set, &checkpoint_keys);
        invalid.transition_signatures.truncate(1);
        write_history(
            dir.path(),
            &[
                ValidatorSetSnapshot {
                    effective_from_height: 5,
                    validators: checkpoint.validators.clone(),
                    certificate: Some(checkpoint.clone()),
                },
                ValidatorSetSnapshot {
                    effective_from_height: 6,
                    validators: invalid.validators.clone(),
                    certificate: Some(invalid),
                },
            ],
        );
        let before = std::fs::read(history_path(dir.path())).expect("read original history");

        assert!(matches!(
            record_configured_checkpoint_against(dir.path(), &checkpoint, &checkpoint),
            Err(RecordAuthenticatedError::Persistence { .. })
        ));
        assert_eq!(
            std::fs::read(history_path(dir.path())).expect("read retained history"),
            before,
            "invalid descendants must abort before any durable replacement"
        );
    }

    #[test]
    fn retention_preserves_a_configured_checkpoint_chain_beyond_the_cap() {
        let keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let validators = keyed_set(&keys);
        let snapshots: Vec<_> = (1..=(MAX_SNAPSHOTS as u64 + 1))
            .map(|height| {
                let certificate = signed_transition(height, &validators, &validators, &keys);
                ValidatorSetSnapshot {
                    effective_from_height: height,
                    validators: validators.clone(),
                    certificate: Some(certificate),
                }
            })
            .collect();
        let checkpoint_hash = common::validator_snapshot_certificate_hash(
            snapshots[0]
                .certificate
                .as_ref()
                .expect("checkpoint certificate"),
        );

        let mut checkpoint_history = snapshots.clone();
        enforce_retention_for_checkpoint_hash(&mut checkpoint_history, Some(&checkpoint_hash));
        assert_eq!(checkpoint_history.len(), MAX_SNAPSHOTS + 1);
        assert_eq!(checkpoint_history[0].effective_from_height, 1);

        let mut compatibility_history = snapshots;
        enforce_retention_for_checkpoint_hash(&mut compatibility_history, None);
        assert_eq!(compatibility_history.len(), MAX_SNAPSHOTS);
        assert_eq!(compatibility_history[0].effective_from_height, 2);
    }
}
