// crates/node/src/ipfs_fetch.rs
//! Bounded, injection-safe IPFS `/api/v0/cat` fetcher (`VNA-11`, `VNA-13`).

use common::{parse_env_bool, validate_ipfs_cid, validate_ipfs_cid_strict};
use reqwest::Client;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum IpfsFetchError {
    #[error("invalid IPFS CID {cid:?}: {reason}")]
    InvalidCid { cid: String, reason: &'static str },
    #[error("IPFS request failed for CID {cid}: {source}")]
    Transport {
        cid: String,
        #[source]
        source: anyhow::Error,
    },
    #[error("IPFS returned HTTP {status} for CID {cid}: {body}")]
    HttpStatus {
        cid: String,
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("IPFS payload for CID {cid} is {size} bytes, above limit {limit} bytes")]
    PayloadTooLarge { cid: String, size: u64, limit: u64 },
}

/// Validates a CID using strict rules in `CREG_PRODUCTION=true` mode or
/// canonical ASCII alphanumeric rules otherwise (`VNA-13`).
pub fn validate_cid_for_node(cid: &str) -> Result<(), &'static str> {
    if parse_env_bool("CREG_PRODUCTION") {
        validate_ipfs_cid_strict(cid)
    } else {
        validate_ipfs_cid(cid)
    }
}

/// Downloads an IPFS object via Kubo `/api/v0/cat` using query parameter
/// encoding and streaming chunk accounting bounded to `max_bytes` BEFORE allocation.
pub async fn fetch_ipfs_bounded(
    cid: &str,
    ipfs_url: &str,
    max_bytes: u64,
    timeout_secs: u64,
) -> Result<Vec<u8>, IpfsFetchError> {
    validate_cid_for_node(cid).map_err(|reason| IpfsFetchError::InvalidCid {
        cid: cid.to_string(),
        reason,
    })?;

    let base = ipfs_url.trim_end_matches('/');
    let endpoint = format!("{base}/api/v0/cat");
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .build()
        .map_err(|source| IpfsFetchError::Transport {
            cid: cid.to_string(),
            source: source.into(),
        })?;

    let mut response = client
        .post(&endpoint)
        .query(&[("arg", cid)])
        .send()
        .await
        .map_err(|source| IpfsFetchError::Transport {
            cid: cid.to_string(),
            source: source.into(),
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(IpfsFetchError::HttpStatus {
            cid: cid.to_string(),
            status,
            body,
        });
    }

    if let Some(len) = response.content_length() {
        if len > max_bytes {
            return Err(IpfsFetchError::PayloadTooLarge {
                cid: cid.to_string(),
                size: len,
                limit: max_bytes,
            });
        }
    }

    let initial_cap = response
        .content_length()
        .unwrap_or(0)
        .min(max_bytes)
        .min(1024 * 1024) as usize;
    let mut payload = Vec::with_capacity(initial_cap);

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|source| IpfsFetchError::Transport {
            cid: cid.to_string(),
            source: source.into(),
        })?
    {
        let next_len = (payload.len() as u64).saturating_add(chunk.len() as u64);
        if next_len > max_bytes {
            return Err(IpfsFetchError::PayloadTooLarge {
                cid: cid.to_string(),
                size: next_len,
                limit: max_bytes,
            });
        }
        payload.extend_from_slice(&chunk);
    }

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_chunked_server(chunks: Vec<&'static [u8]>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut req_buf = [0u8; 1024];
                let _ = socket.read(&mut req_buf).await;
                let header = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
                let _ = socket.write_all(header.as_bytes()).await;
                for c in &chunks {
                    let prefix = format!("{:x}\r\n", c.len());
                    let _ = socket.write_all(prefix.as_bytes()).await;
                    let _ = socket.write_all(c).await;
                    let _ = socket.write_all(b"\r\n").await;
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn rejects_cid_with_query_injection_before_network() {
        let err = fetch_ipfs_bounded("bafyabc&offline=true", "http://127.0.0.1:1", 1024, 2)
            .await
            .unwrap_err();
        assert!(matches!(err, IpfsFetchError::InvalidCid { .. }));
    }

    #[tokio::test]
    async fn aborts_chunked_transfer_exceeding_max_bytes() {
        let url = spawn_chunked_server(vec![b"123456", b"789012"]).await;
        let err = fetch_ipfs_bounded("bafychunktest", &url, 10, 5)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            IpfsFetchError::PayloadTooLarge {
                limit: 10,
                size: 12,
                ..
            }
        ));

        let ok = fetch_ipfs_bounded("bafychunktest", &url, 12, 5)
            .await
            .expect("exact limit succeeds");
        assert_eq!(ok, b"123456789012");
    }
}
