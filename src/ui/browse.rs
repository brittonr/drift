use std::collections::HashSet;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem},
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
        .enumerate()
        .map(|(i, playlist)| {
            let style = if i == state.selected_playlist && state.selected_tab == 0 {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let mut display = TidalClient::format_playlist_display(playlist);
            if state.synced_playlist_ids.contains(&playlist.id) {
                display = format!("[S] {}", display);
            }
            ListItem::new(display).style(style)
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
        );
    f.render_widget(playlists_widget, left_area);

    // Right panel - Tracks
    let tracks: Vec<ListItem> = state
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let style = if i == state.selected_track && state.selected_tab == 1 {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let display = TidalClient::format_track_display(track);
            ListItem::new(display).style(style)
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
        );
    f.render_widget(tracks_widget, right_area);

    (left_area, right_area)
}
