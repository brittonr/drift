use std::collections::HashSet;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

use crate::tidal::{Playlist, TidalClient, Track};

pub struct BrowseViewState<'a> {
    pub playlists: &'a [Playlist],
    pub tracks: &'a [Track],
    pub selected_playlist: usize,
    pub selected_track: usize,
    pub selected_tab: usize,
    pub synced_playlist_ids: HashSet<String>,
}

pub fn render_browse_view(
    f: &mut Frame,
    state: &BrowseViewState,
    area: Rect,
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
            let mut display = TidalClient::format_playlist_display(playlist);
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
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        )
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
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
        .map(|track| {
            let display = TidalClient::format_track_display(track);
            ListItem::new(display)
        })
        .collect();

    let tracks_widget = List::new(tracks)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Tracks [p/Enter: play | y: add to queue]")
                .border_style(if state.selected_tab == 1 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        )
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
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
