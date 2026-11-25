use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::Mutex;

use crate::zones::default_operator_key_file;

const MAX_LOG_LINES: usize = 1_000;

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

    let docker_available = tokio::process::Command::new("docker")
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
    let out = tokio::process::Command::new("docker")
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
            let out = tokio::process::Command::new("docker")
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

            let mut cmd = tokio::process::Command::new(&binary);
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
            let out = tokio::process::Command::new("docker")
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
            let out = tokio::process::Command::new("docker")
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
