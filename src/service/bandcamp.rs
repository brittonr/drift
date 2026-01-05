use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use scraper::{Html, Selector};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use super::bandcamp_storage::{
    BandcampStorage, SavedPlaylist, StoredAlbum, StoredArtist, StoredTrack,
};
use super::{Album, Artist, CoverArt, MusicService, Playlist, SearchResults, ServiceType, Track};
use crate::config::BandcampConfig;

/// Bandcamp client using yt-dlp for stream extraction and HTML scraping for search
pub struct BandcampClient {
    ytdlp_path: PathBuf,
    config: BandcampConfig,
    http_client: reqwest::Client,
    audio_quality: String,
    authenticated: bool,
}

impl BandcampClient {
    /// Create a new Bandcamp client
    ///
    /// Errors if yt-dlp is not found in PATH.
    pub async fn new(config: BandcampConfig) -> Result<Self> {
        let ytdlp_path = "yt-dlp";

        // Verify yt-dlp exists
        let check = Command::new(ytdlp_path)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        match check {
            Ok(status) if status.success() => {}
            Ok(_) => {
                return Err(anyhow!(
                    "yt-dlp found but returned error. Please ensure yt-dlp is properly installed."
                ))
            }
            Err(_) => {
                return Err(anyhow!(
                    "yt-dlp not found. Bandcamp requires yt-dlp to be installed.\n\
                     Install via: nix-shell -p yt-dlp, brew install yt-dlp, or pip install yt-dlp"
                ))
            }
        }

        let http_client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:120.0) Gecko/20100101 Firefox/120.0")
            .build()?;

        // Test authentication if credentials provided
        let authenticated = config.cookie_file.is_some() || config.cookies_from_browser.is_some();

        Ok(Self {
            ytdlp_path: PathBuf::from(ytdlp_path),
            config,
            http_client,
            audio_quality: "mp3-128".to_string(),
            authenticated,
        })
    }

    // === yt-dlp Helper Methods ===

    /// Build cookie arguments for yt-dlp
    fn cookie_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(ref cookie_file) = self.config.cookie_file {
            args.push("--cookies".to_string());
            args.push(cookie_file.clone());
        } else if let Some(ref browser) = self.config.cookies_from_browser {
            args.push("--cookies-from-browser".to_string());
            args.push(browser.clone());
        }
        args
    }

    /// Execute yt-dlp and return JSON output as parsed values
    async fn run_ytdlp_json(&self, args: &[&str]) -> Result<Vec<Value>> {
        let mut cmd = Command::new(&self.ytdlp_path);
        cmd.args(args).args(["--no-warnings"]);

        // Add cookie args
        for arg in self.cookie_args() {
            cmd.arg(&arg);
        }

        let output = cmd.output().await?;

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

    /// Get track/album metadata (flat, no download)
    async fn get_playlist_info(&self, url: &str) -> Result<Vec<Value>> {
        self.run_ytdlp_json(&["--flat-playlist", "-j", url]).await
    }

    /// Get full track info
    async fn get_track_info(&self, track_url: &str) -> Result<Value> {
        let results = self.run_ytdlp_json(&["-j", track_url]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("No track info returned"))
    }

    /// Parse a JSON value into a Track
    fn parse_track(json: &Value) -> Option<Track> {
        // Bandcamp URLs serve as unique IDs
        let url = json.get("webpage_url")?.as_str()?.to_string();

        let title = json
            .get("track")
            .or_else(|| json.get("title"))
            .and_then(|v| v.as_str())?
            .to_string();

        let artist = json
            .get("artist")
            .or_else(|| json.get("uploader"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        let album = json
            .get("album")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let duration = json
            .get("duration")
            .and_then(|v| v.as_f64())
            .map(|d| d as u32)
            .unwrap_or(0);

        let thumbnail = Self::extract_thumbnail(json);

        Some(Track {
            id: url, // Use full URL as ID for Bandcamp
            title,
            artist,
            album,
            duration_seconds: duration,
            cover_art: thumbnail.map(CoverArt::Url).unwrap_or(CoverArt::None),
            service: ServiceType::Bandcamp,
        })
    }

    /// Parse a JSON value into an Album
    #[allow(dead_code)]
    fn parse_album(json: &Value) -> Option<Album> {
        let url = json.get("webpage_url")?.as_str()?.to_string();

        let title = json.get("title")?.as_str()?.to_string();

        let artist = json
            .get("artist")
            .or_else(|| json.get("uploader"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        let num_tracks = json
            .get("playlist_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        let thumbnail = Self::extract_thumbnail(json);

        Some(Album {
            id: url,
            title,
            artist,
            num_tracks,
            cover_art: thumbnail.map(CoverArt::Url).unwrap_or(CoverArt::None),
            service: ServiceType::Bandcamp,
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

    /// Extract artist subdomain from URL
    fn extract_subdomain(url: &str) -> Option<String> {
        // e.g., "https://phoebebridgers.bandcamp.com/track/..." -> "phoebebridgers"
        url.strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
            .and_then(|s| s.split('.').next())
            .map(|s| s.to_string())
    }

    /// Convert StoredTrack to Track
    fn stored_to_track(stored: &StoredTrack) -> Track {
        Track {
            id: stored.url.clone(),
            title: stored.title.clone(),
            artist: stored.artist.clone(),
            album: stored.album.clone(),
            duration_seconds: stored.duration_seconds,
            cover_art: stored
                .thumbnail_url
                .clone()
                .map(CoverArt::Url)
                .unwrap_or(CoverArt::None),
            service: ServiceType::Bandcamp,
        }
    }

    /// Convert StoredAlbum to Album
    fn stored_to_album(stored: &StoredAlbum) -> Album {
        Album {
            id: stored.url.clone(),
            title: stored.title.clone(),
            artist: stored.artist.clone(),
            num_tracks: stored.num_tracks,
            cover_art: stored
                .thumbnail_url
                .clone()
                .map(CoverArt::Url)
                .unwrap_or(CoverArt::None),
            service: ServiceType::Bandcamp,
        }
    }

    /// Convert StoredArtist to Artist
    fn stored_to_artist(stored: &StoredArtist) -> Artist {
        Artist {
            id: stored.subdomain.clone(),
            name: stored.name.clone(),
            service: ServiceType::Bandcamp,
        }
    }

    /// Convert SavedPlaylist to Playlist
    fn saved_to_playlist(saved: &SavedPlaylist) -> Playlist {
        Playlist {
            id: saved.id.clone(),
            title: saved.title.clone(),
            description: saved.description.clone(),
            num_tracks: saved.num_tracks,
            service: ServiceType::Bandcamp,
        }
    }

    // === HTML Search Scraping ===

    /// Search Bandcamp for tracks
    async fn search_tracks(&self, query: &str, limit: usize) -> Vec<Track> {
        let url = format!(
            "https://bandcamp.com/search?q={}&item_type=t",
            urlencoding::encode(query)
        );

        let html = match self.http_client.get(&url).send().await {
            Ok(resp) => match resp.text().await {
                Ok(text) => text,
                Err(_) => return vec![],
            },
            Err(_) => return vec![],
        };

        let document = Html::parse_document(&html);
        let result_selector = Selector::parse(".searchresult.track").unwrap_or_else(|_| {
            Selector::parse(".result-info").unwrap()
        });
        let title_selector = Selector::parse(".heading a").unwrap_or_else(|_| {
            Selector::parse(".itemurl a").unwrap()
        });
        let artist_selector = Selector::parse(".subhead").unwrap_or_else(|_| {
            Selector::parse(".itemurl a").unwrap()
        });
        let art_selector = Selector::parse(".art img").unwrap_or_else(|_| {
            Selector::parse("img").unwrap()
        });

        let mut tracks = Vec::new();

        for result in document.select(&result_selector).take(limit) {
            let title = result
                .select(&title_selector)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            let track_url = result
                .select(&title_selector)
                .next()
                .and_then(|el| el.value().attr("href"))
                .map(|s| s.to_string())
                .unwrap_or_default();

            let artist = result
                .select(&artist_selector)
                .next()
                .map(|el| {
                    el.text()
                        .collect::<String>()
                        .trim()
                        .trim_start_matches("by ")
                        .trim_start_matches("from ")
                        .to_string()
                })
                .unwrap_or_else(|| "Unknown".to_string());

            let thumbnail = result
                .select(&art_selector)
                .next()
                .and_then(|el| el.value().attr("src"))
                .map(|s| s.to_string());

            if !track_url.is_empty() && !title.is_empty() {
                tracks.push(Track {
                    id: track_url,
                    title,
                    artist,
                    album: String::new(),
                    duration_seconds: 0, // Search results don't include duration
                    cover_art: thumbnail.map(CoverArt::Url).unwrap_or(CoverArt::None),
                    service: ServiceType::Bandcamp,
                });
            }
        }

        tracks
    }

    /// Search Bandcamp for albums
    async fn search_albums(&self, query: &str, limit: usize) -> Vec<Album> {
        let url = format!(
            "https://bandcamp.com/search?q={}&item_type=a",
            urlencoding::encode(query)
        );

        let html = match self.http_client.get(&url).send().await {
            Ok(resp) => match resp.text().await {
                Ok(text) => text,
                Err(_) => return vec![],
            },
            Err(_) => return vec![],
        };

        let document = Html::parse_document(&html);
        let result_selector = Selector::parse(".searchresult.album").unwrap_or_else(|_| {
            Selector::parse(".result-info").unwrap()
        });
        let title_selector = Selector::parse(".heading a").unwrap_or_else(|_| {
            Selector::parse(".itemurl a").unwrap()
        });
        let artist_selector = Selector::parse(".subhead").unwrap_or_else(|_| {
            Selector::parse(".itemurl a").unwrap()
        });
        let art_selector = Selector::parse(".art img").unwrap_or_else(|_| {
            Selector::parse("img").unwrap()
        });

        let mut albums = Vec::new();

        for result in document.select(&result_selector).take(limit) {
            let title = result
                .select(&title_selector)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            let album_url = result
                .select(&title_selector)
                .next()
                .and_then(|el| el.value().attr("href"))
                .map(|s| s.to_string())
                .unwrap_or_default();

            let artist = result
                .select(&artist_selector)
                .next()
                .map(|el| {
                    el.text()
                        .collect::<String>()
                        .trim()
                        .trim_start_matches("by ")
                        .to_string()
                })
                .unwrap_or_else(|| "Unknown".to_string());

            let thumbnail = result
                .select(&art_selector)
                .next()
                .and_then(|el| el.value().attr("src"))
                .map(|s| s.to_string());

            if !album_url.is_empty() && !title.is_empty() {
                albums.push(Album {
                    id: album_url,
                    title,
                    artist,
                    num_tracks: 0, // Search results don't include track count
                    cover_art: thumbnail.map(CoverArt::Url).unwrap_or(CoverArt::None),
                    service: ServiceType::Bandcamp,
                });
            }
        }

        albums
    }

    /// Search Bandcamp for artists (bands)
    async fn search_artists(&self, query: &str, limit: usize) -> Vec<Artist> {
        let url = format!(
            "https://bandcamp.com/search?q={}&item_type=b",
            urlencoding::encode(query)
        );

        let html = match self.http_client.get(&url).send().await {
            Ok(resp) => match resp.text().await {
                Ok(text) => text,
                Err(_) => return vec![],
            },
            Err(_) => return vec![],
        };

        let document = Html::parse_document(&html);
        let result_selector = Selector::parse(".searchresult.band").unwrap_or_else(|_| {
            Selector::parse(".result-info").unwrap()
        });
        let name_selector = Selector::parse(".heading a").unwrap_or_else(|_| {
            Selector::parse(".itemurl a").unwrap()
        });

        let mut artists = Vec::new();

        for result in document.select(&result_selector).take(limit) {
            let name = result
                .select(&name_selector)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            let artist_url = result
                .select(&name_selector)
                .next()
                .and_then(|el| el.value().attr("href"))
                .map(|s| s.to_string())
                .unwrap_or_default();

            // Extract subdomain as ID
            let subdomain = Self::extract_subdomain(&artist_url).unwrap_or_default();

            if !subdomain.is_empty() && !name.is_empty() {
                artists.push(Artist {
                    id: subdomain,
                    name,
                    service: ServiceType::Bandcamp,
                });
            }
        }

        artists
    }

    /// Get user's collection (purchased/wishlisted items)
    async fn get_user_collection(&self) -> Result<Vec<Track>> {
        let username = self
            .config
            .username
            .as_ref()
            .ok_or_else(|| anyhow!("Bandcamp username not configured"))?;

        let url = format!("https://bandcamp.com/{}", username);
        let results = self.get_playlist_info(&url).await?;

        Ok(results.iter().filter_map(Self::parse_track).collect())
    }
}

#[async_trait]
impl MusicService for BandcampClient {
    fn service_type(&self) -> ServiceType {
        ServiceType::Bandcamp
    }

    fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    fn set_audio_quality(&mut self, quality: &str) {
        self.audio_quality = match quality.to_lowercase().as_str() {
            "low" => "mp3-v0".to_string(),
            "high" => "mp3-128".to_string(),
            "lossless" | "master" => "flac".to_string(),
            _ => "mp3-128".to_string(),
        };
    }

    async fn get_stream_url(&mut self, track_id: &str) -> Result<String> {
        // track_id is the full URL for Bandcamp
        let mut cmd = Command::new(&self.ytdlp_path);
        cmd.args([
            "-f",
            &self.audio_quality,
            "-g", // Get URL only
            "--no-warnings",
            track_id,
        ]);

        // Add cookie args
        for arg in self.cookie_args() {
            cmd.arg(&arg);
        }

        let output = cmd.output().await?;

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
        let storage = BandcampStorage::load().unwrap_or_default();
        let mut playlists: Vec<Playlist> = storage
            .saved_playlists
            .iter()
            .map(Self::saved_to_playlist)
            .collect();

        // Add collection as pseudo-playlist if authenticated
        if self.authenticated && self.config.username.is_some() {
            let username = self.config.username.as_ref().unwrap();
            playlists.insert(
                0,
                Playlist {
                    id: format!("collection:{}", username),
                    title: "My Collection".to_string(),
                    description: Some("Purchased and wishlisted items".to_string()),
                    num_tracks: 0, // Unknown without fetching
                    service: ServiceType::Bandcamp,
                },
            );
        }

        Ok(playlists)
    }

    async fn get_playlist_tracks(&mut self, playlist_id: &str) -> Result<Vec<Track>> {
        // Handle collection pseudo-playlist
        if playlist_id.starts_with("collection:") {
            return self.get_user_collection().await;
        }

        let storage = BandcampStorage::load().unwrap_or_default();

        // Local playlist - get track URLs and fetch info
        let track_urls = storage.get_local_playlist_tracks(playlist_id);
        let mut tracks = Vec::new();

        for url in track_urls {
            // Try to find in favorites first (cached metadata)
            if let Some(stored) = storage.find_track(&url) {
                tracks.push(Self::stored_to_track(stored));
            } else {
                // Fetch info for tracks not in favorites
                if let Ok(info) = self.get_track_info(&url).await {
                    if let Some(track) = Self::parse_track(&info) {
                        tracks.push(track);
                    }
                }
            }
        }

        Ok(tracks)
    }

    // === Library: Favorites ===

    async fn get_favorite_tracks(&mut self) -> Result<Vec<Track>> {
        let storage = BandcampStorage::load().unwrap_or_default();
        Ok(storage
            .favorite_tracks
            .iter()
            .map(Self::stored_to_track)
            .collect())
    }

    async fn get_favorite_albums(&mut self) -> Result<Vec<Album>> {
        let storage = BandcampStorage::load().unwrap_or_default();
        Ok(storage
            .favorite_albums
            .iter()
            .map(Self::stored_to_album)
            .collect())
    }

    async fn get_favorite_artists(&mut self) -> Result<Vec<Artist>> {
        let storage = BandcampStorage::load().unwrap_or_default();
        Ok(storage
            .favorite_artists
            .iter()
            .map(Self::stored_to_artist)
            .collect())
    }

    async fn add_favorite_track(&mut self, track_id: &str) -> Result<()> {
        // track_id is the full URL
        let info = self.get_track_info(track_id).await?;

        let subdomain = Self::extract_subdomain(track_id).unwrap_or_default();

        let stored = StoredTrack {
            url: track_id.to_string(),
            track_id: info
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            title: info
                .get("track")
                .or_else(|| info.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string(),
            artist: info
                .get("artist")
                .or_else(|| info.get("uploader"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string(),
            artist_subdomain: subdomain,
            album: info
                .get("album")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            album_url: info
                .get("playlist_url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            duration_seconds: info
                .get("duration")
                .and_then(|v| v.as_f64())
                .map(|d| d as u32)
                .unwrap_or(0),
            thumbnail_url: Self::extract_thumbnail(&info),
            added_at: Utc::now(),
        };

        let mut storage = BandcampStorage::load().unwrap_or_default();
        storage.add_favorite_track(stored);
        storage.save()?;

        Ok(())
    }

    async fn remove_favorite_track(&mut self, track_id: &str) -> Result<()> {
        let mut storage = BandcampStorage::load().unwrap_or_default();
        storage.remove_favorite_track(track_id);
        storage.save()?;
        Ok(())
    }

    // === Search ===

    async fn search(&mut self, query: &str, limit: usize) -> Result<SearchResults> {
        // Run all three searches in parallel
        let (tracks, albums, artists) = tokio::join!(
            self.search_tracks(query, limit),
            self.search_albums(query, limit.min(10)),
            self.search_artists(query, limit.min(10))
        );

        Ok(SearchResults {
            tracks,
            albums,
            artists,
        })
    }

    // === Album/Artist Details ===

    async fn get_album_tracks(&mut self, album_id: &str) -> Result<Vec<Track>> {
        // album_id is the full album URL
        let results = self.get_playlist_info(album_id).await?;
        Ok(results.iter().filter_map(Self::parse_track).collect())
    }

    async fn get_artist_top_tracks(&mut self, artist_id: &str) -> Result<Vec<Track>> {
        // artist_id is the subdomain (e.g., "phoebebridgers")
        let artist_url = format!("https://{}.bandcamp.com", artist_id);

        // Get artist's discography
        let results = self.get_playlist_info(&artist_url).await?;

        // Return first 20 tracks from their releases
        Ok(results
            .iter()
            .filter_map(Self::parse_track)
            .take(20)
            .collect())
    }

    async fn get_artist_albums(&mut self, artist_id: &str) -> Result<Vec<Album>> {
        // Scrape the artist's /music page for albums
        let url = format!("https://{}.bandcamp.com/music", artist_id);

        let html = match self.http_client.get(&url).send().await {
            Ok(resp) => match resp.text().await {
                Ok(text) => text,
                Err(e) => return Err(anyhow!("Failed to read response: {}", e)),
            },
            Err(e) => return Err(anyhow!("Failed to fetch artist page: {}", e)),
        };

        let document = Html::parse_document(&html);

        // Try multiple selectors for album items
        let album_selector = Selector::parse(".music-grid-item").unwrap_or_else(|_| {
            Selector::parse("li.music-grid-item").unwrap_or_else(|_| {
                Selector::parse(".album-grid-item").unwrap()
            })
        });
        let title_selector = Selector::parse(".title").unwrap_or_else(|_| {
            Selector::parse("p.title").unwrap()
        });
        let link_selector = Selector::parse("a").unwrap();
        let art_selector = Selector::parse("img").unwrap();

        let mut albums = Vec::new();
        let base_url = format!("https://{}.bandcamp.com", artist_id);

        for item in document.select(&album_selector) {
            let title = item
                .select(&title_selector)
                .next()
                .map(|el| el.text().collect::<String>().trim().to_string())
                .unwrap_or_default();

            let album_path = item
                .select(&link_selector)
                .next()
                .and_then(|el| el.value().attr("href"))
                .unwrap_or_default();

            let album_url = if album_path.starts_with("http") {
                album_path.to_string()
            } else {
                format!("{}{}", base_url, album_path)
            };

            let thumbnail = item
                .select(&art_selector)
                .next()
                .and_then(|el| el.value().attr("src").or_else(|| el.value().attr("data-original")))
                .map(|s| s.to_string());

            if !title.is_empty() && album_url.contains("/album/") {
                albums.push(Album {
                    id: album_url,
                    title,
                    artist: artist_id.to_string(),
                    num_tracks: 0,
                    cover_art: thumbnail.map(CoverArt::Url).unwrap_or(CoverArt::None),
                    service: ServiceType::Bandcamp,
                });
            }
        }

        Ok(albums)
    }

    // === Radio/Recommendations ===

    async fn get_track_radio(&mut self, track_id: &str, limit: usize) -> Result<Vec<Track>> {
        // Strategy 1: Get other tracks from the same album
        if let Ok(info) = self.get_track_info(track_id).await {
            if let Some(album_url) = info.get("playlist_url").and_then(|v| v.as_str()) {
                let album_tracks = self.get_album_tracks(album_url).await?;
                let tracks: Vec<Track> = album_tracks
                    .into_iter()
                    .filter(|t| t.id != track_id)
                    .take(limit)
                    .collect();

                if !tracks.is_empty() {
                    return Ok(tracks);
                }
            }

            // Strategy 2: Get tracks from the same artist
            if let Some(subdomain) = Self::extract_subdomain(track_id) {
                let artist_tracks = self.get_artist_top_tracks(&subdomain).await?;
                let tracks: Vec<Track> = artist_tracks
                    .into_iter()
                    .filter(|t| t.id != track_id)
                    .take(limit)
                    .collect();

                if !tracks.is_empty() {
                    return Ok(tracks);
                }
            }

            // Strategy 3: Search for similar content
            let title = info
                .get("track")
                .or_else(|| info.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("music");

            let search_results = self.search(title, limit + 5).await?;
            return Ok(search_results
                .tracks
                .into_iter()
                .filter(|t| t.id != track_id)
                .take(limit)
                .collect());
        }

        Ok(vec![])
    }

    async fn get_artist_radio(&mut self, artist_id: &str, limit: usize) -> Result<Vec<Track>> {
        // Get top tracks from the artist
        let tracks = self.get_artist_top_tracks(artist_id).await?;
        Ok(tracks.into_iter().take(limit).collect())
    }

    async fn get_playlist_radio(&mut self, playlist_id: &str, limit: usize) -> Result<Vec<Track>> {
        // Get first track from playlist and use its radio
        let tracks = self.get_playlist_tracks(playlist_id).await?;

        if let Some(first_track) = tracks.first() {
            return self.get_track_radio(&first_track.id, limit).await;
        }

        Ok(vec![])
    }

    // === Playlist Management (Local Only) ===

    async fn create_playlist(&mut self, name: &str, description: Option<&str>) -> Result<Playlist> {
        let mut storage = BandcampStorage::load().unwrap_or_default();
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
        // Cannot update collection pseudo-playlist
        if playlist_id.starts_with("collection:") {
            return Err(anyhow!("Cannot modify collection"));
        }

        let mut storage = BandcampStorage::load()?;

        if storage.update_playlist(playlist_id, title, description) {
            storage.save()?;
            Ok(())
        } else {
            Err(anyhow!("Playlist not found"))
        }
    }

    async fn delete_playlist(&mut self, playlist_id: &str) -> Result<()> {
        if playlist_id.starts_with("collection:") {
            return Err(anyhow!("Cannot delete collection"));
        }

        let mut storage = BandcampStorage::load()?;

        if storage.remove_playlist(playlist_id) {
            storage.save()?;
            Ok(())
        } else {
            Err(anyhow!("Playlist not found"))
        }
    }

    async fn add_tracks_to_playlist(
        &mut self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<()> {
        if playlist_id.starts_with("collection:") {
            return Err(anyhow!("Cannot add tracks to collection"));
        }

        let mut storage = BandcampStorage::load()?;

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
        if playlist_id.starts_with("collection:") {
            return Err(anyhow!("Cannot remove tracks from collection"));
        }

        let mut storage = BandcampStorage::load()?;

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
            CoverArt::ServiceId { .. } => None, // Bandcamp uses direct URLs
            CoverArt::None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_subdomain() {
        assert_eq!(
            BandcampClient::extract_subdomain("https://phoebebridgers.bandcamp.com/track/motion-sickness"),
            Some("phoebebridgers".to_string())
        );
        assert_eq!(
            BandcampClient::extract_subdomain("https://artist.bandcamp.com/album/test"),
            Some("artist".to_string())
        );
        assert_eq!(
            BandcampClient::extract_subdomain("http://test.bandcamp.com"),
            Some("test".to_string())
        );
    }

    #[test]
    fn test_parse_track() {
        let json: Value = serde_json::json!({
            "webpage_url": "https://artist.bandcamp.com/track/test-track",
            "track": "Test Track",
            "artist": "Test Artist",
            "album": "Test Album",
            "duration": 180.5,
            "thumbnail": "https://example.com/thumb.jpg"
        });

        let track = BandcampClient::parse_track(&json).unwrap();
        assert_eq!(track.id, "https://artist.bandcamp.com/track/test-track");
        assert_eq!(track.title, "Test Track");
        assert_eq!(track.artist, "Test Artist");
        assert_eq!(track.album, "Test Album");
        assert_eq!(track.duration_seconds, 180);
        assert!(matches!(track.service, ServiceType::Bandcamp));
    }

    #[test]
    fn test_parse_album() {
        let json: Value = serde_json::json!({
            "webpage_url": "https://artist.bandcamp.com/album/test-album",
            "title": "Test Album",
            "artist": "Test Artist",
            "playlist_count": 10,
            "thumbnail": "https://example.com/cover.jpg"
        });

        let album = BandcampClient::parse_album(&json).unwrap();
        assert_eq!(album.id, "https://artist.bandcamp.com/album/test-album");
        assert_eq!(album.title, "Test Album");
        assert_eq!(album.artist, "Test Artist");
        assert_eq!(album.num_tracks, 10);
    }
}
