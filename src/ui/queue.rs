use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::tidal::Track;

pub fn render_queue(
    f: &mut Frame,
    local_queue: &[Track],
    selected_queue_item: usize,
    area: Rect,
) -> Rect {
    if local_queue.is_empty() {
        let empty_msg = Paragraph::new("Queue is empty\n\nPress 'y' to add selected track\nPress 'Y' to add all tracks")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title("Queue (0 tracks) [y: add | Y: add all | w: hide]")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan)),
            );
        f.render_widget(empty_msg, area);
        return area;
    }

    let mut items = vec![];

    for (i, track) in local_queue.iter().enumerate() {
        let style = if i == selected_queue_item {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let duration_str = format!("{}:{:02}", track.duration_seconds / 60, track.duration_seconds % 60);

        let content = format!(
            "{:2}. {} - {} [{}]",
            i + 1,
            track.artist,
            track.title,
            duration_str
        );

        items.push(ListItem::new(content).style(style));
    }

    let queue_list = List::new(items)
        .block(
            Block::default()
                .title(format!("Queue ({} tracks) [p/Enter: play | y: add | d: remove | D: clear]", local_queue.len()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    f.render_stateful_widget(
        queue_list,
        area,
        &mut ListState::default().with_selected(Some(selected_queue_item)),
    );

    area
}
