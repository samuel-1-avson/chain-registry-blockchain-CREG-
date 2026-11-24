//! Versioned diagnostic payloads and owner-only operator key bundle for
//! four-zone `zone-ipc` inspection (`ADR-019` §4.3, `ECA-04`).

use crate::{ZoneEndpoint, ZoneId, ZoneIpcClient, ZoneLinkKey};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use zeroize::Zeroize;

pub const OPERATOR_KEY_FILE_ENV: &str = "CREG_OPERATOR_IPC_KEY_FILE";
const MAX_KEY_BUNDLE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneCapabilityReportV1 {
    pub schema_version: u16,
    pub zone: ZoneId,
    pub status: String,
    pub capabilities: ZoneCapabilitiesV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ZoneCapabilitiesV1 {
    Gateway {
        public_listen: String,
        admin_listen: String,
        ipc_listen: String,
        rate_limit_max: usize,
        rate_limit_window_secs: u64,
        admin_token_configured: bool,
        consensus_ipc_reachable: bool,
    },
    Consensus {
        node_id: String,
        chain_id: String,
        tip_height: u64,
        hybrid_active: bool,
        signing_backend: String,
        active_validators: usize,
    },
    Sandbox {
        selected_engine: String,
        available_engines: Vec<String>,
        fallback_active: bool,
        rootfs_configured: bool,
        max_timeout_secs: u64,
    },
    AgentRunner {
        ai_mode: String,
        configured_providers: Vec<String>,
        distinct_model_families: usize,
        diversity_warning: Option<String>,
        daily_budget_usd: f64,
        spent_today_usd: f64,
        ipfs_configured: bool,
        sandbox_ipc_configured: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProbeEntryV1 {
    pub provider: String,
    pub kind: String,
    pub model: String,
    pub model_family: String,
    pub endpoint_class: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub valid_tool_json: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderProbeReportV1 {
    pub schema_version: u16,
    pub ok: bool,
    pub providers: Vec<ProviderProbeEntryV1>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorZoneEndpointKeyV1 {
    pub endpoint: String,
    pub link_key_hex: String,
}

impl std::fmt::Debug for OperatorZoneEndpointKeyV1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperatorZoneEndpointKeyV1")
            .field("endpoint", &self.endpoint)
            .field("link_key_hex", &"[REDACTED]")
            .finish()
    }
}

impl Drop for OperatorZoneEndpointKeyV1 {
    fn drop(&mut self) {
        self.link_key_hex.zeroize();
    }
}

/// Owner-only key bundle written by `creg-node-supervisor` so the operator CLI
/// can send read-only `Health` / `Capabilities` / `ProbeProvider` frames over
/// per-link `ZoneId::Operator -> Zone` keys without knowing zone master tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorIpcKeyBundleV1 {
    pub schema_version: u16,
    pub gateway: OperatorZoneEndpointKeyV1,
    pub consensus: OperatorZoneEndpointKeyV1,
    pub sandbox: OperatorZoneEndpointKeyV1,
    pub agent_runner: OperatorZoneEndpointKeyV1,
}

impl OperatorIpcKeyBundleV1 {
    pub fn endpoint_and_key(&self, target: ZoneId) -> Result<&OperatorZoneEndpointKeyV1> {
        match target {
            ZoneId::Gateway => Ok(&self.gateway),
            ZoneId::Consensus => Ok(&self.consensus),
            ZoneId::Sandbox => Ok(&self.sandbox),
            ZoneId::AgentRunner => Ok(&self.agent_runner),
            ZoneId::Operator => bail!("operator zone has no target endpoint"),
        }
    }

    pub fn client_for(&self, target: ZoneId) -> Result<ZoneIpcClient> {
        let entry = self.endpoint_and_key(target)?;
        let endpoint = ZoneEndpoint::parse(&entry.endpoint)?;
        let key = ZoneLinkKey::from_hex(&entry.link_key_hex)?;
        Ok(ZoneIpcClient::with_link_key(
            ZoneId::Operator,
            target,
            endpoint,
            key,
        ))
    }

    /// Write the key bundle atomically (`write temp -> sync_all -> rename`) with
    /// owner-only permissions (`0700` parent dir, `0600` file on Unix).
    pub fn write_owner_only(&self, path: &Path) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "unsupported OperatorIpcKeyBundleV1 schema_version {} (expected 1)",
                self.schema_version
            );
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .context("operator key bundle path missing parent directory")?;
        if !parent.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(parent)
                    .with_context(|| format!("create directory {}", parent.display()))?;
            }
            #[cfg(not(unix))]
            {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create directory {}", parent.display()))?;
            }
        }

        let payload = zeroize::Zeroizing::new(
            serde_json::to_vec_pretty(self).context("serialize OperatorIpcKeyBundleV1")?,
        );
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("operator-ipc-keys.json");
        let tmp_path = parent.join(format!(".{file_name}.tmp.{}", std::process::id()));

        {
            use std::io::Write;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut file = opts
                .open(&tmp_path)
                .with_context(|| format!("create temp key bundle {}", tmp_path.display()))?;
            file.write_all(&payload)?;
            file.sync_all()?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
        }

        std::fs::rename(&tmp_path, path).with_context(|| {
            format!(
                "rename temp key bundle {} -> {}",
                tmp_path.display(),
                path.display()
            )
        })?;
        Ok(())
    }

    /// Read and validate owner-only permissions (`0600` on Unix) before loading.
    pub fn read_owner_only(path: &Path) -> Result<Self> {
        let meta = std::fs::symlink_metadata(path)
            .with_context(|| format!("stat operator key bundle {}", path.display()))?;
        let ft = meta.file_type();
        if ft.is_symlink() || !ft.is_file() {
            bail!(
                "operator key bundle '{}' must be a regular non-symlink file",
                path.display()
            );
        }
        if meta.len() == 0 || meta.len() > MAX_KEY_BUNDLE_BYTES {
            bail!(
                "operator key bundle '{}' has invalid size {} bytes",
                path.display(),
                meta.len()
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = meta.permissions().mode() & 0o777;
            if (mode & 0o077) != 0 {
                bail!(
                    "operator key bundle '{}' has mode {:04o}; owner-only (0600 or 0400) required",
                    path.display(),
                    mode
                );
            }
        }
        let raw = zeroize::Zeroizing::new(
            std::fs::read(path)
                .with_context(|| format!("read operator key bundle {}", path.display()))?,
        );
        let bundle: Self = serde_json::from_slice(&raw).context("decode OperatorIpcKeyBundleV1")?;
        if bundle.schema_version != 1 {
            bail!(
                "unsupported OperatorIpcKeyBundleV1 schema_version {} (expected 1)",
                bundle.schema_version
            );
        }
        Ok(bundle)
    }
}
