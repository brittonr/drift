use std::collections::HashSet;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

use crate::service::{Playlist, Track};
use super::styles::{format_track_with_indicator, is_track_playing, service_badge};
use super::theme::Theme;

pub struct BrowseViewState<'a> {
    pub playlists: &'a [Playlist],
    pub tracks: &'a [Track],
    pub selected_playlist: usize,
    pub selected_track: usize,
    pub selected_tab: usize,
    pub synced_playlist_ids: HashSet<String>,
    pub current_track_id: Option<&'a str>,
}

pub fn render_browse_view(
    f: &mut Frame,
    state: &BrowseViewState,
    area: Rect,
    theme: &Theme,
) -> (Rect, Rect) {
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)].as_ref())
        .split(area);

    let left_area = content_chunks[0];
    let right_area = content_chunks[1];

    // Left panel - Playlists
    let playlists: Vec<ListItem> = state
        .playlists
        .iter()
        .map(|playlist| {
            let mut display = format!("{} ({} tracks)", playlist.title, playlist.num_tracks);
            if state.synced_playlist_ids.contains(&playlist.id) {
                display = format!("[S] {}", display);
            }
            ListItem::new(display)
        })
        .collect();

    let playlists_widget = List::new(playlists)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Playlists [h/l: switch | Enter: load | S: sync]")
                .border_style(if state.selected_tab == 0 {
                    Style::default().fg(theme.warning())
                } else {
                    Style::default().fg(theme.border_normal())
                }),
        )
        .highlight_style(theme.highlight_style())
        .highlight_symbol("> ");

    let selected_playlist = if state.selected_tab == 0 {
        Some(state.selected_playlist)
    } else {
        None
    };
    f.render_stateful_widget(
        playlists_widget,
        left_area,
        &mut ListState::default().with_selected(selected_playlist),
    );

    // Right panel - Tracks
    let tracks: Vec<ListItem> = state
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let is_selected = state.selected_tab == 1 && i == state.selected_track;
            let is_playing = is_track_playing(&track.id, state.current_track_id);
            let style = theme.track_style(is_selected, is_playing);

            let display = format!(
                "{} {} - {} ({}:{:02})",
                service_badge(track.service),
                track.artist,
                track.title,
                track.duration_seconds / 60,
                track.duration_seconds % 60
            );
            let display = format_track_with_indicator(display, is_playing);
            ListItem::new(display).style(style)
        })
        .collect();

    let tracks_widget = List::new(tracks)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Tracks [p/Enter: play | y: add to queue]")
                .border_style(if state.selected_tab == 1 {
                    Style::default().fg(theme.warning())
                } else {
                    Style::default().fg(theme.border_normal())
                }),
        )
        .highlight_style(theme.highlight_style())
        .highlight_symbol("> ");

    let selected_track = if state.selected_tab == 1 {
        Some(state.selected_track)
    } else {
        None
    };
    f.render_stateful_widget(
        tracks_widget,
        right_area,
        &mut ListState::default().with_selected(selected_track),
    );

    (left_area, right_area)
}
