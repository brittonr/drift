use anyhow::{Result, anyhow};
use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use reqwest::{Client as HttpClient, header};
use serde_json::Value;
use chrono::{DateTime, Utc, Duration};
use base64::{Engine as _, engine::general_purpose};

#[derive(Debug, Serialize, Deserialize)]
pub struct TidalConfig {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub user_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

// Data models for UI
#[derive(Debug, Clone)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub num_tracks: usize,
}

#[derive(Debug, Clone)]
pub struct Track {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: u32,
    pub album_cover_id: Option<String>,  // Tidal cover ID (e.g., "abc123-def4-5678")
}

#[derive(Debug, Clone)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
}

#[derive(Debug, Clone)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub num_tracks: u32,
}

#[derive(Debug, Clone)]
pub struct Artist {
    pub id: u64,
    pub name: String,
}

// API Response models
#[derive(Debug, Deserialize)]
struct PlaylistsResponse {
    items: Option<Vec<PlaylistItem>>,
    #[serde(rename = "totalNumberOfItems")]
    total: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct PlaylistItem {
    uuid: String,
    title: String,
    description: Option<String>,
    #[serde(rename = "numberOfTracks")]
    number_of_tracks: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct TracksResponse {
    items: Option<Vec<TrackItem>>,
    #[serde(rename = "totalNumberOfItems")]
    total: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct TrackItem {
    item: Option<TrackData>,
}

#[derive(Debug, Deserialize)]
struct TrackData {
    id: u64,
    title: String,
    artist: Option<ArtistResponse>,
    artists: Option<Vec<ArtistResponse>>,
    album: Option<AlbumResponse>,
    duration: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ArtistResponse {
    name: String,
}

#[derive(Debug, Deserialize)]
struct AlbumResponse {
    title: String,
    cover: Option<String>,  // Album cover ID from Tidal
}

pub struct TidalClient {
    pub config: Option<TidalConfig>,
    http_client: HttpClient,
}

impl TidalClient {
    pub async fn new() -> Result<Self> {
        // First try to load from existing upmpdcli credentials
        let upmpdcli_path = Self::upmpdcli_path()?;
        let tidal_tui_path = Self::config_path()?;

        let config = if upmpdcli_path.exists() {
            println!("Loading existing upmpdcli Tidal credentials...");
            let config = Self::load_config(&upmpdcli_path)?;
            // Copy to our config location
            if !tidal_tui_path.exists() {
                let contents = fs::read_to_string(&upmpdcli_path)?;
                fs::write(&tidal_tui_path, contents)?;
            }
            Some(config)
        } else if tidal_tui_path.exists() {
            println!("Loading existing Tidal credentials...");
            Some(Self::load_config(&tidal_tui_path)?)
        } else {
            println!("No Tidal credentials found. Running in demo mode.");
            None
        };

        let http_client = HttpClient::new();

        Ok(Self { config, http_client })
    }

    fn config_path() -> Result<PathBuf> {
        let mut path = config_dir().ok_or_else(|| anyhow!("Could not find config directory"))?;
        path.push("tidal-tui");
        fs::create_dir_all(&path)?;
        path.push("credentials.json");
        Ok(path)
    }

    fn upmpdcli_path() -> Result<PathBuf> {
        let mut path = config_dir().ok_or_else(|| anyhow!("Could not find config directory"))?;
        path.push("upmpdcli");
        path.push("qobuz");
        path.push("oauth2.credentials.json");
        Ok(path)
    }

    fn load_config(path: &PathBuf) -> Result<TidalConfig> {
        let contents = fs::read_to_string(path)?;
        let config: TidalConfig = serde_json::from_str(&contents)?;
        Ok(config)
    }

    pub async fn save_config(&self) -> Result<()> {
        if let Some(ref config) = self.config {
            let path = Self::config_path()?;
            let contents = serde_json::to_string_pretty(config)?;
            fs::write(&path, contents)?;
        }
        Ok(())
    }

    pub async fn get_playlists(&mut self) -> Result<Vec<Playlist>> {
        // Try up to 2 times (initial + 1 retry after refresh)
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                // Try to fetch real playlists
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/playlists",
                    config.user_id
                );

                let response = self.http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US"), ("limit", "50")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let playlists_resp: PlaylistsResponse = resp.json().await?;

                        if let Some(items) = playlists_resp.items {
                            let playlists = items.into_iter().map(|item| Playlist {
                                id: item.uuid,
                                title: item.title,
                                description: item.description,
                                num_tracks: item.number_of_tracks.unwrap_or(0) as usize,
                            }).collect();
                            return Ok(playlists);
                        }
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        // Try to refresh token and retry
                        eprintln!("Got 401, attempting to refresh token...");
                        if self.refresh_token().await.is_ok() {
                            continue; // Retry with new token
                        }
                    }
                    Ok(resp) => {
                        eprintln!("API request failed with status: {}", resp.status());
                    }
                    Err(e) => {
                        eprintln!("Network error fetching playlists: {}", e);
                    }
                }
            }
            break; // Exit loop if no retry needed
        }

        // Fallback to demo playlists
        Ok(self.get_demo_playlists())
    }

    fn get_demo_playlists(&self) -> Vec<Playlist> {
        vec![
            Playlist {
                id: "demo-1".to_string(),
                title: "My Mix 1".to_string(),
                description: Some("Your personalized mix".to_string()),
                num_tracks: 50,
            },
            Playlist {
                id: "demo-2".to_string(),
                title: "Favorites".to_string(),
                description: Some("Your favorite tracks".to_string()),
                num_tracks: 123,
            },
            Playlist {
                id: "demo-3".to_string(),
                title: "Recently Played".to_string(),
                description: Some("Recently played tracks".to_string()),
                num_tracks: 25,
            },
        ]
    }

    pub async fn get_tracks(&mut self, playlist_id: &str) -> Result<Vec<Track>> {
        if !playlist_id.starts_with("demo-") {
            // Try up to 2 times (initial + 1 retry after refresh)
            for attempt in 0..2 {
                if let Some(ref config) = self.config {
                    // Try to fetch real tracks
                    let url = format!(
                        "https://api.tidal.com/v1/playlists/{}/items",
                        playlist_id
                    );

                    let response = self.http_client
                        .get(&url)
                        .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                        .query(&[("countryCode", "US"), ("limit", "100")])
                        .send()
                        .await;

                    match response {
                        Ok(resp) if resp.status().is_success() => {
                        let tracks_resp: TracksResponse = resp.json().await?;

                        if let Some(items) = tracks_resp.items {
                            let tracks = items.into_iter().filter_map(|item| {
                                item.item.map(|track| {
                                    let artist_name = track.artist
                                        .or_else(|| track.artists.and_then(|a| a.into_iter().next()))
                                        .map(|a| a.name)
                                        .unwrap_or_else(|| "Unknown Artist".to_string());

                                    let (album_title, album_cover_id) = track.album
                                        .map(|a| (a.title, a.cover))
                                        .unwrap_or_else(|| ("Unknown Album".to_string(), None));

                                    Track {
                                        id: track.id,
                                        title: track.title,
                                        artist: artist_name,
                                        album: album_title,
                                        duration_seconds: track.duration.unwrap_or(0),
                                        album_cover_id,
                                    }
                                })
                            }).collect();
                            return Ok(tracks);
                        }
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        // Try to refresh token and retry
                        eprintln!("Got 401, attempting to refresh token...");
                        if self.refresh_token().await.is_ok() {
                            continue; // Retry with new token
                        }
                    }
                    Ok(resp) => {
                        eprintln!("API request failed with status: {}", resp.status());
                    }
                    Err(e) => {
                        eprintln!("Network error fetching tracks: {}", e);
                    }
                }
            }
            break; // Exit loop if no retry needed
        }
    }

        // Fallback to demo tracks
        Ok(self.get_demo_tracks())
    }

    fn get_demo_tracks(&self) -> Vec<Track> {
        vec![
            Track {
                id: 1,
                title: "Bohemian Rhapsody".to_string(),
                artist: "Queen".to_string(),
                album: "A Night at the Opera".to_string(),
                duration_seconds: 354,
                album_cover_id: None,
            },
            Track {
                id: 2,
                title: "Stairway to Heaven".to_string(),
                artist: "Led Zeppelin".to_string(),
                album: "Led Zeppelin IV".to_string(),
                duration_seconds: 482,
                album_cover_id: None,
            },
            Track {
                id: 3,
                title: "Hotel California".to_string(),
                artist: "Eagles".to_string(),
                album: "Hotel California".to_string(),
                duration_seconds: 391,
                album_cover_id: None,
            },
        ]
    }

    pub async fn get_stream_url(&mut self, track_id: &str) -> Result<String> {
        if !track_id.starts_with("demo") {
            // Try up to 2 times (initial + 1 retry after refresh)
            for attempt in 0..2 {
                // Get a fresh config reference each iteration to avoid borrow issues
                let (token, user_id) = if let Some(ref config) = self.config {
                    (config.access_token.clone(), config.user_id)
                } else {
                    break;
                };
                // Try playbackinfo endpoint first (newer API)
                let url = format!(
                    "https://api.tidal.com/v1/tracks/{}/playbackinfo",
                    track_id
                );

                let response = self.http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", token))
                    .query(&[
                        ("countryCode", "US"),
                        ("assetpresentation", "FULL"),  // Required parameter (lowercase!)
                        ("audioquality", "HIGH"),
                        ("playbackmode", "STREAM"),
                    ])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        // Try different possible response structures
                        if let Some(manifest) = json.get("manifest").and_then(|v| v.as_str()) {
                            // Playbackinfo returns a manifest (base64 encoded)
                            if let Ok(decoded) = general_purpose::STANDARD.decode(manifest) {
                                if let Ok(manifest_str) = String::from_utf8(decoded) {
                                    // Parse the manifest JSON
                                    if let Ok(manifest_json) = serde_json::from_str::<Value>(&manifest_str) {
                                        if let Some(urls) = manifest_json.get("urls").and_then(|u| u.as_array()) {
                                            if let Some(first_url) = urls.first().and_then(|u| u.as_str()) {
                                                return Ok(first_url.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Try direct URL field (older API)
                        if let Some(url) = json.get("url").and_then(|v| v.as_str()) {
                            return Ok(url.to_string());
                        }

                        // Try urls array
                        if let Some(urls) = json.get("urls").and_then(|v| v.as_array()) {
                            if let Some(first_url) = urls.first().and_then(|u| u.as_str()) {
                                return Ok(first_url.to_string());
                            }
                        }

                        eprintln!("DEBUG: Stream response structure: {}", serde_json::to_string_pretty(&json)?);
                        return Err(anyhow!("Could not find stream URL in response"));
                    }
                    Ok(resp) => {
                        let status = resp.status();

                        // If we get 401 on first attempt, try to refresh and retry
                        if status.as_u16() == 401 && attempt == 0 {
                            drop(resp); // Drop the response to release the borrow
                            eprintln!("Got 401 for stream URL, attempting to refresh token...");
                            if self.refresh_token().await.is_ok() {
                                continue; // Retry with new token
                            }
                            // If refresh failed, we'll fall through to error handling
                            return Err(anyhow!("Failed to get stream URL after token refresh"));
                        }

                        // If playbackinfo fails with 401/403, try the older streamUrl endpoint
                        if status.as_u16() == 401 || status.as_u16() == 403 {
                            eprintln!("Playbackinfo failed with {}, trying streamUrl endpoint...", status);

                            let stream_url = format!(
                                "https://api.tidal.com/v1/tracks/{}/streamUrl",
                                track_id
                            );

                            let stream_response = self.http_client
                                .get(&stream_url)
                                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                                .query(&[
                                    ("countryCode", "US"),
                                    ("soundQuality", "HIGH"),
                                    ("assetpresentation", "FULL"),  // lowercase!
                                ])
                                .send()
                                .await;

                            if let Ok(stream_resp) = stream_response {
                                if stream_resp.status().is_success() {
                                    let json: Value = stream_resp.json().await?;
                                    if let Some(url) = json.get("url").and_then(|v| v.as_str()) {
                                        return Ok(url.to_string());
                                    }
                                }
                            }
                        }

                        let body = resp.text().await.unwrap_or_default();
                        eprintln!("Stream URL request failed:");
                        eprintln!("  Status: {}", status);
                        eprintln!("  URL: {}", url);
                        eprintln!("  Response: {}", &body[..500.min(body.len())]);

                        // Try to parse error details
                        if let Ok(error_json) = serde_json::from_str::<Value>(&body) {
                            if let Some(user_message) = error_json.get("userMessage").and_then(|m| m.as_str()) {
                                return Err(anyhow!("Tidal API error: {}", user_message));
                            }
                        }

                        return Err(anyhow!("Failed to get stream URL. Status: {} - {}", status, &body[..100.min(body.len())]));
                    }
                    Err(e) => {
                        return Err(anyhow!("Network error getting stream URL: {}", e));
                    }
                }
            }
        }

        // Fallback URL that won't actually play
        Ok(format!("tidal://track/{}", track_id))
    }

    pub fn format_track_display(track: &Track) -> String {
        format!("{} - {} ({}:{:02})",
            track.artist,
            track.title,
            track.duration_seconds / 60,
            track.duration_seconds % 60
        )
    }

    /// Get album cover URL from Tidal cover ID
    /// Size can be: 80, 160, 320, 640, 1280
    pub fn get_album_cover_url(cover_id: &str, size: u32) -> String {
        let path = cover_id.replace('-', "/");
        format!("https://resources.tidal.com/images/{}/{}x{}.jpg",
            path, size, size
        )
    }

    pub fn format_playlist_display(playlist: &Playlist) -> String {
        format!("{} ({} tracks)", playlist.title, playlist.num_tracks)
    }

    pub async fn search(&mut self, query: &str, limit: usize) -> Result<SearchResults> {
        // Try up to 2 times (initial + 1 retry after refresh)
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = "https://api.tidal.com/v1/search";

                let response = self.http_client
                    .get(url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[
                        ("query", query),
                        ("limit", &limit.to_string()),
                        ("countryCode", "US"),
                        ("types", "TRACKS,ALBUMS,ARTISTS"),
                    ])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                    let json: Value = resp.json().await?;

                    // Parse tracks
                    let tracks = if let Some(tracks_data) = json.get("tracks").and_then(|t| t.get("items")) {
                        tracks_data.as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(|item| {
                                let id = item.get("id")?.as_u64()?;
                                let title = item.get("title")?.as_str()?.to_string();

                                // Parse artist - search results use "artists" array
                                let artist = item.get("artists")
                                    .and_then(|artists| artists.as_array())
                                    .map(|arr| {
                                        let names: Vec<&str> = arr.iter()
                                            .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                                            .collect();
                                        if names.is_empty() {
                                            "Unknown Artist".to_string()
                                        } else {
                                            names.join(", ")
                                        }
                                    })
                                    .or_else(|| {
                                        // Fallback to artist object (singular)
                                        item.get("artist")
                                            .and_then(|a| a.get("name"))
                                            .and_then(|n| n.as_str())
                                            .map(|s| s.to_string())
                                    })
                                    .unwrap_or_else(|| "Unknown Artist".to_string());

                                let album = item.get("album")
                                    .and_then(|a| a.get("title"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("Unknown Album")
                                    .to_string();

                                let album_cover_id = item.get("album")
                                    .and_then(|a| a.get("cover"))
                                    .and_then(|c| c.as_str())
                                    .map(|s| s.to_string());

                                let duration = item.get("duration")?.as_u64()? as u32;

                                Some(Track {
                                    id,
                                    title,
                                    artist,
                                    album,
                                    duration_seconds: duration,
                                    album_cover_id,
                                })
                            })
                            .collect()
                    } else {
                        vec![]
                    };

                    // Parse albums
                    let albums = if let Some(albums_data) = json.get("albums").and_then(|a| a.get("items")) {
                        albums_data.as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(|item| {
                                let id = item.get("id")?.as_u64()?.to_string();
                                let title = item.get("title")?.as_str()?.to_string();
                                let artist = item.get("artist")
                                    .and_then(|a| a.get("name"))
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("Unknown Artist")
                                    .to_string();
                                let num_tracks = item.get("numberOfTracks")
                                    .and_then(|n| n.as_u64())
                                    .unwrap_or(0) as u32;

                                Some(Album {
                                    id,
                                    title,
                                    artist,
                                    num_tracks,
                                })
                            })
                            .collect()
                    } else {
                        vec![]
                    };

                    // Parse artists
                    let artists = if let Some(artists_data) = json.get("artists").and_then(|a| a.get("items")) {
                        artists_data.as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(|item| {
                                let id = item.get("id")?.as_u64()?;
                                let name = item.get("name")?.as_str()?.to_string();

                                Some(Artist { id, name })
                            })
                            .collect()
                    } else {
                        vec![]
                    };

                    return Ok(SearchResults {
                        tracks,
                        albums,
                        artists,
                    });
                }
                Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                    // Try to refresh token and retry
                    eprintln!("Got 401, attempting to refresh token...");
                    if self.refresh_token().await.is_ok() {
                        continue; // Retry with new token
                    }
                    return Err(anyhow!("Search failed: token refresh failed"));
                }
                Ok(resp) => {
                    return Err(anyhow!("Search failed with status: {}", resp.status()));
                }
                Err(e) => {
                    return Err(anyhow!("Network error during search: {}", e));
                }
            }
        }
        break; // Exit loop if no config
    }

        // Return empty results if no credentials
        Ok(SearchResults {
            tracks: vec![],
            albums: vec![],
            artists: vec![],
        })
    }

    // Token refresh functionality
    fn is_token_expired(&self) -> bool {
        if let Some(ref config) = self.config {
            if let Some(expires_at) = config.expires_at {
                // Check if token expires in less than 5 minutes
                return expires_at - Duration::minutes(5) < Utc::now();
            }
            // If no expiry is stored, don't force refresh
            // The token will be refreshed if we get a 401 error
            return false;
        }
        false
    }

    pub async fn refresh_token_if_needed(&mut self) -> Result<()> {
        if !self.is_token_expired() {
            return Ok(());
        }

        eprintln!("Token expired or expiring soon, attempting to refresh...");
        self.refresh_token().await
    }

    async fn refresh_token(&mut self) -> Result<()> {
        if let Some(ref mut config) = self.config {
            let refresh_token = config.refresh_token.clone();

            // Tidal OAuth2 refresh endpoint
            let url = "https://auth.tidal.com/v1/oauth2/token";

            // Use the correct client ID - this should match what upmpdcli uses
            let params = [
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh_token),
                ("client_id", "dN2N95wCyEBTllu4"),  // upmpdcli client ID
            ];

            let response = self.http_client
                .post(url)
                .form(&params)
                .send()
                .await?;

            if response.status().is_success() {
                let json: Value = response.json().await?;

                // Update tokens
                if let Some(access_token) = json.get("access_token").and_then(|v| v.as_str()) {
                    config.access_token = access_token.to_string();
                }

                if let Some(refresh_token) = json.get("refresh_token").and_then(|v| v.as_str()) {
                    config.refresh_token = refresh_token.to_string();
                }

                // Calculate and store expiry time
                if let Some(expires_in) = json.get("expires_in").and_then(|v| v.as_i64()) {
                    config.expires_at = Some(Utc::now() + Duration::seconds(expires_in));
                }

                // Save updated config
                self.save_config().await?;
                eprintln!("Token refreshed successfully!");

                Ok(())
            } else {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                Err(anyhow!("Failed to refresh token. Status: {} - {}", status, body))
            }
        } else {
            Err(anyhow!("No configuration available to refresh"))
        }
    }

    /// Get tracks from an album
    pub async fn get_album_tracks(&mut self, album_id: &str) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/albums/{}/items", album_id);

                let response = self.http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US"), ("limit", "100")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks = if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                            items.iter().filter_map(|item| {
                                // Album items have the track nested under "item"
                                let track_data = item.get("item")?;

                                let id = track_data.get("id")?.as_u64()?;
                                let title = track_data.get("title")?.as_str()?.to_string();

                                let artist = track_data.get("artist")
                                    .and_then(|a| a.get("name"))
                                    .and_then(|n| n.as_str())
                                    .or_else(|| {
                                        track_data.get("artists")
                                            .and_then(|a| a.as_array())
                                            .and_then(|arr| arr.first())
                                            .and_then(|a| a.get("name"))
                                            .and_then(|n| n.as_str())
                                    })
                                    .unwrap_or("Unknown Artist")
                                    .to_string();

                                let album = track_data.get("album")
                                    .and_then(|a| a.get("title"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("Unknown Album")
                                    .to_string();

                                let album_cover_id = track_data.get("album")
                                    .and_then(|a| a.get("cover"))
                                    .and_then(|c| c.as_str())
                                    .map(|s| s.to_string());

                                let duration = track_data.get("duration")?.as_u64()? as u32;

                                Some(Track {
                                    id,
                                    title,
                                    artist,
                                    album,
                                    duration_seconds: duration,
                                    album_cover_id,
                                })
                            }).collect()
                        } else {
                            vec![]
                        };

                        return Ok(tracks);
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        eprintln!("Album tracks request failed: {}", resp.status());
                    }
                    Err(e) => {
                        eprintln!("Network error fetching album tracks: {}", e);
                    }
                }
            }
            break;
        }

        Ok(vec![])
    }

    /// Get user's favorite tracks
    pub async fn get_favorite_tracks(&mut self) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/favorites/tracks",
                    config.user_id
                );

                let response = self.http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US"), ("limit", "100")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks = if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                            items.iter().filter_map(|item| {
                                // Favorite tracks have the track nested under "item"
                                let track_data = item.get("item")?;

                                let id = track_data.get("id")?.as_u64()?;
                                let title = track_data.get("title")?.as_str()?.to_string();

                                let artist = track_data.get("artist")
                                    .and_then(|a| a.get("name"))
                                    .and_then(|n| n.as_str())
                                    .or_else(|| {
                                        track_data.get("artists")
                                            .and_then(|a| a.as_array())
                                            .and_then(|arr| arr.first())
                                            .and_then(|a| a.get("name"))
                                            .and_then(|n| n.as_str())
                                    })
                                    .unwrap_or("Unknown Artist")
                                    .to_string();

                                let album = track_data.get("album")
                                    .and_then(|a| a.get("title"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("Unknown Album")
                                    .to_string();

                                let album_cover_id = track_data.get("album")
                                    .and_then(|a| a.get("cover"))
                                    .and_then(|c| c.as_str())
                                    .map(|s| s.to_string());

                                let duration = track_data.get("duration")?.as_u64()? as u32;

                                Some(Track {
                                    id,
                                    title,
                                    artist,
                                    album,
                                    duration_seconds: duration,
                                    album_cover_id,
                                })
                            }).collect()
                        } else {
                            vec![]
                        };

                        return Ok(tracks);
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        eprintln!("Favorite tracks request failed: {}", resp.status());
                    }
                    Err(e) => {
                        eprintln!("Network error fetching favorite tracks: {}", e);
                    }
                }
            }
            break;
        }

        Ok(vec![])
    }

    /// Get user's favorite albums
    pub async fn get_favorite_albums(&mut self) -> Result<Vec<Album>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/favorites/albums",
                    config.user_id
                );

                let response = self.http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US"), ("limit", "100")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let albums = if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                            items.iter().filter_map(|item| {
                                // Favorite albums have the album nested under "item"
                                let album_data = item.get("item")?;

                                let id = album_data.get("id")?.as_u64()?.to_string();
                                let title = album_data.get("title")?.as_str()?.to_string();
                                let artist = album_data.get("artist")
                                    .and_then(|a| a.get("name"))
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("Unknown Artist")
                                    .to_string();
                                let num_tracks = album_data.get("numberOfTracks")
                                    .and_then(|n| n.as_u64())
                                    .unwrap_or(0) as u32;

                                Some(Album {
                                    id,
                                    title,
                                    artist,
                                    num_tracks,
                                })
                            }).collect()
                        } else {
                            vec![]
                        };

                        return Ok(albums);
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        eprintln!("Favorite albums request failed: {}", resp.status());
                    }
                    Err(e) => {
                        eprintln!("Network error fetching favorite albums: {}", e);
                    }
                }
            }
            break;
        }

        Ok(vec![])
    }

    /// Get user's favorite artists
    pub async fn get_favorite_artists(&mut self) -> Result<Vec<Artist>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/favorites/artists",
                    config.user_id
                );

                let response = self.http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US"), ("limit", "100")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let artists = if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                            items.iter().filter_map(|item| {
                                // Favorite artists have the artist nested under "item"
                                let artist_data = item.get("item")?;

                                let id = artist_data.get("id")?.as_u64()?;
                                let name = artist_data.get("name")?.as_str()?.to_string();

                                Some(Artist { id, name })
                            }).collect()
                        } else {
                            vec![]
                        };

                        return Ok(artists);
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        eprintln!("Favorite artists request failed: {}", resp.status());
                    }
                    Err(e) => {
                        eprintln!("Network error fetching favorite artists: {}", e);
                    }
                }
            }
            break;
        }

        Ok(vec![])
    }

    /// Add a track to favorites
    pub async fn add_favorite_track(&mut self, track_id: u64) -> Result<()> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/favorites/tracks",
                    config.user_id
                );

                let response = self.http_client
                    .post(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US")])
                    .form(&[("trackIds", track_id.to_string())])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 200 || resp.status().as_u16() == 201 => {
                        return Ok(());
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        return Err(anyhow!("Failed to add favorite: {} - {}", status, body));
                    }
                    Err(e) => {
                        return Err(anyhow!("Network error adding favorite: {}", e));
                    }
                }
            }
            break;
        }

        Err(anyhow!("No configuration available"))
    }

    /// Remove a track from favorites
    pub async fn remove_favorite_track(&mut self, track_id: u64) -> Result<()> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/favorites/tracks/{}",
                    config.user_id, track_id
                );

                let response = self.http_client
                    .delete(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 200 || resp.status().as_u16() == 204 => {
                        return Ok(());
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        return Err(anyhow!("Failed to remove favorite: {} - {}", status, body));
                    }
                    Err(e) => {
                        return Err(anyhow!("Network error removing favorite: {}", e));
                    }
                }
            }
            break;
        }

        Err(anyhow!("No configuration available"))
    }

    /// Get top tracks for an artist
    pub async fn get_artist_top_tracks(&mut self, artist_id: u64) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/artists/{}/toptracks", artist_id);

                let response = self.http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US"), ("limit", "20")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks = if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                            items.iter().filter_map(|item| {
                                let id = item.get("id")?.as_u64()?;
                                let title = item.get("title")?.as_str()?.to_string();

                                let artist = item.get("artist")
                                    .and_then(|a| a.get("name"))
                                    .and_then(|n| n.as_str())
                                    .or_else(|| {
                                        item.get("artists")
                                            .and_then(|a| a.as_array())
                                            .and_then(|arr| arr.first())
                                            .and_then(|a| a.get("name"))
                                            .and_then(|n| n.as_str())
                                    })
                                    .unwrap_or("Unknown Artist")
                                    .to_string();

                                let album = item.get("album")
                                    .and_then(|a| a.get("title"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("Unknown Album")
                                    .to_string();

                                let album_cover_id = item.get("album")
                                    .and_then(|a| a.get("cover"))
                                    .and_then(|c| c.as_str())
                                    .map(|s| s.to_string());

                                let duration = item.get("duration")?.as_u64()? as u32;

                                Some(Track {
                                    id,
                                    title,
                                    artist,
                                    album,
                                    duration_seconds: duration,
                                    album_cover_id,
                                })
                            }).collect()
                        } else {
                            vec![]
                        };

                        return Ok(tracks);
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        eprintln!("Artist top tracks request failed: {}", resp.status());
                    }
                    Err(e) => {
                        eprintln!("Network error fetching artist top tracks: {}", e);
                    }
                }
            }
            break;
        }

        Ok(vec![])
    }

    /// Get track radio (similar tracks)
    pub async fn get_track_radio(&mut self, track_id: u64, limit: usize) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/tracks/{}/radio", track_id);

                let response = self.http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US"), ("limit", &limit.to_string())])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks = if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                            items.iter().filter_map(|item| {
                                let id = item.get("id")?.as_u64()?;
                                let title = item.get("title")?.as_str()?.to_string();

                                let artist = item.get("artist")
                                    .and_then(|a| a.get("name"))
                                    .and_then(|n| n.as_str())
                                    .or_else(|| {
                                        item.get("artists")
                                            .and_then(|a| a.as_array())
                                            .and_then(|arr| arr.first())
                                            .and_then(|a| a.get("name"))
                                            .and_then(|n| n.as_str())
                                    })
                                    .unwrap_or("Unknown Artist")
                                    .to_string();

                                let album = item.get("album")
                                    .and_then(|a| a.get("title"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("Unknown Album")
                                    .to_string();

                                let album_cover_id = item.get("album")
                                    .and_then(|a| a.get("cover"))
                                    .and_then(|c| c.as_str())
                                    .map(|s| s.to_string());

                                let duration = item.get("duration")?.as_u64()? as u32;

                                Some(Track {
                                    id,
                                    title,
                                    artist,
                                    album,
                                    duration_seconds: duration,
                                    album_cover_id,
                                })
                            }).collect()
                        } else {
                            vec![]
                        };

                        return Ok(tracks);
                    }
                    Ok(resp) if resp.status().as_u16() == 404 => {
                        // Some tracks don't have radio, return empty
                        return Ok(vec![]);
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        eprintln!("Track radio request failed: {}", resp.status());
                    }
                    Err(e) => {
                        eprintln!("Network error fetching track radio: {}", e);
                    }
                }
            }
            break;
        }

        Ok(vec![])
    }

    /// Get playlist radio (mix radio - similar tracks based on playlist)
    pub async fn get_playlist_radio(&mut self, playlist_id: &str, limit: usize) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/playlists/{}/radio", playlist_id);

                let response = self.http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US"), ("limit", &limit.to_string())])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks = if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                            items.iter().filter_map(|item| {
                                let id = item.get("id")?.as_u64()?;
                                let title = item.get("title")?.as_str()?.to_string();

                                let artist = item.get("artist")
                                    .and_then(|a| a.get("name"))
                                    .and_then(|n| n.as_str())
                                    .or_else(|| {
                                        item.get("artists")
                                            .and_then(|a| a.as_array())
                                            .and_then(|arr| arr.first())
                                            .and_then(|a| a.get("name"))
                                            .and_then(|n| n.as_str())
                                    })
                                    .unwrap_or("Unknown Artist")
                                    .to_string();

                                let album = item.get("album")
                                    .and_then(|a| a.get("title"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("Unknown Album")
                                    .to_string();

                                let album_cover_id = item.get("album")
                                    .and_then(|a| a.get("cover"))
                                    .and_then(|c| c.as_str())
                                    .map(|s| s.to_string());

                                let duration = item.get("duration")?.as_u64()? as u32;

                                Some(Track {
                                    id,
                                    title,
                                    artist,
                                    album,
                                    duration_seconds: duration,
                                    album_cover_id,
                                })
                            }).collect()
                        } else {
                            vec![]
                        };

                        return Ok(tracks);
                    }
                    Ok(resp) if resp.status().as_u16() == 404 => {
                        // Playlist radio not available
                        return Ok(vec![]);
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        eprintln!("Playlist radio request failed: {}", resp.status());
                    }
                    Err(e) => {
                        eprintln!("Network error fetching playlist radio: {}", e);
                    }
                }
            }
            break;
        }

        Ok(vec![])
    }

    /// Get artist radio (similar tracks based on artist)
    pub async fn get_artist_radio(&mut self, artist_id: u64, limit: usize) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/artists/{}/radio", artist_id);

                let response = self.http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US"), ("limit", &limit.to_string())])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks = if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                            items.iter().filter_map(|item| {
                                let id = item.get("id")?.as_u64()?;
                                let title = item.get("title")?.as_str()?.to_string();

                                let artist = item.get("artist")
                                    .and_then(|a| a.get("name"))
                                    .and_then(|n| n.as_str())
                                    .or_else(|| {
                                        item.get("artists")
                                            .and_then(|a| a.as_array())
                                            .and_then(|arr| arr.first())
                                            .and_then(|a| a.get("name"))
                                            .and_then(|n| n.as_str())
                                    })
                                    .unwrap_or("Unknown Artist")
                                    .to_string();

                                let album = item.get("album")
                                    .and_then(|a| a.get("title"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("Unknown Album")
                                    .to_string();

                                let album_cover_id = item.get("album")
                                    .and_then(|a| a.get("cover"))
                                    .and_then(|c| c.as_str())
                                    .map(|s| s.to_string());

                                let duration = item.get("duration")?.as_u64()? as u32;

                                Some(Track {
                                    id,
                                    title,
                                    artist,
                                    album,
                                    duration_seconds: duration,
                                    album_cover_id,
                                })
                            }).collect()
                        } else {
                            vec![]
                        };

                        return Ok(tracks);
                    }
                    Ok(resp) if resp.status().as_u16() == 404 => {
                        // Artist radio not available
                        return Ok(vec![]);
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        eprintln!("Artist radio request failed: {}", resp.status());
                    }
                    Err(e) => {
                        eprintln!("Network error fetching artist radio: {}", e);
                    }
                }
            }
            break;
        }

        Ok(vec![])
    }

    // ========== Playlist Management ==========

    /// Create a new playlist
    /// Uses the v2 API: PUT my-collection/playlists/folders/create-playlist
    pub async fn create_playlist(&mut self, name: &str, description: Option<&str>) -> Result<Playlist> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = "https://api.tidal.com/v2/my-collection/playlists/folders/create-playlist";

                let mut query_params = vec![
                    ("name", name.to_string()),
                    ("folderId", "root".to_string()),
                ];
                if let Some(desc) = description {
                    query_params.push(("description", desc.to_string()));
                }

                let response = self.http_client
                    .put(url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&query_params)
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        // Response structure: { "data": { "uuid": "...", "title": "...", ... } }
                        let data = json.get("data").ok_or_else(|| anyhow!("Missing data field"))?;

                        let uuid = data.get("uuid")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("Missing uuid"))?
                            .to_string();

                        let title = data.get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or(name)
                            .to_string();

                        let description = data.get("description")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        let num_tracks = data.get("numberOfTracks")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;

                        return Ok(Playlist {
                            id: uuid,
                            title,
                            description,
                            num_tracks,
                        });
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        return Err(anyhow!("Failed to create playlist: {} - {}", status, body));
                    }
                    Err(e) => {
                        return Err(anyhow!("Network error creating playlist: {}", e));
                    }
                }
            }
            break;
        }

        Err(anyhow!("No configuration available"))
    }

    /// Add tracks to a playlist
    /// POST playlists/{playlist_id}/items with trackIds in form data
    pub async fn add_tracks_to_playlist(&mut self, playlist_id: &str, track_ids: &[u64]) -> Result<()> {
        if track_ids.is_empty() {
            return Ok(());
        }

        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/playlists/{}/items", playlist_id);

                let track_ids_str = track_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(",");

                let response = self.http_client
                    .post(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US")])
                    .form(&[
                        ("trackIds", track_ids_str.as_str()),
                        ("onArtifactNotFound", "SKIP"),
                        ("onDupes", "ADD"),
                    ])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 200 || resp.status().as_u16() == 201 => {
                        return Ok(());
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        return Err(anyhow!("Failed to add tracks to playlist: {} - {}", status, body));
                    }
                    Err(e) => {
                        return Err(anyhow!("Network error adding tracks: {}", e));
                    }
                }
            }
            break;
        }

        Err(anyhow!("No configuration available"))
    }

    /// Remove tracks from a playlist by their indices (0-based positions)
    /// DELETE playlists/{playlist_id}/items/{indices}
    pub async fn remove_tracks_from_playlist(&mut self, playlist_id: &str, indices: &[usize]) -> Result<()> {
        if indices.is_empty() {
            return Ok(());
        }

        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let indices_str = indices
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(",");

                let url = format!(
                    "https://api.tidal.com/v1/playlists/{}/items/{}",
                    playlist_id, indices_str
                );

                let response = self.http_client
                    .delete(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 200 || resp.status().as_u16() == 204 => {
                        return Ok(());
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        return Err(anyhow!("Failed to remove tracks: {} - {}", status, body));
                    }
                    Err(e) => {
                        return Err(anyhow!("Network error removing tracks: {}", e));
                    }
                }
            }
            break;
        }

        Err(anyhow!("No configuration available"))
    }

    /// Delete a playlist
    /// DELETE playlists/{playlist_id}
    pub async fn delete_playlist(&mut self, playlist_id: &str) -> Result<()> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/playlists/{}", playlist_id);

                let response = self.http_client
                    .delete(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 200 || resp.status().as_u16() == 204 => {
                        return Ok(());
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        return Err(anyhow!("Failed to delete playlist: {} - {}", status, body));
                    }
                    Err(e) => {
                        return Err(anyhow!("Network error deleting playlist: {}", e));
                    }
                }
            }
            break;
        }

        Err(anyhow!("No configuration available"))
    }

    /// Rename/update a playlist
    /// POST playlists/{playlist_id} with title and/or description
    pub async fn update_playlist(&mut self, playlist_id: &str, title: Option<&str>, description: Option<&str>) -> Result<()> {
        if title.is_none() && description.is_none() {
            return Ok(());
        }

        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/playlists/{}", playlist_id);

                let mut form_params: Vec<(&str, &str)> = Vec::new();
                if let Some(t) = title {
                    form_params.push(("title", t));
                }
                if let Some(d) = description {
                    form_params.push(("description", d));
                }

                let response = self.http_client
                    .post(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .form(&form_params)
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 200 => {
                        return Ok(());
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        return Err(anyhow!("Failed to update playlist: {} - {}", status, body));
                    }
                    Err(e) => {
                        return Err(anyhow!("Network error updating playlist: {}", e));
                    }
                }
            }
            break;
        }

        Err(anyhow!("No configuration available"))
    }

    // ========== End Playlist Management ==========

    /// Get albums for an artist (discography)
    pub async fn get_artist_albums(&mut self, artist_id: u64) -> Result<Vec<Album>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/artists/{}/albums", artist_id);

                let response = self.http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", config.access_token))
                    .query(&[("countryCode", "US"), ("limit", "50")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let albums = if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                            items.iter().filter_map(|item| {
                                let id = item.get("id")?.as_u64()?.to_string();
                                let title = item.get("title")?.as_str()?.to_string();

                                let artist = item.get("artist")
                                    .and_then(|a| a.get("name"))
                                    .and_then(|n| n.as_str())
                                    .or_else(|| {
                                        item.get("artists")
                                            .and_then(|a| a.as_array())
                                            .and_then(|arr| arr.first())
                                            .and_then(|a| a.get("name"))
                                            .and_then(|n| n.as_str())
                                    })
                                    .unwrap_or("Unknown Artist")
                                    .to_string();

                                let num_tracks = item.get("numberOfTracks")
                                    .and_then(|n| n.as_u64())
                                    .unwrap_or(0) as u32;

                                Some(Album {
                                    id,
                                    title,
                                    artist,
                                    num_tracks,
                                })
                            }).collect()
                        } else {
                            vec![]
                        };

                        return Ok(albums);
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        if self.refresh_token().await.is_ok() {
                            continue;
                        }
                    }
                    Ok(resp) => {
                        eprintln!("Artist albums request failed: {}", resp.status());
                    }
                    Err(e) => {
                        eprintln!("Network error fetching artist albums: {}", e);
                    }
                }
            }
            break;
        }

        Ok(vec![])
    }
}