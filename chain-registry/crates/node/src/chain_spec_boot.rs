// crates/node/src/chain_spec_boot.rs
// Chain spec resolution: fetch → cache → validate → apply.

/// Resolve a chain spec from network or cache.
///
/// 1. If `url` is provided and `offline` is false, fetch from network with retry.
/// 2. On fetch success, validate JSON parse and cache to disk.
/// 3. On fetch failure (or offline), fall back to cached copy.
/// 4. If no cache exists, return an error.
pub async fn resolve_chain_spec(
    url: Option<&str>,
    data_dir: &std::path::Path,
    offline: bool,
) -> anyhow::Result<common::ChainSpec> {
    let cache_path = data_dir.join("chain-spec.cached.json");

    // Try network fetch first (unless offline)
    if !offline {
        if let Some(url) = url {
            match fetch_with_retry(url, 3).await {
                Ok(json) => {
                    let spec: common::ChainSpec = serde_json::from_str(&json)
                        .map_err(|e| anyhow::anyhow!("Failed to parse fetched spec: {}", e))?;
                    // Cache for next boot
                    if let Err(e) = tokio::fs::write(&cache_path, &json).await {
                        tracing::warn!(
                            "Failed to cache chain spec to {}: {}",
                            cache_path.display(),
                            e
                        );
                    }
                    return Ok(spec);
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch chain spec from {}: {}", url, e);
                }
            }
        }
    }

    // Fallback to cache
    if cache_path.exists() {
        let json = tokio::fs::read_to_string(&cache_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read cached spec: {}", e))?;
        let spec: common::ChainSpec = serde_json::from_str(&json)
            .map_err(|e| anyhow::anyhow!("Failed to parse cached spec: {}", e))?;
        tracing::warn!(
            "Using cached chain spec from {} (network fetch failed or offline)",
            cache_path.display()
        );
        return Ok(spec);
    }

    anyhow::bail!(
        "No chain spec available. Set CREG_CHAIN_SPEC_URL or CREG_CHAIN_SPEC_OFFLINE=true with a cached spec."
    )
}

/// Fetch a URL with exponential backoff retry.
async fn fetch_with_retry(url: &str, max_retries: u32) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    for attempt in 1..=max_retries {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                return Ok(resp.text().await?);
            }
            Ok(resp) => {
                tracing::warn!(
                    "Chain spec fetch attempt {}/{}: HTTP {}",
                    attempt,
                    max_retries,
                    resp.status()
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Chain spec fetch attempt {}/{}: {}",
                    attempt,
                    max_retries,
                    e
                );
            }
        }
        let delay = std::time::Duration::from_secs(2u64.pow((attempt - 1).min(4)));
        tokio::time::sleep(delay).await;
    }

    anyhow::bail!("Failed to fetch chain spec after {} retries", max_retries)
}

/// Fetch the detached signature for a chain spec.
pub async fn fetch_spec_signature(url: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} fetching signature", resp.status());
    }
    Ok(resp.text().await?.trim().to_string())
}

/// Resolve the detached chain-spec signature from an explicit local file,
/// the network, or the data-directory cache. The caller must still verify the
/// returned signature against the binary-pinned key and the resolved spec.
pub async fn resolve_spec_signature(
    url: &str,
    data_dir: &std::path::Path,
    offline: bool,
    signature_path: Option<&std::path::Path>,
) -> anyhow::Result<String> {
    if let Some(path) = signature_path {
        return read_signature(path).await;
    }

    let cache_path = data_dir.join("chain-spec.cached.json.sig");
    if !offline {
        match fetch_spec_signature(url).await {
            Ok(signature) => {
                if let Err(error) = tokio::fs::write(&cache_path, &signature).await {
                    tracing::warn!(
                        "Failed to cache chain-spec signature to {}: {}",
                        cache_path.display(),
                        error
                    );
                }
                return Ok(signature);
            }
            Err(error) => {
                tracing::warn!(
                    "Failed to fetch chain-spec signature from {}: {}",
                    url,
                    error
                );
            }
        }
    }

    read_signature(&cache_path).await.map_err(|error| {
        anyhow::anyhow!(
            "could not resolve chain-spec signature (offline={offline}, cache={}): {error}",
            cache_path.display()
        )
    })
}

async fn read_signature(path: &std::path::Path) -> anyhow::Result<String> {
    let signature = tokio::fs::read_to_string(path).await?;
    let signature = signature.trim();
    if signature.is_empty() {
        anyhow::bail!("chain-spec signature at {} is empty", path.display());
    }
    Ok(signature.to_owned())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_with_retry_hits_mock_server() {
        // This test requires a mock HTTP server; placeholder for now.
        // In practice, use wiremock or a local tokio::net::TcpListener.
    }

    #[test]
    fn test_cache_path_construction() {
        let dir = std::path::PathBuf::from("/tmp/creg");
        let path = dir.join("chain-spec.cached.json");
        assert_eq!(
            path,
            std::path::PathBuf::from("/tmp/creg/chain-spec.cached.json")
        );
    }

    #[tokio::test]
    async fn explicit_signature_path_works_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let signature_path = dir.path().join("chain-spec.json.sig");
        tokio::fs::write(&signature_path, "  signed-spec  ")
            .await
            .unwrap();

        let signature = resolve_spec_signature(
            "http://127.0.0.1:1/unreachable",
            dir.path(),
            false,
            Some(&signature_path),
        )
        .await
        .unwrap();

        assert_eq!(signature, "signed-spec");
    }

    #[tokio::test]
    async fn offline_signature_uses_cached_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("chain-spec.cached.json.sig");
        tokio::fs::write(&cache_path, "cached-signature")
            .await
            .unwrap();

        let signature =
            resolve_spec_signature("http://127.0.0.1:1/unreachable", dir.path(), true, None)
                .await
                .unwrap();

        assert_eq!(signature, "cached-signature");
    }
}
