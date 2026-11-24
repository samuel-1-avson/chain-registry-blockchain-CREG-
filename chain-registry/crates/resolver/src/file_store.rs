// crates/resolver/src/file_store.rs
// Shared advisory file lock, crash-safe atomic write, and corruption quarantine
// for resolver local state (ECA-15, ADR-019 Increment 3).

use anyhow::{Context, Result};
use common::DisplayText;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Cross-process exclusive lock backed by `std::fs::File::lock()` (`flock` on
/// Unix, `LockFileEx` on Windows). Callers in the same process must also hold
/// an in-process `Mutex` because Unix `flock` locks per open file description
/// and Windows `LockFileEx` locks per handle.
pub(crate) struct FileLockGuard {
    file: File,
}

impl FileLockGuard {
    pub(crate) fn acquire(target_path: &Path) -> Result<Self> {
        let lock_path = lock_path_for(target_path);
        let parent = parent_dir(&lock_path);
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create lock directory {}", parent.display()))?;
        // Read+write mode is required for `LockFileEx` on Windows. The lock
        // file is never truncated or unlinked so waiting processes always lock
        // the same inode/path.
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("open lock file {}", lock_path.display()))?;
        file.lock()
            .with_context(|| format!("acquire lock on {}", lock_path.display()))?;
        Ok(Self { file })
    }
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(crate) fn lock_path_for(target_path: &Path) -> PathBuf {
    let mut os = target_path.as_os_str().to_os_string();
    os.push(".lock");
    PathBuf::from(os)
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Write `bytes` to `path` via `NamedTempFile` -> `sync_all` -> `persist` ->
/// `sync_all(dir on Unix)` without pre-deleting `path`.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = parent_dir(path);
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create state directory {}", parent.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create temp file in {}", parent.display()))?;
    tmp.write_all(bytes)
        .with_context(|| format!("write temp file for {}", path.display()))?;
    tmp.as_file()
        .sync_all()
        .with_context(|| format!("sync temp file for {}", path.display()))?;
    tmp.persist(path)
        .map_err(|err| err.error)
        .with_context(|| format!("replace {} atomically", path.display()))?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .with_context(|| format!("sync parent directory {}", parent.display()))?;
    Ok(())
}

/// Move a corrupt or oversized state file aside to `<path>.corrupt-<unix_ms>`
/// before failing closed, so the operator can inspect it and subsequent runs
/// do not keep hitting the damaged file without notice.
pub(crate) fn quarantine_corrupt_file(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    let ts_ms = chrono::Utc::now().timestamp_millis().max(0);
    let mut candidate = {
        let mut os = path.as_os_str().to_os_string();
        os.push(format!(".corrupt-{ts_ms}"));
        PathBuf::from(os)
    };
    if candidate.exists() {
        let mut found = None;
        for attempt in 1u32..1000 {
            let mut os = path.as_os_str().to_os_string();
            os.push(format!(".corrupt-{ts_ms}-{attempt}"));
            let next = PathBuf::from(os);
            if !next.exists() {
                found = Some(next);
                break;
            }
        }
        candidate = found?;
    }
    match std::fs::rename(path, &candidate) {
        Ok(()) => {
            tracing::warn!(
                "quarantined corrupt resolver state file {} -> {}",
                DisplayText::for_terminal(&path.display().to_string()),
                DisplayText::for_terminal(&candidate.display().to_string())
            );
            Some(candidate)
        }
        Err(err) => {
            tracing::warn!(
                "failed to quarantine corrupt resolver state file {}: {}",
                DisplayText::for_terminal(&path.display().to_string()),
                err
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_and_quarantine_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("state.json");
        atomic_write(&target, br#"{"v":1}"#).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), br#"{"v":1}"#);
        atomic_write(&target, br#"{"v":2}"#).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), br#"{"v":2}"#);

        let quarantined = quarantine_corrupt_file(&target).expect("quarantined");
        assert!(!target.exists());
        assert_eq!(std::fs::read(&quarantined).unwrap(), br#"{"v":2}"#);
    }
}
