use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

use crate::service::{Album, Track};
use super::styles::{format_track_with_indicator, is_track_playing};
use super::theme::Theme;

pub struct AlbumDetailViewState<'a> {
    pub album: Option<&'a Album>,
    pub tracks: &'a [Track],
    pub selected_track: usize,
    pub current_track_id: Option<&'a str>,
}

pub fn render_album_detail_view(f: &mut Frame, state: &AlbumDetailViewState, area: Rect, theme: &Theme) -> Rect {
    let album_info = state
        .album
        .map(|a| format!("{} - {}", a.artist, a.title))
        .unwrap_or_else(|| "Unknown Album".to_string());

    let track_items: Vec<ListItem> = state
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let is_selected = i == state.selected_track;
            let is_playing = is_track_playing(&track.id, state.current_track_id);
            let style = theme.track_style(is_selected, is_playing);

            // Show track number
            let display = format!(
                "{}. {} ({}:{:02})",
                i + 1,
                track.title,
                track.duration_seconds / 60,
                track.duration_seconds % 60
            );
            let display = format_track_with_indicator(display, is_playing);
            ListItem::new(display).style(style)
        })
        .collect();

    let title = format!(
        "{} ({} tracks) [p: play | y: queue | Y: queue all | Esc: back]",
        album_info,
        state.tracks.len()
    );
    let list = List::new(track_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(theme.primary())),
        )
        .highlight_style(theme.highlight_style())
        .highlight_symbol("> ");
    f.render_stateful_widget(
        list,
        area,
        &mut ListState::default().with_selected(Some(state.selected_track)),
    );

    area
}
