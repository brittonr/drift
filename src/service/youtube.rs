use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use super::youtube_storage::{SavedPlaylist, StoredTrack, YouTubeStorage};
use super::{Album, Artist, CoverArt, MusicService, Playlist, SearchResults, ServiceType, Track};

/// YouTube Music client using yt-dlp for search and stream extraction
pub struct YouTubeClient {
    ytdlp_path: PathBuf,
    audio_quality: String,
}

impl YouTubeClient {
    /// Create a new YouTube client
    ///
    /// This will error if yt-dlp is not found in PATH or at the configured path.
    pub async fn new(ytdlp_path: Option<&str>) -> Result<Self> {
        let path = ytdlp_path.unwrap_or("yt-dlp");

        // Verify yt-dlp exists and is executable
        let check = Command::new(path)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        match check {
            Ok(status) if status.success() => Ok(Self {
                ytdlp_path: PathBuf::from(path),
                audio_quality: "bestaudio".to_string(),
            }),
            Ok(_) => Err(anyhow!(
                "yt-dlp found but returned error. Please ensure yt-dlp is properly installed."
            )),
            Err(_) => Err(anyhow!(
                "yt-dlp not found at '{}'. YouTube Music requires yt-dlp to be installed.\n\
                 Install via: nix-shell -p yt-dlp, brew install yt-dlp, or pip install yt-dlp",
                path
            )),
        }
    }

    // === yt-dlp Helper Methods ===

    /// Execute yt-dlp and return JSON output as parsed values
    async fn run_ytdlp_json(&self, args: &[&str]) -> Result<Vec<Value>> {
        let output = Command::new(&self.ytdlp_path)
            .args(args)
            .args(["--no-warnings"])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("yt-dlp failed: {}", stderr));
        }

        let stdout = String::from_utf8(output.stdout)?;
        Ok(stdout
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect())
    }

    /// Get playlist/search metadata (flat, no download)
    async fn get_playlist_info(&self, url: &str) -> Result<Vec<Value>> {
        self.run_ytdlp_json(&["--flat-playlist", "-j", url]).await
    }

    /// Get full video info including related videos
    async fn get_video_info(&self, video_id: &str) -> Result<Value> {
        let url = format!("https://www.youtube.com/watch?v={}", video_id);
        let results = self.run_ytdlp_json(&["-j", &url]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("No video info returned"))
    }

    /// Parse a JSON value into a Track
    fn parse_track(json: &Value) -> Option<Track> {
        let id = json.get("id")?.as_str()?.to_string();
        let title = json.get("title")?.as_str()?.to_string();

        let artist = json
            .get("uploader")
            .or_else(|| json.get("channel"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        let duration = json
            .get("duration")
            .and_then(|v| v.as_f64())
            .map(|d| d as u32)
            .unwrap_or(0);

        let thumbnail = Self::extract_thumbnail(json);

        Some(Track {
            id,
            title,
            artist,
            album: String::new(), // YouTube videos don't have albums
            duration_seconds: duration,
            cover_art: thumbnail.map(CoverArt::Url).unwrap_or(CoverArt::None),
            service: ServiceType::YouTube,
        })
    }

    /// Extract best thumbnail URL from JSON
    fn extract_thumbnail(json: &Value) -> Option<String> {
        json.get("thumbnail")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                json.get("thumbnails")
                    .and_then(|t| t.as_array())
                    .and_then(|arr| arr.last())
                    .and_then(|t| t.get("url"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
    }

    /// Extract channel ID from JSON
    fn extract_channel_id(json: &Value) -> Option<String> {
        json.get("channel_id")
            .or_else(|| json.get("uploader_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// Convert StoredTrack to Track
    fn stored_to_track(stored: &StoredTrack) -> Track {
        Track {
            id: stored.id.clone(),
            title: stored.title.clone(),
            artist: stored.channel_name.clone(),
            album: String::new(),
            duration_seconds: stored.duration_seconds,
            cover_art: stored
                .thumbnail_url
                .clone()
                .map(CoverArt::Url)
                .unwrap_or(CoverArt::None),
            service: ServiceType::YouTube,
        }
    }

    /// Convert SavedPlaylist to Playlist
    fn saved_to_playlist(saved: &SavedPlaylist) -> Playlist {
        Playlist {
            id: saved.id.clone(),
            title: saved.title.clone(),
            description: saved.description.clone(),
            num_tracks: saved.num_tracks,
            service: ServiceType::YouTube,
        }
    }
}

#[async_trait]
impl MusicService for YouTubeClient {
    fn service_type(&self) -> ServiceType {
        ServiceType::YouTube
    }

    fn is_authenticated(&self) -> bool {
        // Unauthenticated mode - always false
        false
    }

    fn set_audio_quality(&mut self, quality: &str) {
        self.audio_quality = match quality.to_lowercase().as_str() {
            "low" => "worstaudio",
            "high" | "lossless" | "master" => "bestaudio",
            _ => "bestaudio",
        }
        .to_string();
    }

    async fn get_stream_url(&mut self, track_id: &str) -> Result<String> {
        let url = format!("https://www.youtube.com/watch?v={}", track_id);

        let output = Command::new(&self.ytdlp_path)
            .args([
                "-f",
                &self.audio_quality,
                "-g", // Get URL only, don't download
                "--no-warnings",
                &url,
            ])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("yt-dlp failed to extract stream URL: {}", stderr));
        }

        let stream_url = String::from_utf8(output.stdout)?
            .trim()
            .lines()
            .next()
            .ok_or_else(|| anyhow!("yt-dlp returned empty output"))?
            .to_string();

        Ok(stream_url)
    }

    // === Library: Playlists ===

    async fn get_playlists(&mut self) -> Result<Vec<Playlist>> {
        let storage = YouTubeStorage::load().unwrap_or_default();
        Ok(storage
            .saved_playlists
            .iter()
            .map(Self::saved_to_playlist)
            .collect())
    }

    async fn get_playlist_tracks(&mut self, playlist_id: &str) -> Result<Vec<Track>> {
        let storage = YouTubeStorage::load().unwrap_or_default();

        // Check if it's a local playlist
        if let Some(saved) = storage.saved_playlists.iter().find(|p| p.id == playlist_id) {
            if saved.is_user_created {
                // Local playlist - get tracks from storage
                let track_ids = storage.get_local_playlist_tracks(playlist_id);
                let mut tracks = Vec::new();

                for track_id in track_ids {
                    // Try to find in favorites first (cached metadata)
                    if let Some(stored) = storage.favorite_tracks.iter().find(|t| t.id == track_id) {
                        tracks.push(Self::stored_to_track(stored));
                    } else {
                        // Fetch info for tracks not in favorites
                        if let Ok(info) = self.get_video_info(&track_id).await {
                            if let Some(track) = Self::parse_track(&info) {
                                tracks.push(track);
                            }
                        }
                    }
                }
                return Ok(tracks);
            }
        }

        // YouTube playlist - fetch via yt-dlp
        let url = format!("https://www.youtube.com/playlist?list={}", playlist_id);
        let results = self.get_playlist_info(&url).await?;

        Ok(results.iter().filter_map(Self::parse_track).collect())
    }

    // === Library: Favorites ===

    async fn get_favorite_tracks(&mut self) -> Result<Vec<Track>> {
        let storage = YouTubeStorage::load().unwrap_or_default();
        Ok(storage
            .favorite_tracks
            .iter()
            .map(Self::stored_to_track)
            .collect())
    }

    async fn get_favorite_albums(&mut self) -> Result<Vec<Album>> {
        // YouTube doesn't have albums in the traditional sense
        // Could potentially return saved playlists as "albums"
        Ok(vec![])
    }

    async fn get_favorite_artists(&mut self) -> Result<Vec<Artist>> {
        let storage = YouTubeStorage::load().unwrap_or_default();
        Ok(storage
            .favorite_channels
            .iter()
            .map(|c| Artist {
                id: c.id.clone(),
                name: c.name.clone(),
                service: ServiceType::YouTube,
            })
            .collect())
    }

    async fn add_favorite_track(&mut self, track_id: &str) -> Result<()> {
        // Fetch track info to store complete metadata
        let info = self.get_video_info(track_id).await?;

        let stored = StoredTrack {
            id: track_id.to_string(),
            title: info
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string(),
            channel_id: Self::extract_channel_id(&info).unwrap_or_default(),
            channel_name: info
                .get("uploader")
                .or_else(|| info.get("channel"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string(),
            duration_seconds: info
                .get("duration")
                .and_then(|v| v.as_f64())
                .map(|d| d as u32)
                .unwrap_or(0),
            thumbnail_url: Self::extract_thumbnail(&info),
            added_at: Utc::now(),
        };

        let mut storage = YouTubeStorage::load().unwrap_or_default();
        storage.add_favorite_track(stored);
        storage.save()?;

        Ok(())
    }

    async fn remove_favorite_track(&mut self, track_id: &str) -> Result<()> {
        let mut storage = YouTubeStorage::load().unwrap_or_default();
        storage.remove_favorite_track(track_id);
        storage.save()?;
        Ok(())
    }

    // === Search ===

    async fn search(&mut self, query: &str, limit: usize) -> Result<SearchResults> {
        // Track search
        let track_query = format!("ytsearch{}:{}", limit, query);
        let track_results = self.get_playlist_info(&track_query).await.unwrap_or_default();

        let tracks: Vec<Track> = track_results
            .iter()
            .filter_map(Self::parse_track)
            .collect();

        // Extract unique channels from track results as "artists"
        let mut seen_channels = HashSet::new();
        let artists: Vec<Artist> = track_results
            .iter()
            .filter_map(|json| {
                let channel_id = Self::extract_channel_id(json)?;
                let channel_name = json
                    .get("uploader")
                    .or_else(|| json.get("channel"))
                    .and_then(|v| v.as_str())?
                    .to_string();

                if seen_channels.insert(channel_id.clone()) {
                    Some(Artist {
                        id: channel_id,
                        name: channel_name,
                        service: ServiceType::YouTube,
                    })
                } else {
                    None
                }
            })
            .take(limit.min(10))
            .collect();

        // Playlist search (as "albums")
        let playlist_query = format!("ytsearch{}:{} playlist", limit.min(5), query);
        let playlist_results = self
            .get_playlist_info(&playlist_query)
            .await
            .unwrap_or_default();

        let albums: Vec<Album> = playlist_results
            .iter()
            .filter_map(|json| {
                let id = json
                    .get("playlist_id")
                    .or_else(|| json.get("id"))
                    .and_then(|v| v.as_str())?
                    .to_string();

                // Only include if it looks like a playlist
                if !id.starts_with("PL") && !id.starts_with("UU") && !id.starts_with("OL") {
                    return None;
                }

                let title = json
                    .get("playlist_title")
                    .or_else(|| json.get("title"))
                    .and_then(|v| v.as_str())?
                    .to_string();

                let uploader = json
                    .get("playlist_uploader")
                    .or_else(|| json.get("uploader"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("YouTube")
                    .to_string();

                let count = json
                    .get("playlist_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;

                let thumbnail = Self::extract_thumbnail(json);

                Some(Album {
                    id,
                    title,
                    artist: uploader,
                    num_tracks: count,
                    cover_art: thumbnail.map(CoverArt::Url).unwrap_or(CoverArt::None),
                    service: ServiceType::YouTube,
                })
            })
            .collect();

        Ok(SearchResults {
            tracks,
            albums,
            artists,
        })
    }

    // === Album/Artist Details ===

    async fn get_album_tracks(&mut self, album_id: &str) -> Result<Vec<Track>> {
        // Albums are playlists on YouTube
        let url = format!("https://www.youtube.com/playlist?list={}", album_id);
        let results = self.get_playlist_info(&url).await?;
        Ok(results.iter().filter_map(Self::parse_track).collect())
    }

    async fn get_artist_top_tracks(&mut self, artist_id: &str) -> Result<Vec<Track>> {
        // Use channel URL with popular sort
        let channel_url = if artist_id.starts_with("UC") || artist_id.starts_with("@") {
            if artist_id.starts_with("@") {
                format!("https://www.youtube.com/{}/videos", artist_id)
            } else {
                format!("https://www.youtube.com/channel/{}/videos", artist_id)
            }
        } else {
            // Assume it's a channel name/handle
            format!("https://www.youtube.com/@{}/videos", artist_id)
        };

        let output = Command::new(&self.ytdlp_path)
            .args([
                "--flat-playlist",
                "-j",
                "--no-warnings",
                "--playlist-end",
                "20",
                &channel_url,
            ])
            .output()
            .await?;

        if !output.status.success() {
            return Ok(vec![]);
        }

        let tracks: Vec<Track> = String::from_utf8(output.stdout)?
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .filter_map(|json: Value| Self::parse_track(&json))
            .collect();

        Ok(tracks)
    }

    async fn get_artist_albums(&mut self, artist_id: &str) -> Result<Vec<Album>> {
        // Get playlists from channel
        let playlists_url = if artist_id.starts_with("UC") || artist_id.starts_with("@") {
            if artist_id.starts_with("@") {
                format!("https://www.youtube.com/{}/playlists", artist_id)
            } else {
                format!("https://www.youtube.com/channel/{}/playlists", artist_id)
            }
        } else {
            format!("https://www.youtube.com/@{}/playlists", artist_id)
        };

        let output = Command::new(&self.ytdlp_path)
            .args([
                "--flat-playlist",
                "-j",
                "--no-warnings",
                "--playlist-end",
                "20",
                &playlists_url,
            ])
            .output()
            .await?;

        if !output.status.success() {
            return Ok(vec![]);
        }

        let albums: Vec<Album> = String::from_utf8(output.stdout)?
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .filter_map(|json: Value| {
                let id = json.get("id").and_then(|v| v.as_str())?.to_string();
                let title = json.get("title").and_then(|v| v.as_str())?.to_string();
                let count = json
                    .get("playlist_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;

                Some(Album {
                    id,
                    title,
                    artist: artist_id.to_string(),
                    num_tracks: count,
                    cover_art: Self::extract_thumbnail(&json)
                        .map(CoverArt::Url)
                        .unwrap_or(CoverArt::None),
                    service: ServiceType::YouTube,
                })
            })
            .collect();

        Ok(albums)
    }

    // === Radio/Recommendations ===

    async fn get_track_radio(&mut self, track_id: &str, limit: usize) -> Result<Vec<Track>> {
        // Strategy 1: Use YouTube Mix playlist (RD prefix)
        let mix_id = format!("RD{}", track_id);
        let mix_url = format!("https://www.youtube.com/playlist?list={}", mix_id);

        if let Ok(results) = self.get_playlist_info(&mix_url).await {
            if !results.is_empty() {
                let tracks: Vec<Track> = results
                    .iter()
                    .filter_map(Self::parse_track)
                    .filter(|t| t.id != track_id) // Exclude seed track
                    .take(limit)
                    .collect();

                if !tracks.is_empty() {
                    return Ok(tracks);
                }
            }
        }

        // Strategy 2: Search for similar content using the track title
        if let Ok(info) = self.get_video_info(track_id).await {
            let title = info
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("music");

            let search_results = self.search(title, limit + 5).await?;
            let tracks: Vec<Track> = search_results
                .tracks
                .into_iter()
                .filter(|t| t.id != track_id)
                .take(limit)
                .collect();

            return Ok(tracks);
        }

        Ok(vec![])
    }

    async fn get_artist_radio(&mut self, artist_id: &str, limit: usize) -> Result<Vec<Track>> {
        // Get top tracks from the artist as a simple "radio"
        let top_tracks = self.get_artist_top_tracks(artist_id).await?;
        if !top_tracks.is_empty() {
            return Ok(top_tracks.into_iter().take(limit).collect());
        }

        // Fallback: search for content by the artist
        let query = format!("{} music", artist_id);
        let results = self.search(&query, limit).await?;
        Ok(results.tracks)
    }

    async fn get_playlist_radio(&mut self, playlist_id: &str, limit: usize) -> Result<Vec<Track>> {
        // Get first track from playlist and use its radio
        let tracks = self.get_playlist_tracks(playlist_id).await?;

        if let Some(first_track) = tracks.first() {
            return self.get_track_radio(&first_track.id, limit).await;
        }

        Ok(vec![])
    }

    // === Playlist Management ===

    async fn create_playlist(&mut self, name: &str, description: Option<&str>) -> Result<Playlist> {
        let mut storage = YouTubeStorage::load().unwrap_or_default();
        let saved = storage.create_local_playlist(name, description);
        storage.save()?;

        Ok(Self::saved_to_playlist(&saved))
    }

    async fn update_playlist(
        &mut self,
        playlist_id: &str,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        let mut storage = YouTubeStorage::load()?;

        // Check if it's a local playlist
        let is_local = storage
            .saved_playlists
            .iter()
            .any(|p| p.id == playlist_id && p.is_user_created);

        if !is_local {
            return Err(anyhow!(
                "Cannot modify YouTube playlists, only local playlists"
            ));
        }

        if storage.update_playlist(playlist_id, title, description) {
            storage.save()?;
            Ok(())
        } else {
            Err(anyhow!("Playlist not found or cannot be modified"))
        }
    }

    async fn delete_playlist(&mut self, playlist_id: &str) -> Result<()> {
        let mut storage = YouTubeStorage::load()?;

        // Check if it's a local playlist
        let is_local = storage
            .saved_playlists
            .iter()
            .any(|p| p.id == playlist_id && p.is_user_created);

        if !is_local {
            return Err(anyhow!(
                "Cannot delete YouTube playlists, only local playlists"
            ));
        }

        storage.remove_saved_playlist(playlist_id);
        storage.save()?;
        Ok(())
    }

    async fn add_tracks_to_playlist(
        &mut self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<()> {
        let mut storage = YouTubeStorage::load()?;

        // Check if it's a local playlist
        let is_local = storage
            .saved_playlists
            .iter()
            .any(|p| p.id == playlist_id && p.is_user_created);

        if !is_local {
            return Err(anyhow!(
                "Cannot add tracks to YouTube playlists, only local playlists"
            ));
        }

        if storage.add_tracks_to_local_playlist(playlist_id, track_ids) {
            storage.save()?;
            Ok(())
        } else {
            Err(anyhow!("Playlist not found"))
        }
    }

    async fn remove_tracks_from_playlist(
        &mut self,
        playlist_id: &str,
        indices: &[usize],
    ) -> Result<()> {
        let mut storage = YouTubeStorage::load()?;

        // Check if it's a local playlist
        let is_local = storage
            .saved_playlists
            .iter()
            .any(|p| p.id == playlist_id && p.is_user_created);

        if !is_local {
            return Err(anyhow!(
                "Cannot remove tracks from YouTube playlists, only local playlists"
            ));
        }

        if storage.remove_tracks_from_local_playlist(playlist_id, indices) {
            storage.save()?;
            Ok(())
        } else {
            Err(anyhow!("Playlist not found"))
        }
    }

    // === Cover Art ===

    fn get_cover_url(&self, cover: &CoverArt, _size: u32) -> Option<String> {
        match cover {
            CoverArt::Url(url) => Some(url.clone()),
            CoverArt::ServiceId { .. } => None, // YouTube uses direct URLs
            CoverArt::None => None,
        }
    }
}
