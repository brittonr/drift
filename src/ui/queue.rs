use ratatui::{
    layout::{Alignment, Rect},
    style::Style,
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::service::Track;
use super::styles::{format_track_with_indicator, is_track_playing};
use super::theme::Theme;

pub fn render_queue(
    f: &mut Frame,
    local_queue: &[Track],
    selected_queue_item: usize,
    current_track_id: Option<&str>,
    area: Rect,
    theme: &Theme,
) -> Rect {
    if local_queue.is_empty() {
        let empty_msg = Paragraph::new("Queue is empty\n\nPress 'y' to add selected track\nPress 'Y' to add all tracks")
            .style(Style::default().fg(theme.text_disabled()))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title("Queue (0 tracks) [y: add | Y: add all | w: hide]")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(theme.primary())),
            );
        f.render_widget(empty_msg, area);
        return area;
    }

    let mut items = vec![];

    for (i, track) in local_queue.iter().enumerate() {
        let is_selected = i == selected_queue_item;
        let is_playing = is_track_playing(&track.id, current_track_id);
        let style = theme.track_style(is_selected, is_playing);

        let duration_str = format!("{}:{:02}", track.duration_seconds / 60, track.duration_seconds % 60);

        let content = format!(
            "{:2}. {} - {} [{}]",
            i + 1,
            track.artist,
            track.title,
            duration_str
        );

        let display = format_track_with_indicator(content, is_playing);
        items.push(ListItem::new(display).style(style));
    }

    let queue_list = List::new(items)
        .block(
            Block::default()
                .title(format!("Queue ({} tracks) [p/Enter: play | y: add | d: remove | D: clear]", local_queue.len()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.primary())),
        )
        .highlight_style(theme.highlight_style())
        .highlight_symbol("> ");

    f.render_stateful_widget(
        queue_list,
        area,
        &mut ListState::default().with_selected(Some(selected_queue_item)),
    );

    area
}
