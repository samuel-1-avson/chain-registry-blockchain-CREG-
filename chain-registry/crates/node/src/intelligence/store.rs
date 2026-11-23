use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use common::PackageIntelligenceReport;

/// Lane C reports received from `creg-agent-runner`, keyed by content hash.
/// Public routes serve only `registry_view::IntelligenceSummaryV1` built from
/// these; the full report is operator-only (ADR-019 §11 #7).
#[derive(Debug, Clone)]
pub struct IntelligenceStore {
    dir: PathBuf,
}

impl IntelligenceStore {
    pub fn new(data_dir: &Path) -> Self {
        let dir = data_dir.join("intelligence");
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    /// File for a SHA-256 content hash; `None` for anything else, so a
    /// malformed hash can never name a path outside the store.
    fn path_for(&self, content_hash: &str) -> Option<PathBuf> {
        let key = content_hash
            .trim()
            .trim_start_matches("0x")
            .to_ascii_lowercase();
        (key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| self.dir.join(format!("{key}.json")))
    }

    pub fn get_by_content_hash(&self, content_hash: &str) -> Option<PackageIntelligenceReport> {
        let path = self.path_for(content_hash)?;
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn put(&self, report: &PackageIntelligenceReport) -> Result<()> {
        let path = self
            .path_for(&report.content_hash)
            .context("intelligence report content hash is not a SHA-256 hex digest")?;
        let json = serde_json::to_string_pretty(report).context("serialize intelligence report")?;
        std::fs::write(path, json).context("write intelligence report")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_sha256_keys_name_files_inside_the_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = IntelligenceStore::new(dir.path());
        let hash = "ab".repeat(32);
        let report = PackageIntelligenceReport::pending("npm:x@1.0.0", &hash);
        store.put(&report).expect("put");
        assert!(store
            .get_by_content_hash(&hash.to_ascii_uppercase())
            .is_some());

        let escaping = PackageIntelligenceReport::pending("npm:x@1.0.0", "../../etc/passwd");
        assert!(store.put(&escaping).is_err());
        assert!(store.get_by_content_hash("../../etc/passwd").is_none());
    }
}
