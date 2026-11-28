use anyhow::{bail, Context, Result};
use chrono::Utc;
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::chain_client::{
    fetch_r7_admission_status, fetch_r7_certified_identity, fetch_r7_health, R7AdmissionSummary,
    R7CertifiedIdentity, R7NodeHealth,
};
use crate::zones::default_operator_key_file;

const MAX_LOG_LINES: usize = 1_000;
const R7_CHAIN_ID: &str = "creg-phase-c-testnet-2026-09-26-r7";

fn background_command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    DockerCompose,
    LocalSupervisor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    pub repo_root: Option<String>,
    pub compose_file: Option<String>,
    pub docker_available: bool,
    pub supervisor_binary: Option<String>,
    pub creg_cli_available: bool,
    pub default_operator_key_file: String,
    pub default_ai_config_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeContainerStatus {
    pub name: String,
    pub state: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub mode: RuntimeMode,
    pub running: bool,
    pub pid: Option<u32>,
    pub started_at_unix: Option<i64>,
    pub containers: Vec<RuntimeContainerStatus>,
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct R7RuntimeStatus {
    pub container_name: String,
    pub node_id: String,
    pub container_state: String,
    pub http_url: Option<String>,
    pub health: Option<R7NodeHealth>,
    pub certified_identity: Option<R7CertifiedIdentity>,
    pub admission: Option<R7AdmissionSummary>,
}

fn parse_r7_container(line: &str) -> Option<(&str, String, &str)> {
    let mut fields = line.split_whitespace();
    let name = fields.next()?;
    let state = fields.next()?;
    if let Some(label) = fields.next() {
        let node_id = name.strip_prefix("creg-validator-")?;
        let (chain_id, public_key) = label.split_once('/')?;
        return (fields.next().is_none()
            && !node_id.is_empty()
            && chain_id == R7_CHAIN_ID
            && public_key.len() == 64
            && public_key.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| (name, node_id.to_owned(), state));
    }

    let live_name = name.split_once("-before-").map_or(name, |(base, _)| base);
    if let Some(index) = live_name.strip_prefix("creg-phase-c-r7-v") {
        return index
            .parse::<u16>()
            .ok()
            .map(|index| (name, format!("validator-{index}"), state));
    }
    let node_id = live_name.strip_prefix("creg-validator-")?;
    node_id.strip_prefix("ext-val-cli-")?.parse::<u16>().ok()?;
    Some((name, node_id.to_owned(), state))
}

fn r7_loopback_port(output: &str) -> Option<u16> {
    ["127.0.0.1:", "0.0.0.0:"].into_iter().find_map(|binding| {
        output.lines().find_map(|line| {
            let port = line.trim().strip_prefix(binding)?.parse::<u16>().ok()?;
            (port != 0).then_some(port)
        })
    })
}

async fn docker_status_output(args: &[&str]) -> Result<Output> {
    let mut command = background_command("docker");
    command.args(args).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(4), command.output())
        .await
        .context("Docker status check timed out")?
        .context("Docker CLI is unavailable")?;
    anyhow::ensure!(
        output.status.success(),
        "Docker could not report the local R7 node status"
    );
    Ok(output)
}

async fn r7_container_inventory() -> Result<String> {
    let output = docker_status_output(&[
        "container",
        "ls",
        "-a",
        "--filter",
        "name=creg-",
        "--format",
        r#"{{.Names}} {{.State}} {{.Label "dev.creg.validator"}}"#,
    ])
    .await?;
    Ok(String::from_utf8(output.stdout)?)
}

async fn r7_http_port(name: &str) -> Option<u16> {
    let output = docker_status_output(&["port", name, "8080/tcp"])
        .await
        .ok()?;
    r7_loopback_port(&String::from_utf8_lossy(&output.stdout))
}

pub async fn get_r7_runtime_status() -> Result<Vec<R7RuntimeStatus>> {
    let stdout = r7_container_inventory().await?;
    let mut runtimes = join_all(stdout.lines().filter_map(parse_r7_container).map(
        |(name, node_id, state)| async move {
            let mut local_port = None;
            let mut health = None;
            if state == "running" {
                local_port = r7_http_port(name).await;
                if let Some(port) = local_port {
                    health = fetch_r7_health(port, &node_id, R7_CHAIN_ID).await.ok();
                }
            }
            let (admission, certified_identity) = match local_port.filter(|_| health.is_some()) {
                Some(port) => tokio::join!(
                    fetch_r7_admission_status(Some(port), &node_id),
                    fetch_r7_certified_identity(port, &node_id),
                ),
                None => (None, None),
            };
            R7RuntimeStatus {
                container_name: name.to_owned(),
                node_id,
                container_state: state.to_owned(),
                http_url: local_port.map(|port| format!("http://127.0.0.1:{port}")),
                health,
                certified_identity,
                admission,
            }
        },
    ))
    .await;
    runtimes.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    Ok(runtimes)
}

#[derive(Debug)]
pub struct LogBuffer {
    cap: usize,
    lines: VecDeque<String>,
}

impl LogBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            lines: VecDeque::with_capacity(cap.min(256)),
        }
    }

    pub fn push_line(&mut self, line: impl Into<String>) {
        if self.lines.len() >= self.cap {
            self.lines.pop_front();
        }
        self.lines.push_back(line.into());
    }

    pub fn tail(&self, limit: usize) -> Vec<String> {
        let take = limit.min(self.lines.len());
        let skip = self.lines.len().saturating_sub(take);
        self.lines.iter().skip(skip).cloned().collect()
    }
}

#[derive(Debug)]
pub struct RuntimeManager {
    pub mode: RuntimeMode,
    pub child: Option<Child>,
    pub pid: Option<u32>,
    pub started_at_unix: Option<i64>,
    pub last_error: Option<String>,
    pub logs: Arc<std::sync::Mutex<LogBuffer>>,
}

impl Default for RuntimeManager {
    fn default() -> Self {
        Self {
            mode: RuntimeMode::DockerCompose,
            child: None,
            pid: None,
            started_at_unix: None,
            last_error: None,
            logs: Arc::new(std::sync::Mutex::new(LogBuffer::new(MAX_LOG_LINES))),
        }
    }
}

#[derive(Clone, Default)]
pub struct RuntimeState {
    pub inner: Arc<Mutex<RuntimeManager>>,
}

pub fn find_repo_root() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.to_path_buf());
        }
    }
    // A local build can find its source checkout when launched from Explorer.
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));

    for start in candidates {
        let mut cursor = Some(start.as_path());
        while let Some(dir) = cursor {
            if dir
                .join("chain-registry")
                .join("docker-compose.validator-v2.yml")
                .exists()
            {
                return Some(dir.to_path_buf());
            }
            if dir.join("docker-compose.validator-v2.yml").exists() {
                if let Some(parent) = dir.parent() {
                    return Some(parent.to_path_buf());
                }
            }
            cursor = dir.parent();
        }
    }
    None
}

fn find_binary_on_disk(repo_root: Option<&Path>, names: &[&str]) -> Option<String> {
    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
    let mut dirs_to_check = Vec::new();
    if let Ok(target) = std::env::var("CARGO_TARGET_DIR") {
        let p = PathBuf::from(target);
        dirs_to_check.push(p.join("release"));
        dirs_to_check.push(p.join("debug"));
    }
    if let Some(home) = dirs::home_dir() {
        let creg_target = home.join(".cargo-targets").join("creg");
        dirs_to_check.push(creg_target.join("release"));
        dirs_to_check.push(creg_target.join("debug"));
    }
    if let Some(root) = repo_root {
        dirs_to_check.push(root.join("chain-registry").join("dist-bin"));
        dirs_to_check.push(root.join("chain-registry").join("target").join("release"));
        dirs_to_check.push(root.join("chain-registry").join("target").join("debug"));
    }

    for name in names {
        let file_name = format!("{name}{exe_suffix}");
        for dir in &dirs_to_check {
            let candidate = dir.join(&file_name);
            if candidate.is_file() {
                return Some(candidate.display().to_string());
            }
        }
        if let Some(path_var) = std::env::var_os("PATH") {
            for entry in std::env::split_paths(&path_var) {
                let candidate = entry.join(&file_name);
                if candidate.is_file() {
                    return Some(candidate.display().to_string());
                }
            }
        }
    }
    None
}

pub async fn detect_environment() -> EnvironmentInfo {
    let repo_root = find_repo_root();
    let compose_file = repo_root
        .as_ref()
        .map(|r| {
            r.join("chain-registry")
                .join("docker-compose.validator-v2.yml")
        })
        .filter(|p| p.exists())
        .map(|p| p.display().to_string());

    let docker_available = background_command("docker")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success());

    let supervisor_binary = find_binary_on_disk(
        repo_root.as_deref(),
        &["creg-node-supervisor", "creg-consensusd"],
    );
    let creg_cli_available = find_binary_on_disk(repo_root.as_deref(), &["creg"]).is_some();

    let default_ai = std::env::var("CREG_AI_CONFIG")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".creg").join("validator-ai.toml"))
                .unwrap_or_else(|| PathBuf::from(ai_config::DEFAULT_AI_CONFIG_PATH))
                .display()
                .to_string()
        });

    EnvironmentInfo {
        repo_root: repo_root.map(|r| r.display().to_string()),
        compose_file,
        docker_available,
        supervisor_binary,
        creg_cli_available,
        default_operator_key_file: default_operator_key_file().display().to_string(),
        default_ai_config_path: default_ai,
    }
}

pub async fn open_validator_onboarding(container_name: Option<&str>) -> Result<()> {
    let repo_root = find_repo_root();
    let bundled_cli = std::env::current_exe()
        .ok()
        .map(|application| {
            application.with_file_name(format!("creg{}", std::env::consts::EXE_SUFFIX))
        })
        .filter(|candidate| candidate.is_file());
    let cli = bundled_cli
        .or_else(|| find_binary_on_disk(repo_root.as_deref(), &["creg"]).map(PathBuf::from))
        .context("Install a CREG CLI build that includes 'creg validator onboard'")?;
    let mut command = background_command(cli);
    command
        .args(["validator", "onboard"])
        .env_remove("CREG_NODE_URL")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(container_name) = container_name {
        let inventory = r7_container_inventory().await?;
        let (name, node_id, _) = inventory
            .lines()
            .filter_map(parse_r7_container)
            .find(|(name, _, state)| *name == container_name && *state == "running")
            .context("The selected R7 validator is no longer running")?;
        let port = r7_http_port(name)
            .await
            .context("The selected validator has no local HTTP port")?;
        fetch_r7_health(port, &node_id, R7_CHAIN_ID)
            .await
            .context("Could not verify the selected validator and signed R7 network")?;
        command.args([
            "--manage-existing",
            "--node-id",
            &node_id,
            "--local-node-url",
            &format!("http://127.0.0.1:{port}"),
        ]);
    }
    if let Some(root) = &repo_root {
        command.current_dir(root);
    }
    let mut child = command
        .spawn()
        .context("Could not start CREG validator onboarding")?;
    let stdout = child
        .stdout
        .take()
        .context("Onboarding output unavailable")?;
    let mut lines = BufReader::new(stdout).lines();
    let started = tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line())
        .await
        .context("Timed out starting CREG validator onboarding")??;
    if !started.is_some_and(|line| line.starts_with("Validator onboarding: http://127.0.0.1:")) {
        let output = child.wait_with_output().await?;
        let reason: String = String::from_utf8_lossy(&output.stderr)
            .chars()
            .filter(|ch| !ch.is_control())
            .take(400)
            .collect();
        anyhow::bail!("Could not start validator onboarding. {reason}");
    }
    tokio::spawn(async move {
        while let Ok(Some(_)) = lines.next_line().await {}
        let _ = child.wait().await;
    });
    Ok(())
}

pub fn compose_up_args(compose_file: &str) -> Vec<String> {
    vec![
        "compose".to_string(),
        "-f".to_string(),
        compose_file.to_string(),
        "up".to_string(),
        "-d".to_string(),
    ]
}

pub fn compose_down_args(compose_file: &str) -> Vec<String> {
    vec![
        "compose".to_string(),
        "-f".to_string(),
        compose_file.to_string(),
        "down".to_string(),
    ]
}

async fn query_compose_ps(compose_file: &str) -> Result<Vec<RuntimeContainerStatus>> {
    let out = background_command("docker")
        .args(["compose", "-f", compose_file, "ps", "--format", "json"])
        .output()
        .await
        .context("failed to run docker compose ps")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("docker compose ps failed: {}", stderr.trim());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut list = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let name = val
                .get("Service")
                .or_else(|| val.get("Name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let state = val
                .get("State")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let status = val
                .get("Status")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            list.push(RuntimeContainerStatus {
                name,
                state,
                status,
            });
        }
    }
    Ok(list)
}

pub async fn get_runtime_status(
    state: &RuntimeState,
    mode_override: Option<RuntimeMode>,
    compose_override: Option<&str>,
) -> RuntimeStatus {
    let env = detect_environment().await;
    let mut mgr = state.inner.lock().await;
    let mode = mode_override.unwrap_or(mgr.mode);
    mgr.mode = mode;

    match mode {
        RuntimeMode::DockerCompose => {
            let compose = compose_override.or(env.compose_file.as_deref());
            let (containers, err) = match compose {
                Some(path) if env.docker_available => match query_compose_ps(path).await {
                    Ok(c) => (c, None),
                    Err(e) => (Vec::new(), Some(format!("{e:#}"))),
                },
                Some(_) => (Vec::new(), Some("Docker CLI not found in PATH".to_string())),
                None => (
                    Vec::new(),
                    Some("docker-compose.validator-v2.yml not found".to_string()),
                ),
            };
            let running = containers
                .iter()
                .any(|c| c.state.eq_ignore_ascii_case("running"));
            RuntimeStatus {
                mode,
                running,
                pid: None,
                started_at_unix: if running { mgr.started_at_unix } else { None },
                containers,
                last_error: err.or_else(|| mgr.last_error.clone()),
            }
        }
        RuntimeMode::LocalSupervisor => {
            if let Some(child) = mgr.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        mgr.last_error = if status.success() {
                            None
                        } else {
                            Some(format!("supervisor exited with {status}"))
                        };
                        mgr.child = None;
                        mgr.pid = None;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        mgr.last_error = Some(format!("failed polling child: {e}"));
                    }
                }
            }
            RuntimeStatus {
                mode,
                running: mgr.child.is_some(),
                pid: mgr.pid,
                started_at_unix: mgr.started_at_unix,
                containers: Vec::new(),
                last_error: mgr.last_error.clone(),
            }
        }
    }
}

pub async fn start_validator(
    state: &RuntimeState,
    mode: RuntimeMode,
    compose_override: Option<&str>,
    binary_override: Option<&str>,
    args: Option<Vec<String>>,
) -> Result<RuntimeStatus> {
    let env = detect_environment().await;
    match mode {
        RuntimeMode::DockerCompose => {
            let compose = compose_override
                .map(str::to_string)
                .or(env.compose_file)
                .context("docker-compose.validator-v2.yml path not found")?;
            if cfg!(windows)
                && std::fs::read_to_string(&compose)
                    .is_ok_and(|raw| raw.contains("/dev/kvm"))
            {
                let msg = "The four-zone Docker Compose stack requires Linux KVM (/dev/kvm). Manage your testnet node in Local R7 validators.".to_string();
                let mut mgr = state.inner.lock().await;
                mgr.mode = RuntimeMode::DockerCompose;
                mgr.last_error = Some(msg.clone());
                bail!("{msg}");
            }
            let out = background_command("docker")
                .args(compose_up_args(&compose))
                .output()
                .await
                .context("failed to execute docker compose up")?;
            let mut mgr = state.inner.lock().await;
            mgr.mode = RuntimeMode::DockerCompose;
            if !out.status.success() {
                let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                mgr.last_error = Some(err_msg.clone());
                bail!("docker compose up failed: {err_msg}");
            }
            mgr.started_at_unix = Some(Utc::now().timestamp());
            mgr.last_error = None;
            drop(mgr);
            Ok(get_runtime_status(state, Some(mode), Some(&compose)).await)
        }
        RuntimeMode::LocalSupervisor => {
            let mut mgr = state.inner.lock().await;
            mgr.mode = RuntimeMode::LocalSupervisor;
            if mgr.child.is_some() {
                drop(mgr);
                return Ok(get_runtime_status(state, Some(mode), None).await);
            }
            let binary = binary_override
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or(env.supervisor_binary)
                .context("no creg-node-supervisor or creg-consensusd binary found")?;

            let mut cmd = background_command(&binary);
            if let Some(extra_args) = args {
                cmd.args(extra_args);
            }
            if let Some(root) = env.repo_root {
                cmd.current_dir(Path::new(&root).join("chain-registry"));
            }
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

            let mut child = cmd
                .spawn()
                .with_context(|| format!("failed to spawn local validator binary '{binary}'"))?;
            let pid = child.id();
            let logs = Arc::clone(&mgr.logs);
            if let Some(stdout) = child.stdout.take() {
                let logs_out = Arc::clone(&logs);
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = reader.next_line().await {
                        if let Ok(mut guard) = logs_out.lock() {
                            guard.push_line(line);
                        }
                    }
                });
            }
            if let Some(stderr) = child.stderr.take() {
                let logs_err = Arc::clone(&logs);
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = reader.next_line().await {
                        if let Ok(mut guard) = logs_err.lock() {
                            guard.push_line(format!("[stderr] {line}"));
                        }
                    }
                });
            }
            mgr.child = Some(child);
            mgr.pid = pid;
            mgr.started_at_unix = Some(Utc::now().timestamp());
            mgr.last_error = None;
            drop(mgr);
            Ok(get_runtime_status(state, Some(mode), None).await)
        }
    }
}

pub async fn stop_validator(
    state: &RuntimeState,
    mode: RuntimeMode,
    compose_override: Option<&str>,
) -> Result<RuntimeStatus> {
    let env = detect_environment().await;
    match mode {
        RuntimeMode::DockerCompose => {
            let compose = compose_override
                .map(str::to_string)
                .or(env.compose_file)
                .context("docker-compose.validator-v2.yml path not found")?;
            let out = background_command("docker")
                .args(compose_down_args(&compose))
                .output()
                .await
                .context("failed to execute docker compose down")?;
            let mut mgr = state.inner.lock().await;
            mgr.mode = RuntimeMode::DockerCompose;
            if !out.status.success() {
                let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                mgr.last_error = Some(err_msg.clone());
                bail!("docker compose down failed: {err_msg}");
            }
            mgr.started_at_unix = None;
            mgr.last_error = None;
            drop(mgr);
            Ok(get_runtime_status(state, Some(mode), Some(&compose)).await)
        }
        RuntimeMode::LocalSupervisor => {
            let mut mgr = state.inner.lock().await;
            if let Some(mut child) = mgr.child.take() {
                child
                    .kill()
                    .await
                    .context("failed to terminate local validator child process")?;
                let _ = child.wait().await;
            }
            mgr.pid = None;
            mgr.started_at_unix = None;
            drop(mgr);
            Ok(get_runtime_status(state, Some(mode), None).await)
        }
    }
}

pub async fn get_runtime_logs(
    state: &RuntimeState,
    mode: RuntimeMode,
    compose_override: Option<&str>,
    tail: usize,
) -> Result<Vec<String>> {
    let tail_clamped = tail.clamp(1, MAX_LOG_LINES);
    match mode {
        RuntimeMode::DockerCompose => {
            let env = detect_environment().await;
            let compose = compose_override
                .map(str::to_string)
                .or(env.compose_file)
                .context("docker-compose.validator-v2.yml path not found")?;
            let out = background_command("docker")
                .args([
                    "compose",
                    "-f",
                    &compose,
                    "logs",
                    "--tail",
                    &tail_clamped.to_string(),
                    "--no-color",
                ])
                .output()
                .await
                .context("failed to fetch docker compose logs")?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                bail!("docker compose logs failed: {}", stderr.trim());
            }
            Ok(String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::to_string)
                .collect())
        }
        RuntimeMode::LocalSupervisor => {
            let mgr = state.inner.lock().await;
            let guard = mgr
                .logs
                .lock()
                .map_err(|_| anyhow::anyhow!("log buffer lock poisoned"))?;
            Ok(guard.tail(tail_clamped))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r7_status_discovers_managed_and_manually_launched_r7_nodes() {
        let public_key = "f".repeat(64);
        let entry = format!("creg-validator-example running {R7_CHAIN_ID}/{public_key}");
        assert_eq!(
            parse_r7_container(&entry),
            Some(("creg-validator-example", "example".to_owned(), "running"))
        );
        assert!(
            parse_r7_container(&format!("creg-validator-example running old/{public_key}"))
                .is_none()
        );
        assert_eq!(
            parse_r7_container("creg-phase-c-r7-v1 running"),
            Some(("creg-phase-c-r7-v1", "validator-1".to_owned(), "running"))
        );
        assert_eq!(
            parse_r7_container("creg-validator-ext-val-cli-2 running"),
            Some((
                "creg-validator-ext-val-cli-2",
                "ext-val-cli-2".to_owned(),
                "running"
            ))
        );
        assert!(parse_r7_container("bench-v1 running").is_none());
        assert!(parse_r7_container("creg-validator-unlabeled running").is_none());
        assert_eq!(
            r7_loopback_port("0.0.0.0:8080\n127.0.0.1:59567\n"),
            Some(59567)
        );
        assert_eq!(
            r7_loopback_port("0.0.0.0:18084\n127.0.0.1:0\n"),
            Some(18084)
        );
        assert_eq!(r7_loopback_port("192.0.2.1:8080\n"), None);
    }

    #[test]
    fn log_buffer_evicts_oldest_beyond_capacity() {
        let mut buf = LogBuffer::new(3);
        buf.push_line("a");
        buf.push_line("b");
        buf.push_line("c");
        buf.push_line("d");
        assert_eq!(buf.tail(10), vec!["b", "c", "d"]);
        assert_eq!(buf.tail(2), vec!["c", "d"]);
    }

    #[test]
    fn compose_args_match_v2_validator_stack() {
        assert_eq!(
            compose_up_args("docker-compose.validator-v2.yml"),
            vec![
                "compose",
                "-f",
                "docker-compose.validator-v2.yml",
                "up",
                "-d"
            ]
        );
        assert_eq!(
            compose_down_args("docker-compose.validator-v2.yml"),
            vec!["compose", "-f", "docker-compose.validator-v2.yml", "down"]
        );
    }
}
