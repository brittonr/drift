use anyhow::Result;

use super::App;
use super::state::ViewMode;
use crate::queue_persistence::{self, PersistedQueue};
use crate::tidal::Track;
use crate::ui::search::SearchTab;

impl App {
    pub async fn save_queue_state(&mut self) {
        if self.local_queue.is_empty() {
            let persisted = PersistedQueue::new();
            if let Err(e) = queue_persistence::save_queue(&persisted) {
                self.add_debug(format!("Failed to save queue: {}", e));
            }
            return;
        }

        let (position, elapsed) = match self.mpd_controller.get_playback_position().await {
            Ok(Some((pos, elapsed))) => (Some(pos), Some(elapsed)),
            Ok(None) => (None, None),
            Err(_) => (None, None),
        };

        let persisted = PersistedQueue::from_tracks(
            &self.local_queue,
            position,
            elapsed,
        );

        match queue_persistence::save_queue(&persisted) {
            Ok(()) => {
                if let (Some(pos), Some(el)) = (position, elapsed) {
                    self.add_debug(format!("Saved {} tracks (pos {}, {}s)", self.local_queue.len(), pos + 1, el));
                } else {
                    self.add_debug(format!("Saved {} tracks to queue", self.local_queue.len()));
                }
            }
            Err(e) => {
                self.add_debug(format!("Failed to save queue: {}", e));
            }
        }
    }

    pub async fn restore_queue(&mut self, persisted: PersistedQueue) {
        if persisted.tracks.is_empty() {
            return;
        }

        self.add_debug(format!("Restoring {} tracks to MPD...", persisted.tracks.len()));

        if let Err(e) = self.mpd_controller.clear_queue(&mut self.debug_log).await {
            self.add_debug(format!("Failed to clear MPD queue: {}", e));
            return;
        }

        let mut added = 0;
        for pt in &persisted.tracks {
            let track = Track::from(pt);
            match self.tidal_client.get_stream_url(&track.id.to_string()).await {
                Ok(url) => {
                    if let Err(e) = self.mpd_controller.add_track(&url, &mut self.debug_log).await {
                        self.add_debug(format!("Failed to add track {}: {}", track.title, e));
                    } else {
                        added += 1;
                    }
                }
                Err(e) => {
                    self.add_debug(format!("Failed to get URL for {}: {}", track.title, e));
                }
            }
        }

        self.add_debug(format!("Restored {}/{} tracks to MPD", added, persisted.tracks.len()));

        if let Some(pos) = persisted.current_position {
            if pos < added {
                self.add_debug(format!("Resuming from track {}", pos + 1));
                if let Err(e) = self.mpd_controller.play_position(pos, &mut self.debug_log).await {
                    self.add_debug(format!("Failed to resume playback: {}", e));
                } else if let Some(elapsed) = persisted.elapsed_seconds {
                    if elapsed > 0 {
                        self.add_debug(format!("Seeking to {}s", elapsed));
                        if let Err(e) = self.mpd_controller.seek_to(elapsed, &mut self.debug_log).await {
                            self.add_debug(format!("Failed to seek: {}", e));
                        }
                    }
                }
            }
        }
    }

    pub async fn add_track_to_queue(&mut self, track: Track) -> Result<()> {
        self.add_debug(format!("Adding to queue: {} - {}", track.artist, track.title));

        self.add_debug(format!("Getting stream URL for track ID {}...", track.id));
        let stream_url = match self.tidal_client.get_stream_url(&track.id.to_string()).await {
            Ok(url) => {
                self.add_debug(format!("Got URL: {}...", &url[..50.min(url.len())]));
                url
            }
            Err(e) => {
                self.add_debug(format!("Failed to get URL: {}", e));
                return Err(e);
            }
        };

        self.add_debug("Adding to MPD queue...".to_string());
        if let Err(e) = self.mpd_controller.add_track(&stream_url, &mut self.debug_log).await {
            self.add_debug(format!("Add to MPD failed: {}", e));
            return Err(e);
        }

        self.add_debug(format!("Added to queue: {}", track.title));
        self.local_queue.push(track.clone());

        if let Ok(queue) = self.mpd_controller.get_queue().await {
            self.add_debug(format!("  Queue now has {} tracks", queue.len()));
            self.queue = queue;
        }

        if self.current_track.is_none() {
            self.current_track = Some(track);
        }

        let status = self.mpd_controller.get_status(&mut self.debug_log).await?;
        if !status.is_playing {
            self.add_debug("No playback detected, starting...".to_string());
            if let Err(e) = self.mpd_controller.play(&mut self.debug_log).await {
                self.add_debug(format!("Play failed: {}", e));
                return Err(e);
            }
            self.playback.is_playing = true;
        } else {
            self.add_debug("Playback already active, track queued".to_string());
        }

        Ok(())
    }

    pub async fn add_selected_track_to_queue(&mut self) -> Result<()> {
        let track = if self.view_mode == ViewMode::Browse {
            if self.browse.selected_track < self.tracks.len() {
                self.tracks[self.browse.selected_track].clone()
            } else {
                return Ok(());
            }
        } else {
            if let Some(ref results) = self.search_results {
                if self.search.tab == SearchTab::Tracks && self.search.selected_track < results.tracks.len() {
                    results.tracks[self.search.selected_track].clone()
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        };

        self.add_track_to_queue(track).await
    }

    pub async fn add_all_tracks_to_queue(&mut self) -> Result<()> {
        let tracks_to_add = if self.view_mode == ViewMode::Browse {
            if self.browse.selected_tab == 1 && !self.tracks.is_empty() {
                self.tracks.clone()
            } else {
                return Ok(());
            }
        } else {
            if let Some(ref results) = self.search_results {
                if self.search.tab == SearchTab::Tracks && !results.tracks.is_empty() {
                    results.tracks.clone()
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        };

        self.add_debug(format!("Adding {} tracks to queue...", tracks_to_add.len()));

        let was_playing = self.mpd_controller.get_status(&mut self.debug_log).await?.is_playing;

        let mut added_count = 0;
        for (i, track) in tracks_to_add.iter().enumerate() {
            self.add_debug(format!("[{}/{}] {} - {}", i+1, tracks_to_add.len(), track.artist, track.title));

            match self.tidal_client.get_stream_url(&track.id.to_string()).await {
                Ok(url) => {
                    if let Err(e) = self.mpd_controller.add_track(&url, &mut self.debug_log).await {
                        self.add_debug(format!("  Failed to add: {}", e));
                    } else {
                        self.local_queue.push(track.clone());
                        added_count += 1;
                    }
                }
                Err(e) => {
                    self.add_debug(format!("  Failed to get URL: {}", e));
                }
            }
        }

        self.add_debug(format!("Added {}/{} tracks to queue", added_count, tracks_to_add.len()));

        if let Ok(queue) = self.mpd_controller.get_queue().await {
            self.queue = queue;
            self.add_debug(format!("Queue now has {} total tracks", self.queue.len()));
        }

        if !was_playing && added_count > 0 {
            self.add_debug("Starting playback...".to_string());
            if let Err(e) = self.mpd_controller.play(&mut self.debug_log).await {
                self.add_debug(format!("Play failed: {}", e));
            } else {
                self.playback.is_playing = true;
            }
        }

        Ok(())
    }

    pub async fn add_album_to_queue(&mut self) -> Result<()> {
        let album = if let Some(ref results) = self.search_results {
            if self.search.tab == SearchTab::Albums && self.search.selected_album < results.albums.len() {
                results.albums[self.search.selected_album].clone()
            } else {
                return Ok(());
            }
        } else {
            return Ok(());
        };

        self.add_debug(format!("Fetching tracks for album: {} - {}", album.artist, album.title));

        let tracks = match self.tidal_client.get_album_tracks(&album.id).await {
            Ok(t) => t,
            Err(e) => {
                self.add_debug(format!("Failed to get album tracks: {}", e));
                return Ok(());
            }
        };

        if tracks.is_empty() {
            self.add_debug("No tracks found for album".to_string());
            return Ok(());
        }

        self.add_debug(format!("Adding {} tracks from album...", tracks.len()));

        let was_playing = self.mpd_controller.get_status(&mut self.debug_log).await?.is_playing;
        let mut added_count = 0;

        for track in &tracks {
            match self.tidal_client.get_stream_url(&track.id.to_string()).await {
                Ok(url) => {
                    if let Err(e) = self.mpd_controller.add_track(&url, &mut self.debug_log).await {
                        self.add_debug(format!("Failed to add {}: {}", track.title, e));
                    } else {
                        self.local_queue.push(track.clone());
                        added_count += 1;
                    }
                }
                Err(e) => {
                    self.add_debug(format!("Failed to get URL for {}: {}", track.title, e));
                }
            }
        }

        self.add_debug(format!("Added {}/{} tracks from album", added_count, tracks.len()));

        if let Ok(queue) = self.mpd_controller.get_queue().await {
            self.queue = queue;
        }

        if !was_playing && added_count > 0 {
            if let Err(e) = self.mpd_controller.play(&mut self.debug_log).await {
                self.add_debug(format!("Play failed: {}", e));
            } else {
                self.playback.is_playing = true;
            }
        }

        Ok(())
    }

    /// Add album tracks to queue by album ID (used from detail views)
    pub async fn add_album_by_id(&mut self, album_id: &str) -> Result<()> {
        let tracks = match self.tidal_client.get_album_tracks(album_id).await {
            Ok(t) => t,
            Err(e) => {
                self.add_debug(format!("Failed to get album tracks: {}", e));
                return Ok(());
            }
        };

        if tracks.is_empty() {
            self.add_debug("No tracks found for album".to_string());
            return Ok(());
        }

        self.add_debug(format!("Adding {} tracks from album...", tracks.len()));

        let was_playing = self.mpd_controller.get_status(&mut self.debug_log).await?.is_playing;
        let mut added_count = 0;

        for track in &tracks {
            match self.tidal_client.get_stream_url(&track.id.to_string()).await {
                Ok(url) => {
                    if let Err(e) = self.mpd_controller.add_track(&url, &mut self.debug_log).await {
                        self.add_debug(format!("Failed to add {}: {}", track.title, e));
                    } else {
                        self.local_queue.push(track.clone());
                        added_count += 1;
                    }
                }
                Err(e) => {
                    self.add_debug(format!("Failed to get URL for {}: {}", track.title, e));
                }
            }
        }

        self.add_debug(format!("Added {}/{} tracks from album", added_count, tracks.len()));

        if let Ok(queue) = self.mpd_controller.get_queue().await {
            self.queue = queue;
        }

        if !was_playing && added_count > 0 {
            if let Err(e) = self.mpd_controller.play(&mut self.debug_log).await {
                self.add_debug(format!("Play failed: {}", e));
            } else {
                self.playback.is_playing = true;
            }
        }

        Ok(())
    }

    /// Add all tracks from album detail view to queue
    pub async fn add_album_detail_tracks_to_queue(&mut self) -> Result<()> {
        if self.album_detail.tracks.is_empty() {
            self.add_debug("No tracks in album detail".to_string());
            return Ok(());
        }

        let tracks = self.album_detail.tracks.clone();
        self.add_debug(format!("Adding {} tracks from album...", tracks.len()));

        let was_playing = self.mpd_controller.get_status(&mut self.debug_log).await?.is_playing;
        let mut added_count = 0;

        for track in &tracks {
            match self.tidal_client.get_stream_url(&track.id.to_string()).await {
                Ok(url) => {
                    if let Err(e) = self.mpd_controller.add_track(&url, &mut self.debug_log).await {
                        self.add_debug(format!("Failed to add {}: {}", track.title, e));
                    } else {
                        self.local_queue.push(track.clone());
                        added_count += 1;
                    }
                }
                Err(e) => {
                    self.add_debug(format!("Failed to get URL for {}: {}", track.title, e));
                }
            }
        }

        self.add_debug(format!("Added {}/{} tracks", added_count, tracks.len()));

        if let Ok(queue) = self.mpd_controller.get_queue().await {
            self.queue = queue;
        }

        if !was_playing && added_count > 0 {
            if let Err(e) = self.mpd_controller.play(&mut self.debug_log).await {
                self.add_debug(format!("Play failed: {}", e));
            } else {
                self.playback.is_playing = true;
            }
        }

        Ok(())
    }

    pub async fn add_artist_to_queue(&mut self) -> Result<()> {
        let artist = if let Some(ref results) = self.search_results {
            if self.search.tab == SearchTab::Artists && self.search.selected_artist < results.artists.len() {
                results.artists[self.search.selected_artist].clone()
            } else {
                return Ok(());
            }
        } else {
            return Ok(());
        };

        self.add_debug(format!("Fetching top tracks for artist: {}", artist.name));

        let tracks = match self.tidal_client.get_artist_top_tracks(artist.id).await {
            Ok(t) => t,
            Err(e) => {
                self.add_debug(format!("Failed to get artist tracks: {}", e));
                return Ok(());
            }
        };

        if tracks.is_empty() {
            self.add_debug("No tracks found for artist".to_string());
            return Ok(());
        }

        self.add_debug(format!("Adding {} top tracks from artist...", tracks.len()));

        let was_playing = self.mpd_controller.get_status(&mut self.debug_log).await?.is_playing;
        let mut added_count = 0;

        for track in &tracks {
            match self.tidal_client.get_stream_url(&track.id.to_string()).await {
                Ok(url) => {
                    if let Err(e) = self.mpd_controller.add_track(&url, &mut self.debug_log).await {
                        self.add_debug(format!("Failed to add {}: {}", track.title, e));
                    } else {
                        self.local_queue.push(track.clone());
                        added_count += 1;
                    }
                }
                Err(e) => {
                    self.add_debug(format!("Failed to get URL for {}: {}", track.title, e));
                }
            }
        }

        self.add_debug(format!("Added {}/{} top tracks from artist", added_count, tracks.len()));

        if let Ok(queue) = self.mpd_controller.get_queue().await {
            self.queue = queue;
        }

        if !was_playing && added_count > 0 {
            if let Err(e) = self.mpd_controller.play(&mut self.debug_log).await {
                self.add_debug(format!("Play failed: {}", e));
            } else {
                self.playback.is_playing = true;
            }
        }

        Ok(())
    }
}
