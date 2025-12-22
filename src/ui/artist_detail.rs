use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::tidal::{Album, Artist, TidalClient, Track};

pub struct ArtistDetailViewState<'a> {
    pub artist: Option<&'a Artist>,
    pub top_tracks: &'a [Track],
    pub albums: &'a [Album],
    pub selected_track: usize,
    pub selected_album: usize,
    pub selected_panel: usize, // 0 = top tracks, 1 = albums
}

pub fn render_artist_detail_view(
    f: &mut Frame,
    state: &ArtistDetailViewState,
    area: Rect,
) -> (Rect, Rect) {
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let left_area = content_chunks[0];
    let right_area = content_chunks[1];

    let artist_name = state
        .artist
        .map(|a| a.name.as_str())
        .unwrap_or("Unknown Artist");

    // Left panel - Top Tracks
    let track_items: Vec<ListItem> = state
        .top_tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let style = if i == state.selected_track && state.selected_panel == 0 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(TidalClient::format_track_display(track)).style(style)
        })
        .collect();

    let tracks_title = format!(
        "{} - Top Tracks ({}) [p: play | y: queue]",
        artist_name,
        state.top_tracks.len()
    );
    let tracks_widget = List::new(track_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(tracks_title)
            .border_style(if state.selected_panel == 0 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            }),
    );
    f.render_widget(tracks_widget, left_area);

    // Right panel - Albums/Discography
    let album_items: Vec<ListItem> = state
        .albums
        .iter()
        .enumerate()
        .map(|(i, album)| {
            let style = if i == state.selected_album && state.selected_panel == 1 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let display = format!("{} ({} tracks)", album.title, album.num_tracks);
            ListItem::new(display).style(style)
        })
        .collect();

    let albums_title = format!(
        "Discography ({}) [v: view | y: queue]",
        state.albums.len()
    );
    let albums_widget = List::new(album_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(albums_title)
            .border_style(if state.selected_panel == 1 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            }),
    );
    f.render_widget(albums_widget, right_area);

    (left_area, right_area)
}
