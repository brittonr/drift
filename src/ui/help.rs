use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use super::keybindings::KEYBINDING_CATEGORIES;

pub struct HelpPanelState {
    pub scroll_offset: usize,
}

pub fn render_help_panel(f: &mut Frame, state: &HelpPanelState, area: Rect) {
    // Calculate centered overlay area (80% width, 90% height)
    let popup_width = (area.width as f32 * 0.80) as u16;
    let popup_height = (area.height as f32 * 0.90) as u16;
    let popup_x = area.x + (area.width - popup_width) / 2;
    let popup_y = area.y + (area.height - popup_height) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // Clear the background
    f.render_widget(Clear, popup_area);

    // Build content lines
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        "Keybindings",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for category in KEYBINDING_CATEGORIES {
        lines.push(Line::from(Span::styled(
            category.name,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));

        for binding in category.bindings {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:16}", binding.keys),
                    Style::default().fg(Color::Green),
                ),
                Span::raw(binding.description),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Apply scroll offset
    let visible_lines: Vec<Line> = lines.into_iter().skip(state.scroll_offset).collect();

    let help_paragraph = Paragraph::new(visible_lines)
        .block(
            Block::default()
                .title(" Help - Press any key to close (j/k to scroll) ")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(help_paragraph, popup_area);
}
