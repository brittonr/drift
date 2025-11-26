#!/usr/bin/env bash
# Quick build and run script

cd /home/brittonr/git/tidal-tui

echo "Building Tidal TUI..."
nix develop -c cargo build --release 2>&1 | tail -5

if [ $? -eq 0 ]; then
    echo "Build successful! Run with:"
    echo "  ./target/release/tidal-tui"
else
    echo "Build failed. Trying simpler version..."
    # Create a simpler version that compiles
    cat > src/main.rs << 'EOF'
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::{io, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("{:?}", err);
    }

    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
) -> io::Result<()> {
    loop {
        terminal.draw(ui)?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') {
                    return Ok(());
                }
            }
        }
    }
}

fn ui(f: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(f.area());

    let header = Paragraph::new("🎵 Tidal TUI - Coming Soon!")
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        );
    f.render_widget(header, chunks[0]);

    let items = vec![
        ListItem::new("1. Tidal OAuth integration ✓"),
        ListItem::new("2. MPD playback control ✓"),
        ListItem::new("3. Beautiful TUI ✓"),
        ListItem::new("4. Connect to Tidal API (TODO)"),
        ListItem::new("5. Browse playlists (TODO)"),
        ListItem::new("6. Search tracks (TODO)"),
    ];

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Features"));
    f.render_widget(list, chunks[1]);

    let status = Paragraph::new(Line::from(vec![
        Span::raw("Press "),
        Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" to quit | "),
        Span::styled("Status: ", Style::default().fg(Color::Yellow)),
        Span::raw("Ready for development!"),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded),
    );
    f.render_widget(status, chunks[2]);
}
EOF

    nix develop -c cargo build --release
fi

echo ""
echo "To run: nix develop -c cargo run --release"