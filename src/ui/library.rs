use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::tidal::{Album, Artist, TidalClient, Track};

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum LibraryTab {
    Tracks,
    Albums,
    Artists,
}

pub struct LibraryViewState<'a> {
    pub library_tab: LibraryTab,
    pub favorite_tracks: &'a [Track],
    pub favorite_albums: &'a [Album],
    pub favorite_artists: &'a [Artist],
    pub selected_favorite_track: usize,
    pub selected_favorite_album: usize,
    pub selected_favorite_artist: usize,
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
                .enumerate()
                .map(|(i, track)| {
                    let style = if i == state.selected_favorite_track {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let display = TidalClient::format_track_display(track);
                    ListItem::new(display).style(style)
                })
                .collect();

            let count = state.favorite_tracks.len();
            let list = List::new(items)
                .block(
                    Block::default()
                        .title(format!("Favorite Tracks ({}) [p: play | y: queue]", count))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                );
            f.render_widget(list, content_area);
        }
        LibraryTab::Albums => {
            let items: Vec<ListItem> = state
                .favorite_albums
                .iter()
                .enumerate()
                .map(|(i, album)| {
                    let style = if i == state.selected_favorite_album {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    let display = format!("{} - {} ({} tracks)", album.artist, album.title, album.num_tracks);
                    ListItem::new(display).style(style)
                })
                .collect();

            let count = state.favorite_albums.len();
            let list = List::new(items)
                .block(
                    Block::default()
                        .title(format!("Favorite Albums ({}) [Enter: add to queue]", count))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                );
            f.render_widget(list, content_area);
        }
        LibraryTab::Artists => {
            let items: Vec<ListItem> = state
                .favorite_artists
                .iter()
                .enumerate()
                .map(|(i, artist)| {
                    let style = if i == state.selected_favorite_artist {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    ListItem::new(artist.name.clone()).style(style)
                })
                .collect();

            let count = state.favorite_artists.len();
            let list = List::new(items)
                .block(
                    Block::default()
                        .title(format!("Favorite Artists ({}) [Enter: add top tracks]", count))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan)),
                );
            f.render_widget(list, content_area);
        }
    }

    content_area
}
