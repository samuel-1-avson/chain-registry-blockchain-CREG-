use common::PublishRequest;

const DEFAULT_MAX_SCAN_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_IPFS_TIMEOUT_SECS: u64 = 30;
const BLOCKING_YARA_THREAT_LEVEL: u8 = 4;

#[derive(Debug, thiserror::Error)]
pub enum AdmissionScanError {
    #[error("pre-mempool YARA rules are unavailable; set CREG_YARA_RULES_DIR to a directory containing .yar/.yara rules")]
    RulesUnavailable,
    #[error("IPFS fetch failed for CID {cid}: {source}")]
    IpfsFetch {
        cid: String,
        #[source]
        source: anyhow::Error,
    },
    #[error(
        "IPFS payload for CID {cid} is {size} bytes, above pre-mempool scan limit {limit} bytes"
    )]
    PayloadTooLarge { cid: String, size: u64, limit: u64 },
    #[error("content hash mismatch for {canonical}: declared {declared}, computed {computed}")]
    ContentHashMismatch {
        canonical: String,
        declared: String,
        computed: String,
    },
    #[error("YARA extraction failed for {canonical}: {source}")]
    ExtractionFailed {
        canonical: String,
        #[source]
        source: ml_validator::MlError,
    },
    #[error("YARA rejected {canonical}: {summary}")]
    Rejected { canonical: String, summary: String },
}

fn max_scan_bytes() -> u64 {
    std::env::var("CREG_PRE_MEMPOOL_SCAN_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_SCAN_BYTES)
}

fn ipfs_timeout_secs() -> u64 {
    std::env::var("CREG_PRE_MEMPOOL_IPFS_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_IPFS_TIMEOUT_SECS)
}

async fn fetch_ipfs_payload(cid: &str, ipfs_url: &str) -> Result<Vec<u8>, AdmissionScanError> {
    let limit = max_scan_bytes();
    match crate::ipfs_fetch::fetch_ipfs_bounded(cid, ipfs_url, limit, ipfs_timeout_secs()).await {
        Ok(bytes) => Ok(bytes),
        Err(crate::ipfs_fetch::IpfsFetchError::PayloadTooLarge { cid, size, limit }) => {
            Err(AdmissionScanError::PayloadTooLarge { cid, size, limit })
        }
        Err(err) => Err(AdmissionScanError::IpfsFetch {
            cid: cid.to_string(),
            source: err.into(),
        }),
    }
}

pub async fn run_pre_mempool_yara_gate(
    request: &PublishRequest,
    ipfs_url: &str,
) -> Result<(), AdmissionScanError> {
    if request.shielded {
        // IPFS holds encrypted bytes; plaintext hash is checked after decryption in
        // validator_pipeline. YARA on ciphertext is not meaningful at admission.
        tracing::debug!(
            canonical = %request.id.canonical(),
            "Skipping pre-mempool YARA for shielded publish (SEC-305)"
        );
        return Ok(());
    }

    if !ml_validator::yara_scanner::rules_available() {
        return Err(AdmissionScanError::RulesUnavailable);
    }

    let canonical = request.id.canonical();
    let payload = fetch_ipfs_payload(&request.ipfs_cid, ipfs_url).await?;
    let computed_hash = common::sha256_hex(&payload);
    if computed_hash != request.content_hash {
        return Err(AdmissionScanError::ContentHashMismatch {
            canonical,
            declared: request.content_hash.clone(),
            computed: computed_hash,
        });
    }

    let ecosystem = request.id.ecosystem.clone();
    let matches = tokio::task::spawn_blocking(move || {
        ml_validator::deep_scan::scan_tarball_with_yara(&payload, &ecosystem)
    })
    .await
    .map_err(|e| AdmissionScanError::ExtractionFailed {
        canonical: request.id.canonical(),
        source: ml_validator::MlError::ExtractionError(e.to_string()),
    })?
    .map_err(|source| AdmissionScanError::ExtractionFailed {
        canonical: request.id.canonical(),
        source,
    })?;

    let blocking: Vec<_> = matches
        .into_iter()
        .filter(|m| m.threat_level >= BLOCKING_YARA_THREAT_LEVEL)
        .collect();

    if !blocking.is_empty() {
        let summary = blocking
            .iter()
            .take(5)
            .map(|m| {
                format!(
                    "{}:{}:{}:{}",
                    m.rule_name, m.threat_level, m.category, m.matched_file
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AdmissionScanError::Rejected {
            canonical: request.id.canonical(),
            summary,
        });
    }

    tracing::info!(
        "Pre-mempool YARA gate accepted {} from CID {}",
        request.id.canonical(),
        request.ipfs_cid
    );
    Ok(())
}
