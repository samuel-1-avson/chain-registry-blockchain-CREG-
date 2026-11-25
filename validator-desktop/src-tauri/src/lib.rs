pub mod chain_client;
pub mod runtime;
pub mod zones;

use ai_config::AiConfigCheckReportV1;
use chain_client::{NodeOverview, PackageEvidenceBundle, SseStreamHandle};
use registry_view::index::{CommitteeDiversityV1, QuarantineQueueV1};
use runtime::{EnvironmentInfo, RuntimeMode, RuntimeState, RuntimeStatus};
use std::path::PathBuf;
use tauri::{AppHandle, State};
use zone_ipc::ProviderProbeReportV1;
use zones::DesktopDoctorReport;

#[tauri::command]
async fn detect_environment() -> EnvironmentInfo {
    runtime::detect_environment().await
}

#[tauri::command]
async fn get_runtime_status(
    state: State<'_, RuntimeState>,
    mode: Option<RuntimeMode>,
    compose_file: Option<String>,
) -> Result<RuntimeStatus, String> {
    Ok(runtime::get_runtime_status(&state, mode, compose_file.as_deref()).await)
}

#[tauri::command]
async fn start_validator(
    state: State<'_, RuntimeState>,
    mode: RuntimeMode,
    compose_file: Option<String>,
    supervisor_binary: Option<String>,
    args: Option<Vec<String>>,
) -> Result<RuntimeStatus, String> {
    runtime::start_validator(
        &state,
        mode,
        compose_file.as_deref(),
        supervisor_binary.as_deref(),
        args,
    )
    .await
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn stop_validator(
    state: State<'_, RuntimeState>,
    mode: RuntimeMode,
    compose_file: Option<String>,
) -> Result<RuntimeStatus, String> {
    runtime::stop_validator(&state, mode, compose_file.as_deref())
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn get_runtime_logs(
    state: State<'_, RuntimeState>,
    mode: RuntimeMode,
    compose_file: Option<String>,
    tail: Option<usize>,
) -> Result<Vec<String>, String> {
    runtime::get_runtime_logs(&state, mode, compose_file.as_deref(), tail.unwrap_or(200))
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn run_validator_doctor(
    operator_key_file: Option<String>,
    ai_config_path: Option<String>,
) -> Result<DesktopDoctorReport, String> {
    let key = operator_key_file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let ai = ai_config_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    Ok(zones::probe_all_zones(key.as_deref(), ai.as_deref()).await)
}

#[tauri::command]
fn check_ai_config(
    config_path: Option<String>,
    toml_text: Option<String>,
) -> Result<AiConfigCheckReportV1, String> {
    if let Some(raw) = toml_text {
        return zones::check_ai_config_toml_str(&raw, config_path).map_err(|e| format!("{e:#}"));
    }
    let path = config_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    zones::check_ai_config_file(path.as_deref()).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn read_ai_config(config_path: String) -> Result<String, String> {
    zones::read_ai_config_text(&PathBuf::from(config_path.trim())).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn save_ai_config(config_path: String, toml_text: String) -> Result<AiConfigCheckReportV1, String> {
    zones::save_ai_config_text(&PathBuf::from(config_path.trim()), &toml_text)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn probe_ai_providers(
    operator_key_file: Option<String>,
    provider: Option<String>,
) -> Result<ProviderProbeReportV1, String> {
    let key = operator_key_file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    zones::probe_ai_providers(key.as_deref(), provider)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn fetch_node_overview(node_url: String) -> Result<NodeOverview, String> {
    chain_client::fetch_node_overview(&node_url)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn fetch_quarantine_queue(
    node_url: String,
    indexer_url: Option<String>,
) -> Result<QuarantineQueueV1, String> {
    chain_client::fetch_quarantine_queue(&node_url, indexer_url.as_deref())
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn fetch_committee_diversity(
    indexer_url: String,
    epoch: u64,
) -> Result<CommitteeDiversityV1, String> {
    chain_client::fetch_committee_diversity(&indexer_url, epoch)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn fetch_package_evidence(
    node_url: String,
    indexer_url: Option<String>,
    canonical: String,
) -> Result<PackageEvidenceBundle, String> {
    chain_client::fetch_package_evidence(&node_url, indexer_url.as_deref(), &canonical)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn start_sse_stream(
    app: AppHandle,
    sse: State<'_, SseStreamHandle>,
    stream_url: String,
) -> Result<(), String> {
    chain_client::start_sse_stream(app, &sse, &stream_url)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn stop_sse_stream(sse: State<'_, SseStreamHandle>) -> Result<(), String> {
    chain_client::stop_sse_stream(&sse).await;
    Ok(())
}

pub fn run() {
    let runtime_state = RuntimeState::default();
    let sse_handle = SseStreamHandle::default();
    let result = tauri::Builder::default()
        .manage(runtime_state)
        .manage(sse_handle)
        .invoke_handler(tauri::generate_handler![
            detect_environment,
            get_runtime_status,
            start_validator,
            stop_validator,
            get_runtime_logs,
            run_validator_doctor,
            check_ai_config,
            read_ai_config,
            save_ai_config,
            probe_ai_providers,
            fetch_node_overview,
            fetch_quarantine_queue,
            fetch_committee_diversity,
            fetch_package_evidence,
            start_sse_stream,
            stop_sse_stream,
        ])
        .run(tauri::generate_context!());
    if let Err(err) = result {
        eprintln!("error while running CREG Validator Console: {err}");
    }
}
