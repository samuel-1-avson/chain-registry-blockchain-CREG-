// crates/node/src/main.rs
// Chain registry node — single binary that runs all subsystems.

mod api;
mod block_producer;
mod chain_store;
mod config;
mod db_sync_proxy;
mod events;
mod finalized_tx;
mod gossip;
mod metrics;
mod pending_pool;
mod proof;
mod publisher_index;
mod sync;
mod explorer;
mod rate_limit;
mod p2p_rate_limit;
mod validator_pipeline;
mod p2p;
mod bridge;
mod grpc;

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::Serialize;
use tracing_subscriber::EnvFilter;

use events::{EventBus, new_event_bus};
use finalized_tx::{FinalizedTxReceiver, FinalizedTxSender};
use publisher_index::PublisherIndex;

/// Shared mutable state passed to every subsystem via Arc<RwLock<_>>.
pub struct NodeState {
    pub chain:           chain_store::ChainStore,
    pub pending_pool:    pending_pool::PendingPool,
    pub publisher_index: PublisherIndex,
    pub validator_set:   common::ValidatorSet,
    pub votes:           std::collections::HashMap<String, Vec<common::ValidatorSignature>>, // block_hash/canonical -> sigs
    pub config:          config::NodeConfig,
    pub event_bus:       EventBus,
    pub p2p:             p2p::P2PHandle,
    pub zk_validator:    Arc<zk_validator::ZkValidator>,
    pub tx_sender:       FinalizedTxSender,
    // Live metrics for Explorer
    pub p2p_status:      P2PStatus,
    pub bridge_status:   BridgeStatus,
    /// Cached VRF proofs from other validators: validator_id -> (output, proof)
    pub vrf_proofs:      std::collections::HashMap<String, (String, String)>,
}

#[derive(Serialize, Clone, Default)]
pub struct P2PStatus {
    pub peers: Vec<String>,
    pub protocols: Vec<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct BridgeStatus {
    pub last_finalized_eth_block: u64,
    pub registry_address: String,
    pub bridge_sync_status: String,
    pub current_state_root: String,
}

pub type SharedState = Arc<RwLock<NodeState>>;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info"))
        )
        .with_target(true)
        .init();

    let config = config::NodeConfig::from_env();

    // ── Validate configuration early — fail fast with clear messages ──────────
    let config_errors = config.validate();
    if !config_errors.is_empty() {
        tracing::warn!("Configuration warnings/errors:");
        for err in &config_errors {
            tracing::warn!("  ✗ {}", err);
        }
        // Non-fatal for warnings (e.g. zero registry addr), but validator key
        // absence on a validator node is a hard stop.
        let hard_errors: Vec<_> = config_errors.iter()
            .filter(|e| e.contains("CREG_VALIDATOR_KEY"))
            .collect();
        if !hard_errors.is_empty() {
            anyhow::bail!("Cannot start validator node due to configuration errors. Fix the above and restart.");
        }
    }

    tracing::info!("╔══════════════════════════════════════╗");
    tracing::info!("║    chain-registry node v{}        ║", env!("CARGO_PKG_VERSION"));
    tracing::info!("╚══════════════════════════════════════╝");
    tracing::info!("  listen:      {}", config.listen_addr);
    tracing::info!("  data dir:    {}", config.data_dir.display());
    tracing::info!("  node id:     {}", config.node_id);
    tracing::info!("  validator:   {}", config.is_validator);
    tracing::info!("  peers:       {}", config.peers.len());

    // ── Open persistent storage ───────────────────────────────────────────────
    let chain = chain_store::ChainStore::open(&config.data_dir)?;
    let chain_for_sync = chain.clone();
    let tip   = chain.tip_height()?;
    tracing::info!("  chain tip:   height={}", tip);

    // ── Rebuild publisher index from chain history ────────────────────────────
    let mut publisher_index = PublisherIndex::new();
    {
        let mut blocks = Vec::new();
        for h in 0..=tip {
            if let Ok(Some(b)) = chain.get_block_by_height(h) {
                blocks.push(b);
            }
        }
        publisher_index.rebuild_from_chain(blocks.iter());
        tracing::info!("  publishers:  {}", publisher_index.publisher_count());
    }

    // ── Event bus (broadcast channel for SSE clients) ─────────────────────────
    let event_bus = new_event_bus();

    // ── P2P Networking (libp2p) ───────────────────────────────────────────────
    let (p2p_node, p2p_handle) = p2p::P2PNode::new(&config.p2p_listen)?;
    
    // ── Finalized-tx channel (created before state so API can send to it) ───────
    let (tx_sender, tx_receiver): (FinalizedTxSender, FinalizedTxReceiver) =
        finalized_tx::channel();

    // ── Shared state ──────────────────────────────────────────────────────────
    let state: SharedState = Arc::new(RwLock::new(NodeState {
        chain,
        pending_pool:    pending_pool::PendingPool::new(),
        publisher_index,
        validator_set:   config.validator_set.clone(),
        votes:           std::collections::HashMap::new(),
        config:          config.clone(),
        event_bus:       Arc::clone(&event_bus),
        p2p:             p2p_handle.clone(),
        zk_validator:    Arc::new(zk_validator::ZkValidator::default()),
        tx_sender:       tx_sender.clone(),
        p2p_status:      P2PStatus::default(),
        bridge_status:   BridgeStatus {
            registry_address: config.registry_addr.clone(),
            ..BridgeStatus::default()
        },
        vrf_proofs:      std::collections::HashMap::new(),
    }));

    // Start P2P node in background
    let p2p_handle_for_seeds = p2p_handle.clone();
    let seeds = config.p2p_seeds.clone();
    tokio::spawn(async move {
        for seed in seeds {
            if let Ok(addr) = seed.parse() {
                let _ = p2p_handle_for_seeds.sender.send(p2p::P2PCommand::Dial { addr }).await;
            }
        }
    });

    tokio::spawn(p2p_node.run(Arc::clone(&state)));

    // ── Spawn background tasks ────────────────────────────────────────────────
    tracing::info!("Starting subsystems...");

    // PostgreSQL sync worker (sled → PostgreSQL ETL)
    if !config.pg_url.is_empty() {
        let sync_config = db_sync::sync_worker::SyncConfig {
            poll_interval: std::time::Duration::from_secs(1),
            pg_url: config.pg_url.clone(),
        };
        let chain_proxy: db_sync::sync_worker::ChainStoreHandle =
            Arc::new(tokio::sync::RwLock::new(chain_for_sync));
        match db_sync::SyncWorker::new(sync_config, chain_proxy).await {
            Ok(worker) => {
                tokio::spawn(worker.run());
                tracing::info!("PostgreSQL sync worker started");
            }
            Err(e) => {
                tracing::warn!("Failed to start PostgreSQL sync worker: {}", e);
            }
        }
    }

    tokio::spawn(sync::run(
        Arc::clone(&state),
    ));

    tokio::spawn(validator_pipeline::run(
        Arc::clone(&state),
        tx_sender,
    ));

    tokio::spawn(block_producer::run(
        Arc::clone(&state),
        tx_receiver,
    ));

    tokio::spawn(bridge::run(
        Arc::clone(&state),
    ));

    // ── Start gRPC Server (Industrial Speed) ──────────────────────────────────
    let grpc_state = Arc::clone(&state);
    tokio::spawn(async move {
        let addr = "0.0.0.0:50051".parse().expect("gRPC bind address must be valid");
        let registry = grpc::MyRegistry::new(Arc::clone(&grpc_state));
        let watcher  = grpc::MyWatcher::new(Arc::clone(&grpc_state));
        let explorer = grpc::MyExplorer::new(Arc::clone(&grpc_state));

        tracing::info!("gRPC API listening on {}", addr);

        tonic::transport::Server::builder()
            .add_service(common::proto::registry_service_server::RegistryServiceServer::new(registry))
            .add_service(common::proto::watch_service_server::WatchServiceServer::new(watcher))
            .add_service(common::proto::explorer_service_server::ExplorerServiceServer::new(explorer))
            .serve(addr)
            .await
            .expect("gRPC server failed");
    });

    // ── Start REST API + SSE + Metrics ────────────────────────────────────────
    let limiter = rate_limit::RateLimiter::new(Default::default());
    rate_limit::spawn_purge_task(limiter.clone());

    let app = api::router(Arc::clone(&state), event_bus, limiter);
    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    tracing::info!("REST API listening on http://{}", config.listen_addr);

    // ── Graceful shutdown on SIGINT (Ctrl-C) or SIGTERM ───────────────────────
    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("Node shut down cleanly.");
    Ok(())
}

/// Returns a future that resolves when a shutdown signal is received.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c    => { tracing::info!("Received Ctrl-C — shutting down..."); }
        _ = terminate => { tracing::info!("Received SIGTERM — shutting down..."); }
    }
}
// explorer and rate_limit are declared here so they're available to api.rs
