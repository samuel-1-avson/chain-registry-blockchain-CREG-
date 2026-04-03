use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, List, ListItem, Paragraph, Row, Table},
    Frame, Terminal,
};
use serde_json::Value;
use std::{
    io,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

// const API_BASE: &str = "http://localhost:8080";

#[allow(dead_code)]
struct App {
    stats: Value,
    nodes: Vec<Value>,
    events: Vec<String>,
    blocks: Vec<Value>,
    bridge_status: Value,
    error: Option<String>,
    last_tick: Instant,
}

impl App {
    fn new() -> App {
        App {
            stats: serde_json::json!({ "tip_height": 0, "package_count": 0 }),
            nodes: Vec::new(),
            events: Vec::new(),
            blocks: Vec::new(),
            bridge_status: serde_json::json!({ "bridge_sync_status": "Starting...", "current_state_root": "0x0" }),
            error: None,
            last_tick: Instant::now(),
        }
    }

    async fn refresh_data(&mut self, api_base: &str) -> Result<()> {
        let client = reqwest::Client::new();

        // Stats
        if let Ok(res) = client
            .get(format!("{}/v1/chain/stats", api_base))
            .send()
            .await
        {
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

        // Blocks (fetch recent ones)
        let height = self.stats["tip_height"].as_u64().unwrap_or(0);
        let mut recent_blocks = Vec::new();
        for h in (height.saturating_sub(10)..=height).rev() {
            if let Ok(res) = client
                .get(format!("{}/v1/blocks/{}", api_base, h))
                .send()
                .await
            {
                if let Ok(json) = res.json::<Value>().await {
                    recent_blocks.push(json);
                }
            }
        }
        self.blocks = recent_blocks;

        // Bridge Status
        if let Ok(res) = client
            .get(format!("{}/v1/bridge/status", api_base))
            .send()
            .await
        {
            if let Ok(json) = res.json::<Value>().await {
                self.bridge_status = json;
            }
        }

        Ok(())
    }
}

pub async fn run(node_url: Option<&str>) -> Result<()> {
    let api_base = node_url
        .map(String::from)
        .unwrap_or_else(|| {
            std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
        })
        .trim_end_matches('/')
        .to_string();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    // Load data immediately — don't make the user wait 5 seconds
    let _ = app.refresh_data(&api_base).await;

    let (tx, mut rx) = mpsc::channel(100);

    // Spawn SSE listener
    let tx_sse = tx.clone();
    let api_base_sse = api_base.clone();
    tokio::spawn(async move {
        let _ = listen_sse(tx_sse, api_base_sse).await;
    });

    let tick_rate = Duration::from_millis(1000);
    let mut last_refresh = Instant::now();

    loop {
        terminal.draw(|f| ui(f, &app))?;

        let timeout = tick_rate
            .checked_sub(app.last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if let KeyCode::Char('q') = key.code {
                    break;
                }
            }
        }

        if app.last_tick.elapsed() >= tick_rate {
            app.last_tick = Instant::now();
        }

        if last_refresh.elapsed() >= Duration::from_secs(5) {
            app.refresh_data(&api_base).await?;
            last_refresh = Instant::now();
        }

        // Handle SSE messages
        while let Ok(msg) = rx.try_recv() {
            app.events.insert(0, msg);
            if app.events.len() > 50 {
                app.events.pop();
            }
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
                            let payload = v["payload"]
                                .get("canonical")
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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(10),
        ])
        .split(f.size());

    // ── Header ───────────────────────────────────────────────────────────────
    let height = app.stats["tip_height"].as_u64().unwrap_or(0);
    let pkg_count = app.stats["package_count"].as_u64().unwrap_or(0);
    let rollup_status = app.bridge_status["bridge_sync_status"]
        .as_str()
        .unwrap_or("?");
    let root = app.bridge_status["current_state_root"]
        .as_str()
        .unwrap_or("0x0");
    let root_short = if root.len() > 10 {
        format!("{}...{}", &root[..6], &root[root.len() - 4..])
    } else {
        root.to_string()
    };

    let header_text = format!(
        " CHAIN REGISTRY | Height: {} | Pkgs: {} | Nodes: {} | Rollup: {} ({})",
        height,
        pkg_count,
        app.nodes.len(),
        rollup_status,
        root_short
    );
    let header = Paragraph::new(header_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" DASHBOARD ")
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(header, chunks[0]);

    // ── Main Content ──────────────────────────────────────────────────────────
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);

    // Left: Blocks
    let block_items: Vec<ListItem> = app
        .blocks
        .iter()
        .map(|b| {
            let h = b["header"]["height"].as_u64().unwrap_or(0);
            // API doesn't return top-level hash — use merkle_root as block fingerprint
            let root = b["header"]["merkle_root"]
                .as_str()
                .unwrap_or("0000000000000000");
            let hash_display = &root[..root.len().min(14)];
            let txs = b["transactions"].as_array().map(|v| v.len()).unwrap_or(0);
            let content = format!("#{:<4}  {}..  ({} tx)", h, hash_display, txs);
            let style = if h == 0 {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(content).style(style)
        })
        .collect();
    let block_list = List::new(block_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" RECENT BLOCKS "),
        )
        .highlight_style(Style::default().add_modifier(Modifier::ITALIC))
        .highlight_symbol(">>");
    f.render_widget(block_list, body_chunks[0]);

    // Right: Network
    let rows: Vec<Row> = app
        .nodes
        .iter()
        .map(|n| {
            let id = n["id"].as_str().unwrap_or("?");
            let alias = n["alias"].as_str().unwrap_or("");
            let stake = format!("{} CREG", n["stake"].as_u64().unwrap_or(0));
            let rep = format!("{}/100", n["reputation"].as_u64().unwrap_or(0));
            let status = n["status"].as_str().unwrap_or("?");
            let label = if alias.is_empty() {
                id.to_string()
            } else {
                format!("{} ({})", id, alias)
            };
            Row::new(vec![
                Span::styled(label, Style::default().fg(Color::Yellow)),
                Span::raw(stake),
                Span::raw(rep),
                Span::styled(
                    status,
                    Style::default().fg(if status == "online" || status == "self" {
                        Color::Green
                    } else {
                        Color::Red
                    }),
                ),
            ])
        })
        .collect();
    let network_table = Table::new(
        rows,
        [
            Constraint::Percentage(35),
            Constraint::Percentage(25),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" NETWORK HEALTH "),
    )
    .header(
        Row::new(vec!["Validator", "Stake", "Rep", "Status"])
            .style(Style::default().fg(Color::Gray)),
    );
    f.render_widget(network_table, body_chunks[1]);

    // ── Footer: Live Events ──────────────────────────────────────────────────
    let height = app.stats["tip_height"].as_u64().unwrap_or(0);
    let pkg_count = app.stats["package_count"].as_u64().unwrap_or(0);
    // Build feed: chain summary lines + live SSE events
    let mut feed: Vec<ListItem> = Vec::new();
    feed.push(
        ListItem::new(format!(
            "[chain]  height={} packages={} blocks={}",
            height,
            pkg_count,
            app.stats["block_count"].as_u64().unwrap_or(0)
        ))
        .style(Style::default().fg(Color::Cyan)),
    );
    for b in app.blocks.iter().take(5) {
        let h = b["header"]["height"].as_u64().unwrap_or(0);
        let txs = b["transactions"].as_array().map(|v| v.len()).unwrap_or(0);
        let tx_kinds: Vec<&str> = b["transactions"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|t| t["type"].as_str()).collect())
            .unwrap_or_default();
        let summary = if tx_kinds.is_empty() {
            "empty".into()
        } else {
            tx_kinds.join(", ")
        };
        feed.push(
            ListItem::new(format!("[block#{}] {} tx — {}", h, txs, summary))
                .style(Style::default().fg(Color::DarkGray)),
        );
    }
    for e in app.events.iter() {
        feed.push(ListItem::new(e.clone()).style(Style::default().fg(Color::Green)));
    }
    let event_list = List::new(feed)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" LIVE FEED (Press 'q' to quit) "),
        )
        .style(Style::default().fg(Color::Gray));
    f.render_widget(event_list, chunks[2]);
}
