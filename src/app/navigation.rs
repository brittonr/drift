use super::App;
use super::state::ViewMode;
use crate::ui::{SearchTab, LibraryTab};

impl App {
    pub fn move_down(&mut self) {
        if self.playback.show_queue && !self.queue.is_empty() {
            self.playback.selected_queue_item = (self.playback.selected_queue_item + 1).min(self.queue.len() - 1);
        } else if self.view_mode == ViewMode::Downloads {
            if !self.download_records.is_empty() {
                self.downloads.selected = (self.downloads.selected + 1).min(self.download_records.len() - 1);
            }
        } else if self.view_mode == ViewMode::Library {
            match self.library.tab {
                LibraryTab::Tracks if !self.favorite_tracks.is_empty() => {
                    self.library.selected_track = (self.library.selected_track + 1).min(self.favorite_tracks.len() - 1);
                }
                LibraryTab::Albums if !self.favorite_albums.is_empty() => {
                    self.library.selected_album = (self.library.selected_album + 1).min(self.favorite_albums.len() - 1);
                }
                LibraryTab::Artists if !self.favorite_artists.is_empty() => {
                    self.library.selected_artist = (self.library.selected_artist + 1).min(self.favorite_artists.len() - 1);
                }
                _ => {}
            }
        } else if self.view_mode == ViewMode::Browse {
            if self.browse.selected_tab == 0 && !self.playlists.is_empty() {
                self.browse.selected_playlist = (self.browse.selected_playlist + 1).min(self.playlists.len() - 1);
            } else if self.browse.selected_tab == 1 && !self.tracks.is_empty() {
                self.browse.selected_track = (self.browse.selected_track + 1).min(self.tracks.len() - 1);
            }
        } else if let Some(ref results) = self.search_results {
            match self.search.tab {
                SearchTab::Tracks if !results.tracks.is_empty() => {
                    self.search.selected_track = (self.search.selected_track + 1).min(results.tracks.len() - 1);
                }
                SearchTab::Albums if !results.albums.is_empty() => {
                    self.search.selected_album = (self.search.selected_album + 1).min(results.albums.len() - 1);
                }
                SearchTab::Artists if !results.artists.is_empty() => {
                    self.search.selected_artist = (self.search.selected_artist + 1).min(results.artists.len() - 1);
                }
                _ => {}
            }
        }
    }

    pub fn move_up(&mut self) {
        if self.playback.show_queue && !self.queue.is_empty() {
            if self.playback.selected_queue_item > 0 {
                self.playback.selected_queue_item -= 1;
            }
        } else if self.view_mode == ViewMode::Downloads {
            if self.downloads.selected > 0 {
                self.downloads.selected -= 1;
            }
        } else if self.view_mode == ViewMode::Library {
            match self.library.tab {
                LibraryTab::Tracks if self.library.selected_track > 0 => {
                    self.library.selected_track -= 1;
                }
                LibraryTab::Albums if self.library.selected_album > 0 => {
                    self.library.selected_album -= 1;
                }
                LibraryTab::Artists if self.library.selected_artist > 0 => {
                    self.library.selected_artist -= 1;
                }
                _ => {}
            }
        } else if self.view_mode == ViewMode::Browse {
            if self.browse.selected_tab == 0 && self.browse.selected_playlist > 0 {
                self.browse.selected_playlist -= 1;
            } else if self.browse.selected_tab == 1 && self.browse.selected_track > 0 {
                self.browse.selected_track -= 1;
            }
        } else if let Some(ref results) = self.search_results {
            match self.search.tab {
                SearchTab::Tracks if self.search.selected_track > 0 => {
                    self.search.selected_track -= 1;
                }
                SearchTab::Albums if self.search.selected_album > 0 => {
                    self.search.selected_album -= 1;
                }
                SearchTab::Artists if self.search.selected_artist > 0 => {
                    self.search.selected_artist -= 1;
                }
                _ => {}
            }
        }
    }

    pub fn move_left(&mut self) {
        if self.view_mode == ViewMode::Browse && self.browse.selected_tab > 0 {
            self.browse.selected_tab = 0;
            self.add_debug("Switched to playlists panel".to_string());
        }
    }

    pub fn move_right(&mut self) {
        if self.view_mode == ViewMode::Browse && self.browse.selected_tab < 1 {
            self.browse.selected_tab = 1;
            self.add_debug("Switched to tracks panel".to_string());
        }
    }

    pub fn jump_to_top(&mut self) {
        if self.playback.show_queue {
            self.playback.selected_queue_item = 0;
        } else if self.view_mode == ViewMode::Browse {
            if self.browse.selected_tab == 0 {
                self.browse.selected_playlist = 0;
            } else {
                self.browse.selected_track = 0;
            }
        } else {
            match self.search.tab {
                SearchTab::Tracks => self.search.selected_track = 0,
                SearchTab::Albums => self.search.selected_album = 0,
                SearchTab::Artists => self.search.selected_artist = 0,
            }
        }
    }

    pub fn jump_to_end(&mut self) {
        if self.playback.show_queue && !self.queue.is_empty() {
            self.playback.selected_queue_item = self.queue.len() - 1;
        } else if self.view_mode == ViewMode::Browse {
            if self.browse.selected_tab == 0 && !self.playlists.is_empty() {
                self.browse.selected_playlist = self.playlists.len() - 1;
            } else if self.browse.selected_tab == 1 && !self.tracks.is_empty() {
                self.browse.selected_track = self.tracks.len() - 1;
            }
        } else if let Some(ref results) = self.search_results {
            match self.search.tab {
                SearchTab::Tracks if !results.tracks.is_empty() => {
                    self.search.selected_track = results.tracks.len() - 1;
                }
                SearchTab::Albums if !results.albums.is_empty() => {
                    self.search.selected_album = results.albums.len() - 1;
                }
                SearchTab::Artists if !results.artists.is_empty() => {
                    self.search.selected_artist = results.artists.len() - 1;
                }
                _ => {}
            }
        }
    }

    pub async fn handle_mouse_click(&mut self, col: u16, row: u16) {
        // Check progress bar for seeking
        if let Some(progress_area) = self.clickable_areas.progress_bar {
            if col >= progress_area.x
                && col < progress_area.x + progress_area.width
                && row >= progress_area.y
                && row < progress_area.y + progress_area.height
            {
                let click_offset = col - progress_area.x;
                let progress_ratio = click_offset as f64 / progress_area.width as f64;

                if let Some(ref song) = self.current_song {
                    let seek_seconds = (song.duration.as_secs_f64() * progress_ratio) as u32;
                    self.add_debug(format!("Seeking to {}s ({}%)", seek_seconds, (progress_ratio * 100.0) as u8));
                    if let Err(e) = self.mpd_controller.seek_to(seek_seconds, &mut self.debug_log).await {
                        self.add_debug(format!("Seek failed: {}", e));
                    }
                }
                return;
            }
        }

        // Check queue list
        if let Some(queue_area) = self.clickable_areas.queue_list {
            if col >= queue_area.x
                && col < queue_area.x + queue_area.width
                && row >= queue_area.y
                && row < queue_area.y + queue_area.height
            {
                let clicked_row = (row - queue_area.y).saturating_sub(1) as usize;
                if clicked_row < self.local_queue.len() {
                    self.playback.selected_queue_item = clicked_row;
                    self.add_debug(format!("Selected queue item {}", clicked_row + 1));
                }
                return;
            }
        }

        // Check left list
        if let Some(left_area) = self.clickable_areas.left_list {
            if col >= left_area.x
                && col < left_area.x + left_area.width
                && row >= left_area.y
                && row < left_area.y + left_area.height
            {
                let clicked_row = (row - left_area.y).saturating_sub(1) as usize;
                if self.view_mode == ViewMode::Browse {
                    self.browse.selected_tab = 0;
                    if clicked_row < self.playlists.len() {
                        self.browse.selected_playlist = clicked_row;
                        self.add_debug(format!("Selected playlist {}", clicked_row + 1));
                    }
                }
                return;
            }
        }

        // Check right list
        if let Some(right_area) = self.clickable_areas.right_list {
            if col >= right_area.x
                && col < right_area.x + right_area.width
                && row >= right_area.y
                && row < right_area.y + right_area.height
            {
                let clicked_row = (row - right_area.y).saturating_sub(1) as usize;
                if self.view_mode == ViewMode::Browse {
                    self.browse.selected_tab = 1;
                    if clicked_row < self.tracks.len() {
                        self.browse.selected_track = clicked_row;
                        self.add_debug(format!("Selected track {}", clicked_row + 1));
                    }
                } else if let Some(ref results) = self.search_results {
                    match self.search.tab {
                        SearchTab::Tracks if clicked_row < results.tracks.len() => {
                            self.search.selected_track = clicked_row;
                        }
                        SearchTab::Albums if clicked_row < results.albums.len() => {
                            self.search.selected_album = clicked_row;
                        }
                        SearchTab::Artists if clicked_row < results.artists.len() => {
                            self.search.selected_artist = clicked_row;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
