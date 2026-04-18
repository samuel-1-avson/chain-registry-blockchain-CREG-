// crates/node/src/main.rs
// Chain registry node — single binary that runs all subsystems.
#![deny(clippy::unwrap_used)]

mod api;
mod block_producer;
mod bridge;
mod chain_store;
mod config;
mod consensus_admission;
mod db_sync_proxy;
mod events;
mod explorer;
mod finalized_tx;
mod gossip;
mod grpc;
mod metrics;
mod openapi;
mod p2p;
mod p2p_rate_limit;
mod pending_pool;
mod pidlock;
mod proof;
mod publisher_index;
mod rate_limit;
mod state;
mod sync;
mod validator_pipeline;

use alloy::{
    providers::{Provider, ProviderBuilder},
    sol,
};
use anyhow::Result;
use chrono::Utc;
use common::ValidatorIdentity;
use std::{collections::HashMap, sync::Arc};
use tokio::{
    sync::RwLock,
    time::{interval, sleep, Duration},
};
use tracing_subscriber::EnvFilter;

use events::{new_event_bus, EventBus};
use finalized_tx::{FinalizedTxReceiver, FinalizedTxSender};
use publisher_index::PublisherIndex;
use state::{
    BridgeStatus, NodeState, P2PStatus, SharedState, ValidatorRegistrationStatus,
    normalized_validator_key, validator_registration_status_text,
};

sol!(
    #[sol(rpc)]
    interface IStakingRead {
        function validators(address)
            external
            view
            returns (
                uint256 stake,
                uint8 state,
                uint256 unbondingAt,
                uint256 slashCount,
                uint256 ejectedAt,
                uint256 appliedAt
            );
    }
);

fn staking_state_label(state: u8) -> &'static str {
    match state {
        0 => "none",
        1 => "pending",
        2 => "active",
        3 => "unbonding",
        4 => "withdrawn",
        5 => "rejected",
        6 => "expired",
        _ => "unknown",
    }
}

fn upsert_registered_validator(
    validator_set: &mut common::ValidatorSet,
    registration: &ValidatorRegistrationStatus,
) {
    let identity = registration.identity.normalized();
    if !identity.is_complete() {
        return;
    }

    let alias = if registration.alias.trim().is_empty() {
        identity.node_id.clone()
    } else {
        registration.alias.trim().to_string()
    };

    if let Some(existing) = validator_set
        .validators
        .iter_mut()
        .find(|validator| {
            validator.id == identity.node_id || validator.pubkey == identity.ed25519_pubkey
        })
    {
        existing.id = identity.node_id;
        existing.alias = alias;
        existing.pubkey = identity.ed25519_pubkey;
        existing.stake = registration.stake;
        existing.reputation = registration.reputation.max(existing.reputation).max(100);
        if existing.status != "self" {
            existing.status = "online".to_string();
        }
        return;
    }

    validator_set.validators.push(common::Validator {
        id: identity.node_id,
        alias,
        pubkey: identity.ed25519_pubkey,
        stake: registration.stake,
        reputation: registration.reputation.max(100),
        status: "online".to_string(),
    });
}

fn remove_registered_validator(
    validator_set: &mut common::ValidatorSet,
    identity: &ValidatorIdentity,
) {
    let identity = identity.normalized();
    validator_set.validators.retain(|validator| {
        validator.id != identity.node_id && validator.pubkey != identity.ed25519_pubkey
    });
}

fn wei_to_creg_u64(value: alloy::primitives::U256) -> u64 {
    let whole_creg = value / alloy::primitives::U256::from(1_000_000_000_000_000_000u128);
    whole_creg.to_string().parse::<u64>().unwrap_or(u64::MAX)
}

async fn fetch_contract_code(
    client: &reqwest::Client,
    rpc_url: &str,
    address: &str,
) -> Result<String> {
    let response: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getCode",
            "params": [address, "latest"],
            "id": 1,
        }))
        .send()
        .await?
        .json()
        .await?;

    if let Some(error) = response.get("error") {
        anyhow::bail!("eth_getCode failed for {}: {}", address, error);
    }

    response
        .get("result")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .ok_or_else(|| anyhow::anyhow!("missing eth_getCode result for {}", address))
}

async fn validate_contract_addresses(config: &config::NodeConfig) -> Result<()> {
    let contracts = [
        ("CREG_REGISTRY_ADDR", config.registry_addr.as_str()),
        ("CREG_GOVERNANCE_ADDR", config.governance_addr.as_str()),
        ("CREG_TOKEN_ADDR", config.token_addr.as_str()),
        ("CREG_STAKING_ADDR", config.staking_addr.as_str()),
    ];

    let configured_contracts: Vec<_> = contracts
        .into_iter()
        .filter(|(_, address)| {
            let trimmed = address.trim();
            !trimmed.is_empty()
                && !trimmed.eq_ignore_ascii_case("0x0000000000000000000000000000000000000000")
        })
        .collect();

    if configured_contracts.is_empty() {
        return Ok(());
    }

    let client = reqwest::Client::new();
    for attempt in 1..=10 {
        let mut errors = Vec::new();
        for (name, address) in &configured_contracts {
            match fetch_contract_code(&client, &config.eth_rpc_url, address).await {
                Ok(code) if code != "0x" && code != "0x0" => {}
                Ok(_) => errors.push(format!("{}={} has no deployed bytecode", name, address)),
                Err(error) => errors.push(format!("{}={} validation failed: {}", name, address, error)),
            }
        }

        if errors.is_empty() {
            return Ok(());
        }

        if attempt == 10 {
            anyhow::bail!(
                "Configured contract address validation failed against {}: {}",
                config.eth_rpc_url,
                errors.join("; ")
            );
        }

        tracing::warn!(
            "Contract validation attempt {}/10 failed: {}. Retrying...",
            attempt,
            errors.join("; ")
        );
        sleep(Duration::from_secs(2)).await;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
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
        let hard_errors: Vec<_> = config_errors
            .iter()
            .filter(|e| e.contains("CREG_VALIDATOR_KEY"))
            .collect();
        if !hard_errors.is_empty() {
            anyhow::bail!("Cannot start validator node due to configuration errors. Fix the above and restart.");
        }
    }

    validate_contract_addresses(&config).await?;

    // ── Single-node enforcement (mainnet only) ────────────────────────────────
    // On mainnet, acquire a PID lock in the data directory to prevent multiple
    // nodes from running on the same machine. Testnet skips this entirely.
    let _pid_lock = if config.is_testnet {
        tracing::info!("  mode:        testnet (multi-node allowed)");
        None
    } else {
        tracing::info!("  mode:        mainnet (single node enforced)");
        Some(pidlock::PidLock::acquire(&config.data_dir)?)
    };

    tracing::info!("╔══════════════════════════════════════╗");
    tracing::info!(
        "║    chain-registry node v{}        ║",
        env!("CARGO_PKG_VERSION")
    );
    tracing::info!("╚══════════════════════════════════════╝");
    tracing::info!("  listen:      {}", config.listen_addr);
    tracing::info!("  data dir:    {}", config.data_dir.display());
    tracing::info!("  node id:     {}", config.node_id);
    tracing::info!("  validator:   {}", config.is_validator);
    tracing::info!("  peers:       {}", config.peers.len());

    // ── Open persistent storage ───────────────────────────────────────────────
    let chain = chain_store::ChainStore::open(&config.data_dir)?;
    let chain_for_sync = chain.clone();
    let tip = chain.tip_height()?;
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
        pending_pool: pending_pool::PendingPool::new(),
        publisher_index,
        validator_set: config.validator_set.clone(),
        votes: std::collections::HashMap::new(),
        config: config.clone(),
        event_bus: Arc::clone(&event_bus),
        p2p: p2p_handle.clone(),
        zk_validator: Arc::new(zk_validator::ZkValidator::default()),
        tx_sender: tx_sender.clone(),
        p2p_status: P2PStatus::default(),
        bridge_status: BridgeStatus {
            registry_address: config.registry_addr.clone(),
            ..BridgeStatus::default()
        },
        vrf_proofs: std::collections::HashMap::new(),
        decryption_shares: std::collections::HashMap::new(),
        validator_registrations: HashMap::new(),
        view_change_certs: HashMap::new(),
    }));

    // Start P2P node in background
    let p2p_handle_for_seeds = p2p_handle.clone();
    let seeds = config.p2p_seeds.clone();
    tokio::spawn(async move {
        for seed in seeds {
            if let Ok(addr) = seed.parse() {
                let _ = p2p_handle_for_seeds
                    .sender
                    .send(p2p::P2PCommand::Dial { addr })
                    .await;
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
            ..Default::default()
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

    tokio::spawn(sync::run(Arc::clone(&state)));

    // ── ML model existence check (T6) ─────────────────────────────────────────
    {
        let scanner = ml_validator::deep_scan::DeepScanner::default();
        if let Err(e) = scanner.validate_at_startup() {
            tracing::warn!("ML model validation: {}", e);
        }
    }

    tokio::spawn(validator_pipeline::run(Arc::clone(&state), tx_sender));

    tokio::spawn(block_producer::run(Arc::clone(&state), tx_receiver));

    tokio::spawn(bridge::run(Arc::clone(&state)));

    tokio::spawn(sync_validator_registrations(Arc::clone(&state)));

    let admission_store = consensus_admission::AttestationStore::new();
    tokio::spawn(consensus_admission::run(
        Arc::clone(&state),
        Arc::clone(&admission_store),
    ));

    // ── Start gRPC Server (Industrial Speed) ──────────────────────────────────
    let grpc_state = Arc::clone(&state);
    tokio::spawn(async move {
        let addr = "0.0.0.0:50051"
            .parse()
            .expect("gRPC bind address must be valid");
        let registry = grpc::MyRegistry::new(Arc::clone(&grpc_state));
        let watcher = grpc::MyWatcher::new(Arc::clone(&grpc_state));
        let explorer = grpc::MyExplorer::new(Arc::clone(&grpc_state));

        tracing::info!("gRPC API listening on {}", addr);

        tonic::transport::Server::builder()
            .add_service(
                common::proto::registry_service_server::RegistryServiceServer::new(registry),
            )
            .add_service(common::proto::watch_service_server::WatchServiceServer::new(watcher))
            .add_service(
                common::proto::explorer_service_server::ExplorerServiceServer::new(explorer),
            )
            .serve(addr)
            .await
            .expect("gRPC server failed");
    });

    // ── Start REST API + SSE + Metrics ────────────────────────────────────────
    let limiter = rate_limit::RateLimiter::new(Default::default());
    rate_limit::spawn_purge_task(limiter.clone());

    let app = api::router(
        Arc::clone(&state),
        event_bus,
        limiter,
        Arc::clone(&admission_store),
    );

    // ── Optional TLS termination ──────────────────────────────────────────────
    // Set CREG_TLS_CERT and CREG_TLS_KEY environment variables to enable HTTPS.
    #[cfg(feature = "tls")]
    {
        let tls_cert = std::env::var("CREG_TLS_CERT").ok();
        let tls_key = std::env::var("CREG_TLS_KEY").ok();

        if let (Some(cert_path), Some(key_path)) = (tls_cert, tls_key) {
            use axum_server::tls_rustls::RustlsConfig;

            let tls_config =
                RustlsConfig::from_pem_file(&cert_path, &key_path)
                    .await
                    .expect("Failed to load TLS certificate/key");

            let addr: std::net::SocketAddr = config.listen_addr.parse()
                .expect("listen_addr must be a valid socket address");

            tracing::info!("REST API listening on https://{}", addr);

            axum_server::bind_rustls(addr, tls_config)
                .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await?;

            tracing::info!("Node shut down cleanly.");
            return Ok(());
        }
    }

    // ── Plain HTTP (default) ──────────────────────────────────────────────────
    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    tracing::info!("REST API listening on http://{}", config.listen_addr);

    // ── Graceful shutdown on SIGINT (Ctrl-C) or SIGTERM ───────────────────────
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // Release the PID lock explicitly before logging clean shutdown.
    drop(_pid_lock);

    tracing::info!("Node shut down cleanly.");
    Ok(())
}

async fn sync_validator_registrations(state: SharedState) {
    let mut ticker = interval(Duration::from_secs(5));
    loop {
        ticker.tick().await;
        if let Err(error) = sync_validator_registrations_once(&state).await {
            tracing::warn!("validator registration sync failed: {}", error);
        }
    }
}

async fn sync_validator_registrations_once(state: &SharedState) -> Result<()> {
    let (rpc_url, staking_addr, registrations) = {
        let state_guard = state.read().await;
        (
            state_guard.config.eth_rpc_url.clone(),
            state_guard.config.staking_addr.clone(),
            state_guard
                .validator_registrations
                .iter()
                .map(|(key, registration)| (key.clone(), registration.clone()))
                .collect::<Vec<_>>(),
        )
    };

    if registrations.is_empty()
        || staking_addr.trim().is_empty()
        || staking_addr.eq_ignore_ascii_case("0x0000000000000000000000000000000000000000")
    {
        return Ok(());
    }

    let provider = ProviderBuilder::new().on_http(rpc_url.parse()?);
    let staking = IStakingRead::new(staking_addr.parse()?, &provider);
    let mut updates = Vec::with_capacity(registrations.len());

    for (key, registration) in registrations {
        let identity = registration.identity.normalized();
        let update = match identity.evm_address.parse() {
            Ok(address) => match staking.validators(address).call().await {
                Ok(result) => Ok((wei_to_creg_u64(result.stake), result.state)),
                Err(error) => Err(format!("staking lookup failed: {}", error)),
            },
            Err(error) => Err(format!("invalid EVM address: {}", error)),
        };
        updates.push((key, update));
    }

    let mut state_guard = state.write().await;
    for (key, update) in updates {
        let Some(mut registration) = state_guard.validator_registrations.remove(&key) else {
            continue;
        };

        registration.registered_with_node = true;
        registration.last_synced_at = Some(Utc::now().to_rfc3339());

        match update {
            Ok((stake, staking_state)) => {
                registration.last_error = None;
                registration.stake = stake;
                registration.applied_on_chain = staking_state != 0;
                registration.governance_approved = matches!(staking_state, 2 | 3 | 4);
                registration.staking_state = staking_state_label(staking_state).to_string();

                let should_admit = staking_state == 2 && registration.identity.normalized().is_complete();
                if should_admit {
                    upsert_registered_validator(&mut state_guard.validator_set, &registration);
                    registration.admitted_to_consensus = true;
                    registration.active = true;
                } else {
                    if registration.admitted_to_consensus {
                        remove_registered_validator(&mut state_guard.validator_set, &registration.identity);
                    }
                    registration.admitted_to_consensus = false;
                    registration.active = false;
                }
            }
            Err(error) => {
                registration.last_error = Some(error);
            }
        }

        registration.status = validator_registration_status_text(&registration);
        state_guard.validator_registrations.insert(key, registration);
    }

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
