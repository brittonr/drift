use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::tidal::{SearchResults, TidalClient};

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SearchTab {
    Tracks,
    Albums,
    Artists,
}

pub struct SearchViewState<'a> {
    pub search_query: &'a str,
    pub search_results: Option<&'a SearchResults>,
    pub search_tab: SearchTab,
    pub selected_search_track: usize,
    pub selected_search_album: usize,
    pub selected_search_artist: usize,
    pub is_searching: bool,
}

pub fn render_search_view(
    f: &mut Frame,
    state: &SearchViewState,
    area: Rect,
) -> Rect {
    let search_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)].as_ref())
        .split(area);

    let results_area = search_chunks[1];

    // Search input box
    let search_input = Paragraph::new(state.search_query)
        .style(if state.is_searching {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if state.is_searching {
                    "Search (Enter to search, Esc to cancel)"
                } else {
                    "Search (/ to search again)"
                })
                .border_style(if state.is_searching {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        );
    f.render_widget(search_input, search_chunks[0]);

    // Search results
    if let Some(results) = state.search_results {
        match state.search_tab {
            SearchTab::Tracks => {
                let items: Vec<ListItem> = results
                    .tracks
                    .iter()
                    .map(|track| {
                        let display = TidalClient::format_track_display(track);
                        ListItem::new(display)
                    })
                    .collect();

                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!("Tracks ({}) [Tab: cycle results | p: play | y: add]", results.tracks.len()))
                            .border_style(Style::default().fg(Color::Cyan)),
                    )
                    .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                    .highlight_symbol("> ");
                f.render_stateful_widget(
                    list,
                    results_area,
                    &mut ListState::default().with_selected(Some(state.selected_search_track)),
                );
            }
            SearchTab::Albums => {
                let items: Vec<ListItem> = results
                    .albums
                    .iter()
                    .map(|album| {
                        let display = format!("{} - {} ({} tracks)", album.artist, album.title, album.num_tracks);
                        ListItem::new(display)
                    })
                    .collect();

                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!("Albums ({}) [Tab: cycle results]", results.albums.len()))
                            .border_style(Style::default().fg(Color::Magenta)),
                    )
                    .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                    .highlight_symbol("> ");
                f.render_stateful_widget(
                    list,
                    results_area,
                    &mut ListState::default().with_selected(Some(state.selected_search_album)),
                );
            }
            SearchTab::Artists => {
                let items: Vec<ListItem> = results
                    .artists
                    .iter()
                    .map(|artist| {
                        ListItem::new(artist.name.clone())
                    })
                    .collect();

                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!("Artists ({}) [Tab: cycle results]", results.artists.len()))
                            .border_style(Style::default().fg(Color::Green)),
                    )
                    .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                    .highlight_symbol("> ");
                f.render_stateful_widget(
                    list,
                    results_area,
                    &mut ListState::default().with_selected(Some(state.selected_search_artist)),
                );
            }
        }
    } else {
        let empty_msg = if state.search_query.is_empty() {
            "Type to search for tracks, albums, and artists"
        } else {
            "No results. Press Enter to search."
        };

        let empty = Paragraph::new(empty_msg)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Search Results"),
            );
        f.render_widget(empty, results_area);
    }

    results_area
}
