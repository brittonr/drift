pub mod bandcamp;
pub mod bandcamp_storage;
pub mod mixed_playlist;
pub mod multi;
pub mod tidal;
pub mod youtube;
pub mod youtube_storage;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Identifies which music service a resource comes from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceType {
    Tidal,
    YouTube,
    Bandcamp,
}

impl std::fmt::Display for ServiceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceType::Tidal => write!(f, "tidal"),
            ServiceType::YouTube => write!(f, "youtube"),
            ServiceType::Bandcamp => write!(f, "bandcamp"),
        }
    }
}

impl std::str::FromStr for ServiceType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tidal" => Ok(ServiceType::Tidal),
            "youtube" | "ytmusic" | "youtube_music" => Ok(ServiceType::YouTube),
            "bandcamp" | "bc" => Ok(ServiceType::Bandcamp),
            _ => Err(anyhow::anyhow!("Unknown service type: {}", s)),
        }
    }
}

/// Cover art representation - service-agnostic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CoverArt {
    /// Service-specific ID that needs URL construction (e.g., Tidal cover IDs)
    ServiceId { id: String, service: ServiceType },
    /// Direct URL (e.g., YouTube provides full URLs)
    Url(String),
    /// No cover available
    None,
}

impl CoverArt {
    /// Create a Tidal cover art reference
    pub fn tidal(id: String) -> Self {
        CoverArt::ServiceId {
            id,
            service: ServiceType::Tidal,
        }
    }

    /// Create from an optional Tidal cover ID (for backward compatibility)
    pub fn from_tidal_option(cover_id: Option<String>) -> Self {
        match cover_id {
            Some(id) => CoverArt::tidal(id),
            None => CoverArt::None,
        }
    }
}

/// A track from any music service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    /// Unique identifier (string to support both numeric Tidal IDs and YouTube video IDs)
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: u32,
    pub cover_art: CoverArt,
    pub service: ServiceType,
}

/// A playlist from any music service
#[derive(Debug, Clone)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub num_tracks: usize,
    pub service: ServiceType,
}

/// An album from any music service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub num_tracks: u32,
    pub cover_art: CoverArt,
    pub service: ServiceType,
}

/// An artist from any music service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub service: ServiceType,
}

/// Search results from any music service
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
}


/// The core music service abstraction
///
/// This trait defines all operations that a music service must support.
/// Services that don't support certain features should return empty results
/// or appropriate errors.
#[async_trait]
pub trait MusicService: Send + Sync {
    /// Get the service type identifier
    fn service_type(&self) -> ServiceType;

    /// Check if the service is authenticated
    fn is_authenticated(&self) -> bool;

    /// Set audio quality preference
    fn set_audio_quality(&mut self, quality: &str);

    // === Playback ===

    /// Get stream URL for a track
    async fn get_stream_url(&mut self, track_id: &str) -> Result<String>;

    // === Library ===

    /// Get user's playlists
    async fn get_playlists(&mut self) -> Result<Vec<Playlist>>;

    /// Get tracks in a playlist
    async fn get_playlist_tracks(&mut self, playlist_id: &str) -> Result<Vec<Track>>;

    /// Get user's favorite tracks
    async fn get_favorite_tracks(&mut self) -> Result<Vec<Track>>;

    /// Get user's favorite albums
    async fn get_favorite_albums(&mut self) -> Result<Vec<Album>>;

    /// Get user's favorite artists
    async fn get_favorite_artists(&mut self) -> Result<Vec<Artist>>;

    // === Favorites Management ===

    /// Add a track to favorites
    async fn add_favorite_track(&mut self, track_id: &str) -> Result<()>;

    /// Remove a track from favorites
    async fn remove_favorite_track(&mut self, track_id: &str) -> Result<()>;

    // === Search ===

    /// Search for tracks, albums, and artists
    async fn search(&mut self, query: &str, limit: usize) -> Result<SearchResults>;

    // === Album/Artist Details ===

    /// Get tracks from an album
    async fn get_album_tracks(&mut self, album_id: &str) -> Result<Vec<Track>>;

    /// Get top tracks from an artist
    async fn get_artist_top_tracks(&mut self, artist_id: &str) -> Result<Vec<Track>>;

    /// Get albums from an artist
    async fn get_artist_albums(&mut self, artist_id: &str) -> Result<Vec<Album>>;

    // === Radio/Recommendations ===

    /// Get radio tracks based on a seed track
    async fn get_track_radio(&mut self, track_id: &str, limit: usize) -> Result<Vec<Track>>;

    /// Get radio tracks based on a seed artist
    async fn get_artist_radio(&mut self, artist_id: &str, limit: usize) -> Result<Vec<Track>>;

    /// Get radio tracks based on a seed playlist
    async fn get_playlist_radio(&mut self, playlist_id: &str, limit: usize) -> Result<Vec<Track>>;

    // === Playlist Management ===

    /// Create a new playlist
    async fn create_playlist(&mut self, name: &str, description: Option<&str>) -> Result<Playlist>;

    /// Update playlist metadata
    async fn update_playlist(
        &mut self,
        playlist_id: &str,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<()>;

    /// Delete a playlist
    async fn delete_playlist(&mut self, playlist_id: &str) -> Result<()>;

    /// Add tracks to a playlist
    async fn add_tracks_to_playlist(
        &mut self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<()>;

    /// Remove tracks from a playlist by indices
    async fn remove_tracks_from_playlist(
        &mut self,
        playlist_id: &str,
        indices: &[usize],
    ) -> Result<()>;

    // === Cover Art ===

    /// Resolve cover art to a URL
    fn get_cover_url(&self, cover: &CoverArt, size: u32) -> Option<String>;

    /// Get cover URL from a legacy cover ID string (for backward compatibility)
    fn get_cover_url_from_id(&self, cover_id: &str, size: u32) -> Option<String> {
        self.get_cover_url(
            &CoverArt::ServiceId {
                id: cover_id.to_string(),
                service: self.service_type(),
            },
            size,
        )
    }
}

// Re-export the service implementations
pub use bandcamp::BandcampClient;
pub use mixed_playlist::MixedPlaylistStorage;
pub use multi::MultiServiceManager;
pub use tidal::TidalClient;
pub use youtube::YouTubeClient;
