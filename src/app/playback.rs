use anyhow::Result;

use super::App;
use super::state::{RadioSeed, ViewMode};
use crate::tidal::Track;
use crate::ui::{SearchTab, LibraryTab};

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
        self.record_history(&track);
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
                match self.library.tab {
                    LibraryTab::Tracks if self.library.selected_track < self.favorite_tracks.len() => {
                        self.favorite_tracks[self.library.selected_track].clone()
                    }
                    LibraryTab::History if self.library.selected_history < self.history_entries.len() => {
                        crate::tidal::Track::from(&self.history_entries[self.library.selected_history])
                    }
                    _ => return Ok(()),
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

        // Check if we need to add radio tracks
        self.check_radio_queue().await;

        Ok(())
    }

    pub async fn check_radio_queue(&mut self) {
        // Skip if radio mode is off or we're already fetching
        if self.playback.radio_seed.is_none() || self.playback.radio_fetching {
            return;
        }

        // Check remaining tracks in queue
        let remaining = match self.mpd_controller.get_remaining_queue_count().await {
            Ok(count) => count,
            Err(e) => {
                self.add_debug(format!("Radio: failed to get queue count: {}", e));
                return;
            }
        };

        // Only fetch when queue runs low (2 or fewer remaining tracks)
        if remaining > 2 {
            return;
        }

        self.playback.radio_fetching = true;

        // Clone the seed to avoid borrow issues
        let radio_seed = match self.playback.radio_seed.clone() {
            Some(seed) => seed,
            None => {
                self.playback.radio_fetching = false;
                return;
            }
        };

        // Fetch radio tracks based on seed type
        let radio_tracks = match radio_seed {
            RadioSeed::Track(track_id) => {
                self.add_debug(format!("Radio: fetching similar tracks (seed track: {})", track_id));
                match self.tidal_client.get_track_radio(track_id, 10).await {
                    Ok(tracks) => tracks,
                    Err(e) => {
                        self.add_debug(format!("Radio: failed to fetch tracks: {}", e));
                        self.playback.radio_fetching = false;
                        return;
                    }
                }
            }
            RadioSeed::Playlist(playlist_id) => {
                self.add_debug(format!("Mix: fetching similar tracks (seed playlist: {})", playlist_id));
                match self.tidal_client.get_playlist_radio(&playlist_id, 10).await {
                    Ok(tracks) => tracks,
                    Err(e) => {
                        self.add_debug(format!("Mix: failed to fetch tracks: {}", e));
                        self.playback.radio_fetching = false;
                        return;
                    }
                }
            }
            RadioSeed::Artist(artist_id) => {
                self.add_debug(format!("Artist Radio: fetching similar tracks (artist: {})", artist_id));
                match self.tidal_client.get_artist_radio(artist_id, 10).await {
                    Ok(tracks) => tracks,
                    Err(e) => {
                        self.add_debug(format!("Artist Radio: failed to fetch tracks: {}", e));
                        self.playback.radio_fetching = false;
                        return;
                    }
                }
            }
            RadioSeed::Album(album_id) => {
                self.add_debug(format!("Album Radio: fetching similar tracks (album: {})", album_id));
                // Album radio fallback: get album tracks, seed from random track
                let album_tracks = match self.tidal_client.get_album_tracks(&album_id).await {
                    Ok(tracks) => tracks,
                    Err(e) => {
                        self.add_debug(format!("Album Radio: failed to get album tracks: {}", e));
                        self.playback.radio_fetching = false;
                        return;
                    }
                };
                if album_tracks.is_empty() {
                    self.add_debug("Album Radio: album has no tracks".to_string());
                    self.playback.radio_fetching = false;
                    return;
                }
                // Pick a random track from the album
                use rand::Rng;
                let idx = rand::thread_rng().gen_range(0..album_tracks.len());
                match self.tidal_client.get_track_radio(album_tracks[idx].id, 10).await {
                    Ok(tracks) => tracks,
                    Err(e) => {
                        self.add_debug(format!("Album Radio: failed to fetch tracks: {}", e));
                        self.playback.radio_fetching = false;
                        return;
                    }
                }
            }
        };

        if radio_tracks.is_empty() {
            self.add_debug("Radio: no similar tracks found".to_string());
            self.playback.radio_fetching = false;
            return;
        }

        // Filter out duplicates (tracks already in local_queue)
        let existing_ids: std::collections::HashSet<u64> = self.local_queue.iter().map(|t| t.id).collect();
        let new_tracks: Vec<_> = radio_tracks
            .into_iter()
            .filter(|t| !existing_ids.contains(&t.id))
            .collect();

        if new_tracks.is_empty() {
            self.add_debug("Radio: all tracks already in queue".to_string());
            self.playback.radio_fetching = false;
            return;
        }

        self.add_debug(format!("Radio: adding {} new tracks", new_tracks.len()));

        // Add tracks to queue
        let mut added = 0;
        for track in new_tracks {
            match self.tidal_client.get_stream_url(&track.id.to_string()).await {
                Ok(url) => {
                    if self.mpd_controller.add_track(&url, &mut self.debug_log).await.is_ok() {
                        self.local_queue.push(track);
                        added += 1;
                    }
                }
                Err(e) => {
                    self.add_debug(format!("Radio: failed to get URL: {}", e));
                }
            }
        }

        if added > 0 {
            self.add_debug(format!("Radio: added {} tracks to queue", added));
            self.playback.queue_dirty = true;
        }

        self.playback.radio_fetching = false;
    }
}
