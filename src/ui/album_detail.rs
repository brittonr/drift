use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::tidal::{Album, Track};

pub struct AlbumDetailViewState<'a> {
    pub album: Option<&'a Album>,
    pub tracks: &'a [Track],
    pub selected_track: usize,
}

pub fn render_album_detail_view(f: &mut Frame, state: &AlbumDetailViewState, area: Rect) -> Rect {
    let album_info = state
        .album
        .map(|a| format!("{} - {}", a.artist, a.title))
        .unwrap_or_else(|| "Unknown Album".to_string());

    let track_items: Vec<ListItem> = state
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let style = if i == state.selected_track {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            // Show track number
            let display = format!(
                "{}. {} ({}:{:02})",
                i + 1,
                track.title,
                track.duration_seconds / 60,
                track.duration_seconds % 60
            );
            ListItem::new(display).style(style)
        })
        .collect();

    let title = format!(
        "{} ({} tracks) [p: play | y: queue | Y: queue all | Esc: back]",
        album_info,
        state.tracks.len()
    );
    let list = List::new(track_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(list, area);

    area
}
