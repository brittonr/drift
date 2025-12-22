use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, ViewMode};
use crate::ui::library::LibraryTab;
use crate::ui::search::SearchTab;

pub enum KeyAction {
    Continue,
    Quit,
}

pub async fn handle_key_event(app: &mut App, key: KeyEvent) -> KeyAction {
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

async fn handle_search_input(app: &mut App, key: KeyEvent) -> KeyAction {
    match key.code {
        KeyCode::Enter => {
            app.search.is_active = false;
            if let Err(e) = app.search().await {
                app.add_debug(format!("Search error: {}", e));
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
                app.add_debug(format!("Error toggling playback: {}", e));
            }
        }
        KeyCode::Char('n') => {
            app.add_debug("Next track".to_string());
            if let Err(e) = app.mpd_controller.next(&mut app.debug_log).await {
                app.add_debug(format!("Next failed: {}", e));
            }
        }
        KeyCode::Char('b') => {
            app.add_debug("Previous track".to_string());
            if let Err(e) = app.mpd_controller.previous(&mut app.debug_log).await {
                app.add_debug(format!("Previous failed: {}", e));
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
                app.add_debug(format!("Failed to export log: {}", e));
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
            if let Err(e) = app.add_all_tracks_to_queue().await {
                app.add_debug(format!("Failed to add tracks: {}", e));
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
                app.add_debug(format!("Failed to clear queue: {}", e));
            } else {
                app.queue.clear();
                app.local_queue.clear();
                app.add_debug("Queue cleared".to_string());
                app.playback.queue_dirty = true;
            }
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
                        app.add_debug(format!("Failed to load queue: {}", e));
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
                app.add_debug(format!("Volume error: {}", e));
            }
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            if let Err(e) = app.mpd_controller.volume_down(&mut app.debug_log).await {
                app.add_debug(format!("Volume error: {}", e));
            }
        }

        // Seek controls
        KeyCode::Char('>') | KeyCode::Char('.') => {
            if let Err(e) = app.mpd_controller.seek_forward(&mut app.debug_log).await {
                app.add_debug(format!("Seek error: {}", e));
            }
        }
        KeyCode::Char('<') | KeyCode::Char(',') => {
            if let Err(e) = app.mpd_controller.seek_backward(&mut app.debug_log).await {
                app.add_debug(format!("Seek error: {}", e));
            }
        }

        // Playback mode toggles
        KeyCode::Char('r') => {
            if app.view_mode == ViewMode::Library {
                app.library.loaded = false;
                app.add_debug("Refreshing favorites...".to_string());
            } else {
                if let Err(e) = app.mpd_controller.toggle_repeat(&mut app.debug_log).await {
                    app.add_debug(format!("Repeat toggle error: {}", e));
                }
            }
        }
        KeyCode::Char('s') => {
            if let Err(e) = app.mpd_controller.toggle_random(&mut app.debug_log).await {
                app.add_debug(format!("Shuffle toggle error: {}", e));
            }
        }
        KeyCode::Char('1') => {
            if let Err(e) = app.mpd_controller.toggle_single(&mut app.debug_log).await {
                app.add_debug(format!("Single toggle error: {}", e));
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
            } else {
                // Add selected track to favorites
                if let Some(track) = app.get_selected_track() {
                    app.add_favorite_track(track).await;
                } else {
                    app.add_debug("No track selected to favorite".to_string());
                }
            }
        }

        _ => {}
    }

    KeyAction::Continue
}

async fn handle_enter(app: &mut App) {
    if app.playback.show_queue && !app.local_queue.is_empty() && app.playback.selected_queue_item < app.local_queue.len() {
        app.add_debug(format!("Playing from queue position {}", app.playback.selected_queue_item + 1));
        if let Err(e) = app.mpd_controller.play_position(app.playback.selected_queue_item, &mut app.debug_log).await {
            app.add_debug(format!("Failed to play from queue: {}", e));
        }
    } else if app.view_mode == ViewMode::Browse {
        if app.browse.selected_tab == 0 {
            if let Err(e) = app.load_playlist(app.browse.selected_playlist).await {
                app.add_debug(format!("Error loading playlist: {}", e));
            }
        } else if app.browse.selected_tab == 1 {
            if let Err(e) = app.play_selected_track().await {
                app.add_debug(format!("Error playing track: {}", e));
            }
        }
    } else if app.view_mode == ViewMode::Search {
        match app.search.tab {
            SearchTab::Tracks => {
                if let Err(e) = app.play_selected_track().await {
                    app.add_debug(format!("Error playing track: {}", e));
                }
            }
            SearchTab::Albums => {
                if let Err(e) = app.add_album_to_queue().await {
                    app.add_debug(format!("Error adding album: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
            SearchTab::Artists => {
                if let Err(e) = app.add_artist_to_queue().await {
                    app.add_debug(format!("Error adding artist: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
        }
    }
}

async fn handle_yank(app: &mut App) {
    if app.view_mode == ViewMode::Search {
        match app.search.tab {
            SearchTab::Tracks => {
                if let Err(e) = app.add_selected_track_to_queue().await {
                    app.add_debug(format!("Failed to add track: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
            SearchTab::Albums => {
                if let Err(e) = app.add_album_to_queue().await {
                    app.add_debug(format!("Failed to add album: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
            SearchTab::Artists => {
                if let Err(e) = app.add_artist_to_queue().await {
                    app.add_debug(format!("Failed to add artist: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
        }
    } else if let Err(e) = app.add_selected_track_to_queue().await {
        app.add_debug(format!("Failed to add track: {}", e));
    } else {
        app.playback.queue_dirty = true;
    }
}

async fn handle_play(app: &mut App) {
    if app.playback.show_queue && !app.local_queue.is_empty() && app.playback.selected_queue_item < app.local_queue.len() {
        app.add_debug(format!("Playing from queue position {}", app.playback.selected_queue_item + 1));
        if let Err(e) = app.mpd_controller.play_position(app.playback.selected_queue_item, &mut app.debug_log).await {
            app.add_debug(format!("Failed to play from queue: {}", e));
        }
    } else if app.view_mode == ViewMode::Browse && app.browse.selected_tab == 1 {
        if let Err(e) = app.play_selected_track().await {
            app.add_debug(format!("Error playing track: {}", e));
        }
    } else if app.view_mode == ViewMode::Search {
        match app.search.tab {
            SearchTab::Tracks => {
                if let Err(e) = app.play_selected_track().await {
                    app.add_debug(format!("Error playing track: {}", e));
                }
            }
            SearchTab::Albums => {
                if let Err(e) = app.add_album_to_queue().await {
                    app.add_debug(format!("Error adding album: {}", e));
                } else {
                    app.playback.queue_dirty = true;
                }
            }
            SearchTab::Artists => {
                if let Err(e) = app.add_artist_to_queue().await {
                    app.add_debug(format!("Error adding artist: {}", e));
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
                app.add_debug(format!("Failed to remove track: {}", e));
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

fn handle_tab(app: &mut App) {
    if app.view_mode == ViewMode::Browse {
        app.browse.selected_tab = (app.browse.selected_tab + 1) % 2;
        app.add_debug(format!("Switched to {} panel",
            if app.browse.selected_tab == 0 { "playlists" } else { "tracks" }));
    } else if app.view_mode == ViewMode::Library {
        app.library.tab = match app.library.tab {
            LibraryTab::Tracks => LibraryTab::Albums,
            LibraryTab::Albums => LibraryTab::Artists,
            LibraryTab::Artists => LibraryTab::Tracks,
        };
        app.add_debug(format!("Switched to {:?} favorites", app.library.tab));
    } else if app.view_mode == ViewMode::Search {
        app.search.tab = match app.search.tab {
            SearchTab::Tracks => SearchTab::Albums,
            SearchTab::Albums => SearchTab::Artists,
            SearchTab::Artists => SearchTab::Tracks,
        };
        app.add_debug(format!("Switched to {:?} results", app.search.tab));
    }
}
