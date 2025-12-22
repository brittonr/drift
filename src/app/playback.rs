use anyhow::Result;

use super::App;
use crate::tidal::Track;
use crate::ui::{SearchTab, LibraryTab};
use super::state::ViewMode;

impl App {
    pub async fn play_track(&mut self, track: Track) -> Result<()> {
        self.add_debug(format!("Playing: {} - {}", track.artist, track.title));

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

        self.add_debug("Clearing MPD queue...".to_string());
        if let Err(e) = self.mpd_controller.clear_queue(&mut self.debug_log).await {
            self.add_debug(format!("Clear failed: {}", e));
            return Err(e);
        }
        self.local_queue.clear();

        self.add_debug("Adding track to MPD...".to_string());
        if let Err(e) = self.mpd_controller.add_track(&stream_url, &mut self.debug_log).await {
            self.add_debug(format!("Add failed: {}", e));
            return Err(e);
        }

        self.add_debug("Starting playback...".to_string());
        if let Err(e) = self.mpd_controller.play(&mut self.debug_log).await {
            self.add_debug(format!("Play failed: {}", e));
            return Err(e);
        }

        self.playback.is_playing = true;
        self.current_track = Some(track);
        self.add_debug("Playback started".to_string());
        Ok(())
    }

    pub async fn play_selected_track(&mut self) -> Result<()> {
        let track = match self.view_mode {
            ViewMode::Browse => {
                if self.browse.selected_track < self.tracks.len() {
                    self.tracks[self.browse.selected_track].clone()
                } else {
                    return Ok(());
                }
            }
            ViewMode::Search => {
                if let Some(ref results) = self.search_results {
                    if self.search.tab == SearchTab::Tracks && self.search.selected_track < results.tracks.len() {
                        results.tracks[self.search.selected_track].clone()
                    } else {
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            }
            ViewMode::Library => {
                if self.library.tab == LibraryTab::Tracks && self.library.selected_track < self.favorite_tracks.len() {
                    self.favorite_tracks[self.library.selected_track].clone()
                } else {
                    return Ok(());
                }
            }
            ViewMode::Downloads | ViewMode::ArtistDetail | ViewMode::AlbumDetail => return Ok(()),
        };

        self.play_track(track).await
    }

    pub async fn toggle_playback(&mut self) -> Result<()> {
        if self.playback.is_playing {
            self.add_debug("Pausing playback...".to_string());
            self.mpd_controller.pause(&mut self.debug_log).await?;
        } else {
            self.add_debug("Resuming playback...".to_string());
            self.mpd_controller.play(&mut self.debug_log).await?;
        }
        self.playback.is_playing = !self.playback.is_playing;
        Ok(())
    }

    pub async fn check_mpd_status(&mut self) -> Result<()> {
        let status = self.mpd_controller.get_status(&mut self.debug_log).await?;

        if status.is_playing != self.playback.is_playing {
            self.playback.is_playing = status.is_playing;
            self.add_debug(format!("Playback state: {}", if self.playback.is_playing { "playing" } else { "paused" }));
        }

        if let Some(vol) = status.volume {
            self.playback.volume = vol;
        }
        self.playback.repeat_mode = status.repeat;
        self.playback.random_mode = status.random;
        self.playback.single_mode = status.single;

        if let Some(ref track) = self.current_track {
            match self.mpd_controller.get_timing_info().await {
                Ok((elapsed, duration)) => {
                    self.current_song = Some(crate::mpd::CurrentSong {
                        artist: track.artist.clone(),
                        title: track.title.clone(),
                        album: track.album.clone(),
                        elapsed,
                        duration,
                    });

                    if let Some(ref cover_id) = track.album_cover_id {
                        if !self.album_art_cache.has_cached(cover_id, 320) {
                            if let Err(e) = self.album_art_cache.get_album_art(cover_id, 320).await {
                                self.add_debug(format!("Failed to download album art: {}", e));
                            }
                        }
                    }
                }
                Err(e) => {
                    self.add_debug(format!("Failed to get timing info: {}", e));
                }
            }
        } else if self.current_song.is_some() {
            self.current_song = None;
        }

        if self.playback.show_queue {
            if let Ok(queue) = self.mpd_controller.get_queue().await {
                self.queue = queue;
            }
        }

        Ok(())
    }
}
