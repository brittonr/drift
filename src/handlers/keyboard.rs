use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, DialogMode, ViewMode};
use crate::app::state::RadioSeed;
use crate::ui::library::LibraryTab;
use crate::ui::search::SearchTab;
use crate::ui::help_content_height;

pub enum KeyAction {
    Continue,
    Quit,
}

pub async fn handle_key_event(app: &mut App, key: KeyEvent) -> KeyAction {
    // Handle dialogs first (highest priority)
    if app.is_dialog_open() {
        return handle_dialog_input(app, key).await;
    }

    // Handle help panel - any key dismisses it (except j/k for scrolling)
    if app.show_help {
        match key.code {
            KeyCode::Char('j') => {
                let max_scroll = help_content_height().saturating_sub(20);
                if app.help.scroll_offset < max_scroll {
                    app.help.scroll_offset += 1;
                }
            }
            KeyCode::Char('k') => {
                app.help.scroll_offset = app.help.scroll_offset.saturating_sub(1);
            }
            _ => {
                // Any other key dismisses help
                app.show_help = false;
                app.help.scroll_offset = 0;
            }
        }
        return KeyAction::Continue;
    }

    // Handle search input mode separately
    if app.search.is_active {
        return handle_search_input(app, key).await;
    }

    // Handle Space-prefixed commands
    if app.key_state.space_pressed {
        return handle_space_command(app, key).await;
    }

    // Handle 'g' prefix for jump commands
    if app.key_state.pending_key == Some('g') {
        return handle_g_command(app, key);
    }

    // Main helix-style commands
    handle_normal_mode(app, key).await
}

async fn handle_dialog_input(app: &mut App, key: KeyEvent) -> KeyAction {
    match &app.dialog.mode {
        DialogMode::None => {}

        DialogMode::CreatePlaylist | DialogMode::RenamePlaylist { .. } => {
            // Text input mode
            match key.code {
                KeyCode::Enter => {
                    match &app.dialog.mode {
                        DialogMode::CreatePlaylist => {
                            app.create_playlist_from_dialog().await;
                        }
                        DialogMode::RenamePlaylist { .. } => {
                            app.rename_playlist_from_dialog().await;
                        }
                        _ => {}
                    }
                }
                KeyCode::Esc => {
                    app.close_dialog();
                }
                KeyCode::Backspace => {
                    app.dialog.input_text.pop();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.dialog.input_text.push(c);
                }
                _ => {}
            }
        }

        DialogMode::AddToPlaylist { .. } => {
            // Playlist selection mode
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    if app.dialog.selected_index < app.playlists.len().saturating_sub(1) {
                        app.dialog.selected_index += 1;
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    if app.dialog.selected_index > 0 {
                        app.dialog.selected_index -= 1;
                    }
                }
                KeyCode::Enter => {
                    app.add_track_to_playlist_from_dialog().await;
                }
                KeyCode::Esc => {
                    app.close_dialog();
                }
                _ => {}
            }
        }

        DialogMode::ConfirmDeletePlaylist { .. } => {
            // Confirmation mode
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    app.delete_playlist_from_dialog().await;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    app.close_dialog();
                }
                _ => {}
            }
        }
    }

    KeyAction::Continue
}

async fn handle_search_input(app: &mut App, key: KeyEvent) -> KeyAction {
    match key.code {
        KeyCode::Enter => {
            app.search.is_active = false;
            if let Err(e) = app.search().await {
                app.set_status_error(format!("Search error: {}", e));
            }
        }
        KeyCode::Esc => {
            app.search.is_active = false;
            app.search.query.clear();
        }
        KeyCode::Backspace => {
            app.search.query.pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.search.query.push(c);
        }
        _ => {}
    }
    KeyAction::Continue
}

async fn handle_space_command(app: &mut App, key: KeyEvent) -> KeyAction {
    app.key_state.space_pressed = false;

    match key.code {
        KeyCode::Char('q') => {
            if app.playback.queue_dirty {
                app.save_queue_state().await;
            }
            return KeyAction::Quit;
        }
        KeyCode::Char('p') => {
            if let Err(e) = app.toggle_playback().await {
                app.set_status_error(format!("Error toggling playback: {}", e));
            }
        }
        KeyCode::Char('n') => {
            app.add_debug("Next track".to_string());
            if let Err(e) = app.mpd_controller.next(&mut app.debug_log).await {
                app.set_status_error(format!("Next failed: {}", e));
            }
        }
        KeyCode::Char('b') => {
            app.add_debug("Previous track".to_string());
            if let Err(e) = app.mpd_controller.previous(&mut app.debug_log).await {
                app.set_status_error(format!("Previous failed: {}", e));
            }
        }
        KeyCode::Char('v') => {
            app.show_visualizer = !app.show_visualizer;
            app.add_debug(format!("Visualizer {}", if app.show_visualizer { "enabled" } else { "disabled" }));
        }
        KeyCode::Char('c') => {
            app.debug_log.clear();
            app.add_debug("Debug log cleared".to_string());
        }
        KeyCode::Char('e') => {
            let export_path = "/tmp/tidal-tui-export.log";
            let mut content = String::new();
            for line in &app.debug_log {
                content.push_str(line);
                content.push('\n');
            }
            if let Err(e) = std::fs::write(export_path, content) {
                app.set_status_error(format!("Failed to export log: {}", e));
            } else {
                app.add_debug(format!("Debug log exported to {}", export_path));
            }
        }
        _ => {}
    }
    KeyAction::Continue
}

fn handle_g_command(app: &mut App, key: KeyEvent) -> KeyAction {
    app.key_state.pending_key = None;

    match key.code {
        KeyCode::Char('g') => {
            app.jump_to_top();
        }
        KeyCode::Char('e') => {
            app.jump_to_end();
        }
        _ => {}
    }
    KeyAction::Continue
}

async fn handle_normal_mode(app: &mut App, key: KeyEvent) -> KeyAction {
    match key.code {
        // Navigation
        KeyCode::Char('h') => app.move_left(),
        KeyCode::Char('j') => app.move_down(),
        KeyCode::Char('k') => app.move_up(),
        KeyCode::Char('l') => app.move_right(),

        // Jump commands (prefix)
        KeyCode::Char('g') => {
            app.key_state.pending_key = Some('g');
        }

        // Space prefix for commands
        KeyCode::Char(' ') => {
            app.key_state.space_pressed = true;
        }

        // Enter: load playlist or play track
        KeyCode::Enter => {
            handle_enter(app).await;
        }

        // y: yank/add to queue
        KeyCode::Char('y') => {
            handle_yank(app).await;
        }

        // Y: yank all
        KeyCode::Char('Y') => {
            if app.view_mode == ViewMode::AlbumDetail {
                if let Err(e) = app.add_album_detail_tracks_to_queue().await {
                    app.set_status_error(format!("Failed to add tracks: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            } else if let Err(e) = app.add_all_tracks_to_queue().await {
                app.set_status_error(format!("Failed to add tracks: {}", e));
            } else {
                app.playback.queue_dirty = true;
            }
        }

        // p: play selected
        KeyCode::Char('p') => {
            handle_play(app).await;
        }

        // d: delete/remove from queue
        KeyCode::Char('d') => {
            handle_delete(app).await;
        }

        // D: clear entire queue
        KeyCode::Char('D') => {
            if let Err(e) = app.mpd_controller.clear_queue(&mut app.debug_log).await {
                app.set_status_error(format!("Failed to clear queue: {}", e));
            } else {
                app.queue.clear();
                app.local_queue.clear();
                app.add_debug("Queue cleared".to_string());
                app.playback.queue_dirty = true;
            }
        }

        // J: Move selected track down in queue
        KeyCode::Char('J') => {
            handle_queue_move_down(app).await;
        }

        // K: Move selected track up in queue
        KeyCode::Char('K') => {
            handle_queue_move_up(app).await;
        }

        // /: search
        KeyCode::Char('/') => {
            app.view_mode = ViewMode::Search;
            app.search.is_active = true;
            app.search.query.clear();
            app.add_debug("Search mode activated".to_string());
        }

        // b: browse mode
        KeyCode::Char('b') => {
            app.view_mode = ViewMode::Browse;
            app.add_debug("Browse mode activated".to_string());
        }

        // w: toggle queue
        KeyCode::Char('w') => {
            app.playback.show_queue = !app.playback.show_queue;
            if app.playback.show_queue {
                match app.mpd_controller.get_queue().await {
                    Ok(queue) => {
                        app.queue = queue;
                        app.add_debug(format!("Queue loaded: {} tracks", app.queue.len()));
                    }
                    Err(e) => {
                        app.set_status_error(format!("Failed to load queue: {}", e));
                    }
                }
            }
            app.add_debug(format!("Queue {}", if app.playback.show_queue { "shown" } else { "hidden" }));
        }

        // Tab: cycle through tabs
        KeyCode::Tab => {
            handle_tab(app);
        }

        // Volume controls
        KeyCode::Char('=') | KeyCode::Char('+') => {
            if let Err(e) = app.mpd_controller.volume_up(&mut app.debug_log).await {
                app.set_status_error(format!("Volume error: {}", e));
            }
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            if let Err(e) = app.mpd_controller.volume_down(&mut app.debug_log).await {
                app.set_status_error(format!("Volume error: {}", e));
            }
        }

        // Seek controls
        KeyCode::Char('>') | KeyCode::Char('.') | KeyCode::Char(']') => {
            if let Err(e) = app.mpd_controller.seek_forward(&mut app.debug_log).await {
                app.set_status_error(format!("Seek error: {}", e));
            }
        }
        KeyCode::Char('<') | KeyCode::Char(',') | KeyCode::Char('[') => {
            if let Err(e) = app.mpd_controller.seek_backward(&mut app.debug_log).await {
                app.set_status_error(format!("Seek error: {}", e));
            }
        }

        // Playback mode toggles
        KeyCode::Char('r') => {
            if app.view_mode == ViewMode::Library {
                app.library.loaded = false;
                app.add_debug("Refreshing favorites...".to_string());
            } else {
                if let Err(e) = app.mpd_controller.toggle_repeat(&mut app.debug_log).await {
                    app.set_status_error(format!("Repeat toggle error: {}", e));
                }
            }
        }
        KeyCode::Char('s') => {
            if let Err(e) = app.mpd_controller.toggle_random(&mut app.debug_log).await {
                app.set_status_error(format!("Shuffle toggle error: {}", e));
            }
        }
        KeyCode::Char('1') => {
            if let Err(e) = app.mpd_controller.toggle_single(&mut app.debug_log).await {
                app.set_status_error(format!("Single toggle error: {}", e));
            }
        }

        // Download controls
        KeyCode::Char('O') => {
            app.download_selected_track();
        }

        KeyCode::Char('S') => {
            app.sync_selected_playlist();
        }

        KeyCode::Char('o') => {
            app.downloads.offline_mode = !app.downloads.offline_mode;
            app.add_debug(format!("Offline mode: {}", if app.downloads.offline_mode { "ON" } else { "OFF" }));
        }

        KeyCode::Char('W') => {
            app.view_mode = ViewMode::Downloads;
            app.refresh_download_list();
            app.add_debug("Downloads view".to_string());
        }

        KeyCode::Char('x') => {
            if app.view_mode == ViewMode::Downloads {
                app.delete_selected_download();
            }
        }

        KeyCode::Char('R') => {
            if app.view_mode == ViewMode::Downloads {
                app.retry_selected_download();
            } else if app.playback.radio_mode() {
                // Turn off radio mode
                app.playback.radio_seed = None;
                app.add_debug("Radio mode OFF".to_string());
            } else {
                // Turn on radio - determine seed based on context
                // Priority: ArtistDetail, AlbumDetail, Library tabs, Search tabs, Browse playlists, current track
                if app.view_mode == ViewMode::ArtistDetail {
                    // Artist detail view - use the viewed artist
                    if let Some(ref artist) = app.artist_detail.artist {
                        app.playback.radio_seed = Some(RadioSeed::Artist(artist.id));
                        app.add_debug(format!("Artist Radio ON ({})", artist.name));
                    } else {
                        app.add_debug("No artist loaded for Radio".to_string());
                    }
                } else if app.view_mode == ViewMode::AlbumDetail {
                    // Album detail view - use the viewed album
                    if let Some(ref album) = app.album_detail.album {
                        app.playback.radio_seed = Some(RadioSeed::Album(album.id.clone()));
                        app.add_debug(format!("Album Radio ON ({})", album.title));
                    } else {
                        app.add_debug("No album loaded for Radio".to_string());
                    }
                } else if app.view_mode == ViewMode::Library && app.library.tab == LibraryTab::Artists {
                    // Library Artists tab - use selected favorite artist
                    if app.library.selected_artist < app.favorite_artists.len() {
                        let artist = &app.favorite_artists[app.library.selected_artist];
                        app.playback.radio_seed = Some(RadioSeed::Artist(artist.id));
                        app.add_debug(format!("Artist Radio ON ({})", artist.name));
                    } else {
                        app.add_debug("No artist selected for Radio".to_string());
                    }
                } else if app.view_mode == ViewMode::Library && app.library.tab == LibraryTab::Albums {
                    // Library Albums tab - use selected favorite album
                    if app.library.selected_album < app.favorite_albums.len() {
                        let album = &app.favorite_albums[app.library.selected_album];
                        app.playback.radio_seed = Some(RadioSeed::Album(album.id.clone()));
                        app.add_debug(format!("Album Radio ON ({})", album.title));
                    } else {
                        app.add_debug("No album selected for Radio".to_string());
                    }
                } else if app.view_mode == ViewMode::Search {
                    if let Some(ref results) = app.search_results {
                        match app.search.tab {
                            SearchTab::Artists => {
                                if app.search.selected_artist < results.artists.len() {
                                    let artist = &results.artists[app.search.selected_artist];
                                    app.playback.radio_seed = Some(RadioSeed::Artist(artist.id));
                                    app.add_debug(format!("Artist Radio ON ({})", artist.name));
                                } else {
                                    app.add_debug("No artist selected for Radio".to_string());
                                }
                            }
                            SearchTab::Albums => {
                                if app.search.selected_album < results.albums.len() {
                                    let album = &results.albums[app.search.selected_album];
                                    app.playback.radio_seed = Some(RadioSeed::Album(album.id.clone()));
                                    app.add_debug(format!("Album Radio ON ({})", album.title));
                                } else {
                                    app.add_debug("No album selected for Radio".to_string());
                                }
                            }
                            _ => {
                                // Fall through to track-based radio
                                if let Some(ref track) = app.current_track {
                                    app.playback.radio_seed = Some(RadioSeed::Track(track.id));
                                    app.add_debug(format!("Radio ON (seed: {})", track.title));
                                } else {
                                    app.add_debug("No track playing for Radio seed".to_string());
                                }
                            }
                        }
                    } else if let Some(ref track) = app.current_track {
                        app.playback.radio_seed = Some(RadioSeed::Track(track.id));
                        app.add_debug(format!("Radio ON (seed: {})", track.title));
                    } else {
                        app.add_debug("No track playing for Radio seed".to_string());
                    }
                } else if app.view_mode == ViewMode::Browse && app.browse.selected_tab == 0 {
                    // Playlist tab selected - use playlist as seed for mix radio
                    if app.browse.selected_playlist < app.playlists.len() {
                        let playlist = &app.playlists[app.browse.selected_playlist];
                        app.playback.radio_seed = Some(RadioSeed::Playlist(playlist.id.clone()));
                        app.add_debug(format!("Mix Radio ON (playlist: {})", playlist.title));
                    } else {
                        app.add_debug("No playlist selected for Mix Radio".to_string());
                    }
                } else if let Some(ref track) = app.current_track {
                    // Fallback: use current playing track as seed
                    app.playback.radio_seed = Some(RadioSeed::Track(track.id));
                    app.add_debug(format!("Radio ON (seed: {})", track.title));
                } else {
                    app.add_debug("No track playing for Radio seed".to_string());
                }
            }
        }

        KeyCode::Char('L') => {
            app.view_mode = ViewMode::Library;
            if !app.library.loaded {
                app.add_debug("Loading favorites...".to_string());
            }
            app.add_debug("Library view".to_string());
        }

        KeyCode::Char('f') => {
            if app.view_mode == ViewMode::Library && app.library.tab == LibraryTab::Tracks {
                // Remove from favorites
                if !app.favorite_tracks.is_empty() && app.library.selected_track < app.favorite_tracks.len() {
                    app.remove_favorite_track(app.library.selected_track).await;
                }
            } else if app.view_mode == ViewMode::Library && app.library.tab == LibraryTab::History {
                // Add history track to favorites
                if app.library.selected_history < app.history_entries.len() {
                    let track = crate::tidal::Track::from(&app.history_entries[app.library.selected_history]);
                    app.add_favorite_track(track).await;
                }
            } else {
                // Add selected track to favorites
                if let Some(track) = app.get_selected_track() {
                    app.add_favorite_track(track).await;
                } else {
                    app.add_debug("No track selected to favorite".to_string());
                }
            }
        }

        // v: view detail (open artist/album detail view)
        KeyCode::Char('v') => {
            handle_view_detail(app).await;
        }

        // Esc: back navigation for detail views
        KeyCode::Esc => {
            if app.view_mode == ViewMode::ArtistDetail || app.view_mode == ViewMode::AlbumDetail {
                app.pop_view();
                app.add_debug("Back to previous view".to_string());
            }
        }

        // ?: show help
        KeyCode::Char('?') => {
            app.show_help = true;
            app.help.scroll_offset = 0;
        }

        // C: create new playlist
        KeyCode::Char('C') => {
            app.open_create_playlist_dialog();
        }

        // a: add track to playlist
        KeyCode::Char('a') => {
            handle_add_to_playlist(app);
        }

        // e: rename/edit playlist (when on playlists panel)
        KeyCode::Char('e') => {
            if app.view_mode == ViewMode::Browse && app.browse.selected_tab == 0 {
                if app.browse.selected_playlist < app.playlists.len() {
                    let playlist = app.playlists[app.browse.selected_playlist].clone();
                    if playlist.id.starts_with("demo-") {
                        app.add_debug("Cannot rename demo playlists".to_string());
                    } else {
                        app.open_rename_playlist_dialog(&playlist);
                    }
                }
            }
        }

        // X: delete playlist (when on playlists panel) or remove track from playlist (when on tracks panel)
        KeyCode::Char('X') => {
            if app.view_mode == ViewMode::Browse {
                if app.browse.selected_tab == 0 {
                    // Delete playlist
                    if app.browse.selected_playlist < app.playlists.len() {
                        let playlist = app.playlists[app.browse.selected_playlist].clone();
                        if playlist.id.starts_with("demo-") {
                            app.add_debug("Cannot delete demo playlists".to_string());
                        } else {
                            app.open_delete_playlist_dialog(&playlist);
                        }
                    }
                } else if app.browse.selected_tab == 1 {
                    // Remove track from current playlist
                    app.remove_track_from_current_playlist().await;
                }
            }
        }

        _ => {}
    }

    KeyAction::Continue
}

fn handle_add_to_playlist(app: &mut App) {
    // Find the track to add based on current view/context
    let track = match app.view_mode {
        ViewMode::Browse => {
            if app.browse.selected_tab == 1 && app.browse.selected_track < app.tracks.len() {
                Some(app.tracks[app.browse.selected_track].clone())
            } else {
                None
            }
        }
        ViewMode::Search => {
            if let Some(ref results) = app.search_results {
                if app.search.tab == SearchTab::Tracks && app.search.selected_track < results.tracks.len() {
                    Some(results.tracks[app.search.selected_track].clone())
                } else {
                    None
                }
            } else {
                None
            }
        }
        ViewMode::Library => {
            if app.library.tab == LibraryTab::Tracks && app.library.selected_track < app.favorite_tracks.len() {
                Some(app.favorite_tracks[app.library.selected_track].clone())
            } else if app.library.tab == LibraryTab::History && app.library.selected_history < app.history_entries.len() {
                Some(crate::tidal::Track::from(&app.history_entries[app.library.selected_history]))
            } else {
                None
            }
        }
        ViewMode::ArtistDetail => {
            if app.artist_detail.selected_panel == 0 && app.artist_detail.selected_track < app.artist_detail.top_tracks.len() {
                Some(app.artist_detail.top_tracks[app.artist_detail.selected_track].clone())
            } else {
                None
            }
        }
        ViewMode::AlbumDetail => {
            if app.album_detail.selected_track < app.album_detail.tracks.len() {
                Some(app.album_detail.tracks[app.album_detail.selected_track].clone())
            } else {
                None
            }
        }
        _ => None,
    };

    if let Some(track) = track {
        app.open_add_to_playlist_dialog(&track);
    } else {
        app.add_debug("No track selected to add to playlist".to_string());
    }
}

async fn handle_enter(app: &mut App) {
    if app.playback.show_queue && !app.local_queue.is_empty() && app.playback.selected_queue_item < app.local_queue.len() {
        app.add_debug(format!("Playing from queue position {}", app.playback.selected_queue_item + 1));
        if let Err(e) = app.mpd_controller.play_position(app.playback.selected_queue_item, &mut app.debug_log).await {
            app.set_status_error(format!("Failed to play from queue: {}", e));
        }
    } else if app.view_mode == ViewMode::ArtistDetail {
        if app.artist_detail.selected_panel == 0 {
            // Play selected top track
            if app.artist_detail.selected_track < app.artist_detail.top_tracks.len() {
                let track = app.artist_detail.top_tracks[app.artist_detail.selected_track].clone();
                if let Err(e) = app.play_track(track).await {
                    app.set_status_error(format!("Error playing track: {}", e));
                }
            }
        } else {
            // Queue entire album
            if app.artist_detail.selected_album < app.artist_detail.albums.len() {
                let album = app.artist_detail.albums[app.artist_detail.selected_album].clone();
                app.add_debug(format!("Adding album to queue: {}", album.title));
                if let Err(e) = app.add_album_by_id(&album.id).await {
                    app.set_status_error(format!("Error adding album: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
        }
    } else if app.view_mode == ViewMode::AlbumDetail {
        // Play selected track
        if app.album_detail.selected_track < app.album_detail.tracks.len() {
            let track = app.album_detail.tracks[app.album_detail.selected_track].clone();
            if let Err(e) = app.play_track(track).await {
                app.set_status_error(format!("Error playing track: {}", e));
            }
        }
    } else if app.view_mode == ViewMode::Browse {
        if app.browse.selected_tab == 0 {
            if let Err(e) = app.load_playlist(app.browse.selected_playlist).await {
                app.set_status_error(format!("Error loading playlist: {}", e));
            }
        } else if app.browse.selected_tab == 1 {
            if let Err(e) = app.play_selected_track().await {
                app.set_status_error(format!("Error playing track: {}", e));
            }
        }
    } else if app.view_mode == ViewMode::Search {
        match app.search.tab {
            SearchTab::Tracks => {
                if let Err(e) = app.play_selected_track().await {
                    app.set_status_error(format!("Error playing track: {}", e));
                }
            }
            SearchTab::Albums => {
                if let Err(e) = app.add_album_to_queue().await {
                    app.set_status_error(format!("Error adding album: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
            SearchTab::Artists => {
                if let Err(e) = app.add_artist_to_queue().await {
                    app.set_status_error(format!("Error adding artist: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
        }
    }
}

async fn handle_yank(app: &mut App) {
    if app.view_mode == ViewMode::ArtistDetail {
        if app.artist_detail.selected_panel == 0 {
            // Add selected top track to queue
            if app.artist_detail.selected_track < app.artist_detail.top_tracks.len() {
                let track = app.artist_detail.top_tracks[app.artist_detail.selected_track].clone();
                if let Err(e) = app.add_track_to_queue(track).await {
                    app.set_status_error(format!("Failed to add track: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
        } else {
            // Add all album tracks to queue
            if app.artist_detail.selected_album < app.artist_detail.albums.len() {
                let album = app.artist_detail.albums[app.artist_detail.selected_album].clone();
                if let Err(e) = app.add_album_by_id(&album.id).await {
                    app.set_status_error(format!("Failed to add album: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
        }
    } else if app.view_mode == ViewMode::AlbumDetail {
        // Add selected track to queue
        if app.album_detail.selected_track < app.album_detail.tracks.len() {
            let track = app.album_detail.tracks[app.album_detail.selected_track].clone();
            if let Err(e) = app.add_track_to_queue(track).await {
                app.set_status_error(format!("Failed to add track: {}", e));
            } else {
                app.playback.queue_dirty = true;
            }
        }
    } else if app.view_mode == ViewMode::Search {
        match app.search.tab {
            SearchTab::Tracks => {
                if let Err(e) = app.add_selected_track_to_queue().await {
                    app.set_status_error(format!("Failed to add track: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
            SearchTab::Albums => {
                if let Err(e) = app.add_album_to_queue().await {
                    app.set_status_error(format!("Failed to add album: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
            SearchTab::Artists => {
                if let Err(e) = app.add_artist_to_queue().await {
                    app.set_status_error(format!("Failed to add artist: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
        }
    } else if app.view_mode == ViewMode::Library && app.library.tab == LibraryTab::History {
        // Add history track to queue
        if app.library.selected_history < app.history_entries.len() {
            let track = crate::tidal::Track::from(&app.history_entries[app.library.selected_history]);
            if let Err(e) = app.add_track_to_queue(track).await {
                app.set_status_error(format!("Failed to add track: {}", e));
            } else {
                app.playback.queue_dirty = true;
            }
        }
    } else if let Err(e) = app.add_selected_track_to_queue().await {
        app.set_status_error(format!("Failed to add track: {}", e));
    } else {
        app.playback.queue_dirty = true;
    }
}

async fn handle_play(app: &mut App) {
    if app.playback.show_queue && !app.local_queue.is_empty() && app.playback.selected_queue_item < app.local_queue.len() {
        app.add_debug(format!("Playing from queue position {}", app.playback.selected_queue_item + 1));
        if let Err(e) = app.mpd_controller.play_position(app.playback.selected_queue_item, &mut app.debug_log).await {
            app.set_status_error(format!("Failed to play from queue: {}", e));
        }
    } else if app.view_mode == ViewMode::ArtistDetail {
        if app.artist_detail.selected_panel == 0 {
            // Play selected top track
            if app.artist_detail.selected_track < app.artist_detail.top_tracks.len() {
                let track = app.artist_detail.top_tracks[app.artist_detail.selected_track].clone();
                if let Err(e) = app.play_track(track).await {
                    app.set_status_error(format!("Error playing track: {}", e));
                }
            }
        } else {
            // Queue and play album
            if app.artist_detail.selected_album < app.artist_detail.albums.len() {
                let album = app.artist_detail.albums[app.artist_detail.selected_album].clone();
                if let Err(e) = app.add_album_by_id(&album.id).await {
                    app.set_status_error(format!("Error adding album: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
        }
    } else if app.view_mode == ViewMode::AlbumDetail {
        // Play selected track
        if app.album_detail.selected_track < app.album_detail.tracks.len() {
            let track = app.album_detail.tracks[app.album_detail.selected_track].clone();
            if let Err(e) = app.play_track(track).await {
                app.set_status_error(format!("Error playing track: {}", e));
            }
        }
    } else if app.view_mode == ViewMode::Browse && app.browse.selected_tab == 1 {
        if let Err(e) = app.play_selected_track().await {
            app.set_status_error(format!("Error playing track: {}", e));
        }
    } else if app.view_mode == ViewMode::Search {
        match app.search.tab {
            SearchTab::Tracks => {
                if let Err(e) = app.play_selected_track().await {
                    app.set_status_error(format!("Error playing track: {}", e));
                }
            }
            SearchTab::Albums => {
                if let Err(e) = app.add_album_to_queue().await {
                    app.set_status_error(format!("Error adding album: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
            SearchTab::Artists => {
                if let Err(e) = app.add_artist_to_queue().await {
                    app.set_status_error(format!("Error adding artist: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
        }
    }
}

async fn handle_delete(app: &mut App) {
    if app.playback.show_queue && !app.local_queue.is_empty() {
        if app.playback.selected_queue_item < app.local_queue.len() {
            if let Err(e) = app.mpd_controller.remove_from_queue(app.playback.selected_queue_item, &mut app.debug_log).await {
                app.set_status_error(format!("Failed to remove track: {}", e));
            } else {
                app.local_queue.remove(app.playback.selected_queue_item);
                if app.playback.selected_queue_item > 0 && app.playback.selected_queue_item >= app.local_queue.len() {
                    app.playback.selected_queue_item -= 1;
                }
                app.add_debug(format!("Removed track from queue, {} remaining", app.local_queue.len()));
                app.playback.queue_dirty = true;
            }
        }
    }
}

async fn handle_queue_move_up(app: &mut App) {
    // Only works when queue is visible and has items
    if !app.playback.show_queue || app.local_queue.is_empty() {
        return;
    }

    let selected = app.playback.selected_queue_item;

    // Can't move first item up
    if selected == 0 {
        return;
    }

    let target = selected - 1;

    // Move in MPD first
    if let Err(e) = app
        .mpd_controller
        .move_in_queue(selected, target, &mut app.debug_log)
        .await
    {
        app.set_status_error(format!("Failed to move track up: {}", e));
        return;
    }

    // Update local queue
    app.local_queue.swap(selected, target);

    // Also update the QueueItem vec if populated
    if !app.queue.is_empty() && selected < app.queue.len() && target < app.queue.len() {
        app.queue.swap(selected, target);
    }

    // Move selection to follow the track
    app.playback.selected_queue_item = target;
    app.playback.queue_dirty = true;
}

async fn handle_queue_move_down(app: &mut App) {
    // Only works when queue is visible and has items
    if !app.playback.show_queue || app.local_queue.is_empty() {
        return;
    }

    let selected = app.playback.selected_queue_item;

    // Can't move last item down
    if selected >= app.local_queue.len() - 1 {
        return;
    }

    let target = selected + 1;

    // Move in MPD first
    if let Err(e) = app
        .mpd_controller
        .move_in_queue(selected, target, &mut app.debug_log)
        .await
    {
        app.set_status_error(format!("Failed to move track down: {}", e));
        return;
    }

    // Update local queue
    app.local_queue.swap(selected, target);

    // Also update the QueueItem vec if populated
    if !app.queue.is_empty() && selected < app.queue.len() && target < app.queue.len() {
        app.queue.swap(selected, target);
    }

    // Move selection to follow the track
    app.playback.selected_queue_item = target;
    app.playback.queue_dirty = true;
}

fn handle_tab(app: &mut App) {
    if app.view_mode == ViewMode::Browse {
        app.browse.selected_tab = (app.browse.selected_tab + 1) % 2;
        app.add_debug(format!("Switched to {} panel",
            if app.browse.selected_tab == 0 { "playlists" } else { "tracks" }));
    } else if app.view_mode == ViewMode::ArtistDetail {
        app.artist_detail.selected_panel = (app.artist_detail.selected_panel + 1) % 2;
        app.add_debug(format!("Switched to {} panel",
            if app.artist_detail.selected_panel == 0 { "top tracks" } else { "albums" }));
    } else if app.view_mode == ViewMode::Library {
        app.library.tab = match app.library.tab {
            LibraryTab::Tracks => LibraryTab::Albums,
            LibraryTab::Albums => LibraryTab::Artists,
            LibraryTab::Artists => LibraryTab::History,
            LibraryTab::History => LibraryTab::Tracks,
        };
        app.add_debug(format!("Switched to {:?} tab", app.library.tab));
    } else if app.view_mode == ViewMode::Search {
        app.search.tab = match app.search.tab {
            SearchTab::Tracks => SearchTab::Albums,
            SearchTab::Albums => SearchTab::Artists,
            SearchTab::Artists => SearchTab::Tracks,
        };
        app.add_debug(format!("Switched to {:?} results", app.search.tab));
    }
}

async fn handle_view_detail(app: &mut App) {
    match app.view_mode {
        ViewMode::Search => {
            if let Some(ref results) = app.search_results {
                match app.search.tab {
                    SearchTab::Artists => {
                        if app.search.selected_artist < results.artists.len() {
                            let artist = results.artists[app.search.selected_artist].clone();
                            app.add_debug(format!("Opening artist: {}", artist.name));
                            app.push_view(ViewMode::ArtistDetail);
                            app.load_artist_detail(artist).await;
                        }
                    }
                    SearchTab::Albums => {
                        if app.search.selected_album < results.albums.len() {
                            let album = results.albums[app.search.selected_album].clone();
                            app.add_debug(format!("Opening album: {}", album.title));
                            app.push_view(ViewMode::AlbumDetail);
                            app.load_album_detail(album).await;
                        }
                    }
                    _ => {
                        app.add_debug("Use 'v' on Artists or Albums tab".to_string());
                    }
                }
            }
        }
        ViewMode::Library => {
            match app.library.tab {
                LibraryTab::Artists => {
                    if app.library.selected_artist < app.favorite_artists.len() {
                        let artist = app.favorite_artists[app.library.selected_artist].clone();
                        app.add_debug(format!("Opening artist: {}", artist.name));
                        app.push_view(ViewMode::ArtistDetail);
                        app.load_artist_detail(artist).await;
                    }
                }
                LibraryTab::Albums => {
                    if app.library.selected_album < app.favorite_albums.len() {
                        let album = app.favorite_albums[app.library.selected_album].clone();
                        app.add_debug(format!("Opening album: {}", album.title));
                        app.push_view(ViewMode::AlbumDetail);
                        app.load_album_detail(album).await;
                    }
                }
                _ => {
                    app.add_debug("Use 'v' on Artists or Albums tab".to_string());
                }
            }
        }
        ViewMode::ArtistDetail => {
            // From artist detail, 'v' on an album opens album detail
            if app.artist_detail.selected_panel == 1 {
                if app.artist_detail.selected_album < app.artist_detail.albums.len() {
                    let album = app.artist_detail.albums[app.artist_detail.selected_album].clone();
                    app.add_debug(format!("Opening album: {}", album.title));
                    app.push_view(ViewMode::AlbumDetail);
                    app.load_album_detail(album).await;
                }
            } else {
                app.add_debug("Switch to albums panel (h/l) to view album details".to_string());
            }
        }
        _ => {
            app.add_debug("Use 'v' in Search or Library view on artists/albums".to_string());
        }
    }
}
