//! Runtime identity and readiness for deterministic package-analysis inputs.

use crate::{osv_snapshot, threat_intel, yara_scanner};

pub const SCANNER_VERSION: &str = "creg-deterministic-scanner-v1.1.0";

#[derive(Debug, Clone)]
pub struct ScannerProfileStatus {
    pub ready: bool,
    pub strict_phase_c: bool,
    pub scanner_version: String,
    pub digest: String,
    pub yara_available: bool,
    pub yara_digest: String,
    pub yara_files: usize,
    pub threat_intel_available: bool,
    pub threat_intel_digest: String,
    pub threat_intel_source: String,
    pub threat_intel_built_at: String,
    pub threat_intel_entries: usize,
    pub osv_available: bool,
    pub osv_digest: String,
    pub osv_epoch: String,
    pub osv_source: String,
    pub osv_built_at: String,
    pub osv_lookups: usize,
    pub osv_records: usize,
    pub errors: Vec<String>,
}

pub fn current() -> ScannerProfileStatus {
    let yara = yara_scanner::rules_status();
    let threat = threat_intel::database_status();
    let osv = osv_snapshot::snapshot_status();
    let strict_phase_c = cfg!(feature = "phase-c-real-fleet");

    let mut errors = Vec::new();
    if !yara.available {
        errors.push(format!(
            "YARA rule bundle is unavailable: {}",
            yara.error.as_deref().unwrap_or("unknown error")
        ));
    }
    if strict_phase_c && !threat.available {
        errors.push(format!(
            "real threat-intelligence snapshot is unavailable: {}",
            threat.error.as_deref().unwrap_or("unknown error")
        ));
    }
    if strict_phase_c && !osv.available {
        errors.push(format!(
            "pinned OSV snapshot is unavailable: {}",
            osv.error.as_deref().unwrap_or("unknown error")
        ));
    }
    if strict_phase_c && osv_snapshot::osv_live_fallback_enabled() {
        errors.push("live OSV fallback must be disabled for consensus scanning".to_string());
    }

    let threat_digest = if threat.digest.is_empty() {
        "missing"
    } else {
        threat.digest.as_str()
    };
    let osv_digest = if osv.digest.is_empty() {
        "missing"
    } else {
        osv.digest.as_str()
    };
    let yara_digest = if yara.digest.is_empty() {
        "missing"
    } else {
        yara.digest.as_str()
    };
    let digest_input = format!(
        "creg-scanner-profile-v1|version={SCANNER_VERSION}|yara={yara_digest}|threat={threat_digest}|osv_enabled={}|osv_epoch={}|osv={osv_digest}|osv_block_critical={}|osv_live_fallback={}",
        osv.enabled,
        osv.epoch,
        osv_snapshot::osv_block_critical_enabled(),
        osv_snapshot::osv_live_fallback_enabled(),
    );
    let digest = common::sha256_hex(digest_input.as_bytes());
    let expected_digest = std::env::var("CREG_SCANNER_PROFILE_DIGEST")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if strict_phase_c {
        match expected_digest.as_deref() {
            Some(expected) if expected == digest => {}
            Some(_) => errors.push(
                "runtime scanner profile does not match CREG_SCANNER_PROFILE_DIGEST".to_string(),
            ),
            None => errors.push(
                "CREG_SCANNER_PROFILE_DIGEST must pin the reviewed scanner profile".to_string(),
            ),
        }
    }
    let ready = errors.is_empty() && (!strict_phase_c || (threat.available && osv.available));
    let scanner_version = if !yara.available {
        "degraded-yara-missing".to_string()
    } else if strict_phase_c && !ready {
        "degraded-scanner-assets-missing".to_string()
    } else {
        format!("{SCANNER_VERSION}+sha256:{digest}")
    };

    ScannerProfileStatus {
        ready,
        strict_phase_c,
        scanner_version,
        digest,
        yara_available: yara.available,
        yara_digest: yara.digest,
        yara_files: yara.files,
        threat_intel_available: threat.available,
        threat_intel_digest: threat.digest,
        threat_intel_source: threat.source,
        threat_intel_built_at: threat.built_at,
        threat_intel_entries: threat.entries,
        osv_available: osv.available,
        osv_digest: osv.digest,
        osv_epoch: osv.epoch,
        osv_source: osv.source,
        osv_built_at: osv.built_at,
        osv_lookups: osv.lookups,
        osv_records: osv.records,
        errors,
    }
}
