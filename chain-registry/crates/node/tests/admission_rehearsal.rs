//! End-to-end Anvil + 4-validator + fresh-applicant automatic admission rehearsal.
//!
//! Exercises:
//!   1. Real Anvil EVM chain + real compiled CregToken & Staking contracts.
//!   2. 4 active bootstrap validators + 1 fresh observer candidate (`admission_auto_activate=true`).
//!   3. Dual-signed registration gossiped across loopback libp2p mesh.
//!   4. Gas-only-on-submitter constraint (only v1 holds ETH; v2..v4 have 0 ETH).
//!   5. 2-of-4 insufficiency + missing signer setup telemetry.
//!   6. Interrupted run safe resume from durable disk journal without duplicate inflation.
//!   7. 3-of-4 automatic quorum -> Sepolia `approveByConsensus` -> certified R7 transition at block #2 ->
//!      observer auto-activation -> 5-validator quorum (4-of-5) finalizes block #2 with candidate's vote.
//!   8. Separate timing measurements for all four stages + live `/v1/public/validator/admission-status`.

use alloy::{
    network::{EthereumWallet, TransactionBuilder},
    primitives::{keccak256, Address, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::{local::PrivateKeySigner, Signer as _},
    sol,
    sol_types::SolValue,
};
use chrono::Utc;
use common::{
    validator_identity_registration_message, AnalysisBundleRefs, ChainRecord,
    DeterministicRiskSummary, PackageId, PackageStatus, Transaction, Validator, ValidatorIdentity,
    ValidatorSet, ValidatorSignature, ValidatorVote,
};
use consensus::pbft::verify_pbft_phase_signature;
use ed25519_dalek::{Signer as _, SigningKey};
use node::{
    api, block_producer,
    chain_store::ChainStore,
    config::{CorsConfig, NodeConfig},
    consensus_admission::{self, AttestationStore},
    equivocation_monitor::EquivocationMonitor,
    events::new_event_bus,
    finalized_tx,
    p2p::{P2PCommand, P2PHandle, P2PNode},
    pending_pool::PendingPool,
    publisher_index::PublisherIndex,
    rate_limit::{RateLimitConfig, RateLimiter},
    state::{ArcSwap, ForcedInclusionTracker, PbftEngine, ValidatorSetSyncStatus},
    validator_set_history, BridgeStatus, NodeState, P2PStatus, SharedState,
};
use std::{
    collections::HashMap,
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio::{sync::RwLock, task::JoinHandle};

sol!(
    #[sol(rpc)]
    interface IRehearsalToken {
        function transfer(address to, uint256 value) external returns (bool);
        function approve(address spender, uint256 value) external returns (bool);
        function balanceOf(address owner) external view returns (uint256);
    }

    #[sol(rpc)]
    interface IRehearsalStaking {
        function applyToBeValidator(uint256 amount) external;
        function approveValidator(address validator) external;
        function activeValidatorCount() external view returns (uint256);
        function validatorSetEpoch() external view returns (uint256);
        function validators(address) external view returns (
            uint256 stake,
            uint8 state,
            uint256 unbondingAt,
            uint256 slashCount,
            uint256 ejectedAt,
            uint256 appliedAt
        );
    }
);

struct AnvilGuard {
    child: Child,
    rpc_url: String,
}

impl Drop for AnvilGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_loopback_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    l.local_addr().unwrap().port()
}

fn find_anvil_binary() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let exe_name = if cfg!(windows) { "anvil.exe" } else { "anvil" };
    let bundled = manifest_dir
        .join("..")
        .join("..")
        .join("testnet")
        .join(".tools")
        .join("foundry")
        .join(exe_name);
    if bundled.is_file() {
        return bundled;
    }
    PathBuf::from("anvil")
}

async fn spawn_anvil() -> AnvilGuard {
    let bin = find_anvil_binary();
    let port = free_loopback_port();
    let child = Command::new(&bin)
        .args(["--port", &port.to_string(), "--chain-id", "31337", "--silent"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", bin.display()));
    let rpc_url = format!("http://127.0.0.1:{port}");
    let guard = AnvilGuard { child, rpc_url };

    let client = reqwest::Client::new();
    for _ in 0..50 {
        if let Ok(resp) = client
            .post(&guard.rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_chainId",
                "params": [],
                "id": 1
            }))
            .send()
            .await
        {
            if resp.status().is_success() {
                return guard;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("anvil did not respond on {}", guard.rpc_url);
}

fn load_contract_bytecode(artifact_rel_path: &str) -> Vec<u8> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir
        .join("..")
        .join("..")
        .join("contracts")
        .join("out")
        .join(artifact_rel_path);
    let raw = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("missing Foundry artifact {}: {e}", path.display()));
    let json: serde_json::Value = serde_json::from_slice(&raw).expect("valid artifact json");
    let obj = json["bytecode"]["object"]
        .as_str()
        .expect("bytecode.object string");
    hex::decode(obj.trim_start_matches("0x")).expect("valid bytecode hex")
}

async fn anvil_set_balance(rpc_url: &str, addr: Address, wei: U256) {
    let client = reqwest::Client::new();
    let resp = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "anvil_setBalance",
            "params": [format!("0x{:040x}", addr), format!("0x{:x}", wei)],
            "id": 1
        }))
        .send()
        .await
        .expect("anvil_setBalance post")
        .error_for_status()
        .expect("anvil_setBalance ok");
    let val: serde_json::Value = resp.json().await.expect("json");
    assert!(val.get("error").is_none(), "anvil_setBalance error: {val}");
}

async fn deploy_contract(rpc_url: &str, deployer_key: &str, init_code: Vec<u8>) -> Address {
    let signer: PrivateKeySigner = deployer_key.parse().unwrap();
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(rpc_url.parse().unwrap());
    let tx = TransactionRequest::default().with_deploy_code(init_code);
    let receipt = provider
        .send_transaction(tx)
        .await
        .expect("send deploy tx")
        .get_receipt()
        .await
        .expect("deploy receipt");
    assert!(receipt.status(), "contract deployment reverted");
    receipt
        .contract_address
        .expect("deployed contract address")
}

fn signed_publish_tx(name: &str, signers: &[(&str, &SigningKey)]) -> Transaction {
    let canonical = format!("npm:{name}@1.0.0");
    let content_hash = common::sha256_hex(format!("rehearsal-pkg-{name}").as_bytes());
    let model_version = "creg-detect-v1.0.0".to_string();
    let analysis_bundles = AnalysisBundleRefs {
        policy_bundle_id: "policy-v1".into(),
        feature_schema_id: "features-v1".into(),
        expert_bundle_id: "experts-v1".into(),
        embedding_model_id: "embeddings-v1".into(),
        index_epoch: "index-epoch-1".into(),
        threshold_profile_id: "thresholds-v1".into(),
        llm_prompt_profile_id: "llm-prompt-v1".into(),
        osv_snapshot_epoch: "osv-off".into(),
    };
    let validator_signatures = signers
        .iter()
        .map(|(id, sk)| {
            let validator_pubkey = hex::encode(sk.verifying_key().as_bytes());
            let evidence_digest =
                common::sha256_hex(format!("{canonical}:{content_hash}:evidence").as_bytes());
            let scanner_profile = common::scanner_profile_digest(&model_version, &analysis_bundles);
            let msg = node::gossip::canonical_vote_message(
                &canonical,
                &content_hash,
                true,
                &validator_pubkey,
                &scanner_profile,
                &evidence_digest,
            );
            ValidatorSignature {
                validator_id: (*id).into(),
                validator_pubkey,
                signature: hex::encode(sk.sign(msg.as_bytes()).to_bytes()),
                vote: ValidatorVote::Approve,
                signed_at: Utc::now(),
                ml_model_version: model_version.clone(),
                analysis_bundles: analysis_bundles.clone(),
                evidence_digest,
                deterministic_risk: DeterministicRiskSummary::default(),
                is_mandatory: true,
                ..Default::default()
            }
        })
        .collect();

    Transaction::Publish(ChainRecord {
        id: PackageId::new("npm", name, "1.0.0"),
        content_hash,
        ipfs_cid: "bafyrehearsal".into(),
        publisher_pubkey: "pk-rehearsal".into(),
        block_hash: "0".repeat(64),
        published_at: Utc::now(),
        validator_signatures,
        status: PackageStatus::Verified,
        ..Default::default()
    })
}

struct NodeHarness {
    node_id: String,
    ed_sk: SigningKey,
    evm_key_hex: String,
    evm_addr: Address,
    data_dir: PathBuf,
    _temp_dir: TempDir,
    state: SharedState,
    store: Arc<AttestationStore>,
    p2p_handle: P2PHandle,
    p2p_multiaddr: String,
    tx_sender: finalized_tx::FinalizedTxSender,
    http_url: String,
    p2p_task: JoinHandle<()>,
    producer_task: JoinHandle<()>,
    http_task: JoinHandle<()>,
    admission_task: Option<JoinHandle<()>>,
}

impl Drop for NodeHarness {
    fn drop(&mut self) {
        self.p2p_task.abort();
        self.producer_task.abort();
        self.http_task.abort();
        if let Some(t) = self.admission_task.take() {
            t.abort();
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn spawn_rehearsal_node(
    node_id: &str,
    ed_seed: u8,
    evm_key_hex: &str,
    bootstrap: &ValidatorSet,
    is_validator: bool,
    admission_auto_activate: bool,
    eth_rpc_url: &str,
    staking_addr: Address,
) -> NodeHarness {
    let temp_dir = TempDir::new().expect("temp dir");
    let data_dir = temp_dir.path().to_path_buf();
    let ed_sk = SigningKey::from_bytes(&[ed_seed; 32]);
    let ed_priv_hex = hex::encode(ed_sk.to_bytes());
    let ed_pub_hex = hex::encode(ed_sk.verifying_key().as_bytes());
    let evm_signer: PrivateKeySigner = evm_key_hex.parse().unwrap();
    let evm_addr = evm_signer.address();

    let chain = ChainStore::open(&data_dir).expect("open chain store");
    let genesis = chain.get_block_by_height(0).unwrap().unwrap();
    let genesis_hash = genesis.hash();
    validator_set_history::record(&data_dir, 0, bootstrap).expect("seed history");
    if is_validator {
        chain
            .initialize_pbft_safety_journal(&genesis_hash, node_id, &ed_pub_hex)
            .expect("init pbft journal");
    }

    let p2p_port = free_loopback_port();
    let p2p_multiaddr = format!("/ip4/127.0.0.1/tcp/{p2p_port}");
    let (p2p_node, p2p_handle) = P2PNode::new(&p2p_multiaddr).expect("create p2p");

    let mut cfg = NodeConfig {
        chain_id: "creg-rehearsal-r7".into(),
        network_genesis_hash: genesis_hash,
        node_id: node_id.into(),
        data_dir: data_dir.clone(),
        p2p_listen: p2p_multiaddr.clone(),
        eth_rpc_url: eth_rpc_url.into(),
        staking_addr: format!("0x{:040x}", staking_addr),
        validator_privkey: Some(ed_priv_hex.into()),
        is_validator,
        admission_auto_activate,
        is_testnet: true,
        block_interval_secs: 1,
        validator_set: bootstrap.clone(),
        ..Default::default()
    };
    if is_validator {
        for v in &mut cfg.validator_set.validators {
            if v.id == node_id {
                v.status = "self".into();
            } else {
                v.status = "online".into();
            }
        }
    }

    let (tx_sender, tx_receiver) = finalized_tx::channel();
    let event_bus = new_event_bus();
    let store = AttestationStore::for_chain_in_dir(Some(31337), Some(data_dir.clone()));
    store.set_finality_lag_override(Some(0)).await;

    let state: SharedState = Arc::new(RwLock::new(NodeState {
        chain,
        pending_pool: PendingPool::open(&data_dir),
        publisher_index: PublisherIndex::new(),
        validator_set_bootstrap: bootstrap.clone(),
        validator_set_snapshot: Arc::new(ArcSwap::from_pointee(cfg.validator_set.clone())),
        validator_set: cfg.validator_set.clone(),
        package_rounds: HashMap::new(),
        package_committees: HashMap::new(),
        config: cfg,
        p2p_status: P2PStatus::default(),
        bridge_status: BridgeStatus::default(),
        vrf_proofs: HashMap::new(),
        decryption_shares: HashMap::new(),
        validator_registrations: HashMap::new(),
        validator_set_sync: ValidatorSetSyncStatus::default(),
        view_change_certs: HashMap::new(),
        reorgs: Vec::new(),
        pbft_engine: PbftEngine::new(),
        forced_inclusion_tracker: ForcedInclusionTracker::new(),
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
        equivocation: EquivocationMonitor::disabled(),
    }));

    let p2p_task = {
        let st = Arc::clone(&state);
        let eb = Arc::clone(&event_bus);
        let adm = Arc::clone(&store);
        tokio::spawn(async move {
            let _ = p2p_node.run(st, eb, adm).await;
        })
    };

    let producer_task = {
        let st = Arc::clone(&state);
        let p2p_h = p2p_handle.clone();
        tokio::spawn(block_producer::run(st, tx_receiver, p2p_h))
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind http");
    let http_url = format!("http://{}", listener.local_addr().unwrap());
    let router = api::router(
        Arc::clone(&state),
        event_bus,
        RateLimiter::new(RateLimitConfig::default()),
        Arc::clone(&store),
        CorsConfig::default(),
        tx_sender.clone(),
        p2p_handle.clone(),
    );
    let http_task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    NodeHarness {
        node_id: node_id.into(),
        ed_sk,
        evm_key_hex: evm_key_hex.into(),
        evm_addr,
        data_dir,
        _temp_dir: temp_dir,
        state,
        store,
        p2p_handle,
        p2p_multiaddr,
        tx_sender,
        http_url,
        p2p_task,
        producer_task,
        http_task,
        admission_task: None,
    }
}

async fn query_admission_status(
    client: &reqwest::Client,
    http_url: &str,
    applicant: Address,
) -> serde_json::Value {
    client
        .get(format!(
            "{http_url}/v1/public/validator/admission-status?applicant=0x{:040x}",
            applicant
        ))
        .send()
        .await
        .expect("query admission-status")
        .error_for_status()
        .expect("admission-status ok")
        .json()
        .await
        .expect("admission-status json")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn end_to_end_automatic_validator_admission_rehearsal() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,node::consensus_admission=info,consensus::pbft=info")
        .try_init();

    // 1. Start real Anvil & deploy compiled CregToken + Staking contracts
    let anvil = spawn_anvil().await;
    let gov_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let gov_signer: PrivateKeySigner = gov_key.parse().unwrap();
    let gov_addr = gov_signer.address();

    let mut token_code = load_contract_bytecode("CregToken.sol/CregToken.json");
    token_code.extend_from_slice(&(gov_addr, gov_addr, gov_addr, gov_addr).abi_encode());
    let token_addr = deploy_contract(&anvil.rpc_url, gov_key, token_code).await;

    let mut staking_code = load_contract_bytecode("Staking.sol/Staking.json");
    staking_code.extend_from_slice(&(gov_addr, token_addr).abi_encode());
    let staking_addr = deploy_contract(&anvil.rpc_url, gov_key, staking_code).await;

    // 4 existing validators + 1 fresh applicant EVM keys (Foundry test keys 1..=5)
    let evm_keys = [
        "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d", // v1 (only funded submitter)
        "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a", // v2 (0 ETH after setup)
        "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6", // v3 (0 ETH after setup)
        "0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a", // v4 (0 ETH after setup)
        "0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba", // applicant-5
    ];
    let one_hundred_creg = U256::from(100u64) * U256::from(10u64).pow(U256::from(18u64));
    let ten_eth = U256::from(10u64) * U256::from(10u64).pow(U256::from(18u64));

    let gov_provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(EthereumWallet::from(gov_signer))
        .on_http(anvil.rpc_url.parse().unwrap());
    let gov_token = IRehearsalToken::new(token_addr, &gov_provider);
    let gov_staking = IRehearsalStaking::new(staking_addr, &gov_provider);

    let mut addrs = Vec::new();
    for (idx, k) in evm_keys.iter().enumerate() {
        let s: PrivateKeySigner = k.parse().unwrap();
        let addr = s.address();
        addrs.push(addr);
        anvil_set_balance(&anvil.rpc_url, addr, ten_eth).await;
        let rec = gov_token
            .transfer(addr, one_hundred_creg)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert!(rec.status());

        let node_provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(EthereumWallet::from(s))
            .on_http(anvil.rpc_url.parse().unwrap());
        let tok = IRehearsalToken::new(token_addr, &node_provider);
        let stk = IRehearsalStaking::new(staking_addr, &node_provider);
        assert!(tok
            .approve(staking_addr, one_hundred_creg)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap()
            .status());
        assert!(stk
            .applyToBeValidator(one_hundred_creg)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap()
            .status());
        if idx < 4 {
            assert!(gov_staking
                .approveValidator(addr)
                .send()
                .await
                .unwrap()
                .get_receipt()
                .await
                .unwrap()
                .status());
        }
    }

    // Drain v2, v3, v4 ETH so ONLY v1 holds gas for L1 submission.
    for addr in &addrs[1..4] {
        anvil_set_balance(&anvil.rpc_url, *addr, U256::ZERO).await;
        assert_eq!(gov_provider.get_balance(*addr).await.unwrap(), U256::ZERO);
    }
    assert!(gov_provider.get_balance(addrs[0]).await.unwrap() > U256::ZERO);
    assert_eq!(gov_staking.activeValidatorCount().call().await.unwrap()._0, U256::from(4u64));
    assert_eq!(gov_staking.validatorSetEpoch().call().await.unwrap()._0, U256::from(4u64));
    assert_eq!(gov_staking.validators(addrs[4]).call().await.unwrap().state, 1u8);

    // 2. Build bootstrap 4-validator set and spawn 4 validators + 1 fresh observer
    let seeds = [11u8, 12, 13, 14, 15];
    let bootstrap_validators: Vec<Validator> = (0..4)
        .map(|i| {
            let sk = SigningKey::from_bytes(&[seeds[i]; 32]);
            Validator {
                id: format!("val-{}", i + 1),
                alias: format!("val-{}", i + 1),
                pubkey: hex::encode(sk.verifying_key().as_bytes()),
                eth_address: format!("0x{:040x}", addrs[i]),
                stake: 100,
                reputation: 100,
                status: "active".into(),
                performance_score: 500,
                contribution_count: 0,
                pq_pubkey: None,
            }
        })
        .collect();
    let bootstrap = ValidatorSet::new(bootstrap_validators);

    let mut validators = Vec::new();
    for i in 0..4 {
        validators.push(
            spawn_rehearsal_node(
                &format!("val-{}", i + 1),
                seeds[i],
                evm_keys[i],
                &bootstrap,
                true,
                false,
                &anvil.rpc_url,
                staking_addr,
            )
            .await,
        );
    }
    let mut applicant_node = spawn_rehearsal_node(
        "ext-val-applicant",
        seeds[4],
        evm_keys[4],
        &bootstrap,
        false,
        true,
        &anvil.rpc_url,
        staking_addr,
    )
    .await;

    // Form connected P2P mesh
    let v1_addr: libp2p::Multiaddr = validators[0].p2p_multiaddr.parse().unwrap();
    let v2_addr: libp2p::Multiaddr = validators[1].p2p_multiaddr.parse().unwrap();
    for node in validators.iter().skip(1).chain(std::iter::once(&applicant_node)) {
        node.p2p_handle
            .sender
            .send(P2PCommand::Dial {
                addr: v1_addr.clone(),
            })
            .await
            .unwrap();
    }
    for node in validators.iter().skip(2).chain(std::iter::once(&applicant_node)) {
        node.p2p_handle
            .sender
            .send(P2PCommand::Dial {
                addr: v2_addr.clone(),
            })
            .await
            .unwrap();
    }

    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let mut ready = true;
            for n in validators.iter().chain(std::iter::once(&applicant_node)) {
                if n.state.read().await.p2p_status.peers.len() < 2 {
                    ready = false;
                    break;
                }
            }
            if ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("loopback P2P mesh connects");
    tokio::time::sleep(Duration::from_secs(1)).await;

    // 3. Finalize baseline block #1 under the 4 bootstrap validators
    let signer_refs: Vec<(&str, &SigningKey)> = validators
        .iter()
        .map(|v| (v.node_id.as_str(), &v.ed_sk))
        .collect();
    let tx1 = signed_publish_tx("rehearsal-baseline", &signer_refs);
    for v in &validators {
        v.tx_sender.send(tx1.clone()).await.unwrap();
    }

    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let mut all_at_1 = true;
            for n in validators.iter().chain(std::iter::once(&applicant_node)) {
                if n.state.read().await.chain.tip_height().unwrap_or(0) < 1 {
                    all_at_1 = false;
                    break;
                }
            }
            if all_at_1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("block #1 finalizes across all 5 nodes");
    assert!(
        !applicant_node.state.read().await.config.is_validator,
        "applicant must stay observer before certified admission"
    );

    // 4. Dual-signed registration for applicant-5 via v1 REST API
    let candidate_identity = ValidatorIdentity {
        evm_address: format!("0x{:040x}", applicant_node.evm_addr),
        node_id: applicant_node.node_id.clone(),
        ed25519_pubkey: hex::encode(applicant_node.ed_sk.verifying_key().as_bytes()),
    }
    .normalized();
    let nonce = "rehearsal-nonce-1";
    let reg_msg = validator_identity_registration_message(
        "creg-rehearsal-r7",
        &candidate_identity.evm_address,
        &candidate_identity.node_id,
        &candidate_identity.ed25519_pubkey,
        nonce,
    );
    let eth_digest = keccak256(format!(
        "\x19Ethereum Signed Message:\n{}{}",
        reg_msg.len(),
        reg_msg
    ));
    let app_evm_signer: PrivateKeySigner = applicant_node.evm_key_hex.parse().unwrap();
    let eth_sig = app_evm_signer.sign_hash(&eth_digest).await.unwrap();
    let eth_sig_hex = format!("0x{}", hex::encode(eth_sig.as_bytes()));
    let ed_sig_hex = hex::encode(applicant_node.ed_sk.sign(reg_msg.as_bytes()).to_bytes());

    let http_client = reqwest::Client::new();
    let reg_resp = http_client
        .post(format!("{}/v1/validator/register", validators[0].http_url))
        .json(&serde_json::json!({
            "evm_address": candidate_identity.evm_address,
            "node_id": candidate_identity.node_id,
            "ed25519_pubkey": candidate_identity.ed25519_pubkey,
            "alias": "ext-val-applicant",
            "nonce": nonce,
            "evm_signature": eth_sig_hex,
            "ed25519_signature": ed_sig_hex
        }))
        .send()
        .await
        .expect("POST /v1/validator/register");
    assert_eq!(reg_resp.status(), reqwest::StatusCode::ACCEPTED);

    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let mut synced = true;
            for n in validators.iter().chain(std::iter::once(&applicant_node)) {
                if !n
                    .state
                    .read()
                    .await
                    .validator_registrations
                    .contains_key(&candidate_identity.evm_address)
                {
                    synced = false;
                    break;
                }
            }
            if synced {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("registration gossips to all 5 nodes");

    // 5. Phase A: Provision ONLY v1 and v2 signers -> prove 2-of-4 is insufficient
    validators[0]
        .store
        .set_admission_signer_override(Some(Some(validators[0].evm_key_hex.clone())))
        .await;
    validators[1]
        .store
        .set_admission_signer_override(Some(Some(validators[1].evm_key_hex.clone())))
        .await;
    validators[2].store.set_admission_signer_override(Some(None)).await;
    validators[3].store.set_admission_signer_override(Some(None)).await;

    for v in &mut validators {
        v.admission_task = Some(tokio::spawn(consensus_admission::run_with_interval(
            Arc::clone(&v.state),
            Arc::clone(&v.store),
            v.p2p_handle.clone(),
            Duration::from_millis(250),
        )));
    }
    applicant_node.admission_task = Some(tokio::spawn(consensus_admission::run_with_interval(
        Arc::clone(&applicant_node.state),
        Arc::clone(&applicant_node.store),
        applicant_node.p2p_handle.clone(),
        Duration::from_millis(250),
    )));

    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let status = query_admission_status(&http_client, &validators[0].http_url, addrs[4]).await;
            if status["applicants"]
                .as_array()
                .and_then(|a| a.first())
                .is_some_and(|app| app["l1_approvals_collected"] == 2)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    })
    .await
    .expect("2/3 approvals collected cleanly");

    tokio::time::sleep(Duration::from_millis(1500)).await;
    let status_2of4 = query_admission_status(&http_client, &validators[0].http_url, addrs[4]).await;
    let app_2of4 = &status_2of4["applicants"][0];
    assert_eq!(app_2of4["l1_approvals_collected"], 2);
    assert_eq!(app_2of4["l1_approvals_required"], 3);
    assert_eq!(app_2of4["l1_tx_status"], "not_submitted");
    assert_eq!(app_2of4["stage"], "collecting_approvals");
    assert_eq!(gov_staking.validators(addrs[4]).call().await.unwrap().state, 1u8);
    assert!(!applicant_node.state.read().await.config.is_validator);

    let v3_status = query_admission_status(&http_client, &validators[2].http_url, addrs[4]).await;
    assert_eq!(v3_status["local_node"]["admission_signer_configured"], false);
    assert!(
        !v3_status["applicants"][0]["missing_signer_setup"]
            .as_array()
            .unwrap()
            .is_empty(),
        "v3 without signer must surface missing_signer_setup"
    );

    // 6. Phase B: Interrupt v2's admission worker and reload durable store from disk
    validators[1].admission_task.take().unwrap().abort();
    let v2_reloaded = AttestationStore::for_chain_in_dir(
        Some(31337),
        Some(validators[1].data_dir.clone()),
    );
    v2_reloaded.set_finality_lag_override(Some(0)).await;
    v2_reloaded
        .set_admission_signer_override(Some(Some(validators[1].evm_key_hex.clone())))
        .await;
    validators[1].store = Arc::clone(&v2_reloaded);
    validators[1].admission_task = Some(tokio::spawn(consensus_admission::run_with_interval(
        Arc::clone(&validators[1].state),
        v2_reloaded,
        validators[1].p2p_handle.clone(),
        Duration::from_millis(250),
    )));
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let status_after_restart =
        query_admission_status(&http_client, &validators[0].http_url, addrs[4]).await;
    assert_eq!(
        status_after_restart["applicants"][0]["l1_approvals_collected"], 2,
        "resuming interrupted node must not duplicate approvals"
    );

    // 7. Phase C: Enable v3 signer (0 ETH!) -> 3-of-4 advances automatically to active voting
    let t_start = Instant::now();
    validators[2]
        .store
        .set_admission_signer_override(Some(Some(validators[2].evm_key_hex.clone())))
        .await;

    // 7a. Measure signature collection
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let s = query_admission_status(&http_client, &validators[0].http_url, addrs[4]).await;
            let app = &s["applicants"][0];
            if app["l1_approvals_collected"].as_u64().unwrap_or(0) >= 3
                || app["l1_tx_status"] == "confirmed"
                || app["l1_tx_status"] == "submitted"
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("signature collection reaches 3/3");
    let d_sig_collection = t_start.elapsed();
    let t_quorum = Instant::now();

    // 7b. Measure L1 inclusion & confirmation
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let v = gov_staking.validators(addrs[4]).call().await.unwrap();
            let s = query_admission_status(&http_client, &validators[0].http_url, addrs[4]).await;
            if v.state == 2u8 && s["applicants"][0]["l1_tx_status"] == "confirmed" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("approveByConsensus confirms on Anvil");
    let d_l1_confirm = t_quorum.elapsed();
    let t_l1_done = Instant::now();
    assert_eq!(gov_staking.validatorSetEpoch().call().await.unwrap()._0, U256::from(5u64));
    assert_eq!(gov_staking.activeValidatorCount().call().await.unwrap()._0, U256::from(5u64));
    assert_eq!(
        gov_provider.get_balance(addrs[2]).await.unwrap(),
        U256::ZERO,
        "non-submitting signer v3 must spend zero ETH"
    );

    // 7c. Measure voting-set activation across fleet & candidate observer auto-activation
    let act_height = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if applicant_node.state.read().await.config.is_validator {
                let act_h_opt = {
                    let s = applicant_node.state.read().await;
                    let pk = hex::encode(applicant_node.ed_sk.verifying_key().as_bytes());
                    validator_set_history::certified_activation_height_for(
                        &s.config.data_dir,
                        &applicant_node.node_id,
                        &pk,
                        &format!("0x{:040x}", applicant_node.evm_addr),
                    )
                    .ok()
                    .flatten()
                };
                if let Some(act_h) = act_h_opt {
                    let mut all_five = true;
                    for n in validators.iter().chain(std::iter::once(&applicant_node)) {
                        let s = n.state.read().await;
                        let auth = validator_set_history::verification_set_at(
                            &s.config.data_dir,
                            act_h,
                            &s.validator_set_bootstrap,
                        )
                        .unwrap();
                        if auth.validators.len() != 5 {
                            all_five = false;
                            break;
                        }
                    }
                    if all_five {
                        break act_h;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("certified 5-validator transition activates candidate observer");
    let d_voting_activation = t_l1_done.elapsed();
    let t_activated = Instant::now();
    assert!(act_height >= 2);
    assert!(applicant_node.state.read().await.chain.pbft_safety_journal().is_ok());

    // Disconnect one non-leader bootstrap validator among v2..v4 so 4-of-5 PBFT quorum
    // strictly requires applicant-5's commit signature to finalize subsequent blocks.
    let pre_stop_tip = applicant_node.state.read().await.chain.tip_height().unwrap();
    let pre_stop_hash = validators[0].state.read().await.chain.tip_hash().unwrap();
    let rank0_id = {
        let s = validators[0].state.read().await;
        let auth = validator_set_history::verification_set_at(
            &s.config.data_dir,
            pre_stop_tip.saturating_add(1),
            &s.validator_set_bootstrap,
        )
        .unwrap();
        let vrf_vals: Vec<consensus::vrf::VrfValidator> = auth
            .validators
            .iter()
            .map(|v| consensus::vrf::VrfValidator {
                id: v.id.clone(),
                pubkey: v.pubkey.clone(),
                vrf_output: None,
                vrf_proof: None,
            })
            .collect();
        consensus::vrf::rank_proposers(&vrf_vals, &pre_stop_hash)
            .into_iter()
            .next()
            .unwrap()
    };
    let stop_idx = (1..=3)
        .find(|&idx| validators[idx].node_id != rank0_id)
        .expect("at least one non-leader in v2..v4");
    validators[stop_idx].p2p_task.abort();
    validators[stop_idx].producer_task.abort();
    if let Some(t) = validators[stop_idx].admission_task.take() {
        t.abort();
    }

    // 7d. Propose & finalize a block under 5-validator quorum (4 of 5) and capture first vote
    let mut five_signers: Vec<(&str, &SigningKey)> = validators
        .iter()
        .map(|v| (v.node_id.as_str(), &v.ed_sk))
        .collect();
    five_signers.push((applicant_node.node_id.as_str(), &applicant_node.ed_sk));
    let tx2 = signed_publish_tx("rehearsal-post-admission", &five_signers);
    for (idx, v) in validators.iter().enumerate() {
        if idx != stop_idx {
            v.tx_sender.send(tx2.clone()).await.unwrap();
        }
    }
    applicant_node.tx_sender.send(tx2).await.unwrap();

    let vote_wait = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let s = applicant_node.state.read().await;
            let tip = s.chain.tip_height().unwrap_or(0);
            for h in act_height..=tip {
                if let Ok(Some(blk)) = s.chain.get_block_by_height(h) {
                    if let Some(sig) = blk
                        .pbft_signatures
                        .iter()
                        .find(|sig| sig.validator_id == applicant_node.node_id)
                    {
                        verify_pbft_phase_signature("commit", &blk.hash(), &sig.pubkey, &sig.signature)
                            .expect("applicant-5 commit signature verifies");
                        return h;
                    }
                }
            }
            drop(s);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    let first_vote_h = match vote_wait {
        Ok(h) => h,
        Err(elapsed) => {
            for (idx, n) in validators
                .iter()
                .enumerate()
                .filter(|(idx, _)| *idx != stop_idx)
                .map(|(_, v)| ((), v))
                .chain(std::iter::once(((), &applicant_node)))
            {
                let _ = idx;
                let s = n.state.read().await;
                let tip = s.chain.tip_height().unwrap_or(0);
                let live_round = s.pbft_engine.non_terminal_round_at_height(tip.saturating_add(1));
                eprintln!(
                    "VOTE_DIAG node={} tip={} is_val={} peers={} live_round={:?}",
                    n.node_id,
                    tip,
                    s.config.is_validator,
                    s.p2p_status.peers.len(),
                    live_round
                );
            }
            panic!("post-activation block finalizes with applicant-5's commit signature: {elapsed:?}");
        }
    };
    let d_first_vote = t_activated.elapsed();
    assert!(first_vote_h >= act_height);

    // 8. Verify admission-status telemetry on v1 (wait one short tick for first vote cache refresh)
    let final_v1_status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let st = query_admission_status(&http_client, &validators[0].http_url, addrs[4]).await;
            if st["applicants"][0]["stage"] == "active_voting" {
                break st;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    })
    .await
    .expect("admission-status transitions to active_voting");
    let final_app = &final_v1_status["applicants"][0];
    assert_eq!(final_app["stage"], "active_voting");
    assert_eq!(final_app["l1_tx_status"], "confirmed");
    assert!(final_app["l1_tx_hash"].as_str().is_some());
    assert_eq!(final_app["voting_activation_height"], act_height);
    assert_eq!(final_app["first_finalized_vote_height"], first_vote_h);

    println!("\n=== ADMISSION REHEARSAL STAGE TIMINGS (4 validators + 1 applicant on Anvil) ===");
    println!("  Certified activation height:                #{act_height}");
    println!("  First finalized vote height:                #{first_vote_h}");
    println!("  1. Signature collection (3/3 EIP-712):      {d_sig_collection:?}");
    println!("  2. L1 inclusion & confirmation (Anvil tx):  {d_l1_confirm:?}");
    println!("  3. Voting-set activation (R7 cert):         {d_voting_activation:?}");
    println!("  4. First finalized vote (4-of-5 commit):    {d_first_vote:?}");
    println!(
        "  Total end-to-end admission elapsed:         {:?}",
        t_start.elapsed()
    );
    println!("================================================================================\n");
}
