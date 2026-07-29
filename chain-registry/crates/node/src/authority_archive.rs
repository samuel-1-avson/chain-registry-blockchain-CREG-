//! Bounded, content-addressed storage for accepted namespace-authority inputs.
//!
//! These bytes are validator-local audit material, not consensus state.  The
//! archive is therefore protected by an explicit byte/entry quota and a
//! minimum-retention window.  Garbage collection runs only under quota
//! pressure and never removes either input of the observation currently being
//! archived.

use std::{
    collections::HashSet,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, SystemTime},
};

use tokio::{io::AsyncWriteExt, sync::Mutex};

use crate::authority_adapter::ArtifactDigestAlgorithm;

pub const DEFAULT_ARCHIVE_MAX_BYTES: u64 = 10 * 1024 * 1024 * 1024;
pub const HARD_MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
pub const DEFAULT_ARCHIVE_MAX_ENTRIES: usize = 100_000;
pub const HARD_MAX_ARCHIVE_ENTRIES: usize = 1_000_000;
pub const DEFAULT_ARCHIVE_MIN_RETENTION_SECS: u64 = 365 * 24 * 60 * 60;
pub const HARD_MAX_ARCHIVE_MIN_RETENTION_SECS: u64 = 10 * 365 * 24 * 60 * 60;
const STALE_TEMP_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

static ARCHIVE_MUTATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
pub struct AuthorityArchivePolicy {
    pub max_bytes: u64,
    pub max_entries: usize,
    pub minimum_retention: Duration,
}

impl AuthorityArchivePolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_bytes == 0 || self.max_bytes > HARD_MAX_ARCHIVE_BYTES {
            return Err(format!(
                "authority archive maximum bytes must be between 1 and {HARD_MAX_ARCHIVE_BYTES}"
            ));
        }
        if self.max_entries == 0 || self.max_entries > HARD_MAX_ARCHIVE_ENTRIES {
            return Err(format!(
                "authority archive maximum entries must be between 1 and {HARD_MAX_ARCHIVE_ENTRIES}"
            ));
        }
        if self.minimum_retention.is_zero()
            || self.minimum_retention > Duration::from_secs(HARD_MAX_ARCHIVE_MIN_RETENTION_SECS)
        {
            return Err(format!(
                "authority archive minimum retention must be between 1 and \
                 {HARD_MAX_ARCHIVE_MIN_RETENTION_SECS} seconds"
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AuthorityArchiveOutcome {
    pub created_entries: usize,
    pub reclaimed_entries: usize,
    pub reclaimed_bytes: u64,
}

#[derive(Clone, Copy)]
struct ArchiveBlob<'a> {
    collection: &'static str,
    algorithm: ArtifactDigestAlgorithm,
    digest: &'a str,
    bytes: &'a [u8],
}

impl ArchiveBlob<'_> {
    fn validate(&self) -> Result<(), String> {
        let expected_len = match self.algorithm {
            ArtifactDigestAlgorithm::Sha256 => 64,
            ArtifactDigestAlgorithm::Sha512 => 128,
        };
        if self.digest.len() != expected_len
            || self.digest != self.digest.to_ascii_lowercase()
            || !self.digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "authority {} archive digest is invalid",
                self.collection
            ));
        }
        if self.algorithm.digest(self.bytes) != self.digest {
            return Err(format!(
                "authority {} archive digest mismatch",
                self.collection
            ));
        }
        Ok(())
    }

    fn destination(&self, data_dir: &Path) -> PathBuf {
        data_dir
            .join(self.collection)
            .join(self.algorithm.name())
            .join(&self.digest[..2])
            .join(format!("{}.bin", self.digest))
    }
}

#[derive(Debug)]
struct ArchiveEntry {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
    temporary: bool,
}

/// Archive the evidence and artifact as one quota decision.  Both digests are
/// validated before any filesystem mutation.  A normal write failure rolls
/// back every entry newly created by this call.
pub async fn archive_authority_observation(
    data_dir: &Path,
    policy: AuthorityArchivePolicy,
    evidence_digest: &str,
    evidence: &[u8],
    artifact_algorithm: ArtifactDigestAlgorithm,
    artifact_digest: &str,
    artifact: &[u8],
) -> Result<AuthorityArchiveOutcome, String> {
    archive_authority_observation_at(
        data_dir,
        policy,
        evidence_digest,
        evidence,
        artifact_algorithm,
        artifact_digest,
        artifact,
        SystemTime::now(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn archive_authority_observation_at(
    data_dir: &Path,
    policy: AuthorityArchivePolicy,
    evidence_digest: &str,
    evidence: &[u8],
    artifact_algorithm: ArtifactDigestAlgorithm,
    artifact_digest: &str,
    artifact: &[u8],
    now: SystemTime,
) -> Result<AuthorityArchiveOutcome, String> {
    policy.validate()?;
    let blobs = [
        ArchiveBlob {
            collection: "authority-evidence",
            algorithm: ArtifactDigestAlgorithm::Sha256,
            digest: evidence_digest,
            bytes: evidence,
        },
        ArchiveBlob {
            collection: "authority-artifacts",
            algorithm: artifact_algorithm,
            digest: artifact_digest,
            bytes: artifact,
        },
    ];
    for blob in &blobs {
        blob.validate()?;
    }

    let lock = ARCHIVE_MUTATION_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().await;

    let destinations = blobs
        .iter()
        .map(|blob| blob.destination(data_dir))
        .collect::<Vec<_>>();
    let protected = destinations.iter().cloned().collect::<HashSet<_>>();
    let mut missing = Vec::new();
    for (index, blob) in blobs.iter().enumerate() {
        if !existing_blob_is_valid(&destinations[index], blob).await? {
            missing.push(index);
        }
    }

    let required_bytes = missing.iter().try_fold(0u64, |total, index| {
        total
            .checked_add(blobs[*index].bytes.len() as u64)
            .ok_or_else(|| "authority archive request size overflowed".to_owned())
    })?;
    let required_entries = missing.len();
    if required_bytes > policy.max_bytes || required_entries > policy.max_entries {
        return Err(
            "authority archive observation exceeds the configured archive quota".to_owned(),
        );
    }

    // Scan to the protocol hard ceiling so lowering an operator quota can
    // reclaim an oversized pre-existing archive instead of making recovery
    // impossible.  The hard ceiling still bounds memory and traversal work.
    let mut entries = scan_archive(data_dir, HARD_MAX_ARCHIVE_ENTRIES).await?;
    let mut total_bytes = entries.iter().try_fold(0u64, |total, entry| {
        total
            .checked_add(entry.bytes)
            .ok_or_else(|| "authority archive size overflowed".to_owned())
    })?;
    let mut total_entries = entries.len();
    let mut outcome = AuthorityArchiveOutcome::default();

    // Crash-left temporary files are never valid evidence.  Remove them after
    // a fixed grace period before considering retained content for eviction.
    entries.sort_by(|left, right| {
        left.modified
            .cmp(&right.modified)
            .then_with(|| left.path.cmp(&right.path))
    });
    for entry in entries.iter().filter(|entry| {
        entry.temporary && elapsed_at_least(now, entry.modified, STALE_TEMP_RETENTION)
    }) {
        remove_archive_entry(entry).await?;
        total_bytes = total_bytes.saturating_sub(entry.bytes);
        total_entries = total_entries.saturating_sub(1);
        outcome.reclaimed_bytes = outcome.reclaimed_bytes.saturating_add(entry.bytes);
        outcome.reclaimed_entries = outcome.reclaimed_entries.saturating_add(1);
    }

    let quota_satisfied = |bytes: u64, count: usize| {
        bytes
            .checked_add(required_bytes)
            .is_some_and(|projected| projected <= policy.max_bytes)
            && count
                .checked_add(required_entries)
                .is_some_and(|projected| projected <= policy.max_entries)
    };
    if !quota_satisfied(total_bytes, total_entries) {
        for entry in entries.iter().filter(|entry| {
            !entry.temporary
                && !protected.contains(&entry.path)
                && elapsed_at_least(now, entry.modified, policy.minimum_retention)
        }) {
            remove_archive_entry(entry).await?;
            total_bytes = total_bytes.saturating_sub(entry.bytes);
            total_entries = total_entries.saturating_sub(1);
            outcome.reclaimed_bytes = outcome.reclaimed_bytes.saturating_add(entry.bytes);
            outcome.reclaimed_entries = outcome.reclaimed_entries.saturating_add(1);
            if quota_satisfied(total_bytes, total_entries) {
                break;
            }
        }
    }
    if !quota_satisfied(total_bytes, total_entries) {
        return Err(
            "authority archive quota is exhausted and no retention-eligible entries can be reclaimed"
                .to_owned(),
        );
    }

    let mut created = Vec::new();
    for index in missing {
        match write_blob_atomically(data_dir, &blobs[index]).await {
            Ok(true) => {
                created.push(destinations[index].clone());
                outcome.created_entries += 1;
            }
            Ok(false) => {}
            Err(error) => {
                for path in created {
                    if tokio::fs::remove_file(&path).await.is_ok() {
                        let _ = sync_parent_directory(&path).await;
                    }
                }
                return Err(error);
            }
        }
    }
    Ok(outcome)
}

fn elapsed_at_least(now: SystemTime, modified: SystemTime, minimum: Duration) -> bool {
    now.duration_since(modified)
        .is_ok_and(|elapsed| elapsed >= minimum)
}

async fn existing_blob_is_valid(path: &Path, blob: &ArchiveBlob<'_>) -> Result<bool, String> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "cannot inspect authority {} archive entry: {error}",
                blob.collection
            ))
        }
    };
    if !metadata.file_type().is_file() {
        return Err(format!(
            "authority {} archive entry is not a regular file",
            blob.collection
        ));
    }
    if metadata.len() != blob.bytes.len() as u64 {
        return Err(format!(
            "existing authority {} archive entry is corrupted",
            blob.collection
        ));
    }
    let existing = tokio::fs::read(path).await.map_err(|error| {
        format!(
            "cannot read authority {} archive entry: {error}",
            blob.collection
        )
    })?;
    if blob.algorithm.digest(&existing) != blob.digest {
        return Err(format!(
            "existing authority {} archive entry is corrupted",
            blob.collection
        ));
    }
    Ok(true)
}

async fn scan_archive(
    data_dir: &Path,
    maximum_entries: usize,
) -> Result<Vec<ArchiveEntry>, String> {
    let mut stack = vec![
        data_dir.join("authority-evidence"),
        data_dir.join("authority-artifacts"),
    ];
    let mut entries = Vec::new();
    while let Some(directory) = stack.pop() {
        let mut reader = match tokio::fs::read_dir(&directory).await {
            Ok(reader) => reader,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "cannot scan authority archive directory {}: {error}",
                    directory.display()
                ))
            }
        };
        while let Some(entry) = reader
            .next_entry()
            .await
            .map_err(|error| format!("cannot scan authority archive: {error}"))?
        {
            let file_type = entry
                .file_type()
                .await
                .map_err(|error| format!("cannot inspect authority archive entry: {error}"))?;
            if file_type.is_symlink() {
                return Err(format!(
                    "authority archive contains a symbolic link: {}",
                    entry.path().display()
                ));
            }
            if file_type.is_dir() {
                stack.push(entry.path());
                continue;
            }
            if !file_type.is_file() {
                return Err(format!(
                    "authority archive contains a non-regular entry: {}",
                    entry.path().display()
                ));
            }
            if entries.len() >= maximum_entries {
                return Err(
                    "authority archive entry scan exceeds the configured entry limit".to_owned(),
                );
            }
            let filename = entry.file_name();
            let filename = filename.to_string_lossy();
            let temporary = filename.ends_with(".tmp");
            if !temporary && !filename.ends_with(".bin") {
                return Err(format!(
                    "authority archive contains an unexpected file: {}",
                    entry.path().display()
                ));
            }
            let metadata = entry
                .metadata()
                .await
                .map_err(|error| format!("cannot inspect authority archive entry: {error}"))?;
            entries.push(ArchiveEntry {
                path: entry.path(),
                bytes: metadata.len(),
                modified: metadata.modified().map_err(|error| {
                    format!("authority archive entry has no modification time: {error}")
                })?,
                temporary,
            });
        }
    }
    Ok(entries)
}

async fn remove_archive_entry(entry: &ArchiveEntry) -> Result<(), String> {
    match tokio::fs::remove_file(&entry.path).await {
        Ok(()) => sync_parent_directory(&entry.path).await,
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "cannot garbage-collect authority archive entry {}: {error}",
            entry.path.display()
        )),
    }
}

async fn write_blob_atomically(data_dir: &Path, blob: &ArchiveBlob<'_>) -> Result<bool, String> {
    let destination = blob.destination(data_dir);
    let directory = destination
        .parent()
        .ok_or_else(|| "authority archive destination has no parent".to_owned())?;
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|error| {
            format!(
                "cannot create authority {} archive: {error}",
                blob.collection
            )
        })?;
    if existing_blob_is_valid(&destination, blob).await? {
        return Ok(false);
    }

    let temporary = directory.join(format!("{}.{}.tmp", blob.digest, uuid::Uuid::new_v4()));
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .await
        .map_err(|error| {
            format!(
                "cannot create authority {} archive entry: {error}",
                blob.collection
            )
        })?;
    if let Err(error) = file.write_all(blob.bytes).await {
        drop(file);
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(format!(
            "cannot write authority {} archive entry: {error}",
            blob.collection
        ));
    }
    if let Err(error) = file.sync_all().await {
        drop(file);
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(format!(
            "cannot sync authority {} archive entry: {error}",
            blob.collection
        ));
    }
    drop(file);
    if let Err(error) = tokio::fs::rename(&temporary, &destination).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        if existing_blob_is_valid(&destination, blob).await? {
            return Ok(false);
        }
        return Err(format!(
            "cannot commit authority {} archive entry: {error}",
            blob.collection
        ));
    }
    if let Err(error) = sync_parent_directory(&destination).await {
        let _ = tokio::fs::remove_file(&destination).await;
        let _ = sync_parent_directory(&destination).await;
        return Err(format!(
            "cannot durably commit authority {} archive entry: {error}",
            blob.collection
        ));
    }
    Ok(true)
}

#[cfg(unix)]
async fn sync_parent_directory(path: &Path) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or_else(|| "authority archive entry has no parent directory".to_owned())?;
    let directory_handle = tokio::fs::File::open(directory).await.map_err(|error| {
        format!(
            "cannot open authority archive directory {}: {error}",
            directory.display()
        )
    })?;
    directory_handle.sync_all().await.map_err(|error| {
        format!(
            "cannot sync authority archive directory {}: {error}",
            directory.display()
        )
    })
}

#[cfg(not(unix))]
async fn sync_parent_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(max_bytes: u64, max_entries: usize) -> AuthorityArchivePolicy {
        AuthorityArchivePolicy {
            max_bytes,
            max_entries,
            minimum_retention: Duration::from_secs(60),
        }
    }

    fn digest(algorithm: ArtifactDigestAlgorithm, bytes: &[u8]) -> String {
        algorithm.digest(bytes)
    }

    #[tokio::test]
    async fn archives_pair_and_exact_replay_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let evidence = b"evidence";
        let artifact = b"artifact";
        let evidence_digest = digest(ArtifactDigestAlgorithm::Sha256, evidence);
        let artifact_digest = digest(ArtifactDigestAlgorithm::Sha512, artifact);

        let first = archive_authority_observation(
            root.path(),
            policy(64, 2),
            &evidence_digest,
            evidence,
            ArtifactDigestAlgorithm::Sha512,
            &artifact_digest,
            artifact,
        )
        .await
        .unwrap();
        assert_eq!(first.created_entries, 2);

        let replay = archive_authority_observation(
            root.path(),
            policy(64, 2),
            &evidence_digest,
            evidence,
            ArtifactDigestAlgorithm::Sha512,
            &artifact_digest,
            artifact,
        )
        .await
        .unwrap();
        assert_eq!(replay.created_entries, 0);
        assert_eq!(scan_archive(root.path(), 2).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn quota_rejects_when_retained_entries_are_not_eligible_for_gc() {
        let root = tempfile::tempdir().unwrap();
        let first_evidence = b"1111";
        let first_artifact = b"2222";
        archive_authority_observation(
            root.path(),
            policy(8, 2),
            &digest(ArtifactDigestAlgorithm::Sha256, first_evidence),
            first_evidence,
            ArtifactDigestAlgorithm::Sha256,
            &digest(ArtifactDigestAlgorithm::Sha256, first_artifact),
            first_artifact,
        )
        .await
        .unwrap();

        let second_evidence = b"3333";
        let second_artifact = b"4444";
        let error = archive_authority_observation(
            root.path(),
            policy(8, 2),
            &digest(ArtifactDigestAlgorithm::Sha256, second_evidence),
            second_evidence,
            ArtifactDigestAlgorithm::Sha256,
            &digest(ArtifactDigestAlgorithm::Sha256, second_artifact),
            second_artifact,
        )
        .await
        .unwrap_err();
        assert!(error.contains("quota is exhausted"));
        assert_eq!(scan_archive(root.path(), 2).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn entry_count_quota_is_enforced_independently_of_byte_capacity() {
        let root = tempfile::tempdir().unwrap();
        let first_evidence = b"first evidence";
        let first_artifact = b"first artifact";
        archive_authority_observation(
            root.path(),
            policy(1024, 2),
            &digest(ArtifactDigestAlgorithm::Sha256, first_evidence),
            first_evidence,
            ArtifactDigestAlgorithm::Sha256,
            &digest(ArtifactDigestAlgorithm::Sha256, first_artifact),
            first_artifact,
        )
        .await
        .unwrap();

        let second_evidence = b"e";
        let second_artifact = b"a";
        let error = archive_authority_observation(
            root.path(),
            policy(1024, 2),
            &digest(ArtifactDigestAlgorithm::Sha256, second_evidence),
            second_evidence,
            ArtifactDigestAlgorithm::Sha256,
            &digest(ArtifactDigestAlgorithm::Sha256, second_artifact),
            second_artifact,
        )
        .await
        .unwrap_err();
        assert!(error.contains("quota is exhausted"));
    }

    #[tokio::test]
    async fn concurrent_distinct_pairs_cannot_race_past_the_quota() {
        let root = tempfile::tempdir().unwrap();
        let first_evidence = b"1111";
        let first_artifact = b"2222";
        let second_evidence = b"3333";
        let second_artifact = b"4444";
        let first_evidence_digest = digest(ArtifactDigestAlgorithm::Sha256, first_evidence);
        let first_artifact_digest = digest(ArtifactDigestAlgorithm::Sha256, first_artifact);
        let second_evidence_digest = digest(ArtifactDigestAlgorithm::Sha256, second_evidence);
        let second_artifact_digest = digest(ArtifactDigestAlgorithm::Sha256, second_artifact);

        let first = archive_authority_observation(
            root.path(),
            policy(8, 2),
            &first_evidence_digest,
            first_evidence,
            ArtifactDigestAlgorithm::Sha256,
            &first_artifact_digest,
            first_artifact,
        );
        let second = archive_authority_observation(
            root.path(),
            policy(8, 2),
            &second_evidence_digest,
            second_evidence,
            ArtifactDigestAlgorithm::Sha256,
            &second_artifact_digest,
            second_artifact,
        );
        let (first, second) = tokio::join!(first, second);

        assert_ne!(first.is_ok(), second.is_ok());
        assert_eq!(scan_archive(root.path(), 2).await.unwrap().len(), 2);
        assert_eq!(
            scan_archive(root.path(), 2)
                .await
                .unwrap()
                .iter()
                .map(|entry| entry.bytes)
                .sum::<u64>(),
            8
        );
    }

    #[tokio::test]
    async fn quota_pressure_collects_only_retention_eligible_entries() {
        let root = tempfile::tempdir().unwrap();
        let first_evidence = b"1111";
        let first_artifact = b"2222";
        let initial_now = SystemTime::now();
        archive_authority_observation_at(
            root.path(),
            policy(8, 2),
            &digest(ArtifactDigestAlgorithm::Sha256, first_evidence),
            first_evidence,
            ArtifactDigestAlgorithm::Sha256,
            &digest(ArtifactDigestAlgorithm::Sha256, first_artifact),
            first_artifact,
            initial_now,
        )
        .await
        .unwrap();

        let second_evidence = b"3333";
        let second_artifact = b"4444";
        let outcome = archive_authority_observation_at(
            root.path(),
            policy(8, 2),
            &digest(ArtifactDigestAlgorithm::Sha256, second_evidence),
            second_evidence,
            ArtifactDigestAlgorithm::Sha256,
            &digest(ArtifactDigestAlgorithm::Sha256, second_artifact),
            second_artifact,
            // Base the retention advance on a clock sample taken after the
            // first write. On a slow filesystem, its actual mtime can be more
            // than one second newer than `initial_now`.
            SystemTime::now() + Duration::from_secs(61),
        )
        .await
        .unwrap();
        assert_eq!(outcome.reclaimed_entries, 2);
        assert_eq!(outcome.reclaimed_bytes, 8);
        assert_eq!(outcome.created_entries, 2);
    }

    #[tokio::test]
    async fn corrupted_existing_entry_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let evidence = b"evidence";
        let artifact = b"artifact";
        let evidence_digest = digest(ArtifactDigestAlgorithm::Sha256, evidence);
        let artifact_digest = digest(ArtifactDigestAlgorithm::Sha256, artifact);
        let evidence_blob = ArchiveBlob {
            collection: "authority-evidence",
            algorithm: ArtifactDigestAlgorithm::Sha256,
            digest: &evidence_digest,
            bytes: evidence,
        };
        let destination = evidence_blob.destination(root.path());
        tokio::fs::create_dir_all(destination.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&destination, b"corrupt").await.unwrap();

        let error = archive_authority_observation(
            root.path(),
            policy(64, 4),
            &evidence_digest,
            evidence,
            ArtifactDigestAlgorithm::Sha256,
            &artifact_digest,
            artifact,
        )
        .await
        .unwrap_err();
        assert!(error.contains("corrupted"));
        assert!(!ArchiveBlob {
            collection: "authority-artifacts",
            algorithm: ArtifactDigestAlgorithm::Sha256,
            digest: &artifact_digest,
            bytes: artifact,
        }
        .destination(root.path())
        .exists());
    }

    #[tokio::test]
    async fn validates_both_inputs_before_creating_either_entry() {
        let root = tempfile::tempdir().unwrap();
        let evidence = b"evidence";
        let artifact = b"artifact";
        let error = archive_authority_observation(
            root.path(),
            policy(64, 4),
            &digest(ArtifactDigestAlgorithm::Sha256, evidence),
            evidence,
            ArtifactDigestAlgorithm::Sha256,
            &"00".repeat(32),
            artifact,
        )
        .await
        .unwrap_err();
        assert!(error.contains("digest mismatch"));
        assert!(scan_archive(root.path(), 4).await.unwrap().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symbolic_link_archive_entry_is_rejected_without_following_it() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let evidence = b"evidence";
        let artifact = b"artifact";
        let evidence_digest = digest(ArtifactDigestAlgorithm::Sha256, evidence);
        let malicious = root
            .path()
            .join("authority-evidence")
            .join("sha256")
            .join("00")
            .join(format!("{}.bin", "00".repeat(32)));
        std::fs::create_dir_all(malicious.parent().unwrap()).unwrap();
        symlink(outside.path(), &malicious).unwrap();

        let error = archive_authority_observation(
            root.path(),
            policy(64, 4),
            &evidence_digest,
            evidence,
            ArtifactDigestAlgorithm::Sha256,
            &digest(ArtifactDigestAlgorithm::Sha256, artifact),
            artifact,
        )
        .await
        .unwrap_err();
        assert!(error.contains("symbolic link"));
        assert_eq!(std::fs::read(outside.path()).unwrap(), Vec::<u8>::new());
    }
}
