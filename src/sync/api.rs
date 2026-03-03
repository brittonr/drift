//! Tidal API client for bulk sync operations.
//!
//! Purpose-built for library sync: full pagination, rate limiting with
//! 429/backoff, quality cascade for stream URLs, and automatic token refresh.

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;

/// Tidal OAuth2 credentials (compatible with drift and tidal-dl JSON files).
#[derive(Debug, Serialize, Deserialize)]
pub struct TidalCreds {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub user_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

const API_BASE: &str = "https://api.tidal.com/v1";
const AUTH_URL: &str = "https://auth.tidal.com/v1/oauth2/token";
const CLIENT_ID: &str = "dN2N95wCyEBTllu4";
const COUNTRY: &str = "US";
const PAGE_SIZE: u32 = 100;

// Rate limiting
const REQUEST_DELAY: Duration = Duration::from_millis(300);
const DOWNLOAD_DELAY: Duration = Duration::from_millis(500);

// Retry
const RETRY_ATTEMPTS: usize = 3;
const RETRY_BACKOFF_SECS: [u64; 3] = [5, 15, 30];
const MAX_RETRY_AFTER_SECS: u64 = 60;

// Quality cascade — try best first, fall back
pub const QUALITY_CASCADE: &[&str] = &["HI_RES_LOSSLESS", "HI_RES", "LOSSLESS", "HIGH"];

// ── Error types ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ApiError {
    /// 403 Forbidden — track is region-locked or delisted.
    Forbidden(String),
    /// 5xx — Tidal server error, not quality-dependent.
    ServerError(String),
    /// Any other error.
    Other(anyhow::Error),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Forbidden(msg) => write!(f, "Forbidden: {}", msg),
            ApiError::ServerError(msg) => write!(f, "Server error: {}", msg),
            ApiError::Other(e) => write!(f, "{}", e),
        }
    }
}

// ── Sync-specific types ──────────────────────────────────────────────────────

/// Album metadata for sync (includes track count for completion caching).
#[derive(Debug, Clone)]
pub struct SyncAlbum {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub num_tracks: u32,
}

/// Track metadata for sync (includes track_number, volume_number, album_artist
/// that drift's generic Track type doesn't carry).
#[derive(Debug, Clone)]
pub struct SyncTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub duration_seconds: u32,
    pub track_number: u32,
    pub volume_number: u32,
}

/// Playlist metadata for sync.
#[derive(Debug, Clone)]
pub struct SyncPlaylist {
    pub id: String,
    pub title: String,
    pub num_tracks: u32,
}

// ── API Client ───────────────────────────────────────────────────────────────

pub struct SyncApiClient {
    http: Client,
    config: TidalCreds,
    creds_path: PathBuf,
}

impl SyncApiClient {
    pub fn new(config: TidalCreds, creds_path: PathBuf) -> Self {
        Self {
            http: Client::new(),
            config,
            creds_path,
        }
    }

    /// Load credentials from disk. Tries drift path first, then tidal-tui.
    pub fn load() -> Result<Self> {
        let (config, path) = Self::find_and_load_creds()?;
        Ok(Self::new(config, path))
    }

    fn find_and_load_creds() -> Result<(TidalCreds, PathBuf)> {
        let config_dir =
            dirs::config_dir().context("Could not determine config directory")?;

        // Try drift path first
        let drift_path = config_dir.join("drift").join("credentials.json");
        if drift_path.exists() {
            let config = Self::load_creds_from(&drift_path)?;
            return Ok((config, drift_path));
        }

        // Fall back to tidal-tui path (legacy tidal-dl)
        let tidal_tui_path = config_dir.join("tidal-tui").join("credentials.json");
        if tidal_tui_path.exists() {
            let config = Self::load_creds_from(&tidal_tui_path)?;
            return Ok((config, tidal_tui_path));
        }

        Err(anyhow!(
            "No Tidal credentials found.\n\
             Expected at: {}\n\
             Or legacy:   {}\n\
             Run drift first to authenticate with Tidal.",
            drift_path.display(),
            tidal_tui_path.display(),
        ))
    }

    fn load_creds_from(path: &PathBuf) -> Result<TidalCreds> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read credentials from {}", path.display()))?;
        serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse credentials from {}", path.display()))
    }

    fn save_creds(&self) -> Result<()> {
        let contents = serde_json::to_string_pretty(&self.config)?;
        std::fs::write(&self.creds_path, contents)?;
        Ok(())
    }

    pub fn user_id(&self) -> i64 {
        self.config.user_id
    }

    /// Refresh the access token and save to disk.
    pub async fn refresh_token(&mut self) -> Result<()> {
        println!("  ↻ Refreshing access token...");
        let resp = self
            .http
            .post(AUTH_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &self.config.refresh_token),
                ("client_id", CLIENT_ID),
            ])
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .context("Token refresh request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Token refresh failed: {} {}", status, body));
        }

        let data: Value = resp.json().await?;
        if let Some(token) = data["access_token"].as_str() {
            self.config.access_token = token.to_string();
        }
        if let Some(refresh) = data["refresh_token"].as_str() {
            self.config.refresh_token = refresh.to_string();
        }
        if let Some(expires_in) = data["expires_in"].as_i64() {
            self.config.expires_at =
                Some(chrono::Utc::now() + chrono::Duration::seconds(expires_in));
        }

        self.save_creds()?;
        println!("  ✓ Token refreshed");
        Ok(())
    }

    // ── Core HTTP ────────────────────────────────────────────────────────

    /// Authenticated GET with retry on 401 (refresh), 429 (rate limit),
    /// 5xx (server error), and network timeouts.
    async fn api_get(
        &mut self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<Value, ApiError> {
        let url = if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}/{}", API_BASE, path)
        };

        let mut all_params: Vec<(&str, &str)> = vec![("countryCode", COUNTRY)];
        all_params.extend_from_slice(params);

        let mut refreshed = false;

        for attempt in 0..=RETRY_ATTEMPTS {
            sleep(REQUEST_DELAY).await;

            let result = self
                .http
                .get(&url)
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {}", self.config.access_token),
                )
                .query(&all_params)
                .timeout(Duration::from_secs(30))
                .send()
                .await;

            let resp = match result {
                Ok(r) => r,
                Err(e) => {
                    if attempt < RETRY_ATTEMPTS {
                        let wait = RETRY_BACKOFF_SECS[attempt];
                        eprintln!(
                            "  ⏳ Request error on {} — retrying in {}s",
                            path, wait
                        );
                        sleep(Duration::from_secs(wait)).await;
                        continue;
                    }
                    return Err(ApiError::Other(anyhow!(
                        "Request failed after retries: {}",
                        e
                    )));
                }
            };

            let status = resp.status();

            // 401 — refresh token and retry once
            if status == StatusCode::UNAUTHORIZED && !refreshed {
                refreshed = true;
                if let Err(e) = self.refresh_token().await {
                    return Err(ApiError::Other(e));
                }
                continue;
            }

            // 429 — rate limited, honor Retry-After header
            if status == StatusCode::TOO_MANY_REQUESTS {
                let raw_wait = resp
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(
                        RETRY_BACKOFF_SECS[attempt.min(RETRY_BACKOFF_SECS.len() - 1)],
                    );
                let wait = raw_wait.min(MAX_RETRY_AFTER_SECS);
                println!(
                    "  ⏳ Rate limited — waiting {}s ({}/{})",
                    wait,
                    attempt + 1,
                    RETRY_ATTEMPTS
                );
                sleep(Duration::from_secs(wait)).await;
                continue;
            }

            // 403 — region-locked or delisted
            if status == StatusCode::FORBIDDEN {
                return Err(ApiError::Forbidden(format!("403 on {}", path)));
            }

            // 5xx — server error with retry
            if status.is_server_error() {
                if attempt < RETRY_ATTEMPTS {
                    let wait = RETRY_BACKOFF_SECS[attempt];
                    eprintln!(
                        "  ✗ Server error {} on {} — retrying in {}s",
                        status, path, wait
                    );
                    sleep(Duration::from_secs(wait)).await;
                    continue;
                }
                return Err(ApiError::ServerError(format!(
                    "{} on {}",
                    status, path
                )));
            }

            if !status.is_success() {
                return Err(ApiError::Other(anyhow!(
                    "API error: {} on {}",
                    status,
                    path
                )));
            }

            match resp.json::<Value>().await {
                Ok(json) => return Ok(json),
                Err(e) => {
                    if attempt < RETRY_ATTEMPTS {
                        eprintln!("  ⏳ Invalid JSON from {} — retrying", path);
                        sleep(Duration::from_secs(RETRY_BACKOFF_SECS[attempt])).await;
                        continue;
                    }
                    return Err(ApiError::Other(anyhow!(
                        "Invalid JSON from {}: {}",
                        path,
                        e
                    )));
                }
            }
        }

        Err(ApiError::Other(anyhow!(
            "Failed {} after {} retries",
            path,
            RETRY_ATTEMPTS
        )))
    }

    /// Paginate through all results of an endpoint.
    async fn paginate(&mut self, path: &str, key: &str) -> Vec<Value> {
        let mut all_items = Vec::new();
        let mut offset = 0u32;

        loop {
            let offset_str = offset.to_string();
            let limit_str = PAGE_SIZE.to_string();
            let params = [
                ("limit", limit_str.as_str()),
                ("offset", offset_str.as_str()),
            ];

            let data = match self.api_get(path, &params).await {
                Ok(d) => d,
                Err(ApiError::ServerError(_)) => {
                    eprintln!(
                        "  ✗ Server error during pagination of {}, returning {} items so far",
                        path,
                        all_items.len()
                    );
                    break;
                }
                Err(_) => break,
            };

            let items = data
                .get(key)
                .and_then(|v| v.as_array())
                .map(|a| a.to_vec())
                .unwrap_or_default();

            if items.is_empty() {
                break;
            }

            all_items.extend(items);

            let total = data
                .get("totalNumberOfItems")
                .and_then(|v| v.as_u64())
                .unwrap_or(all_items.len() as u64);

            if all_items.len() as u64 >= total {
                break;
            }

            offset += PAGE_SIZE;
        }

        all_items
    }

    // ── Library fetchers ─────────────────────────────────────────────────

    pub async fn get_favorite_albums(&mut self) -> Vec<SyncAlbum> {
        let path = format!("users/{}/favorites/albums", self.config.user_id);
        let items = self.paginate(&path, "items").await;
        items
            .iter()
            .filter_map(|item| {
                let album = item.get("item").unwrap_or(item);
                let id = album.get("id")?.as_u64()?.to_string();
                let title = album.get("title")?.as_str()?.to_string();
                let artist = album
                    .get("artist")
                    .and_then(|a| a.get("name"))
                    .and_then(|n| n.as_str())
                    .or_else(|| {
                        album
                            .get("artists")
                            .and_then(|a| a.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|a| a.get("name"))
                            .and_then(|n| n.as_str())
                    })
                    .unwrap_or("Unknown Artist")
                    .to_string();
                let num_tracks = album
                    .get("numberOfTracks")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0) as u32;
                Some(SyncAlbum {
                    id,
                    title,
                    artist,
                    num_tracks,
                })
            })
            .collect()
    }

    pub async fn get_favorite_tracks(&mut self) -> Vec<SyncTrack> {
        let path = format!("users/{}/favorites/tracks", self.config.user_id);
        let items = self.paginate(&path, "items").await;
        items
            .iter()
            .filter_map(|item| parse_sync_track(item.get("item").unwrap_or(item)))
            .collect()
    }

    pub async fn get_playlists(&mut self) -> Vec<SyncPlaylist> {
        let path = format!("users/{}/playlists", self.config.user_id);
        let items = self.paginate(&path, "items").await;
        items
            .iter()
            .filter_map(|p| {
                let id = p
                    .get("uuid")
                    .or_else(|| p.get("id"))?
                    .as_str()?
                    .to_string();
                let title = p.get("title")?.as_str()?.to_string();
                let num_tracks = p
                    .get("numberOfTracks")
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0) as u32;
                Some(SyncPlaylist {
                    id,
                    title,
                    num_tracks,
                })
            })
            .collect()
    }

    pub async fn get_album_tracks(&mut self, album_id: &str) -> Vec<SyncTrack> {
        let path = format!("albums/{}/items", album_id);
        let items = self.paginate(&path, "items").await;
        items
            .iter()
            .filter_map(|item| parse_sync_track(item.get("item").unwrap_or(item)))
            .collect()
    }

    pub async fn get_playlist_tracks(&mut self, playlist_id: &str) -> Vec<SyncTrack> {
        let path = format!("playlists/{}/items", playlist_id);
        let items = self.paginate(&path, "items").await;
        items
            .iter()
            .filter_map(|item| parse_sync_track(item.get("item").unwrap_or(item)))
            .collect()
    }

    // ── Stream URL with quality cascade ──────────────────────────────────

    /// Get the stream URL for a track at the highest available quality.
    /// Tries HI_RES_LOSSLESS → HI_RES → LOSSLESS → HIGH.
    /// Returns (url, codec) on success.
    pub async fn get_stream_url(
        &mut self,
        track_id: &str,
    ) -> Result<(String, String), ApiError> {
        for attempt in 0..=RETRY_ATTEMPTS {
            for quality in QUALITY_CASCADE {
                let path = format!("tracks/{}/playbackinfo", track_id);
                let params = [
                    ("audioquality", *quality),
                    ("assetpresentation", "FULL"),
                    ("playbackmode", "STREAM"),
                ];

                match self.api_get(&path, &params).await {
                    Ok(data) => {
                        let codec = data
                            .get("audioQuality")
                            .and_then(|v| v.as_str())
                            .unwrap_or(quality)
                            .to_string();

                        // Try manifest (base64-encoded JSON or DASH XML)
                        if let Some(manifest_b64) =
                            data.get("manifest").and_then(|v| v.as_str())
                        {
                            if let Ok(decoded) =
                                general_purpose::STANDARD.decode(manifest_b64)
                            {
                                if let Ok(manifest_str) = String::from_utf8(decoded) {
                                    let manifest_mime = data
                                        .get("manifestMimeType")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");

                                    if manifest_mime.contains("dash+xml") {
                                        if let Some(url) =
                                            extract_dash_url(&manifest_str)
                                        {
                                            return Ok((url, codec));
                                        }
                                    } else if let Ok(mj) =
                                        serde_json::from_str::<Value>(&manifest_str)
                                    {
                                        if let Some(url) = mj
                                            .get("urls")
                                            .and_then(|u| u.as_array())
                                            .and_then(|a| a.first())
                                            .and_then(|u| u.as_str())
                                        {
                                            return Ok((url.to_string(), codec));
                                        }
                                    }
                                }
                            }
                        }

                        // Try direct URL field
                        if let Some(url) = data.get("url").and_then(|v| v.as_str()) {
                            return Ok((url.to_string(), codec));
                        }

                        // This quality didn't yield a URL, try next
                        continue;
                    }
                    Err(ApiError::Forbidden(_)) => {
                        // 403 isn't quality-dependent — rate limit or unavailable.
                        // Don't try lower qualities, retry from top quality with backoff.
                        if attempt < RETRY_ATTEMPTS {
                            let wait = RETRY_BACKOFF_SECS[attempt];
                            println!(
                                "    ⏳ 403 — backing off {}s (attempt {}/{})",
                                wait,
                                attempt + 1,
                                RETRY_ATTEMPTS
                            );
                            sleep(Duration::from_secs(wait)).await;
                            break; // break quality loop, retry outer loop
                        }
                        return Err(ApiError::Forbidden(format!(
                            "Track {} unavailable (403 after {} retries)",
                            track_id, RETRY_ATTEMPTS
                        )));
                    }
                    Err(ApiError::ServerError(msg)) => {
                        // 5xx is not quality-dependent
                        return Err(ApiError::ServerError(format!(
                            "Track {} unavailable (server error): {}",
                            track_id, msg
                        )));
                    }
                    Err(ApiError::Other(_)) => {
                        continue; // try next quality level
                    }
                }
            }
        }

        Err(ApiError::Other(anyhow!(
            "No stream URL found for track {}",
            track_id
        )))
    }

    /// Delay between track downloads (rate limiting).
    pub fn download_delay(&self) -> Duration {
        DOWNLOAD_DELAY
    }

    /// Borrow the HTTP client for raw downloads.
    pub fn http(&self) -> &Client {
        &self.http
    }
}

// ── JSON parsing helpers ─────────────────────────────────────────────────────

fn parse_sync_track(data: &Value) -> Option<SyncTrack> {
    let id = data.get("id")?.as_u64()?.to_string();
    let title = data.get("title")?.as_str()?.to_string();

    let artist = data
        .get("artist")
        .and_then(|a| a.get("name"))
        .and_then(|n| n.as_str())
        .or_else(|| {
            data.get("artists")
                .and_then(|a| a.as_array())
                .and_then(|arr| arr.first())
                .and_then(|a| a.get("name"))
                .and_then(|n| n.as_str())
        })
        .unwrap_or("Unknown Artist")
        .to_string();

    let album_data = data.get("album");
    let album = album_data
        .and_then(|a| a.get("title"))
        .and_then(|t| t.as_str())
        .unwrap_or("Unknown Album")
        .to_string();
    let album_artist = album_data
        .and_then(|a| a.get("artist"))
        .and_then(|a| a.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or(&artist)
        .to_string();

    let duration_seconds = data.get("duration")?.as_u64()? as u32;
    let track_number = data
        .get("trackNumber")
        .and_then(|n| n.as_u64())
        .unwrap_or(0) as u32;
    let volume_number = data
        .get("volumeNumber")
        .and_then(|n| n.as_u64())
        .unwrap_or(1) as u32;

    Some(SyncTrack {
        id,
        title,
        artist,
        album,
        album_artist,
        duration_seconds,
        track_number,
        volume_number,
    })
}

/// Extract <BaseURL> from a DASH manifest XML string.
fn extract_dash_url(manifest_xml: &str) -> Option<String> {
    let start = manifest_xml.find("<BaseURL>")?;
    let content_start = start + "<BaseURL>".len();
    let end = manifest_xml[content_start..].find("</BaseURL>")?;
    Some(manifest_xml[content_start..content_start + end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_simple() {
        assert_eq!(1 + 1, 2);
    }

    #[test]
    fn test_extract_dash_url_simple() {
        let manifest = r#"<?xml version="1.0"?>
<MPD>
  <Period>
    <AdaptationSet>
      <Representation>
        <BaseURL>https://example.com/track.flac</BaseURL>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
        
        let url = extract_dash_url(manifest);
        assert_eq!(url, Some("https://example.com/track.flac".to_string()));
    }

    #[test]
    fn test_extract_dash_url_no_base_url() {
        let manifest = r#"<?xml version="1.0"?><MPD></MPD>"#;
        let url = extract_dash_url(manifest);
        assert_eq!(url, None);
    }

    #[test]
    fn test_extract_dash_url_incomplete() {
        let manifest = r#"<BaseURL>https://example.com/track.flac"#;
        let url = extract_dash_url(manifest);
        assert_eq!(url, None);
    }

    #[test]
    fn test_extract_dash_url_with_special_chars() {
        let manifest = r#"<BaseURL>https://example.com/track?token=abc123&amp;format=flac</BaseURL>"#;
        let url = extract_dash_url(manifest);
        assert_eq!(
            url,
            Some("https://example.com/track?token=abc123&amp;format=flac".to_string())
        );
    }

    #[test]
    fn test_quality_cascade_order() {
        // Ensure quality cascade tries best quality first
        assert_eq!(QUALITY_CASCADE.len(), 4);
        assert_eq!(QUALITY_CASCADE[0], "HI_RES_LOSSLESS");
        assert_eq!(QUALITY_CASCADE[1], "HI_RES");
        assert_eq!(QUALITY_CASCADE[2], "LOSSLESS");
        assert_eq!(QUALITY_CASCADE[3], "HIGH");
    }

    #[test]
    fn test_parse_sync_track_complete() {
        let json = serde_json::json!({
            "id": 123456,
            "title": "Test Track",
            "artist": {"name": "Test Artist"},
            "album": {
                "title": "Test Album",
                "artist": {"name": "Album Artist"}
            },
            "duration": 240,
            "trackNumber": 3,
            "volumeNumber": 2
        });

        let track = parse_sync_track(&json).unwrap();
        assert_eq!(track.id, "123456");
        assert_eq!(track.title, "Test Track");
        assert_eq!(track.artist, "Test Artist");
        assert_eq!(track.album, "Test Album");
        assert_eq!(track.album_artist, "Album Artist");
        assert_eq!(track.duration_seconds, 240);
        assert_eq!(track.track_number, 3);
        assert_eq!(track.volume_number, 2);
    }

    #[test]
    fn test_parse_sync_track_minimal() {
        let json = serde_json::json!({
            "id": 789,
            "title": "Minimal Track",
            "duration": 180
        });

        let track = parse_sync_track(&json).unwrap();
        assert_eq!(track.id, "789");
        assert_eq!(track.title, "Minimal Track");
        assert_eq!(track.artist, "Unknown Artist");
        assert_eq!(track.album, "Unknown Album");
        assert_eq!(track.duration_seconds, 180);
        assert_eq!(track.track_number, 0);
        assert_eq!(track.volume_number, 1);
    }

    #[test]
    fn test_parse_sync_track_artists_array() {
        let json = serde_json::json!({
            "id": 999,
            "title": "Multi Artist Track",
            "artists": [
                {"name": "First Artist"},
                {"name": "Second Artist"}
            ],
            "album": {"title": "Compilation"},
            "duration": 200
        });

        let track = parse_sync_track(&json).unwrap();
        assert_eq!(track.artist, "First Artist");
    }

    #[test]
    fn test_parse_sync_track_missing_id() {
        let json = serde_json::json!({
            "title": "No ID Track",
            "duration": 100
        });

        let track = parse_sync_track(&json);
        assert!(track.is_none());
    }

    #[test]
    fn test_parse_sync_track_missing_duration() {
        let json = serde_json::json!({
            "id": 111,
            "title": "No Duration Track"
        });

        let track = parse_sync_track(&json);
        assert!(track.is_none());
    }

    #[test]
    fn test_tidal_creds_serialization() {
        let creds = TidalCreds {
            access_token: "access123".to_string(),
            refresh_token: "refresh456".to_string(),
            token_type: "Bearer".to_string(),
            user_id: 12345,
            expires_at: None,
        };

        let json = serde_json::to_string(&creds).unwrap();
        assert!(json.contains("access123"));
        assert!(json.contains("refresh456"));
        assert!(json.contains("12345"));

        let deserialized: TidalCreds = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.access_token, "access123");
        assert_eq!(deserialized.user_id, 12345);
    }

    #[test]
    fn test_tidal_creds_with_expiry() {
        let now = chrono::Utc::now();
        let creds = TidalCreds {
            access_token: "token".to_string(),
            refresh_token: "refresh".to_string(),
            token_type: "Bearer".to_string(),
            user_id: 999,
            expires_at: Some(now),
        };

        let json = serde_json::to_string(&creds).unwrap();
        let deserialized: TidalCreds = serde_json::from_str(&json).unwrap();
        assert!(deserialized.expires_at.is_some());
    }

    #[test]
    fn test_load_creds_from_valid_file() {
        let temp_dir = TempDir::new().unwrap();
        let creds_path = temp_dir.path().join("credentials.json");
        
        let creds = TidalCreds {
            access_token: "test_access".to_string(),
            refresh_token: "test_refresh".to_string(),
            token_type: "Bearer".to_string(),
            user_id: 54321,
            expires_at: None,
        };
        
        let json = serde_json::to_string_pretty(&creds).unwrap();
        fs::write(&creds_path, json).unwrap();
        
        let loaded = SyncApiClient::load_creds_from(&creds_path).unwrap();
        assert_eq!(loaded.access_token, "test_access");
        assert_eq!(loaded.refresh_token, "test_refresh");
        assert_eq!(loaded.user_id, 54321);
    }

    #[test]
    fn test_load_creds_from_nonexistent_file() {
        let path = PathBuf::from("/nonexistent/path/credentials.json");
        let result = SyncApiClient::load_creds_from(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_creds_from_invalid_json() {
        let temp_dir = TempDir::new().unwrap();
        let creds_path = temp_dir.path().join("bad.json");
        fs::write(&creds_path, "{ invalid json }").unwrap();
        
        let result = SyncApiClient::load_creds_from(&creds_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_api_error_display() {
        let forbidden = ApiError::Forbidden("Track not available".to_string());
        assert!(forbidden.to_string().contains("Forbidden"));
        
        let server = ApiError::ServerError("500 Internal Server Error".to_string());
        assert!(server.to_string().contains("Server error"));
        
        let other = ApiError::Other(anyhow!("Generic error"));
        assert!(other.to_string().contains("Generic error"));
    }

    #[test]
    fn test_sync_album_construction() {
        let album = SyncAlbum {
            id: "12345".to_string(),
            title: "Greatest Hits".to_string(),
            artist: "Test Artist".to_string(),
            num_tracks: 15,
        };
        
        assert_eq!(album.id, "12345");
        assert_eq!(album.num_tracks, 15);
    }

    #[test]
    fn test_sync_track_construction() {
        let track = SyncTrack {
            id: "98765".to_string(),
            title: "Test Song".to_string(),
            artist: "Artist Name".to_string(),
            album: "Album Name".to_string(),
            album_artist: "Album Artist".to_string(),
            duration_seconds: 210,
            track_number: 5,
            volume_number: 1,
        };
        
        assert_eq!(track.id, "98765");
        assert_eq!(track.track_number, 5);
        assert_eq!(track.volume_number, 1);
    }

    #[test]
    fn test_sync_playlist_construction() {
        let playlist = SyncPlaylist {
            id: "playlist-uuid".to_string(),
            title: "My Favorites".to_string(),
            num_tracks: 50,
        };
        
        assert_eq!(playlist.id, "playlist-uuid");
        assert_eq!(playlist.num_tracks, 50);
    }

    #[test]
    fn test_constants_are_valid() {
        assert!(!API_BASE.is_empty());
        assert!(API_BASE.starts_with("https://"));
        assert!(!AUTH_URL.is_empty());
        assert!(AUTH_URL.starts_with("https://"));
        assert!(!CLIENT_ID.is_empty());
        assert_eq!(COUNTRY, "US");
        assert!(PAGE_SIZE > 0);
        assert!(REQUEST_DELAY.as_millis() > 0);
        assert!(DOWNLOAD_DELAY.as_millis() > 0);
        assert!(RETRY_ATTEMPTS > 0);
        assert_eq!(RETRY_BACKOFF_SECS.len(), 3);
    }

    #[test]
    fn test_user_id_accessor() {
        let creds = TidalCreds {
            access_token: "token".to_string(),
            refresh_token: "refresh".to_string(),
            token_type: "Bearer".to_string(),
            user_id: 99999,
            expires_at: None,
        };
        
        let client = SyncApiClient::new(creds, PathBuf::from("/tmp/test.json"));
        assert_eq!(client.user_id(), 99999);
    }

    #[test]
    fn test_download_delay() {
        let creds = TidalCreds {
            access_token: "token".to_string(),
            refresh_token: "refresh".to_string(),
            token_type: "Bearer".to_string(),
            user_id: 1,
            expires_at: None,
        };
        
        let client = SyncApiClient::new(creds, PathBuf::from("/tmp/test.json"));
        let delay = client.download_delay();
        assert_eq!(delay, DOWNLOAD_DELAY);
    }
}
