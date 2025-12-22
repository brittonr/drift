use chrono::{DateTime, Utc};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::history_db::HistoryEntry;
use crate::tidal::{Album, Artist, TidalClient, Track};

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum LibraryTab {
    Tracks,
    Albums,
    Artists,
    History,
}

pub struct LibraryViewState<'a> {
    pub library_tab: LibraryTab,
    pub favorite_tracks: &'a [Track],
    pub favorite_albums: &'a [Album],
    pub favorite_artists: &'a [Artist],
    pub history_entries: &'a [HistoryEntry],
    pub selected_favorite_track: usize,
    pub selected_favorite_album: usize,
    pub selected_favorite_artist: usize,
    pub selected_history_entry: usize,
}

fn format_time_ago(played_at: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(played_at);

    if duration.num_minutes() < 1 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{}m ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_days() < 7 {
        format!("{}d ago", duration.num_days())
    } else {
        played_at.format("%Y-%m-%d").to_string()
    }
}

pub fn render_library_view(
    f: &mut Frame,
    state: &LibraryViewState,
    area: Rect,
) -> Rect {
    let library_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)].as_ref())
        .split(area);

    // Tab bar
    let tabs = vec![
        Span::styled(
            " Tracks ",
            if state.library_tab == LibraryTab::Tracks {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        ),
        Span::raw(" | "),
        Span::styled(
            " Albums ",
            if state.library_tab == LibraryTab::Albums {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        ),
        Span::raw(" | "),
        Span::styled(
            " Artists ",
            if state.library_tab == LibraryTab::Artists {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        ),
        Span::raw(" | "),
        Span::styled(
            " History ",
            if state.library_tab == LibraryTab::History {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        ),
    ];

    let tab_line = Paragraph::new(Line::from(tabs))
        .block(
            Block::default()
                .title("Library [Tab: switch | r: refresh | f: unfavorite | b: back]")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .alignment(Alignment::Center);
    f.render_widget(tab_line, library_chunks[0]);

    let content_area = library_chunks[1];

    // Content based on selected tab
    match state.library_tab {
        LibraryTab::Tracks => {
            let items: Vec<ListItem> = state
                .favorite_tracks
                .iter()
                .map(|track| {
                    let display = TidalClient::format_track_display(track);
                    ListItem::new(display)
                })
                .collect();

            let count = state.favorite_tracks.len();
            let list = List::new(items)
                .block(
                    Block::default()
                        .title(format!("Favorite Tracks ({}) [p: play | y: queue]", count))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .highlight_symbol("> ");
            f.render_stateful_widget(
                list,
                content_area,
                &mut ListState::default().with_selected(Some(state.selected_favorite_track)),
            );
        }
        LibraryTab::Albums => {
            let items: Vec<ListItem> = state
                .favorite_albums
                .iter()
                .map(|album| {
                    let display = format!("{} - {} ({} tracks)", album.artist, album.title, album.num_tracks);
                    ListItem::new(display)
                })
                .collect();

            let count = state.favorite_albums.len();
            let list = List::new(items)
                .block(
                    Block::default()
                        .title(format!("Favorite Albums ({}) [Enter: add to queue]", count))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .highlight_symbol("> ");
            f.render_stateful_widget(
                list,
                content_area,
                &mut ListState::default().with_selected(Some(state.selected_favorite_album)),
            );
        }
        LibraryTab::Artists => {
            let items: Vec<ListItem> = state
                .favorite_artists
                .iter()
                .map(|artist| {
                    ListItem::new(artist.name.clone())
                })
                .collect();

            let count = state.favorite_artists.len();
            let list = List::new(items)
                .block(
                    Block::default()
                        .title(format!("Favorite Artists ({}) [Enter: add top tracks]", count))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .highlight_symbol("> ");
            f.render_stateful_widget(
                list,
                content_area,
                &mut ListState::default().with_selected(Some(state.selected_favorite_artist)),
            );
        }
        LibraryTab::History => {
            let items: Vec<ListItem> = state
                .history_entries
                .iter()
                .map(|entry| {
                    let time_ago = format_time_ago(entry.played_at);
                    let display = format!("{} - {} [{}]", entry.artist, entry.title, time_ago);
                    ListItem::new(display)
                })
                .collect();

            let count = state.history_entries.len();
            let list = List::new(items)
                .block(
                    Block::default()
                        .title(format!("Playback History ({}) [p: play | y: queue | f: favorite]", count))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .highlight_symbol("> ");
            f.render_stateful_widget(
                list,
                content_area,
                &mut ListState::default().with_selected(Some(state.selected_history_entry)),
            );
        }
    }

    content_area
}
