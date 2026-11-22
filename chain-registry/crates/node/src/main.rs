// crates/node/src/main.rs
// Chain registry node â€” single binary that runs all subsystems.
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod admission_scan;
mod api;
mod authority_adapter;
mod authority_archive;
mod block_producer;
mod block_time;
mod bridge;
mod bridge_anchors;
mod chain_store;
mod config;
mod consensus_admission;
mod db_sync_proxy;
mod equivocation_monitor;
mod events;
mod explorer;
mod finalized_tx;
mod gossip;
mod governance_api;
mod grpc;
mod intelligence;
mod ipfs_fetch;
mod json_rpc;
mod l1_quorum;
mod metrics;
mod namespace_consensus;
mod openapi;
mod p2p;
mod p2p_peer_score;
mod p2p_rate_limit;
mod p2p_subnet_filter;
mod package_admission;
mod pbft_safety;
mod pending_pool;
mod pidlock;
mod proof;
mod publisher_index;
mod rate_limit;
mod revocation_gossip;
mod rpc_failover;
mod state;
mod sync;
mod validator_pipeline;
mod validator_registry_gossip;
mod validator_set_history;
// Phase-1 scaffold for chain-derived validator set. Compiled and tested but
// not yet wired into runtime â€” see docs/VALIDATOR_SET_SYNC_DESIGN.md.
mod chain_spec_boot;
mod validator_set_sync;

use alloy::{providers::Provider, sol};
use anyhow::{Context, Result};
use chrono::Utc;
use common::ValidatorIdentity;
use std::{collections::HashMap, sync::Arc};
use tokio::{
    sync::RwLock,
    time::{interval, sleep, Duration},
};
use tracing_subscriber::EnvFilter;

use events::new_event_bus;
use finalized_tx::{FinalizedTxReceiver, FinalizedTxSender};
use publisher_index::PublisherIndex;
use state::{
    normalized_validator_key, validator_registration_status_text, BridgeStatus, NodeState,
    P2PStatus, SharedState, ValidatorRegistrationStatus,
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

    if let Some(existing) = validator_set.validators.iter_mut().find(|validator| {
        validator.id == identity.node_id
            || validator.pubkey == identity.ed25519_pubkey
            || validator.eth_address == identity.evm_address
    }) {
        existing.id = identity.node_id;
        existing.alias = alias;
        existing.pubkey = identity.ed25519_pubkey;
        existing.eth_address = identity.evm_address;
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
        eth_address: identity.evm_address,
        stake: registration.stake,
        reputation: registration.reputation.max(100),
        status: "online".to_string(),
        performance_score: 500,
        contribution_count: 0,
    });
}

fn remove_registered_validator(
    validator_set: &mut common::ValidatorSet,
    identity: &ValidatorIdentity,
) {
    let identity = identity.normalized();
    validator_set.validators.retain(|validator| {
        validator.id != identity.node_id
            && validator.pubkey != identity.ed25519_pubkey
            && validator.eth_address != identity.evm_address
    });
}

fn wei_to_creg_u64(value: alloy::primitives::U256) -> u64 {
    let whole_creg = value / alloy::primitives::U256::from(1_000_000_000_000_000_000u128);
    whole_creg.to_string().parse::<u64>().unwrap_or(u64::MAX)
}

/// Verify the L1 RPC reports the chain ID we expect. Operators set
/// `CREG_EXPECTED_L1_CHAIN_ID` to e.g. `11155111` (Sepolia) or `1` (mainnet)
/// to guard against silently joining the wrong settlement chain â€” a very easy
/// misconfiguration when copying contract addresses between environments.
///
/// Skipped when the env var is unset, so local Anvil dev keeps working.
fn build_rpc_validation_client() -> Result<reqwest::Client> {
    // Bound external RPC waits so startup can retry or fail closed instead of
    // leaving a live process without an API listener indefinitely.
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to build bounded L1 RPC client")
}

async fn fetch_l1_chain_id(client: &reqwest::Client, rpc_url: &str) -> Result<u64> {
    let response: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_chainId",
            "params": [],
            "id": 1,
        }))
        .send()
        .await?
        .json()
        .await?;
    if response.get("error").is_some() {
        anyhow::bail!("eth_chainId returned a JSON-RPC error");
    }
    let raw = response
        .get("result")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("eth_chainId returned no result"))?;
    u64::from_str_radix(raw.trim_start_matches("0x"), 16)
        .map_err(|_| anyhow::anyhow!("eth_chainId returned a non-hex result"))
}

async fn fetch_l1_chain_id_with_retry(client: &reqwest::Client, rpc_url: &str) -> Result<u64> {
    for attempt in 1..=10 {
        match fetch_l1_chain_id(client, rpc_url).await {
            Ok(observed) => return Ok(observed),
            Err(_) if attempt < 10 => {
                tracing::warn!(
                    "L1 chain ID probe failed (attempt {}/10); retrying after bounded request timeout",
                    attempt
                );
                sleep(Duration::from_secs(2)).await;
            }
            Err(_) => {
                anyhow::bail!("L1 chain ID probe failed after ten bounded attempts")
            }
        }
    }
    anyhow::bail!("L1 chain ID probe did not complete")
}

async fn validate_l1_chain_id(rpc_url: &str, expected_from_spec: Option<u64>) -> Result<()> {
    let endpoints = crate::rpc_failover::configured_rpc_urls(rpc_url)?;
    let expected = if let Some(spec_id) = expected_from_spec {
        spec_id
    } else {
        match std::env::var("CREG_EXPECTED_L1_CHAIN_ID") {
            Ok(v) if !v.trim().is_empty() => v.trim().parse().map_err(|_| {
                anyhow::anyhow!(
                    "CREG_EXPECTED_L1_CHAIN_ID must be a positive integer (got {:?})",
                    v
                )
            })?,
            _ => return Ok(()),
        }
    };

    // Verify the endpoint set against the signed L1 chain ID. An unavailable
    // endpoint is tolerated only while a strict majority remains reachable; a
    // responding endpoint on a different chain always fails closed.
    let client = build_rpc_validation_client()?;
    let probes = futures::future::join_all(
        endpoints
            .iter()
            .map(|url| fetch_l1_chain_id_with_retry(&client, url)),
    )
    .await;
    let quorum = endpoints.len() / 2 + 1;
    let mut matching = 0usize;
    let mut offline = 0usize;
    for (index, result) in probes.into_iter().enumerate() {
        match result {
            Ok(observed) if observed == expected => matching += 1,
            Ok(observed) => {
                let host = reqwest::Url::parse(&endpoints[index])
                    .ok()
                    .and_then(|parsed| parsed.host_str().map(ToOwned::to_owned))
                    .unwrap_or_else(|| format!("endpoint-{}", index + 1));
                anyhow::bail!(
                    "L1 RPC endpoint {host} reports chain id {observed}, but the signed configuration requires {expected}"
                );
            }
            Err(_) => offline += 1,
        }
    }
    if matching < quorum {
        anyhow::bail!(
            "only {matching} of {} configured L1 RPC endpoints report the signed chain id {expected}; strict majority requires {quorum} ({offline} offline)",
            endpoints.len()
        );
    }
    tracing::info!(
        "  L1 chain id: {} verified by {}/{} configured RPC endpoints",
        expected,
        matching,
        endpoints.len()
    );
    Ok(())
}

/// Read deployed bytecode through the shared L1 transport, so the answer must
/// be agreed by a strict majority of the configured endpoints. One provider
/// can neither hide a deployed contract nor invent one.
async fn fetch_contract_code(rpc_url: &str, address: &str) -> Result<alloy::primitives::Bytes> {
    let parsed: alloy::primitives::Address = address
        .trim()
        .parse()
        .with_context(|| format!("{address} is not a valid contract address"))?;
    let provider = crate::rpc_failover::build_l1_provider(rpc_url)?;
    provider.get_code_at(parsed).await.map_err(|_| {
        anyhow::anyhow!(
            "eth_getCode for {address} failed or did not reach quorum on the configured L1 RPC endpoints"
        )
    })
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

    for attempt in 1..=10 {
        let mut errors = Vec::new();
        for (name, address) in &configured_contracts {
            match fetch_contract_code(&config.eth_rpc_url, address).await {
                Ok(code) if !code.is_empty() => {}
                Ok(_) => errors.push(format!("{}={} has no deployed bytecode", name, address)),
                Err(_) => errors.push(format!("{} bytecode request failed", name)),
            }
        }

        if errors.is_empty() {
            return Ok(());
        }

        if attempt == 10 {
            anyhow::bail!(
                "Configured contract bytecode validation failed after ten bounded attempts: {}",
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
    // Keep the service binary self-describing for release smoke tests and
    // operator tooling. These flags must return before loading RPC/spec state.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("creg-node {}", env!("CREG_EFFECTIVE_VERSION"));
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "Chain Registry node {}\n\nEnvironment-driven service binary.\n\n\
             Required for a signed testnet: CREG_CHAIN_SPEC_URL and a build-time\n\
             CREG_SPEC_SIGNING_PUBKEY matching the published chain spec.\n\n\
             Common endpoints: REST 0.0.0.0:8080, gRPC 0.0.0.0:50051,\n\
             P2P /ip4/0.0.0.0/tcp/9000.",
            env!("CREG_EFFECTIVE_VERSION")
        );
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(true)
        .init();

    let mut config = config::NodeConfig::load().await;
    if cfg!(feature = "phase-c-real-fleet") != common::compiled_consensus_encoding().is_some() {
        anyhow::bail!(
            "the Phase C node profile and common consensus codec must be enabled together"
        );
    }
    #[cfg(feature = "phase-c-real-fleet")]
    crate::block_producer::validate_commit_capture_arm(&config)?;
    {
        let scanner = ml_validator::deep_scan::DeepScanner::default();
        if let Err(error) = scanner.validate_at_startup() {
            if cfg!(feature = "phase-c-real-fleet") {
                return Err(error.into());
            }
            tracing::warn!("Scanner profile inspection: {}", error);
        }
    }

    // â”€â”€ Chain Spec Boot Validation â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    // Replaces the "read N envs and pray" model with a single fetched, signed,
    // validated chain spec. See docs/CHAIN_SPEC_DESIGN.md.

    let spec_url = std::env::var("CREG_CHAIN_SPEC_URL").ok();
    let spec_offline = std::env::var("CREG_CHAIN_SPEC_OFFLINE").unwrap_or_default() == "true";
    let pinned_genesis_hash = std::env::var("CREG_GENESIS_HASH").ok();
    let pinned_chain_id = std::env::var("CREG_CHAIN_ID").ok();
    let mut chain_spec_signature_verified = false;

    let chain_spec = match chain_spec_boot::resolve_chain_spec(
        spec_url.as_deref(),
        &config.data_dir,
        spec_offline,
    )
    .await
    {
        Ok(spec) => {
            tracing::info!(
                "Chain spec resolved: {} (version {})",
                spec.chain_id,
                spec.spec_version
            );
            Some(spec)
        }
        Err(e) if spec_url.is_none() => {
            tracing::warn!(
                "CREG_CHAIN_SPEC_URL not set; falling back to legacy env-based config: {}",
                e
            );
            None
        }
        Err(e) => {
            anyhow::bail!("Failed to resolve chain spec: {}", e);
        }
    };

    // If we have a spec, run the full 8-step validation.
    if let Some(spec) = &chain_spec {
        // Step 2: Verify signature (pinned pubkey from binary build)
        let pinned_pubkey = option_env!("CREG_SPEC_SIGNING_PUBKEY")
            .unwrap_or("0000000000000000000000000000000000000000000000000000000000000000");

        if pinned_pubkey.chars().all(|c| c == '0') {
            anyhow::bail!(
                "CREG_CHAIN_SPEC_URL is configured but this binary has no pinned CREG_SPEC_SIGNING_PUBKEY; refusing to start without signed-spec verification"
            );
        } else {
            if !spec
                .signing
                .signing_key_pubkey_hex
                .eq_ignore_ascii_case(pinned_pubkey)
            {
                anyhow::bail!(
                    "chain spec signing key {} does not match the binary-pinned CREG_SPEC_SIGNING_PUBKEY {}",
                    spec.signing.signing_key_pubkey_hex,
                    pinned_pubkey
                );
            }
            let sig_url = std::env::var("CREG_SPEC_SIGNATURE_URL")
                .unwrap_or_else(|_| spec.signing.detached_signature_url.clone());
            let signature_path =
                std::env::var_os("CREG_CHAIN_SPEC_SIG_PATH").map(std::path::PathBuf::from);
            let sig = chain_spec_boot::resolve_spec_signature(
                &sig_url,
                &config.data_dir,
                spec_offline,
                signature_path.as_deref(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to resolve spec signature: {}", e))?;
            spec.verify_signature(&sig, pinned_pubkey)
                .map_err(|e| anyhow::anyhow!("Spec signature invalid: {}", e))?;
            chain_spec_signature_verified = true;
            tracing::info!("Spec signature verified");
        }

        // Step 3: Schema validation (spec_version)
        if spec.spec_version != common::CURRENT_SPEC_VERSION {
            anyhow::bail!(
                "Unknown spec_version={}. This binary supports version {} only.",
                spec.spec_version,
                common::CURRENT_SPEC_VERSION
            );
        }

        spec.validate_consensus_identity()
            .context("signed chain-spec consensus identity is unsupported or inconsistent")?;

        // Step 4: Pinning checks
        if let Some(expected) = &pinned_chain_id {
            if &spec.chain_id != expected {
                anyhow::bail!(
                    "chain_id mismatch â€” CREG_CHAIN_ID={} but spec says {}. Refusing to start.",
                    expected,
                    spec.chain_id
                );
            }
        }

        let computed_genesis_hash = spec
            .compute_genesis_hash()
            .map_err(|e| anyhow::anyhow!("Failed to compute genesis hash: {}", e))?;
        if let Some(expected) = &pinned_genesis_hash {
            if &computed_genesis_hash != expected {
                anyhow::bail!(
                    "genesis_hash mismatch â€” CREG_GENESIS_HASH={} but spec computes {}. Refusing to start.",
                    expected,
                    computed_genesis_hash
                );
            }
        }
        tracing::info!("Computed genesis hash: {}", computed_genesis_hash);
        let spec_contract_errors = config.apply_chain_spec(spec, chain_spec_signature_verified);
        if !spec_contract_errors.is_empty() {
            for err in &spec_contract_errors {
                tracing::error!("  ✗ {}", err);
            }
            anyhow::bail!(
                "Cannot start node: CREG_*_ADDR env disagrees with the signed chain spec. \
                 Unset the stale overrides or set them to the spec addresses."
            );
        }
        tracing::info!("Chain spec applied to runtime config");

        // Step 5: L1 connectivity probe (already exists; now driven by spec)
        validate_l1_chain_id(&config.eth_rpc_url, Some(spec.l1.chain_id)).await?;

        // Step 6: Contract bytecode probe (warn-only on testnet)
        // We run this after applying the spec so config has the right addresses.
        // Step 8 is applied below after legacy validation.
    } else {
        // Legacy path: use env vars only
        validate_l1_chain_id(&config.eth_rpc_url, None).await?;
    }

    // ── Validate configuration early — fail fast with clear messages ──────────
    let config_errors = config.validate();
    if !config_errors.is_empty() {
        let (warnings, hard_errors) = partition_config_errors(&config_errors, config.is_validator);
        if !warnings.is_empty() {
            tracing::warn!("Configuration notices (non-fatal for optional L1 integrations):");
            for warn in &warnings {
                tracing::warn!("  ! {}", warn);
            }
        }
        if !hard_errors.is_empty() {
            tracing::error!("Fatal configuration errors:");
            for err in &hard_errors {
                tracing::error!("  x {}", err);
            }
            anyhow::bail!(
                "Cannot start node due to {} configuration error(s). Fix the above and restart.",
                hard_errors.len()
            );
        }
    }

    let production_security_errors = config.validate_production_security();
    if !production_security_errors.is_empty() {
        for err in &production_security_errors {
            tracing::error!("  âœ— {}", err);
        }
        anyhow::bail!(
            "Cannot start node: unsafe environment for production (CREG_TESTNET=false). \
             Unset CREG_DEV_SANDBOX and CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM, use CREG_SECRETS_BACKEND=vault for hot keys, \
             or set CREG_TESTNET=true for local clusters."
        );
    }

    let mal001_errors = config.validate_mal001_public_validator();
    if !mal001_errors.is_empty() {
        for err in &mal001_errors {
            tracing::error!("  ✗ {}", err);
        }
        anyhow::bail!(
            "Cannot start node: MAL-001 public-validator sandbox policy violated. \
             See docs/CREG_LIMITATIONS_PUBLIC_READINESS_PLAN.md (MAL-001)."
        );
    }
    if config.is_validator && config::creg_public_validator() {
        let sandbox = validator::sandbox::engine_status().await;
        if sandbox.dev_bypass {
            anyhow::bail!(
                "MAL-001: public validator detected sandbox dev bypass at runtime (engine={}). \
                 Rebuild with the secure fleet image or unset CREG_DEV_SANDBOX.",
                sandbox.engine
            );
        }
        if !sandbox.isolated {
            anyhow::bail!(
                "MAL-001: public validator requires an isolated sandbox engine (got '{}'). \
                 Install Firecracker/jailer (Tier 1) or hardened nsjail + pinned rootfs (Tier 2) before serving public traffic.",
                sandbox.engine
            );
        }
        tracing::info!(
            "  MAL-001: public validator sandbox engine={} isolated=true",
            sandbox.engine
        );
    }

    // ISSUE-005: load package-circuit keys before spawning tasks. Sepolia
    // keeps zk_validation false, so this is a no-op there. Production never
    // mints ephemeral keys.
    if config.zk_validation {
        anyhow::bail!("zk_validation is retired (ADR-018 / VNA-22); unset CREG_ZK_VALIDATION");
    }

    // The signed-spec path already verified its canonical genesis hash above.
    // Keep the legacy identity check only for env-only compatibility mode.
    if chain_spec.is_none() {
        match config.validate_genesis_hash() {
            Ok(hash) => tracing::info!("  genesis hash: 0x{}", hash),
            Err(msg) => anyhow::bail!(msg),
        }
    }

    validate_contract_addresses(&config).await?;

    // â”€â”€ Single-node enforcement (mainnet only) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    // On mainnet, acquire a PID lock in the data directory to prevent multiple
    // nodes from running on the same machine. Testnet skips this entirely.
    let _pid_lock = if config.is_testnet {
        tracing::info!("  mode:        testnet (multi-node allowed)");
        None
    } else {
        tracing::info!("  mode:        mainnet (single node enforced)");
        Some(pidlock::PidLock::acquire(&config.data_dir)?)
    };

    tracing::info!("â•”â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•—");
    tracing::info!(
        "â•‘    chain-registry node v{}        â•‘",
        env!("CREG_EFFECTIVE_VERSION")
    );
    tracing::info!("â•šâ•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•");
    tracing::info!("  listen:      {}", config.listen_addr);
    tracing::info!("  data dir:    {}", config.data_dir.display());
    tracing::info!("  node id:     {}", config.node_id);
    tracing::info!("  validator:   {}", config.is_validator);
    tracing::info!("  peers:       {}", config.peers.len());

    // â”€â”€ Open persistent storage â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let chain = if cfg!(feature = "phase-c-real-fleet") {
        if chain_spec.is_none() {
            anyhow::bail!("Phase C consensus requires a verified signed chain spec");
        }
        chain_store::ChainStore::open_phase_c(&config.data_dir, &config.network_genesis_hash)?
    } else {
        chain_store::ChainStore::open(&config.data_dir)?
    };
    if config.is_validator {
        let genesis = chain
            .get_block_by_height(0)?
            .context("validator startup cannot load the canonical genesis block")?;
        let private_key_hex = config
            .validator_privkey
            .as_deref()
            .context("validator startup has no PBFT signing key")?;
        let private_key_bytes = hex::decode(private_key_hex.trim_start_matches("0x"))
            .context("validator PBFT signing key is not valid hex")?;
        let signing_key = common::crypto::SigningKey::from_seed(
            common::crypto::SignatureScheme::Ed25519,
            &private_key_bytes,
        )
        .context("validator PBFT signing key must contain 32 bytes")?;
        let validator_pubkey = signing_key.public_key().to_hex();
        if cfg!(feature = "phase-c-real-fleet")
            && !config.validator_set.validators.iter().any(|validator| {
                validator.id == config.node_id
                    && validator.pubkey == validator_pubkey
                    && common::validator_status_is_active(&validator.status)
            })
        {
            anyhow::bail!(
                "local Phase C signing identity is outside the signed active bootstrap set"
            );
        }
        chain.initialize_pbft_safety_journal(
            &genesis.hash(),
            &config.node_id,
            &validator_pubkey,
        )?;
    }
    chain.configure_namespace_consensus(
        namespace_consensus::NamespaceConsensusConfig::from_node_config(&config),
    )?;
    if config.namespace_authority_activation_height.is_some() {
        // Fail closed before serving or joining consensus if canonical history
        // cannot deterministically rebuild the authoritative namespace state.
        chain.rebuild_package_index()?;
    }
    let chain_for_sync = chain.clone();
    let tip = chain.tip_height()?;
    tracing::info!("  chain tip:   height={}", tip);
    let mut pbft_engine = crate::state::PbftEngine::new();
    if cfg!(feature = "phase-c-real-fleet") && pbft_engine.allow_small_cluster_quorum() {
        anyhow::bail!("Phase C cannot enable a reduced small-cluster quorum");
    }
    if config.is_validator {
        let tip_hash = chain.tip_hash()?;
        let journal = chain.pbft_safety_journal()?;
        for (block, certificate) in journal.recovered_prepared()? {
            let height = block.header.height;
            let authority = validator_set_history::verification_set_at(
                &config.data_dir,
                height,
                &config.validator_set,
            )
            .map_err(anyhow::Error::msg)?;
            let pbft_authority: consensus::ValidatorSet = authority.into();
            let next_view = certificate
                .prepared_view
                .checked_add(1)
                .context("recovered prepared view has no successor")?;
            let vote_context = common::ViewChangeVote {
                height,
                prev_hash: block.header.prev_hash.clone(),
                new_view: next_view,
                validator_id: config.node_id.clone(),
                prepared: None,
                signature: String::new(),
            };
            consensus::pbft::verify_prepared_certificate(
                &vote_context,
                &certificate,
                &pbft_authority,
                pbft_engine.allow_small_cluster_quorum(),
            )?;
            if height <= tip {
                let canonical = chain
                    .get_block_by_height(height)?
                    .context("recovered prepared height is absent from the canonical chain")?;
                if canonical.hash() != block.hash() {
                    anyhow::bail!(
                        "recovered prepared value conflicts with finalized chain history"
                    );
                }
            } else if height == tip.saturating_add(1) && block.header.prev_hash == tip_hash {
                pbft_engine.restore_prepared(block, certificate, pbft_authority)?;
                tracing::warn!(height, "Recovered durable PBFT prepared round");
            } else {
                anyhow::bail!("recovered prepared value cannot be attached to the canonical tip");
            }
        }
        for (height, prev_hash, block_hash, signature) in journal.recovered_commits()? {
            if height <= tip {
                let canonical = chain
                    .get_block_by_height(height)?
                    .context("recovered COMMIT height is absent from canonical history")?;
                if canonical.hash() != block_hash || canonical.header.prev_hash != prev_hash {
                    anyhow::bail!("recovered COMMIT conflicts with finalized chain history");
                }
                continue;
            }
            if height != tip.saturating_add(1) || prev_hash != tip_hash {
                anyhow::bail!("recovered COMMIT cannot be attached to the canonical tip");
            }
            let vote = common::BlockSignature {
                validator_id: config.node_id.clone(),
                pubkey: journal.validator_pubkey().to_owned(),
                signature,
            };
            pbft_engine
                .commit(&block_hash, &config.node_id, vote)
                .context("recovered COMMIT has no matching durable prepared round")?;
            tracing::warn!(
                event = "pbft_local_commit_recovered",
                validator_id = %config.node_id,
                chain_id = %config.chain_id,
                genesis_hash = %config.network_genesis_hash,
                height,
                parent_hash = %prev_hash,
                block_hash = %block_hash,
                "Recovered durable local PBFT COMMIT before P2P and sync services start"
            );
        }
        for vote in journal.recovered_view_change_votes()? {
            let authority = validator_set_history::verification_set_at(
                &config.data_dir,
                vote.height,
                &config.validator_set,
            )
            .map_err(anyhow::Error::msg)?;
            let pbft_authority: consensus::ValidatorSet = authority.into();
            consensus::pbft::verify_view_change_vote(
                &vote,
                &pbft_authority,
                pbft_engine.allow_small_cluster_quorum(),
            )?;
            if vote.height <= tip {
                continue;
            }
            if vote.height != tip.saturating_add(1) || vote.prev_hash != tip_hash {
                anyhow::bail!("recovered view-change vote cannot be attached to the canonical tip");
            }
            pbft_engine.receive_view_change(vote, &pbft_authority)?;
        }
    }

    // â”€â”€ Rebuild publisher index from chain history â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let publisher_index = chain.load_or_sync_publisher_index()?;
    tracing::info!("  publishers:  {}", publisher_index.publisher_count());
    let mut forced_inclusion_tracker = crate::state::ForcedInclusionTracker::new();
    for (transaction, submitted_at_height) in chain.finalized_work_for_tracker(tip)? {
        forced_inclusion_tracker.submit(transaction, submitted_at_height);
    }
    if forced_inclusion_tracker.pending_count() > 0 {
        tracing::info!(
            "  forced work: {} durable transaction(s) restored",
            forced_inclusion_tracker.pending_count()
        );
    }

    // â”€â”€ Event bus (broadcast channel for SSE clients) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let event_bus = new_event_bus();
    chain.attach_event_bus(Arc::clone(&event_bus))?;

    let (dialable_seeds, invalid_seeds) = p2p::parse_p2p_seed_multiaddrs(&config.p2p_seeds);
    for (seed, error) in &invalid_seeds {
        tracing::warn!(seed = %seed, error = %error, "Ignoring invalid P2P seed multiaddr");
    }
    if config.is_validator
        && dialable_seeds.is_empty()
        && !config::allow_isolated_empty_p2p_seeds(config.is_testnet)
    {
        anyhow::bail!(
            "CREG_IS_VALIDATOR=true requires at least one dialable P2P seed \
             (CREG_P2P_SEEDS or CREG_VALIDATOR_P2P_SEEDS). Isolated lab nodes may set \
             CREG_TESTNET=true and CREG_ALLOW_EMPTY_P2P_SEEDS=true."
        );
    }

    let p2p_identity_path = config.data_dir.join("p2p").join("identity.key");
    let (p2p_node, p2p_handle) =
        p2p::P2PNode::with_identity(&config.p2p_listen, Some(&p2p_identity_path))?;
    tracing::info!(
        peer_id = %p2p_node.peer_id,
        identity = %p2p_identity_path.display(),
        "P2P identity ready"
    );

    // Make the p2p sender available to API/worker code that needs to gossip
    // validator identity registrations fleet-wide.
    validator_registry_gossip::set_gossip_sender(p2p_handle.sender.clone());

    // â”€â”€ Finalized-tx channel (created before state so API can send to it) â”€â”€â”€â”€â”€â”€â”€
    let (tx_sender, tx_receiver): (FinalizedTxSender, FinalizedTxReceiver) =
        finalized_tx::channel();
    // â”€â”€ Shared state â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let state: SharedState = Arc::new(RwLock::new(NodeState {
        chain,
        pending_pool: pending_pool::PendingPool::open(&config.data_dir),
        publisher_index,
        validator_set_bootstrap: config.validator_set.clone(),
        validator_set_snapshot: Arc::new(crate::state::ArcSwap::from_pointee(
            config.validator_set.clone(),
        )),
        validator_set: config.validator_set.clone(),
        package_rounds: std::collections::HashMap::new(),
        package_committees: std::collections::HashMap::new(),
        config: config.clone(),
        p2p_status: P2PStatus::default(),
        bridge_status: BridgeStatus {
            registry_address: config.registry_addr.clone(),
            ..BridgeStatus::default()
        },
        vrf_proofs: std::collections::HashMap::new(),
        decryption_shares: std::collections::HashMap::new(),
        validator_registrations: HashMap::new(),
        validator_set_sync: crate::state::ValidatorSetSyncStatus {
            enabled: false,
            mode: "static".to_string(),
            state: "disabled".to_string(),
            ..Default::default()
        },
        view_change_certs: HashMap::new(),
        reorgs: Vec::new(),
        pbft_engine,
        forced_inclusion_tracker,
        sync_lag_blocks: 0,
        sync_max_peer_tip: 0,
        reward_emissions_succeeded: 0,
        reward_emissions_failed: 0,
        reward_emissions_skipped: 0,
        treasury_balance: 0,
        pending_rewards_estimate: 0,
        total_rewards_distributed: 0,
        rewards_pool_balance: 0,
        last_incentives_refresh: None,
        publisher_stake_cache: HashMap::new(),
        equivocation: crate::equivocation_monitor::EquivocationMonitor::from_env(&config.data_dir),
    }));

    // Seed the validator-set history with the bootstrap set effective from
    // genesis, so blocks at early heights verify against the right set
    // (ISSUE-050). Subsequent changes are recorded by the reconcile path.
    if let Err(e) = validator_set_history::record(&config.data_dir, 0, &config.validator_set) {
        tracing::warn!("Failed to seed validator-set history: {}", e);
    }

    // Start P2P node in background
    let p2p_handle_for_seeds = p2p_handle.clone();
    tokio::spawn(async move {
        if dialable_seeds.is_empty() {
            return;
        }

        let mut seed_redial = interval(Duration::from_secs(10));
        loop {
            seed_redial.tick().await;

            for addr in &dialable_seeds {
                let _ = p2p_handle_for_seeds
                    .sender
                    .send(p2p::P2PCommand::Dial { addr: addr.clone() })
                    .await;
            }
        }
    });

    tokio::spawn(p2p_node.run(Arc::clone(&state), Arc::clone(&event_bus)));
    tokio::spawn(revocation_gossip::run(
        Arc::clone(&state),
        p2p_handle.clone(),
    ));

    // â”€â”€ Validator identity registrations: restore + periodic re-broadcast â”€â”€â”€â”€â”€â”€
    // Reapply persisted registrations so a restart keeps every known validator
    // identity, then periodically re-gossip them so late-joining / restarted
    // peers converge without operator re-POSTing to each node.
    {
        let state_for_registry = Arc::clone(&state);
        let data_dir = config.data_dir.clone();
        tokio::spawn(async move {
            for proof in validator_registry_gossip::load_all(&data_dir) {
                match api::apply_validator_registration(&state_for_registry, &proof).await {
                    Ok(_) => tracing::info!(
                        "Restored persisted validator registration for {}",
                        proof.node_id
                    ),
                    Err((_, e)) => tracing::warn!(
                        "Could not restore validator registration for {}: {}",
                        proof.node_id,
                        e
                    ),
                }
            }

            let mut ticker = interval(Duration::from_secs(120));
            ticker.tick().await; // consume immediate first tick
            loop {
                ticker.tick().await;
                for proof in validator_registry_gossip::load_all(&data_dir) {
                    validator_registry_gossip::broadcast(proof).await;
                }
            }
        });
    }

    // â”€â”€ Spawn background tasks â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    tracing::info!("Starting subsystems...");

    // PostgreSQL sync worker (chain store â†’ PostgreSQL ETL)
    // Default to the dedicated `creg-indexer` service so validators do not
    // own both the consensus path and the explorer mirror path in the same
    // process. Set CREG_PG_SYNC_IN_PROCESS=true to restore the legacy mode.
    if !config.pg_url.is_empty() {
        if std::env::var("CREG_PG_SYNC_IN_PROCESS").ok().as_deref() == Some("true") {
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
                    tracing::info!("Legacy in-process PostgreSQL sync worker started");
                }
                Err(e) => {
                    tracing::warn!("Failed to start PostgreSQL sync worker: {}", e);
                }
            }
        } else {
            tracing::info!(
                "Skipping in-process PostgreSQL sync worker; run the dedicated creg-indexer service or set CREG_PG_SYNC_IN_PROCESS=true for legacy mode"
            );
        }
    }

    tokio::spawn(sync::run(Arc::clone(&state)));

    // â”€â”€ ML model existence check (T6) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    tokio::spawn(validator_pipeline::run(
        Arc::clone(&state),
        tx_sender.clone(),
        p2p_handle.clone(),
    ));

    tokio::spawn(block_producer::run(
        Arc::clone(&state),
        tx_receiver,
        p2p_handle.clone(),
    ));

    tokio::spawn(bridge::run(Arc::clone(&state)));

    tokio::spawn(sync_validator_registrations(Arc::clone(&state)));

    let admission_store = consensus_admission::AttestationStore::new();
    tokio::spawn(consensus_admission::run(
        Arc::clone(&state),
        Arc::clone(&admission_store),
    ));

    // â”€â”€ Validator set sync (chain-authoritative with static bootstrap) â”€â”€â”€â”€â”€â”€
    {
        let start_block = chain_spec
            .as_ref()
            .map(|spec| spec.validator_set.epoch_block_height);
        // Apply L1 staking events only after a finality buffer so shallow
        // Sepolia reorgs cannot flap the validator set. Override with
        // CREG_VALIDATOR_SET_FINALITY_LAG (in L1 blocks).
        let default_finality_lag: u64 = if config.is_testnet { 2 } else { 6 };
        let finality_lag_blocks = std::env::var("CREG_VALIDATOR_SET_FINALITY_LAG")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(default_finality_lag);
        if finality_lag_blocks == 0 {
            tracing::warn!(
                "CREG_VALIDATOR_SET_FINALITY_LAG=0 — L1 staking events are applied with no \
                 finality buffer; a shallow L1 reorg can flap validator membership."
            );
        }
        // Additional independent L1 RPC endpoints for quorum reads, so one
        // stale/compromised RPC cannot skew the validator set. Comma-separated.
        let extra_rpc_urls: Vec<String> =
            crate::rpc_failover::configured_rpc_urls(&config.eth_rpc_url)?
                .into_iter()
                .skip(1)
                .collect();
        let staking_addr = match config::parse_staking_contract_address(&config.staking_addr) {
            Ok(addr) => addr,
            Err(error) => anyhow::bail!("{error}"),
        };
        let sync_config = staking_addr.map(|staking_addr| validator_set_sync::SyncConfig {
            eth_rpc_url: config.eth_rpc_url.clone(),
            extra_rpc_urls,
            staking_addr,
            finality_lag_blocks,
            poll_interval_secs: 30,
            start_block,
        });
        {
            let mut state_guard = state.write().await;
            let enabled = sync_config.is_some();
            state_guard.validator_set_sync.enabled = enabled;
            state_guard.validator_set_sync.mode = if enabled {
                "chain-authoritative".to_string()
            } else {
                "static".to_string()
            };
            state_guard.validator_set_sync.state = if enabled {
                "starting".to_string()
            } else {
                "disabled".to_string()
            };
            if !enabled {
                state_guard.validator_set_sync.last_error = Some(
                    "validator-set sync disabled: CREG_STAKING_ADDR is unset or zero".to_string(),
                );
                tracing::warn!(
                    "Validator set sync is disabled (static spec set). Observer/dev nodes may \
                     start this way; CREG_IS_VALIDATOR=true refuses a missing or invalid staking address."
                );
            }
        }
        if let Some(sync_config) = sync_config {
            tracing::info!(
                start_block = ?sync_config.start_block,
                finality_lag_blocks = sync_config.finality_lag_blocks,
                poll_interval_secs = sync_config.poll_interval_secs,
                "Validator set source: chain-authoritative"
            );
            let sync_state = Arc::clone(&state);
            tokio::spawn(async move {
                loop {
                    if let Err(error) = validator_set_sync::run(
                        sync_config.clone(),
                        validator_set_sync::SyncMode::ChainAuthoritative,
                        Arc::clone(&sync_state),
                    )
                    .await
                    {
                        {
                            let mut state_guard = sync_state.write().await;
                            state_guard.validator_set_sync.state = "degraded".to_string();
                            state_guard.validator_set_sync.last_error = Some(
                                "validator-set sync worker stopped; retry scheduled".to_string(),
                            );
                        }
                        tracing::error!(
                            target: "validator_set_sync",
                            error = %error,
                            "validator-set sync worker stopped; retrying in 30 seconds"
                        );
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                }
            });
        }
    }

    // â”€â”€ Incentives status refresher (background cached L1 queries for treasury monitoring) â”€â”€
    {
        let inc_state = Arc::clone(&state);
        let inc_cfg = config.clone();
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(60));
            loop {
                ticker.tick().await;
                crate::api::refresh_incentives_status(Arc::clone(&inc_state), &inc_cfg).await;
            }
        });
        tracing::info!("Incentives status refresher started (60s interval)");
    }

    // ── Start gRPC Server (disabled by default; loopback 127.0.0.1:50051 when enabled; VNA-04) ──
    if config.grpc_enabled {
        let grpc_addr: std::net::SocketAddr = config
            .grpc_listen_addr
            .parse()
            .with_context(|| format!("invalid CREG_GRPC_LISTEN: {}", config.grpc_listen_addr))?;
        let grpc_state = Arc::clone(&state);
        let watcher_bus = Arc::clone(&event_bus);
        tokio::spawn(async move {
            let registry = grpc::MyRegistry::new(Arc::clone(&grpc_state));
            let watcher = grpc::MyWatcher::new(watcher_bus);
            let explorer = grpc::MyExplorer::new(Arc::clone(&grpc_state));

            tracing::info!("gRPC API listening on {}", grpc_addr);

            if let Err(err) = tonic::transport::Server::builder()
                .concurrency_limit_per_connection(32)
                .tcp_nodelay(true)
                .add_service(
                    common::proto::registry_service_server::RegistryServiceServer::new(registry),
                )
                .add_service(common::proto::watch_service_server::WatchServiceServer::new(watcher))
                .add_service(
                    common::proto::explorer_service_server::ExplorerServiceServer::new(explorer),
                )
                .serve(grpc_addr)
                .await
            {
                tracing::error!("gRPC server exited with error: {err}");
            }
        });
    } else {
        tracing::info!("gRPC server disabled (set CREG_GRPC_ENABLED=true to enable)");
    }

    // ── Start REST API + SSE + Metrics ────────────────────────────────────────
    let (health_tx, health_rx) = tokio::sync::watch::channel(crate::state::NodeHealthSnapshot {
        node_id: config.node_id.clone(),
        is_validator: config.is_validator,
        ..Default::default()
    });
    {
        let health_state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(2));
            loop {
                ticker.tick().await;
                let (
                    node_id,
                    is_validator,
                    peers,
                    validator_set_sync,
                    sync_lag_blocks,
                    sync_max_peer_tip,
                ) = {
                    let s = health_state.read().await;
                    (
                        s.config.node_id.clone(),
                        s.config.is_validator,
                        s.config.peers.clone(),
                        s.validator_set_sync.clone(),
                        s.sync_lag_blocks,
                        s.sync_max_peer_tip,
                    )
                };
                let mut sandbox = validator::sandbox::engine_status().await;
                if !is_validator && sandbox.engine == "none" {
                    if let Some(peer_sandbox) =
                        validator::sandbox::fleet_sandbox_from_peers(&peers).await
                    {
                        sandbox = peer_sandbox;
                    }
                }
                let _ = health_tx.send(crate::state::NodeHealthSnapshot {
                    node_id,
                    is_validator,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    validator_set_sync,
                    sync_lag_blocks,
                    sync_max_peer_tip,
                    sandbox,
                });
            }
        });
    }

    let limiter = rate_limit::RateLimiter::new(Default::default());
    rate_limit::spawn_purge_task(limiter.clone());

    if let Some(ref admin_addr) = config.admin_listen_addr {
        let admin_app = api::admin_router(
            Arc::clone(&state),
            Arc::clone(&event_bus),
            limiter.clone(),
            Arc::clone(&admission_store),
            config.cors.clone(),
            tx_sender.clone(),
            p2p_handle.clone(),
        )
        .layer(axum::extract::Extension(health_rx.clone()));
        let admin_listener = tokio::net::TcpListener::bind(admin_addr).await?;
        let admin_addr_log = admin_addr.clone();
        tracing::info!("Admin / Metrics API listening on http://{}", admin_addr_log);
        tokio::spawn(async move {
            if let Err(err) = axum::serve(
                admin_listener,
                admin_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown_signal())
            .await
            {
                tracing::error!("Admin listener {admin_addr_log} exited with error: {err}");
            }
        });
    }

    let app = api::router(
        Arc::clone(&state),
        event_bus,
        limiter,
        Arc::clone(&admission_store),
        config.cors.clone(),
        tx_sender.clone(),
        p2p_handle,
    )
    .layer(axum::extract::Extension(health_rx.clone()));

    // â”€â”€ Optional TLS termination â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    // Set CREG_TLS_CERT and CREG_TLS_KEY environment variables to enable HTTPS.
    #[cfg(feature = "tls")]
    {
        let tls_cert = std::env::var("CREG_TLS_CERT").ok();
        let tls_key = std::env::var("CREG_TLS_KEY").ok();

        if let (Some(cert_path), Some(key_path)) = (tls_cert, tls_key) {
            use axum_server::tls_rustls::RustlsConfig;

            let tls_config = RustlsConfig::from_pem_file(&cert_path, &key_path)
                .await
                .expect("Failed to load TLS certificate/key");

            let addr: std::net::SocketAddr = config
                .listen_addr
                .parse()
                .expect("listen_addr must be a valid socket address");

            tracing::info!("REST API listening on https://{}", addr);

            axum_server::bind_rustls(addr, tls_config)
                .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await?;

            tracing::info!("Node shut down cleanly.");
            return Ok(());
        }
    }

    // â”€â”€ Plain HTTP (default) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    let listener = tokio::net::TcpListener::bind(&config.listen_addr).await?;
    tracing::info!("REST API listening on http://{}", config.listen_addr);

    // â”€â”€ Graceful shutdown on SIGINT (Ctrl-C) or SIGTERM â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
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

    let provider = crate::rpc_failover::build_l1_provider(&rpc_url)?;
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

                let identity = registration.identity.normalized();
                let should_admit = staking_state == 2
                    && identity.is_complete()
                    && state_guard
                        .validator_set
                        .validators
                        .iter()
                        .any(|validator| {
                            validator
                                .eth_address
                                .eq_ignore_ascii_case(&identity.evm_address)
                        });
                registration.admitted_to_consensus = should_admit;
                registration.active = should_admit;
            }
            Err(error) => {
                registration.last_error = Some(error);
            }
        }

        registration.status = validator_registration_status_text(&registration);
        state_guard
            .validator_registrations
            .insert(key, registration);
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
        _ = ctrl_c    => { tracing::info!("Received Ctrl-C â€” shutting down..."); }
        _ = terminate => { tracing::info!("Received SIGTERM â€” shutting down..."); }
    }
}
fn partition_config_errors(errors: &[String], is_validator: bool) -> (Vec<String>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut hard_errors = Vec::new();
    for err in errors {
        let is_optional_l1_notice = err.starts_with("CREG_REGISTRY_ADDR is the zero address")
            || err.starts_with("CREG_GOVERNANCE_ADDR is the zero address")
            || err.starts_with("CREG_TOKEN_ADDR is not set")
            || (!is_validator && err.starts_with("CREG_STAKING_ADDR is not set"));
        if is_optional_l1_notice {
            warnings.push(err.clone());
        } else {
            hard_errors.push(err.clone());
        }
    }
    (warnings, hard_errors)
}

#[cfg(test)]
mod config_partition_tests {
    use super::partition_config_errors;

    #[test]
    fn cors_interval_ipfs_and_secrets_errors_are_always_fatal() {
        let sample = vec![
            "CREG_TOKEN_ADDR is not set. Wallet balances...".to_string(),
            "CREG_CORS_ALLOW_CREDENTIALS=true cannot be used with wildcard".to_string(),
            "CREG_BLOCK_INTERVAL must be > 0 seconds".to_string(),
            "CREG_IPFS_URL is empty.".to_string(),
            "CREG_VALIDATOR_KEY must not be set when CREG_SECRETS_BACKEND=vault".to_string(),
        ];
        let (warnings, hard) = partition_config_errors(&sample, false);
        assert_eq!(warnings.len(), 1);
        assert_eq!(hard.len(), 4);
    }
}
// explorer and rate_limit are declared here so they're available to api.rs
