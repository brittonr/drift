use super::App;
use super::state::ViewMode;
use crate::download_db::DownloadStatus;
use crate::downloads::{DownloadEvent, sanitize_filename};
use crate::service::{MusicService, Track};
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

    /// Periodically re-check synced playlists for new tracks.
    ///
    /// Fetches current track lists from the API for each synced playlist
    /// and queues any new tracks that weren't there last time.
    pub async fn auto_sync_playlists(&mut self) {
        let interval_mins = self.config.downloads.sync_interval_minutes;
        if interval_mins == 0 {
            return; // Disabled
        }

        let interval = std::time::Duration::from_secs(interval_mins * 60);
        if self.last_playlist_sync.elapsed() < interval {
            return; // Not time yet
        }

        self.last_playlist_sync = std::time::Instant::now();

        // Get the list of synced playlist IDs
        let synced_ids: Vec<String> = self.downloads.synced_playlist_ids.iter().cloned().collect();
        if synced_ids.is_empty() {
            return;
        }

        self.add_debug(format!("Auto-sync: checking {} synced playlist(s) for new tracks", synced_ids.len()));

        let mut total_new = 0usize;
        for playlist_id in &synced_ids {
            // Find the playlist metadata (title) from our loaded playlists
            let playlist = self.playlists.iter().find(|p| &p.id == playlist_id).cloned();
            let playlist = match playlist {
                Some(p) => p,
                None => {
                    // Playlist not in our current list — skip (user may have removed it)
                    continue;
                }
            };

            // Fetch current tracks from the service
            match self.music_service.get_playlist_tracks(playlist_id).await {
                Ok(tracks) => {
                    if let Some(ref dm) = self.download_manager {
                        match dm.sync_playlist(&playlist, &tracks) {
                            Ok(new_count) => {
                                if new_count > 0 {
                                    self.add_debug(format!(
                                        "Auto-sync '{}': {} new track(s) queued",
                                        playlist.title, new_count
                                    ));
                                    total_new += new_count;
                                }
                            }
                            Err(e) => {
                                self.add_debug(format!(
                                    "Auto-sync '{}' failed: {}",
                                    playlist.title, e
                                ));
                            }
                        }
                    }
                }
                Err(e) => {
                    self.add_debug(format!(
                        "Auto-sync: failed to fetch tracks for '{}': {}",
                        playlist.title, e
                    ));
                }
            }
        }

        if total_new > 0 {
            self.refresh_download_list();
            self.set_status_info(format!("Auto-sync: {} new track(s) queued for download", total_new));
        }
    }

    pub async fn process_downloads(&mut self) {
        // Try to satisfy pending downloads from the cluster blob store first
        if self.download_manager.is_some() {
            let pending = self.download_manager.as_ref().unwrap()
                .get_pending_downloads().unwrap_or_default();
            if let Some(record) = pending.first() {
                let record = record.clone();
                if let Some(path) = self.try_blob_download(&record).await {
                    self.add_debug(format!("Downloaded from cluster: {} → {}", record.title, path));
                    self.refresh_download_list();
                    return; // One at a time, check next tick
                }
            }
        }

        // Fall back to downloading from the music service
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
                DownloadEvent::BlobUploadReady { track_id, file_path } => {
                    self.pending_blob_uploads.push((track_id, file_path));
                }
            }
        }

        if needs_refresh {
            self.refresh_download_list();
        }
    }

    /// Upload completed downloads to the distributed blob store.
    ///
    /// Drains `pending_blob_uploads` and fires off uploads one at a time.
    /// Failures are logged but don't affect local functionality.
    pub async fn process_blob_uploads(&mut self) {
        if self.pending_blob_uploads.is_empty() {
            return;
        }

        let uploads: Vec<(String, String)> = std::mem::take(&mut self.pending_blob_uploads);
        for (track_id, file_path) in uploads {
            match self.storage.upload_blob(&track_id, &file_path).await {
                Ok(Some(hash)) => {
                    self.add_debug(format!("Uploaded to cluster: {} ({})", track_id, &hash[..12]));
                }
                Ok(None) => {
                    // Blob storage unavailable (local backend or disconnected), silently skip
                }
                Err(e) => {
                    self.add_debug(format!("Blob upload failed for {}: {}", track_id, e));
                }
            }
        }
    }

    /// Try to download a track from the blob store instead of the music service.
    ///
    /// Returns the local file path if the blob was found and written to disk.
    pub async fn try_blob_download(&mut self, record: &crate::download_db::DownloadRecord) -> Option<String> {
        // Check if this track exists in the cluster
        let blob_ref = match self.storage.has_blob(&record.track_id).await {
            Ok(Some(r)) => r,
            Ok(None) => return None,
            Err(e) => {
                self.add_debug(format!("Blob check failed for {}: {}", record.track_id, e));
                return None;
            }
        };

        self.add_debug(format!(
            "Found {} in cluster ({}, {:.1} MB), fetching...",
            record.title,
            blob_ref.format,
            blob_ref.size as f64 / (1024.0 * 1024.0),
        ));

        // Fetch the blob data
        let data = match self.storage.fetch_blob(&record.track_id).await {
            Ok(Some(d)) => d,
            Ok(None) => {
                self.add_debug(format!("Blob indexed but not available for {}", record.title));
                return None;
            }
            Err(e) => {
                self.add_debug(format!("Blob fetch failed for {}: {}", record.title, e));
                return None;
            }
        };

        // Write to the download path
        let dm = self.download_manager.as_ref()?;
        let track = Track::from(record);
        let download_dir = dm.get_download_dir_path();
        let artist = sanitize_filename(&track.artist);
        let album = sanitize_filename(&track.album);
        let title = sanitize_filename(&track.title);
        let file_path = download_dir
            .join(&artist)
            .join(&album)
            .join(format!("{}.{}", title, blob_ref.format));

        if let Some(parent) = file_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                self.add_debug(format!("Failed to create dir for blob: {}", e));
                return None;
            }
        }

        if let Err(e) = std::fs::write(&file_path, &data) {
            self.add_debug(format!("Failed to write blob to disk: {}", e));
            return None;
        }

        // Mark as completed in the download database
        let path_str = file_path.to_string_lossy().to_string();
        if let Err(e) = dm.mark_download_completed(&record.track_id, &path_str) {
            self.add_debug(format!("Failed to mark blob download complete: {}", e));
            return None;
        }

        Some(path_str)
    }
}
