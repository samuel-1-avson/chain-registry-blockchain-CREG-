// crates/node/src/validator_set_history.rs
//
// Height-indexed history of the active validator set (ISSUE-050).
//
// Block signatures must be verified against the validator set that was active
// AT THE BLOCK'S HEIGHT, not the current set. Otherwise, after a membership
// rotation, a node re-verifying or reorg-recovering older blocks counts only
// the signers still in today's set and can wrongly reject a block that had a
// valid quorum when it was produced.
//
// This module records a snapshot whenever the set changes (effective from the
// next block height) and lets the sync/verify path look up the set active at
// any height. It is persisted to disk so the history survives restarts.
//
// SAFETY / SCOPE: lookups are ADDITIVE — callers fall back to the current set
// when no snapshot covers a height, so behaviour is never stricter than before.
// A node syncing historical heights it never witnessed (e.g. from genesis
// across a past rotation) has no snapshot for them and falls back to current,
// exactly as before. Full reconstruction of pre-join history from L1 staking
// events is a larger follow-up.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use common::{AuthenticatedValidatorSetSnapshot, ValidatorSet};
use serde::{Deserialize, Serialize};

const HISTORY_FILE: &str = "validator-set-history.json";

/// Load a deployment-provisioned weak-subjectivity checkpoint. It is never
/// accepted from the network; operators must provision the same certificate
/// out of band for a new/rebuilding node.
pub fn checkpoint_from_env() -> Result<Option<AuthenticatedValidatorSetSnapshot>, String> {
    let Ok(raw) = std::env::var("CREG_WEAK_SUBJECTIVITY_CHECKPOINT") else {
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
    if checkpoint.schema_version != common::VALIDATOR_SNAPSHOT_SCHEMA_VERSION
        || checkpoint.effective_from_height == 0
        || checkpoint.validators.validators.is_empty()
        || checkpoint.validator_set_hash
            != common::canonical_validator_set_hash(&checkpoint.validators)
        || checkpoint.previous_block_hash.len() != 64
        || checkpoint.previous_validator_set_hash.len() != 64
        || !checkpoint
            .previous_block_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || !checkpoint
            .previous_validator_set_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || checkpoint.transition_signatures.is_empty()
    {
        return Err(
            "invalid CREG_WEAK_SUBJECTIVITY_CHECKPOINT: inconsistent certificate structure"
                .to_string(),
        );
    }
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

fn history_path(data_dir: &Path) -> PathBuf {
    data_dir.join(HISTORY_FILE)
}

fn validate_history_bytes(bytes: &[u8]) -> anyhow::Result<()> {
    serde_json::from_slice::<Vec<ValidatorSetSnapshot>>(bytes)
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("invalid validator-set history snapshot: {error}"))
}

/// Persist a complete history replacement durably before reporting success.
/// Callers hold `history_write_guard`, so the fixed temporary path cannot race
/// another local history writer.
fn persist_history_bytes(data_dir: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    validate_history_bytes(bytes)?;
    std::fs::create_dir_all(data_dir)?;
    let path = history_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, &path)?;
    // Best-effort directory sync strengthens rename durability on platforms
    // that support opening directories as files.
    if let Ok(directory) = std::fs::File::open(data_dir) {
        let _ = directory.sync_all();
    }
    Ok(())
}

/// Run `persist` while the exact pre-reorg history snapshot is protected from
/// concurrent local writers. The closure durably writes the RocksDB journal,
/// making that journal the transaction boundary for subsequent history edits.
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
        None => match std::fs::remove_file(history_path(data_dir)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

/// Load all snapshots (ascending `effective_from_height`). Empty if missing.
pub fn load(data_dir: &Path) -> Vec<ValidatorSetSnapshot> {
    let mut snaps: Vec<ValidatorSetSnapshot> = match std::fs::read(history_path(data_dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    snaps.sort_by_key(|s| s.effective_from_height);
    snaps
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

/// Return the validator set active at `height` from the on-disk history, or
/// `None` to signal the caller should use its current set (no regression).
pub fn set_at(data_dir: &Path, height: u64) -> Option<ValidatorSet> {
    let snapshots = load(data_dir);
    let snap = select_at(&snapshots, height)?;
    if snap.validators.validators.is_empty() {
        None
    } else {
        Some(snap.validators.clone())
    }
}

/// Return only a quorum-certified snapshot suitable for a new security
/// decision such as committee assignment. Legacy local history intentionally
/// returns `None` here rather than being upgraded by assertion.
pub fn authenticated_at(data_dir: &Path, height: u64) -> Option<AuthenticatedValidatorSetSnapshot> {
    let snapshots = load(data_dir);
    select_at(&snapshots, height)?.certificate.clone()
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
    let snapshots = load(data_dir);
    let checkpoint_hash = common::validator_snapshot_certificate_hash(checkpoint);
    let mut current_hash = checkpoint.validator_set_hash.clone();
    let mut current_set = checkpoint.validators.clone();
    let mut found = false;
    for entry in snapshots
        .iter()
        .filter(|entry| entry.effective_from_height <= height)
    {
        if !found {
            // Compatibility/bootstrap entries may predate the configured
            // checkpoint and therefore legitimately have no certificate.
            // They are not authority anchors; skip them while searching for
            // the exact out-of-band checkpoint.
            if let Some(certificate) = entry.certificate.as_ref() {
                if common::validator_snapshot_certificate_hash(certificate) == checkpoint_hash
                    && entry.effective_from_height == certificate.effective_from_height
                    && common::canonical_validator_set_hash(&entry.validators)
                        == certificate.validator_set_hash
                {
                    found = true;
                    current_hash = certificate.validator_set_hash.clone();
                    current_set = certificate.validators.clone();
                }
            }
            continue;
        }
        // Once anchored, every subsequent history entry must be certified.
        let certificate = entry.certificate.as_ref()?;
        if entry.effective_from_height != certificate.effective_from_height
            || common::canonical_validator_set_hash(&entry.validators)
                != certificate.validator_set_hash
            || certificate.previous_validator_set_hash != current_hash
            || certificate.validate_against(&current_set).is_err()
        {
            return None;
        }
        current_hash = certificate.validator_set_hash.clone();
        current_set = certificate.validators.clone();
    }
    if !found {
        return None;
    }
    select_at(&snapshots, height)?.certificate.clone()
}

/// Resolve committee authority under the configured weak-subjectivity policy.
/// Malformed checkpoint configuration is an error, never a legacy fallback.
pub fn committee_authority_at(
    data_dir: &Path,
    height: u64,
) -> Result<Option<AuthenticatedValidatorSetSnapshot>, String> {
    match checkpoint_from_env()? {
        Some(checkpoint) => Ok(authenticated_at_from_checkpoint(
            data_dir,
            height,
            &checkpoint,
        )),
        None => Ok(authenticated_at(data_dir, height)),
    }
}

/// Resolve the validator set used for historical block verification. At and
/// after a configured checkpoint this is fail-closed; prior bootstrap heights
/// retain compatibility lookup semantics.
pub fn verification_set_at(
    data_dir: &Path,
    height: u64,
    current: &ValidatorSet,
) -> Result<ValidatorSet, String> {
    let checkpoint = checkpoint_from_env()?;
    verification_set_at_with_checkpoint(data_dir, height, current, checkpoint.as_ref())
}

fn verification_set_at_with_checkpoint(
    data_dir: &Path,
    height: u64,
    current: &ValidatorSet,
    checkpoint: Option<&AuthenticatedValidatorSetSnapshot>,
) -> Result<ValidatorSet, String> {
    match checkpoint {
        Some(checkpoint) if height >= checkpoint.effective_from_height => {
            authenticated_at_from_checkpoint(data_dir, height, checkpoint)
                .map(|snapshot| snapshot.validators)
                .ok_or_else(|| {
                    format!("no checkpoint-descended validator authority at height {height}")
                })
        }
        _ => Ok(set_at(data_dir, height).unwrap_or_else(|| current.clone())),
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
    let mut snapshots = load(data_dir);

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
    let mut snapshots = load(data_dir);
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
) -> anyhow::Result<()> {
    let _write_guard = history_write_guard()?;
    snapshot
        .validate_against(previous)
        .map_err(|error| anyhow::anyhow!("invalid validator-set transition: {error}"))?;
    let mut snapshots = load(data_dir);
    if let Some(active) = select_at(&snapshots, snapshot.effective_from_height.saturating_sub(1)) {
        if common::canonical_validator_set_hash(&active.validators)
            != common::canonical_validator_set_hash(previous)
        {
            anyhow::bail!("validator-set transition does not continue the persisted active set");
        }
    }
    if let Some(existing) = snapshots
        .iter()
        .find(|existing| existing.effective_from_height == snapshot.effective_from_height)
    {
        if common::canonical_validator_set_hash(&existing.validators)
            != common::canonical_validator_set_hash(&snapshot.validators)
        {
            anyhow::bail!("conflicting validator-set transition at the same effective height");
        }
        if let Some(existing_certificate) = &existing.certificate {
            if common::validator_snapshot_certificate_hash(existing_certificate)
                == common::validator_snapshot_certificate_hash(&snapshot)
            {
                return Ok(()); // exact idempotent replay
            }
            anyhow::bail!("conflicting validator-set certificate at the same effective height");
        }
    }
    snapshots.retain(|existing| existing.effective_from_height != snapshot.effective_from_height);
    snapshots.push(ValidatorSetSnapshot {
        effective_from_height: snapshot.effective_from_height,
        validators: snapshot.validators.clone(),
        certificate: Some(snapshot),
    });
    snapshots.sort_by_key(|snapshot| snapshot.effective_from_height);
    enforce_retention(&mut snapshots)?;
    let bytes = serde_json::to_vec_pretty(&snapshots)?;
    persist_history_bytes(data_dir, &bytes)
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
    fn set_at_returns_none_when_empty_and_some_otherwise() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(set_at(dir.path(), 1).is_none());
        record(dir.path(), 0, &set(&["aa", "bb"])).expect("record");
        let got = set_at(dir.path(), 50).expect("set present");
        assert_eq!(got.validators.len(), 2);
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
        assert!(record_authenticated(dir.path(), alternate_certificate, &previous).is_err());
        assert_eq!(
            authenticated_at(dir.path(), 7)
                .expect("certified snapshot")
                .validator_set_hash,
            transition.validator_set_hash
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
