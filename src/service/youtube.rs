use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use super::{Album, Artist, CoverArt, MusicService, Playlist, SearchResults, ServiceType, Track};

/// YouTube Music client using ytmapi-rs for search and yt-dlp for stream extraction
pub struct YouTubeClient {
    ytdlp_path: PathBuf,
    #[allow(dead_code)]
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
            Ok(status) if status.success() => {
                Ok(Self {
                    ytdlp_path: PathBuf::from(path),
                    audio_quality: "best".to_string(),
                })
            }
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
        let url = format!("https://music.youtube.com/watch?v={}", track_id);

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

    async fn get_playlists(&mut self) -> Result<Vec<Playlist>> {
        // Unauthenticated mode - no playlists available
        Ok(vec![])
    }

    async fn get_playlist_tracks(&mut self, _playlist_id: &str) -> Result<Vec<Track>> {
        // Unauthenticated mode - no playlist access
        Err(anyhow!(
            "YouTube Music playlist access requires authentication (not supported)"
        ))
    }

    async fn get_favorite_tracks(&mut self) -> Result<Vec<Track>> {
        // Unauthenticated mode - no favorites
        Ok(vec![])
    }

    async fn get_favorite_albums(&mut self) -> Result<Vec<Album>> {
        // Unauthenticated mode - no favorites
        Ok(vec![])
    }

    async fn get_favorite_artists(&mut self) -> Result<Vec<Artist>> {
        // Unauthenticated mode - no favorites
        Ok(vec![])
    }

    async fn add_favorite_track(&mut self, _track_id: &str) -> Result<()> {
        Err(anyhow!(
            "YouTube Music favorites require authentication (not supported)"
        ))
    }

    async fn remove_favorite_track(&mut self, _track_id: &str) -> Result<()> {
        Err(anyhow!(
            "YouTube Music favorites require authentication (not supported)"
        ))
    }

    async fn search(&mut self, query: &str, limit: usize) -> Result<SearchResults> {
        // Use yt-dlp to search YouTube Music
        // Format: ytsearch{limit}:{query}
        let search_query = format!("ytsearch{}:{}", limit, query);

        let output = Command::new(&self.ytdlp_path)
            .args([
                "--flat-playlist",
                "-j", // JSON output
                "--no-warnings",
                &search_query,
            ])
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("yt-dlp search failed: {}", stderr));
        }

        let stdout = String::from_utf8(output.stdout)?;

        let tracks: Vec<Track> = stdout
            .lines()
            .filter_map(|line| {
                let json: serde_json::Value = serde_json::from_str(line).ok()?;

                let id = json.get("id")?.as_str()?.to_string();
                let title = json.get("title")?.as_str()?.to_string();

                let artist = json
                    .get("uploader")
                    .or_else(|| json.get("channel"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Artist")
                    .to_string();

                let duration = json
                    .get("duration")
                    .and_then(|v| v.as_f64())
                    .map(|d| d as u32)
                    .unwrap_or(0);

                let thumbnail = json
                    .get("thumbnail")
                    .or_else(|| {
                        json.get("thumbnails")
                            .and_then(|t| t.as_array())
                            .and_then(|arr| arr.last())
                            .and_then(|t| t.get("url"))
                    })
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                Some(Track {
                    id,
                    title,
                    artist,
                    album: String::new(), // YouTube doesn't have album info in search
                    duration_seconds: duration,
                    cover_art: thumbnail.map(CoverArt::Url).unwrap_or(CoverArt::None),
                    service: ServiceType::YouTube,
                })
            })
            .collect();

        Ok(SearchResults {
            tracks,
            albums: vec![], // yt-dlp search returns videos, not albums
            artists: vec![],
        })
    }

    async fn get_album_tracks(&mut self, _album_id: &str) -> Result<Vec<Track>> {
        // YouTube albums are actually playlists - not supported in unauthenticated mode
        Ok(vec![])
    }

    async fn get_artist_top_tracks(&mut self, _artist_id: &str) -> Result<Vec<Track>> {
        // Not supported in unauthenticated mode
        Ok(vec![])
    }

    async fn get_artist_albums(&mut self, _artist_id: &str) -> Result<Vec<Album>> {
        // Not supported in unauthenticated mode
        Ok(vec![])
    }

    async fn get_track_radio(&mut self, _track_id: &str, _limit: usize) -> Result<Vec<Track>> {
        // YouTube Mix requires authentication
        Ok(vec![])
    }

    async fn get_artist_radio(&mut self, _artist_id: &str, _limit: usize) -> Result<Vec<Track>> {
        // Not supported in unauthenticated mode
        Ok(vec![])
    }

    async fn get_playlist_radio(
        &mut self,
        _playlist_id: &str,
        _limit: usize,
    ) -> Result<Vec<Track>> {
        // Not supported in unauthenticated mode
        Ok(vec![])
    }

    async fn create_playlist(
        &mut self,
        _name: &str,
        _description: Option<&str>,
    ) -> Result<Playlist> {
        Err(anyhow!(
            "YouTube Music playlist creation requires authentication (not supported)"
        ))
    }

    async fn update_playlist(
        &mut self,
        _playlist_id: &str,
        _title: Option<&str>,
        _description: Option<&str>,
    ) -> Result<()> {
        Err(anyhow!(
            "YouTube Music playlist editing requires authentication (not supported)"
        ))
    }

    async fn delete_playlist(&mut self, _playlist_id: &str) -> Result<()> {
        Err(anyhow!(
            "YouTube Music playlist deletion requires authentication (not supported)"
        ))
    }

    async fn add_tracks_to_playlist(
        &mut self,
        _playlist_id: &str,
        _track_ids: &[String],
    ) -> Result<()> {
        Err(anyhow!(
            "YouTube Music playlist editing requires authentication (not supported)"
        ))
    }

    async fn remove_tracks_from_playlist(
        &mut self,
        _playlist_id: &str,
        _indices: &[usize],
    ) -> Result<()> {
        Err(anyhow!(
            "YouTube Music playlist editing requires authentication (not supported)"
        ))
    }

    fn get_cover_url(&self, cover: &CoverArt, _size: u32) -> Option<String> {
        match cover {
            CoverArt::Url(url) => Some(url.clone()),
            CoverArt::ServiceId { .. } => None, // YouTube uses direct URLs
            CoverArt::None => None,
        }
    }
}
