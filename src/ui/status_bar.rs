use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use super::theme::Theme;

pub struct StatusBarState {
    pub is_searching: bool,
    pub space_pressed: bool,
    pub pending_key: Option<char>,
    pub status_message: Option<(String, bool)>, // (message, is_error)
}

pub fn render_status_bar(
    f: &mut Frame,
    state: &StatusBarState,
    area: ratatui::layout::Rect,
    theme: &Theme,
) {
    let status_bar = if let Some((ref msg, is_error)) = state.status_message {
        let color = if is_error { theme.error() } else { theme.warning() };
        Paragraph::new(Line::from(vec![
            Span::styled(
                if is_error { "ERROR" } else { "INFO" },
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(": "),
            Span::styled(msg.as_str(), Style::default().fg(color)),
        ]))
    } else if state.is_searching {
        Paragraph::new(Line::from(vec![
            Span::styled(
                "INSERT MODE",
                Style::default().fg(theme.warning()).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" | "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": search | "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": cancel"),
        ]))
    } else if state.space_pressed {
        Paragraph::new(Line::from(vec![
            Span::styled(
                "SPACE",
                Style::default().fg(theme.primary()).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" + "),
            Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": quit | "),
            Span::styled("p", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": pause | "),
            Span::styled("n", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": next | "),
            Span::styled("b", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": prev | "),
            Span::styled("v", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": visualizer | "),
            Span::styled("c", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": clear log | "),
            Span::styled("e", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": export"),
        ]))
    } else if state.pending_key == Some('g') {
        Paragraph::new(Line::from(vec![
            Span::styled(
                "g",
                Style::default().fg(theme.primary()).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" + "),
            Span::styled("g", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": top | "),
            Span::styled("e", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": end"),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled(
                "NORMAL",
                Style::default().fg(theme.success()).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" | "),
            Span::styled("hjkl", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": move | "),
            Span::styled("+/-", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": vol | "),
            Span::styled("</>", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": seek | "),
            Span::styled("r", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": repeat | "),
            Span::styled("s", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": shuffle | "),
            Span::styled("Space", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": cmd | "),
            Span::styled("?", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": help"),
        ]))
    };

    let status_bar = status_bar.block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded),
    );
    f.render_widget(status_bar, area);
}
