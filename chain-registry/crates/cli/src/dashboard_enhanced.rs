// crates/cli/src/dashboard_enhanced.rs
// Enhanced TUI dashboard with interactive features.

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect, Alignment, Margin},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, List, ListItem, Paragraph, Row, Table, Clear, Wrap},
    Frame, Terminal,
};
use serde_json::Value;
use std::{io, time::{Duration, Instant}};
use tokio::sync::mpsc;
use futures::StreamExt;

// const API_BASE: &str = "http://localhost:8080";

/// Current view state for navigation
#[derive(Debug, Clone, Copy, PartialEq)]
enum ViewState {
    Main,
    PackageDetail,
    ValidatorDetail,
    BlockDetail,
}

/// Selected item indices for navigation
#[derive(Debug)]
struct Selection {
    block_index: usize,
    validator_index: usize,
    event_index: usize,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            block_index: 0,
            validator_index: 0,
            event_index: 0,
        }
    }
}

#[derive(Debug)]
struct App {
    stats: Value,
    nodes: Vec<Value>,
    events: Vec<String>,
    blocks: Vec<Value>,
    error: Option<String>,
    last_tick: Instant,
    view: ViewState,
    selection: Selection,
    show_help: bool,
    filter: String,
    bridge: Value,
    api_base: String,
}

impl App {
    fn new(api_base: String) -> App {
        App {
            stats: serde_json::json!({ "tip_height": 0, "package_count": 0 }),
            nodes: Vec::new(),
            events: Vec::new(),
            blocks: Vec::new(),
            error: None,
            last_tick: Instant::now(),
            view: ViewState::Main,
            selection: Selection::default(),
            show_help: false,
            filter: String::new(),
            bridge: serde_json::json!({ "bridge_sync_status": "Idle", "current_state_root": "0x0", "last_finalized_eth_block": 0 }),
            api_base,
        }
    }

    async fn refresh_data(&mut self) -> Result<()> {
        let client = reqwest::Client::new();
        let api_base = &self.api_base;
        
        // Stats
        if let Ok(res) = client.get(format!("{}/v1/chain/stats", api_base)).send().await {
            if let Ok(json) = res.json::<Value>().await {
                self.stats = json;
            }
        }
        
        // Nodes
        if let Ok(res) = client.get(format!("{}/v1/nodes", api_base)).send().await {
            if let Ok(json) = res.json::<Vec<Value>>().await {
                self.nodes = json;
            }
        }

        // Blocks
        let height = self.stats["tip_height"].as_u64().unwrap_or(0);
        let mut recent_blocks = Vec::new();
        for h in (height.saturating_sub(20)..=height).rev() {
            if let Ok(res) = client.get(format!("{}/v1/blocks/{}", api_base, h)).send().await {
                if let Ok(json) = res.json::<Value>().await {
                    recent_blocks.push(json);
                }
            }
        }
        self.blocks = recent_blocks;

        // Bridge
        if let Ok(res) = client.get(format!("{}/v1/bridge/status", api_base)).send().await {
            if let Ok(json) = res.json::<Value>().await {
                self.bridge = json;
            }
        }

        Ok(())
    }

    fn next_block(&mut self) {
        if !self.blocks.is_empty() {
            self.selection.block_index = (self.selection.block_index + 1) % self.blocks.len();
        }
    }

    fn previous_block(&mut self) {
        if !self.blocks.is_empty() {
            self.selection.block_index = self.selection.block_index.saturating_sub(1);
        }
    }

    fn next_validator(&mut self) {
        if !self.nodes.is_empty() {
            self.selection.validator_index = (self.selection.validator_index + 1) % self.nodes.len();
        }
    }

    fn previous_validator(&mut self) {
        if !self.nodes.is_empty() {
            self.selection.validator_index = self.selection.validator_index.saturating_sub(1);
        }
    }

    fn get_selected_block(&self) -> Option<&Value> {
        self.blocks.get(self.selection.block_index)
    }

    fn get_selected_validator(&self) -> Option<&Value> {
        self.nodes.get(self.selection.validator_index)
    }
}

pub async fn run(node_url: Option<&str>) -> Result<()> {
    let api_base = node_url
        .map(String::from)
        .unwrap_or_else(|| {
            std::env::var("CREG_NODE_URL")
                .unwrap_or_else(|_| "http://localhost:8080".into())
        })
        .trim_end_matches('/')
        .to_string();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(api_base.clone());
    let (tx, mut rx) = mpsc::channel(100);
    
    // Spawn SSE listener
    let tx_sse = tx.clone();
    let api_base_sse = api_base.clone();
    tokio::spawn(async move {
        let _ = listen_sse(tx_sse, api_base_sse).await;
    });

    let tick_rate = Duration::from_millis(100);
    let mut last_refresh = Instant::now();

    loop {
        terminal.draw(|f| ui(f, &app))?;

        let timeout = tick_rate
            .checked_sub(app.last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match app.view {
                        ViewState::Main => {
                            match key.code {
                                KeyCode::Char('q') => break,
                                KeyCode::Char('h') => app.show_help = !app.show_help,
                                KeyCode::Char('p') => {
                                    if !app.blocks.is_empty() {
                                        app.view = ViewState::PackageDetail;
                                    }
                                }
                                KeyCode::Char('v') => {
                                    if !app.nodes.is_empty() {
                                        app.view = ViewState::ValidatorDetail;
                                    }
                                }
                                KeyCode::Char('b') => {
                                    if !app.blocks.is_empty() {
                                        app.view = ViewState::BlockDetail;
                                    }
                                }
                                KeyCode::Char('r') => {
                                    let _ = app.refresh_data().await;
                                }
                                KeyCode::Char('/') => {
                                    // Start filtering (simplified)
                                    app.filter = "filter: ".to_string();
                                }
                                KeyCode::Down => app.next_block(),
                                KeyCode::Up => app.previous_block(),
                                KeyCode::Right => app.next_validator(),
                                KeyCode::Left => app.previous_validator(),
                                _ => {}
                            }
                        }
                        _ => {
                            // Any key returns to main view
                            if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                                app.view = ViewState::Main;
                            }
                        }
                    }
                }
            }
        }

        if app.last_tick.elapsed() >= tick_rate {
            app.last_tick = Instant::now();
        }

        if last_refresh.elapsed() >= Duration::from_secs(5) {
            app.refresh_data().await?;
            last_refresh = Instant::now();
        }

        // Handle SSE messages
        while let Ok(msg) = rx.try_recv() {
            app.events.insert(0, msg);
            if app.events.len() > 100 { app.events.pop(); }
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

async fn listen_sse(tx: mpsc::Sender<String>, api_base: String) -> Result<()> {
    let client = reqwest::Client::new();
    let mut stream = client
        .get(format!("{}/v1/events", api_base))
        .header("Accept", "text/event-stream")
        .send()
        .await?
        .bytes_stream();

    let mut buffer = String::new();
    while let Some(item) = stream.next().await {
        if let Ok(chunk) = item {
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = buffer.find("\n\n") {
                let msg = buffer[..end].to_string();
                buffer = buffer[end + 2..].to_string();
                
                for line in msg.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        if let Ok(v) = serde_json::from_str::<Value>(data) {
                            let kind = v["kind"].as_str().unwrap_or("Event");
                            let payload = v["payload"].get("canonical")
                                .or_else(|| v["payload"].get("hash"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let summary = format!("[{}] {}", kind, payload);
                            let _ = tx.send(summary).await;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn ui(f: &mut Frame, app: &App) {
    match app.view {
        ViewState::Main => draw_main_view(f, app),
        ViewState::PackageDetail => draw_package_detail(f, app),
        ViewState::ValidatorDetail => draw_validator_detail(f, app),
        ViewState::BlockDetail => draw_block_detail(f, app),
    }
    
    if app.show_help {
        draw_help_popup(f);
    }
}

fn draw_main_view(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(12),
            Constraint::Length(4), // New L2 Summary section
        ])
        .split(f.size());

    // Header
    let height = app.stats["tip_height"].as_u64().unwrap_or(0);
    let pkg_count = app.stats["package_count"].as_u64().unwrap_or(0);
    let header_text = format!(
        " CHAIN REGISTRY | Height: {} | Verified: {} | Nodes: {} | Status: ONLINE",
        height, pkg_count, app.nodes.len()
    );
    let header = Paragraph::new(header_text)
        .block(Block::default().borders(Borders::ALL).title(" DASHBOARD ").border_style(Style::default().fg(Color::Cyan)))
        .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));
    f.render_widget(header, chunks[0]);

    // Main content
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);

    // Blocks list with selection
    let block_items: Vec<ListItem> = app.blocks.iter().enumerate()
        .map(|(i, b)| {
            let h = b["header"]["height"].as_u64().unwrap_or(0);
            let hash = b["hash"].as_str().unwrap_or("?");
            let txs = b["transactions"].as_array().map(|v| v.len()).unwrap_or(0);
            let is_shielded = b["transactions"].as_array()
                .map(|txs_arr| txs_arr.iter().any(|tx| tx["shielded"].as_bool().unwrap_or(false)))
                .unwrap_or(false);
            let icon = if is_shielded { "🔒" } else { "  " };
            let hash_display = if hash.len() > 16 { &hash[..16] } else { hash };
            let content = format!("#{} {} {} ({} tx)", h, icon, hash_display, txs);
            
            let style = if i == app.selection.block_index {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD).add_modifier(Modifier::REVERSED)
            } else if is_shielded {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::Gray)
            };
            
            ListItem::new(content).style(style)
        })
        .collect();
    
    let block_list = List::new(block_items)
        .block(Block::default().borders(Borders::ALL).title(" RECENT BLOCKS (↑↓ to navigate, b for details) "))
        .highlight_style(Style::default().add_modifier(Modifier::ITALIC))
        .highlight_symbol(">>");
    f.render_widget(block_list, body_chunks[0]);

    // Validator table with selection
    let rows: Vec<Row> = app.nodes.iter().enumerate()
        .map(|(i, n)| {
            let id = n["id"].as_str().unwrap_or("?");
            let stake = format!("{}k", n["stake"].as_u64().unwrap_or(0) / 1000);
            let status = n["status"].as_str().unwrap_or("?");
            
            let style = if i == app.selection.validator_index {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            
            Row::new(vec![
                Span::styled(id, Style::default().fg(Color::Yellow)),
                Span::raw(stake),
                Span::styled(status, Style::default().fg(if status == "online" || status == "self" { Color::Green } else { Color::Red })),
            ]).style(style)
        })
        .collect();
    
    let network_table = Table::new(rows, [Constraint::Percentage(40), Constraint::Percentage(30), Constraint::Percentage(30)])
        .block(Block::default().borders(Borders::ALL).title(" NETWORK HEALTH (←→ to navigate, v for details) "))
        .header(Row::new(vec!["Validator", "Stake", "Status"]).style(Style::default().fg(Color::Gray)));
    f.render_widget(network_table, body_chunks[1]);

    // Event feed
    let event_items: Vec<ListItem> = app.events.iter()
        .map(|e| ListItem::new(e.clone()).style(Style::default().fg(Color::DarkGray)))
        .collect();
    let event_list = List::new(event_items)
        .block(Block::default().borders(Borders::ALL).title(" LIVE FEED "))
        .style(Style::default().fg(Color::Gray));
    f.render_widget(event_list, chunks[2]);

    // L2 Settlement Health (Enhanced with Efficiency Metric)
    let rollup_status = app.bridge["bridge_sync_status"].as_str().unwrap_or("Idle");
    let state_root = app.bridge["current_state_root"].as_str().unwrap_or("0x0");
    let eth_block = app.bridge["last_finalized_eth_block"].as_u64().unwrap_or(0);
    
    // Efficiency Calculation: 120k gas per tx on L1 vs L2 batching.
    let verified_count = app.stats["package_count"].as_u64().unwrap_or(0);
    let estimated_savings = verified_count * 115_000; // Average savings per pkg
    
    let l2_info = format!(
        "Rollup: {} | Root: {} | L1 Finality: #{}\n⚡ Gas Savings: {}k units (Estimated)",
        rollup_status, 
        if state_root.len() > 18 { format!("{}...{}", &state_root[..8], &state_root[state_root.len()-6..]) } else { state_root.to_string() },
        eth_block,
        estimated_savings / 1000
    );
    
    let l2_block = Paragraph::new(l2_info)
        .block(Block::default().borders(Borders::ALL).title(" ⛓️ L2 SETTLEMENT HEALTH ")
        .border_style(Style::default().fg(if rollup_status == "L2 Scaled" { Color::Green } else { Color::Yellow })))
        .style(Style::default().fg(Color::White))
        .alignment(Alignment::Center);
    f.render_widget(l2_block, chunks[3]);

    // Help hint at bottom
    let help_text = Paragraph::new("Press 'h' for help, 'q' to quit, 'r' to refresh")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    f.render_widget(help_text, chunks[2].inner(&Margin { horizontal: 1, vertical: 1 }));
}

fn draw_package_detail(f: &mut Frame, app: &App) {
    let area = centered_rect(80, 80, f.size());
    
    let block = app.get_selected_block();
    let content = if let Some(b) = block {
        let height = b["header"]["height"].as_u64().unwrap_or(0);
        let hash = b["hash"].as_str().unwrap_or("?");
        let txs = b["transactions"].as_array().map(|v| v.len()).unwrap_or(0);
        
        format!(
            "Block Details\n\n\
            Height: {}\n\
            Hash: {}\n\
            Transactions: {}\n\n\
            Press ESC or q to return",
            height, hash, txs
        )
    } else {
        "No block selected\n\nPress ESC or q to return".to_string()
    };
    
    let paragraph = Paragraph::new(content)
        .block(Block::default().borders(Borders::ALL).title(" Block Detail "))
        .wrap(Wrap { trim: true });
    
    f.render_widget(Clear, area);
    f.render_widget(paragraph, area);
}

fn draw_validator_detail(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 60, f.size());
    
    let validator = app.get_selected_validator();
    let content = if let Some(v) = validator {
        let id = v["id"].as_str().unwrap_or("?");
        let stake = v["stake"].as_u64().unwrap_or(0);
        let reputation = v["reputation"].as_u64().unwrap_or(0);
        let status = v["status"].as_str().unwrap_or("?");
        let alias = v["alias"].as_str().unwrap_or("?");
        
        format!(
            "Validator Details\n\n\
            ID: {}\n\
            Alias: {}\n\
            Stake: {} ETH\n\
            Reputation: {}/100\n\
            Status: {}\n\n\
            Press ESC or q to return",
            id, alias, stake / 1_000_000_000_000_000_000, reputation, status
        )
    } else {
        "No validator selected\n\nPress ESC or q to return".to_string()
    };
    
    let paragraph = Paragraph::new(content)
        .block(Block::default().borders(Borders::ALL).title(" Validator Detail "))
        .wrap(Wrap { trim: true });
    
    f.render_widget(Clear, area);
    f.render_widget(paragraph, area);
}

fn draw_block_detail(f: &mut Frame, app: &App) {
    // Similar to package detail but with block-specific info
    draw_package_detail(f, app);
}

fn draw_help_popup(f: &mut Frame) {
    let area = centered_rect(70, 70, f.size());
    
    let help_text = r#"Keyboard Shortcuts

Navigation:
  ↑ / Down     Navigate blocks list
  ← / Right    Navigate validators
  Enter        Select item
  Esc / q      Go back / Quit

Views:
  b            Show block details
  v            Show validator details
  p            Show package details

Actions:
  r            Refresh data
  h            Toggle this help
  /            Filter events

Press any key to close this help.
"#;
    
    let paragraph = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title(" Help "))
        .wrap(Wrap { trim: true });
    
    f.render_widget(Clear, area);
    f.render_widget(paragraph, area);
}

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
