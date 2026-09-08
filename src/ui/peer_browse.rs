//! Peer playlists browse view.
//!
//! Three-panel layout: Peers | Playlists | Tracks
//! Activated when browse.selected_tab == 2.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

use super::styles::{format_track_with_indicator, is_track_playing, service_badge};
use super::theme::Theme;
use crate::service::Track;
use drift_plugin::playlist::PlaylistIndexEntry;

pub struct PeerBrowseViewState<'a> {
    pub peer_names: &'a [String],
    pub playlists: &'a [PlaylistIndexEntry],
    pub tracks: &'a [Track],
    pub selected_peer: usize,
    pub selected_playlist: usize,
    pub selected_track: usize,
    pub active_panel: usize,
    pub current_track_id: Option<&'a str>,
}

pub fn render_peer_browse_view(
    f: &mut Frame,
    state: &PeerBrowseViewState,
    area: Rect,
    theme: &Theme,
) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage(20),
                Constraint::Percentage(30),
                Constraint::Percentage(50),
            ],
        )
        .split(area);

    let peers_area = chunks[0];
    let playlists_area = chunks[1];
    let tracks_area = chunks[2];

    // ── Panel 0: Peers list ──────────────────────────────────────
    let peers: Vec<ListItem> = if state.peer_names.is_empty() {
        vec![ListItem::new("  No peers configured")]
    } else {
        state
            .peer_names
            .iter()
            .map(|name| ListItem::new(format!("  {}", name)))
            .collect()
    };

    let peers_widget = List::new(peers)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Peers [h/l: panels]")
                .border_style(if state.active_panel == 0 {
                    Style::default().fg(theme.warning())
                } else {
                    Style::default().fg(theme.border_normal())
                }),
        )
        .highlight_style(theme.highlight_style())
        .highlight_symbol("> ");

    let selected_peer = if !state.peer_names.is_empty() {
        Some(state.selected_peer)
    } else {
        None
    };
    f.render_stateful_widget(
        peers_widget,
        peers_area,
        &mut ListState::default().with_selected(selected_peer),
    );

    // ── Panel 1: Peer's playlists ────────────────────────────────
    let playlists: Vec<ListItem> = if state.playlists.is_empty() {
        if state.peer_names.is_empty() {
            vec![ListItem::new("  Add peers in config")]
        } else {
            vec![ListItem::new("  No shared playlists")]
        }
    } else {
        state
            .playlists
            .iter()
            .map(|pl| ListItem::new(format!("  {} ({} tracks)", pl.title, pl.track_count)))
            .collect()
    };

    let playlists_widget = List::new(playlists)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Playlists")
                .border_style(if state.active_panel == 1 {
                    Style::default().fg(theme.warning())
                } else {
                    Style::default().fg(theme.border_normal())
                }),
        )
        .highlight_style(theme.highlight_style())
        .highlight_symbol("> ");

    let selected_playlist = if !state.playlists.is_empty() {
        Some(state.selected_playlist)
    } else {
        None
    };
    f.render_stateful_widget(
        playlists_widget,
        playlists_area,
        &mut ListState::default().with_selected(selected_playlist),
    );

    // ── Panel 2: Tracks ──────────────────────────────────────────
    let tracks: Vec<ListItem> = if state.tracks.is_empty() {
        if state.playlists.is_empty() {
            vec![ListItem::new("  Select a playlist")]
        } else {
            vec![ListItem::new("  No tracks")]
        }
    } else {
        state
            .tracks
            .iter()
            .enumerate()
            .map(|(i, track)| {
                let is_selected = state.active_panel == 2 && i == state.selected_track;
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
            .collect()
    };

    let tracks_widget = List::new(tracks)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Tracks [p: play | y: queue]")
                .border_style(if state.active_panel == 2 {
                    Style::default().fg(theme.warning())
                } else {
                    Style::default().fg(theme.border_normal())
                }),
        )
        .highlight_style(theme.highlight_style())
        .highlight_symbol("> ");

    let selected_track = if state.active_panel == 2 && !state.tracks.is_empty() {
        Some(state.selected_track)
    } else {
        None
    };
    f.render_stateful_widget(
        tracks_widget,
        tracks_area,
        &mut ListState::default().with_selected(selected_track),
    );

    (peers_area, tracks_area)
}
