//! Single-Command Validator Four-Zone Supervisor (`creg-node-supervisor`, `ADR-018` Increment 6).
//!
//! Spawns `creg-sandboxd` (Zone 3), `creg-agent-runner` (Zone 4), `creg-consensusd` (Zone 2),
//! and `creg-gateway` (Zone 1) with `.env_clear().envs(...)` strict per-zone allowlists,
//! per-boot random `ZoneIpcToken`, and OS child-death containment (`PR_SET_PDEATHSIG` on Linux,
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` Job Object on Windows).

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use zone_ipc::{
    allowed_inbound_callers, allowed_outbound_targets, link_key_env_var, OperatorIpcKeyBundleV1,
    OperatorZoneEndpointKeyV1, ZoneId, ZoneIpcToken, ZONE_IPC_TOKEN_ENV,
};

#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub data_dir: String,
    pub chain_spec: String,
    pub node_id: String,
    pub p2p_listen: String,
    pub gateway_public_listen: String,
    pub gateway_admin_listen: String,
    pub gateway_ipc_addr: String,
    pub consensus_ipc_addr: String,
    pub sandbox_ipc_addr: String,
    pub agent_ipc_addr: String,
    pub operator_key_file: PathBuf,
    pub ipc_token_hex: String,
    pub admin_token: String,
}

impl SupervisorConfig {
    pub fn from_parent_env(parent: &BTreeMap<String, String>) -> Self {
        let get = |k: &str, default: &str| {
            parent
                .get(k)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        let ipc_token_hex = parent
            .get(ZONE_IPC_TOKEN_ENV)
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| ZoneIpcToken::generate().to_hex());
        let admin_token = parent
            .get("CREG_GATEWAY_ADMIN_TOKEN")
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| ZoneIpcToken::generate().to_hex());
        let data_dir = get("CREG_DATA_DIR", "/var/lib/creg");
        let operator_key_file = parent
            .get(zone_ipc::OPERATOR_KEY_FILE_ENV)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&data_dir).join("operator-ipc-keys.json"));

        Self {
            data_dir,
            chain_spec: get("CREG_CHAIN_SPEC", "/etc/creg/chain-spec.json"),
            node_id: get("CREG_NODE_ID", "validator-01"),
            p2p_listen: get("CREG_P2P_LISTEN", "0.0.0.0:9000"),
            gateway_public_listen: get("CREG_GATEWAY_PUBLIC_LISTEN", "0.0.0.0:8080"),
            gateway_admin_listen: get("CREG_GATEWAY_ADMIN_LISTEN", "127.0.0.1:18080"),
            gateway_ipc_addr: get("CREG_GATEWAY_IPC_ADDR", "127.0.0.1:19001"),
            consensus_ipc_addr: get("CREG_CONSENSUS_IPC_ADDR", "127.0.0.1:19002"),
            sandbox_ipc_addr: get("CREG_SANDBOX_IPC_ADDR", "127.0.0.1:19003"),
            agent_ipc_addr: get("CREG_AGENT_IPC_ADDR", "127.0.0.1:19004"),
            operator_key_file,
            ipc_token_hex,
            admin_token,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneChildEnvs {
    pub gateway: BTreeMap<String, String>,
    pub consensusd: BTreeMap<String, String>,
    pub sandboxd: BTreeMap<String, String>,
    pub agent_runner: BTreeMap<String, String>,
}

fn copy_allowlisted(
    parent_env: &BTreeMap<String, String>,
    dest: &mut BTreeMap<String, String>,
    allowlist: &[&str],
) {
    for &key in allowlist {
        if let Some(val) = parent_env.get(key) {
            if !val.is_empty() {
                dest.insert(key.to_string(), val.clone());
            }
        }
    }
}

fn inject_zone_link_keys(zone: ZoneId, master: &ZoneIpcToken, env: &mut BTreeMap<String, String>) {
    for &caller in allowed_inbound_callers(zone) {
        env.insert(
            link_key_env_var(caller, zone),
            master.derive_link_key(caller, zone).to_hex(),
        );
    }
    for &target in allowed_outbound_targets(zone) {
        env.insert(
            link_key_env_var(zone, target),
            master.derive_link_key(zone, target).to_hex(),
        );
    }
}

/// Build the owner-only operator key bundle holding ONLY `ZoneId::Operator -> Zone`
/// derived link keys (`ADR-019` §4.3, `ECA-04`).
pub fn build_operator_key_bundle(cfg: &SupervisorConfig) -> Result<OperatorIpcKeyBundleV1> {
    let master = ZoneIpcToken::from_hex(&cfg.ipc_token_hex)
        .context("parse supervisor master zone-ipc token")?;
    Ok(OperatorIpcKeyBundleV1 {
        schema_version: 1,
        gateway: OperatorZoneEndpointKeyV1 {
            endpoint: cfg.gateway_ipc_addr.clone(),
            link_key_hex: master
                .derive_link_key(ZoneId::Operator, ZoneId::Gateway)
                .to_hex(),
        },
        consensus: OperatorZoneEndpointKeyV1 {
            endpoint: cfg.consensus_ipc_addr.clone(),
            link_key_hex: master
                .derive_link_key(ZoneId::Operator, ZoneId::Consensus)
                .to_hex(),
        },
        sandbox: OperatorZoneEndpointKeyV1 {
            endpoint: cfg.sandbox_ipc_addr.clone(),
            link_key_hex: master
                .derive_link_key(ZoneId::Operator, ZoneId::Sandbox)
                .to_hex(),
        },
        agent_runner: OperatorZoneEndpointKeyV1 {
            endpoint: cfg.agent_ipc_addr.clone(),
            link_key_hex: master
                .derive_link_key(ZoneId::Operator, ZoneId::AgentRunner)
                .to_hex(),
        },
    })
}

/// Strictly construct isolated child environments (`env_clear()` + allowlist)
/// so no cross-zone secrets ever leak across process boundaries (`INV-01`), and
/// each child receives ONLY its own HKDF-SHA256 per-link keys (`ECA-04`).
pub fn build_zone_child_envs(
    parent_env: &BTreeMap<String, String>,
    cfg: &SupervisorConfig,
) -> Result<ZoneChildEnvs> {
    let master = ZoneIpcToken::from_hex(&cfg.ipc_token_hex)
        .context("parse supervisor master zone-ipc token")?;

    let mut gateway = BTreeMap::new();
    copy_allowlisted(
        parent_env,
        &mut gateway,
        &["RUST_LOG", "CREG_GATEWAY_RATE_LIMIT_MAX"],
    );
    gateway.insert(
        "CREG_GATEWAY_PUBLIC_LISTEN".into(),
        cfg.gateway_public_listen.clone(),
    );
    gateway.insert(
        "CREG_GATEWAY_ADMIN_LISTEN".into(),
        cfg.gateway_admin_listen.clone(),
    );
    gateway.insert("CREG_GATEWAY_ADMIN_TOKEN".into(), cfg.admin_token.clone());
    gateway.insert("CREG_GATEWAY_IPC_ADDR".into(), cfg.gateway_ipc_addr.clone());
    gateway.insert(
        "CREG_CONSENSUS_IPC_ADDR".into(),
        cfg.consensus_ipc_addr.clone(),
    );
    inject_zone_link_keys(ZoneId::Gateway, &master, &mut gateway);

    let mut consensusd = BTreeMap::new();
    copy_allowlisted(
        parent_env,
        &mut consensusd,
        &[
            "RUST_LOG",
            "CREG_CHAIN_ID",
            "CREG_VALIDATOR_ADDRESS",
            // The names the node reads (`HotKeyRole::env_var`).
            "CREG_VALIDATOR_KEY",
            "CREG_VALIDATOR_PQ_KEY",
            "CREG_VALIDATOR_PRIVKEY_X25519",
            "CREG_BRIDGE_KEY",
            "CREG_GOVERNANCE_KEY",
            "CREG_L1_PRIVATE_KEY",
            "CREG_SECRETS_BACKEND",
            "CREG_VAULT_SECRET_PATH_VALIDATOR",
            "CREG_VAULT_SECRET_PATH_VALIDATOR_PQ",
            "CREG_VAULT_SECRET_FIELD",
            "VAULT_ADDR",
            "VAULT_TOKEN",
            "VAULT_NAMESPACE",
        ],
    );
    // Legacy deployments set `CREG_VALIDATOR_PRIVKEY`, which the node never
    // read; hand it over under the name it does read.
    if !consensusd.contains_key("CREG_VALIDATOR_KEY") {
        if let Some(legacy) = parent_env
            .get("CREG_VALIDATOR_PRIVKEY")
            .filter(|value| !value.is_empty())
        {
            consensusd.insert("CREG_VALIDATOR_KEY".into(), legacy.clone());
        }
    }
    consensusd.insert("CREG_DATA_DIR".into(), cfg.data_dir.clone());
    consensusd.insert("CREG_CHAIN_SPEC".into(), cfg.chain_spec.clone());
    consensusd.insert("CREG_NODE_ID".into(), cfg.node_id.clone());
    consensusd.insert("CREG_P2P_LISTEN".into(), cfg.p2p_listen.clone());
    consensusd.insert(
        "CREG_CONSENSUS_IPC_ADDR".into(),
        cfg.consensus_ipc_addr.clone(),
    );
    consensusd.insert("CREG_SANDBOX_IPC_ADDR".into(), cfg.sandbox_ipc_addr.clone());
    consensusd.insert("CREG_AGENT_IPC_ADDR".into(), cfg.agent_ipc_addr.clone());
    inject_zone_link_keys(ZoneId::Consensus, &master, &mut consensusd);

    let mut sandboxd = BTreeMap::new();
    copy_allowlisted(
        parent_env,
        &mut sandboxd,
        &[
            "RUST_LOG",
            "CREG_SANDBOX_ROOTFS",
            "CREG_SANDBOX_ENGINE",
            "CREG_FIRECRACKER_BIN",
            "CREG_JAILER_BIN",
            "CREG_NSJAIL_BIN",
            "CREG_DEV_SANDBOX",
            "CREG_TESTNET",
            "CREG_PUBLIC_VALIDATOR",
            "CREG_PRODUCTION",
        ],
    );
    sandboxd.insert("CREG_SANDBOX_IPC_ADDR".into(), cfg.sandbox_ipc_addr.clone());
    inject_zone_link_keys(ZoneId::Sandbox, &master, &mut sandboxd);

    let mut agent_runner = BTreeMap::new();
    copy_allowlisted(
        parent_env,
        &mut agent_runner,
        &[
            "RUST_LOG",
            "CREG_AI_CONFIG",
            "CREG_LLM_ENABLED",
            "CREG_AI_MODE",
            "CREG_LLM_PROVIDERS",
            "CREG_AI_API_KEY",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
            "OPENROUTER_API_KEY",
            "OLLAMA_HOST",
            // Lane C fetches committed tarballs itself (ADR-019 `ECA-03`).
            "CREG_IPFS_URL",
            "CREG_PRODUCTION",
        ],
    );
    agent_runner.insert("CREG_AGENT_IPC_ADDR".into(), cfg.agent_ipc_addr.clone());
    agent_runner.insert("CREG_SANDBOX_IPC_ADDR".into(), cfg.sandbox_ipc_addr.clone());
    inject_zone_link_keys(ZoneId::AgentRunner, &master, &mut agent_runner);

    Ok(ZoneChildEnvs {
        gateway,
        consensusd,
        sandboxd,
        agent_runner,
    })
}

pub struct ManagedChild {
    pub name: &'static str,
    pub child: Child,
    #[cfg(windows)]
    _job: WindowsJobGuard,
}

#[cfg(windows)]
struct WindowsJobGuard(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for WindowsJobGuard {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        if !self.0.is_null() {
            unsafe {
                let _ = TerminateJobObject(self.0, 1);
                let _ = CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn attach_child_death_containment(child: &Child) -> Result<WindowsJobGuard> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        anyhow::bail!("CreateJobObjectW failed while containing zone child");
    }
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &mut limits as *mut _ as *mut _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    } != 0;
    let assigned = configured
        && unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) } != 0;
    if !assigned {
        unsafe { CloseHandle(job) };
        anyhow::bail!("AssignProcessToJobObject failed while containing zone child");
    }
    Ok(WindowsJobGuard(job))
}

fn spawn_contained_zone(
    name: &'static str,
    bin_path: &Path,
    env_map: &BTreeMap<String, String>,
) -> Result<ManagedChild> {
    let mut cmd = Command::new(bin_path);
    cmd.env_clear()
        .envs(env_map)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn zone '{name}' from {}", bin_path.display()))?;

    #[cfg(windows)]
    let job = match attach_child_death_containment(&child) {
        Ok(job) => job,
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err);
        }
    };

    Ok(ManagedChild {
        name,
        child,
        #[cfg(windows)]
        _job: job,
    })
}

fn resolve_sibling_bin(name: &str) -> PathBuf {
    let exe_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join(&exe_name)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(exe_name))
}

fn main() -> Result<()> {
    let parent_env: BTreeMap<String, String> = std::env::vars().collect();
    let cfg = SupervisorConfig::from_parent_env(&parent_env);
    let operator_bundle = build_operator_key_bundle(&cfg)?;
    operator_bundle.write_owner_only(&cfg.operator_key_file)?;
    let envs = build_zone_child_envs(&parent_env, &cfg)?;

    let mut children = vec![
        spawn_contained_zone(
            "sandboxd",
            &resolve_sibling_bin("creg-sandboxd"),
            &envs.sandboxd,
        )?,
        spawn_contained_zone(
            "agent-runner",
            &resolve_sibling_bin("creg-agent-runner"),
            &envs.agent_runner,
        )?,
        spawn_contained_zone(
            "consensusd",
            &resolve_sibling_bin("creg-consensusd"),
            &envs.consensusd,
        )?,
        spawn_contained_zone(
            "gateway",
            &resolve_sibling_bin("creg-gateway"),
            &envs.gateway,
        )?,
    ];

    loop {
        for managed in &mut children {
            if let Some(status) = managed.child.try_wait()? {
                anyhow::bail!(
                    "zone child '{}' exited unexpectedly with {}",
                    managed.name,
                    status
                );
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zone_ipc::{
        check_forbidden_env_btree, ZoneId, AGENT_RUNNER_FORBIDDEN_ENV, CONSENSUS_FORBIDDEN_ENV,
        GATEWAY_FORBIDDEN_ENV, SANDBOX_FORBIDDEN_ENV,
    };

    #[test]
    fn supervisor_builds_strict_isolated_envs_with_zero_cross_zone_leakage() {
        let mut parent = BTreeMap::new();
        parent.insert("RUST_LOG".into(), "debug".into());
        parent.insert("CREG_VALIDATOR_PRIVKEY".into(), "11".repeat(32));
        parent.insert("CREG_VALIDATOR_PQ_KEY".into(), "22".repeat(32));
        parent.insert("CREG_L1_PRIVATE_KEY".into(), "33".repeat(32));
        parent.insert("VAULT_TOKEN".into(), "hvs.secret-vault".into());
        parent.insert("OPENAI_API_KEY".into(), "sk-openai".into());
        parent.insert("ANTHROPIC_API_KEY".into(), "sk-anthropic".into());
        parent.insert("GEMINI_API_KEY".into(), "AIza-gemini".into());
        parent.insert("CREG_AI_API_KEY".into(), "sk-custom-ai".into());
        parent.insert("CREG_SANDBOX_ENGINE".into(), "firecracker".into());
        parent.insert("UNRELATED_AMBIENT_SECRET".into(), "must-not-leak".into());

        let cfg = SupervisorConfig::from_parent_env(&parent);
        assert_eq!(cfg.ipc_token_hex.len(), 64);
        assert_eq!(cfg.admin_token.len(), 64);

        let envs = build_zone_child_envs(&parent, &cfg).unwrap();

        // Runtime INV-01 forbidden-env guards MUST pass for every single zone.
        assert!(
            check_forbidden_env_btree(ZoneId::Gateway, &envs.gateway, GATEWAY_FORBIDDEN_ENV)
                .is_ok()
        );
        assert!(check_forbidden_env_btree(
            ZoneId::Consensus,
            &envs.consensusd,
            CONSENSUS_FORBIDDEN_ENV
        )
        .is_ok());
        assert!(
            check_forbidden_env_btree(ZoneId::Sandbox, &envs.sandboxd, SANDBOX_FORBIDDEN_ENV)
                .is_ok()
        );
        assert!(check_forbidden_env_btree(
            ZoneId::AgentRunner,
            &envs.agent_runner,
            AGENT_RUNNER_FORBIDDEN_ENV
        )
        .is_ok());

        // Master token must NEVER enter child envs; only per-link HKDF keys are injected.
        for map in [
            &envs.gateway,
            &envs.consensusd,
            &envs.sandboxd,
            &envs.agent_runner,
        ] {
            assert!(!map.contains_key("UNRELATED_AMBIENT_SECRET"));
            assert!(!map.contains_key(ZONE_IPC_TOKEN_ENV));
        }

        // Cross-zone link isolation: gateway never sees consensus->agent or agent->sandbox keys.
        assert!(envs
            .gateway
            .contains_key(&link_key_env_var(ZoneId::Gateway, ZoneId::Consensus)));
        assert!(envs
            .gateway
            .contains_key(&link_key_env_var(ZoneId::Operator, ZoneId::Gateway)));
        assert!(!envs
            .gateway
            .contains_key(&link_key_env_var(ZoneId::Consensus, ZoneId::AgentRunner)));
        assert!(!envs
            .gateway
            .contains_key(&link_key_env_var(ZoneId::AgentRunner, ZoneId::Sandbox)));
        assert!(!envs
            .sandboxd
            .contains_key(&link_key_env_var(ZoneId::Gateway, ZoneId::Consensus)));
        assert!(!envs
            .agent_runner
            .contains_key(&link_key_env_var(ZoneId::Gateway, ZoneId::Consensus)));

        // Operator bundle holds ONLY Operator->Zone link keys matching each child's Operator key.
        let bundle = build_operator_key_bundle(&cfg).unwrap();
        assert_eq!(
            Some(&bundle.gateway.link_key_hex),
            envs.gateway
                .get(&link_key_env_var(ZoneId::Operator, ZoneId::Gateway))
        );
        assert_eq!(
            Some(&bundle.consensus.link_key_hex),
            envs.consensusd
                .get(&link_key_env_var(ZoneId::Operator, ZoneId::Consensus))
        );
        assert_eq!(
            Some(&bundle.sandbox.link_key_hex),
            envs.sandboxd
                .get(&link_key_env_var(ZoneId::Operator, ZoneId::Sandbox))
        );
        assert_eq!(
            Some(&bundle.agent_runner.link_key_hex),
            envs.agent_runner
                .get(&link_key_env_var(ZoneId::Operator, ZoneId::AgentRunner))
        );

        // Only gateway holds admin bearer token; only consensus holds signing/vault; only agent holds LLM keys.
        assert!(envs.gateway.contains_key("CREG_GATEWAY_ADMIN_TOKEN"));
        assert!(!envs.consensusd.contains_key("CREG_GATEWAY_ADMIN_TOKEN"));
        assert!(!envs.sandboxd.contains_key("CREG_GATEWAY_ADMIN_TOKEN"));
        assert!(!envs.agent_runner.contains_key("CREG_GATEWAY_ADMIN_TOKEN"));

        // The legacy name reaches consensus under the name the node reads.
        assert_eq!(
            envs.consensusd.get("CREG_VALIDATOR_KEY"),
            Some(&"11".repeat(32))
        );
        assert!(!envs.consensusd.contains_key("CREG_VALIDATOR_PRIVKEY"));
        assert!(envs.agent_runner.contains_key("OPENAI_API_KEY"));
        assert!(envs.sandboxd.contains_key("CREG_SANDBOX_ENGINE"));
    }

    #[test]
    fn node_key_names_reach_only_the_consensus_zone() {
        let mut parent = BTreeMap::new();
        for key in zone_ipc::CONSENSUS_SIGNING_SECRETS_ENV {
            parent.insert((*key).to_string(), "44".repeat(32));
        }
        parent.insert("CREG_VALIDATOR_KEY".into(), "55".repeat(32));
        parent.insert("CREG_IPFS_URL".into(), "http://127.0.0.1:5001".into());
        let cfg = SupervisorConfig::from_parent_env(&parent);
        let envs = build_zone_child_envs(&parent, &cfg).unwrap();

        // The current name wins over the legacy alias.
        assert_eq!(
            envs.consensusd.get("CREG_VALIDATOR_KEY"),
            Some(&"55".repeat(32))
        );
        assert!(envs
            .consensusd
            .contains_key("CREG_VALIDATOR_PRIVKEY_X25519"));
        for (zone, env, forbidden) in [
            (ZoneId::Gateway, &envs.gateway, GATEWAY_FORBIDDEN_ENV),
            (ZoneId::Sandbox, &envs.sandboxd, SANDBOX_FORBIDDEN_ENV),
            (
                ZoneId::AgentRunner,
                &envs.agent_runner,
                AGENT_RUNNER_FORBIDDEN_ENV,
            ),
        ] {
            assert!(check_forbidden_env_btree(zone, env, forbidden).is_ok());
        }
        // Lane C fetches tarballs from the agent runner.
        assert_eq!(
            envs.agent_runner.get("CREG_IPFS_URL").map(String::as_str),
            Some("http://127.0.0.1:5001")
        );
    }
}
