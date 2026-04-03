// crates/cli/src/dashboard_interactive.rs
// Interactive TUI dashboard with clickable actions

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Row, Table, Tabs, Wrap},
    Frame, Terminal,
};
use std::{io, time::Duration};

pub enum MenuItem {
    Overview,
    Packages,
    Validator,
    Settings,
}

pub struct DashboardApp {
    pub active_tab: usize,
    pub menu_items: Vec<(MenuItem, String)>,
    pub show_help: bool,
    pub show_stake_dialog: bool,
    pub show_publish_dialog: bool,
    pub quit: bool,
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

    let mut last_tick = std::time::Instant::now();
    let tick_rate = Duration::from_millis(250);

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

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
            // Refresh data here
            last_tick = std::time::Instant::now();
        }

        if app.quit {
            break;
        }
    }

    disable_raw_mode()?;
    Ok(())
}

fn ui(f: &mut Frame, app: &mut DashboardApp) {
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

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Chain Registry Dashboard"),
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
        MenuItem::Overview => draw_overview_tab(f, chunks[1]),
        MenuItem::Packages => draw_packages_tab(f, chunks[1]),
        MenuItem::Validator => draw_validator_tab(f, chunks[1]),
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

fn draw_overview_tab(f: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left: Stats
    let stats_text = vec![
        Line::from(vec![
            Span::raw("Blocks: "),
            Span::styled(
                "15,234",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("Packages: "),
            Span::styled("1,892", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Validators: "),
            Span::styled("10/10", Style::default().fg(Color::Green)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("Network Status: "),
            Span::styled("✓ Healthy", Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::raw("Sync: "),
            Span::styled("100%", Style::default().fg(Color::Green)),
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

    // Right: Recent Activity
    let activities = vec![
        Line::from("• express@4.18.2 verified (2s ago)"),
        Line::from("• Block #15234 proposed (5s ago)"),
        Line::from("• lodash@4.17.21 verified (12s ago)"),
        Line::from("• Validator node-7 voted (15s ago)"),
    ];

    let activity = Paragraph::new(activities).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Recent Activity"),
    );
    f.render_widget(activity, chunks[1]);
}

fn draw_packages_tab(f: &mut Frame, area: Rect) {
    let items: Vec<ListItem> = vec![
        ListItem::new("✓ express@4.18.2    Verified    3/3 votes"),
        ListItem::new("✓ lodash@4.17.21    Verified    3/3 votes"),
        ListItem::new("⚠ axios@1.4.0       Pending     1/3 votes"),
        ListItem::new("✗ malicious-pkg     Rejected    0/3 votes"),
    ];

    let packages = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Recent Packages"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    f.render_widget(packages, area);
}

fn draw_validator_tab(f: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(0)])
        .split(area);

    // Validator status
    let status_text = vec![
        Line::from(vec![
            Span::raw("Status: "),
            Span::styled(
                "🟢 Active",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("Stake: "),
            Span::styled("50 CREG", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Rewards: "),
            Span::styled("12.5 CREG", Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::raw("Reputation: "),
            Span::styled("98/100 ⭐⭐⭐⭐⭐", Style::default().fg(Color::Yellow)),
        ]),
    ];

    let status = Paragraph::new(status_text)
        .block(Block::default().borders(Borders::ALL).title("My Validator"));
    f.render_widget(status, chunks[0]);

    // Performance table
    let rows = vec![
        Row::new(vec!["Packages Validated", "1,234"]),
        Row::new(vec!["Correct Votes", "1,230 (99.7%)"]),
        Row::new(vec!["Block Proposals", "45"]),
        Row::new(vec!["Uptime", "99.9%"]),
    ];

    let table = Table::new(
        rows,
        vec![Constraint::Percentage(50), Constraint::Percentage(50)],
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Performance (30 days)"),
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

    let dialog = Paragraph::new("Stake Dialog\n\nPress ESC to close")
        .block(Block::default().borders(Borders::ALL).title("Stake CREG"));

    f.render_widget(Clear, area);
    f.render_widget(dialog, area);
}

fn draw_publish_dialog(f: &mut Frame) {
    let area = centered_rect(60, 40, f.size());

    let dialog = Paragraph::new("Publish Dialog\n\nPress ESC to close").block(
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
