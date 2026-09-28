// crates/cli/src/explorer_tui.rs
// Chain Registry Console — the single supported terminal operator surface.
//
// Features:
// - Real-time blockchain data from node API
// - Multiple views: Overview, Blocks, Validators, Packages, Network, Mempool, Operator
// - Live SSE event streaming
// - Interactive navigation with vim-style keybindings
// - Beautiful UI with gradients, borders, and animations
// - Detailed drill-down views for all data types

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, Gauge, List, ListItem, Paragraph, Row, Sparkline,
        Table, Wrap,
    },
    Frame, Terminal,
};
use serde_json::Value;
use std::{
    collections::VecDeque,
    fs, io,
    io::Write,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, RwLock};

#[derive(Clone, Copy)]
enum ApiRouteScope {
    Public,
    Operator,
    Validator,
}

fn operator_api_key() -> Option<String> {
    std::env::var("CREG_OPERATOR_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn is_legacy_fallback_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 404 | 405 | 501)
}

fn scoped_get(
    client: &reqwest::Client,
    url: String,
    scope: ApiRouteScope,
) -> reqwest::RequestBuilder {
    let request = client.get(url);
    match scope {
        ApiRouteScope::Operator => match operator_api_key() {
            Some(api_key) => request.header("X-Operator-Key", api_key),
            None => request,
        },
        ApiRouteScope::Public | ApiRouteScope::Validator => request,
    }
}

async fn get_json_with_fallback(
    client: &reqwest::Client,
    grouped_url: String,
    legacy_url: Option<String>,
    scope: ApiRouteScope,
) -> Result<Value> {
    let grouped = scoped_get(client, grouped_url, scope).send().await?;
    if grouped.status().is_success() {
        return Ok(grouped.json().await?);
    }

    if let Some(legacy_url) = legacy_url {
        if is_legacy_fallback_status(grouped.status()) {
            let legacy = scoped_get(client, legacy_url, scope).send().await?;
            return Ok(legacy.error_for_status()?.json().await?);
        }
    }

    Ok(grouped.error_for_status()?.json().await?)
}

async fn open_stream_with_fallback(
    client: &reqwest::Client,
    grouped_url: String,
    legacy_url: Option<String>,
    scope: ApiRouteScope,
) -> Result<reqwest::Response> {
    let grouped = scoped_get(client, grouped_url, scope)
        .header("Accept", "text/event-stream")
        .send()
        .await?;
    if grouped.status().is_success() {
        return Ok(grouped);
    }

    if let Some(legacy_url) = legacy_url {
        if is_legacy_fallback_status(grouped.status()) {
            let legacy = scoped_get(client, legacy_url, scope)
                .header("Accept", "text/event-stream")
                .send()
                .await?;
            return Ok(legacy.error_for_status()?);
        }
    }

    Ok(grouped.error_for_status()?)
}

// ============================================================================
// CONSTANTS & STYLING
// ============================================================================

const TICK_RATE_MS: u64 = 100;
const REFRESH_INTERVAL_SECS: u64 = 3;
const MAX_EVENTS: usize = 200;
const MAX_BLOCKS: usize = 100;
const TOAST_SECS: u64 = 4;
const QUIT_ARM_SECS: u64 = 3;
const HEADER_HEIGHT: u16 = 3;
const TAB_HEIGHT: u16 = 3;
const FOOTER_HEIGHT: u16 = 3;

use std::sync::atomic::{AtomicBool, Ordering};
static IS_LIGHT_THEME: AtomicBool = AtomicBool::new(false);
static FORCE_REFRESH: AtomicBool = AtomicBool::new(false);

/// CREG console palette. Dark is the default operator surface.
struct Theme;
impl Theme {
    fn primary() -> Color {
        if IS_LIGHT_THEME.load(Ordering::Relaxed) {
            Color::Rgb(13, 148, 136)
        } else {
            Color::Rgb(45, 212, 191)
        }
    }
    fn secondary() -> Color {
        if IS_LIGHT_THEME.load(Ordering::Relaxed) {
            Color::Rgb(3, 105, 161)
        } else {
            Color::Rgb(56, 189, 248)
        }
    }
    fn success() -> Color {
        if IS_LIGHT_THEME.load(Ordering::Relaxed) {
            Color::Rgb(21, 128, 61)
        } else {
            Color::Rgb(74, 222, 128)
        }
    }
    fn warning() -> Color {
        if IS_LIGHT_THEME.load(Ordering::Relaxed) {
            Color::Rgb(180, 83, 9)
        } else {
            Color::Rgb(251, 191, 36)
        }
    }
    fn error() -> Color {
        if IS_LIGHT_THEME.load(Ordering::Relaxed) {
            Color::Rgb(185, 28, 28)
        } else {
            Color::Rgb(248, 113, 113)
        }
    }
    fn accent() -> Color {
        if IS_LIGHT_THEME.load(Ordering::Relaxed) {
            Color::Rgb(217, 119, 6)
        } else {
            Color::Rgb(251, 191, 36)
        }
    }
    fn text() -> Color {
        if IS_LIGHT_THEME.load(Ordering::Relaxed) {
            Color::Rgb(15, 23, 42)
        } else {
            Color::Rgb(226, 232, 240)
        }
    }
    fn text_dim() -> Color {
        if IS_LIGHT_THEME.load(Ordering::Relaxed) {
            Color::Rgb(71, 85, 105)
        } else {
            Color::Rgb(148, 163, 184)
        }
    }
    fn border() -> Color {
        if IS_LIGHT_THEME.load(Ordering::Relaxed) {
            Color::Rgb(148, 163, 184)
        } else {
            Color::Rgb(51, 65, 85)
        }
    }
    fn highlight() -> Color {
        if IS_LIGHT_THEME.load(Ordering::Relaxed) {
            Color::Rgb(204, 251, 241)
        } else {
            Color::Rgb(19, 78, 74)
        }
    }
    fn tab_active_fg() -> Color {
        if IS_LIGHT_THEME.load(Ordering::Relaxed) {
            Color::Rgb(15, 23, 42)
        } else {
            Color::Rgb(15, 23, 42)
        }
    }
}

fn rounded_block(title: impl Into<String>, focused: bool) -> Block<'static> {
    Block::default()
        .title(title.into())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused {
            Theme::primary()
        } else {
            Theme::border()
        }))
}

// ============================================================================
// DATA MODELS
// ============================================================================

#[derive(Debug, Clone)]
struct BlockInfo {
    height: u64,
    hash: String,
    timestamp: String,
    proposer: String,
    tx_count: usize,
    transactions: Vec<TransactionInfo>,
    merkle_root: String,
}

#[derive(Debug, Clone)]
struct TransactionInfo {
    _id: String,
    tx_type: String,
    package_name: Option<String>,
    package_version: Option<String>,
    publisher: Option<String>,
    status: String,
}

#[derive(Debug, Clone)]
struct ValidatorInfo {
    id: String,
    alias: String,
    stake: u64,
    reputation: u8,
    status: String,
    is_active: bool,
    pub_key: String,
}

#[derive(Debug, Clone)]
struct PackageInfo {
    name: String,
    ecosystem: String,
    version: String,
    status: String,
    publisher: String,
    verified_at: Option<String>,
    content_hash: String,
}

#[derive(Debug, Clone)]
struct NetworkStats {
    chain_id: String,
    genesis_hash: Option<String>,
    spec_signature_verified: bool,
    l1_rpc_configured: bool,
    tip_height: u64,
    package_count: u64,
    _block_count: u64,
    validator_count: usize,
    total_stake: u64,
    peer_count: usize,
    bridge_status: String,
    l1_block: u64,
    finalized_height: Option<u64>,
    finalization_lag: Option<u64>,
    validator_set_source: String,
    validator_set_sync_state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreflightStatus {
    Pass,
    Warn,
    Fail,
    Pending,
}

impl PreflightStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
            Self::Pending => "PENDING",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Pass => Theme::success(),
            Self::Warn => Theme::warning(),
            Self::Fail => Theme::error(),
            Self::Pending => Theme::text_dim(),
        }
    }
}

#[derive(Debug, Clone)]
struct PreflightCheck {
    name: String,
    status: PreflightStatus,
    detail: String,
}

#[derive(Debug, Clone)]
struct PreflightState {
    checks: Vec<PreflightCheck>,
    updated_at: Instant,
}

impl Default for PreflightState {
    fn default() -> Self {
        Self {
            checks: Vec::new(),
            updated_at: Instant::now(),
        }
    }
}

#[derive(Debug, Clone)]
struct MempoolTx {
    id: String,
    tx_type: String,
    _size: usize,
    timestamp: Instant,
}

// ============================================================================
// APPLICATION STATE
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
enum View {
    Overview,
    Preflight,
    Blocks,
    BlockDetail,
    Validators,
    ValidatorDetail,
    Packages,
    PackageDetail,
    Network,
    Mempool,
    Events,
    Operator,
    Consensus,
    Faucet,
    Help,
    Bridge,
    Metrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToastKind {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
struct Toast {
    message: String,
    kind: ToastKind,
    shown_at: Instant,
}

#[derive(Debug, Clone, Default)]
struct PaletteState {
    open: bool,
    query: String,
    selected: usize,
}

#[derive(Debug, Clone, Copy)]
struct PaletteCommand {
    id: &'static str,
    label: &'static str,
    hint: &'static str,
}

fn palette_commands() -> &'static [PaletteCommand] {
    &[
        PaletteCommand {
            id: "overview",
            label: "Go to Overview",
            hint: "1",
        },
        PaletteCommand {
            id: "blocks",
            label: "Go to Blocks",
            hint: "2",
        },
        PaletteCommand {
            id: "validators",
            label: "Go to Validators",
            hint: "3",
        },
        PaletteCommand {
            id: "packages",
            label: "Go to Packages",
            hint: "4",
        },
        PaletteCommand {
            id: "network",
            label: "Go to Network",
            hint: "5",
        },
        PaletteCommand {
            id: "mempool",
            label: "Go to Mempool",
            hint: "6",
        },
        PaletteCommand {
            id: "events",
            label: "Go to Events",
            hint: "7",
        },
        PaletteCommand {
            id: "operator",
            label: "Go to Operator",
            hint: "8",
        },
        PaletteCommand {
            id: "consensus",
            label: "Go to PBFT consensus",
            hint: "9",
        },
        PaletteCommand {
            id: "faucet",
            label: "Go to Faucet",
            hint: "0",
        },
        PaletteCommand {
            id: "preflight",
            label: "Go to Preflight",
            hint: "p",
        },
        PaletteCommand {
            id: "bridge",
            label: "Go to Bridge",
            hint: ":",
        },
        PaletteCommand {
            id: "metrics",
            label: "Go to Metrics",
            hint: "m",
        },
        PaletteCommand {
            id: "search",
            label: "Search / filter lists",
            hint: "/",
        },
        PaletteCommand {
            id: "yank",
            label: "Copy selection to clipboard",
            hint: "y",
        },
        PaletteCommand {
            id: "export",
            label: "Export redacted diagnostics",
            hint: "x",
        },
        PaletteCommand {
            id: "refresh",
            label: "Refresh node data now",
            hint: "r",
        },
        PaletteCommand {
            id: "profile",
            label: "Show authenticated profile identity",
            hint: "i",
        },
        PaletteCommand {
            id: "theme",
            label: "Toggle light / dark theme",
            hint: "t",
        },
        PaletteCommand {
            id: "help",
            label: "Open help",
            hint: "?",
        },
        PaletteCommand {
            id: "quit",
            label: "Quit console",
            hint: "q",
        },
    ]
}

fn filtered_palette_indices(query: &str) -> Vec<usize> {
    let q = query.trim().to_ascii_lowercase();
    palette_commands()
        .iter()
        .enumerate()
        .filter(|(_, cmd)| {
            q.is_empty()
                || cmd.label.to_ascii_lowercase().contains(&q)
                || cmd.id.contains(&q)
                || cmd.hint.to_ascii_lowercase().contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}

fn console_tabs() -> &'static [(View, &'static str, &'static str)] {
    &[
        (View::Overview, "Overview", "1"),
        (View::Blocks, "Blocks", "2"),
        (View::Validators, "Validators", "3"),
        (View::Packages, "Packages", "4"),
        (View::Network, "Network", "5"),
        (View::Mempool, "Pool", "6"),
        (View::Events, "Events", "7"),
        (View::Operator, "Ops", "8"),
        (View::Consensus, "PBFT", "9"),
        (View::Faucet, "Faucet", "0"),
        (View::Preflight, "Ready", "p"),
        (View::Bridge, "Bridge", "-"),
        (View::Metrics, "Metrics", "m"),
    ]
}

fn cycle_tab(current: View, delta: i32) -> View {
    let tabs = console_tabs();
    let idx = tabs
        .iter()
        .position(|(view, _, _)| *view == current)
        .unwrap_or(0);
    let len = tabs.len() as i32;
    let next = (idx as i32 + delta).rem_euclid(len) as usize;
    tabs[next].0
}

fn tab_at_column(column: u16, width: u16) -> Option<View> {
    let tabs = console_tabs();
    if width == 0 || tabs.is_empty() {
        return None;
    }
    let cell = (width / tabs.len() as u16).max(1);
    let idx = (column / cell) as usize;
    tabs.get(idx).map(|(view, _, _)| *view)
}

fn is_detail_view(view: View) -> bool {
    matches!(
        view,
        View::BlockDetail | View::ValidatorDetail | View::PackageDetail
    )
}

/// State of the Faucet pane's drip flow.
#[derive(Debug, Clone)]
enum FaucetStatus {
    /// No active request.
    Idle,
    /// PoW being solved and drip request in-flight.
    Working {
        started_at: Instant,
        message: String,
    },
    /// Drip completed; show success for a few seconds.
    Success {
        tx_hash: String,
        amount: String,
        at: Instant,
    },
    /// Drip failed.
    Failed { error: String, at: Instant },
}

#[derive(Debug, Clone)]
struct FaucetView {
    /// Base URL of the faucet service (e.g. http://localhost:8082).
    base: String,
    /// Address being edited or submitted.
    address_input: String,
    /// Whether the address input field has focus (insert mode).
    editing: bool,
    /// Current drip status.
    status: FaucetStatus,
    /// Last fetched token balance (raw wei-style string from the faucet).
    last_balance: Option<String>,
    /// Last fetched native (ETH) balance (raw wei).
    last_native_balance: Option<String>,
    /// Pre-formatted decimal tCREG balance from the faucet (e.g. "1000.00").
    last_balance_fmt: Option<String>,
    /// Pre-formatted decimal ETH balance from the faucet (e.g. "10000.0000").
    last_native_balance_fmt: Option<String>,
    /// Faucet /health: "online" | "offline" | "unknown"
    health: String,
    /// Faucet's own tCREG reserve (so operators know if it can still drip).
    faucet_token_reserve: Option<String>,
    /// Chain info from /api/network.
    network: Option<crate::faucet_client::NetworkInfo>,
}

impl FaucetView {
    fn new(base: String) -> Self {
        Self {
            base,
            address_input: String::new(),
            editing: false,
            status: FaucetStatus::Idle,
            last_balance: None,
            last_native_balance: None,
            last_balance_fmt: None,
            last_native_balance_fmt: None,
            health: "unknown".into(),
            faucet_token_reserve: None,
            network: None,
        }
    }
}

/// Live PBFT round snapshot fetched from the node or derived from stats.
#[derive(Debug, Clone)]
struct ConsensusState {
    /// Current round number (typically tip_height + 1)
    round: u64,
    /// Current PBFT phase: "PRE-PREPARE", "PREPARE", "COMMIT", "FINALIZED"
    phase: String,
    /// Proposer pubkey (truncated) for this round
    proposer: String,
    /// Number of PREPARE votes collected so far
    prepare_votes: usize,
    /// Number of COMMIT votes collected so far
    commit_votes: usize,
    /// Quorum threshold (2f+1)
    quorum: usize,
    /// Total validator set size
    total_validators: usize,
    /// When this snapshot was fetched
    fetched_at: Instant,
}

#[derive(Debug, Clone)]
struct RuntimeIdentity {
    node_id: String,
    validator_pubkey: Option<String>,
    chain_id: String,
    network: String,
    genesis_hash: Option<String>,
    spec_signature_verified: bool,
    l1_rpc_configured: bool,
    is_validator: bool,
    validator_set_source: String,
    validator_set_sync_state: String,
}

#[derive(Debug)]
struct App {
    // Navigation
    current_view: View,
    previous_view: Option<View>,

    // Selection indices
    selected_block: usize,
    selected_validator: usize,
    selected_package: usize,
    selected_event: usize,
    _selected_tab: usize,

    // Data
    stats: NetworkStats,
    blocks: VecDeque<BlockInfo>,
    validators: Vec<ValidatorInfo>,
    packages: Vec<PackageInfo>,
    events: VecDeque<(Instant, String, String)>, // (timestamp, type, message)
    mempool: Vec<MempoolTx>,
    peer_ids: Vec<String>,
    runtime_identity: Option<RuntimeIdentity>,
    preflight: PreflightState,

    // UI State
    _show_help: bool,
    search_query: String,
    is_searching: bool,
    _scroll_offset: usize,

    // Async
    _api_base: String,
    data_tx: mpsc::Sender<DataUpdate>,
    _last_refresh: Instant,
    tick_count: u64,

    // Consensus
    /// Latest PBFT round snapshot.
    consensus_state: Option<ConsensusState>,

    // Connection health
    /// True while the SSE stream is connected and delivering events.
    node_connected: bool,
    /// Wall-clock time the last SSE event was received.
    last_event_at: Instant,
    /// Seconds until the next reconnect attempt (shown in disconnect banner).
    reconnect_in: u64,
    /// Last successful REST API response, separate from the SSE stream state.
    last_api_success_at: Option<Instant>,
    /// Redacted, non-sensitive API failure summary.
    last_api_error: Option<String>,
    /// Most recent diagnostics export result.
    diagnostics_message: Option<String>,
    toast: Option<Toast>,
    palette: PaletteState,
    quit_armed_at: Option<Instant>,
    last_width: u16,
    /// Validator/operator monitoring is read-only by default. The TUI never
    /// accepts or stores private keys.
    read_only: bool,
    authenticated_wallet: Option<String>,
    authenticated_pubkey: Option<String>,

    // Sparkline data for TPS visualization
    tps_history: VecDeque<u64>,

    // Faucet pane state
    faucet: FaucetView,
    bridge_anchors: Vec<serde_json::Value>,
    metrics_history: Vec<serde_json::Value>,
    metrics_history_status: String,
}

#[derive(Debug)]
enum DataUpdate {
    Stats(NetworkStats),
    Block(BlockInfo),
    Validators(Vec<ValidatorInfo>),
    Packages(Vec<PackageInfo>),
    Event(String, String), // (type, message)
    MempoolSnapshot(Vec<MempoolTx>),
    Peers(Vec<String>),
    RuntimeIdentity(RuntimeIdentity),
    /// REST API responded successfully.
    ApiHealthy,
    /// REST API could not be reached; the message is intentionally redacted.
    ApiError(String),
    /// SSE stream (re-)connected successfully.
    NodeConnected,
    /// SSE stream dropped; UI should show disconnect banner.
    NodeDisconnected {
        retry_in_secs: u64,
    },
    /// Live PBFT round snapshot.
    Consensus(ConsensusState),
    /// Server-retained Prometheus samples for the selected chain.
    MetricsHistory {
        samples: Vec<serde_json::Value>,
        status: String,
    },
    /// Faucet health/reserve snapshot.
    FaucetHealth {
        healthy: bool,
        token_reserve: Option<String>,
    },
    /// Faucet network info (chain id, RPC URL, etc.).
    FaucetNetwork(crate::faucet_client::NetworkInfo),
    /// Faucet balance lookup result for the current input address.
    /// `token`/`native` are raw wei; `token_fmt`/`native_fmt` are pre-formatted
    /// decimals from the faucet (preferred for display).
    FaucetBalance {
        token: Option<String>,
        native: Option<String>,
        token_fmt: Option<String>,
        native_fmt: Option<String>,
    },
    /// Drip operation ended.
    FaucetDripResult(Result<(String, String), String>),
    /// Drip operation progress message (e.g. "solving PoW", "submitting…").
    FaucetDripProgress(String),
    /// A generic faucet-pane error (balance fetch, etc.). Renders as FaucetStatus::Failed.
    FaucetError(String),
}

impl App {
    fn new(api_base: String, data_tx: mpsc::Sender<DataUpdate>) -> Self {
        Self {
            current_view: View::Overview,
            previous_view: None,
            selected_block: 0,
            selected_validator: 0,
            selected_package: 0,
            selected_event: 0,
            _selected_tab: 0,
            stats: NetworkStats {
                chain_id: "unknown".into(),
                genesis_hash: None,
                spec_signature_verified: false,
                l1_rpc_configured: false,
                tip_height: 0,
                package_count: 0,
                _block_count: 0,
                validator_count: 0,
                total_stake: 0,
                peer_count: 0,
                bridge_status: "Unknown".to_string(),
                l1_block: 0,
                finalized_height: None,
                finalization_lag: None,
                validator_set_source: "unknown".into(),
                validator_set_sync_state: "unknown".into(),
            },
            blocks: VecDeque::with_capacity(MAX_BLOCKS),
            validators: Vec::new(),
            packages: Vec::new(),
            events: VecDeque::with_capacity(MAX_EVENTS),
            mempool: Vec::new(),
            peer_ids: Vec::new(),
            runtime_identity: None,
            preflight: PreflightState::default(),
            _show_help: false,
            search_query: String::new(),
            is_searching: false,
            _scroll_offset: 0,
            _api_base: api_base,
            data_tx,
            _last_refresh: Instant::now(),
            tick_count: 0,
            consensus_state: None,
            node_connected: false,
            last_event_at: Instant::now(),
            reconnect_in: 0,
            last_api_success_at: None,
            last_api_error: None,
            diagnostics_message: None,
            toast: None,
            palette: PaletteState::default(),
            quit_armed_at: None,
            last_width: 120,
            read_only: true,
            authenticated_wallet: {
                crate::profile::load_active_profile()
                    .ok()
                    .flatten()
                    .map(|p| p.wallet_address)
            },
            authenticated_pubkey: {
                crate::profile::load_active_profile()
                    .ok()
                    .flatten()
                    .map(|p| p.ed25519_pubkey)
            },
            tps_history: VecDeque::with_capacity(60),
            faucet: {
                let mut f = FaucetView::new(
                    std::env::var("CREG_FAUCET_URL")
                        // Use internal service name "faucet" for container-to-container comms.
                        .unwrap_or_else(|_| "http://faucet:8082".into())
                        .trim_end_matches('/')
                        .to_string(),
                );
                if let Ok(Some(prof)) = crate::profile::load_active_profile() {
                    f.address_input = prof.wallet_address;
                }
                f
            },
            bridge_anchors: Vec::new(),
            metrics_history: Vec::new(),
            metrics_history_status: "waiting for the metrics collector".into(),
        }
    }

    fn selected_block(&self) -> Option<&BlockInfo> {
        self.blocks.get(self.selected_block)
    }

    fn selected_validator(&self) -> Option<&ValidatorInfo> {
        self.validators.get(self.selected_validator)
    }

    fn selected_package(&self) -> Option<&PackageInfo> {
        self.packages.get(self.selected_package)
    }

    fn displayed_validator_count(&self) -> usize {
        if self.validators.is_empty() {
            self.stats.validator_count
        } else {
            self.validators.len()
        }
    }

    fn displayed_total_stake(&self) -> u64 {
        if self.stats.total_stake > 0 || self.validators.is_empty() {
            self.stats.total_stake
        } else {
            self.validators
                .iter()
                .map(|validator| validator.stake)
                .sum()
        }
    }

    fn push_event(&mut self, event_type: String, message: String) {
        self.events
            .push_front((Instant::now(), event_type, message));
        while self.events.len() > MAX_EVENTS {
            self.events.pop_back();
        }
    }

    fn push_toast(&mut self, kind: ToastKind, message: impl Into<String>) {
        let message = message.into();
        self.diagnostics_message = Some(message.clone());
        self.toast = Some(Toast {
            message,
            kind,
            shown_at: Instant::now(),
        });
    }

    fn expire_overlays(&mut self) {
        if let Some(toast) = &self.toast {
            if toast.shown_at.elapsed() >= Duration::from_secs(TOAST_SECS) {
                self.toast = None;
            }
        }
        if let Some(armed) = self.quit_armed_at {
            if armed.elapsed() >= Duration::from_secs(QUIT_ARM_SECS) {
                self.quit_armed_at = None;
            }
        }
    }

    fn matching_packages(&self) -> Vec<usize> {
        let query = self.search_query.to_ascii_lowercase();
        self.packages
            .iter()
            .enumerate()
            .filter(|(_, pkg)| {
                query.is_empty()
                    || pkg.name.to_ascii_lowercase().contains(&query)
                    || pkg.ecosystem.to_ascii_lowercase().contains(&query)
                    || pkg.version.to_ascii_lowercase().contains(&query)
                    || pkg.publisher.to_ascii_lowercase().contains(&query)
                    || pkg.status.to_ascii_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn matching_blocks(&self) -> Vec<usize> {
        let query = self.search_query.to_ascii_lowercase();
        self.blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| {
                query.is_empty()
                    || block.hash.to_ascii_lowercase().contains(&query)
                    || block.proposer.to_ascii_lowercase().contains(&query)
                    || block.height.to_string().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn matching_validators(&self) -> Vec<usize> {
        let query = self.search_query.to_ascii_lowercase();
        self.validators
            .iter()
            .enumerate()
            .filter(|(_, validator)| {
                query.is_empty()
                    || validator.id.to_ascii_lowercase().contains(&query)
                    || validator.alias.to_ascii_lowercase().contains(&query)
                    || validator.pub_key.to_ascii_lowercase().contains(&query)
                    || validator.status.to_ascii_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

// ============================================================================
// MAIN ENTRY POINT
// ============================================================================

pub async fn run(node_url: Option<&str>) -> Result<()> {
    let api_base = crate::config_file::effective_node_url(node_url)
        .trim_end_matches('/')
        .to_string();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Setup async channels
    let (data_tx, mut data_rx) = mpsc::channel::<DataUpdate>(1000);
    let app = Arc::new(RwLock::new(App::new(api_base.clone(), data_tx.clone())));

    // Spawn background tasks
    let app_clone = app.clone();
    let api_base_clone = api_base.clone();
    tokio::spawn(async move {
        data_fetcher_loop(app_clone, api_base_clone, data_tx.clone()).await;
    });

    // Spawn SSE event listener
    {
        let app_sse = app.clone();
        let api_sse = api_base.clone();
        let tx_sse = app.read().await.data_tx.clone();
        tokio::spawn(async move {
            sse_event_listener(app_sse, api_sse, tx_sse).await;
        });
    };

    // Main loop
    let tick_rate = Duration::from_millis(TICK_RATE_MS);
    let mut last_tick = Instant::now();

    loop {
        // Draw UI
        let app_read = app.read().await;
        terminal.draw(|f| draw_ui(f, &app_read))?;
        let frame_width = terminal.size().map(|size| size.width).unwrap_or(120);
        drop(app_read);
        {
            let mut app_write = app.write().await;
            app_write.last_width = frame_width;
        }

        // Handle events
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let mut app_write = app.write().await;
                    if handle_key(&mut app_write, key).await {
                        break;
                    }
                }
                Event::Mouse(mouse) => {
                    let mut app_write = app.write().await;
                    handle_mouse(&mut app_write, mouse);
                }
                _ => {}
            }
        }

        // Process data updates
        while let Ok(update) = data_rx.try_recv() {
            let mut app_write = app.write().await;
            apply_data_update(&mut app_write, update);
        }

        if last_tick.elapsed() >= tick_rate {
            let mut app_write = app.write().await;
            app_write.tick_count += 1;
            app_write.expire_overlays();
            last_tick = Instant::now();
            drop(app_write);
        }
    }

    // Cleanup
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

// ============================================================================
// DATA FETCHING
// ============================================================================

async fn data_fetcher_loop(app: Arc<RwLock<App>>, api_base: String, tx: mpsc::Sender<DataUpdate>) {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(15)).build().expect("HTTP client");
    let mut last_stats_refresh = Instant::now();
    let mut last_metrics_refresh: Option<Instant> = None;
    let mut last_block_height: Option<u64> = None;
    let mut last_genesis_hash: Option<String> = None;
    let faucet_base = app.read().await.faucet.base.clone();
    let mut faucet_network_fetched = false;
    let mut runtime_config_fetched = false;

    loop {
        // Fetch runtime config once at startup
        if !runtime_config_fetched {
            if let Ok(identity) = fetch_runtime_config(&client, &api_base).await {
                let _ = tx.send(DataUpdate::RuntimeIdentity(identity)).await;
                runtime_config_fetched = true;
            }
        }

        // Fetch stats periodically
        if last_stats_refresh.elapsed() >= Duration::from_secs(REFRESH_INTERVAL_SECS)
            || FORCE_REFRESH.swap(false, Ordering::Relaxed)
        {
            if let Ok(stats) = fetch_stats(&client, &api_base).await {
                if stats.genesis_hash != last_genesis_hash {
                    last_block_height = None;
                    last_genesis_hash = stats.genesis_hash.clone();
                }
                let _ = tx.send(DataUpdate::Stats(stats.clone())).await;
                let _ = tx.send(DataUpdate::ApiHealthy).await;

                // Fetch new blocks if height changed
                if last_block_height != Some(stats.tip_height) {
                    let start = stats.tip_height.saturating_sub(49);
                    let mut complete = true;
                    for h in start..=stats.tip_height {
                        if let Ok(block) = fetch_block(&client, &api_base, h).await {
                            let _ = tx.send(DataUpdate::Block(block)).await;
                        } else {
                            complete = false;
                        }
                    }
                    if complete { last_block_height = Some(stats.tip_height); }
                }
            } else {
                // Keep the UI explicit about REST availability without
                // forwarding reqwest URLs or credentials into the report.
                let _ = tx
                    .send(DataUpdate::ApiError("node REST API unavailable".into()))
                    .await;
            }

            // Fetch validators
            if let Ok(validators) = fetch_validators(&client, &api_base).await {
                let _ = tx.send(DataUpdate::Validators(validators)).await;
            }

            // Fetch peers
            if let Ok(peers) = fetch_peers(&client, &api_base).await {
                let _ = tx.send(DataUpdate::Peers(peers)).await;
            }

            // Fetch pending packages
            if let Ok(packages) = fetch_pending_packages(&client, &api_base).await {
                let _ = tx.send(DataUpdate::Packages(packages)).await;
            }

            // Fetch mempool (pending pool entries for the Mempool view)
            if let Ok(mempool) = fetch_mempool(&client, &api_base).await {
                let _ = tx.send(DataUpdate::MempoolSnapshot(mempool)).await;
            }

            // Fetch consensus state (best-effort — node may not expose this endpoint)
            if let Ok(cs) = fetch_consensus_state(&client, &api_base).await {
                let _ = tx.send(DataUpdate::Consensus(cs)).await;
            }

            // Poll faucet health (best-effort; faucet may be offline).
            if let Some((healthy, reserve)) = fetch_faucet_health(&client, &faucet_base).await {
                let _ = tx
                    .send(DataUpdate::FaucetHealth {
                        healthy,
                        token_reserve: reserve,
                    })
                    .await;
            }

            // Fetch the network info once; it's static per session.
            if !faucet_network_fetched {
                if let Ok(info) = crate::faucet_client::get_network(&client, &faucet_base).await {
                    let _ = tx.send(DataUpdate::FaucetNetwork(info)).await;
                    faucet_network_fetched = true;
                }
            }

            last_stats_refresh = Instant::now();
        }

        if last_metrics_refresh
            .map(|last| last.elapsed() >= Duration::from_secs(30))
            .unwrap_or(true)
        {
            let update = match get_json_with_fallback(
                &client,
                format!("{}/v1/public/metrics/history?range=1h", api_base),
                None,
                ApiRouteScope::Public,
            )
            .await
            {
                Ok(json) => DataUpdate::MetricsHistory {
                    samples: json["samples"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                    status: json["status"]
                        .as_str()
                        .unwrap_or("collector unavailable")
                        .to_string(),
                },
                Err(_) => DataUpdate::MetricsHistory {
                    samples: Vec::new(),
                    status: "collector unavailable".into(),
                },
            };
            let _ = tx.send(update).await;
            last_metrics_refresh = Some(Instant::now());
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn fetch_stats(client: &reqwest::Client, api_base: &str) -> Result<NetworkStats> {
    let json = get_json_with_fallback(
        client,
        format!("{}/v1/public/chain/stats", api_base),
        Some(format!("{}/v1/chain/stats", api_base)),
        ApiRouteScope::Public,
    )
    .await?;

    Ok(NetworkStats {
        chain_id: json["chain_id"].as_str().unwrap_or("unknown").to_string(),
        genesis_hash: json["genesis_hash"].as_str().map(|s| s.to_string()),
        spec_signature_verified: json["spec_signature_verified"].as_bool().unwrap_or(false),
        l1_rpc_configured: json["l1_rpc_configured"].as_bool().unwrap_or(false),
        tip_height: json["tip_height"].as_u64().unwrap_or(0),
        package_count: json["package_count"].as_u64().unwrap_or(0),
        _block_count: json["block_count"].as_u64().unwrap_or(0),
        validator_count: json["validator_count"].as_u64().unwrap_or(0) as usize,
        total_stake: json["total_stake"].as_u64().unwrap_or(0),
        peer_count: json["peer_count"].as_u64().unwrap_or(0) as usize,
        bridge_status: json["bridge_status"]
            .as_str()
            .unwrap_or("Unknown")
            .to_string(),
        l1_block: json["l1_block"].as_u64().unwrap_or(0),
        finalized_height: json.get("finalization_source").and_then(|_| json["finalized_height"].as_u64()),
        finalization_lag: json.get("finalization_source").and_then(|_| json["finalization_lag"].as_u64()),
        validator_set_source: json["validator_set_source"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        validator_set_sync_state: json["validator_set_sync_state"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
    })
}

async fn fetch_block(client: &reqwest::Client, api_base: &str, height: u64) -> Result<BlockInfo> {
    let json = get_json_with_fallback(
        client,
        format!("{}/v1/public/blocks/{}", api_base, height),
        Some(format!("{}/v1/blocks/{}", api_base, height)),
        ApiRouteScope::Public,
    )
    .await?;

    let header = &json["header"];
    let txs: Vec<TransactionInfo> = json["transactions"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|t| {
                    let record = t.get("record").unwrap_or(t);
                    let id = &record["id"];
                    TransactionInfo {
                    _id: t["package_canonical"].as_str().map(str::to_string).unwrap_or_else(|| format!("{}:{}@{}", id["ecosystem"].as_str().unwrap_or("?"), id["name"].as_str().unwrap_or("?"), id["version"].as_str().unwrap_or("?"))),
                    tx_type: t["type"].as_str().unwrap_or("unknown").to_string(),
                    package_name: id["name"].as_str().map(|s| s.to_string()),
                    package_version: id["version"].as_str().map(|s| s.to_string()),
                    publisher: record["publisher_pubkey"]
                        .as_str()
                        .map(|s| s[..8.min(s.len())].to_string()),
                    status: if t["type"] == "revoke" { "revoked" } else { record["status"].as_str().unwrap_or("unknown") }.to_string(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(BlockInfo {
        height: header["height"].as_u64().unwrap_or(0),
        hash: json["hash"].as_str().unwrap_or("").to_string(),
        timestamp: header["timestamp"].as_str().unwrap_or("").to_string(),
        proposer: header["proposer_id"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        tx_count: txs.len(),
        transactions: txs,
        merkle_root: header["merkle_root"].as_str().unwrap_or("").to_string(),
    })
}

async fn fetch_validators(client: &reqwest::Client, api_base: &str) -> Result<Vec<ValidatorInfo>> {
    let json = get_json_with_fallback(
        client,
        format!("{}/v1/validator/consensus/state", api_base),
        Some(format!("{}/v1/consensus/state", api_base)),
        ApiRouteScope::Validator,
    )
    .await?;
    let validators = json["validators"].as_array().cloned().unwrap_or_default();

    Ok(validators
        .iter()
        .map(|v| ValidatorInfo {
            id: v["id"].as_str().unwrap_or("unknown").to_string(),
            alias: v["alias"].as_str().unwrap_or("").to_string(),
            stake: v["stake"].as_u64().unwrap_or(0),
            reputation: v["reputation"].as_u64().unwrap_or(50) as u8,
            status: v["status"].as_str().unwrap_or("unknown").to_string(),
            is_active: true,
            pub_key: v["pubkey"].as_str().unwrap_or("").to_string(),
        })
        .collect())
}

async fn fetch_peers(client: &reqwest::Client, api_base: &str) -> Result<Vec<String>> {
    let json = get_json_with_fallback(
        client,
        format!("{}/v1/operator/p2p/status", api_base),
        Some(format!("{}/v1/p2p/status", api_base)),
        ApiRouteScope::Operator,
    )
    .await?;

    Ok(json["peers"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default())
}

/// Fetch packages from both the verified list and the pending pool, merge them.
///
/// `/v1/packages?limit=50` returns full package objects for finalized packages.
/// `/v1/pending` returns a bare list of canonical IDs still in the mempool.
/// We show verified packages first (richest data), then pending ones.
async fn fetch_pending_packages(
    client: &reqwest::Client,
    api_base: &str,
) -> Result<Vec<PackageInfo>> {
    let mut packages: Vec<PackageInfo> = Vec::new();

    // 1. Finalized/verified packages — full objects from the chain store.
    if let Ok(json) = get_json_with_fallback(
        client,
        format!("{}/v1/public/packages?limit=50", api_base),
        Some(format!("{}/v1/packages?limit=50", api_base)),
        ApiRouteScope::Public,
    )
    .await
    {
        if let Some(arr) = json["packages"].as_array() {
            for v in arr {
                let canonical = v["canonical"].as_str().unwrap_or_default();
                if canonical.is_empty() {
                    continue;
                }
                // canonical is e.g. "npm/express@4.18.0"
                let parts: Vec<&str> = canonical.splitn(2, ':').collect();
                let ecosystem = parts.first().copied().unwrap_or("?").to_string();
                let name_ver = parts.get(1).copied().unwrap_or(canonical);
                let (pkg_name, version) = if let Some(pos) = name_ver.rfind('@') {
                    (&name_ver[..pos], &name_ver[pos + 1..])
                } else {
                    (name_ver, "?")
                };
                packages.push(PackageInfo {
                    name: pkg_name.to_string(),
                    ecosystem,
                    version: version.to_string(),
                    status: v["status"].as_str().unwrap_or("verified").to_string(),
                    publisher: v["publisher"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(16)
                        .collect(),
                    verified_at: v["published_at"].as_str().map(|s| s.to_string()),
                    content_hash: v["content_hash"].as_str().unwrap_or("").to_string(),
                });
            }
        }
    }

    // 2. Pending pool — bare canonical ID strings not yet in the chain store.
    if let Ok(json) = get_json_with_fallback(
        client,
        format!("{}/v1/operator/pending", api_base),
        Some(format!("{}/v1/pending", api_base)),
        ApiRouteScope::Operator,
    )
    .await
    {
        if let Some(arr) = json["packages"].as_array() {
            for v in arr {
                let canonical = v.as_str().unwrap_or_default().to_string();
                if canonical.is_empty() {
                    continue;
                }
                // Skip if already present from the verified list.
                if packages
                    .iter()
                    .any(|p| format!("{}:{}@{}", p.ecosystem, p.name, p.version) == canonical)
                {
                    continue;
                }
                let parts: Vec<&str> = canonical.splitn(2, ':').collect();
                let ecosystem = parts.first().copied().unwrap_or("?").to_string();
                let name_ver = parts.get(1).copied().unwrap_or(&canonical);
                let (pkg_name, version) = if let Some(pos) = name_ver.rfind('@') {
                    (&name_ver[..pos], &name_ver[pos + 1..])
                } else {
                    (name_ver, "?")
                };
                packages.push(PackageInfo {
                    name: pkg_name.to_string(),
                    ecosystem,
                    version: version.to_string(),
                    status: "pending".to_string(),
                    publisher: String::new(),
                    verified_at: None,
                    content_hash: String::new(),
                });
            }
        }
    }

    Ok(packages)
}

/// Fetch pending-pool entries for the Mempool view.
///
/// Calls `/v1/pending` which returns `{count, packages: ["npm/foo@1.0", …]}`.
/// Each canonical string is parsed into a `MempoolTx` for display.
async fn fetch_mempool(client: &reqwest::Client, api_base: &str) -> Result<Vec<MempoolTx>> {
    let json = get_json_with_fallback(
        client,
        format!("{}/v1/operator/pending", api_base),
        Some(format!("{}/v1/pending", api_base)),
        ApiRouteScope::Operator,
    )
    .await?;

    let mut txs = Vec::new();
    if let Some(arr) = json["packages"].as_array() {
        let now = Instant::now();
        for v in arr {
            let canonical = v.as_str().unwrap_or_default();
            if canonical.is_empty() {
                continue;
            }
            // Parse "npm/express@4.18.0" → ecosystem + name + version
            let (ecosystem, rest) = canonical.split_once(':').unwrap_or(("unknown", canonical));
            let (_name, _version) = rest.rsplit_once('@').unwrap_or((rest, "?"));
            txs.push(MempoolTx {
                id: canonical.to_string(),
                tx_type: format!("publish({})", ecosystem),
                _size: canonical.len(), // approximate — no real size available
                timestamp: now,
            });
        }
    }
    Ok(txs)
}

/// Fetch PBFT consensus round state.
/// Tries `/v1/consensus/state`; if that returns an error or is not implemented,
/// derives a best-effort snapshot from `/v1/chain/stats` and `/v1/validators`.
async fn fetch_runtime_config(client: &reqwest::Client, api_base: &str) -> Result<RuntimeIdentity> {
    if operator_api_key().is_none() {
        let stats = get_json_with_fallback(client,
            format!("{api_base}/v1/public/chain/stats"), Some(format!("{api_base}/v1/chain/stats")), ApiRouteScope::Public).await?;
        let health = get_json_with_fallback(client,
            format!("{api_base}/v1/public/health"), Some(format!("{api_base}/v1/health")), ApiRouteScope::Public).await?;
        return Ok(RuntimeIdentity {
            node_id: health["node_id"].as_str().unwrap_or("unknown").into(),
            validator_pubkey: None,
            chain_id: stats["chain_id"].as_str().unwrap_or("unknown").into(),
            network: "unknown".into(),
            genesis_hash: stats["genesis_hash"].as_str().map(str::to_string),
            spec_signature_verified: stats["spec_signature_verified"].as_bool().unwrap_or(false),
            l1_rpc_configured: stats["l1_rpc_configured"].as_bool().unwrap_or(false),
            is_validator: health["is_validator"].as_bool().unwrap_or(false),
            validator_set_source: stats["validator_set_source"].as_str().unwrap_or("unknown").into(),
            validator_set_sync_state: stats["validator_set_sync_state"].as_str().unwrap_or("unknown").into(),
        });
    }
    let json = get_json_with_fallback(
        client,
        format!("{}/v1/operator/runtime/config", api_base),
        Some(format!("{}/v1/runtime/config", api_base)),
        ApiRouteScope::Operator,
    )
    .await?;

    Ok(RuntimeIdentity {
        node_id: json["node_id"].as_str().unwrap_or("unknown").to_string(),
        validator_pubkey: json["validator_pubkey"].as_str().map(|s| s.to_string()),
        chain_id: json["chain_id"].as_str().unwrap_or("unknown").to_string(),
        network: json["network"].as_str().unwrap_or("unknown").to_string(),
        genesis_hash: json["genesis_hash"].as_str().map(|s| s.to_string()),
        spec_signature_verified: json["spec_signature_verified"].as_bool().unwrap_or(false),
        l1_rpc_configured: json["l1_rpc_configured"].as_bool().unwrap_or(false),
        is_validator: json["is_validator"].as_bool().unwrap_or(false),
        validator_set_source: json["validator_set_source"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        validator_set_sync_state: json["validator_set_sync_state"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
    })
}

async fn fetch_consensus_state(client: &reqwest::Client, api_base: &str) -> Result<ConsensusState> {
    // Try the dedicated endpoint first.
    if let Ok(json) = get_json_with_fallback(
        client,
        format!("{}/v1/validator/consensus/state", api_base),
        Some(format!("{}/v1/consensus/state", api_base)),
        ApiRouteScope::Validator,
    )
    .await
    {
        let total = json["total_validators"].as_u64().unwrap_or(0) as usize;
        let round = json["active_rounds"].as_array().and_then(|rounds| rounds.first());
        let round_data = round.unwrap_or(&json);
        return Ok(ConsensusState {
            round: round_data["height"].as_u64().or_else(|| round_data["round"].as_u64()).unwrap_or(0),
            phase: round_data["phase"].as_str().unwrap_or(if round.is_some() { "UNKNOWN" } else { "IDLE" }).to_uppercase(),
            proposer: round_data["proposer"]
                .as_str()
                .unwrap_or("unknown")
                .chars()
                .take(16)
                .collect::<String>()
                + "…",
            prepare_votes: round_data["prepare_votes"].as_u64().unwrap_or(0) as usize,
            commit_votes: round_data["commit_votes"].as_u64().unwrap_or(0) as usize,
            quorum: json["quorum"].as_u64().unwrap_or((total * 2 / 3 + 1) as u64) as usize,
            total_validators: total,
            fetched_at: Instant::now(),
        });
    }

    // Fallback: derive from stats endpoint.
    let json = get_json_with_fallback(
        client,
        format!("{}/v1/public/chain/stats", api_base),
        Some(format!("{}/v1/chain/stats", api_base)),
        ApiRouteScope::Public,
    )
    .await?;

    let tip = json["tip_height"].as_u64().unwrap_or(0);
    let total = json["validator_count"].as_u64().unwrap_or(0) as usize;
    let quorum = if total > 0 { total * 2 / 3 + 1 } else { 1 };

    Ok(ConsensusState {
        round: tip.saturating_add(1),
        phase: "UNAVAILABLE".to_string(),
        proposer: "unknown".to_string(),
        prepare_votes: 0,
        commit_votes: 0,
        quorum,
        total_validators: total,
        fetched_at: Instant::now(),
    })
}

async fn sse_event_listener(
    _app: Arc<RwLock<App>>,
    api_base: String,
    tx: mpsc::Sender<DataUpdate>,
) {
    const RETRY_SECS: u64 = 5;
    let client = reqwest::Client::new();

    loop {
        let res = match open_stream_with_fallback(
            &client,
            format!("{}/v1/public/events", api_base),
            Some(format!("{}/v1/events", api_base)),
            ApiRouteScope::Public,
        )
        .await
        {
            Ok(r) => r,
            Err(_) => {
                // Connection refused / DNS failure — node is unreachable.
                let _ = tx
                    .send(DataUpdate::NodeDisconnected {
                        retry_in_secs: RETRY_SECS,
                    })
                    .await;
                tokio::time::sleep(Duration::from_secs(RETRY_SECS)).await;
                continue;
            }
        };

        // Successfully opened the stream — mark as connected.
        let _ = tx.send(DataUpdate::NodeConnected).await;

        let mut stream = res.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(_) => break,
            };

            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE messages (double newline delimited)
            while let Some(pos) = buffer.find("\n\n") {
                let message = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();

                let mut event_type = String::from("Event");
                let mut data = String::new();

                for line in message.lines() {
                    if let Some(t) = line.strip_prefix("event: ") {
                        event_type = t.trim().to_string();
                    } else if let Some(d) = line.strip_prefix("data: ") {
                        data = d.trim().to_string();
                    }
                }

                if !data.is_empty() {
                    // Try to extract a human-readable summary from JSON data
                    let summary = if let Ok(json) = serde_json::from_str::<Value>(&data) {
                        json["message"]
                            .as_str()
                            .or(json["type"].as_str())
                            .unwrap_or(&data)
                            .to_string()
                    } else {
                        data
                    };
                    let _ = tx.send(DataUpdate::Event(event_type, summary)).await;
                }
            }
        }

        // Stream closed — notify UI and schedule reconnect.
        let _ = tx
            .send(DataUpdate::NodeDisconnected {
                retry_in_secs: RETRY_SECS,
            })
            .await;
        tokio::time::sleep(Duration::from_secs(RETRY_SECS)).await;
    }
}

fn refresh_preflight(app: &mut App) {
    let mut checks = Vec::with_capacity(9);
    let identity = app.runtime_identity.as_ref();
    let spec_verified = identity
        .map(|id| id.spec_signature_verified)
        .unwrap_or(app.stats.spec_signature_verified);

    checks.push(PreflightCheck {
        name: "Signed chain spec".into(),
        status: if spec_verified {
            PreflightStatus::Pass
        } else if identity.is_none() && app.last_api_success_at.is_some() {
            PreflightStatus::Warn
        } else if identity.is_some() {
            PreflightStatus::Fail
        } else {
            PreflightStatus::Pending
        },
        detail: if spec_verified {
            "detached signature verified at node startup".into()
        } else if identity.is_none() && app.last_api_success_at.is_some() {
            "public API did not expose signature status; operator auth may be required".into()
        } else if identity.is_some() {
            "node did not report a verified chain-spec signature".into()
        } else {
            "waiting for node API".into()
        },
    });

    let genesis_hash = identity
        .and_then(|id| id.genesis_hash.as_deref())
        .or(app.stats.genesis_hash.as_deref());
    match genesis_hash {
        Some(hash) if !hash.trim().is_empty() => checks.push(PreflightCheck {
            name: "Genesis identity".into(),
            status: PreflightStatus::Pass,
            detail: format!("{}", short_hash(hash)),
        }),
        Some(_) => checks.push(PreflightCheck {
            name: "Genesis identity".into(),
            status: PreflightStatus::Fail,
            detail: "node reported an empty genesis hash".into(),
        }),
        None => checks.push(PreflightCheck {
            name: "Genesis identity".into(),
            status: PreflightStatus::Pending,
            detail: "waiting for runtime configuration".into(),
        }),
    }

    let l1_rpc_configured = identity
        .map(|id| id.l1_rpc_configured)
        .unwrap_or(app.stats.l1_rpc_configured);
    checks.push(PreflightCheck {
        name: "L1 RPC".into(),
        status: if l1_rpc_configured {
            PreflightStatus::Pass
        } else if identity.is_none() && app.last_api_success_at.is_some() {
            PreflightStatus::Warn
        } else if identity.is_some() {
            PreflightStatus::Warn
        } else {
            PreflightStatus::Pending
        },
        detail: if l1_rpc_configured {
            "endpoint configured (URL hidden)".into()
        } else {
            "no L1 RPC endpoint reported".into()
        },
    });

    let peer_count = app.peer_ids.len().max(app.stats.peer_count);
    checks.push(PreflightCheck {
        name: "Peer connectivity".into(),
        status: if peer_count > 0 {
            PreflightStatus::Pass
        } else {
            PreflightStatus::Warn
        },
        detail: if peer_count > 0 {
            format!("{} peer(s) connected", peer_count)
        } else {
            "0 peers; configure independent validator/bootnode seeds".into()
        },
    });

    let api_status = if rest_api_ready(app) {
        PreflightStatus::Pass
    } else if app.last_api_error.is_some() {
        PreflightStatus::Fail
    } else {
        PreflightStatus::Pending
    };
    checks.push(PreflightCheck {
        name: "Node REST API".into(),
        status: api_status,
        detail: if rest_api_ready(app) {
            "live response received".into()
        } else if let Some(error) = &app.last_api_error {
            error.clone()
        } else if app.last_api_success_at.is_some() {
            "last successful response is stale".into()
        } else {
            "waiting for the first successful response".into()
        },
    });

    let consensus_status = match app.consensus_state.as_ref() {
        Some(consensus) if consensus.fetched_at.elapsed() <= Duration::from_secs(15) => {
            PreflightStatus::Pass
        }
        Some(_) => PreflightStatus::Warn,
        None => PreflightStatus::Pending,
    };
    checks.push(PreflightCheck {
        name: "Consensus health".into(),
        status: consensus_status,
        detail: app
            .consensus_state
            .as_ref()
            .map(|state| format!("round {} / {}", state.round, state.phase))
            .unwrap_or_else(|| "waiting for consensus snapshot".into()),
    });

    match identity {
        Some(id) if id.is_validator => {
            let has_pubkey = id
                .validator_pubkey
                .as_deref()
                .is_some_and(|key| !key.is_empty());
            let listed_by_key = id.validator_pubkey.as_deref().is_some_and(|key| {
                app.validators
                    .iter()
                    .any(|validator| validator.pub_key == key)
            });
            let listed_by_node_id = app
                .validators
                .iter()
                .any(|validator| validator.id == id.node_id && validator.is_active);
            let listed = if has_pubkey { listed_by_key } else { listed_by_node_id };
            checks.push(PreflightCheck {
                name: "Validator identity".into(),
                status: if has_pubkey && listed {
                    PreflightStatus::Pass
                } else {
                    PreflightStatus::Warn
                },
                detail: if !has_pubkey {
                    if listed_by_node_id {
                        "node ID matches an active validator; operator-only key details are unavailable".into()
                    } else {
                        "operator-only key details are unavailable; node ID is not in the active validator set".into()
                    }
                } else if listed {
                    "public identity is present in the active set".into()
                } else {
                    "public identity is configured but not in the observed active set".into()
                },
            });
        }
        Some(_) => checks.push(PreflightCheck {
            name: "Validator identity".into(),
            status: PreflightStatus::Warn,
            detail: "observer mode; validator signing is disabled".into(),
        }),
        None => checks.push(PreflightCheck {
            name: "Validator identity".into(),
            status: PreflightStatus::Pending,
            detail: "waiting for runtime configuration".into(),
        }),
    }

    let sync_detail = identity
        .map(|id| {
            format!(
                "{} / {}",
                id.validator_set_source, id.validator_set_sync_state
            )
        })
        .unwrap_or_else(|| "waiting for validator-set status".into());
    let sync_status = match identity.map(|id| id.validator_set_sync_state.to_ascii_lowercase()) {
        Some(state) if ["ready", "synced", "healthy", "active"].contains(&state.as_str()) => {
            PreflightStatus::Pass
        }
        Some(state) if state == "unknown" => PreflightStatus::Pending,
        Some(_) => PreflightStatus::Warn,
        None => PreflightStatus::Pending,
    };
    checks.push(PreflightCheck {
        name: "Validator-set sync".into(),
        status: sync_status,
        detail: sync_detail,
    });

    checks.push(PreflightCheck {
        name: "Action safety".into(),
        status: PreflightStatus::Pass,
        detail: "read-only console; validator mutations require authenticated external tooling"
            .into(),
    });

    app.preflight = PreflightState {
        checks,
        updated_at: Instant::now(),
    };
}

fn short_hash(value: &str) -> String {
    if value.len() <= 18 {
        value.to_string()
    } else {
        format!(
            "{}…{}",
            &value[..10],
            &value[value.len().saturating_sub(8)..]
        )
    }
}

fn rest_api_ready(app: &App) -> bool {
    app.last_api_error.is_none()
        && app.last_api_success_at.is_some_and(|at| at.elapsed() <= Duration::from_secs(15))
}

fn apply_data_update(app: &mut App, update: DataUpdate) {
    match update {
        DataUpdate::Stats(stats) => {
            if stats.genesis_hash != app.stats.genesis_hash {
                app.blocks.clear();
                app.tps_history.clear();
            }
            app.stats = stats;
        }
        DataUpdate::Block(block) => {
            if !app.blocks.iter().any(|b| b.height == block.height) {
                // Update TPS history before moving block
                app.tps_history.push_front(block.tx_count as u64);
                while app.tps_history.len() > 60 {
                    app.tps_history.pop_back();
                }
                app.blocks.push_front(block);
                while app.blocks.len() > MAX_BLOCKS {
                    app.blocks.pop_back();
                }
            }
        }
        DataUpdate::Validators(vals) => {
            app.validators = vals;
            app.stats.validator_count = app.validators.len();
            app.stats.total_stake = app.validators.iter().map(|validator| validator.stake).sum();
        }
        DataUpdate::Packages(pkgs) => {
            app.packages = pkgs;
        }
        DataUpdate::Event(event_type, message) => {
            app.push_event(event_type, message);
        }
        DataUpdate::MempoolSnapshot(txs) => {
            app.mempool = txs;
        }
        DataUpdate::Peers(peers) => {
            app.peer_ids = peers;
        }
        DataUpdate::NodeConnected => {
            app.node_connected = true;
            app.last_event_at = Instant::now();
            app.reconnect_in = 0;
        }
        DataUpdate::NodeDisconnected { retry_in_secs } => {
            app.node_connected = false;
            app.reconnect_in = retry_in_secs;
        }
        DataUpdate::Consensus(state) => {
            app.consensus_state = Some(state);
        }
        DataUpdate::MetricsHistory { samples, status } => {
            app.metrics_history = samples;
            app.metrics_history_status = status;
        }
        DataUpdate::FaucetHealth {
            healthy,
            token_reserve,
        } => {
            app.faucet.health = if healthy { "online" } else { "offline" }.into();
            app.faucet.faucet_token_reserve = token_reserve;
        }
        DataUpdate::FaucetNetwork(info) => {
            app.faucet.network = Some(info);
        }
        DataUpdate::FaucetBalance {
            token,
            native,
            token_fmt,
            native_fmt,
        } => {
            let had_data =
                token.is_some() || native.is_some() || token_fmt.is_some() || native_fmt.is_some();
            app.faucet.last_balance = token.clone();
            app.faucet.last_native_balance = native.clone();
            app.faucet.last_balance_fmt = token_fmt.clone();
            app.faucet.last_native_balance_fmt = native_fmt.clone();
            // Prefer the pre-formatted decimal strings for display so the
            // Status line matches what the web explorer shows (no 18-decimal
            // wei blob). Fall back to format_wei() on the raw value if the
            // faucet omitted the formatted field.
            let display_t = token_fmt
                .clone()
                .or_else(|| token.as_deref().map(format_wei))
                .unwrap_or_else(|| "?".into());
            let display_n = native_fmt
                .clone()
                .or_else(|| native.as_deref().map(format_wei))
                .unwrap_or_else(|| "?".into());
            if matches!(app.faucet.status, FaucetStatus::Working { .. }) {
                app.faucet.status = if had_data {
                    FaucetStatus::Success {
                        tx_hash: "(balance fetched)".into(),
                        amount: format!("tCREG={} / ETH={}", display_t, display_n),
                        at: Instant::now(),
                    }
                } else {
                    FaucetStatus::Failed {
                        error: "faucet returned empty balance response".into(),
                        at: Instant::now(),
                    }
                };
            }
        }
        DataUpdate::FaucetError(error) => {
            app.faucet.status = FaucetStatus::Failed {
                error,
                at: Instant::now(),
            };
        }
        DataUpdate::FaucetDripProgress(msg) => {
            if matches!(app.faucet.status, FaucetStatus::Working { .. }) {
                if let FaucetStatus::Working { started_at, .. } = app.faucet.status {
                    app.faucet.status = FaucetStatus::Working {
                        started_at,
                        message: msg,
                    };
                }
            } else {
                app.faucet.status = FaucetStatus::Working {
                    started_at: Instant::now(),
                    message: msg,
                };
            }
        }
        DataUpdate::FaucetDripResult(result) => match result {
            Ok((tx_hash, amount)) => {
                app.faucet.status = FaucetStatus::Success {
                    tx_hash,
                    amount,
                    at: Instant::now(),
                };
            }
            Err(error) => {
                app.faucet.status = FaucetStatus::Failed {
                    error,
                    at: Instant::now(),
                };
            }
        },
        DataUpdate::RuntimeIdentity(identity) => {
            app.runtime_identity = Some(identity);
        }
        DataUpdate::ApiHealthy => {
            app.last_api_success_at = Some(Instant::now());
            app.last_api_error = None;
        }
        DataUpdate::ApiError(error) => {
            app.last_api_error = Some(error);
        }
    }
    refresh_preflight(app);
}

fn export_diagnostics(app: &App) -> Result<PathBuf> {
    let directory = std::env::var_os("CREG_DIAGNOSTICS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("diagnostics"));
    fs::create_dir_all(&directory)?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let path = directory.join(format!("creg-console-{timestamp}.json"));

    let checks: Vec<Value> = app
        .preflight
        .checks
        .iter()
        .map(|check| {
            serde_json::json!({
                "name": check.name,
                "status": check.status.label(),
                "detail": check.detail,
            })
        })
        .collect();
    let identity = app.runtime_identity.as_ref().map(|id| {
        serde_json::json!({
            "node_id": id.node_id,
            "validator_pubkey": id.validator_pubkey,
            "chain_id": id.chain_id,
            "network": id.network,
            "genesis_hash": id.genesis_hash,
            "spec_signature_verified": id.spec_signature_verified,
            "l1_rpc_configured": id.l1_rpc_configured,
            "is_validator": id.is_validator,
            "validator_set_source": id.validator_set_source,
            "validator_set_sync_state": id.validator_set_sync_state,
        })
    });
    let events: Vec<Value> = app
        .events
        .iter()
        .take(50)
        .map(|(at, event_type, message)| {
            serde_json::json!({
                "age_seconds": at.elapsed().as_secs(),
                "type": event_type,
                "message": message,
            })
        })
        .collect();
    let consensus = app.consensus_state.as_ref().map(|state| {
        serde_json::json!({
            "round": state.round,
            "phase": state.phase,
            "proposer": state.proposer,
            "prepare_votes": state.prepare_votes,
            "commit_votes": state.commit_votes,
            "quorum": state.quorum,
            "total_validators": state.total_validators,
            "age_seconds": state.fetched_at.elapsed().as_secs(),
        })
    });
    let report = serde_json::json!({
        "format": "creg-console-diagnostics-v1",
        "generated_at_unix": timestamp,
        "read_only": app.read_only,
        "node_connected": app.node_connected,
        "reconnect_in_seconds": app.reconnect_in,
        "api_last_success_age_seconds": app.last_api_success_at.map(|at| at.elapsed().as_secs()),
        "api_error": app.last_api_error,
        "stats": {
            "chain_id": app.stats.chain_id,
            "genesis_hash": app.stats.genesis_hash,
            "spec_signature_verified": app.stats.spec_signature_verified,
            "l1_rpc_configured": app.stats.l1_rpc_configured,
            "tip_height": app.stats.tip_height,
            "package_count": app.stats.package_count,
            "validator_count": app.stats.validator_count,
            "total_stake": app.stats.total_stake,
            "peer_count": app.stats.peer_count,
            "bridge_status": app.stats.bridge_status,
            "l1_block": app.stats.l1_block,
            "finalized_height": app.stats.finalized_height,
            "finalization_lag": app.stats.finalization_lag,
            "validator_set_source": app.stats.validator_set_source,
            "validator_set_sync_state": app.stats.validator_set_sync_state,
        },
        "peer_ids": app.peer_ids,
        "runtime_identity": identity,
        "consensus": consensus,
        "preflight": checks,
        "recent_events": events,
        "operator_auth_configured": operator_api_key().is_some(),
        "private_keys": "never collected by the console",
    });
    fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    Ok(path)
}

// ============================================================================
// INPUT HANDLING
// ============================================================================

fn move_filtered(selected: &mut usize, filtered: &[usize], down: bool) {
    if filtered.is_empty() {
        return;
    }
    let pos = filtered
        .iter()
        .position(|idx| *idx == *selected)
        .unwrap_or(0);
    let next = if down {
        (pos + 1).min(filtered.len() - 1)
    } else {
        pos.saturating_sub(1)
    };
    *selected = filtered[next];
}

fn page_filtered(selected: &mut usize, filtered: &[usize], down: bool) {
    if filtered.is_empty() {
        return;
    }
    let pos = filtered
        .iter()
        .position(|idx| *idx == *selected)
        .unwrap_or(0);
    let next = if down {
        (pos + 10).min(filtered.len() - 1)
    } else {
        pos.saturating_sub(10)
    };
    *selected = filtered[next];
}

fn snap_to_filter(selected: &mut usize, filtered: &[usize]) {
    if filtered.is_empty() {
        return;
    }
    if !filtered.contains(selected) {
        *selected = filtered[0];
    }
}

fn request_quit(app: &mut App) -> bool {
    if is_detail_view(app.current_view) {
        app.current_view = app.previous_view.unwrap_or(View::Overview);
        app.previous_view = None;
        return false;
    }
    if app.current_view == View::Help {
        app.current_view = app.previous_view.unwrap_or(View::Overview);
        app.previous_view = None;
        return false;
    }
    if let Some(armed) = app.quit_armed_at {
        if armed.elapsed() < Duration::from_secs(QUIT_ARM_SECS) {
            return true;
        }
    }
    app.quit_armed_at = Some(Instant::now());
    app.push_toast(ToastKind::Warn, "Press q again to quit");
    false
}

fn open_help(app: &mut App) {
    if app.current_view == View::Help {
        app.current_view = app.previous_view.unwrap_or(View::Overview);
        app.previous_view = None;
    } else {
        app.previous_view = Some(app.current_view);
        app.current_view = View::Help;
    }
}

fn export_and_toast(app: &mut App) {
    match export_diagnostics(app) {
        Ok(path) => app.push_toast(ToastKind::Success, format!("exported {}", path.display())),
        Err(error) => app.push_toast(ToastKind::Error, format!("export failed: {error}")),
    }
}

fn selection_yank_text(app: &App) -> Option<(String, String)> {
    match app.current_view {
        View::Blocks | View::Overview | View::BlockDetail => app
            .selected_block()
            .map(|block| (format!("block #{}", block.height), block.hash.clone())),
        View::Validators | View::ValidatorDetail => app.selected_validator().map(|validator| {
            (
                validator.alias.clone(),
                if validator.pub_key.is_empty() {
                    validator.id.clone()
                } else {
                    validator.pub_key.clone()
                },
            )
        }),
        View::Packages | View::PackageDetail => app.selected_package().map(|pkg| {
            (
                pkg.name.clone(),
                if pkg.content_hash.is_empty() {
                    format!("{}@{}", pkg.name, pkg.version)
                } else {
                    pkg.content_hash.clone()
                },
            )
        }),
        _ => Some((
            "chain".into(),
            format!("{}#{}", app.stats.chain_id, app.stats.tip_height),
        )),
    }
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i];
        let b1 = if i + 1 < input.len() { input[i + 1] } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] } else { 0 };
        let triple = ((b0 as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;
        out.push(TABLE[((triple >> 18) & 63) as usize] as char);
        out.push(TABLE[((triple >> 12) & 63) as usize] as char);
        if i + 1 < input.len() {
            out.push(TABLE[((triple >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < input.len() {
            out.push(TABLE[(triple & 63) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

fn copy_to_clipboard(text: &str) -> Result<(), String> {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        if clipboard.set_text(text.to_string()).is_ok() {
            return Ok(());
        }
    }
    let encoded = encode_base64(text.as_bytes());
    write!(io::stdout(), "\x1b]52;c;{encoded}\x07").map_err(|err| err.to_string())?;
    io::stdout().flush().map_err(|err| err.to_string())
}

fn yank_selection(app: &mut App) {
    match selection_yank_text(app) {
        Some((label, text)) if !text.is_empty() => match copy_to_clipboard(&text) {
            Ok(()) => app.push_toast(ToastKind::Success, format!("copied {label}")),
            Err(error) => app.push_toast(ToastKind::Error, format!("copy failed: {error}")),
        },
        _ => app.push_toast(ToastKind::Warn, "nothing to copy"),
    }
}

fn refresh_now(app: &mut App) {
    FORCE_REFRESH.store(true, Ordering::Relaxed);
    app.push_toast(ToastKind::Info, "refreshing node data");
}

fn execute_palette_id(app: &mut App, id: &str) -> bool {
    app.palette.open = false;
    app.palette.query.clear();
    app.palette.selected = 0;
    match id {
        "overview" => app.current_view = View::Overview,
        "blocks" => app.current_view = View::Blocks,
        "validators" => app.current_view = View::Validators,
        "packages" => app.current_view = View::Packages,
        "network" => app.current_view = View::Network,
        "mempool" => app.current_view = View::Mempool,
        "events" => app.current_view = View::Events,
        "operator" => app.current_view = View::Operator,
        "consensus" => app.current_view = View::Consensus,
        "faucet" => app.current_view = View::Faucet,
        "preflight" => app.current_view = View::Preflight,
        "bridge" => app.current_view = View::Bridge,
        "metrics" => app.current_view = View::Metrics,
        "search" => {
            app.is_searching = true;
            app.search_query.clear();
        }
        "yank" => yank_selection(app),
        "export" => export_and_toast(app),
        "refresh" => refresh_now(app),
        "profile" => {
            let msg = match (&app.authenticated_wallet, &app.authenticated_pubkey) {
                (Some(w), Some(k)) => {
                    let short_k = if k.len() > 12 { &k[..12] } else { k.as_str() };
                    format!("Profile: {} | Ed25519: {}…", w, short_k)
                }
                _ => "Guest session. Run 'creg login' in terminal to link Web3 wallet.".to_string(),
            };
            app.push_toast(ToastKind::Info, msg);
        }
        "theme" => {
            let current = IS_LIGHT_THEME.load(Ordering::Relaxed);
            IS_LIGHT_THEME.store(!current, Ordering::Relaxed);
            app.push_toast(
                ToastKind::Info,
                if current { "dark theme" } else { "light theme" },
            );
        }
        "help" => open_help(app),
        "quit" => return request_quit(app),
        _ => {}
    }
    false
}

async fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let code = key.code;

    if app.palette.open {
        match code {
            KeyCode::Esc => {
                app.palette.open = false;
                app.palette.query.clear();
            }
            KeyCode::Enter => {
                let filtered = filtered_palette_indices(&app.palette.query);
                if let Some(idx) = filtered.get(app.palette.selected).copied() {
                    if let Some(cmd) = palette_commands().get(idx) {
                        return execute_palette_id(app, cmd.id);
                    }
                }
            }
            KeyCode::Down => {
                let len = filtered_palette_indices(&app.palette.query).len();
                if len > 0 {
                    app.palette.selected = (app.palette.selected + 1) % len;
                }
            }
            KeyCode::Up => {
                let len = filtered_palette_indices(&app.palette.query).len();
                if len > 0 {
                    app.palette.selected = app.palette.selected.checked_sub(1).unwrap_or(len - 1);
                }
            }
            KeyCode::Backspace => {
                app.palette.query.pop();
                app.palette.selected = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                app.palette.query.push(c);
                app.palette.selected = 0;
            }
            _ => {}
        }
        return false;
    }

    if app.is_searching {
        match code {
            KeyCode::Esc => {
                app.is_searching = false;
                app.search_query.clear();
            }
            KeyCode::Enter => app.is_searching = false,
            KeyCode::Char(c) if !ctrl => app.search_query.push(c),
            KeyCode::Backspace => {
                app.search_query.pop();
            }
            _ => {}
        }
        return false;
    }

    if app.current_view == View::Faucet && app.faucet.editing {
        match code {
            KeyCode::Esc => app.faucet.editing = false,
            KeyCode::Enter => app.faucet.editing = false,
            KeyCode::Backspace => {
                app.faucet.address_input.pop();
            }
            KeyCode::Char(c) if !ctrl => {
                if app.faucet.address_input.len() < 64 {
                    app.faucet.address_input.push(c);
                }
            }
            _ => {}
        }
        return false;
    }

    if app.current_view == View::Help {
        match code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Char('h') => {
                open_help(app);
            }
            _ => {}
        }
        return false;
    }

    if (ctrl && matches!(code, KeyCode::Char('p') | KeyCode::Char('k')))
        || code == KeyCode::Char(':')
    {
        app.palette.open = true;
        app.palette.query.clear();
        app.palette.selected = 0;
        return false;
    }

    if ctrl && matches!(code, KeyCode::Char('y') | KeyCode::Char('c')) {
        yank_selection(app);
        return false;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') => return request_quit(app),
        KeyCode::Esc if app.quit_armed_at.is_some() => {
            app.quit_armed_at = None;
            app.push_toast(ToastKind::Info, "quit cancelled");
            return false;
        }
        KeyCode::Char('?') => {
            open_help(app);
            return false;
        }
        KeyCode::Char('/') => {
            app.is_searching = true;
            app.search_query.clear();
            return false;
        }
        KeyCode::Char('p') => {
            app.current_view = View::Preflight;
            return false;
        }
        KeyCode::Char('x') | KeyCode::Char('X') => {
            export_and_toast(app);
            return false;
        }
        KeyCode::Char('y') => {
            yank_selection(app);
            return false;
        }
        KeyCode::Char('r') if app.current_view != View::Faucet => {
            refresh_now(app);
            return false;
        }
        KeyCode::Char('1') => app.current_view = View::Overview,
        KeyCode::Char('2') => app.current_view = View::Blocks,
        KeyCode::Char('3') => app.current_view = View::Validators,
        KeyCode::Char('4') => app.current_view = View::Packages,
        KeyCode::Char('5') => app.current_view = View::Network,
        KeyCode::Char('6') => app.current_view = View::Mempool,
        KeyCode::Char('7') => app.current_view = View::Events,
        KeyCode::Char('8') | KeyCode::Char('o') | KeyCode::Char('O') => {
            app.current_view = View::Operator
        }
        KeyCode::Char('9') | KeyCode::Char('c') | KeyCode::Char('C') => {
            app.current_view = View::Consensus
        }
        KeyCode::Char('0') | KeyCode::Char('F') => {
            app.current_view = View::Faucet;
        }
        KeyCode::Char('m') => app.current_view = View::Metrics,
        KeyCode::Tab | KeyCode::Char('l') | KeyCode::Char(']') => {
            app.current_view = cycle_tab(app.current_view, 1);
            return false;
        }
        KeyCode::BackTab | KeyCode::Char('h') | KeyCode::Char('[') => {
            app.current_view = cycle_tab(app.current_view, -1);
            return false;
        }
        KeyCode::Char('t') | KeyCode::Char('T') => {
            let current = IS_LIGHT_THEME.load(Ordering::Relaxed);
            IS_LIGHT_THEME.store(!current, Ordering::Relaxed);
            app.push_toast(
                ToastKind::Info,
                if current { "dark theme" } else { "light theme" },
            );
            return false;
        }
        _ => {}
    }

    match app.current_view {
        View::Blocks | View::Overview => match code {
            KeyCode::Down | KeyCode::Char('j') => {
                let filtered = app.matching_blocks();
                snap_to_filter(&mut app.selected_block, &filtered);
                move_filtered(&mut app.selected_block, &filtered, true);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let filtered = app.matching_blocks();
                snap_to_filter(&mut app.selected_block, &filtered);
                move_filtered(&mut app.selected_block, &filtered, false);
            }
            KeyCode::PageDown => {
                let filtered = app.matching_blocks();
                page_filtered(&mut app.selected_block, &filtered, true);
            }
            KeyCode::PageUp => {
                let filtered = app.matching_blocks();
                page_filtered(&mut app.selected_block, &filtered, false);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if let Some(first) = app.matching_blocks().first() {
                    app.selected_block = *first;
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if let Some(last) = app.matching_blocks().last() {
                    app.selected_block = *last;
                }
            }
            KeyCode::Enter | KeyCode::Char('d') => {
                if !app.blocks.is_empty() {
                    app.previous_view = Some(app.current_view);
                    app.current_view = View::BlockDetail;
                }
            }
            _ => {}
        },
        View::Preflight => match code {
            KeyCode::Char('x') | KeyCode::Char('X') => export_and_toast(app),
            _ => {}
        },
        View::Validators => match code {
            KeyCode::Down | KeyCode::Char('j') => {
                let filtered = app.matching_validators();
                snap_to_filter(&mut app.selected_validator, &filtered);
                move_filtered(&mut app.selected_validator, &filtered, true);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let filtered = app.matching_validators();
                snap_to_filter(&mut app.selected_validator, &filtered);
                move_filtered(&mut app.selected_validator, &filtered, false);
            }
            KeyCode::PageDown => {
                let filtered = app.matching_validators();
                page_filtered(&mut app.selected_validator, &filtered, true);
            }
            KeyCode::PageUp => {
                let filtered = app.matching_validators();
                page_filtered(&mut app.selected_validator, &filtered, false);
            }
            KeyCode::Enter | KeyCode::Char('d') => {
                if !app.validators.is_empty() {
                    app.previous_view = Some(app.current_view);
                    app.current_view = View::ValidatorDetail;
                }
            }
            _ => {}
        },
        View::Packages => match code {
            KeyCode::Down | KeyCode::Char('j') => {
                let filtered = app.matching_packages();
                snap_to_filter(&mut app.selected_package, &filtered);
                move_filtered(&mut app.selected_package, &filtered, true);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let filtered = app.matching_packages();
                snap_to_filter(&mut app.selected_package, &filtered);
                move_filtered(&mut app.selected_package, &filtered, false);
            }
            KeyCode::PageDown => {
                let filtered = app.matching_packages();
                page_filtered(&mut app.selected_package, &filtered, true);
            }
            KeyCode::PageUp => {
                let filtered = app.matching_packages();
                page_filtered(&mut app.selected_package, &filtered, false);
            }
            KeyCode::Enter | KeyCode::Char('d') => {
                if !app.packages.is_empty() {
                    app.previous_view = Some(app.current_view);
                    app.current_view = View::PackageDetail;
                }
            }
            _ => {}
        },
        View::Events => match code {
            KeyCode::Down | KeyCode::Char('j') => {
                if app.selected_event < app.events.len().saturating_sub(1) {
                    app.selected_event += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.selected_event > 0 {
                    app.selected_event -= 1;
                }
            }
            _ => {}
        },
        View::BlockDetail | View::ValidatorDetail | View::PackageDetail => match code {
            KeyCode::Esc | KeyCode::Char('b') => {
                app.current_view = app.previous_view.unwrap_or(View::Overview);
                app.previous_view = None;
            }
            _ => {}
        },
        View::Faucet => match code {
            KeyCode::Char('e') | KeyCode::Char('E') | KeyCode::Char('i') => {
                app.faucet.editing = true;
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                app.faucet.address_input.clear();
                app.faucet.editing = true;
            }
            KeyCode::Char('b') | KeyCode::Char('B') => {
                let addr = app.faucet.address_input.trim().to_string();
                if !crate::faucet_client::is_valid_evm_address(&addr) {
                    app.faucet.status = FaucetStatus::Failed {
                        error: "Enter a valid 0x… address first (press 'e' to edit).".into(),
                        at: Instant::now(),
                    };
                } else {
                    app.faucet.status = FaucetStatus::Working {
                        started_at: Instant::now(),
                        message: "Fetching balance…".into(),
                    };
                    let tx = app.data_tx.clone();
                    let base = app.faucet.base.clone();
                    tokio::spawn(async move {
                        let client = reqwest::Client::new();
                        match crate::faucet_client::get_balance(&client, &base, &addr).await {
                            Ok(bal) => {
                                let _ = tx
                                    .send(DataUpdate::FaucetBalance {
                                        token: bal.balance,
                                        native: bal.native_balance,
                                        token_fmt: bal.balance_formatted,
                                        native_fmt: bal.native_balance_formatted,
                                    })
                                    .await;
                            }
                            Err(e) => {
                                let _ = tx
                                    .send(DataUpdate::FaucetError(format!(
                                        "balance fetch failed: {}",
                                        e
                                    )))
                                    .await;
                            }
                        }
                    });
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Enter => {
                let addr = app.faucet.address_input.trim().to_string();
                if !crate::faucet_client::is_valid_evm_address(&addr) {
                    app.faucet.status = FaucetStatus::Failed {
                        error: "Enter a valid 0x… address first (press 'e' to edit).".into(),
                        at: Instant::now(),
                    };
                } else if matches!(app.faucet.status, FaucetStatus::Working { .. }) {
                    // Already running — ignore
                } else {
                    app.faucet.status = FaucetStatus::Working {
                        started_at: Instant::now(),
                        message: "Requesting PoW challenge…".into(),
                    };
                    let tx = app.data_tx.clone();
                    let base = app.faucet.base.clone();
                    tokio::spawn(async move {
                        run_faucet_drip(tx, base, addr).await;
                    });
                }
            }
            _ => {}
        },
        _ => {}
    }

    false
}

fn handle_mouse(app: &mut App, mouse: crossterm::event::MouseEvent) {
    match mouse.kind {
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            let row = mouse.row;
            if row >= HEADER_HEIGHT && row < HEADER_HEIGHT + TAB_HEIGHT {
                if let Some(view) = tab_at_column(mouse.column, app.last_width.max(1)) {
                    app.current_view = view;
                }
            } else if row >= HEADER_HEIGHT + TAB_HEIGHT {
                let index = row.saturating_sub(HEADER_HEIGHT + TAB_HEIGHT + 1) as usize;
                match app.current_view {
                    View::Blocks | View::Overview => {
                        if index < app.blocks.len() {
                            app.selected_block = index;
                        }
                    }
                    View::Validators => {
                        if index < app.validators.len() {
                            app.selected_validator = index;
                        }
                    }
                    View::Packages => {
                        if index < app.packages.len() {
                            app.selected_package = index;
                        }
                    }
                    View::Events => {
                        if index < app.events.len() {
                            app.selected_event = index;
                        }
                    }
                    _ => {}
                }
            }
        }
        MouseEventKind::ScrollDown => match app.current_view {
            View::Blocks | View::Overview => {
                if app.selected_block < app.blocks.len().saturating_sub(1) {
                    app.selected_block += 1;
                }
            }
            View::Validators => {
                if app.selected_validator < app.validators.len().saturating_sub(1) {
                    app.selected_validator += 1;
                }
            }
            View::Packages => {
                let filtered = app.matching_packages();
                move_filtered(&mut app.selected_package, &filtered, true);
            }
            View::Events => {
                if app.selected_event < app.events.len().saturating_sub(1) {
                    app.selected_event += 1;
                }
            }
            _ => {}
        },
        MouseEventKind::ScrollUp => match app.current_view {
            View::Blocks | View::Overview => {
                if app.selected_block > 0 {
                    app.selected_block -= 1;
                }
            }
            View::Validators => {
                if app.selected_validator > 0 {
                    app.selected_validator -= 1;
                }
            }
            View::Packages => {
                let filtered = app.matching_packages();
                move_filtered(&mut app.selected_package, &filtered, false);
            }
            View::Events => {
                if app.selected_event > 0 {
                    app.selected_event -= 1;
                }
            }
            _ => {}
        },
        _ => {}
    }
}

// ============================================================================
// UI RENDERING
// ============================================================================

fn draw_ui(f: &mut Frame, app: &App) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Length(TAB_HEIGHT),
            Constraint::Min(0),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(f.size());

    draw_header(f, app, main_chunks[0]);
    draw_tabs(f, app, main_chunks[1]);
    draw_main_content(f, app, main_chunks[2]);
    draw_footer(f, app, main_chunks[3]);

    if app.is_searching {
        draw_search_popup(f, app);
    }
    if app.palette.open {
        draw_palette(f, app);
    }
    if let Some(toast) = &app.toast {
        draw_toast(f, toast);
    }

    if !app.node_connected || app.last_api_error.is_some() {
        draw_disconnect_banner(f, app);
    }
}

fn draw_consensus(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" ⚙  PBFT Consensus Round ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Theme::primary()));

    let inner = block.inner(area);
    f.render_widget(block, area);

    match &app.consensus_state {
        None => {
            let msg = Paragraph::new("Fetching consensus state…")
                .style(Style::default().fg(Theme::text_dim()))
                .alignment(Alignment::Center);
            f.render_widget(msg, inner);
        }
        Some(cs) => {
            // Phase colour
            let phase_color = match cs.phase.as_str() {
                "PRE-PREPARE" => Color::Cyan,
                "PREPARE" => Color::Yellow,
                "COMMIT" => Color::Green,
                "FINALIZED" => Color::LightGreen,
                _ => Color::Gray,
            };

            // Quorum gauge for PREPARE votes
            let prepare_pct = if cs.quorum > 0 {
                (cs.prepare_votes as f64 / cs.quorum as f64 * 100.0).min(100.0) as u16
            } else {
                0
            };
            let commit_pct = if cs.quorum > 0 {
                (cs.commit_votes as f64 / cs.quorum as f64 * 100.0).min(100.0) as u16
            } else {
                0
            };

            let age_secs = cs.fetched_at.elapsed().as_secs();

            // Pre-compute display strings so we don't reference temporaries.
            let round_str = format!("#{}", cs.round);
            let val_str = format!("{}", cs.total_validators);
            let quorum_str = format!("{}", cs.quorum);
            let prepare_str = format!("{}/{} ({}%)", cs.prepare_votes, cs.quorum, prepare_pct);
            let commit_str = format!("{}/{} ({}%)", cs.commit_votes, cs.quorum, commit_pct);
            let age_str = format!("{}s ago", age_secs);

            let rows = vec![
                Row::new(vec!["Round", round_str.as_str()])
                    .style(Style::default().fg(Theme::text())),
                Row::new(vec!["Phase", cs.phase.as_str()]).style(
                    Style::default()
                        .fg(phase_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Row::new(vec!["Proposer", cs.proposer.as_str()])
                    .style(Style::default().fg(Theme::accent())),
                Row::new(vec!["Validators", val_str.as_str()])
                    .style(Style::default().fg(Theme::text())),
                Row::new(vec!["Quorum (2f+1)", quorum_str.as_str()])
                    .style(Style::default().fg(Theme::text())),
                Row::new(vec!["PREPARE votes", prepare_str.as_str()]).style(Style::default().fg(
                    if prepare_pct >= 100 {
                        Color::Green
                    } else {
                        Color::Yellow
                    },
                )),
                Row::new(vec!["COMMIT votes", commit_str.as_str()]).style(Style::default().fg(
                    if commit_pct >= 100 {
                        Color::Green
                    } else {
                        Color::Yellow
                    },
                )),
                Row::new(vec!["Snapshot age", age_str.as_str()])
                    .style(Style::default().fg(Theme::text_dim())),
            ];

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(rows.len() as u16 + 2),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .split(inner);

            let table = Table::new(
                rows,
                [Constraint::Percentage(35), Constraint::Percentage(65)],
            )
            .block(Block::default().borders(Borders::NONE))
            .header(
                Row::new(vec!["Field", "Value"]).style(
                    Style::default()
                        .fg(Theme::text_dim())
                        .add_modifier(Modifier::BOLD),
                ),
            );
            f.render_widget(table, chunks[0]);

            // PREPARE progress bar
            let prepare_gauge = Gauge::default()
                .block(
                    Block::default()
                        .title(" PREPARE votes ")
                        .borders(Borders::ALL),
                )
                .gauge_style(Style::default().fg(Color::Yellow))
                .percent(prepare_pct);
            f.render_widget(prepare_gauge, chunks[1]);

            // COMMIT progress bar
            let commit_gauge = Gauge::default()
                .block(
                    Block::default()
                        .title(" COMMIT votes ")
                        .borders(Borders::ALL),
                )
                .gauge_style(Style::default().fg(Color::Green))
                .percent(commit_pct);
            f.render_widget(commit_gauge, chunks[2]);
        }
    }
}

fn draw_disconnect_banner(f: &mut Frame, app: &App) {
    let area = f.size();
    // Center a 3-row × 50-col banner at the top of the screen.
    let width: u16 = 54.min(area.width);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let banner_area = Rect {
        x,
        y: area.y,
        width,
        height: 3,
    };

    let rest_ready = rest_api_ready(app);
    let msg = if rest_ready && !app.node_connected && app.reconnect_in > 0 {
        " ⚠ EVENT STREAM RETRYING — REST API ONLINE ".to_string()
    } else if app.reconnect_in > 0 {
        format!(
            " ⚠  NODE DISCONNECTED — reconnecting in {}s ",
            app.reconnect_in
        )
    } else if app.node_connected && !rest_ready {
        " ⚠  NODE REST API OFFLINE — retrying ".to_string()
    } else if rest_ready && !app.node_connected {
        " ⚠ EVENT STREAM CONNECTING — REST API ONLINE ".to_string()
    } else {
        " ⚠  NODE DISCONNECTED — reconnecting… ".to_string()
    };

    f.render_widget(Clear, banner_area);
    let banner = Paragraph::new(msg)
        .block(
            Block::default()
                .borders(Borders::ALL)
        .border_style(Style::default().fg(if rest_ready {
            Color::Yellow
        } else {
                    Color::Red
                })),
        )
        .style(
            Style::default()
                .fg(Color::White)
            .bg(if rest_ready {
                Color::Yellow
                } else {
                    Color::Red
                })
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center);
    f.render_widget(banner, banner_area);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    let pulse = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"][(app.tick_count / 4) as usize % 8];
    let rest_ready = rest_api_ready(app);
    let live = if rest_ready { pulse } else { "○" };
    let filter = if app.search_query.is_empty() {
        String::new()
    } else {
        format!("  ·  filter “{}”", app.search_query)
    };
    let title = format!(
        " CREG  {live}  {}   height {:>6}   pkgs {:>5}   peers {:>3}{filter} ",
        app.stats.chain_id,
        app.stats.tip_height,
        format_number(app.stats.package_count),
        app.stats.peer_count
    );

    let header = Paragraph::new(title).block(rounded_block("", true)).style(
        Style::default()
            .fg(Theme::primary())
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(header, chunks[0]);

    let validator_count = app.displayed_validator_count();
    let total_stake = app.displayed_total_stake();
    let rest = if rest_ready {
        "REST ok"
    } else if app.last_api_error.is_some() {
        "REST down"
    } else {
        "REST wait"
    };
    let sse = if app.node_connected {
        "SSE live"
    } else {
        "SSE off"
    };
    let mode = if app.read_only {
        "read-only"
    } else {
        "read-write"
    };
    let auth_badge = match &app.authenticated_wallet {
        Some(w) => {
            if w.len() > 10 {
                format!("{}…{}", &w[..6], &w[w.len() - 4..])
            } else {
                w.clone()
            }
        }
        None => "guest".to_string(),
    };
    let status_text = format!(
        " id: {auth_badge}  ·  {validator_count} vals  ·  {rest}  ·  {sse}  ·  stake {}  ·  {mode} ",
        format_number(total_stake)
    );
    let status_color = if rest_ready {
        Theme::success()
    } else {
        Theme::warning()
    };
    let status = Paragraph::new(status_text)
        .block(rounded_block("", false).border_style(Style::default().fg(status_color)))
        .style(Style::default().fg(Theme::text()))
        .alignment(Alignment::Right);
    f.render_widget(status, chunks[1]);
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let tabs = console_tabs();
    let cell = (area.width / tabs.len().max(1) as u16).max(1);
    let mut spans: Vec<Span> = Vec::new();
    for (i, (view, label, hint)) in tabs.iter().enumerate() {
        let active = app.current_view == *view
            || (is_detail_view(app.current_view)
                && matches!(
                    (*view, app.current_view),
                    (View::Blocks, View::BlockDetail)
                        | (View::Validators, View::ValidatorDetail)
                        | (View::Packages, View::PackageDetail)
                ));
        let text = if cell >= 12 {
            format!(" {hint}:{label} ")
        } else {
            format!(" {hint} ")
        };
        let style = if active {
            Style::default()
                .fg(Theme::tab_active_fg())
                .bg(Theme::primary())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::text_dim())
        };
        spans.push(Span::styled(text, style));
        if i + 1 < tabs.len() {
            spans.push(Span::styled("│", Style::default().fg(Theme::border())));
        }
    }
    let tabs_line = Paragraph::new(Line::from(spans))
        .block(rounded_block(" views  ·  click or 1-0 / Tab ", false))
        .alignment(Alignment::Left);
    f.render_widget(tabs_line, area);
}

fn draw_main_content(f: &mut Frame, app: &App, area: Rect) {
    match app.current_view {
        View::Overview => draw_overview(f, app, area),
        View::Preflight => draw_preflight(f, app, area),
        View::Blocks => draw_blocks(f, app, area),
        View::BlockDetail => draw_block_detail(f, app, area),
        View::Validators => draw_validators(f, app, area),
        View::ValidatorDetail => draw_validator_detail(f, app, area),
        View::Packages => draw_packages(f, app, area),
        View::PackageDetail => draw_package_detail(f, app, area),
        View::Network => draw_network(f, app, area),
        View::Mempool => draw_mempool(f, app, area),
        View::Events => draw_events(f, app, area),
        View::Operator => draw_operator(f, app, area),
        View::Consensus => draw_consensus(f, app, area),
        View::Faucet => draw_faucet(f, app, area),
        View::Bridge => draw_bridge(f, app, area),
        View::Metrics => draw_metrics(f, app, area),
        View::Help => draw_help(f, app, area),
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let text = match app.current_view {
        View::Overview => {
            " j/k list  ·  Enter detail  ·  Tab views  ·  / filter  ·  : palette  ·  y copy  ·  ? help  ·  q quit "
        }
        View::Preflight => " x export  ·  : palette  ·  ? help  ·  q quit ",
        View::Blocks => " j/k blocks  ·  Enter detail  ·  y copy hash  ·  / filter  ·  ? help  ·  q quit ",
        View::Validators => " j/k  ·  Enter detail  ·  y copy key  ·  / filter  ·  ? help ",
        View::Packages => " j/k  ·  Enter detail  ·  y copy digest  ·  / filter  ·  ? help ",
        View::Events => " j/k scroll  ·  / filter  ·  : palette  ·  ? help ",
        View::Operator => " read-only operator surface  ·  x export  ·  : palette  ·  ? help ",
        View::Consensus => " r refresh  ·  9 PBFT  ·  : palette  ·  ? help ",
        View::Faucet => " e edit  ·  d drip  ·  b balance  ·  a clear  ·  Esc stop editing  ·  q quit ",
        View::BlockDetail | View::ValidatorDetail | View::PackageDetail => {
            " Esc/b back  ·  y copy  ·  ? help  ·  q back (twice to quit) "
        }
        View::Help => " Esc or ? to close  ·  : command palette ",
        _ => " Tab next view  ·  : palette  ·  / filter  ·  y copy  ·  r refresh  ·  ? help  ·  q quit ",
    };

    let footer = Paragraph::new(text)
        .block(rounded_block("", false))
        .style(Style::default().fg(Theme::text_dim()))
        .alignment(Alignment::Center);
    f.render_widget(footer, area);
}

// ============================================================================
// VIEW: OVERVIEW
// ============================================================================

fn draw_preflight(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);

    let identity_line = app
        .runtime_identity
        .as_ref()
        .map(|identity| {
            format!(
                "chain={}  network={}  node={}  mode={}",
                identity.chain_id,
                identity.network,
                identity.node_id,
                if identity.is_validator {
                    "validator"
                } else {
                    "observer"
                }
            )
        })
        .unwrap_or_else(|| {
            format!(
                "chain={}  network=unknown  node=unknown  mode=unknown",
                app.stats.chain_id
            )
        });
    let status_line = format!(
        "{}  |  SSE: {}  |  REST: {}  |  updated {}s ago",
        identity_line,
        if app.node_connected {
            "connected"
        } else {
            "offline"
        },
        if app.last_api_success_at.is_some() {
            "healthy"
        } else {
            "waiting"
        },
        app.preflight.updated_at.elapsed().as_secs()
    );
    let summary = Paragraph::new(status_line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" STARTUP PREFLIGHT ")
                .border_style(Style::default().fg(Theme::primary())),
        )
        .style(Style::default().fg(Theme::text()))
        .wrap(Wrap { trim: true });
    f.render_widget(summary, chunks[0]);

    let rows: Vec<Row> = app
        .preflight
        .checks
        .iter()
        .map(|check| {
            Row::new(vec![
                Cell::from(check.status.label()).style(
                    Style::default()
                        .fg(check.status.color())
                        .add_modifier(Modifier::BOLD),
                ),
                Cell::from(check.name.clone()),
                Cell::from(check.detail.clone()),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(9),
            Constraint::Length(24),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(vec!["Status", "Check", "Detail"]).style(
            Style::default()
                .fg(Theme::primary())
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Validator readiness checks ")
            .border_style(Style::default().fg(Theme::secondary())),
    );
    f.render_widget(table, chunks[1]);

    let message = app
        .diagnostics_message
        .as_deref()
        .unwrap_or("Read-only monitoring. Press x to export a redacted report; private keys are never collected.");
    let footer = Paragraph::new(message)
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Theme::text_dim()))
        .alignment(Alignment::Center);
    f.render_widget(footer, chunks[2]);
}

fn draw_overview(f: &mut Frame, app: &App, area: Rect) {
    let validator_count = app.displayed_validator_count();
    let total_stake = app.displayed_total_stake();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12), // Stats cards
            Constraint::Min(10),    // Main split
            Constraint::Length(10), // Events feed
        ])
        .split(area);

    // Stats row
    let stats_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(chunks[0]);

    draw_stat_card(
        f,
        "BLOCK HEIGHT",
        &format!("#{}", app.stats.tip_height),
        Theme::primary(),
        stats_chunks[0],
    );
    draw_stat_card(
        f,
        "PACKAGES",
        &format_number(app.stats.package_count),
        Theme::success(),
        stats_chunks[1],
    );
    draw_stat_card(
        f,
        "VALIDATORS",
        &validator_count.to_string(),
        Theme::accent(),
        stats_chunks[2],
    );
    draw_stat_card(
        f,
        "TOTAL STAKE",
        &format!("{} CREG", format_number(total_stake)),
        Theme::warning(),
        stats_chunks[3],
    );

    // Main content split
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);

    // Left: Recent blocks
    draw_blocks_list(f, app, main_chunks[0], true);

    // Right: Validator preview + TPS sparkline
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(main_chunks[1]);

    draw_validators_preview(f, app, right_chunks[0]);
    draw_tps_sparkline(f, app, right_chunks[1]);

    // Bottom: Event feed
    draw_event_feed(f, app, chunks[2]);
}

fn draw_stat_card(f: &mut Frame, label: &str, value: &str, color: Color, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = vec![
        Line::from(Span::styled(label, Style::default().fg(Theme::text_dim()))),
        Line::from(""),
        Line::from(Span::styled(
            value,
            Style::default()
                .fg(color)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
        )),
    ];

    let paragraph = Paragraph::new(text).alignment(Alignment::Center);
    f.render_widget(paragraph, inner);
}

fn draw_tps_sparkline(f: &mut Frame, app: &App, area: Rect) {
    let data: Vec<u64> = app.tps_history.iter().copied().collect();
    let max = data.iter().max().copied().unwrap_or(1).max(1);

    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" TRANSACTIONS PER BLOCK ")
                .border_style(Style::default().fg(Theme::secondary())),
        )
        .data(&data)
        .max(max)
        .style(Style::default().fg(Theme::success()));

    f.render_widget(sparkline, area);
}

fn draw_event_feed(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .events
        .iter()
        .take(8)
        .map(|(time, event_type, msg)| {
            let elapsed = time.elapsed().as_secs();
            let time_str = if elapsed < 60 {
                format!("{}s", elapsed)
            } else if elapsed < 3600 {
                format!("{}m", elapsed / 60)
            } else {
                format!("{}h", elapsed / 3600)
            };

            let color = match event_type.as_str() {
                "Block" => Theme::success(),
                "Package" => Theme::primary(),
                "Validator" => Theme::accent(),
                "Slash" => Theme::error(),
                _ => Theme::text_dim(),
            };

            let content = format!("[{:>3}] {:<10} {}", time_str, event_type, msg);
            ListItem::new(content).style(Style::default().fg(color))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" LIVE EVENTS ")
            .border_style(Style::default().fg(Theme::success())),
    );

    f.render_widget(list, area);
}

// ============================================================================
// VIEW: BLOCKS
// ============================================================================

fn draw_blocks(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    draw_blocks_list(f, app, chunks[0], false);
    draw_block_preview(f, app, chunks[1]);
}

fn draw_blocks_list(f: &mut Frame, app: &App, area: Rect, compact: bool) {
    let query = app.search_query.to_ascii_lowercase();
    let title = if compact {
        " RECENT BLOCKS "
    } else if query.is_empty() {
        " BLOCKS (j/k to navigate, Enter for details) "
    } else {
        " BLOCKS (filtered) "
    };

    let items: Vec<ListItem> = app
        .blocks
        .iter()
        .enumerate()
        .filter(|(_i, block)| {
            if query.is_empty() {
                return true;
            }
            let height_str = block.height.to_string();
            height_str.contains(&query)
                || block.hash.to_ascii_lowercase().contains(&query)
                || block.merkle_root.to_ascii_lowercase().contains(&query)
                || block.proposer.to_ascii_lowercase().contains(&query)
        })
        .map(|(i, block)| {
            let hash_short = if block.hash.len() >= 16 {
                format!("{}..", &block.hash[..16])
            } else if block.hash.is_empty() {
                if block.merkle_root.len() >= 16 {
                    format!("{}..", &block.merkle_root[..16])
                } else {
                    block.merkle_root.clone()
                }
            } else {
                block.hash.clone()
            };

            let content = if compact {
                format!(
                    "#{:<6} {}  {} txs",
                    block.height, hash_short, block.tx_count
                )
            } else {
                let time_str = format_timestamp(&block.timestamp);
                format!(
                    "#{:<8} {:<20} {:<12} {:>6} txs  {}",
                    block.height,
                    hash_short,
                    block.proposer.chars().take(12).collect::<String>(),
                    block.tx_count,
                    time_str
                )
            };

            let style = if i == app.selected_block {
                Style::default()
                    .fg(Theme::highlight())
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(Theme::text())
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Theme::primary())),
    );

    f.render_widget(list, area);
}

fn draw_block_preview(f: &mut Frame, app: &App, area: Rect) {
    let block = match app.selected_block() {
        Some(b) => b,
        None => {
            let empty = Paragraph::new("No block selected").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" BLOCK DETAILS "),
            );
            f.render_widget(empty, area);
            return;
        }
    };

    let text = vec![
        Line::from(vec![
            Span::styled("Height: ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                format!("#{}", block.height),
                Style::default()
                    .fg(Theme::primary())
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Block Hash:  ", Style::default().fg(Theme::text_dim())),
            Span::raw(&block.hash),
        ]),
        Line::from(vec![
            Span::styled("Merkle Root: ", Style::default().fg(Theme::text_dim())),
            Span::raw(&block.merkle_root),
        ]),
        Line::from(vec![
            Span::styled("Proposer: ", Style::default().fg(Theme::text_dim())),
            Span::raw(&block.proposer),
        ]),
        Line::from(vec![
            Span::styled("Transactions: ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                block.tx_count.to_string(),
                Style::default().fg(Theme::success()),
            ),
        ]),
        Line::from(vec![
            Span::styled("Timestamp: ", Style::default().fg(Theme::text_dim())),
            Span::raw(format_timestamp(&block.timestamp)),
        ]),
    ];

    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" BLOCK PREVIEW ")
            .border_style(Style::default().fg(Theme::primary())),
    );

    f.render_widget(paragraph, area);
}

fn draw_block_detail(f: &mut Frame, app: &App, area: Rect) {
    let block = match app.selected_block() {
        Some(b) => b,
        None => return,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(0)])
        .split(area);

    // Block header info
    let header_text = vec![
        Line::from(vec![
            Span::styled("Block #", Style::default().fg(Theme::text_dim())),
            Span::styled(
                block.height.to_string(),
                Style::default()
                    .fg(Theme::primary())
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Block Hash:  ", Style::default().fg(Theme::text_dim())),
            Span::raw(&block.hash),
        ]),
        Line::from(vec![
            Span::styled("Merkle Root: ", Style::default().fg(Theme::text_dim())),
            Span::raw(&block.merkle_root),
        ]),
        Line::from(vec![
            Span::styled("Proposer:    ", Style::default().fg(Theme::text_dim())),
            Span::styled(&block.proposer, Style::default().fg(Theme::accent())),
        ]),
        Line::from(vec![
            Span::styled("Timestamp:   ", Style::default().fg(Theme::text_dim())),
            Span::raw(format_timestamp(&block.timestamp)),
        ]),
        Line::from(vec![
            Span::styled("Transactions:", Style::default().fg(Theme::text_dim())),
            Span::styled(
                block.tx_count.to_string(),
                Style::default().fg(Theme::success()),
            ),
        ]),
    ];

    let header = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" BLOCK HEADER ")
            .border_style(Style::default().fg(Theme::primary())),
    );
    f.render_widget(header, chunks[0]);

    // Transactions table
    let rows: Vec<Row> = block
        .transactions
        .iter()
        .map(|tx| {
            Row::new(vec![
                Cell::from(tx.tx_type.clone()).style(Style::default().fg(Theme::primary())),
                Cell::from(tx.package_name.clone().unwrap_or_default()),
                Cell::from(tx.package_version.clone().unwrap_or_default()),
                Cell::from(tx.publisher.clone().unwrap_or_default()),
                Cell::from(tx.status.clone()).style(Style::default().fg(Theme::success())),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Percentage(30),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec!["Type", "Package", "Version", "Publisher", "Status"]).style(
            Style::default()
                .fg(Theme::text_dim())
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" TRANSACTIONS ({}) ", block.tx_count))
            .border_style(Style::default().fg(Theme::secondary())),
    );

    f.render_widget(table, chunks[1]);
}

// ============================================================================
// VIEW: VALIDATORS
// ============================================================================

fn draw_validators(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    // Validators table
    let query = app.search_query.to_ascii_lowercase();
    let rows: Vec<Row> = app
        .validators
        .iter()
        .enumerate()
        .filter(|(_i, v)| {
            if query.is_empty() {
                return true;
            }
            v.id.to_ascii_lowercase().contains(&query)
                || v.alias.to_ascii_lowercase().contains(&query)
                || v.status.to_ascii_lowercase().contains(&query)
        })
        .map(|(i, v)| {
            let status_color = match v.status.as_str() {
                "online" | "self" => Theme::success(),
                "pending" => Theme::warning(),
                _ => Theme::error(),
            };

            let style = if i == app.selected_validator {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };

            let rep_bar = render_reputation_bar(v.reputation);

            Row::new(vec![
                Cell::from(if v.alias.is_empty() {
                    v.id.clone()
                } else {
                    format!("{} ({})", v.id, v.alias)
                }),
                Cell::from(format!("{} CREG", format_number(v.stake))),
                Cell::from(rep_bar),
                Cell::from(v.status.clone()).style(Style::default().fg(status_color)),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(25),
            Constraint::Percentage(30),
            Constraint::Percentage(15),
        ],
    )
    .header(
        Row::new(vec!["Validator", "Stake", "Reputation", "Status"]).style(
            Style::default()
                .fg(Theme::text_dim())
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" VALIDATORS (j/k to navigate, Enter for details) ")
            .border_style(Style::default().fg(Theme::accent())),
    );

    f.render_widget(table, chunks[0]);

    // Validator stats sidebar
    draw_validator_stats(f, app, chunks[1]);
}

fn draw_validators_preview(f: &mut Frame, app: &App, area: Rect) {
    let rows: Vec<Row> = app
        .validators
        .iter()
        .take(10)
        .map(|v| {
            let status_color = match v.status.as_str() {
                "online" | "self" => Theme::success(),
                _ => Theme::text_dim(),
            };

            Row::new(vec![
                Cell::from(v.id.chars().take(20).collect::<String>()),
                Cell::from(v.stake.to_string()),
                Cell::from(format!("{}", v.reputation)).style(Style::default().fg(status_color)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(60),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
    .header(
        Row::new(vec!["Validator", "Stake(k)", "Rep"])
            .style(Style::default().fg(Theme::text_dim())),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" TOP VALIDATORS ")
            .border_style(Style::default().fg(Theme::accent())),
    );

    f.render_widget(table, area);
}

fn draw_validator_stats(f: &mut Frame, app: &App, area: Rect) {
    let active_count = app.validators.iter().filter(|v| v.is_active).count();
    let avg_reputation = if !app.validators.is_empty() {
        app.validators
            .iter()
            .map(|v| v.reputation as u64)
            .sum::<u64>()
            / app.validators.len() as u64
    } else {
        0
    };

    let text = vec![
        Line::from(vec![
            Span::styled("Total: ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                app.validators.len().to_string(),
                Style::default().fg(Theme::text()),
            ),
        ]),
        Line::from(vec![
            Span::styled("Active: ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                active_count.to_string(),
                Style::default().fg(Theme::success()),
            ),
        ]),
        Line::from(vec![
            Span::styled("Avg Rep: ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                format!("{}/100", avg_reputation),
                Style::default().fg(Theme::warning()),
            ),
        ]),
    ];

    let stats = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" STATS ")
            .border_style(Style::default().fg(Theme::accent())),
    );

    f.render_widget(stats, area);
}

fn draw_validator_detail(f: &mut Frame, app: &App, area: Rect) {
    let validator = match app.selected_validator() {
        Some(v) => v,
        None => return,
    };

    let status_color = match validator.status.as_str() {
        "online" | "self" => Theme::success(),
        "pending" => Theme::warning(),
        _ => Theme::error(),
    };

    let text = vec![
        Line::from(vec![Span::styled(
            "VALIDATOR DETAILS\n",
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("ID:          ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                &validator.id,
                Style::default()
                    .fg(Theme::text())
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Alias:       ", Style::default().fg(Theme::text_dim())),
            Span::raw(&validator.alias),
        ]),
        Line::from(vec![
            Span::styled("Public Key:  ", Style::default().fg(Theme::text_dim())),
            Span::raw(format!(
                "{}..",
                &validator.pub_key[..validator.pub_key.len().min(40)]
            )),
        ]),
        Line::from(vec![
            Span::styled("Stake:       ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                format!("{} CREG", format_number(validator.stake)),
                Style::default().fg(Theme::success()),
            ),
        ]),
        Line::from(vec![
            Span::styled("Reputation:  ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                format!("{}/100", validator.reputation),
                Style::default().fg(Theme::warning()),
            ),
        ]),
        Line::from(vec![
            Span::styled("Status:      ", Style::default().fg(Theme::text_dim())),
            Span::styled(&validator.status, Style::default().fg(status_color)),
        ]),
        Line::from(vec![
            Span::styled("Active:      ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                if validator.is_active { "Yes" } else { "No" },
                Style::default().fg(if validator.is_active {
                    Theme::success()
                } else {
                    Theme::error()
                }),
            ),
        ]),
    ];

    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" VALIDATOR ")
            .border_style(Style::default().fg(Theme::accent())),
    );

    f.render_widget(paragraph, area);
}

fn render_reputation_bar(reputation: u8) -> String {
    let filled = (reputation / 10) as usize;
    let empty = 10 - filled;
    let bar = "█".repeat(filled) + &"░".repeat(empty);
    format!("{} {}%", bar, reputation)
}

// ============================================================================
// VIEW: PACKAGES
// ============================================================================

fn draw_packages(f: &mut Frame, app: &App, area: Rect) {
    if app.packages.is_empty() {
        let text = Paragraph::new(
            "No packages found. Packages will appear here when published to the registry.",
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" PACKAGES ({} on-chain) ", app.stats.package_count))
                .border_style(Style::default().fg(Theme::primary())),
        )
        .style(Style::default().fg(Theme::text_dim()));
        f.render_widget(text, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let query = app.search_query.to_ascii_lowercase();
    let matching = app.matching_packages();
    let items: Vec<ListItem> = matching
        .iter()
        .map(|&i| {
            let pkg = &app.packages[i];
            let icon = match pkg.status.as_str() {
                "verified" => "✓",
                "pending" => "⏳",
                "rejected" => "✗",
                _ => "?",
            };
            let content = format!(
                "{} {:<30} {:<12} {}",
                icon, pkg.name, pkg.version, pkg.status
            );
            let style = if i == app.selected_package {
                Style::default()
                    .fg(Theme::highlight())
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::REVERSED)
            } else {
                let color = match pkg.status.as_str() {
                    "verified" => Theme::success(),
                    "pending" => Theme::warning(),
                    "rejected" => Theme::error(),
                    _ => Theme::text(),
                };
                Style::default().fg(color)
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let pkg_title = if query.is_empty() {
        format!(
            " PACKAGES ({}) — j/k to navigate, Enter for details ",
            app.packages.len()
        )
    } else {
        format!(
            " PACKAGES (filtered: {}/{}) ",
            items.len(),
            app.packages.len()
        )
    };
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(pkg_title)
            .border_style(Style::default().fg(Theme::primary())),
    );
    f.render_widget(list, chunks[0]);

    // Package detail preview
    match app.selected_package() {
        Some(pkg) => {
            let text = vec![
                Line::from(vec![
                    Span::styled("Name:       ", Style::default().fg(Theme::text_dim())),
                    Span::styled(
                        &pkg.name,
                        Style::default()
                            .fg(Theme::primary())
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Version:    ", Style::default().fg(Theme::text_dim())),
                    Span::raw(&pkg.version),
                ]),
                Line::from(vec![
                    Span::styled("Ecosystem:  ", Style::default().fg(Theme::text_dim())),
                    Span::raw(&pkg.ecosystem),
                ]),
                Line::from(vec![
                    Span::styled("Status:     ", Style::default().fg(Theme::text_dim())),
                    Span::styled(
                        &pkg.status,
                        Style::default().fg(match pkg.status.as_str() {
                            "verified" => Theme::success(),
                            "pending" => Theme::warning(),
                            _ => Theme::error(),
                        }),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Publisher:  ", Style::default().fg(Theme::text_dim())),
                    Span::raw(if pkg.publisher.len() > 16 {
                        format!("{}...", &pkg.publisher[..16])
                    } else {
                        pkg.publisher.clone()
                    }),
                ]),
                Line::from(vec![
                    Span::styled("Hash:       ", Style::default().fg(Theme::text_dim())),
                    Span::raw(if pkg.content_hash.len() > 20 {
                        format!("{}...", &pkg.content_hash[..20])
                    } else {
                        pkg.content_hash.clone()
                    }),
                ]),
            ];
            let detail = Paragraph::new(text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" PACKAGE PREVIEW ")
                    .border_style(Style::default().fg(Theme::primary())),
            );
            f.render_widget(detail, chunks[1]);
        }
        None => {
            let empty = Paragraph::new("Select a package to see details")
                .style(Style::default().fg(Theme::text_dim()))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" PACKAGE PREVIEW ")
                        .border_style(Style::default().fg(Theme::primary())),
                );
            f.render_widget(empty, chunks[1]);
        }
    }
}

fn draw_package_detail(f: &mut Frame, app: &App, area: Rect) {
    let pkg = match app.selected_package() {
        Some(p) => p,
        None => return,
    };

    let text = vec![
        Line::from(vec![Span::styled(
            "PACKAGE DETAILS\n",
            Style::default()
                .fg(Theme::primary())
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Name:         ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                &pkg.name,
                Style::default()
                    .fg(Theme::text())
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Version:      ", Style::default().fg(Theme::text_dim())),
            Span::raw(&pkg.version),
        ]),
        Line::from(vec![
            Span::styled("Ecosystem:    ", Style::default().fg(Theme::text_dim())),
            Span::raw(&pkg.ecosystem),
        ]),
        Line::from(vec![
            Span::styled("Status:       ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                &pkg.status,
                Style::default().fg(match pkg.status.as_str() {
                    "verified" => Theme::success(),
                    "pending" => Theme::warning(),
                    _ => Theme::error(),
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("Publisher:    ", Style::default().fg(Theme::text_dim())),
            Span::raw(&pkg.publisher),
        ]),
        Line::from(vec![
            Span::styled("Content Hash: ", Style::default().fg(Theme::text_dim())),
            Span::raw(&pkg.content_hash),
        ]),
        Line::from(vec![
            Span::styled("Verified At:  ", Style::default().fg(Theme::text_dim())),
            Span::raw(pkg.verified_at.as_deref().unwrap_or("Not yet")),
        ]),
    ];

    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" PACKAGE ")
            .border_style(Style::default().fg(Theme::primary())),
    );
    f.render_widget(paragraph, area);
}

// ============================================================================
// VIEW: NETWORK
// ============================================================================

fn draw_network(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(13), Constraint::Min(0)])
        .split(area);

    // Network stats
    let stats_text = vec![
        Line::from(vec![
            Span::styled("Connected Peers: ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                app.stats.peer_count.to_string(),
                Style::default().fg(Theme::success()),
            ),
        ]),
        Line::from(vec![
            Span::styled("Bridge Status:   ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                &app.stats.bridge_status,
                Style::default().fg(Theme::primary()),
            ),
        ]),
        Line::from(vec![
            Span::styled("L1 Block:        ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                format!("#{}", app.stats.l1_block),
                Style::default().fg(Theme::warning()),
            ),
        ]),
        Line::from(vec![
            Span::styled("Finalized Height: ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                format!(
                    "#{} (lag {})",
                    app.stats.finalized_height.map(|h| h.to_string()).unwrap_or_else(|| "unknown".into()), app.stats.finalization_lag.map(|h| h.to_string()).unwrap_or_else(|| "unknown".into())
                ),
                Style::default().fg(if app.stats.finalization_lag.is_some_and(|lag| lag <= 2) {
                    Theme::success()
                } else {
                    Theme::warning()
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("Validator Set:    ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                format!(
                    "{} / {}",
                    app.stats.validator_set_source, app.stats.validator_set_sync_state
                ),
                Style::default().fg(Theme::primary()),
            ),
        ]),
    ];

    let stats = Paragraph::new(stats_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" NETWORK STATUS ")
            .border_style(Style::default().fg(Theme::secondary())),
    );
    f.render_widget(stats, chunks[0]);

    // Peer list
    let peers: Vec<ListItem> = app
        .peer_ids
        .iter()
        .map(|p| ListItem::new(format!("● {}", p)).style(Style::default().fg(Theme::success())))
        .collect();

    let peer_list = List::new(peers).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" CONNECTED PEERS ")
            .border_style(Style::default().fg(Theme::secondary())),
    );

    f.render_widget(peer_list, chunks[1]);
}

// ============================================================================
// VIEW: MEMPOOL
// ============================================================================

fn draw_mempool(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // Header with count
    let header_text = if operator_api_key().is_none() {
        " Pending details require operator credentials; count is available in public chain stats".to_string()
    } else { format!(" {} pending transactions waiting for validator consensus", app.mempool.len()) };
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Theme::text()))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" MEMPOOL ")
                .border_style(Style::default().fg(Theme::warning())),
        );
    f.render_widget(header, chunks[0]);

    if app.mempool.is_empty() {
        let empty = Paragraph::new("  No pending transactions — mempool is empty")
            .style(Style::default().fg(Theme::text_dim()))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Theme::border())),
            );
        f.render_widget(empty, chunks[1]);
        return;
    }

    // Table of mempool entries
    let header_cells = ["#", "Canonical ID", "Type", "Age"].iter().map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Theme::primary())
                .add_modifier(Modifier::BOLD),
        )
    });
    let table_header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = app
        .mempool
        .iter()
        .enumerate()
        .map(|(i, tx)| {
            let age_secs = tx.timestamp.elapsed().as_secs();
            let age_str = if age_secs < 60 {
                format!("{}s", age_secs)
            } else if age_secs < 3600 {
                format!("{}m {}s", age_secs / 60, age_secs % 60)
            } else {
                format!("{}h {}m", age_secs / 3600, (age_secs % 3600) / 60)
            };

            let style = if i % 2 == 0 {
                Style::default().fg(Theme::text())
            } else {
                Style::default().fg(Theme::text_dim())
            };

            Row::new(vec![
                Cell::from(format!("{}", i + 1)),
                Cell::from(tx.id.clone()),
                Cell::from(tx.tx_type.clone()),
                Cell::from(age_str),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Min(30),
            Constraint::Length(16),
            Constraint::Length(10),
        ],
    )
    .header(table_header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Pending Transactions ")
            .border_style(Style::default().fg(Theme::border())),
    );

    f.render_widget(table, chunks[1]);
}

// ============================================================================
// VIEW: EVENTS
// ============================================================================

fn draw_events(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .events
        .iter()
        .enumerate()
        .map(|(i, (time, event_type, msg))| {
            let elapsed = time.elapsed().as_secs();
            let time_str = if elapsed < 60 {
                format!("{}s ago", elapsed)
            } else if elapsed < 3600 {
                format!("{}m ago", elapsed / 60)
            } else {
                format!("{}h ago", elapsed / 3600)
            };

            let color = match event_type.as_str() {
                "Block" => Theme::success(),
                "Package" => Theme::primary(),
                "Validator" => Theme::accent(),
                "Slash" => Theme::error(),
                _ => Theme::text_dim(),
            };

            let style = if i == app.selected_event {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(color)
            };

            let content = format!("[{:>8}] {:<12} {}", time_str, event_type, msg);
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" EVENT LOG ({}) ", app.events.len()))
            .border_style(Style::default().fg(Theme::success())),
    );

    f.render_widget(list, area);
}

// ============================================================================
// VIEW: OPERATOR
// ============================================================================

fn draw_operator(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(6), // Identity block
            Constraint::Length(8),
            Constraint::Min(6),
        ])
        .split(area);

    let active_validators = app
        .validators
        .iter()
        .filter(|v| v.is_active || common::validator_status_is_active(&v.status))
        .count();

    let summary = Paragraph::new(vec![
        Line::from(format!("Mode: validator console")),
        Line::from(format!(
            "Validators online: {}/{}",
            active_validators,
            app.validators.len()
        )),
        Line::from(format!("Connected peers: {}", app.peer_ids.len())),
        Line::from(format!("Bridge status: {}", app.stats.bridge_status)),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Operator Summary "),
    )
    .style(Style::default().fg(Theme::text()))
    .wrap(Wrap { trim: true });
    f.render_widget(summary, chunks[0]);

    // Identity block
    let mut identity_lines = Vec::new();
    if let Some(id) = &app.runtime_identity {
        identity_lines.push(Line::from(vec![
            Span::styled("Node ID:   ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                &id.node_id,
                Style::default()
                    .fg(Theme::primary())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        identity_lines.push(Line::from(vec![
            Span::styled("Ed25519:   ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                id.validator_pubkey.as_deref().unwrap_or("not configured"),
                Style::default().fg(Theme::accent()),
            ),
        ]));
        identity_lines.push(Line::from(vec![
            Span::styled("Action:    ", Style::default().fg(Theme::text_dim())),
            Span::raw("Copy these values to the Web Explorer to register your identity."),
        ]));
    } else {
        identity_lines.push(Line::from(Span::styled(
            "Fetching local node identity...",
            Style::default().fg(Theme::text_dim()),
        )));
    }

    let identity = Paragraph::new(identity_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Detected Validator Identity ")
                .border_style(Style::default().fg(Theme::primary())),
        )
        .style(Style::default().fg(Theme::text()));
    f.render_widget(identity, chunks[1]);

    let commands = Paragraph::new(vec![
        Line::from("Browser explorer:  http://localhost:3007"),
        Line::from("Node health:        creg testnet status --node-url http://localhost:8080"),
        Line::from(
            "Register ID:        creg testnet register-validator --node-id <id> --pubkey <key>",
        ),
        Line::from("Validator actions:  use an external wallet/CLI; this console stays read-only"),
        Line::from("Diagnostics:        press p for preflight, x to export a redacted report"),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Operator Commands "),
    )
    .style(Style::default().fg(Theme::text()))
    .wrap(Wrap { trim: true });
    f.render_widget(commands, chunks[2]);

    let workflow = Paragraph::new(vec![
        Line::from("This console is the operator surface for validator monitoring."),
        Line::from("Use it to inspect blocks, validator health, packages, peers, and live events."),
        Line::from("No private key is accepted, stored, or displayed by the console."),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Operator Workflow "),
    )
    .style(Style::default().fg(Theme::text()))
    .wrap(Wrap { trim: true });
    f.render_widget(workflow, chunks[3]);
}

// ============================================================================
// VIEW: HELP
// ============================================================================

fn draw_help(f: &mut Frame, _app: &App, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            "CREG console",
            Style::default()
                .fg(Theme::primary())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled("Move", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD))),
        Line::from("  j/k or arrows     list   ·   Tab / h/l   next/prev view   ·   Enter/d   detail"),
        Line::from("  Esc/b             back   ·   PgUp/PgDn   page   ·   g / G   top / bottom"),
        Line::from(""),
        Line::from(Span::styled("Actions", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD))),
        Line::from("  : or Ctrl+P/K     command palette     ·   /   filter lists     ·   y   copy selection"),
        Line::from("  r                 refresh now         ·   x   export diagnostics ·   t   theme"),
        Line::from("  q                 back, or quit after confirm"),
        Line::from(""),
        Line::from(Span::styled("Views", Style::default().fg(Theme::accent()).add_modifier(Modifier::BOLD))),
        Line::from("  1 Overview  2 Blocks  3 Validators  4 Packages  5 Network  6 Mempool  7 Events"),
        Line::from("  8 Operator  9 PBFT  0 Faucet  p Preflight  m Metrics   ·   click the tab bar"),
        Line::from(""),
        Line::from("Read-only: this console never collects private keys. Unsigned /head is not install-eligible."),
        Line::from("Press Esc or ? to close."),
    ];
    let help = Paragraph::new(lines)
        .block(rounded_block(" help ", true))
        .style(Style::default().fg(Theme::text()))
        .wrap(Wrap { trim: false });
    f.render_widget(help, area);
}

fn draw_search_popup(f: &mut Frame, app: &App) {
    let area = centered_rect(64, 18, f.size());
    let cursor = if app.tick_count % 12 < 6 { "▌" } else { " " };
    let text = vec![
        Line::from(Span::styled(
            "Filter lists (name, hash, id, status)",
            Style::default().fg(Theme::text_dim()),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}{}", app.search_query, cursor),
            Style::default()
                .fg(Theme::primary())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Enter apply  ·  Esc clear",
            Style::default().fg(Theme::text_dim()),
        )),
    ];
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(text).block(rounded_block(" search ", true)),
        area,
    );
}

fn draw_palette(f: &mut Frame, app: &App) {
    let area = centered_rect(70, 55, f.size());
    let filtered = filtered_palette_indices(&app.palette.query);
    let commands = palette_commands();
    let mut lines = vec![Line::from(Span::styled(
        format!("> {}_", app.palette.query),
        Style::default()
            .fg(Theme::primary())
            .add_modifier(Modifier::BOLD),
    ))];
    for (visible_idx, cmd_idx) in filtered.iter().enumerate().take(12) {
        let cmd = &commands[*cmd_idx];
        let active = visible_idx == app.palette.selected;
        let marker = if active { "▸" } else { " " };
        let style = if active {
            Style::default()
                .fg(Theme::tab_active_fg())
                .bg(Theme::primary())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Theme::text())
        };
        lines.push(Line::from(Span::styled(
            format!(" {marker} {:<28} {:>4}", cmd.label, cmd.hint),
            style,
        )));
    }
    if filtered.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no matching commands",
            Style::default().fg(Theme::text_dim()),
        )));
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(rounded_block(
            " command palette  ·  Enter run  ·  Esc close ",
            true,
        )),
        area,
    );
}

fn draw_toast(f: &mut Frame, toast: &Toast) {
    let area = f.size();
    let width = 56.min(area.width.saturating_sub(2)).max(20);
    let toast_area = Rect {
        x: area.x + area.width.saturating_sub(width + 1),
        y: area.y + HEADER_HEIGHT + TAB_HEIGHT,
        width,
        height: 3,
    };
    let color = match toast.kind {
        ToastKind::Info => Theme::secondary(),
        ToastKind::Success => Theme::success(),
        ToastKind::Warn => Theme::warning(),
        ToastKind::Error => Theme::error(),
    };
    f.render_widget(Clear, toast_area);
    f.render_widget(
        Paragraph::new(format!(" {} ", toast.message))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(color)),
            )
            .style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
        toast_area,
    );
}

// ============================================================================
// UTILITIES
// ============================================================================

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn format_number(n: u64) -> String {
    let num = n as f64;
    if n >= 1_000_000_000 {
        format!("{:.2}B", num / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.2}M", num / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", num / 1_000.0)
    } else {
        n.to_string()
    }
}

fn format_timestamp(ts: &str) -> String {
    // Simple formatting - in production would parse and format properly
    if ts.len() > 19 {
        ts[..19].to_string()
    } else {
        ts.to_string()
    }
}

// ============================================================================
// FAUCET PANE
// ============================================================================

/// Poll the faucet /health endpoint. Returns (healthy, token_reserve_str) or None.
async fn fetch_faucet_health(
    client: &reqwest::Client,
    base: &str,
) -> Option<(bool, Option<String>)> {
    let url = format!("{}/health", base.trim_end_matches('/'));
    let res = client
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return Some((false, None));
    }
    let json: Value = res.json().await.ok()?;
    let healthy =
        json["status"].as_str() == Some("healthy") && json["faucet"].as_str() == Some("online");
    let reserve = json["faucet_balance"].as_str().map(|s| s.to_string());
    Some((healthy, reserve))
}

/// Drive the full PoW-guarded drip flow: fetch challenge → solve PoW → POST drip.
/// Emits progress messages and a final `FaucetDripResult` over `tx`.
async fn run_faucet_drip(tx: mpsc::Sender<DataUpdate>, base: String, address: String) {
    let client = reqwest::Client::new();

    let _ = tx
        .send(DataUpdate::FaucetDripProgress(
            "Requesting PoW challenge…".into(),
        ))
        .await;

    let challenge = match crate::faucet_client::get_challenge(&client, &base).await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx
                .send(DataUpdate::FaucetDripResult(Err(format!(
                    "challenge failed: {}",
                    e
                ))))
                .await;
            return;
        }
    };

    let _ = tx
        .send(DataUpdate::FaucetDripProgress(format!(
            "Solving PoW (difficulty={})…",
            challenge.difficulty
        )))
        .await;

    // Hashing is CPU-bound; keep it off the async runtime thread.
    let challenge_for_task = challenge.challenge.clone();
    let difficulty = challenge.difficulty;
    let nonce = match tokio::task::spawn_blocking(move || {
        crate::faucet_client::solve_pow(&challenge_for_task, difficulty)
    })
    .await
    {
        Ok(n) => n,
        Err(e) => {
            let _ = tx
                .send(DataUpdate::FaucetDripResult(Err(format!(
                    "PoW solver crashed: {}",
                    e
                ))))
                .await;
            return;
        }
    };

    let _ = tx
        .send(DataUpdate::FaucetDripProgress(
            "Submitting drip request…".into(),
        ))
        .await;

    match crate::faucet_client::drip(&client, &base, &address, &challenge.challenge, &nonce).await {
        Ok(resp) if resp.success => {
            let tx_hash = resp.tx_hash.unwrap_or_else(|| "(none)".into());
            let amount = resp.amount.unwrap_or_else(|| "(unknown)".into());
            let _ = tx
                .send(DataUpdate::FaucetDripResult(Ok((tx_hash, amount))))
                .await;

            // Follow up with a balance refresh so the user sees the drip land.
            if let Ok(bal) = crate::faucet_client::get_balance(&client, &base, &address).await {
                let _ = tx
                    .send(DataUpdate::FaucetBalance {
                        token: bal.balance,
                        native: bal.native_balance,
                        token_fmt: bal.balance_formatted,
                        native_fmt: bal.native_balance_formatted,
                    })
                    .await;
            }
        }
        Ok(resp) => {
            let _ = tx
                .send(DataUpdate::FaucetDripResult(Err(resp
                    .error
                    .unwrap_or_else(|| "drip rejected".into()))))
                .await;
        }
        Err(e) => {
            let _ = tx
                .send(DataUpdate::FaucetDripResult(Err(format!(
                    "drip call failed: {}",
                    e
                ))))
                .await;
        }
    }
}

fn draw_faucet(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title
            Constraint::Length(5), // network / health
            Constraint::Length(3), // address input
            Constraint::Length(4), // balance
            Constraint::Min(5),    // status / help
        ])
        .split(area);

    // Title
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "⛲ Testnet Faucet",
            Style::default()
                .fg(Theme::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(
            format!("[{}]", app.faucet.health),
            Style::default().fg(if app.faucet.health == "online" {
                Theme::success()
            } else {
                Theme::warning()
            }),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Theme::border())),
    )
    .alignment(Alignment::Center);
    f.render_widget(title, chunks[0]);

    // Network
    let mut net_lines: Vec<Line> = Vec::new();
    net_lines.push(Line::from(vec![
        Span::styled("Endpoint:  ", Style::default().fg(Theme::text_dim())),
        Span::styled(
            app.faucet.base.clone(),
            Style::default().fg(Theme::highlight()),
        ),
    ]));
    if let Some(net) = &app.faucet.network {
        net_lines.push(Line::from(vec![
            Span::styled("Chain:     ", Style::default().fg(Theme::text_dim())),
            Span::styled(
                format!("{} (id {})", net.chain_name, net.chain_id),
                Style::default().fg(Theme::text()),
            ),
        ]));
        net_lines.push(Line::from(vec![
            Span::styled("RPC:       ", Style::default().fg(Theme::text_dim())),
            Span::styled(net.rpc_url.clone(), Style::default().fg(Theme::text())),
        ]));
    }
    if let Some(reserve) = &app.faucet.faucet_token_reserve {
        net_lines.push(Line::from(vec![
            Span::styled("Reserve:   ", Style::default().fg(Theme::text_dim())),
            Span::styled(format_wei(reserve), Style::default().fg(Theme::primary())),
            Span::styled(" tCREG", Style::default().fg(Theme::text_dim())),
        ]));
    }
    let net_widget = Paragraph::new(net_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Network ")
            .border_style(Style::default().fg(Theme::border())),
    );
    f.render_widget(net_widget, chunks[1]);

    // Address input
    let input_style = if app.faucet.editing {
        Style::default()
            .fg(Theme::highlight())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Theme::text())
    };
    let display = if app.faucet.address_input.is_empty() {
        if app.faucet.editing {
            "▊".to_string()
        } else {
            "(press 'e' to edit)".to_string()
        }
    } else if app.faucet.editing {
        format!("{}▊", app.faucet.address_input)
    } else {
        app.faucet.address_input.clone()
    };
    let input_title = if app.faucet.editing {
        " Recipient address (INSERT — Esc to stop) "
    } else {
        " Recipient address "
    };
    let input_border = if app.faucet.editing {
        Theme::highlight()
    } else {
        Theme::border()
    };
    let input_widget = Paragraph::new(Line::from(Span::styled(display, input_style))).block(
        Block::default()
            .borders(Borders::ALL)
            .title(input_title)
            .border_style(Style::default().fg(input_border)),
    );
    f.render_widget(input_widget, chunks[2]);

    // Balance. Prefer the faucet's pre-formatted decimal strings so the TUI
    // matches the web explorer (e.g. "1000.00" instead of "1000000000000000000000").
    // Fall back to format_wei() on the raw value only if the formatted field is
    // missing from the response.
    let token_display = app
        .faucet
        .last_balance_fmt
        .clone()
        .or_else(|| app.faucet.last_balance.as_deref().map(format_wei));
    let native_display = app
        .faucet
        .last_native_balance_fmt
        .clone()
        .or_else(|| app.faucet.last_native_balance.as_deref().map(format_wei));
    let mut bal_lines: Vec<Line> = Vec::new();
    match (&token_display, &native_display) {
        (Some(t), Some(n)) => {
            bal_lines.push(Line::from(vec![
                Span::styled("tCREG:  ", Style::default().fg(Theme::text_dim())),
                Span::styled(t.clone(), Style::default().fg(Theme::success())),
            ]));
            bal_lines.push(Line::from(vec![
                Span::styled("ETH:    ", Style::default().fg(Theme::text_dim())),
                Span::styled(n.clone(), Style::default().fg(Theme::secondary())),
            ]));
        }
        _ => {
            bal_lines.push(Line::from(Span::styled(
                "Press 'b' to fetch balance for the entered address.",
                Style::default().fg(Theme::text_dim()),
            )));
        }
    }
    let bal_widget = Paragraph::new(bal_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Balance ")
            .border_style(Style::default().fg(Theme::border())),
    );
    f.render_widget(bal_widget, chunks[3]);

    // Status
    let (status_text, status_color) = match &app.faucet.status {
        FaucetStatus::Idle => (
            "Idle. Enter a recipient address and press [d] to drip.".to_string(),
            Theme::text_dim(),
        ),
        FaucetStatus::Working {
            started_at,
            message,
        } => (
            format!(
                "⏳ {} ({}s elapsed)",
                message,
                started_at.elapsed().as_secs()
            ),
            Theme::warning(),
        ),
        FaucetStatus::Success {
            tx_hash,
            amount,
            at,
        } => (
            format!(
                "✓ Drip succeeded ({}s ago)\n  amount: {}\n  tx:     {}",
                at.elapsed().as_secs(),
                amount,
                tx_hash
            ),
            Theme::success(),
        ),
        FaucetStatus::Failed { error, at } => (
            format!("✗ Failed ({}s ago): {}", at.elapsed().as_secs(), error),
            Theme::error(),
        ),
    };
    let status_widget = Paragraph::new(status_text)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Status ")
                .border_style(Style::default().fg(status_color)),
        )
        .style(Style::default().fg(status_color));
    f.render_widget(status_widget, chunks[4]);
}

/// Convert a wei-denominated integer string to a human-friendly 18-decimal token
/// amount with 2 decimal places of precision. Non-integer input is returned verbatim.
fn format_wei(raw: &str) -> String {
    let trimmed = raw.trim();
    let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return trimmed.to_string();
    }
    if digits.len() <= 18 {
        let padded = format!("{:0>18}", digits);
        let whole = "0";
        let frac = &padded[padded.len().saturating_sub(18)..];
        let frac_short = &frac[..frac.len().min(2)];
        format!("{}.{}", whole, frac_short)
    } else {
        let split = digits.len() - 18;
        let whole = &digits[..split];
        let frac = &digits[split..];
        let frac_short = &frac[..frac.len().min(2)];
        format!("{}.{}", whole, frac_short)
    }
}

fn draw_bridge(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" L1 BRIDGE ANCHORS ")
        .border_style(Style::default().fg(Theme::primary()));
    let text = vec![
        Line::from(format!("Bridge Status: {}", app.stats.bridge_status)),
        Line::from(format!("Latest L1 Block: {}", app.stats.l1_block)),
        Line::from(format!(
            "Anchor count in memory: {}",
            app.bridge_anchors.len()
        )),
    ];
    let paragraph = Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(Theme::text()));
    f.render_widget(paragraph, area);
}

fn draw_metrics(f: &mut Frame, app: &App, area: Rect) {
    let heights: Vec<u64> = app
        .metrics_history
        .iter()
        .filter_map(|sample| sample["height"].as_u64())
        .collect();
    let height_max = heights.iter().max().copied().unwrap_or(1).max(1);
    let tx_data: Vec<u64> = app.tps_history.iter().copied().collect();
    let tx_max = tx_data.iter().max().copied().unwrap_or(1).max(1);
    let height_sparkline = Sparkline::default()
        .block(rounded_block(" retained chain height ", true))
        .data(&heights)
        .max(height_max)
        .style(Style::default().fg(Theme::success()));
    let tx_sparkline = Sparkline::default()
        .block(rounded_block(" tx in recently fetched blocks (live only) ", true))
        .data(&tx_data)
        .max(tx_max)
        .style(Style::default().fg(Theme::text_dim()));
    let summary = Paragraph::new(vec![
        Line::from(format!(
            "Tip {}  ·  packages {}  ·  validators {}  ·  retained samples {}",
            app.stats.tip_height,
            app.stats.package_count,
            app.stats.validator_count,
            app.metrics_history.len()
        )),
        Line::from(Span::styled(
            format!("Prometheus history: {}", app.metrics_history_status),
            Style::default().fg(Theme::text_dim()),
        )),
    ])
    .block(rounded_block(" chain metrics ", false));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5), Constraint::Length(4)])
        .split(area);
    f.render_widget(summary, chunks[0]);
    f.render_widget(height_sparkline, chunks[1]);
    f.render_widget(tx_sparkline, chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity() -> RuntimeIdentity {
        RuntimeIdentity {
            node_id: "validator-test-1".into(),
            validator_pubkey: Some("ab".repeat(32)),
            chain_id: "creg-testnet-1".into(),
            network: "testnet".into(),
            genesis_hash: Some("0x".to_string() + &"12".repeat(32)),
            spec_signature_verified: true,
            l1_rpc_configured: true,
            is_validator: true,
            validator_set_source: "chain-authoritative".into(),
            validator_set_sync_state: "ready".into(),
        }
    }

    #[test]
    fn preflight_reports_signed_validator_readiness() {
        let (sender, _receiver) = mpsc::channel(1);
        let mut app = App::new("http://127.0.0.1:8080".into(), sender);
        app.runtime_identity = Some(test_identity());
        app.last_api_success_at = Some(Instant::now());
        app.node_connected = true;
        app.peer_ids.push("peer-1".into());
        app.consensus_state = Some(ConsensusState {
            round: 4,
            phase: "COMMIT".into(),
            proposer: "validator-test-1".into(),
            prepare_votes: 2,
            commit_votes: 2,
            quorum: 2,
            total_validators: 2,
            fetched_at: Instant::now(),
        });
        app.validators.push(ValidatorInfo {
            id: "validator-test-1".into(),
            alias: "test".into(),
            stake: 100,
            reputation: 100,
            status: "online".into(),
            is_active: true,
            pub_key: "ab".repeat(32),
        });
        refresh_preflight(&mut app);

        assert!(app
            .preflight
            .checks
            .iter()
            .all(|check| check.status == PreflightStatus::Pass));
    }

    #[test]
    fn diagnostics_export_contains_no_private_key_material() {
        let (sender, _receiver) = mpsc::channel(1);
        let mut app = App::new("http://127.0.0.1:8080".into(), sender);
        app.runtime_identity = Some(test_identity());
        refresh_preflight(&mut app);
        let directory = tempfile::tempdir().expect("temporary diagnostics directory");
        std::env::set_var("CREG_DIAGNOSTICS_DIR", directory.path());
        let path = export_diagnostics(&app).expect("diagnostics should export");
        let report = fs::read_to_string(path).expect("diagnostics should be readable");
        std::env::remove_var("CREG_DIAGNOSTICS_DIR");

        assert!(report.contains("creg-console-diagnostics-v1"));
        assert!(report.contains("validator_pubkey"));
        assert!(!report.contains("validator_privkey"));
        assert!(!report.contains("deadbeef"));
    }

    #[test]
    fn cycle_tab_wraps_around_primary_views() {
        assert_eq!(cycle_tab(View::Overview, -1), View::Metrics);
        assert_eq!(cycle_tab(View::Metrics, 1), View::Overview);
        assert_eq!(cycle_tab(View::Blocks, 1), View::Validators);
    }

    #[test]
    fn tab_hit_testing_uses_equal_columns() {
        let width = console_tabs().len() as u16 * 10;
        assert_eq!(tab_at_column(0, width), Some(View::Overview));
        assert_eq!(tab_at_column(width - 1, width), Some(View::Metrics));
        assert_eq!(tab_at_column(0, 0), None);
    }

    #[test]
    fn palette_filter_matches_label_and_hint() {
        let yank = filtered_palette_indices("copy");
        assert!(yank.iter().any(|&idx| palette_commands()[idx].id == "yank"));
        let profile = filtered_palette_indices("profile");
        assert!(profile.iter().any(|&idx| palette_commands()[idx].id == "profile"));
        let views = filtered_palette_indices("pbft");
        assert!(views
            .iter()
            .any(|&idx| palette_commands()[idx].id == "consensus"));
        assert!(filtered_palette_indices("no-such-command").is_empty());
    }

    #[test]
    fn filtered_package_navigation_skips_hidden_rows() {
        let (sender, _receiver) = mpsc::channel(1);
        let mut app = App::new("http://127.0.0.1:8080".into(), sender);
        app.packages = vec![
            PackageInfo {
                name: "alpha".into(),
                ecosystem: "npm".into(),
                version: "1.0.0".into(),
                status: "verified".into(),
                publisher: "a".into(),
                verified_at: None,
                content_hash: "aa".into(),
            },
            PackageInfo {
                name: "beta".into(),
                ecosystem: "npm".into(),
                version: "1.0.0".into(),
                status: "pending".into(),
                publisher: "b".into(),
                verified_at: None,
                content_hash: "bb".into(),
            },
            PackageInfo {
                name: "alpha-tools".into(),
                ecosystem: "npm".into(),
                version: "2.0.0".into(),
                status: "verified".into(),
                publisher: "c".into(),
                verified_at: None,
                content_hash: "cc".into(),
            },
        ];
        app.search_query = "alpha".into();
        app.current_view = View::Packages;
        let filtered = app.matching_packages();
        assert_eq!(filtered, vec![0, 2]);
        app.selected_package = 0;
        move_filtered(&mut app.selected_package, &filtered, true);
        assert_eq!(app.selected_package, 2);
        let (label, text) = selection_yank_text(&app).expect("package yank");
        assert_eq!(label, "alpha-tools");
        assert_eq!(text, "cc");
    }

    #[test]
    fn base64_encoder_round_trip_padding() {
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
    }

    #[test]
    fn quit_from_detail_goes_back_instead_of_exiting() {
        let (sender, _receiver) = mpsc::channel(1);
        let mut app = App::new("http://127.0.0.1:8080".into(), sender);
        app.current_view = View::PackageDetail;
        app.previous_view = Some(View::Packages);
        assert!(!request_quit(&mut app));
        assert_eq!(app.current_view, View::Packages);
        assert!(!request_quit(&mut app));
        assert!(app.quit_armed_at.is_some());
        assert!(request_quit(&mut app));
    }

    fn demo_app() -> App {
        let (sender, _receiver) = mpsc::channel(1);
        let mut app = App::new("http://127.0.0.1:8080".into(), sender);
        app.tick_count = 24;
        app.node_connected = true;
        app.last_api_success_at = Some(Instant::now());
        app.last_api_error = None;
        app.read_only = true;
        app.runtime_identity = Some(test_identity());
        app.stats = NetworkStats {
            chain_id: "creg-testnet-1".into(),
            genesis_hash: Some("0x".to_string() + &"12".repeat(32)),
            spec_signature_verified: true,
            l1_rpc_configured: true,
            tip_height: 1842,
            package_count: 128,
            _block_count: 1842,
            validator_count: 4,
            total_stake: 400_000,
            peer_count: 3,
            bridge_status: "paused".into(),
            l1_block: 9_102_441,
            finalized_height: Some(1840),
            finalization_lag: Some(2),
            validator_set_source: "chain-authoritative".into(),
            validator_set_sync_state: "ready".into(),
        };
        app.validators = vec![
            ValidatorInfo {
                id: "validator-alpha".into(),
                alias: "alpha".into(),
                stake: 100_000_000_000,
                reputation: 99,
                status: "online".into(),
                is_active: true,
                pub_key: "aa".repeat(32),
            },
            ValidatorInfo {
                id: "validator-bravo".into(),
                alias: "bravo".into(),
                stake: 100_000_000_000,
                reputation: 97,
                status: "online".into(),
                is_active: true,
                pub_key: "bb".repeat(32),
            },
            ValidatorInfo {
                id: "validator-charlie".into(),
                alias: "charlie".into(),
                stake: 100_000_000_000,
                reputation: 95,
                status: "online".into(),
                is_active: true,
                pub_key: "cc".repeat(32),
            },
            ValidatorInfo {
                id: "validator-delta".into(),
                alias: "delta".into(),
                stake: 100_000_000_000,
                reputation: 94,
                status: "online".into(),
                is_active: true,
                pub_key: "dd".repeat(32),
            },
        ];
        for height in (1836..=1842).rev() {
            app.blocks.push_back(BlockInfo {
                height,
                hash: format!("0x{:064x}", height * 17),
                timestamp: "12s ago".into(),
                proposer: "validator-alpha".into(),
                tx_count: 3,
                transactions: vec![TransactionInfo {
                    _id: format!("tx-{height}"),
                    tx_type: "PublishPackage".into(),
                    package_name: Some("creg-sdk".into()),
                    package_version: Some("0.4.3".into()),
                    publisher: Some("alpha".into()),
                    status: "finalized".into(),
                }],
                merkle_root: format!("0x{:064x}", height * 31),
            });
        }
        app.packages = vec![
            PackageInfo {
                name: "creg-sdk".into(),
                ecosystem: "npm".into(),
                version: "0.4.3".into(),
                status: "verified".into(),
                publisher: "alpha".into(),
                verified_at: Some("1h ago".into()),
                content_hash: "e3b0c44298fc1c149afbf4c8996fb924".into(),
            },
            PackageInfo {
                name: "creg-cli".into(),
                ecosystem: "cargo".into(),
                version: "0.1.0".into(),
                status: "verified".into(),
                publisher: "bravo".into(),
                verified_at: Some("3h ago".into()),
                content_hash: "2c26b46b68ffc68ff99b453c1d304134".into(),
            },
            PackageInfo {
                name: "witness-kit".into(),
                ecosystem: "npm".into(),
                version: "0.2.1".into(),
                status: "pending".into(),
                publisher: "charlie".into(),
                verified_at: None,
                content_hash: "fcde2b2edba56bf408601fb721fe9b5c".into(),
            },
            PackageInfo {
                name: "alpha-tools".into(),
                ecosystem: "npm".into(),
                version: "2.0.0".into(),
                status: "verified".into(),
                publisher: "delta".into(),
                verified_at: Some("1d ago".into()),
                content_hash: "da39a3ee5e6b4b0d3255bfef95601890".into(),
            },
        ];
        app.peer_ids = vec![
            "12D3KooWAlphaPeer".into(),
            "12D3KooWBravoPeer".into(),
            "12D3KooWCharlieP".into(),
        ];
        app.consensus_state = Some(ConsensusState {
            round: 1843,
            phase: "COMMIT".into(),
            proposer: "validator-alpha".into(),
            prepare_votes: 3,
            commit_votes: 3,
            quorum: 3,
            total_validators: 4,
            fetched_at: Instant::now(),
        });
        app.push_event("finalized".into(), "block 1842 committed".into());
        app.push_event("package".into(), "creg-sdk@0.4.3 verified".into());
        app.push_event("peer".into(), "validator-delta heartbeat".into());
        for i in 0..48 {
            let sample = 4 + ((i * 3) % 9) as u64;
            app.tps_history.push_back(sample);
        }
        refresh_preflight(&mut app);
        app
    }

    fn css_color(color: Color, fallback: &str) -> String {
        match color {
            Color::Reset => fallback.to_string(),
            Color::Black => "#020617".into(),
            Color::Red => "#ef4444".into(),
            Color::Green => "#22c55e".into(),
            Color::Yellow => "#eab308".into(),
            Color::Blue => "#3b82f6".into(),
            Color::Magenta => "#d946ef".into(),
            Color::Cyan => "#22d3ee".into(),
            Color::Gray => "#94a3b8".into(),
            Color::DarkGray => "#475569".into(),
            Color::LightRed => "#f87171".into(),
            Color::LightGreen => "#4ade80".into(),
            Color::LightYellow => "#facc15".into(),
            Color::LightBlue => "#38bdf8".into(),
            Color::LightMagenta => "#e879f9".into(),
            Color::LightCyan => "#67e8f9".into(),
            Color::White => "#f8fafc".into(),
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            Color::Indexed(i) => format!("hsl({}, 12%, {}%)", u16::from(i), 20 + (i % 60)),
        }
    }

    fn html_escape(symbol: &str) -> String {
        if symbol.is_empty() || symbol == " " {
            return "&nbsp;".into();
        }
        symbol
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace(' ', "&nbsp;")
    }

    fn buffer_to_html(buffer: &ratatui::buffer::Buffer, title: &str, light: bool) -> String {
        let canvas = if light { "#e2e8f0" } else { "#020617" };
        let ink = if light { "#0f172a" } else { "#e2e8f0" };
        let area = buffer.area;
        let mut rows = String::new();
        for y in 0..area.height {
            rows.push_str("<div class=\"row\">");
            let mut skip = 0u16;
            for x in 0..area.width {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let cell = buffer.get(x, y);
                let mut fg = css_color(cell.fg, ink);
                let mut bg = css_color(cell.bg, canvas);
                if cell.modifier.contains(Modifier::REVERSED) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let mut style = format!("color:{fg};background:{bg}");
                if cell.modifier.contains(Modifier::BOLD) {
                    style.push_str(";font-weight:700");
                }
                if cell.modifier.contains(Modifier::UNDERLINED) {
                    style.push_str(";text-decoration:underline");
                }
                if cell.modifier.contains(Modifier::DIM) {
                    style.push_str(";opacity:0.7");
                }
                let symbol = cell.symbol();
                let wide = x + 1 < area.width
                    && !symbol.is_empty()
                    && buffer.get(x + 1, y).symbol().is_empty();
                if wide {
                    skip = 1;
                    style.push_str(";width:2ch");
                }
                rows.push_str(&format!(
                    "<span class=\"cell\" style=\"{style}\">{}</span>",
                    html_escape(symbol)
                ));
            }
            rows.push_str("</div>");
        }
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
<style>\
html,body{{margin:0;background:{canvas};color:{ink};width:max-content;font-family:ui-sans-serif,system-ui,sans-serif}}\
.wrap{{padding:20px 24px 28px}}\
.caption{{margin:0 0 12px;font-size:12px;letter-spacing:.04em;text-transform:uppercase;color:#64748b}}\
.term{{display:inline-block;background:{canvas};border-radius:10px;padding:8px 10px;box-shadow:0 24px 80px rgba(0,0,0,.45);\
font-family:\"Cascadia Code\",\"JetBrains Mono\",Consolas,ui-monospace,monospace;font-size:10px;line-height:12px}}\
.row{{display:flex;white-space:nowrap}}\
.cell{{display:inline-block;width:1ch;height:12px;overflow:hidden}}\
</style></head><body><div class=\"wrap\"><p class=\"caption\">{title}</p><div class=\"term\">{rows}</div></div></body></html>"
        )
    }

    fn render_scene(app: &App, width: u16, height: u16, title: &str, light: bool) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| draw_ui(frame, app))
            .expect("draw console");
        buffer_to_html(terminal.backend().buffer(), title, light)
    }

    fn write_preview(dir: &std::path::Path, name: &str, html: &str) {
        fs::create_dir_all(dir).expect("preview dir");
        fs::write(dir.join(name), html).expect("write preview html");
    }

    #[test]
    #[ignore]
    fn render_console_preview_html() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/console-preview");
        fs::create_dir_all(&dir).expect("preview dir");
        let dir = dir.canonicalize().unwrap_or(dir);
        IS_LIGHT_THEME.store(false, Ordering::Relaxed);

        let mut app = demo_app();
        write_preview(
            &dir,
            "01-overview.html",
            &render_scene(&app, 169, 36, "CREG console — Overview", false),
        );

        app.palette.open = true;
        app.palette.query = String::new();
        app.palette.selected = 0;
        write_preview(
            &dir,
            "02-palette.html",
            &render_scene(
                &app,
                169,
                36,
                "CREG console — command palette (: / Ctrl+P)",
                false,
            ),
        );
        app.palette.open = false;

        app.previous_view = Some(View::Overview);
        app.current_view = View::Help;
        write_preview(
            &dir,
            "03-help.html",
            &render_scene(&app, 169, 36, "CREG console — help (?)", false),
        );

        app.current_view = View::Packages;
        app.previous_view = None;
        app.search_query = "alpha".into();
        app.selected_package = 3;
        app.push_toast(ToastKind::Success, "copied alpha-tools digest");
        write_preview(
            &dir,
            "04-packages-yank.html",
            &render_scene(
                &app,
                169,
                36,
                "CREG console — Packages filter + yank toast",
                false,
            ),
        );
        app.toast = None;
        app.search_query.clear();

        app.current_view = View::Metrics;
        write_preview(
            &dir,
            "05-metrics.html",
            &render_scene(&app, 169, 36, "CREG console — Metrics", false),
        );

        IS_LIGHT_THEME.store(true, Ordering::Relaxed);
        app.current_view = View::Overview;
        write_preview(
            &dir,
            "06-overview-light.html",
            &render_scene(&app, 169, 36, "CREG console — Overview (light theme)", true),
        );
        IS_LIGHT_THEME.store(false, Ordering::Relaxed);

        eprintln!("console previews written to {}", dir.display());
    }
}
