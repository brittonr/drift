use super::App;
use super::state::ViewMode;
use crate::download_db::DownloadStatus;
use crate::downloads::DownloadEvent;
use crate::service::Track;
use crate::ui::library::LibraryTab;
use crate::ui::search::SearchTab;

impl App {
    pub fn download_selected_track(&mut self) {
        let track = match self.view_mode {
            ViewMode::Browse => {
                if self.browse.selected_tab == 1 && self.browse.selected_track < self.tracks.len() {
                    Some(self.tracks[self.browse.selected_track].clone())
                } else {
                    None
                }
            }
            ViewMode::Search => {
                if let Some(ref results) = self.search_results {
                    match self.search.tab {
                        SearchTab::Tracks if self.search.selected_track < results.tracks.len() => {
                            Some(results.tracks[self.search.selected_track].clone())
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
            ViewMode::Library => {
                if self.library.tab == LibraryTab::Tracks && self.library.selected_track < self.favorite_tracks.len() {
                    Some(self.favorite_tracks[self.library.selected_track].clone())
                } else {
                    None
                }
            }
            ViewMode::AlbumDetail => {
                if self.album_detail.selected_track < self.album_detail.tracks.len() {
                    Some(self.album_detail.tracks[self.album_detail.selected_track].clone())
                } else {
                    None
                }
            }
            ViewMode::ArtistDetail => {
                if self.artist_detail.selected_panel == 0
                    && self.artist_detail.selected_track < self.artist_detail.top_tracks.len()
                {
                    Some(self.artist_detail.top_tracks[self.artist_detail.selected_track].clone())
                } else {
                    None
                }
            }
            ViewMode::Downloads => None,
        };

        if let Some(track) = track {
            if let Some(ref dm) = self.download_manager {
                match dm.queue_track(&track) {
                    Ok(_) => {
                        self.add_debug(format!("Queued download: {} - {}", track.artist, track.title));
                        self.refresh_download_list();
                    }
                    Err(e) => {
                        self.add_debug(format!("Failed to queue download: {}", e));
                    }
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn download_all_tracks(&mut self) {
        let tracks: Vec<Track> = match self.view_mode {
            ViewMode::Browse => self.tracks.clone(),
            ViewMode::Search => {
                if let Some(ref results) = self.search_results {
                    match self.search.tab {
                        SearchTab::Tracks => results.tracks.clone(),
                        _ => Vec::new(),
                    }
                } else {
                    Vec::new()
                }
            }
            ViewMode::Library => {
                if self.library.tab == LibraryTab::Tracks {
                    self.favorite_tracks.clone()
                } else {
                    Vec::new()
                }
            }
            ViewMode::AlbumDetail => self.album_detail.tracks.clone(),
            ViewMode::ArtistDetail => {
                if self.artist_detail.selected_panel == 0 {
                    self.artist_detail.top_tracks.clone()
                } else {
                    Vec::new()
                }
            }
            ViewMode::Downloads => Vec::new(),
        };

        if !tracks.is_empty() {
            if let Some(ref dm) = self.download_manager {
                match dm.queue_tracks(&tracks) {
                    Ok(count) => {
                        self.add_debug(format!("Queued {} tracks for download", count));
                        self.refresh_download_list();
                    }
                    Err(e) => {
                        self.add_debug(format!("Failed to queue downloads: {}", e));
                    }
                }
            }
        }
    }

    pub fn refresh_download_list(&mut self) {
        if let Some(ref dm) = self.download_manager {
            self.download_records = dm.get_all_downloads().unwrap_or_default();
            self.downloads.download_counts = dm.get_download_counts().unwrap_or((0, 0, 0));
            // Refresh cached synced playlist IDs
            self.downloads.synced_playlist_ids.clear();
            if let Ok(playlists) = dm.get_synced_playlists() {
                for playlist in playlists {
                    self.downloads.synced_playlist_ids.insert(playlist.playlist_id);
                }
            }
        }
    }

    pub fn delete_selected_download(&mut self) {
        if self.download_records.is_empty() {
            return;
        }

        let record = &self.download_records[self.downloads.selected];
        let track_id = record.track_id.clone();
        let title = record.title.clone();

        if let Some(ref dm) = self.download_manager {
            match dm.delete_download(&track_id) {
                Ok(_) => {
                    self.add_debug(format!("Deleted download: {}", title));
                    self.refresh_download_list();
                    if self.downloads.selected > 0 && self.downloads.selected >= self.download_records.len() {
                        self.downloads.selected = self.download_records.len().saturating_sub(1);
                    }
                }
                Err(e) => {
                    self.add_debug(format!("Failed to delete download: {}", e));
                }
            }
        }
    }

    pub fn retry_selected_download(&mut self) {
        if self.download_records.is_empty() {
            return;
        }

        let record = &self.download_records[self.downloads.selected];
        if record.status != DownloadStatus::Failed {
            return;
        }

        let track_id = record.track_id.clone();
        let title = record.title.clone();

        if let Some(ref dm) = self.download_manager {
            match dm.retry_failed(&track_id) {
                Ok(_) => {
                    self.add_debug(format!("Retrying download: {}", title));
                    self.refresh_download_list();
                }
                Err(e) => {
                    self.add_debug(format!("Failed to retry download: {}", e));
                }
            }
        }
    }

    pub fn toggle_download_pause(&mut self) {
        if let Some(ref mut dm) = self.download_manager {
            if dm.is_paused() {
                dm.resume();
                self.add_debug("Downloads resumed".to_string());
                self.set_status_info("Downloads resumed".to_string());
            } else {
                dm.pause();
                self.add_debug("Downloads paused".to_string());
                self.set_status_info("Downloads paused".to_string());
            }
        }
    }

    pub fn sync_selected_playlist(&mut self) {
        if self.view_mode != ViewMode::Browse || self.browse.selected_tab != 0 {
            self.add_debug("Select a playlist to sync (browse mode, playlists tab)".to_string());
            return;
        }

        if self.playlists.is_empty() || self.browse.selected_playlist >= self.playlists.len() {
            return;
        }

        let playlist = self.playlists[self.browse.selected_playlist].clone();
        let tracks = self.tracks.clone();

        if let Some(ref dm) = self.download_manager {
            match dm.sync_playlist(&playlist, &tracks) {
                Ok(new_count) => {
                    if new_count > 0 {
                        self.add_debug(format!(
                            "Synced playlist '{}': {} new tracks queued for download",
                            playlist.title, new_count
                        ));
                    } else {
                        self.add_debug(format!(
                            "Playlist '{}' already synced, no new tracks",
                            playlist.title
                        ));
                    }
                    self.refresh_download_list();
                }
                Err(e) => {
                    self.add_debug(format!("Failed to sync playlist: {}", e));
                }
            }
        }
    }

    pub async fn process_downloads(&mut self) {
        if let Some(ref dm) = self.download_manager {
            match dm.process_next_download(&mut self.music_service, &mut self.debug_log).await {
                Ok(processed) => {
                    if processed {
                        self.refresh_download_list();
                    }
                }
                Err(e) => {
                    self.add_debug(format!("Download error: {}", e));
                }
            }
        }
    }

    pub fn handle_download_events(&mut self) {
        let events: Vec<DownloadEvent> = if let Some(ref mut rx) = self.download_event_rx {
            let mut collected = Vec::new();
            while let Ok(event) = rx.try_recv() {
                collected.push(event);
            }
            collected
        } else {
            return;
        };

        let mut needs_refresh = false;
        for event in events {
            match event {
                DownloadEvent::Started { title, .. } => {
                    self.add_debug(format!("Started downloading: {}", title));
                }
                DownloadEvent::Completed { .. } => {
                    needs_refresh = true;
                }
                DownloadEvent::Failed { error, .. } => {
                    self.add_debug(format!("Download failed: {}", error));
                    needs_refresh = true;
                }
                DownloadEvent::QueueUpdated => {
                    needs_refresh = true;
                }
                DownloadEvent::Progress { .. } => {
                    needs_refresh = true;
                }
                DownloadEvent::PlaylistSynced { name, new_tracks, .. } => {
                    self.add_debug(format!(
                        "Playlist '{}' synced: {} new tracks queued",
                        name, new_tracks
                    ));
                    needs_refresh = true;
                }
            }
        }

        if needs_refresh {
            self.refresh_download_list();
        }
    }
}
