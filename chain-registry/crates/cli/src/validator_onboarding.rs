//! R7 validator enrollment. The browser wallet authorizes L1 transactions and
//! the CLI signs the same registration challenge with the local validator key.

use anyhow::{bail, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use common::{validator_identity_registration_message, ChainSpec, ValidatorIdentity};
use ed25519_dalek::{Signer, SigningKey};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;

const R7_SPEC: &str = include_str!("../../../testnet/phase-c/r7/chain-spec.json");
const R7_SIGNATURE: &str = include_str!("../../../testnet/phase-c/r7/chain-spec.json.sig");
const R7_SPEC_SIGNER: &str = "3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01";
const DEFAULT_NODE_ID: &str = "ext-val-primary";
const R7_API: &str = "https://api.testnet.cregnet.dev";
const LOCAL_NODE: &str = "http://127.0.0.1:8080";
const LOCAL_NODE_UNAVAILABLE: &str = "Local node unavailable";
const LOCAL_NODE_MISMATCH: &str = "Local node identity or network does not match";

#[derive(Clone)]
struct Enrollment {
    signing_key: Arc<SigningKey>,
    client: Client,
    network: Url,
    local_node: Url,
    config: BrowserConfig,
    origin: String,
    session: String,
    nonce: String,
    vote_height: Arc<Mutex<Option<u64>>>,
    launcher: Option<Launcher>,
    launch_status: Arc<Mutex<&'static str>>,
}

#[derive(Clone)]
struct Launcher {
    script: PathBuf,
    key_file: PathBuf,
    log_file: PathBuf,
    http_port: Option<u16>,
}

struct ManagedRuntime {
    port: Option<u16>,
    failed: bool,
}

#[derive(Clone, Serialize)]
struct BrowserConfig {
    chain_id: String,
    l1_chain_id: u64,
    staking: String,
    token: String,
    faucet: String,
    node_id: String,
    public_key: String,
    launch_log: Option<String>,
    approve: String,
    apply: String,
    allowance: String,
    balance: String,
    validator: String,
    minimum: String,
}

#[derive(Deserialize)]
struct ChainStats {
    chain_id: String,
    spec_signature_verified: bool,
    finalized_height: u64,
    peer_count: usize,
    validator_set_sync_state: String,
}

#[derive(Deserialize)]
struct Registration {
    identity: ValidatorIdentity,
    #[serde(default)]
    registered_with_node: bool,
    #[serde(default)]
    admitted_to_consensus: bool,
    #[serde(default)]
    active: bool,
}

#[derive(Deserialize)]
struct LocalHealth {
    node_id: String,
    is_validator: bool,
    sandbox: SandboxHealth,
    sync: SyncHealth,
}

#[derive(Deserialize)]
struct SandboxHealth {
    isolated: bool,
    dev_bypass: bool,
    degraded: bool,
}

#[derive(Deserialize)]
struct SyncHealth {
    lag_blocks: u64,
}

#[derive(Serialize)]
struct EnrollmentStatus {
    registered: bool,
    admitted: bool,
    local_node: &'static str,
    vote_height: Option<u64>,
    launch_status: &'static str,
    launch_error: Option<&'static str>,
    admission: Option<AdmissionProgressView>,
    admission_unavailable_reason: Option<String>,
    signer_warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StageTimings {
    #[serde(default)]
    pub started_at_ms: Option<i64>,
    #[serde(default)]
    pub l1_quorum_reached_at_ms: Option<i64>,
    #[serde(default)]
    pub l1_tx_submitted_at_ms: Option<i64>,
    #[serde(default)]
    pub l1_confirmed_at_ms: Option<i64>,
    #[serde(default)]
    pub transition_certified_at_ms: Option<i64>,
    #[serde(default)]
    pub first_vote_observed_at_ms: Option<i64>,
    #[serde(default)]
    pub signature_collection_ms: Option<u64>,
    #[serde(default)]
    pub l1_confirmation_ms: Option<u64>,
    #[serde(default)]
    pub voting_activation_ms: Option<u64>,
    #[serde(default)]
    pub first_finalized_vote_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct NetworkAdmissionSummary {
    #[serde(default)]
    pub chain_id: String,
    #[serde(default)]
    pub tip_height: u64,
    #[serde(default)]
    pub finalized_height: u64,
    #[serde(default)]
    pub active_validators: u64,
    #[serde(default)]
    pub required_l1_approvals: u64,
    #[serde(default)]
    pub required_transition_approvals: u64,
    #[serde(default)]
    pub validator_set_epoch: Option<u64>,
    #[serde(default)]
    pub rpc_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LocalNodeAdmissionSummary {
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub is_validator: bool,
    #[serde(default)]
    pub admission_auto_activate: bool,
    #[serde(default)]
    pub admission_signer_configured: bool,
    #[serde(default)]
    pub admission_signer_address: Option<String>,
    #[serde(default)]
    pub admission_signer_matches_active_wallet: bool,
    #[serde(default)]
    pub admission_signer_active_on_l1: bool,
    #[serde(default)]
    pub admission_signer_has_gas: bool,
    #[serde(default)]
    pub admission_signer_issue: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AdmissionProgressView {
    #[serde(default)]
    pub applicant: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub ed25519_pubkey: String,
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub stage_reason: String,
    #[serde(default)]
    pub staking_state: String,
    #[serde(default)]
    pub stake_creg: u64,
    #[serde(default)]
    pub application_nonce: Option<u64>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub l1_approvals_collected: u64,
    #[serde(default)]
    pub l1_approvals_required: u64,
    #[serde(default)]
    pub l1_signers: Vec<String>,
    #[serde(default)]
    pub transition_approvals_collected: u64,
    #[serde(default)]
    pub transition_approvals_required: u64,
    #[serde(default)]
    pub transition_signers: Vec<String>,
    #[serde(default)]
    pub missing_signer_setup: Vec<String>,
    #[serde(default)]
    pub l1_tx_hash: Option<String>,
    #[serde(default)]
    pub l1_tx_status: String,
    #[serde(default)]
    pub l1_confirmed_block: Option<u64>,
    #[serde(default)]
    pub voting_activation_height: Option<u64>,
    #[serde(default)]
    pub first_finalized_vote_height: Option<u64>,
    #[serde(default)]
    pub timings: StageTimings,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AdmissionStatusReport {
    #[serde(default)]
    pub local_node: LocalNodeAdmissionSummary,
    #[serde(default)]
    pub network: NetworkAdmissionSummary,
    #[serde(default)]
    pub applicants: Vec<AdmissionProgressView>,
}

#[derive(Deserialize)]
struct RecentBlocks {
    blocks: Vec<ObservedBlock>,
}

#[derive(Deserialize)]
struct ObservedBlock {
    header: ObservedHeader,
    finalized: bool,
    pbft_signatures: Vec<common::BlockSignature>,
}

#[derive(Deserialize)]
struct ObservedHeader {
    height: u64,
}

#[derive(Deserialize)]
struct Wallet {
    address: String,
}

#[derive(Deserialize)]
struct WalletProof {
    address: String,
    signature: String,
}

type ApiError = (StatusCode, Json<serde_json::Value>);

fn api_error(status: StatusCode, error: impl Into<String>) -> ApiError {
    (status, Json(serde_json::json!({ "error": error.into() })))
}

fn validate_node_id(node_id: &str) -> Result<()> {
    if node_id.is_empty()
        || node_id.len() > 64
        || !node_id
            .bytes()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric())
        || !node_id
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"._-".contains(&ch))
    {
        bail!("Use a node ID that starts with a letter or digit and contains up to 64 letters, digits, dots, hyphens, or underscores");
    }
    Ok(())
}

fn runtime_key_path(node_id: &str) -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not find your home directory")?;
    let dir = if node_id == DEFAULT_NODE_ID {
        ".creg-validator".to_owned()
    } else {
        format!(".creg-validator-{node_id}")
    };
    Ok(home.join(dir).join("validator.key"))
}

fn runtime_key(path: &Path, create: bool) -> Result<SigningKey> {
    if create && !path.exists() {
        crate::keygen::refuse_in_repo_key_path(path)?;
        let parent = path
            .parent()
            .context("Validator key needs a parent directory")?;
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(hex::encode(rand::random::<[u8; 32]>()).as_bytes())?;
        file.as_file().sync_all()?;
        // Publish the complete key only if another session has not created it.
        match file.persist_noclobber(path) {
            Ok(_) => {}
            Err(err) if err.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err.error).context("Could not create validator key"),
        }
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Could not read validator key at {}", path.display()))?;
    let mut seed = [0; 32];
    hex::decode_to_slice(text.trim().trim_start_matches("0x"), &mut seed).map_err(|_| {
        anyhow::anyhow!("The node key file must contain a 32-byte Ed25519 seed in hex")
    })?;
    Ok(SigningKey::from_bytes(&seed))
}

fn selector(signature: &str) -> String {
    format!(
        "0x{}",
        hex::encode(&Keccak256::digest(signature.as_bytes())[..4])
    )
}

fn browser_config(node_id: String, key: &SigningKey) -> Result<BrowserConfig> {
    let spec: ChainSpec = serde_json::from_str(R7_SPEC)?;
    spec.verify_signature(R7_SIGNATURE.trim(), R7_SPEC_SIGNER)
        .context("The bundled R7 chain specification could not be verified")?;
    let staking = spec
        .contract("staking")
        .context("R7 staking contract missing")?
        .clone();
    let token = spec
        .contract("creg_token")
        .context("R7 token contract missing")?
        .clone();
    Ok(BrowserConfig {
        chain_id: spec.chain_id,
        l1_chain_id: spec.l1.chain_id,
        staking,
        token,
        faucet: "https://faucet.testnet.cregnet.dev".into(),
        node_id,
        public_key: hex::encode(key.verifying_key().as_bytes()),
        launch_log: None,
        approve: selector("approve(address,uint256)"),
        apply: selector("applyToBeValidator(uint256)"),
        allowance: selector("allowance(address,address)"),
        balance: selector("balanceOf(address)"),
        validator: selector("validators(address)"),
        minimum: selector("minValidatorStake()"),
    })
}

fn local_launcher(node_id: &str, key_path: &Path, http_port: Option<u16>) -> Option<Launcher> {
    let script = if cfg!(windows) {
        "start-external-validator.ps1"
    } else {
        "start-external-validator.sh"
    };
    let script = std::env::current_dir()
        .ok()?
        .ancestors()
        .map(|dir| dir.join("chain-registry").join("testnet").join(script))
        .find(|path| path.is_file())?;
    let log_file = runtime_key_path(node_id)
        .ok()?
        .with_file_name("onboarding-launch.log");
    Some(Launcher {
        script,
        key_file: std::path::absolute(key_path).ok()?,
        log_file,
        http_port,
    })
}

impl Launcher {
    async fn docker_output(args: &[&str]) -> Option<String> {
        let mut command = tokio::process::Command::new("docker");
        command.args(args).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let output = tokio::time::timeout(Duration::from_secs(2), command.output())
            .await
            .ok()?
            .ok()?;
        output.status.success().then(|| {
            format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    async fn runtime(&self, config: &BrowserConfig) -> Option<ManagedRuntime> {
        const FORMAT: &str = concat!(
            r#"{{index .Config.Labels "dev.creg.validator"}} {{.State.Status}} "#,
            r#"{{.State.ExitCode}} {{.RestartCount}}"#,
            r#"{{range index .NetworkSettings.Ports "8080/tcp"}} {{.HostIp}} {{.HostPort}}{{end}}"#
        );
        let name = format!("creg-validator-{}", config.node_id);
        let label = format!("{}/{}", config.chain_id, config.public_key);
        let output =
            Self::docker_output(&["container", "inspect", "--format", FORMAT, &name]).await?;
        ManagedRuntime::parse(output.lines().next()?, &label)
    }

    async fn runtime_error(&self, config: &BrowserConfig) -> &'static str {
        let name = format!("creg-validator-{}", config.node_id);
        let logs = Self::docker_output(&["container", "logs", "--tail", "30", &name]).await;
        match logs.as_deref().unwrap_or_default() {
            logs if logs.contains("outside the signed active bootstrap set") =>
                "This R7 image only starts signing nodes from the original roster. Use the reviewed observer profile to sync a new node.",
            logs if logs.contains("has no certified validator admission") =>
                "This identity needs a certified R7 validator-set transition before it can sign.",
            logs if logs.contains("Phase C scanner profile") && logs.contains("is incomplete") =>
                "The validator could not load the reviewed R7 scanner profile. Check the scanner configuration before restarting.",
            _ => "The validator container failed before its health check passed. Check its Docker logs.",
        }
    }

    fn start(&self, config: &BrowserConfig) -> Result<tokio::process::Child> {
        let key = runtime_key(&self.key_file, false)?;
        if hex::encode(key.verifying_key().as_bytes()) != config.public_key {
            bail!("The validator key changed. Reopen onboarding with the current key.");
        }
        let http_port = self.http_port.unwrap_or(0).to_string();
        let mut command = if cfg!(windows) {
            let mut command = tokio::process::Command::new("powershell.exe");
            command
                .args(["-NoProfile", "-ExecutionPolicy", "RemoteSigned", "-File"])
                .arg(&self.script);
            command.arg("-HttpPort").arg(&http_port);
            // The starter treats an omitted ID as the canonical primary directory.
            if config.node_id != DEFAULT_NODE_ID {
                command.arg("-NodeId").arg(&config.node_id);
            }
            command
        } else {
            let mut command = tokio::process::Command::new("bash");
            command
                .arg(&self.script)
                .env("NODE_ID", &config.node_id)
                .env("HTTP_PORT", &http_port);
            command
        };
        let directory = self
            .log_file
            .parent()
            .context("Launcher log needs a parent directory")?;
        std::fs::create_dir_all(directory)?;
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)?;
        command
            .env("CREG_ONBOARDING_KEY_FILE", &self.key_file)
            .env("CREG_ONBOARDING_PUBLIC_KEY", &config.public_key)
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        Ok(command.spawn()?)
    }
}

impl ManagedRuntime {
    fn parse(line: &str, expected_label: &str) -> Option<Self> {
        let mut fields = line.split_whitespace();
        if fields.next()? != expected_label {
            return None;
        }
        let state = fields.next()?;
        let exit_code: u32 = fields.next()?.parse().ok()?;
        let restart_count: u32 = fields.next()?.parse().ok()?;
        let mut port = None;
        while let (Some(host), Some(value)) = (fields.next(), fields.next()) {
            if host == "127.0.0.1" {
                port = value.parse::<u16>().ok().filter(|port| *port > 0);
                break;
            }
        }
        Some(Self {
            port,
            failed: restart_count > 0
                || state == "restarting"
                || state == "dead"
                || (state == "exited" && exit_code != 0),
        })
    }
}

fn node_origin(value: &str, loopback_only: bool) -> Result<Url> {
    let url = Url::parse(value).context("Invalid node URL")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.host_str().is_none()
        || (!local && (loopback_only || url.scheme() != "https"))
        || !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Provide a node origin with HTTPS or a local loopback address");
    }
    Ok(url)
}

fn normalize_address(address: &str) -> Result<String, ApiError> {
    if address.len() != 42
        || !address.starts_with("0x")
        || !address.as_bytes()[2..].iter().all(u8::is_ascii_hexdigit)
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Connect a valid Sepolia wallet",
        ));
    }
    Ok(address.to_ascii_lowercase())
}

impl Enrollment {
    async fn get<T: serde::de::DeserializeOwned>(&self, node: &Url, path: &str) -> Result<T> {
        Ok(self
            .client
            .get(node.join(path)?)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn network_stats(&self) -> Result<ChainStats> {
        let stats: ChainStats = self.get(&self.network, "/v1/public/chain/stats").await?;
        if stats.chain_id != self.config.chain_id || !stats.spec_signature_verified {
            bail!("The selected node is not serving the signed R7 network");
        }
        Ok(stats)
    }

    fn identity(&self, address: String) -> ValidatorIdentity {
        ValidatorIdentity {
            evm_address: address,
            node_id: self.config.node_id.clone(),
            ed25519_pubkey: self.config.public_key.clone(),
        }
    }

    fn challenge(&self, identity: &ValidatorIdentity) -> String {
        validator_identity_registration_message(
            &self.config.chain_id,
            &identity.evm_address,
            &identity.node_id,
            &identity.ed25519_pubkey,
            &self.nonce,
        )
    }

    async fn local_status(
        &self,
        local_node: &Url,
        network_height: u64,
        admitted: bool,
    ) -> &'static str {
        let (stats, health) = tokio::join!(
            self.get::<ChainStats>(local_node, "/v1/public/chain/stats"),
            self.get::<LocalHealth>(local_node, "/v1/health")
        );
        let (Ok(stats), Ok(health)) = (stats, health) else {
            return LOCAL_NODE_UNAVAILABLE;
        };
        if stats.chain_id != self.config.chain_id
            || !stats.spec_signature_verified
            || health.node_id != self.config.node_id
        {
            return LOCAL_NODE_MISMATCH;
        }
        if stats.validator_set_sync_state != "synced"
            || stats.finalized_height < network_height
            || stats.peer_count == 0
            || health.sync.lag_blocks != 0
        {
            return "Syncing with R7";
        }
        if !health.sandbox.isolated || health.sandbox.dev_bypass || health.sandbox.degraded {
            return "Sandbox needs attention";
        }
        if !health.is_validator && admitted {
            "Observer synced. Voting setup pending"
        } else if !health.is_validator {
            "Observer synced, awaiting admission"
        } else if admitted {
            "Synced with active membership"
        } else {
            "Synced, awaiting admission"
        }
    }

    async fn fetch_admission_report(
        &self,
        base: &Url,
        applicant: Option<&str>,
        node_id: Option<&str>,
    ) -> std::result::Result<AdmissionStatusReport, Option<StatusCode>> {
        let mut url = base
            .join("/v1/public/validator/admission-status")
            .map_err(|_| None)?;
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(addr) = applicant.map(str::trim).filter(|v| !v.is_empty()) {
                pairs.append_pair("applicant", addr);
            }
            if let Some(nid) = node_id.map(str::trim).filter(|v| !v.is_empty()) {
                pairs.append_pair("node_id", nid);
            }
        }
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| None)?;
        let code = resp.status();
        if !code.is_success() {
            return Err(Some(code));
        }
        resp.json::<AdmissionStatusReport>()
            .await
            .map_err(|_| None)
    }

    async fn observe_vote(&self, local_origin: Option<&Url>) -> Option<u64> {
        let mut seen = self.vote_height.lock().await;
        if seen.is_none() {
            for base in std::iter::once(&self.network).chain(local_origin.into_iter()) {
                if let Ok(recent) = self
                    .get::<RecentBlocks>(base, "/v1/public/blocks?limit=20")
                    .await
                {
                    let found = recent
                        .blocks
                        .iter()
                        .filter(|block| {
                            block.finalized
                                && block.pbft_signatures.iter().any(|sig| {
                                    sig.validator_id == self.config.node_id
                                        && sig.pubkey.eq_ignore_ascii_case(&self.config.public_key)
                                })
                        })
                        .map(|block| block.header.height)
                        .min();
                    if found.is_some() {
                        *seen = found;
                        break;
                    }
                }
            }
        }
        *seen
    }
}

fn merge_admission_progress(
    primary: Option<AdmissionProgressView>,
    secondary: Option<AdmissionProgressView>,
) -> Option<AdmissionProgressView> {
    match (primary, secondary) {
        (Some(mut a), Some(b)) => {
            if b.l1_approvals_collected > a.l1_approvals_collected {
                a.l1_approvals_collected = b.l1_approvals_collected;
                a.l1_signers = b.l1_signers;
            }
            if b.l1_approvals_required > a.l1_approvals_required {
                a.l1_approvals_required = b.l1_approvals_required;
            }
            if b.transition_approvals_collected > a.transition_approvals_collected {
                a.transition_approvals_collected = b.transition_approvals_collected;
                a.transition_signers = b.transition_signers;
            }
            if b.transition_approvals_required > a.transition_approvals_required {
                a.transition_approvals_required = b.transition_approvals_required;
            }
            if a.l1_tx_hash.is_none() && b.l1_tx_hash.is_some() {
                a.l1_tx_hash = b.l1_tx_hash;
                a.l1_tx_status = b.l1_tx_status;
                a.l1_confirmed_block = b.l1_confirmed_block;
            }
            if a.voting_activation_height.is_none() && b.voting_activation_height.is_some() {
                a.voting_activation_height = b.voting_activation_height;
            }
            if a.first_finalized_vote_height.is_none() && b.first_finalized_vote_height.is_some() {
                a.first_finalized_vote_height = b.first_finalized_vote_height;
                a.stage = b.stage;
                a.stage_reason = b.stage_reason;
            }
            for warn in b.missing_signer_setup {
                if !a.missing_signer_setup.contains(&warn) {
                    a.missing_signer_setup.push(warn);
                }
            }
            Some(a)
        }
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

async fn authorize(
    State(state): State<Enrollment>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    let host = state.origin.trim_start_matches("http://");
    if headers.get(header::HOST).and_then(|v| v.to_str().ok()) != Some(host) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request.uri().path().starts_with("/api/")
        && (headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
            != Some(state.origin.as_str())
            || headers.get("x-creg-session").and_then(|v| v.to_str().ok())
                != Some(state.session.as_str()))
    {
        return api_error(
            StatusCode::FORBIDDEN,
            "Reopen onboarding from CREG to start a new session",
        )
        .into_response();
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("content-security-policy", HeaderValue::from_static(
        "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"
    ));
    response
}

async fn config(State(state): State<Enrollment>) -> Result<Json<BrowserConfig>, ApiError> {
    state.network_stats().await.map_err(|_| {
        api_error(
            StatusCode::BAD_GATEWAY,
            "Could not verify the selected node is on R7. Check its URL and try again.",
        )
    })?;
    Ok(Json(state.config))
}

async fn status(
    State(state): State<Enrollment>,
    Json(wallet): Json<Wallet>,
) -> Result<Json<EnrollmentStatus>, ApiError> {
    let identity = state.identity(normalize_address(&wallet.address)?);
    let unavailable = || {
        api_error(
            StatusCode::BAD_GATEWAY,
            "Could not refresh R7 enrollment status",
        )
    };
    let (stats, registrations) = tokio::join!(
        state.network_stats(),
        state.get::<Vec<Registration>>(&state.network, "/v1/validator/registrations"),
    );
    let stats = stats.map_err(|_| unavailable())?;
    let registrations = registrations.map_err(|_| unavailable())?;
    let found = registrations
        .iter()
        .find(|record| record.identity.normalized() == identity);
    let registered = found.is_some_and(|record| record.registered_with_node);
    let admitted = found.is_some_and(|record| record.admitted_to_consensus && record.active);
    let runtime = match state.launcher.as_ref().filter(|_| registered) {
        Some(launcher) => launcher.runtime(&state.config).await,
        None => None,
    };
    let auto_detect = state
        .launcher
        .as_ref()
        .is_some_and(|launcher| launcher.http_port.is_none());
    let docker_origin = runtime
        .as_ref()
        .filter(|_| auto_detect)
        .and_then(|runtime| runtime.port)
        .and_then(|port| Url::parse(&format!("http://127.0.0.1:{port}")).ok());
    let origin = docker_origin.as_ref().unwrap_or(&state.local_node);
    let mut local_node = state
        .local_status(origin, stats.finalized_height, admitted)
        .await;
    // An unrelated node on the legacy default port does not prevent Docker
    // from assigning a free port for this identity.
    if auto_detect && docker_origin.is_none() && local_node == LOCAL_NODE_MISMATCH {
        local_node = LOCAL_NODE_UNAVAILABLE;
    }
    let launch_error = match &state.launcher {
        Some(launcher)
            if local_node == LOCAL_NODE_UNAVAILABLE
                && runtime.is_some_and(|runtime| runtime.failed) =>
        {
            Some(launcher.runtime_error(&state.config).await)
        }
        _ => None,
    };
    let local_reachable =
        local_node != LOCAL_NODE_UNAVAILABLE && local_node != LOCAL_NODE_MISMATCH;
    let (net_admission_res, local_admission_res) = tokio::join!(
        state.fetch_admission_report(&state.network, Some(&identity.evm_address), None),
        async {
            if local_reachable {
                Some(
                    state
                        .fetch_admission_report(origin, Some(&identity.evm_address), None)
                        .await,
                )
            } else {
                None
            }
        }
    );

    let mut admission_unavailable_reason = None;
    let mut signer_warnings = Vec::new();
    let net_view = match &net_admission_res {
        Ok(report) => {
            if report.local_node.is_validator && !report.local_node.admission_signer_configured {
                if let Some(issue) = &report.local_node.admission_signer_issue {
                    signer_warnings.push(issue.clone());
                }
            }
            report.applicants.first().cloned()
        }
        Err(Some(StatusCode::NOT_FOUND)) => {
            admission_unavailable_reason = Some(
                "Selected R7 API does not yet expose automatic admission telemetry (/v1/public/validator/admission-status returned 404); validator fleet rollout is pending"
                    .to_string(),
            );
            None
        }
        Err(_) => None,
    };
    let local_view = match &local_admission_res {
        Some(Ok(report)) => report.applicants.first().cloned(),
        _ => None,
    };
    let admission = merge_admission_progress(net_view, local_view);
    if let Some(view) = &admission {
        admission_unavailable_reason = None;
        for w in &view.missing_signer_setup {
            if !signer_warnings.contains(w) {
                signer_warnings.push(w.clone());
            }
        }
    }

    let should_observe_vote =
        admitted || admission.as_ref().is_some_and(|a| a.voting_activation_height.is_some());
    let vote_height = admission
        .as_ref()
        .and_then(|a| a.first_finalized_vote_height)
        .or(if should_observe_vote {
            state
                .observe_vote(local_reachable.then_some(origin))
                .await
        } else {
            None
        });
    let launch_status = *state.launch_status.lock().await;
    Ok(Json(EnrollmentStatus {
        registered,
        admitted,
        local_node,
        vote_height,
        launch_status,
        launch_error,
        admission,
        admission_unavailable_reason,
        signer_warnings,
    }))
}

async fn challenge(
    State(state): State<Enrollment>,
    Json(wallet): Json<Wallet>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = state.identity(normalize_address(&wallet.address)?);
    Ok(Json(
        serde_json::json!({ "message": state.challenge(&identity) }),
    ))
}

async fn register(
    State(state): State<Enrollment>,
    Json(proof): Json<WalletProof>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let identity = state.identity(normalize_address(&proof.address)?);
    let signature = proof.signature.strip_prefix("0x").unwrap_or_default();
    if signature.len() != 130 || !signature.bytes().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "The wallet did not return a valid Ethereum signature",
        ));
    }
    let unavailable = || {
        api_error(
            StatusCode::BAD_GATEWAY,
            "Could not submit identity to the selected R7 node",
        )
    };
    state.network_stats().await.map_err(|_| unavailable())?;
    let ed_signature = state
        .signing_key
        .sign(state.challenge(&identity).as_bytes());
    let response = state
        .client
        .post(
            state
                .network
                .join("/v1/validator/register")
                .map_err(|_| unavailable())?,
        )
        .json(&serde_json::json!({
            "evm_address": identity.evm_address,
            "node_id": identity.node_id,
            "ed25519_pubkey": identity.ed25519_pubkey,
            "nonce": state.nonce,
            "evm_signature": proof.signature,
            "ed25519_signature": hex::encode(ed_signature.to_bytes())
        }))
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() {
        let code = response.status();
        let detail = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|body| {
                body.get("error")?
                    .as_str()
                    .map(|s| s.chars().take(360).collect::<String>())
            });
        return Err(api_error(
            code,
            detail.unwrap_or_else(|| format!("R7 registration returned HTTP {}", code.as_u16())),
        ));
    }
    Ok(Json(serde_json::json!({ "registered": true })))
}

async fn start_node(
    State(state): State<Enrollment>,
    Json(wallet): Json<Wallet>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let launcher = state.launcher.as_ref().ok_or_else(|| {
        api_error(
            StatusCode::CONFLICT,
            "Open onboarding from the CREG checkout to launch a local node",
        )
    })?;
    let identity = state.identity(normalize_address(&wallet.address)?);
    state.network_stats().await.map_err(|_| {
        api_error(
            StatusCode::BAD_GATEWAY,
            "Could not verify the selected R7 node",
        )
    })?;
    let registrations: Vec<Registration> = state
        .get(&state.network, "/v1/validator/registrations")
        .await
        .map_err(|_| {
            api_error(
                StatusCode::BAD_GATEWAY,
                "Could not verify validator registration",
            )
        })?;
    if !registrations
        .iter()
        .any(|record| record.registered_with_node && record.identity.normalized() == identity)
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "Register this validator identity before starting the local node",
        ));
    }
    let mut launch_status = state.launch_status.lock().await;
    if *launch_status == "starting" {
        return Ok(Json(serde_json::json!({ "starting": true })));
    }
    let mut child = launcher.start(&state.config).map_err(|_| {
        api_error(StatusCode::INTERNAL_SERVER_ERROR, "Could not execute the R7 launcher. Check the configured Docker and PowerShell or Bash installation.")
    })?;
    *launch_status = "starting";
    drop(launch_status);
    tokio::spawn(async move {
        let successful = child.wait().await.is_ok_and(|status| status.success());
        *state.launch_status.lock().await = if successful { "started" } else { "failed" };
    });
    Ok(Json(serde_json::json!({ "starting": true })))
}

fn app(state: Enrollment) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("validator_onboarding.html")) }),
        )
        .route(
            "/onboarding.js",
            get(|| async {
                (
                    [(
                        header::CONTENT_TYPE,
                        "application/javascript; charset=utf-8",
                    )],
                    include_str!("validator_onboarding.js"),
                )
            }),
        )
        .route(
            "/onboarding.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("validator_onboarding.css"),
                )
            }),
        )
        .route("/api/config", post(config))
        .route("/api/status", post(status))
        .route("/api/challenge", post(challenge))
        .route("/api/register", post(register))
        .route("/api/start", post(start_node))
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
        .with_state(state)
}

pub async fn run(
    node_id: Option<String>,
    key_file: Option<PathBuf>,
    node_url: Option<String>,
    local_node_url: Option<String>,
    no_browser: bool,
) -> Result<()> {
    let storage_id = node_id.as_deref().unwrap_or(DEFAULT_NODE_ID);
    validate_node_id(storage_id)?;
    let network = node_origin(node_url.as_deref().unwrap_or(R7_API), false)?;
    let local_node = node_origin(local_node_url.as_deref().unwrap_or(LOCAL_NODE), true)?;
    let key_path = match &key_file {
        Some(path) => path.clone(),
        None if node_id.is_none() => runtime_key_path(storage_id)?.with_file_name("onboarding.key"),
        None => runtime_key_path(storage_id)?,
    };
    let key = runtime_key(&key_path, key_file.is_none())?;
    let node_id = node_id.unwrap_or_else(|| {
        format!(
            "ext-val-{}",
            hex::encode(&key.verifying_key().as_bytes()[..10])
        )
    });
    let mut config = browser_config(node_id, &key)?;
    let http_port = local_node_url
        .as_ref()
        .and_then(|_| local_node.port_or_known_default());
    let launcher = local_launcher(&config.node_id, &key_path, http_port);
    config.launch_log = launcher
        .as_ref()
        .map(|launcher| launcher.log_file.display().to_string());
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let state = Enrollment {
        signing_key: Arc::new(key),
        client: Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(8))
            .build()?,
        network,
        local_node,
        config,
        origin: format!("http://{}", listener.local_addr()?),
        session: hex::encode(rand::random::<[u8; 32]>()),
        nonce: hex::encode(rand::random::<[u8; 16]>()),
        vote_height: Arc::new(Mutex::new(None)),
        launcher,
        launch_status: Arc::new(Mutex::new("idle")),
    };
    let url = format!("{}/#{}", state.origin, state.session);
    if !no_browser {
        if let Err(error) = crate::auth::open_browser(&url) {
            eprintln!("Could not open the browser: {error}. Open the local link below.");
        }
    }
    println!("Validator onboarding: {url}");
    println!("Local validator key: {}", key_path.display());
    println!("Keep this process open while using the onboarding page. The session expires in 60 minutes.");
    axum::serve(listener, app(state))
        .with_graceful_shutdown(async {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(3600)) => {},
                _ = tokio::signal::ctrl_c() => {},
            }
        })
        .await?;
    Ok(())
}

pub async fn show_status(
    node_url: Option<&str>,
    local_node_url: Option<&str>,
    applicant: Option<&str>,
    node_id: Option<&str>,
    json: bool,
) -> Result<()> {
    let network = node_origin(node_url.unwrap_or(R7_API), false)?;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .build()?;
    let mut url = network.join("/v1/public/validator/admission-status")?;
    {
        let mut pairs = url.query_pairs_mut();
        if let Some(addr) = applicant.map(str::trim).filter(|v| !v.is_empty()) {
            pairs.append_pair("applicant", addr);
        }
        if let Some(nid) = node_id.map(str::trim).filter(|v| !v.is_empty()) {
            pairs.append_pair("node_id", nid);
        }
    }

    let local_health = if let Some(local_raw) = local_node_url {
        let local_url = node_origin(local_raw, true)?;
        client
            .get(local_url.join("/v1/health")?)
            .send()
            .await
            .ok()
            .and_then(|r| if r.status().is_success() { Some(r) } else { None })
    } else {
        None
    };
    let local_health_json: Option<serde_json::Value> = match local_health {
        Some(resp) => resp.json().await.ok(),
        None => None,
    };

    let resp = client.get(url).send().await?;
    if resp.status() == StatusCode::NOT_FOUND {
        let stats: serde_json::Value = client
            .get(network.join("/v1/public/chain/stats")?)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let transitions: Vec<serde_json::Value> = client
            .get(network.join("/v1/public/validator-set-transitions")?)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut regs: Vec<serde_json::Value> = client
            .get(network.join("/v1/validator/registrations")?)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if let Some(addr) = applicant.map(str::trim).filter(|v| !v.is_empty()) {
            let needle = addr.to_ascii_lowercase();
            regs.retain(|r| {
                r["identity"]["evm_address"]
                    .as_str()
                    .is_some_and(|v| v.eq_ignore_ascii_case(&needle))
            });
        }
        if let Some(nid) = node_id.map(str::trim).filter(|v| !v.is_empty()) {
            regs.retain(|r| r["identity"]["node_id"].as_str() == Some(nid));
        }

        let payload = serde_json::json!({
            "node_url": network.as_str(),
            "telemetry_available": false,
            "unavailable_reason": "Queried node does not yet expose /v1/public/validator/admission-status (HTTP 404); automatic admission rollout pending.",
            "stats": stats,
            "certified_transitions": transitions.len(),
            "registrations": regs,
            "local_health": local_health_json,
        });
        if json {
            println!("{}", serde_json::to_string_pretty(&payload)?);
        } else {
            println!(
                "Validator admission status ({})",
                network.as_str().trim_end_matches('/')
            );
            println!(
                "  Blocker:    {} (HTTP 404 on /v1/public/validator/admission-status)",
                payload["unavailable_reason"].as_str().unwrap_or_default()
            );
            println!(
                "  Chain:      {} | tip #{} | finalized #{} | active validators: {} | certified transitions: {}",
                stats["chain_id"].as_str().unwrap_or("unknown"),
                stats["tip_height"].as_u64().unwrap_or(0),
                stats["finalized_height"].as_u64().unwrap_or(0),
                stats["active_validators"].as_u64().unwrap_or(0),
                transitions.len()
            );
            if let Some(lh) = &local_health_json {
                println!(
                    "  Local node: {} | is_validator={} | lag_blocks={}",
                    lh["node_id"].as_str().unwrap_or("unknown"),
                    lh["is_validator"].as_bool().unwrap_or(false),
                    lh["sync"]["lag_blocks"].as_u64().unwrap_or(0),
                );
            }
            for r in regs {
                println!(
                    "  Applicant:  {} ({}) | staking={} | stake={} | admitted={} | active={}",
                    r["identity"]["node_id"].as_str().unwrap_or("unknown"),
                    r["identity"]["evm_address"].as_str().unwrap_or("unknown"),
                    r["staking_state"].as_str().unwrap_or("unknown"),
                    r["stake"].as_u64().unwrap_or(0),
                    r["admitted_to_consensus"].as_bool().unwrap_or(false),
                    r["active"].as_bool().unwrap_or(false),
                );
            }
        }
        return Ok(());
    }

    let report: AdmissionStatusReport = resp.error_for_status()?.json().await?;
    if json {
        let out = serde_json::json!({
            "node_url": network.as_str(),
            "telemetry_available": true,
            "report": report,
            "local_health": local_health_json,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!(
        "Validator admission status ({})",
        network.as_str().trim_end_matches('/')
    );
    println!(
        "  Chain:      {} | tip #{} | finalized #{} | active validators: {} | L1 epoch: {:?}",
        report.network.chain_id,
        report.network.tip_height,
        report.network.finalized_height,
        report.network.active_validators,
        report.network.validator_set_epoch
    );
    if let Some(err) = &report.network.rpc_error {
        println!("  L1 RPC err: {err}");
    }
    println!(
        "  Node signer: node={} is_validator={} configured={} active_on_l1={} has_gas={}",
        report.local_node.node_id,
        report.local_node.is_validator,
        report.local_node.admission_signer_configured,
        report.local_node.admission_signer_active_on_l1,
        report.local_node.admission_signer_has_gas
    );
    if let Some(issue) = &report.local_node.admission_signer_issue {
        println!("  Signer note: {issue}");
    }
    if let Some(lh) = &local_health_json {
        println!(
            "  Local node: {} | is_validator={} | lag_blocks={}",
            lh["node_id"].as_str().unwrap_or("unknown"),
            lh["is_validator"].as_bool().unwrap_or(false),
            lh["sync"]["lag_blocks"].as_u64().unwrap_or(0),
        );
    }
    if report.applicants.is_empty() {
        println!("  Applicants: none matching filter");
    }
    for app in &report.applicants {
        println!("\n  Applicant {} ({})", app.node_id, app.applicant);
        println!("    Stage:             {} — {}", app.stage, app.stage_reason);
        println!("    L1 staking:        {} ({} tCREG)", app.staking_state, app.stake_creg);
        println!(
            "    Sepolia approvals: {}/{}",
            app.l1_approvals_collected, app.l1_approvals_required
        );
        println!(
            "    Sepolia tx:        {} {}",
            app.l1_tx_status,
            app.l1_tx_hash.as_deref().unwrap_or("-")
        );
        if let Some(b) = app.l1_confirmed_block {
            println!("    Sepolia block:     #{b}");
        }
        println!(
            "    Transition sigs:   {}/{}",
            app.transition_approvals_collected, app.transition_approvals_required
        );
        println!(
            "    Activation height: {}",
            app.voting_activation_height
                .map(|h| format!("#{h}"))
                .unwrap_or_else(|| "pending".to_string())
        );
        println!(
            "    First final vote:  {}",
            app.first_finalized_vote_height
                .map(|h| format!("#{h}"))
                .unwrap_or_else(|| "pending".to_string())
        );
        if !app.missing_signer_setup.is_empty() {
            println!(
                "    Signer setup:      {}",
                app.missing_signer_setup.join("; ")
            );
        }
        if app.timings.signature_collection_ms.is_some()
            || app.timings.l1_confirmation_ms.is_some()
            || app.timings.voting_activation_ms.is_some()
            || app.timings.first_finalized_vote_ms.is_some()
        {
            println!(
                "    Timings (ms):      sig={:?} l1_conf={:?} activation={:?} first_vote={:?}",
                app.timings.signature_collection_ms,
                app.timings.l1_confirmation_ms,
                app.timings.voting_activation_ms,
                app.timings.first_finalized_vote_ms
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;
    use serde_json::{json, Value};

    #[test]
    fn discovers_only_the_matching_docker_loopback_port() {
        let label = "r7/registered-key";
        let runtime =
            ManagedRuntime::parse("r7/registered-key running 0 0 127.0.0.1 49155", label).unwrap();
        assert_eq!(runtime.port, Some(49155));
        assert!(!runtime.failed);
        assert!(ManagedRuntime::parse("r7/other-key running 0 0 127.0.0.1 8080", label).is_none());
        let runtime =
            ManagedRuntime::parse("r7/registered-key running 0 0 0.0.0.0 8080", label).unwrap();
        assert_eq!(runtime.port, None);
    }

    #[test]
    fn detects_startup_failure_after_a_detached_docker_launch() {
        let label = "r7/registered-key";
        for state in ["restarting 1 3", "exited 1 0", "running 0 1"] {
            let runtime = ManagedRuntime::parse(&format!("{label} {state}"), label).unwrap();
            assert!(runtime.failed, "{state}");
            assert_eq!(runtime.port, None);
        }
        assert!(
            !ManagedRuntime::parse("r7/registered-key exited 0 0", label)
                .unwrap()
                .failed
        );
    }

    #[tokio::test]
    async fn registers_both_proofs_and_refreshes_r7_status() {
        let key = SigningKey::from_bytes(&[12; 32]);
        let config = browser_config("fixture-validator".into(), &key).unwrap();
        assert_eq!(config.l1_chain_id, 11155111);
        assert_eq!(config.approve, "0x095ea7b3");
        let wallet_key = k256::ecdsa::SigningKey::from_slice(&[16; 32]).unwrap();
        let wallet_point = wallet_key.verifying_key().to_encoded_point(false);
        let wallet_hash = Keccak256::digest(&wallet_point.as_bytes()[1..]);
        let address = format!("0x{}", hex::encode(&wallet_hash[12..]));
        let config_for_mock = config.clone();
        let captured_proof = Arc::new(Mutex::new(None::<Value>));
        let received_proof = captured_proof.clone();
        let registrations = captured_proof.clone();
        let registered_address = address.clone();
        let node = Router::new()
            .route("/v1/public/chain/stats", get(move || {
                let id = config_for_mock.chain_id.clone();
                async move { Json(json!({
                    "chain_id": id, "spec_signature_verified": true, "finalized_height": 25,
                    "peer_count": 2, "validator_set_sync_state": "synced"
                })) }
            }))
            .route("/v1/validator/register", post(move |Json(proof): Json<Value>| {
                let captured = received_proof.clone();
                async move {
                    *captured.lock().await = Some(proof);
                    StatusCode::ACCEPTED
                }
            }))
            .route("/v1/validator/registrations", get(move || {
                let registrations = registrations.clone();
                let registered_address = registered_address.clone();
                async move {
                    let proof = registrations.lock().await;
                    Json(match &*proof {
                        Some(proof) => json!([{
                            "identity": { "evm_address": registered_address, "node_id": proof["node_id"], "ed25519_pubkey": proof["ed25519_pubkey"] },
                            "registered_with_node": true
                        }]),
                        None => json!([]),
                    })
                }
            }))
            .route("/v1/health", get(|| async {
                Json(json!({
                    "node_id": "fixture-validator", "is_validator": false,
                    "sandbox": { "isolated": true, "dev_bypass": false, "degraded": false },
                    "sync": { "lag_blocks": 0 }
                }))
            }));
        let mock = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}", mock.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(mock, node).await.unwrap() });
        let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", http.local_addr().unwrap());
        let state = Enrollment {
            signing_key: Arc::new(key.clone()),
            client: Client::new(),
            network: url.clone(),
            local_node: url,
            config,
            origin: origin.clone(),
            session: "fixture-session".into(),
            nonce: "fixture-challenge-nonce".into(),
            vote_height: Arc::new(Mutex::new(None)),
            launcher: None,
            launch_status: Arc::new(Mutex::new("idle")),
        };
        let router = app(state);
        let enrollment = tokio::spawn(async move { axum::serve(http, router).await.unwrap() });
        let client = Client::new();
        let post = |endpoint: &str, data: Value| {
            client
                .post(format!("{origin}/api/{endpoint}"))
                .header(header::ORIGIN, &origin)
                .header("x-creg-session", "fixture-session")
                .json(&data)
        };
        let unauthorized = client
            .post(format!("{origin}/api/config"))
            .header(header::ORIGIN, "http://other.test")
            .header("x-creg-session", "fixture-session")
            .send()
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::FORBIDDEN);
        let challenge: Value = post("challenge", json!({ "address": address }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let message = challenge["message"].as_str().unwrap();
        let digest = Keccak256::digest(format!(
            "\x19Ethereum Signed Message:\n{}{message}",
            message.len()
        ));
        let (signature, recovery_id) = wallet_key.sign_prehash_recoverable(&digest).unwrap();
        let signature_hex = format!(
            "0x{}{:02x}",
            hex::encode(signature.to_bytes()),
            recovery_id.to_byte() + 27
        );
        post(
            "register",
            json!({ "address": address, "signature": &signature_hex }),
        )
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
        let received: Value = post("status", json!({ "address": address }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(received["registered"], true);
        assert_eq!(received["admitted"], false);
        assert_eq!(
            received["local_node"],
            "Observer synced, awaiting admission"
        );
        assert!(received["vote_height"].is_null());
        assert!(received["admission"].is_null());
        assert!(received["admission_unavailable_reason"]
            .as_str()
            .unwrap()
            .contains("404"));
        // Inspect the same recording used by the mock registration status.
        // The node receives the wallet proof and an Ed25519 signature over the exact challenge.
        let proof = captured_proof.lock().await.clone().unwrap();
        assert_eq!(proof["evm_address"], address);
        assert_eq!(proof["evm_signature"], signature_hex);
        assert_eq!(proof["nonce"], "fixture-challenge-nonce");
        let ed_bytes = hex::decode(proof["ed25519_signature"].as_str().unwrap()).unwrap();
        let ed_signature = ed25519_dalek::Signature::from_slice(&ed_bytes).unwrap();
        key.verifying_key()
            .verify(message.as_bytes(), &ed_signature)
            .unwrap();
        enrollment.abort();
        server.abort();
    }

    #[tokio::test]
    async fn status_surfaces_admission_progress_approvals_tx_activation_and_first_vote() {
        let key = SigningKey::from_bytes(&[22; 32]);
        let config = browser_config("fixture-validator".into(), &key).unwrap();
        let wallet_key = k256::ecdsa::SigningKey::from_slice(&[24; 32]).unwrap();
        let wallet_point = wallet_key.verifying_key().to_encoded_point(false);
        let wallet_hash = Keccak256::digest(&wallet_point.as_bytes()[1..]);
        let address = format!("0x{}", hex::encode(&wallet_hash[12..]));
        let pubkey = config.public_key.clone();
        let pubkey_for_adm = config.public_key.clone();
        let chain_id = config.chain_id.clone();
        let registered_address = address.clone();
        let node = Router::new()
            .route(
                "/v1/public/chain/stats",
                get(move || {
                    let id = chain_id.clone();
                    async move {
                        Json(json!({
                            "chain_id": id,
                            "spec_signature_verified": true,
                            "finalized_height": 23,
                            "peer_count": 3,
                            "validator_set_sync_state": "synced"
                        }))
                    }
                }),
            )
            .route(
                "/v1/validator/registrations",
                get(move || {
                    let registered_address = registered_address.clone();
                    let pubkey = pubkey.clone();
                    async move {
                        Json(json!([{
                            "identity": {
                                "evm_address": registered_address,
                                "node_id": "fixture-validator",
                                "ed25519_pubkey": pubkey
                            },
                            "registered_with_node": true,
                            "admitted_to_consensus": false,
                            "active": false
                        }]))
                    }
                }),
            )
            .route(
                "/v1/health",
                get(|| async {
                    Json(json!({
                        "node_id": "fixture-validator",
                        "is_validator": false,
                        "sandbox": { "isolated": true, "dev_bypass": false, "degraded": false },
                        "sync": { "lag_blocks": 0 }
                    }))
                }),
            )
            .route(
                "/v1/public/validator/admission-status",
                get(move || {
                    let pubkey_for_adm = pubkey_for_adm.clone();
                    async move {
                        Json(json!({
                        "local_node": {
                            "node_id": "v1",
                            "is_validator": true,
                            "admission_signer_configured": false,
                            "admission_signer_issue": "Validator admission signer missing on v3"
                        },
                        "network": {
                            "chain_id": "creg-phase-c-testnet-2026-09-26-r7",
                            "tip_height": 23,
                            "finalized_height": 23,
                            "active_validators": 4,
                            "required_l1_approvals": 3,
                            "required_transition_approvals": 3
                        },
                        "applicants": [{
                            "applicant": address,
                            "node_id": "fixture-validator",
                            "ed25519_pubkey": pubkey_for_adm,
                            "stage": "active_voting",
                            "stage_reason": "Certified at block #22; first finalized vote recorded at block #23",
                            "staking_state": "active",
                            "stake_creg": 100,
                            "l1_approvals_collected": 3,
                            "l1_approvals_required": 3,
                            "l1_tx_hash": "0xabc123",
                            "l1_tx_status": "confirmed",
                            "l1_confirmed_block": 999,
                            "transition_approvals_collected": 3,
                            "transition_approvals_required": 3,
                            "missing_signer_setup": ["v4 missing CREG_VALIDATOR_ADMISSION_KEY"],
                            "voting_activation_height": 22,
                            "first_finalized_vote_height": 23
                        }]
                    }))
                    }
                }),
            );

        let mock = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}", mock.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(mock, node).await.unwrap() });
        let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", http.local_addr().unwrap());
        let state = Enrollment {
            signing_key: Arc::new(key),
            client: Client::new(),
            network: url.clone(),
            local_node: url,
            config,
            origin: origin.clone(),
            session: "fixture-session".into(),
            nonce: "fixture-nonce".into(),
            vote_height: Arc::new(Mutex::new(None)),
            launcher: None,
            launch_status: Arc::new(Mutex::new("idle")),
        };
        let router = app(state);
        let enrollment = tokio::spawn(async move { axum::serve(http, router).await.unwrap() });

        let received: Value = Client::new()
            .post(format!("{origin}/api/status"))
            .header(header::ORIGIN, &origin)
            .header("x-creg-session", "fixture-session")
            .json(&json!({ "address": format!("0x{}", hex::encode(&wallet_hash[12..])) }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(received["vote_height"], 23);
        assert!(received["admission_unavailable_reason"].is_null());
        assert_eq!(received["admission"]["l1_approvals_collected"], 3);
        assert_eq!(received["admission"]["l1_approvals_required"], 3);
        assert_eq!(received["admission"]["l1_tx_hash"], "0xabc123");
        assert_eq!(received["admission"]["voting_activation_height"], 22);
        assert_eq!(received["admission"]["first_finalized_vote_height"], 23);
        assert!(!received["signer_warnings"].as_array().unwrap().is_empty());

        enrollment.abort();
        server.abort();
    }
}
