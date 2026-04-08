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
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Clear, Gauge, LineGauge, List, ListItem, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Sparkline, Table, Tabs, Wrap,
    },
    Frame, Terminal,
};
use serde_json::Value;
use std::{
    collections::VecDeque,
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, RwLock};

// ============================================================================
// CONSTANTS & STYLING
// ============================================================================

const TICK_RATE_MS: u64 = 100;
const REFRESH_INTERVAL_SECS: u64 = 3;
const MAX_EVENTS: usize = 200;
const MAX_BLOCKS: usize = 100;

// Color palette for a cohesive, beautiful look
struct Theme;
impl Theme {
    const PRIMARY: Color = Color::Cyan;
    const SECONDARY: Color = Color::Blue;
    const SUCCESS: Color = Color::Green;
    const WARNING: Color = Color::Yellow;
    const ERROR: Color = Color::Red;
    const ACCENT: Color = Color::Magenta;
    const TEXT: Color = Color::White;
    const TEXT_DIM: Color = Color::Gray;
    const TEXT_DARK: Color = Color::DarkGray;
    const BG: Color = Color::Black;
    const BORDER: Color = Color::DarkGray;
    const HIGHLIGHT: Color = Color::LightCyan;
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
    id: String,
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
    tip_height: u64,
    package_count: u64,
    block_count: u64,
    validator_count: usize,
    total_stake: u64,
    peer_count: usize,
    bridge_status: String,
    l1_block: u64,
}

#[derive(Debug, Clone)]
struct MempoolTx {
    id: String,
    tx_type: String,
    size: usize,
    timestamp: Instant,
}

// ============================================================================
// APPLICATION STATE
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
enum View {
    Overview,
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
    Help,
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
    selected_tab: usize,

    // Data
    stats: NetworkStats,
    blocks: VecDeque<BlockInfo>,
    validators: Vec<ValidatorInfo>,
    packages: Vec<PackageInfo>,
    events: VecDeque<(Instant, String, String)>, // (timestamp, type, message)
    mempool: Vec<MempoolTx>,
    peer_ids: Vec<String>,

    // UI State
    show_help: bool,
    search_query: String,
    is_searching: bool,
    scroll_offset: usize,

    // Async
    api_base: String,
    data_tx: mpsc::Sender<DataUpdate>,
    last_refresh: Instant,
    tick_count: u64,

    // Sparkline data for TPS visualization
    tps_history: VecDeque<u64>,
}

#[derive(Debug)]
enum DataUpdate {
    Stats(NetworkStats),
    Block(BlockInfo),
    Validators(Vec<ValidatorInfo>),
    Packages(Vec<PackageInfo>),
    Event(String, String), // (type, message)
    MempoolTx(MempoolTx),
    Peers(Vec<String>),
    Error(String),
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
            selected_tab: 0,
            stats: NetworkStats {
                tip_height: 0,
                package_count: 0,
                block_count: 0,
                validator_count: 0,
                total_stake: 0,
                peer_count: 0,
                bridge_status: "Unknown".to_string(),
                l1_block: 0,
            },
            blocks: VecDeque::with_capacity(MAX_BLOCKS),
            validators: Vec::new(),
            packages: Vec::new(),
            events: VecDeque::with_capacity(MAX_EVENTS),
            mempool: Vec::new(),
            peer_ids: Vec::new(),
            show_help: false,
            search_query: String::new(),
            is_searching: false,
            scroll_offset: 0,
            api_base,
            data_tx,
            last_refresh: Instant::now(),
            tick_count: 0,
            tps_history: VecDeque::with_capacity(60),
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
            self.validators.iter().map(|validator| validator.stake).sum()
        }
    }

    fn push_event(&mut self, event_type: String, message: String) {
        self.events
            .push_front((Instant::now(), event_type, message));
        while self.events.len() > MAX_EVENTS {
            self.events.pop_back();
        }
    }
}

// ============================================================================
// MAIN ENTRY POINT
// ============================================================================

pub async fn run(node_url: Option<&str>) -> Result<()> {
    let api_base = node_url
        .map(String::from)
        .unwrap_or_else(|| {
            std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
        })
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
        drop(app_read);

        // Handle events
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let mut app_write = app.write().await;
                    if handle_key(&mut app_write, key.code).await {
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
    let client = reqwest::Client::new();
    let mut last_stats_refresh = Instant::now();
    let mut last_block_height: u64 = 0;

    loop {
        // Fetch stats periodically
        if last_stats_refresh.elapsed() >= Duration::from_secs(REFRESH_INTERVAL_SECS) {
            if let Ok(stats) = fetch_stats(&client, &api_base).await {
                let _ = tx.send(DataUpdate::Stats(stats.clone())).await;

                // Fetch new blocks if height changed
                if stats.tip_height > last_block_height {
                    for h in (last_block_height.saturating_add(1)..=stats.tip_height).rev() {
                        if let Ok(block) = fetch_block(&client, &api_base, h).await {
                            let _ = tx.send(DataUpdate::Block(block)).await;
                        }
                    }
                    last_block_height = stats.tip_height;
                }
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

            last_stats_refresh = Instant::now();
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn fetch_stats(client: &reqwest::Client, api_base: &str) -> Result<NetworkStats> {
    let res = client
        .get(format!("{}/v1/chain/stats", api_base))
        .send()
        .await?;
    let json: Value = res.json().await?;

    Ok(NetworkStats {
        tip_height: json["tip_height"].as_u64().unwrap_or(0),
        package_count: json["package_count"].as_u64().unwrap_or(0),
        block_count: json["block_count"].as_u64().unwrap_or(0),
        validator_count: json["validator_count"].as_u64().unwrap_or(0) as usize,
        total_stake: json["total_stake"].as_u64().unwrap_or(0),
        peer_count: json["peer_count"].as_u64().unwrap_or(0) as usize,
        bridge_status: json["bridge_status"]
            .as_str()
            .unwrap_or("Unknown")
            .to_string(),
        l1_block: json["l1_block"].as_u64().unwrap_or(0),
    })
}

async fn fetch_block(client: &reqwest::Client, api_base: &str, height: u64) -> Result<BlockInfo> {
    let res = client
        .get(format!("{}/v1/blocks/{}", api_base, height))
        .send()
        .await?;
    let json: Value = res.json().await?;

    let header = &json["header"];
    let txs: Vec<TransactionInfo> = json["transactions"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|t| TransactionInfo {
                    id: t["id"]["canonical"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string(),
                    tx_type: t["type"].as_str().unwrap_or("unknown").to_string(),
                    package_name: t["id"]["name"].as_str().map(|s| s.to_string()),
                    package_version: t["id"]["version"].as_str().map(|s| s.to_string()),
                    publisher: t["publisher_pubkey"]
                        .as_str()
                        .map(|s| s[..8.min(s.len())].to_string()),
                    status: t["status"].as_str().unwrap_or("pending").to_string(),
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
    let res = client.get(format!("{}/v1/nodes", api_base)).send().await?;
    let json: Vec<Value> = res.json().await?;

    Ok(json
        .iter()
        .map(|v| ValidatorInfo {
            id: v["id"].as_str().unwrap_or("unknown").to_string(),
            alias: v["alias"].as_str().unwrap_or("").to_string(),
            stake: v["stake"].as_u64().unwrap_or(0),
            reputation: v["reputation"].as_u64().unwrap_or(50) as u8,
            status: v["status"].as_str().unwrap_or("unknown").to_string(),
            is_active: v["is_active"].as_bool().unwrap_or(false),
            pub_key: v["pubkey"].as_str().unwrap_or("").to_string(),
        })
        .collect())
}

async fn fetch_peers(client: &reqwest::Client, api_base: &str) -> Result<Vec<String>> {
    let res = client
        .get(format!("{}/v1/p2p/status", api_base))
        .send()
        .await?;
    let json: Value = res.json().await?;

    Ok(json["peers"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default())
}

async fn fetch_pending_packages(
    client: &reqwest::Client,
    api_base: &str,
) -> Result<Vec<PackageInfo>> {
    let res = client
        .get(format!("{}/v1/pending", api_base))
        .send()
        .await?;
    let json: Value = res.json().await?;

    Ok(json["packages"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    let name = v.as_str().unwrap_or_default().to_string();
                    if name.is_empty() {
                        return None;
                    }
                    Some(PackageInfo {
                        name: name.clone(),
                        ecosystem: "npm".to_string(),
                        version: "pending".to_string(),
                        status: "pending".to_string(),
                        publisher: String::new(),
                        verified_at: None,
                        content_hash: String::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

async fn sse_event_listener(
    _app: Arc<RwLock<App>>,
    api_base: String,
    tx: mpsc::Sender<DataUpdate>,
) {
    let client = reqwest::Client::new();
    loop {
        let res = match client
            .get(format!("{}/v1/events", api_base))
            .header("Accept", "text/event-stream")
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

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

        // Stream ended, retry after delay
        let _ = tx
            .send(DataUpdate::Event(
                "System".to_string(),
                "SSE connection lost, reconnecting...".to_string(),
            ))
            .await;
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

fn apply_data_update(app: &mut App, update: DataUpdate) {
    match update {
        DataUpdate::Stats(stats) => {
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
        DataUpdate::MempoolTx(tx) => {
            app.mempool.push(tx);
        }
        DataUpdate::Peers(peers) => {
            app.peer_ids = peers;
        }
        DataUpdate::Error(_) => {}
    }
}

// ============================================================================
// INPUT HANDLING
// ============================================================================

async fn handle_key(app: &mut App, key: KeyCode) -> bool {
    // Handle search mode first
    if app.is_searching {
        match key {
            KeyCode::Esc => app.is_searching = false,
            KeyCode::Enter => app.is_searching = false,
            KeyCode::Char(c) => app.search_query.push(c),
            KeyCode::Backspace => {
                app.search_query.pop();
            }
            _ => {}
        }
        return false;
    }

    // Global shortcuts
    match key {
        KeyCode::Char('q') | KeyCode::Char('Q') => return true,
        KeyCode::Char('?') | KeyCode::Char('h') => {
            if app.current_view == View::Help {
                app.current_view = app.previous_view.unwrap_or(View::Overview);
                app.previous_view = None;
            } else {
                app.previous_view = Some(app.current_view);
                app.current_view = View::Help;
            }
            return false;
        }
        KeyCode::Char('/') => {
            app.is_searching = true;
            app.search_query.clear();
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
        _ => {}
    }

    // View-specific navigation
    match app.current_view {
        View::Blocks | View::Overview => match key {
            KeyCode::Down | KeyCode::Char('j') => {
                if app.selected_block < app.blocks.len().saturating_sub(1) {
                    app.selected_block += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.selected_block > 0 {
                    app.selected_block -= 1;
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
        View::Validators => match key {
            KeyCode::Down | KeyCode::Char('j') => {
                if app.selected_validator < app.validators.len().saturating_sub(1) {
                    app.selected_validator += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.selected_validator > 0 {
                    app.selected_validator -= 1;
                }
            }
            KeyCode::Enter | KeyCode::Char('d') => {
                if !app.validators.is_empty() {
                    app.previous_view = Some(app.current_view);
                    app.current_view = View::ValidatorDetail;
                }
            }
            _ => {}
        },
        View::Packages => match key {
            KeyCode::Down | KeyCode::Char('j') => {
                if app.selected_package < app.packages.len().saturating_sub(1) {
                    app.selected_package += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.selected_package > 0 {
                    app.selected_package -= 1;
                }
            }
            KeyCode::Enter | KeyCode::Char('d') => {
                if !app.packages.is_empty() {
                    app.previous_view = Some(app.current_view);
                    app.current_view = View::PackageDetail;
                }
            }
            _ => {}
        },
        View::Events => match key {
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
        View::BlockDetail | View::ValidatorDetail | View::PackageDetail => match key {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('b') => {
                app.current_view = app.previous_view.unwrap_or(View::Overview);
                app.previous_view = None;
            }
            _ => {}
        },
        _ => {}
    }

    false
}

fn handle_mouse(app: &mut App, mouse: crossterm::event::MouseEvent) {
    match mouse.kind {
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
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Main content
            Constraint::Length(3), // Footer
        ])
        .split(f.size());

    draw_header(f, app, main_chunks[0]);
    draw_main_content(f, app, main_chunks[1]);
    draw_footer(f, app, main_chunks[2]);

    if app.is_searching {
        draw_search_popup(f, app);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // Title with logo-like styling
    let title = format!(
        " ⛓ CHAIN REGISTRY CONSOLE   |  Height: #{}  |  {} Packages  |  {} Peers ",
        app.stats.tip_height,
        format_number(app.stats.package_count),
        app.stats.peer_count
    );

    let header = Paragraph::new(title)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Theme::PRIMARY)),
        )
        .style(
            Style::default()
                .fg(Theme::TEXT)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(header, chunks[0]);

    // Status indicator
    let validator_count = app.displayed_validator_count();
    let total_stake = app.displayed_total_stake();

    let status_color = if validator_count > 0 {
        Theme::SUCCESS
    } else {
        Theme::WARNING
    };

    let status_text = format!(
        " ● {} Validators  |  Total Stake: {} CREG  |  Bridge: {} ",
        validator_count,
        format_number(total_stake),
        if app.stats.bridge_status.len() > 15 {
            format!("{}..", &app.stats.bridge_status[..15])
        } else {
            app.stats.bridge_status.clone()
        }
    );

    let status = Paragraph::new(status_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(status_color)),
        )
        .style(Style::default().fg(Theme::TEXT))
        .alignment(Alignment::Right);
    f.render_widget(status, chunks[1]);
}

fn draw_main_content(f: &mut Frame, app: &App, area: Rect) {
    match app.current_view {
        View::Overview => draw_overview(f, app, area),
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
        View::Help => draw_help(f, app, area),
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let text = match app.current_view {
        View::Overview => " [←/↑/↓/→ or h/j/k/l] Navigate | [Enter/d] Detail | [1-8] Views | [o] Operator | [/] Search | [?] Help | [q] Quit ",
        View::Blocks => " [j/k] Navigate blocks | [Enter/d] Block detail | [b] Back | [?] Help | [q] Quit ",
        View::Validators => " [j/k] Navigate | [Enter/d] Validator detail | [b] Back | [?] Help | [q] Quit ",
        View::Packages => " [j/k] Navigate | [Enter/d] Package detail | [b] Back | [?] Help | [q] Quit ",
        View::Events => " [j/k] Scroll | [b] Back | [?] Help | [q] Quit ",
        View::Operator => " [1-8/o] Switch views | [q] Quit | Browser explorer at http://localhost:3000 ",
        View::BlockDetail | View::ValidatorDetail | View::PackageDetail => " [Esc/q/b] Back | [?] Help ",
        View::Help => " [Any key] Return ",
        _ => " [←→↑↓] Navigate | [?] Help | [q] Quit ",
    };

    let footer = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Theme::TEXT_DIM))
        .alignment(Alignment::Center);
    f.render_widget(footer, area);
}

// ============================================================================
// VIEW: OVERVIEW
// ============================================================================

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
        Theme::PRIMARY,
        stats_chunks[0],
    );
    draw_stat_card(
        f,
        "PACKAGES",
        &format_number(app.stats.package_count),
        Theme::SUCCESS,
        stats_chunks[1],
    );
    draw_stat_card(
        f,
        "VALIDATORS",
        &validator_count.to_string(),
        Theme::ACCENT,
        stats_chunks[2],
    );
    draw_stat_card(
        f,
        "TOTAL STAKE",
        &format!("{} CREG", format_number(total_stake)),
        Theme::WARNING,
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
        .border_style(Style::default().fg(color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = vec![
        Line::from(Span::styled(label, Style::default().fg(Theme::TEXT_DIM))),
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
                .border_style(Style::default().fg(Theme::SECONDARY)),
        )
        .data(&data)
        .max(max)
        .style(Style::default().fg(Theme::SUCCESS));

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
                "Block" => Theme::SUCCESS,
                "Package" => Theme::PRIMARY,
                "Validator" => Theme::ACCENT,
                "Slash" => Theme::ERROR,
                _ => Theme::TEXT_DIM,
            };

            let content = format!("[{:>3}] {:<10} {}", time_str, event_type, msg);
            ListItem::new(content).style(Style::default().fg(color))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" LIVE EVENTS ")
            .border_style(Style::default().fg(Theme::SUCCESS)),
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
    let title = if compact {
        " RECENT BLOCKS "
    } else {
        " BLOCKS (j/k to navigate, Enter for details) "
    };

    let items: Vec<ListItem> = app
        .blocks
        .iter()
        .enumerate()
        .map(|(i, block)| {
            let hash_short = if block.merkle_root.len() >= 16 {
                format!("{}..", &block.merkle_root[..16])
            } else {
                block.merkle_root.clone()
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
                    .fg(Theme::HIGHLIGHT)
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(Theme::TEXT)
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Theme::PRIMARY)),
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
            Span::styled("Height: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                format!("#{}", block.height),
                Style::default()
                    .fg(Theme::PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Merkle Root: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(&block.merkle_root),
        ]),
        Line::from(vec![
            Span::styled("Proposer: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(&block.proposer),
        ]),
        Line::from(vec![
            Span::styled("Transactions: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                block.tx_count.to_string(),
                Style::default().fg(Theme::SUCCESS),
            ),
        ]),
        Line::from(vec![
            Span::styled("Timestamp: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(format_timestamp(&block.timestamp)),
        ]),
    ];

    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" BLOCK PREVIEW ")
            .border_style(Style::default().fg(Theme::PRIMARY)),
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
            Span::styled("Block #", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                block.height.to_string(),
                Style::default()
                    .fg(Theme::PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Merkle Root: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(&block.merkle_root),
        ]),
        Line::from(vec![
            Span::styled("Proposer:    ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(&block.proposer, Style::default().fg(Theme::ACCENT)),
        ]),
        Line::from(vec![
            Span::styled("Timestamp:   ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(format_timestamp(&block.timestamp)),
        ]),
        Line::from(vec![
            Span::styled("Transactions:", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                block.tx_count.to_string(),
                Style::default().fg(Theme::SUCCESS),
            ),
        ]),
    ];

    let header = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" BLOCK HEADER ")
            .border_style(Style::default().fg(Theme::PRIMARY)),
    );
    f.render_widget(header, chunks[0]);

    // Transactions table
    let rows: Vec<Row> = block
        .transactions
        .iter()
        .map(|tx| {
            Row::new(vec![
                Cell::from(tx.tx_type.clone()).style(Style::default().fg(Theme::PRIMARY)),
                Cell::from(tx.package_name.clone().unwrap_or_default()),
                Cell::from(tx.package_version.clone().unwrap_or_default()),
                Cell::from(tx.publisher.clone().unwrap_or_default()),
                Cell::from(tx.status.clone()).style(Style::default().fg(Theme::SUCCESS)),
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
                .fg(Theme::TEXT_DIM)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" TRANSACTIONS ({}) ", block.tx_count))
            .border_style(Style::default().fg(Theme::SECONDARY)),
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
    let rows: Vec<Row> = app
        .validators
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let status_color = match v.status.as_str() {
                "online" | "self" => Theme::SUCCESS,
                "pending" => Theme::WARNING,
                _ => Theme::ERROR,
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
                .fg(Theme::TEXT_DIM)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" VALIDATORS (j/k to navigate, Enter for details) ")
            .border_style(Style::default().fg(Theme::ACCENT)),
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
                "online" | "self" => Theme::SUCCESS,
                _ => Theme::TEXT_DIM,
            };

            Row::new(vec![
                Cell::from(v.id.chars().take(20).collect::<String>()),
                Cell::from(format!("{}", v.stake / 1_000_000_000)),
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
        Row::new(vec!["Validator", "Stake(k)", "Rep"]).style(Style::default().fg(Theme::TEXT_DIM)),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" TOP VALIDATORS ")
            .border_style(Style::default().fg(Theme::ACCENT)),
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
            Span::styled("Total: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                app.validators.len().to_string(),
                Style::default().fg(Theme::TEXT),
            ),
        ]),
        Line::from(vec![
            Span::styled("Active: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                active_count.to_string(),
                Style::default().fg(Theme::SUCCESS),
            ),
        ]),
        Line::from(vec![
            Span::styled("Avg Rep: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                format!("{}/100", avg_reputation),
                Style::default().fg(Theme::WARNING),
            ),
        ]),
    ];

    let stats = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" STATS ")
            .border_style(Style::default().fg(Theme::ACCENT)),
    );

    f.render_widget(stats, area);
}

fn draw_validator_detail(f: &mut Frame, app: &App, area: Rect) {
    let validator = match app.selected_validator() {
        Some(v) => v,
        None => return,
    };

    let status_color = match validator.status.as_str() {
        "online" | "self" => Theme::SUCCESS,
        "pending" => Theme::WARNING,
        _ => Theme::ERROR,
    };

    let text = vec![
        Line::from(vec![Span::styled(
            "VALIDATOR DETAILS\n",
            Style::default()
                .fg(Theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("ID:          ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                &validator.id,
                Style::default()
                    .fg(Theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Alias:       ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(&validator.alias),
        ]),
        Line::from(vec![
            Span::styled("Public Key:  ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(format!(
                "{}..",
                &validator.pub_key[..validator.pub_key.len().min(40)]
            )),
        ]),
        Line::from(vec![
            Span::styled("Stake:       ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                format!("{} CREG", format_number(validator.stake)),
                Style::default().fg(Theme::SUCCESS),
            ),
        ]),
        Line::from(vec![
            Span::styled("Reputation:  ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                format!("{}/100", validator.reputation),
                Style::default().fg(Theme::WARNING),
            ),
        ]),
        Line::from(vec![
            Span::styled("Status:      ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(&validator.status, Style::default().fg(status_color)),
        ]),
        Line::from(vec![
            Span::styled("Active:      ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                if validator.is_active { "Yes" } else { "No" },
                Style::default().fg(if validator.is_active {
                    Theme::SUCCESS
                } else {
                    Theme::ERROR
                }),
            ),
        ]),
    ];

    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" VALIDATOR ")
            .border_style(Style::default().fg(Theme::ACCENT)),
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
        let text = Paragraph::new("No packages found. Packages will appear here when published to the registry.")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" PACKAGES ({} on-chain) ", app.stats.package_count))
                    .border_style(Style::default().fg(Theme::PRIMARY)),
            )
            .style(Style::default().fg(Theme::TEXT_DIM));
        f.render_widget(text, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let items: Vec<ListItem> = app
        .packages
        .iter()
        .enumerate()
        .map(|(i, pkg)| {
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
                    .fg(Theme::HIGHLIGHT)
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::REVERSED)
            } else {
                let color = match pkg.status.as_str() {
                    "verified" => Theme::SUCCESS,
                    "pending" => Theme::WARNING,
                    "rejected" => Theme::ERROR,
                    _ => Theme::TEXT,
                };
                Style::default().fg(color)
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " PACKAGES ({}) — j/k to navigate, Enter for details ",
                app.packages.len()
            ))
            .border_style(Style::default().fg(Theme::PRIMARY)),
    );
    f.render_widget(list, chunks[0]);

    // Package detail preview
    match app.selected_package() {
        Some(pkg) => {
            let text = vec![
                Line::from(vec![
                    Span::styled("Name:       ", Style::default().fg(Theme::TEXT_DIM)),
                    Span::styled(&pkg.name, Style::default().fg(Theme::PRIMARY).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled("Version:    ", Style::default().fg(Theme::TEXT_DIM)),
                    Span::raw(&pkg.version),
                ]),
                Line::from(vec![
                    Span::styled("Ecosystem:  ", Style::default().fg(Theme::TEXT_DIM)),
                    Span::raw(&pkg.ecosystem),
                ]),
                Line::from(vec![
                    Span::styled("Status:     ", Style::default().fg(Theme::TEXT_DIM)),
                    Span::styled(&pkg.status, Style::default().fg(match pkg.status.as_str() {
                        "verified" => Theme::SUCCESS,
                        "pending" => Theme::WARNING,
                        _ => Theme::ERROR,
                    })),
                ]),
                Line::from(vec![
                    Span::styled("Publisher:  ", Style::default().fg(Theme::TEXT_DIM)),
                    Span::raw(if pkg.publisher.len() > 16 {
                        format!("{}...", &pkg.publisher[..16])
                    } else {
                        pkg.publisher.clone()
                    }),
                ]),
                Line::from(vec![
                    Span::styled("Hash:       ", Style::default().fg(Theme::TEXT_DIM)),
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
                    .border_style(Style::default().fg(Theme::PRIMARY)),
            );
            f.render_widget(detail, chunks[1]);
        }
        None => {
            let empty = Paragraph::new("Select a package to see details")
                .style(Style::default().fg(Theme::TEXT_DIM))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" PACKAGE PREVIEW ")
                        .border_style(Style::default().fg(Theme::PRIMARY)),
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
                .fg(Theme::PRIMARY)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Name:         ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(&pkg.name, Style::default().fg(Theme::TEXT).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Version:      ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(&pkg.version),
        ]),
        Line::from(vec![
            Span::styled("Ecosystem:    ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(&pkg.ecosystem),
        ]),
        Line::from(vec![
            Span::styled("Status:       ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(&pkg.status, Style::default().fg(match pkg.status.as_str() {
                "verified" => Theme::SUCCESS,
                "pending" => Theme::WARNING,
                _ => Theme::ERROR,
            })),
        ]),
        Line::from(vec![
            Span::styled("Publisher:    ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(&pkg.publisher),
        ]),
        Line::from(vec![
            Span::styled("Content Hash: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(&pkg.content_hash),
        ]),
        Line::from(vec![
            Span::styled("Verified At:  ", Style::default().fg(Theme::TEXT_DIM)),
            Span::raw(pkg.verified_at.as_deref().unwrap_or("Not yet")),
        ]),
    ];

    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" PACKAGE ")
            .border_style(Style::default().fg(Theme::PRIMARY)),
    );
    f.render_widget(paragraph, area);
}

// ============================================================================
// VIEW: NETWORK
// ============================================================================

fn draw_network(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(0)])
        .split(area);

    // Network stats
    let stats_text = vec![
        Line::from(vec![
            Span::styled("Connected Peers: ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                app.stats.peer_count.to_string(),
                Style::default().fg(Theme::SUCCESS),
            ),
        ]),
        Line::from(vec![
            Span::styled("Bridge Status:   ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                &app.stats.bridge_status,
                Style::default().fg(Theme::PRIMARY),
            ),
        ]),
        Line::from(vec![
            Span::styled("L1 Block:        ", Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                format!("#{}", app.stats.l1_block),
                Style::default().fg(Theme::WARNING),
            ),
        ]),
    ];

    let stats = Paragraph::new(stats_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" NETWORK STATUS ")
            .border_style(Style::default().fg(Theme::SECONDARY)),
    );
    f.render_widget(stats, chunks[0]);

    // Peer list
    let peers: Vec<ListItem> = app
        .peer_ids
        .iter()
        .map(|p| ListItem::new(format!("● {}", p)).style(Style::default().fg(Theme::SUCCESS)))
        .collect();

    let peer_list = List::new(peers).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" CONNECTED PEERS ")
            .border_style(Style::default().fg(Theme::SECONDARY)),
    );

    f.render_widget(peer_list, chunks[1]);
}

// ============================================================================
// VIEW: MEMPOOL
// ============================================================================

fn draw_mempool(f: &mut Frame, app: &App, area: Rect) {
    let text = format!(
        "Mempool Transactions: {}\n\nPending transactions waiting to be included in blocks.",
        app.mempool.len()
    );

    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" MEMPOOL ")
            .border_style(Style::default().fg(Theme::WARNING)),
    );

    f.render_widget(paragraph, area);
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
                "Block" => Theme::SUCCESS,
                "Package" => Theme::PRIMARY,
                "Validator" => Theme::ACCENT,
                "Slash" => Theme::ERROR,
                _ => Theme::TEXT_DIM,
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
            .border_style(Style::default().fg(Theme::SUCCESS)),
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
            Constraint::Length(8),
            Constraint::Min(12),
        ])
        .split(area);

    let active_validators = app
        .validators
        .iter()
        .filter(|v| v.is_active || v.status == "online" || v.status == "self")
        .count();

    let summary = Paragraph::new(vec![
        Line::from(format!("Mode: validator console")),
        Line::from(format!("Validators online: {}/{}", active_validators, app.validators.len())),
        Line::from(format!("Connected peers: {}", app.peer_ids.len())),
        Line::from(format!("Bridge status: {}", app.stats.bridge_status)),
    ])
    .block(Block::default().borders(Borders::ALL).title(" Operator Summary "))
    .style(Style::default().fg(Theme::TEXT))
    .wrap(Wrap { trim: true });
    f.render_widget(summary, chunks[0]);

    let commands = Paragraph::new(vec![
        Line::from("Browser explorer:  http://localhost:3000"),
        Line::from("Node health:        creg testnet status --node-url http://localhost:8080"),
        Line::from("Publish package:    creg publish <tarball>.tar.gz --key-file <publisher.key> --node-url http://localhost:8080"),
        Line::from("Stake validator:    creg testnet stake-validator --key 0x<private-key> 100"),
        Line::from("Stake publisher:    creg testnet stake-publisher --key 0x<private-key> 100"),
    ])
    .block(Block::default().borders(Borders::ALL).title(" Operator Commands "))
    .style(Style::default().fg(Theme::TEXT))
    .wrap(Wrap { trim: true });
    f.render_widget(commands, chunks[1]);

    let workflow = Paragraph::new(vec![
        Line::from("This console is the operator surface for validator monitoring."),
        Line::from("Use it to inspect blocks, validator health, packages, peers, and live events."),
        Line::from(""),
        Line::from("Recommended workflow:"),
        Line::from("  1. Keep `creg console` open during validator operation."),
        Line::from("  2. Use the browser explorer for wallet connect, faucet, and staking UX."),
        Line::from("  3. Use `creg watch` in a second terminal for focused event streams."),
        Line::from(""),
        Line::from("View shortcuts:"),
        Line::from("  1 Overview   2 Blocks   3 Validators   4 Packages"),
        Line::from("  5 Network    6 Mempool  7 Events       8 Operator"),
    ])
    .block(Block::default().borders(Borders::ALL).title(" Operator Workflow "))
    .style(Style::default().fg(Theme::TEXT))
    .wrap(Wrap { trim: true });
    f.render_widget(workflow, chunks[2]);
}

// ============================================================================
// VIEW: HELP
// ============================================================================

fn draw_help(f: &mut Frame, _app: &App, area: Rect) {
    let text = r#"
╔══════════════════════════════════════════════════════════════════════════════╗
║                      CHAIN REGISTRY CONSOLE - HELP                            ║
╠══════════════════════════════════════════════════════════════════════════════╣
║                                                                              ║
║  NAVIGATION                                                                  ║
║  ─────────                                                                   ║
║    ←/→ or h/l     Move between columns/tabs                                  ║
║    ↑/↓ or j/k     Navigate lists                                             ║
║    Enter or d     Open detail view                                           ║
║    Esc or b       Go back                                                    ║
║    q              Quit console                                               ║
║                                                                              ║
║  VIEW SHORTCUTS                                                              ║
║  ─────────────                                                               ║
║    1              Overview dashboard                                         ║
║    2              Blocks view                                                ║
║    3              Validators view                                            ║
║    4              Packages view                                              ║
║    5              Network status                                             ║
║    6              Mempool view                                               ║
║    7              Events log                                                 ║
║    8 / o          Operator view                                              ║
║    ? or h         Toggle this help                                           ║
║                                                                              ║
║  SEARCH & FILTER                                                             ║
║  ──────────────                                                              ║
║    /              Start search                                               ║
║    Esc            Cancel search                                              ║
║    Enter          Confirm search                                             ║
║                                                                              ║
║  MOUSE SUPPORT                                                               ║
║  ─────────────                                                               ║
║    Scroll         Navigate lists                                             ║
║    Click          Select items (where supported)                             ║
║                                                                              ║
║  PRESS ANY KEY TO RETURN...                                                  ║
║                                                                              ║
╚══════════════════════════════════════════════════════════════════════════════╝
"#;

    let help = Paragraph::new(text)
        .block(Block::default())
        .style(Style::default().fg(Theme::PRIMARY));

    f.render_widget(help, area);
}

// ============================================================================
// POPUPS
// ============================================================================

fn draw_search_popup(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 20, f.size());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" SEARCH ")
        .border_style(Style::default().fg(Theme::ACCENT));

    let text = Paragraph::new(format!("Query: {}", app.search_query))
        .block(block)
        .style(Style::default().fg(Theme::TEXT));

    f.render_widget(Clear, area);
    f.render_widget(text, area);
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
