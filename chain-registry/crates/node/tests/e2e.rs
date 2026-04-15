// crates/node/tests/e2e.rs
// End-to-end tests: spin up a real in-process node on a random port,
// submit a package, wait for it to be verified, and confirm via the API.

use chrono::Utc;
use common::{PackageId, PackageManifest, PublishRequest};
use std::{sync::Arc, time::Duration};
use tokio::{sync::RwLock, time::timeout};

/// Helper: start a full node on a random port, return the base URL.
async fn start_test_node() -> (String, tokio::task::JoinHandle<()>) {
    use node::{
        api,
        chain_store::ChainStore,
        config::NodeConfig,
        events::new_event_bus,
        finalized_tx,
        p2p::{P2PCommand, P2PHandle},
        pending_pool::PendingPool,
        publisher_index::PublisherIndex,
        rate_limit::{RateLimitConfig, RateLimiter},
        BridgeStatus, NodeState, P2PStatus,
    };

    let dir = tempfile::TempDir::new().expect("tempdir");
    let chain = ChainStore::open(dir.path()).expect("chain store");

    let config = NodeConfig {
        listen_addr: "127.0.0.1:0".into(), // OS assigns a port
        data_dir: dir.path().to_path_buf(),
        node_id: "e2e-node".into(),
        validator_privkey: None,
        is_validator: true,
        peers: vec![],
        block_interval_secs: 1,
        ipfs_url: "http://127.0.0.1:5001".into(),
        ..NodeConfig::default()
    };

    let event_bus = new_event_bus();
    let (tx_s, tx_r) = finalized_tx::channel();

    // Create a no-op P2P handle (the real P2P stack requires a live network).
    let (p2p_sender, _p2p_rx) = tokio::sync::mpsc::channel::<P2PCommand>(1);
    let p2p = P2PHandle { sender: p2p_sender };

    // ZkValidator generates ephemeral keys when none are found on disk.
    let zk_validator = std::sync::Arc::new(
        zk_validator::ZkValidator::new().expect("ZkValidator init for e2e tests"),
    );

    let state: Arc<RwLock<NodeState>> = Arc::new(RwLock::new(NodeState {
        chain,
        pending_pool: PendingPool::new(),
        publisher_index: PublisherIndex::new(),
        validator_set: common::ValidatorSet::default(),
        votes: std::collections::HashMap::new(),
        config: config.clone(),
        event_bus: Arc::clone(&event_bus),
        p2p,
        zk_validator,
        tx_sender: tx_s.clone(),
        p2p_status: P2PStatus::default(),
        bridge_status: BridgeStatus::default(),
        vrf_proofs: std::collections::HashMap::new(),
        decryption_shares: std::collections::HashMap::new(),
        validator_registrations: std::collections::HashMap::new(),
        view_change_certs: std::collections::HashMap::new(),
    }));

    let limiter = RateLimiter::new(RateLimitConfig::default());

    let app = api::router(Arc::clone(&state), event_bus, limiter);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}", addr);

    let state_bp = Arc::clone(&state);
    let state_vp = Arc::clone(&state);

    let handle = tokio::spawn(async move {
        tokio::spawn(node::block_producer::run(state_bp, tx_r));
        tokio::spawn(node::validator_pipeline::run(state_vp, tx_s));
        axum::serve(listener, app).await.unwrap();
    });

    // Give the node a moment to fully start.
    tokio::time::sleep(Duration::from_millis(50)).await;

    (url, handle)
}

/// Build a minimal signed PublishRequest for testing.
fn make_request(ecosystem: &str, name: &str, version: &str) -> PublishRequest {
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    let signing_key = SigningKey::generate(&mut OsRng);
    let pubkey_hex = hex::encode(signing_key.verifying_key().as_bytes());
    let id = PackageId::new(ecosystem, name, version);
    let content_hash = common::sha256_hex(b"test-tarball-bytes");
    let msg = format!("{}{}", id.canonical(), content_hash);
    let sig = signing_key.sign(msg.as_bytes());

    PublishRequest {
        id,
        content_hash,
        ipfs_cid: format!("bafyDev{}", &common::sha256_hex(b"dev")[..32]),
        publisher_pubkey: pubkey_hex,
        signature: hex::encode(sig.to_bytes()),
        manifest: PackageManifest::default(),
        submitted_at: Utc::now(),
        ..Default::default()
    }
}

#[tokio::test]
async fn e2e_health_check() {
    let (url, _handle) = start_test_node().await;
    let resp = reqwest::get(format!("{}/v1/health", url)).await.unwrap();
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn e2e_submit_and_verify_package() {
    let (url, _handle) = start_test_node().await;

    let request = make_request("npm", "e2e-test-pkg", "1.0.0");
    let canonical = request.id.canonical();

    // Submit the package.
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/packages", url))
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202, "Expected 202 Accepted");

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "accepted");

    // Wait for the validator pipeline + block producer to verify it.
    // In dev mode (no IPFS, no real tarball) this happens quickly.
    let encoded = urlencoding::encode(&canonical).to_string();
    let verified = timeout(Duration::from_secs(10), async {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let resp = reqwest::get(format!("{}/v1/packages/{}", url, encoded))
                .await
                .unwrap();
            if resp.status() == 404 {
                continue;
            }
            let body: serde_json::Value = resp.json().await.unwrap();
            match body["status"].as_str() {
                Some("verified") => return true,
                Some("revoked") => return false,
                _ => continue,
            }
        }
    })
    .await;

    assert!(verified.is_ok(), "Timed out waiting for verification");
    assert!(verified.unwrap(), "Package should be verified, not revoked");
}

#[tokio::test]
async fn e2e_duplicate_submission_rejected() {
    let (url, _handle) = start_test_node().await;
    let request = make_request("npm", "e2e-dup-pkg", "2.0.0");

    // First submission.
    let r1 = reqwest::Client::new()
        .post(format!("{}/v1/packages", url))
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), 202);

    // Wait for verification.
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Second submission of the same package should be rejected.
    let r2 = reqwest::Client::new()
        .post(format!("{}/v1/packages", url))
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), 409, "Duplicate should return 409 Conflict");
}

#[tokio::test]
async fn e2e_invalid_signature_rejected() {
    let (url, _handle) = start_test_node().await;
    let mut request = make_request("npm", "e2e-sig-pkg", "1.0.0");

    // Corrupt the signature.
    request.signature = "deadbeef".repeat(8);

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/packages", url))
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "Invalid signature should return 400");
}

#[tokio::test]
async fn e2e_chain_stats_increase_after_verification() {
    let (url, _handle) = start_test_node().await;

    // Get baseline stats.
    let before: serde_json::Value = reqwest::get(format!("{}/v1/chain/stats", url))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let height_before = before["tip_height"].as_u64().unwrap_or(0);

    // Submit a package.
    let request = make_request("cargo", "e2e-stats-crate", "0.1.0");
    reqwest::Client::new()
        .post(format!("{}/v1/packages", url))
        .json(&request)
        .send()
        .await
        .unwrap();

    // Wait for a block to be produced.
    tokio::time::sleep(Duration::from_secs(4)).await;

    let after: serde_json::Value = reqwest::get(format!("{}/v1/chain/stats", url))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let height_after = after["tip_height"].as_u64().unwrap_or(0);

    assert!(
        height_after > height_before,
        "Chain height should increase after verification"
    );
}

#[tokio::test]
async fn e2e_sse_receives_events() {
    let (url, _handle) = start_test_node().await;

    // Connect to SSE stream with a short timeout.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap();

    let mut resp = client
        .get(format!("{}/v1/events", url))
        .header("Accept", "text/event-stream")
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success());

    // Submit a package — should trigger a submitted event.
    let request = make_request("pypi", "e2e-sse-pkg", "1.0.0");
    reqwest::Client::new()
        .post(format!("{}/v1/packages", url))
        .json(&request)
        .send()
        .await
        .unwrap();

    // Read a few chunks from the stream and check we get event data.
    let mut received_data = false;
    for _ in 0..5 {
        match timeout(Duration::from_secs(2), resp.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                let text = String::from_utf8_lossy(&chunk);
                if text.contains("data:") && text.contains("canonical") {
                    received_data = true;
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(received_data, "SSE stream should deliver package events");
}

#[tokio::test]
async fn e2e_prometheus_metrics_endpoint() {
    let (url, _handle) = start_test_node().await;

    let resp = reqwest::get(format!("{}/metrics", url)).await.unwrap();
    assert!(resp.status().is_success());

    let body = resp.text().await.unwrap();
    assert!(
        body.contains("creg_chain_height"),
        "Metrics should include chain height"
    );
    assert!(
        body.contains("creg_package_count"),
        "Metrics should include package count"
    );
    assert!(
        body.contains("creg_pending_pool_size"),
        "Metrics should include pending pool size"
    );
}
