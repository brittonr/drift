mod mpd;
mod cava;
mod album_art;
mod queue_persistence;
mod download_db;
mod history_db;
mod downloads;
mod config;
mod service;
mod search;
mod search_cache;
mod storage;
mod tidal_db;
mod app;
mod ui;
mod handlers;
mod video;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame, Terminal,
};
use std::{io, time::Duration};

use app::{App, ViewMode};
use handlers::{handle_key_event, KeyAction};
use service::MusicService;
use ui::{
    render_now_playing, render_queue, render_browse_view,
    render_search_view, render_search_preview, render_downloads_view, render_library_view, render_status_bar,
    render_artist_detail_view, render_album_detail_view, render_help_panel, HelpPanelState,
    render_dialog, DialogRenderState, SearchPreviewState,
};

#[tokio::main]
async fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = match App::new().await {
        Ok(app) => app,
        Err(e) => {
            disable_raw_mode()?;
            execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
            eprintln!("Failed to initialize app: {}", e);
            return Err(e);
        }
    };

    let res = run_app(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("{:?}", err);
    }

    Ok(())
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()>
where
    <B as ratatui::backend::Backend>::Error: Send + Sync + 'static,
{
    let mut last_status_check = std::time::Instant::now();

    // Restore saved queue on first tick
    if let Some(persisted) = app.pending_restore.take() {
        app.restore_queue(persisted).await;
    }

    loop {
        // Check MPD status periodically
        if last_status_check.elapsed() > Duration::from_secs(1) {
            if let Err(e) = app.check_mpd_status().await {
                app.add_debug(format!("MPD status check error: {}", e));
            }

            if app.playback.queue_dirty {
                app.save_queue_state().await;
                app.playback.queue_dirty = false;
            }

            app.process_downloads().await;

            if app.view_mode == ViewMode::Library && !app.library.loaded {
                app.load_favorites().await;
            }

            // Check for config file changes
            app.check_config_reload();

            // Poll for cross-device sync events
            app.poll_sync().await;

            last_status_check = std::time::Instant::now();
        }

        app.handle_download_events();
        app.clear_expired_status();

        // Prefetch album art for search preview
        app.prefetch_search_preview_art().await;

        terminal.draw(|f| render_ui(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Mouse(mouse) => {
                    if mouse.kind == MouseEventKind::Down(event::MouseButton::Left) {
                        app.handle_mouse_click(mouse.column, mouse.row).await;
                    }
                }
                Event::Key(key) => {
                    match handle_key_event(app, key).await {
                        KeyAction::Quit => return Ok(()),
                        KeyAction::Continue => {}
                    }
                }
                _ => {}
            }
        }
    }
}

fn render_ui(f: &mut Frame, app: &mut App) {
    // Clone theme early to avoid borrow conflicts with mutable app access
    let theme = app.config.theme.clone();

    // Now Playing height: taller when visualizer is enabled
    let now_playing_height = if app.show_visualizer && app.visualizer.is_some() {
        14  // Extra space for visualizer
    } else {
        9
    };

    let mut constraints = vec![
        Constraint::Length(3),            // Header
        Constraint::Length(now_playing_height),  // Now Playing (with optional visualizer)
    ];

    if app.show_debug {
        constraints.push(Constraint::Percentage(50));  // Main content
        constraints.push(Constraint::Percentage(25));  // Debug panel
    } else {
        constraints.push(Constraint::Min(10));  // Main content takes all remaining space
    }
    constraints.push(Constraint::Length(3));       // Status bar

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(f.area());

    let mut chunk_index = 0;

    // Header
    let header_text = format!(
        "{} - {} Mode",
        if app.music_service.is_authenticated() {
            "Drift - Connected"
        } else {
            "Drift - Demo Mode"
        },
        match app.view_mode {
            ViewMode::Browse => "Browse",
            ViewMode::Search => "Search",
            ViewMode::Downloads => "Downloads",
            ViewMode::Library => "Library",
            ViewMode::ArtistDetail => "Artist",
            ViewMode::AlbumDetail => "Album",
        }
    );
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(theme.primary()).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        );
    f.render_widget(header, main_chunks[chunk_index]);
    chunk_index += 1;

    // Now Playing
    let now_playing_state = ui::now_playing::NowPlayingState {
        current_track: app.current_track.as_ref(),
        current_song: app.current_song.as_ref(),
        is_playing: app.playback.is_playing,
        volume: app.playback.volume,
        repeat_mode: app.playback.repeat_mode,
        random_mode: app.playback.random_mode,
        single_mode: app.playback.single_mode,
        radio_seed: app.playback.radio_seed.clone(),
        local_queue_len: app.local_queue.len(),
        album_art_cache: &mut app.album_art_cache,
        visualizer: if app.show_visualizer { app.visualizer.as_ref() } else { None },
        video_mode: app.playback.video_mode,
    };
    let progress_bar_area = render_now_playing(f, &mut { now_playing_state }, main_chunks[chunk_index], &theme);
    app.clickable_areas.progress_bar = progress_bar_area;
    chunk_index += 1;

    // Main content area
    let content_area = main_chunks[chunk_index];

    if app.playback.show_queue {
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(60),
                Constraint::Percentage(40),
            ])
            .split(content_area);

        render_main_content(f, app, content_chunks[0], &theme);

        let queue_area = render_queue(
            f,
            &app.local_queue,
            app.playback.selected_queue_item,
            app.current_track.as_ref().map(|t| t.id.as_str()),
            content_chunks[1],
            &theme,
        );
        app.clickable_areas.queue_list = Some(queue_area);
    } else {
        app.clickable_areas.queue_list = None;
        render_main_content(f, app, content_area, &theme);
    }
    chunk_index += 1;

    // Debug panel (only shown when enabled)
    if app.show_debug {
        let debug_text: String = app.debug_log
            .iter()
            .rev()
            .take(10)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        let debug_panel = Paragraph::new(debug_text)
            .style(Style::default().fg(theme.text_muted()))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Debug Log [Space+e: export | Space+c: clear | Space+d: hide]")
                    .border_style(Style::default().fg(theme.border_normal())),
            );
        f.render_widget(debug_panel, main_chunks[chunk_index]);
        chunk_index += 1;
    }

    // Status bar
    let status_state = ui::status_bar::StatusBarState {
        is_searching: app.search.is_active,
        space_pressed: app.key_state.space_pressed,
        pending_key: app.key_state.pending_key,
        status_message: app.status_message.as_ref().map(|m| (m.message.clone(), m.is_error)),
    };
    render_status_bar(f, &status_state, main_chunks[chunk_index], &theme);

    // Render help panel as overlay (last, so it's on top)
    if app.show_help {
        let help_state = HelpPanelState {
            scroll_offset: app.help.scroll_offset,
        };
        render_help_panel(f, &help_state, f.area(), &theme);
    }

    // Render dialog as topmost overlay
    if app.is_dialog_open() {
        let dialog_state = DialogRenderState {
            mode: &app.dialog.mode,
            input_text: &app.dialog.input_text,
            selected_index: app.dialog.selected_index,
            playlists: &app.playlists,
        };
        render_dialog(f, &dialog_state, f.area(), &theme);
    }
}

fn render_main_content(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect, theme: &ui::Theme) {
    let current_track_id = app.current_track.as_ref().map(|t| t.id.as_str());

    match app.view_mode {
        ViewMode::Browse => {
            let browse_state = ui::browse::BrowseViewState {
                playlists: &app.playlists,
                tracks: &app.tracks,
                selected_playlist: app.browse.selected_playlist,
                selected_track: app.browse.selected_track,
                selected_tab: app.browse.selected_tab,
                synced_playlist_ids: &app.downloads.synced_playlist_ids,
                current_track_id,
            };
            let (left, right) = render_browse_view(f, &browse_state, area, theme);
            app.clickable_areas.left_list = Some(left);
            app.clickable_areas.right_list = Some(right);
        }
        ViewMode::Search => {
            // Get history suggestions when search is active
            let suggestions: Vec<&str> = if app.search.is_active && app.search.show_suggestions {
                app.search_history.get_suggestions(&app.search.query)
            } else {
                vec![]
            };

            // Split area for search results and preview panel
            let (search_area, preview_area) = if app.search.show_preview {
                let split = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .split(area);
                (split[0], Some(split[1]))
            } else {
                (area, None)
            };

            let search_state = ui::search::SearchViewState {
                search_query: &app.search.query,
                search_results: app.search_results.as_ref(),
                search_tab: app.search.tab,
                selected_search_track: app.search.selected_track,
                selected_search_album: app.search.selected_album,
                selected_search_artist: app.search.selected_artist,
                is_searching: app.search.is_active,
                current_track_id,
                filter_query: &app.search.filter_query,
                filter_active: app.search.filter_active,
                history_suggestions: &suggestions,
                show_suggestions: app.search.show_suggestions,
                selected_suggestion: app.search.history_index.max(0) as usize,
                page: app.search.page,
                has_more: app.search.has_more,
                service_filter: app.search.service_filter,
            };
            app.clickable_areas.left_list = None;
            let right = render_search_view(f, &search_state, search_area, theme);
            app.clickable_areas.right_list = Some(right);

            // Render preview panel as independent panel
            if let Some(preview_rect) = preview_area {
                let mut preview_state = SearchPreviewState {
                    search_results: app.search_results.as_ref(),
                    search_tab: app.search.tab,
                    selected_search_track: app.search.selected_track,
                    selected_search_album: app.search.selected_album,
                    selected_search_artist: app.search.selected_artist,
                    service_filter: app.search.service_filter,
                    album_art_cache: &mut app.album_art_cache,
                };
                render_search_preview(f, &mut preview_state, preview_rect, theme);
            }
        }
        ViewMode::Downloads => {
            let (pending, completed, failed) = app.downloads.download_counts;

            let is_paused = app.download_manager
                .as_ref()
                .map(|dm| dm.is_paused())
                .unwrap_or(false);

            let downloads_state = ui::downloads::DownloadsViewState {
                download_records: &app.download_records,
                selected_download: app.downloads.selected,
                offline_mode: app.downloads.offline_mode,
                is_paused,
                pending_count: pending,
                completed_count: completed,
                failed_count: failed,
            };
            app.clickable_areas.left_list = None;
            let right = render_downloads_view(f, &downloads_state, area, theme);
            app.clickable_areas.right_list = Some(right);
        }
        ViewMode::Library => {
            let library_state = ui::library::LibraryViewState {
                library_tab: app.library.tab,
                favorite_tracks: &app.favorite_tracks,
                favorite_albums: &app.favorite_albums,
                favorite_artists: &app.favorite_artists,
                history_entries: &app.history_entries,
                selected_favorite_track: app.library.selected_track,
                selected_favorite_album: app.library.selected_album,
                selected_favorite_artist: app.library.selected_artist,
                selected_history_entry: app.library.selected_history,
                current_track_id,
                service_filter: app.library.service_filter,
            };
            app.clickable_areas.left_list = None;
            let right = render_library_view(f, &library_state, area, theme);
            app.clickable_areas.right_list = Some(right);
        }
        ViewMode::ArtistDetail => {
            let artist_state = ui::artist_detail::ArtistDetailViewState {
                artist: app.artist_detail.artist.as_ref(),
                top_tracks: &app.artist_detail.top_tracks,
                albums: &app.artist_detail.albums,
                selected_track: app.artist_detail.selected_track,
                selected_album: app.artist_detail.selected_album,
                selected_panel: app.artist_detail.selected_panel,
                current_track_id,
            };
            let (left, right) = render_artist_detail_view(f, &artist_state, area, theme);
            app.clickable_areas.left_list = Some(left);
            app.clickable_areas.right_list = Some(right);
        }
        ViewMode::AlbumDetail => {
            let album_state = ui::album_detail::AlbumDetailViewState {
                album: app.album_detail.album.as_ref(),
                tracks: &app.album_detail.tracks,
                selected_track: app.album_detail.selected_track,
                current_track_id,
            };
            app.clickable_areas.left_list = None;
            let right = render_album_detail_view(f, &album_state, area, theme);
            app.clickable_areas.right_list = Some(right);
        }
    }
}
