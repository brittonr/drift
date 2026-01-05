use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};
use ratatui_image::StatefulImage;

use crate::album_art::AlbumArtCache;
use crate::service::{SearchResults, ServiceType};
use super::styles::{format_track_with_indicator, is_track_playing, service_badge};
use super::theme::Theme;

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
    pub current_track_id: Option<&'a str>,
    /// Filter query for fuzzy filtering results (Ctrl+F)
    pub filter_query: &'a str,
    /// Is filter mode active
    pub filter_active: bool,
    /// History suggestions to show
    pub history_suggestions: &'a [&'a str],
    /// Show history suggestions popup
    pub show_suggestions: bool,
    /// Selected suggestion index
    pub selected_suggestion: usize,
    /// Current page (0-indexed)
    pub page: usize,
    /// Whether more results are available
    pub has_more: bool,
    /// Service filter (None = all, Some = specific service)
    pub service_filter: Option<ServiceType>,
}

/// State for the standalone search preview panel
pub struct SearchPreviewState<'a> {
    pub search_results: Option<&'a SearchResults>,
    pub search_tab: SearchTab,
    pub selected_search_track: usize,
    pub selected_search_album: usize,
    pub selected_search_artist: usize,
    pub service_filter: Option<ServiceType>,
    pub album_art_cache: &'a mut AlbumArtCache,
}

pub fn render_search_view(
    f: &mut Frame,
    state: &SearchViewState,
    area: Rect,
    theme: &Theme,
) -> Rect {
    // Add filter bar if filter mode is active
    let constraints = if state.filter_active {
        vec![Constraint::Length(3), Constraint::Length(3), Constraint::Min(1)]
    } else {
        vec![Constraint::Length(3), Constraint::Min(1)]
    };

    let search_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let results_area = if state.filter_active {
        search_chunks[2]
    } else {
        search_chunks[1]
    };

    // Search input box with enhanced hints
    let title = if state.is_searching {
        "Search (Enter: search | Up/Down: history | Esc: cancel)"
    } else {
        "Search (/: search | Ctrl+F: filter | Tab: cycle results)"
    };

    let search_input = Paragraph::new(state.search_query)
        .style(if state.is_searching {
            Style::default().fg(theme.warning())
        } else {
            Style::default()
        })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(if state.is_searching {
                    Style::default().fg(theme.warning())
                } else {
                    Style::default().fg(theme.border_normal())
                }),
        );
    f.render_widget(search_input, search_chunks[0]);

    // Filter input box (when active)
    if state.filter_active {
        let filter_input = Paragraph::new(state.filter_query)
            .style(Style::default().fg(theme.secondary()))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Filter (Ctrl+F: close | type to fuzzy filter)")
                    .border_style(Style::default().fg(theme.secondary())),
            );
        f.render_widget(filter_input, search_chunks[1]);
    }

    // Service filter indicator
    let service_indicator = match state.service_filter {
        None => String::new(),
        Some(ServiceType::Tidal) => " [Tidal]".to_string(),
        Some(ServiceType::YouTube) => " [YouTube]".to_string(),
        Some(ServiceType::Bandcamp) => " [Bandcamp]".to_string(),
    };

    // Pagination indicator
    let page_indicator = if state.has_more {
        format!(" [pg {}+]", state.page + 1)
    } else if state.page > 0 {
        format!(" [pg {}]", state.page + 1)
    } else {
        String::new()
    };

    // Search results
    if let Some(results) = state.search_results {
        match state.search_tab {
            SearchTab::Tracks => {
                // Filter tracks by service if filter is set
                let filtered_tracks: Vec<_> = results
                    .tracks
                    .iter()
                    .filter(|t| state.service_filter.is_none_or(|s| t.service == s))
                    .collect();

                let items: Vec<ListItem> = filtered_tracks
                    .iter()
                    .enumerate()
                    .map(|(i, track)| {
                        let is_selected = i == state.selected_search_track;
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

                let title = format!(
                    "Tracks ({}){}{} [Tab: cycle | 1/2/3: service | Ctrl+F: filter]",
                    filtered_tracks.len(),
                    service_indicator,
                    page_indicator
                );

                let list = List::new(items)
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
                    results_area,
                    &mut ListState::default().with_selected(Some(state.selected_search_track)),
                );
            }
            SearchTab::Albums => {
                // Filter albums by service if filter is set
                let filtered_albums: Vec<_> = results
                    .albums
                    .iter()
                    .filter(|a| state.service_filter.is_none_or(|s| a.service == s))
                    .collect();

                let items: Vec<ListItem> = filtered_albums
                    .iter()
                    .enumerate()
                    .map(|(i, album)| {
                        let is_selected = i == state.selected_search_album;
                        let display = format!("{} {} - {} ({} tracks)", service_badge(album.service), album.artist, album.title, album.num_tracks);
                        let style = if is_selected {
                            theme.highlight_style()
                        } else {
                            Style::default()
                        };
                        ListItem::new(display).style(style)
                    })
                    .collect();

                let title = format!(
                    "Albums ({}){}{} [Tab: cycle | 1/2/3: service | Ctrl+F: filter]",
                    filtered_albums.len(),
                    service_indicator,
                    page_indicator
                );

                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(title)
                            .border_style(Style::default().fg(theme.secondary())),
                    )
                    .highlight_style(theme.highlight_style())
                    .highlight_symbol("> ");
                f.render_stateful_widget(
                    list,
                    results_area,
                    &mut ListState::default().with_selected(Some(state.selected_search_album)),
                );
            }
            SearchTab::Artists => {
                // Filter artists by service if filter is set
                let filtered_artists: Vec<_> = results
                    .artists
                    .iter()
                    .filter(|a| state.service_filter.is_none_or(|s| a.service == s))
                    .collect();

                let items: Vec<ListItem> = filtered_artists
                    .iter()
                    .enumerate()
                    .map(|(i, artist)| {
                        let is_selected = i == state.selected_search_artist;
                        let display = format!("{} {}", service_badge(artist.service), artist.name);
                        let style = if is_selected {
                            theme.highlight_style()
                        } else {
                            Style::default()
                        };
                        ListItem::new(display).style(style)
                    })
                    .collect();

                let title = format!(
                    "Artists ({}){}{} [Tab: cycle | 1/2/3: service | Ctrl+F: filter]",
                    filtered_artists.len(),
                    service_indicator,
                    page_indicator
                );

                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(title)
                            .border_style(Style::default().fg(theme.success())),
                    )
                    .highlight_style(theme.highlight_style())
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
            .style(Style::default().fg(theme.text_disabled()))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Search Results")
                    .border_style(Style::default().fg(theme.border_normal())),
            );
        f.render_widget(empty, results_area);
    }

    // Render history suggestions popup when typing and suggestions available
    if state.show_suggestions && !state.history_suggestions.is_empty() && state.is_searching {
        render_history_suggestions(f, state, search_chunks[0], theme);
    }

    results_area
}

/// Render the standalone search preview panel with cover art and track/album info
pub fn render_search_preview(
    f: &mut Frame,
    state: &mut SearchPreviewState,
    area: Rect,
    theme: &Theme,
) {
    // Get the selected item based on current tab
    let (cover_art, title, artist, extra_info) = match state.search_tab {
        SearchTab::Tracks => {
            if let Some(results) = state.search_results {
                // Apply service filter
                let filtered: Vec<_> = results.tracks.iter()
                    .filter(|t| state.service_filter.is_none_or(|s| t.service == s))
                    .collect();
                if let Some(track) = filtered.get(state.selected_search_track) {
                    (
                        Some(&track.cover_art),
                        track.title.clone(),
                        track.artist.clone(),
                        format!(
                            "{}:{:02} | {} | {}",
                            track.duration_seconds / 60,
                            track.duration_seconds % 60,
                            track.album,
                            service_badge(track.service)
                        ),
                    )
                } else {
                    (None, String::new(), String::new(), String::new())
                }
            } else {
                (None, String::new(), String::new(), String::new())
            }
        }
        SearchTab::Albums => {
            if let Some(results) = state.search_results {
                let filtered: Vec<_> = results.albums.iter()
                    .filter(|a| state.service_filter.is_none_or(|s| a.service == s))
                    .collect();
                if let Some(album) = filtered.get(state.selected_search_album) {
                    (
                        Some(&album.cover_art),
                        album.title.clone(),
                        album.artist.clone(),
                        format!("{} tracks | {}", album.num_tracks, service_badge(album.service)),
                    )
                } else {
                    (None, String::new(), String::new(), String::new())
                }
            } else {
                (None, String::new(), String::new(), String::new())
            }
        }
        SearchTab::Artists => {
            // Artists don't have cover art in our model
            if let Some(results) = state.search_results {
                let filtered: Vec<_> = results.artists.iter()
                    .filter(|a| state.service_filter.is_none_or(|s| a.service == s))
                    .collect();
                if let Some(artist) = filtered.get(state.selected_search_artist) {
                    (
                        None,
                        artist.name.clone(),
                        String::new(),
                        service_badge(artist.service).to_string(),
                    )
                } else {
                    (None, String::new(), String::new(), String::new())
                }
            } else {
                (None, String::new(), String::new(), String::new())
            }
        }
    };

    // Create the outer block for the panel
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .title(" Preview [P: toggle] ")
        .border_style(Style::default().fg(theme.secondary()));

    let inner_area = outer_block.inner(area);
    f.render_widget(outer_block, area);

    // Layout: art on top, info below - larger album art for better visibility
    let preview_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(75), // Album art area - larger proportion
            Constraint::Min(4),         // Info area
        ])
        .split(inner_area);

    let art_area = preview_chunks[0];
    let info_area = preview_chunks[1];

    // Render album art if available and cached
    let has_art = if let Some(cover) = cover_art {
        if state.album_art_cache.has_cover_cached(cover, 320) {
            let _ = state.album_art_cache.set_current_from_cover(cover, 320);
            if let Some(protocol) = state.album_art_cache.get_protocol_mut() {
                let image_widget = StatefulImage::new(None);
                f.render_stateful_widget(image_widget, art_area, protocol);
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    // Render placeholder if no art
    if !has_art {
        // Center the placeholder text vertically in the art area
        let art_height = art_area.height as usize;
        let padding_lines = art_height.saturating_sub(4) / 2;
        let mut placeholder_lines: Vec<Line> = (0..padding_lines).map(|_| Line::from("")).collect();
        placeholder_lines.push(Line::from(Span::styled("No Preview", Style::default().fg(theme.text_disabled()).add_modifier(Modifier::BOLD))));
        placeholder_lines.push(Line::from(""));
        placeholder_lines.push(Line::from(Span::styled("Loading...", Style::default().fg(theme.text_disabled()).add_modifier(Modifier::ITALIC))));

        let placeholder = Paragraph::new(placeholder_lines)
            .alignment(Alignment::Center);
        f.render_widget(placeholder, art_area);
    }

    // Render track/album info
    let mut info_lines = vec![];

    if !title.is_empty() {
        info_lines.push(Line::from(vec![
            Span::styled(&title, Style::default().fg(theme.text()).add_modifier(Modifier::BOLD)),
        ]));
    }

    if !artist.is_empty() {
        info_lines.push(Line::from(vec![
            Span::styled(&artist, Style::default().fg(theme.primary())),
        ]));
    }

    if !extra_info.is_empty() {
        info_lines.push(Line::from(vec![
            Span::styled(&extra_info, Style::default().fg(theme.text_muted())),
        ]));
    }

    if info_lines.is_empty() {
        info_lines.push(Line::from(Span::styled("Select a result to preview", Style::default().fg(theme.text_disabled()))));
    }

    let info_block = Paragraph::new(info_lines)
        .alignment(Alignment::Center);

    f.render_widget(info_block, info_area);
}

/// Render search history suggestions dropdown
fn render_history_suggestions(
    f: &mut Frame,
    state: &SearchViewState,
    search_input_area: Rect,
    theme: &Theme,
) {
    let suggestions = state.history_suggestions;
    if suggestions.is_empty() {
        return;
    }

    // Position popup below search input
    let popup_height = (suggestions.len() as u16).min(8) + 2; // +2 for borders
    let popup_area = Rect {
        x: search_input_area.x,
        y: search_input_area.y + search_input_area.height,
        width: search_input_area.width.min(50),
        height: popup_height,
    };

    // Clear area for popup
    f.render_widget(Clear, popup_area);

    // Build suggestion items
    let items: Vec<ListItem> = suggestions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == state.selected_suggestion {
                theme.highlight_style().add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_muted())
            };
            ListItem::new(*s).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("History (Up/Down to select)")
                .border_style(Style::default().fg(theme.border_highlight()))
                .style(Style::default().bg(theme.background())),
        )
        .highlight_style(theme.highlight_style())
        .highlight_symbol("> ");

    f.render_stateful_widget(
        list,
        popup_area,
        &mut ListState::default().with_selected(Some(state.selected_suggestion)),
    );
}
