use anyhow::{anyhow, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Duration, Utc};
use dirs::config_dir;
use reqwest::{header, Client as HttpClient};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use super::{Album, Artist, CoverArt, MusicService, Playlist, SearchResults, ServiceType, Track};

#[derive(Debug, Serialize, Deserialize)]
pub struct TidalConfig {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub user_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

// API Response models
#[derive(Debug, Deserialize)]
struct PlaylistsResponse {
    items: Option<Vec<PlaylistItem>>,
    #[serde(rename = "totalNumberOfItems")]
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    cover: Option<String>,
}

pub struct TidalClient {
    pub config: Option<TidalConfig>,
    http_client: HttpClient,
    audio_quality: String,
}

impl TidalClient {
    pub async fn new() -> Result<Self> {
        let upmpdcli_path = Self::upmpdcli_path()?;
        let tidal_tui_path = Self::config_path()?;

        let config = if upmpdcli_path.exists() {
            println!("Loading existing upmpdcli Tidal credentials...");
            let config = Self::load_config(&upmpdcli_path)?;
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

        Ok(Self {
            config,
            http_client,
            audio_quality: "HIGH".to_string(),
        })
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

    fn get_demo_playlists(&self) -> Vec<Playlist> {
        vec![
            Playlist {
                id: "demo-1".to_string(),
                title: "My Mix 1".to_string(),
                description: Some("Your personalized mix".to_string()),
                num_tracks: 50,
                service: ServiceType::Tidal,
            },
            Playlist {
                id: "demo-2".to_string(),
                title: "Favorites".to_string(),
                description: Some("Your favorite tracks".to_string()),
                num_tracks: 123,
                service: ServiceType::Tidal,
            },
            Playlist {
                id: "demo-3".to_string(),
                title: "Recently Played".to_string(),
                description: Some("Recently played tracks".to_string()),
                num_tracks: 25,
                service: ServiceType::Tidal,
            },
        ]
    }

    fn get_demo_tracks(&self) -> Vec<Track> {
        vec![
            Track {
                id: "1".to_string(),
                title: "Bohemian Rhapsody".to_string(),
                artist: "Queen".to_string(),
                album: "A Night at the Opera".to_string(),
                duration_seconds: 354,
                cover_art: CoverArt::None,
                service: ServiceType::Tidal,
            },
            Track {
                id: "2".to_string(),
                title: "Stairway to Heaven".to_string(),
                artist: "Led Zeppelin".to_string(),
                album: "Led Zeppelin IV".to_string(),
                duration_seconds: 482,
                cover_art: CoverArt::None,
                service: ServiceType::Tidal,
            },
            Track {
                id: "3".to_string(),
                title: "Hotel California".to_string(),
                artist: "Eagles".to_string(),
                album: "Hotel California".to_string(),
                duration_seconds: 391,
                cover_art: CoverArt::None,
                service: ServiceType::Tidal,
            },
        ]
    }

    #[allow(dead_code)]
    fn is_token_expired(&self) -> bool {
        if let Some(ref config) = self.config {
            if let Some(expires_at) = config.expires_at {
                return expires_at - Duration::minutes(5) < Utc::now();
            }
            return false;
        }
        false
    }

    #[allow(dead_code)]
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
            let url = "https://auth.tidal.com/v1/oauth2/token";

            let params = [
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh_token),
                ("client_id", "dN2N95wCyEBTllu4"),
            ];

            let response = self.http_client.post(url).form(&params).send().await?;

            if response.status().is_success() {
                let json: Value = response.json().await?;

                if let Some(access_token) = json.get("access_token").and_then(|v| v.as_str()) {
                    config.access_token = access_token.to_string();
                }

                if let Some(refresh_token) = json.get("refresh_token").and_then(|v| v.as_str()) {
                    config.refresh_token = refresh_token.to_string();
                }

                if let Some(expires_in) = json.get("expires_in").and_then(|v| v.as_i64()) {
                    config.expires_at = Some(Utc::now() + Duration::seconds(expires_in));
                }

                self.save_config().await?;
                eprintln!("Token refreshed successfully!");

                Ok(())
            } else {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                Err(anyhow!(
                    "Failed to refresh token. Status: {} - {}",
                    status,
                    body
                ))
            }
        } else {
            Err(anyhow!("No configuration available to refresh"))
        }
    }

    // Helper to parse track from JSON
    fn parse_track_from_json(item: &Value) -> Option<Track> {
        let id = item.get("id")?.as_u64()?.to_string();
        let title = item.get("title")?.as_str()?.to_string();

        let artist = item
            .get("artist")
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

        let album = item
            .get("album")
            .and_then(|a| a.get("title"))
            .and_then(|t| t.as_str())
            .unwrap_or("Unknown Album")
            .to_string();

        let album_cover_id = item
            .get("album")
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
            cover_art: CoverArt::from_tidal_option(album_cover_id),
            service: ServiceType::Tidal,
        })
    }

    // Helper to parse track from nested item structure
    fn parse_track_from_nested(item: &Value) -> Option<Track> {
        let track_data = item.get("item")?;
        Self::parse_track_from_json(track_data)
    }

    // Get tracks from playlist (internal method)
    pub async fn get_tracks(&mut self, playlist_id: &str) -> Result<Vec<Track>> {
        self.get_playlist_tracks(playlist_id).await
    }

    // Static helper for formatting
    pub fn format_track_display(track: &Track) -> String {
        format!(
            "{} - {} ({}:{:02})",
            track.artist,
            track.title,
            track.duration_seconds / 60,
            track.duration_seconds % 60
        )
    }

    pub fn format_playlist_display(playlist: &Playlist) -> String {
        format!("{} ({} tracks)", playlist.title, playlist.num_tracks)
    }

    // Legacy method for Tidal-specific track ID operations
    pub async fn add_favorite_track_by_id(&mut self, track_id: u64) -> Result<()> {
        self.add_favorite_track(&track_id.to_string()).await
    }

    pub async fn remove_favorite_track_by_id(&mut self, track_id: u64) -> Result<()> {
        self.remove_favorite_track(&track_id.to_string()).await
    }

    pub async fn get_track_radio_by_id(&mut self, track_id: u64, limit: usize) -> Result<Vec<Track>> {
        self.get_track_radio(&track_id.to_string(), limit).await
    }

    pub async fn get_artist_radio_by_id(&mut self, artist_id: u64, limit: usize) -> Result<Vec<Track>> {
        self.get_artist_radio(&artist_id.to_string(), limit).await
    }

    pub async fn get_artist_top_tracks_by_id(&mut self, artist_id: u64) -> Result<Vec<Track>> {
        self.get_artist_top_tracks(&artist_id.to_string()).await
    }

    pub async fn get_artist_albums_by_id(&mut self, artist_id: u64) -> Result<Vec<Album>> {
        self.get_artist_albums(&artist_id.to_string()).await
    }

    pub async fn add_tracks_to_playlist_by_ids(&mut self, playlist_id: &str, track_ids: &[u64]) -> Result<()> {
        let string_ids: Vec<String> = track_ids.iter().map(|id| id.to_string()).collect();
        self.add_tracks_to_playlist(playlist_id, &string_ids).await
    }
}

#[async_trait]
impl MusicService for TidalClient {
    fn service_type(&self) -> ServiceType {
        ServiceType::Tidal
    }

    fn is_authenticated(&self) -> bool {
        self.config.is_some()
    }

    fn set_audio_quality(&mut self, quality: &str) {
        self.audio_quality = match quality.to_lowercase().as_str() {
            "low" => "LOW",
            "high" => "HIGH",
            "lossless" => "LOSSLESS",
            "master" | "hifi" | "hi_res" => "HI_RES",
            _ => "HIGH",
        }
        .to_string();
    }

    async fn get_stream_url(&mut self, track_id: &str) -> Result<String> {
        if !track_id.starts_with("demo") {
            for attempt in 0..2 {
                let (token, _user_id) = if let Some(ref config) = self.config {
                    (config.access_token.clone(), config.user_id)
                } else {
                    break;
                };

                let url = format!(
                    "https://api.tidal.com/v1/tracks/{}/playbackinfo",
                    track_id
                );

                let response = self
                    .http_client
                    .get(&url)
                    .header(header::AUTHORIZATION, format!("Bearer {}", token))
                    .query(&[
                        ("countryCode", "US"),
                        ("assetpresentation", "FULL"),
                        ("audioquality", self.audio_quality.as_str()),
                        ("playbackmode", "STREAM"),
                    ])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        if let Some(manifest) = json.get("manifest").and_then(|v| v.as_str()) {
                            if let Ok(decoded) = general_purpose::STANDARD.decode(manifest) {
                                if let Ok(manifest_str) = String::from_utf8(decoded) {
                                    if let Ok(manifest_json) =
                                        serde_json::from_str::<Value>(&manifest_str)
                                    {
                                        if let Some(urls) =
                                            manifest_json.get("urls").and_then(|u| u.as_array())
                                        {
                                            if let Some(first_url) =
                                                urls.first().and_then(|u| u.as_str())
                                            {
                                                return Ok(first_url.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(url) = json.get("url").and_then(|v| v.as_str()) {
                            return Ok(url.to_string());
                        }

                        if let Some(urls) = json.get("urls").and_then(|v| v.as_array()) {
                            if let Some(first_url) = urls.first().and_then(|u| u.as_str()) {
                                return Ok(first_url.to_string());
                            }
                        }

                        return Err(anyhow!("Could not find stream URL in response"));
                    }
                    Ok(resp) => {
                        let status = resp.status();

                        if status.as_u16() == 401 && attempt == 0 {
                            eprintln!("Got 401 for stream URL, attempting to refresh token...");
                            if self.refresh_token().await.is_ok() {
                                continue;
                            }
                            return Err(anyhow!("Failed to get stream URL after token refresh"));
                        }

                        if status.as_u16() == 401 || status.as_u16() == 403 {
                            let stream_url = format!(
                                "https://api.tidal.com/v1/tracks/{}/streamUrl",
                                track_id
                            );

                            let stream_response = self
                                .http_client
                                .get(&stream_url)
                                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                                .query(&[
                                    ("countryCode", "US"),
                                    ("soundQuality", self.audio_quality.as_str()),
                                    ("assetpresentation", "FULL"),
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

                        return Err(anyhow!("Failed to get stream URL. Status: {}", status));
                    }
                    Err(e) => {
                        return Err(anyhow!("Network error getting stream URL: {}", e));
                    }
                }
            }
        }

        Ok(format!("tidal://track/{}", track_id))
    }

    async fn get_playlists(&mut self) -> Result<Vec<Playlist>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/playlists",
                    config.user_id
                );

                let response = self
                    .http_client
                    .get(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US"), ("limit", "50")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let playlists_resp: PlaylistsResponse = resp.json().await?;

                        if let Some(items) = playlists_resp.items {
                            let playlists = items
                                .into_iter()
                                .map(|item| Playlist {
                                    id: item.uuid,
                                    title: item.title,
                                    description: item.description,
                                    num_tracks: item.number_of_tracks.unwrap_or(0) as usize,
                                    service: ServiceType::Tidal,
                                })
                                .collect();
                            return Ok(playlists);
                        }
                    }
                    Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                        eprintln!("Got 401, attempting to refresh token...");
                        if self.refresh_token().await.is_ok() {
                            continue;
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
            break;
        }

        Ok(self.get_demo_playlists())
    }

    async fn get_playlist_tracks(&mut self, playlist_id: &str) -> Result<Vec<Track>> {
        if !playlist_id.starts_with("demo-") {
            for attempt in 0..2 {
                if let Some(ref config) = self.config {
                    let url = format!(
                        "https://api.tidal.com/v1/playlists/{}/items",
                        playlist_id
                    );

                    let response = self
                        .http_client
                        .get(&url)
                        .header(
                            header::AUTHORIZATION,
                            format!("Bearer {}", config.access_token),
                        )
                        .query(&[("countryCode", "US"), ("limit", "100")])
                        .send()
                        .await;

                    match response {
                        Ok(resp) if resp.status().is_success() => {
                            let tracks_resp: TracksResponse = resp.json().await?;

                            if let Some(items) = tracks_resp.items {
                                let tracks = items
                                    .into_iter()
                                    .filter_map(|item| {
                                        item.item.map(|track| {
                                            let artist_name = track
                                                .artist
                                                .or_else(|| {
                                                    track.artists.and_then(|a| a.into_iter().next())
                                                })
                                                .map(|a| a.name)
                                                .unwrap_or_else(|| "Unknown Artist".to_string());

                                            let (album_title, album_cover_id) = track
                                                .album
                                                .map(|a| (a.title, a.cover))
                                                .unwrap_or_else(|| {
                                                    ("Unknown Album".to_string(), None)
                                                });

                                            Track {
                                                id: track.id.to_string(),
                                                title: track.title,
                                                artist: artist_name,
                                                album: album_title,
                                                duration_seconds: track.duration.unwrap_or(0),
                                                cover_art: CoverArt::from_tidal_option(
                                                    album_cover_id,
                                                ),
                                                service: ServiceType::Tidal,
                                            }
                                        })
                                    })
                                    .collect();
                                return Ok(tracks);
                            }
                        }
                        Ok(resp) if resp.status().as_u16() == 401 && attempt == 0 => {
                            eprintln!("Got 401, attempting to refresh token...");
                            if self.refresh_token().await.is_ok() {
                                continue;
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
                break;
            }
        }

        Ok(self.get_demo_tracks())
    }

    async fn get_favorite_tracks(&mut self) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/favorites/tracks",
                    config.user_id
                );

                let response = self
                    .http_client
                    .get(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US"), ("limit", "100")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks = if let Some(items) = json.get("items").and_then(|i| i.as_array())
                        {
                            items
                                .iter()
                                .filter_map(Self::parse_track_from_nested)
                                .collect()
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

    async fn get_favorite_albums(&mut self) -> Result<Vec<Album>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/favorites/albums",
                    config.user_id
                );

                let response = self
                    .http_client
                    .get(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US"), ("limit", "100")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let albums =
                            if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                                items
                                    .iter()
                                    .filter_map(|item| {
                                        let album_data = item.get("item")?;

                                        let id = album_data.get("id")?.as_u64()?.to_string();
                                        let title =
                                            album_data.get("title")?.as_str()?.to_string();
                                        let artist = album_data
                                            .get("artist")
                                            .and_then(|a| a.get("name"))
                                            .and_then(|n| n.as_str())
                                            .unwrap_or("Unknown Artist")
                                            .to_string();
                                        let num_tracks = album_data
                                            .get("numberOfTracks")
                                            .and_then(|n| n.as_u64())
                                            .unwrap_or(0)
                                            as u32;

                                        let cover_id = album_data
                                            .get("cover")
                                            .and_then(|c| c.as_str())
                                            .map(|s| s.to_string());

                                        Some(Album {
                                            id,
                                            title,
                                            artist,
                                            num_tracks,
                                            cover_art: CoverArt::from_tidal_option(cover_id),
                                            service: ServiceType::Tidal,
                                        })
                                    })
                                    .collect()
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

    async fn get_favorite_artists(&mut self) -> Result<Vec<Artist>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/favorites/artists",
                    config.user_id
                );

                let response = self
                    .http_client
                    .get(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US"), ("limit", "100")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let artists =
                            if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                                items
                                    .iter()
                                    .filter_map(|item| {
                                        let artist_data = item.get("item")?;

                                        let id = artist_data.get("id")?.as_u64()?.to_string();
                                        let name =
                                            artist_data.get("name")?.as_str()?.to_string();

                                        Some(Artist {
                                            id,
                                            name,
                                            service: ServiceType::Tidal,
                                        })
                                    })
                                    .collect()
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

    async fn add_favorite_track(&mut self, track_id: &str) -> Result<()> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/favorites/tracks",
                    config.user_id
                );

                let response = self
                    .http_client
                    .post(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US")])
                    .form(&[("trackIds", track_id)])
                    .send()
                    .await;

                match response {
                    Ok(resp)
                        if resp.status().is_success()
                            || resp.status().as_u16() == 200
                            || resp.status().as_u16() == 201 =>
                    {
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

    async fn remove_favorite_track(&mut self, track_id: &str) -> Result<()> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/users/{}/favorites/tracks/{}",
                    config.user_id, track_id
                );

                let response = self
                    .http_client
                    .delete(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US")])
                    .send()
                    .await;

                match response {
                    Ok(resp)
                        if resp.status().is_success()
                            || resp.status().as_u16() == 200
                            || resp.status().as_u16() == 204 =>
                    {
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

    async fn search(&mut self, query: &str, limit: usize) -> Result<SearchResults> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = "https://api.tidal.com/v1/search";

                let response = self
                    .http_client
                    .get(url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
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

                        let tracks = if let Some(tracks_data) =
                            json.get("tracks").and_then(|t| t.get("items"))
                        {
                            tracks_data
                                .as_array()
                                .unwrap_or(&vec![])
                                .iter()
                                .filter_map(|item| {
                                    let id = item.get("id")?.as_u64()?.to_string();
                                    let title = item.get("title")?.as_str()?.to_string();

                                    let artist = item
                                        .get("artists")
                                        .and_then(|artists| artists.as_array())
                                        .map(|arr| {
                                            let names: Vec<&str> = arr
                                                .iter()
                                                .filter_map(|a| {
                                                    a.get("name").and_then(|n| n.as_str())
                                                })
                                                .collect();
                                            if names.is_empty() {
                                                "Unknown Artist".to_string()
                                            } else {
                                                names.join(", ")
                                            }
                                        })
                                        .or_else(|| {
                                            item.get("artist")
                                                .and_then(|a| a.get("name"))
                                                .and_then(|n| n.as_str())
                                                .map(|s| s.to_string())
                                        })
                                        .unwrap_or_else(|| "Unknown Artist".to_string());

                                    let album = item
                                        .get("album")
                                        .and_then(|a| a.get("title"))
                                        .and_then(|t| t.as_str())
                                        .unwrap_or("Unknown Album")
                                        .to_string();

                                    let album_cover_id = item
                                        .get("album")
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
                                        cover_art: CoverArt::from_tidal_option(album_cover_id),
                                        service: ServiceType::Tidal,
                                    })
                                })
                                .collect()
                        } else {
                            vec![]
                        };

                        let albums = if let Some(albums_data) =
                            json.get("albums").and_then(|a| a.get("items"))
                        {
                            albums_data
                                .as_array()
                                .unwrap_or(&vec![])
                                .iter()
                                .filter_map(|item| {
                                    let id = item.get("id")?.as_u64()?.to_string();
                                    let title = item.get("title")?.as_str()?.to_string();
                                    let artist = item
                                        .get("artist")
                                        .and_then(|a| a.get("name"))
                                        .and_then(|n| n.as_str())
                                        .unwrap_or("Unknown Artist")
                                        .to_string();
                                    let num_tracks = item
                                        .get("numberOfTracks")
                                        .and_then(|n| n.as_u64())
                                        .unwrap_or(0)
                                        as u32;

                                    let cover_id = item
                                        .get("cover")
                                        .and_then(|c| c.as_str())
                                        .map(|s| s.to_string());

                                    Some(Album {
                                        id,
                                        title,
                                        artist,
                                        num_tracks,
                                        cover_art: CoverArt::from_tidal_option(cover_id),
                                        service: ServiceType::Tidal,
                                    })
                                })
                                .collect()
                        } else {
                            vec![]
                        };

                        let artists = if let Some(artists_data) =
                            json.get("artists").and_then(|a| a.get("items"))
                        {
                            artists_data
                                .as_array()
                                .unwrap_or(&vec![])
                                .iter()
                                .filter_map(|item| {
                                    let id = item.get("id")?.as_u64()?.to_string();
                                    let name = item.get("name")?.as_str()?.to_string();

                                    Some(Artist {
                                        id,
                                        name,
                                        service: ServiceType::Tidal,
                                    })
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
                        eprintln!("Got 401, attempting to refresh token...");
                        if self.refresh_token().await.is_ok() {
                            continue;
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
            break;
        }

        Ok(SearchResults::default())
    }

    async fn get_album_tracks(&mut self, album_id: &str) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/albums/{}/items", album_id);

                let response = self
                    .http_client
                    .get(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US"), ("limit", "100")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks =
                            if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                                items
                                    .iter()
                                    .filter_map(Self::parse_track_from_nested)
                                    .collect()
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

    async fn get_artist_top_tracks(&mut self, artist_id: &str) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/artists/{}/toptracks",
                    artist_id
                );

                let response = self
                    .http_client
                    .get(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US"), ("limit", "20")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks =
                            if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                                items
                                    .iter()
                                    .filter_map(Self::parse_track_from_json)
                                    .collect()
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

    async fn get_artist_albums(&mut self, artist_id: &str) -> Result<Vec<Album>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/artists/{}/albums", artist_id);

                let response = self
                    .http_client
                    .get(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US"), ("limit", "50")])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let albums =
                            if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                                items
                                    .iter()
                                    .filter_map(|item| {
                                        let id = item.get("id")?.as_u64()?.to_string();
                                        let title = item.get("title")?.as_str()?.to_string();

                                        let artist = item
                                            .get("artist")
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

                                        let num_tracks = item
                                            .get("numberOfTracks")
                                            .and_then(|n| n.as_u64())
                                            .unwrap_or(0)
                                            as u32;

                                        let cover_id = item
                                            .get("cover")
                                            .and_then(|c| c.as_str())
                                            .map(|s| s.to_string());

                                        Some(Album {
                                            id,
                                            title,
                                            artist,
                                            num_tracks,
                                            cover_art: CoverArt::from_tidal_option(cover_id),
                                            service: ServiceType::Tidal,
                                        })
                                    })
                                    .collect()
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

    async fn get_track_radio(&mut self, track_id: &str, limit: usize) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/tracks/{}/radio", track_id);

                let response = self
                    .http_client
                    .get(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US"), ("limit", &limit.to_string())])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks =
                            if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                                items
                                    .iter()
                                    .filter_map(Self::parse_track_from_json)
                                    .collect()
                            } else {
                                vec![]
                            };

                        return Ok(tracks);
                    }
                    Ok(resp) if resp.status().as_u16() == 404 => {
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

    async fn get_artist_radio(&mut self, artist_id: &str, limit: usize) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/artists/{}/radio", artist_id);

                let response = self
                    .http_client
                    .get(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US"), ("limit", &limit.to_string())])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks =
                            if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                                items
                                    .iter()
                                    .filter_map(Self::parse_track_from_json)
                                    .collect()
                            } else {
                                vec![]
                            };

                        return Ok(tracks);
                    }
                    Ok(resp) if resp.status().as_u16() == 404 => {
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

    async fn get_playlist_radio(&mut self, playlist_id: &str, limit: usize) -> Result<Vec<Track>> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/playlists/{}/radio",
                    playlist_id
                );

                let response = self
                    .http_client
                    .get(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US"), ("limit", &limit.to_string())])
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let tracks =
                            if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                                items
                                    .iter()
                                    .filter_map(Self::parse_track_from_json)
                                    .collect()
                            } else {
                                vec![]
                            };

                        return Ok(tracks);
                    }
                    Ok(resp) if resp.status().as_u16() == 404 => {
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

    async fn create_playlist(
        &mut self,
        name: &str,
        description: Option<&str>,
    ) -> Result<Playlist> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url =
                    "https://api.tidal.com/v2/my-collection/playlists/folders/create-playlist";

                let mut query_params = vec![
                    ("name", name.to_string()),
                    ("folderId", "root".to_string()),
                ];
                if let Some(desc) = description {
                    query_params.push(("description", desc.to_string()));
                }

                let response = self
                    .http_client
                    .put(url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&query_params)
                    .send()
                    .await;

                match response {
                    Ok(resp) if resp.status().is_success() => {
                        let json: Value = resp.json().await?;

                        let data =
                            json.get("data").ok_or_else(|| anyhow!("Missing data field"))?;

                        let uuid = data
                            .get("uuid")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("Missing uuid"))?
                            .to_string();

                        let title = data
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or(name)
                            .to_string();

                        let description = data
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        let num_tracks = data
                            .get("numberOfTracks")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;

                        return Ok(Playlist {
                            id: uuid,
                            title,
                            description,
                            num_tracks,
                            service: ServiceType::Tidal,
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
                        return Err(anyhow!(
                            "Failed to create playlist: {} - {}",
                            status,
                            body
                        ));
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

    async fn update_playlist(
        &mut self,
        playlist_id: &str,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
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

                let response = self
                    .http_client
                    .post(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .form(&form_params)
                    .send()
                    .await;

                match response {
                    Ok(resp)
                        if resp.status().is_success() || resp.status().as_u16() == 200 =>
                    {
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
                        return Err(anyhow!(
                            "Failed to update playlist: {} - {}",
                            status,
                            body
                        ));
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

    async fn delete_playlist(&mut self, playlist_id: &str) -> Result<()> {
        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!("https://api.tidal.com/v1/playlists/{}", playlist_id);

                let response = self
                    .http_client
                    .delete(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .send()
                    .await;

                match response {
                    Ok(resp)
                        if resp.status().is_success()
                            || resp.status().as_u16() == 200
                            || resp.status().as_u16() == 204 =>
                    {
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
                        return Err(anyhow!(
                            "Failed to delete playlist: {} - {}",
                            status,
                            body
                        ));
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

    async fn add_tracks_to_playlist(
        &mut self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<()> {
        if track_ids.is_empty() {
            return Ok(());
        }

        for attempt in 0..2 {
            if let Some(ref config) = self.config {
                let url = format!(
                    "https://api.tidal.com/v1/playlists/{}/items",
                    playlist_id
                );

                let track_ids_str = track_ids.join(",");

                let response = self
                    .http_client
                    .post(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US")])
                    .form(&[
                        ("trackIds", track_ids_str.as_str()),
                        ("onArtifactNotFound", "SKIP"),
                        ("onDupes", "ADD"),
                    ])
                    .send()
                    .await;

                match response {
                    Ok(resp)
                        if resp.status().is_success()
                            || resp.status().as_u16() == 200
                            || resp.status().as_u16() == 201 =>
                    {
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
                        return Err(anyhow!(
                            "Failed to add tracks to playlist: {} - {}",
                            status,
                            body
                        ));
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

    async fn remove_tracks_from_playlist(
        &mut self,
        playlist_id: &str,
        indices: &[usize],
    ) -> Result<()> {
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

                let response = self
                    .http_client
                    .delete(&url)
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", config.access_token),
                    )
                    .query(&[("countryCode", "US")])
                    .send()
                    .await;

                match response {
                    Ok(resp)
                        if resp.status().is_success()
                            || resp.status().as_u16() == 200
                            || resp.status().as_u16() == 204 =>
                    {
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

    fn get_cover_url(&self, cover: &CoverArt, size: u32) -> Option<String> {
        match cover {
            CoverArt::Url(url) => Some(url.clone()),
            CoverArt::ServiceId { id, service } => match service {
                ServiceType::Tidal => {
                    let path = id.replace('-', "/");
                    Some(format!(
                        "https://resources.tidal.com/images/{}/{}x{}.jpg",
                        path, size, size
                    ))
                }
                ServiceType::YouTube | ServiceType::Bandcamp => None,
            },
            CoverArt::None => None,
        }
    }
}
