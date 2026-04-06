// crates/cli/src/dashboard_interactive.rs
// Interactive TUI dashboard with live data from node API

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Row, Table, Tabs, Wrap},
    Frame, Terminal,
};
use serde_json::Value;
use std::{io, time::{Duration, Instant}};

const REFRESH_SECS: u64 = 3;

pub enum MenuItem {
    Overview,
    Packages,
    Validator,
    Settings,
}

#[derive(Default)]
struct LiveData {
    tip_height: u64,
    package_count: u64,
    validator_count: usize,
    total_stake: u64,
    peer_count: usize,
    bridge_status: String,
    validators: Vec<ValidatorEntry>,
    pending_packages: Vec<String>,
    pending_count: u64,
    connected: bool,
    last_error: Option<String>,
}

struct ValidatorEntry {
    id: String,
    alias: String,
    stake: u64,
    reputation: u64,
    status: String,
}

pub struct DashboardApp {
    pub active_tab: usize,
    pub menu_items: Vec<(MenuItem, String)>,
    pub show_help: bool,
    pub show_stake_dialog: bool,
    pub show_publish_dialog: bool,
    pub quit: bool,
    data: LiveData,
}

impl DashboardApp {
    pub fn new() -> Self {
        Self {
            active_tab: 0,
            menu_items: vec![
                (MenuItem::Overview, "Overview".to_string()),
                (MenuItem::Packages, "Packages".to_string()),
                (MenuItem::Validator, "Validator".to_string()),
                (MenuItem::Settings, "Settings".to_string()),
            ],
            show_help: false,
            show_stake_dialog: false,
            show_publish_dialog: false,
            quit: false,
            data: LiveData::default(),
        }
    }

    pub fn on_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.quit = true,
            KeyCode::Char('1') => self.active_tab = 0,
            KeyCode::Char('2') => self.active_tab = 1,
            KeyCode::Char('3') => self.active_tab = 2,
            KeyCode::Char('4') => self.active_tab = 3,
            KeyCode::Right => self.next_tab(),
            KeyCode::Left => self.previous_tab(),
            KeyCode::Char('s') => self.show_stake_dialog = true,
            KeyCode::Char('p') => self.show_publish_dialog = true,
            KeyCode::Char('h') => self.show_help = !self.show_help,
            KeyCode::Esc => {
                self.show_stake_dialog = false;
                self.show_publish_dialog = false;
                self.show_help = false;
            }
            _ => {}
        }
    }

    fn next_tab(&mut self) {
        self.active_tab = (self.active_tab + 1) % self.menu_items.len();
    }

    fn previous_tab(&mut self) {
        if self.active_tab == 0 {
            self.active_tab = self.menu_items.len() - 1;
        } else {
            self.active_tab -= 1;
        }
    }
}

pub async fn run(node_url: Option<&str>) -> Result<()> {
    enable_raw_mode()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = DashboardApp::new();
    let api_base = node_url.map(String::from).unwrap_or_else(|| {
        std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let mut last_tick = Instant::now();
    let mut last_fetch = Instant::now() - Duration::from_secs(REFRESH_SECS + 1);
    let tick_rate = Duration::from_millis(250);

    loop {
        // Fetch live data periodically
        if last_fetch.elapsed() >= Duration::from_secs(REFRESH_SECS) {
            fetch_live_data(&client, &api_base, &mut app.data).await;
            last_fetch = Instant::now();
        }

        terminal.draw(|f| ui(f, &app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.on_key(key.code);
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }

        if app.quit {
            break;
        }
    }

    disable_raw_mode()?;
    Ok(())
}

async fn fetch_live_data(client: &reqwest::Client, api_base: &str, data: &mut LiveData) {
    // Fetch chain stats
    match client.get(format!("{}/v1/chain/stats", api_base)).send().await {
        Ok(res) => match res.json::<Value>().await {
            Ok(json) => {
                data.tip_height = json["tip_height"].as_u64().unwrap_or(0);
                data.package_count = json["package_count"].as_u64().unwrap_or(0);
                data.validator_count = json["validator_count"].as_u64().unwrap_or(0) as usize;
                data.total_stake = json["total_stake"].as_u64().unwrap_or(0);
                data.peer_count = json["peer_count"].as_u64().unwrap_or(0) as usize;
                data.bridge_status = json["bridge_status"].as_str().unwrap_or("Unknown").to_string();
                data.connected = true;
                data.last_error = None;
            }
            Err(e) => { data.last_error = Some(format!("Parse error: {}", e)); }
        },
        Err(e) => {
            data.connected = false;
            data.last_error = Some(format!("Connection error: {}", e));
        }
    }

    // Fetch validators
    if let Ok(res) = client.get(format!("{}/v1/nodes", api_base)).send().await {
        if let Ok(json) = res.json::<Vec<Value>>().await {
            data.validators = json.iter().map(|v| ValidatorEntry {
                id: v["id"].as_str().unwrap_or("?").to_string(),
                alias: v["alias"].as_str().unwrap_or("").to_string(),
                stake: v["stake"].as_u64().unwrap_or(0),
                reputation: v["reputation"].as_u64().unwrap_or(0),
                status: v["status"].as_str().unwrap_or("unknown").to_string(),
            }).collect();
        }
    }

    // Fetch pending packages
    if let Ok(res) = client.get(format!("{}/v1/pending", api_base)).send().await {
        if let Ok(json) = res.json::<Value>().await {
            data.pending_count = json["count"].as_u64().unwrap_or(0);
            data.pending_packages = json["packages"].as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
        }
    }
}

fn ui(f: &mut Frame, app: &DashboardApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Main content
            Constraint::Length(3), // Footer
        ])
        .split(f.size());

    // Header with tabs
    let titles: Vec<Line> = app
        .menu_items
        .iter()
        .map(|(_, title)| Line::from(title.as_str()))
        .collect();

    let conn_indicator = if app.data.connected { "● CONNECTED" } else { "○ DISCONNECTED" };
    let header_title = format!("Chain Registry Dashboard  [{}]", conn_indicator);

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(header_title),
        )
        .select(app.active_tab)
        .style(Style::default().fg(Color::White))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .divider(" | ");

    f.render_widget(tabs, chunks[0]);

    // Main content based on tab
    match app.menu_items[app.active_tab].0 {
        MenuItem::Overview => draw_overview_tab(f, &app.data, chunks[1]),
        MenuItem::Packages => draw_packages_tab(f, &app.data, chunks[1]),
        MenuItem::Validator => draw_validator_tab(f, &app.data, chunks[1]),
        MenuItem::Settings => draw_settings_tab(f, chunks[1]),
    }

    // Footer with shortcuts
    let footer_text = "[Q]uit | [1-4] Tabs | [S]take | [P]ublish | [H]elp";
    let footer = Paragraph::new(footer_text)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Gray));
    f.render_widget(footer, chunks[2]);

    // Dialogs
    if app.show_stake_dialog {
        draw_stake_dialog(f);
    }
    if app.show_publish_dialog {
        draw_publish_dialog(f);
    }
    if app.show_help {
        draw_help_dialog(f);
    }
}

fn draw_overview_tab(f: &mut Frame, data: &LiveData, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let active = data.validators.iter().filter(|v| v.status == "online" || v.status == "self").count();
    let total = data.validators.len();

    let stats_text = vec![
        Line::from(vec![
            Span::raw("Block Height: "),
            Span::styled(
                format!("{}", data.tip_height),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("Packages: "),
            Span::styled(format!("{}", data.package_count), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Validators: "),
            Span::styled(
                format!("{}/{}", active, total),
                Style::default().fg(if active == total && total > 0 { Color::Green } else { Color::Yellow }),
            ),
        ]),
        Line::from(vec![
            Span::raw("Total Stake: "),
            Span::styled(format!("{} CREG", data.total_stake), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Peers: "),
            Span::styled(format!("{}", data.peer_count), Style::default().fg(Color::Green)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("Bridge: "),
            Span::styled(&data.bridge_status, Style::default().fg(Color::Yellow)),
        ]),
    ];

    let stats = Paragraph::new(stats_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Network Stats"),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(stats, chunks[0]);

    // Right: Validator list
    let val_items: Vec<ListItem> = data.validators.iter().map(|v| {
        let icon = match v.status.as_str() {
            "online" | "self" => "🟢",
            _ => "🔴",
        };
        ListItem::new(format!("{} {} ({})  {} CREG  rep:{}", icon, v.id, v.alias, v.stake, v.reputation))
    }).collect();

    let val_list = List::new(if val_items.is_empty() {
        vec![ListItem::new("Waiting for validator data...")]
    } else {
        val_items
    }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Validators ({})", data.validators.len())),
    );
    f.render_widget(val_list, chunks[1]);
}

fn draw_packages_tab(f: &mut Frame, data: &LiveData, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let summary = Paragraph::new(format!(
        "On-chain: {}  |  Pending: {}",
        data.package_count, data.pending_count
    ))
    .block(Block::default().borders(Borders::ALL).title("Package Summary"))
    .style(Style::default().fg(Color::Cyan));
    f.render_widget(summary, chunks[0]);

    let items: Vec<ListItem> = if data.pending_packages.is_empty() {
        vec![ListItem::new("  No pending packages").style(Style::default().fg(Color::DarkGray))]
    } else {
        data.pending_packages.iter().map(|name| {
            ListItem::new(format!("  ⏳ {}", name)).style(Style::default().fg(Color::Yellow))
        }).collect()
    };

    let packages = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Pending Packages"),
        );
    f.render_widget(packages, chunks[1]);
}

fn draw_validator_tab(f: &mut Frame, data: &LiveData, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(0)])
        .split(area);

    let active = data.validators.iter().filter(|v| v.status == "online" || v.status == "self").count();
    let avg_rep = if data.validators.is_empty() { 0 } else {
        data.validators.iter().map(|v| v.reputation).sum::<u64>() / data.validators.len() as u64
    };

    let status_text = vec![
        Line::from(vec![
            Span::raw("Active Validators: "),
            Span::styled(
                format!("{}/{}", active, data.validators.len()),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("Average Reputation: "),
            Span::styled(format!("{}/100", avg_rep), Style::default().fg(Color::Yellow)),
        ]),
    ];

    let status = Paragraph::new(status_text)
        .block(Block::default().borders(Borders::ALL).title("Validator Network"));
    f.render_widget(status, chunks[0]);

    let rows: Vec<Row> = data.validators.iter().map(|v| {
        let status_color = match v.status.as_str() {
            "online" | "self" => Color::Green,
            _ => Color::Red,
        };
        Row::new(vec![
            v.id.clone(),
            v.alias.clone(),
            format!("{} CREG", v.stake),
            format!("{}/100", v.reputation),
            v.status.clone(),
        ]).style(Style::default().fg(status_color))
    }).collect();

    let table = Table::new(
        rows,
        vec![
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
    .header(
        Row::new(vec!["ID", "Alias", "Stake", "Reputation", "Status"])
            .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("All Validators"),
    )
    .column_spacing(1);

    f.render_widget(table, chunks[1]);
}

fn draw_settings_tab(f: &mut Frame, area: Rect) {
    let settings = Paragraph::new("Settings panel - WIP")
        .block(Block::default().borders(Borders::ALL).title("Settings"));
    f.render_widget(settings, area);
}

fn draw_stake_dialog(f: &mut Frame) {
    let area = centered_rect(60, 40, f.size());

    let text = vec![
        Line::from(""),
        Line::from("  Use the CLI to stake tokens:"),
        Line::from(""),
        Line::from(Span::styled(
            "  $ creg stake --amount 100 --role publisher",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            "  $ creg stake --amount 100 --role validator",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from("  Or use the web explorer at http://localhost:3000"),
        Line::from(""),
        Line::from(Span::styled("  Press ESC to close", Style::default().fg(Color::DarkGray))),
    ];

    let dialog = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Stake CREG"));

    f.render_widget(Clear, area);
    f.render_widget(dialog, area);
}

fn draw_publish_dialog(f: &mut Frame) {
    let area = centered_rect(60, 40, f.size());

    let text = vec![
        Line::from(""),
        Line::from("  Use the CLI to publish a package:"),
        Line::from(""),
        Line::from(Span::styled(
            "  $ creg publish --name my-pkg --version 1.0.0 ./pkg.tar.gz",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from("  The package will appear in the Packages tab once submitted."),
        Line::from(""),
        Line::from(Span::styled("  Press ESC to close", Style::default().fg(Color::DarkGray))),
    ];

    let dialog = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Publish Package"),
    );

    f.render_widget(Clear, area);
    f.render_widget(dialog, area);
}

fn draw_help_dialog(f: &mut Frame) {
    let area = centered_rect(70, 80, f.size());

    let help_text = vec![
        Line::from("Keyboard Shortcuts:"),
        Line::from(""),
        Line::from("  q/Q    - Quit dashboard"),
        Line::from("  1-4    - Switch tabs"),
        Line::from("  ←/→    - Previous/Next tab"),
        Line::from("  s      - Open stake dialog"),
        Line::from("  p      - Open publish dialog"),
        Line::from("  h      - Toggle this help"),
        Line::from("  ESC    - Close dialogs"),
        Line::from(""),
        Line::from("Press ESC to close this help"),
    ];

    let dialog =
        Paragraph::new(help_text).block(Block::default().borders(Borders::ALL).title("Help"));

    f.render_widget(Clear, area);
    f.render_widget(dialog, area);
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
