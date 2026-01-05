use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Storage structure for Bandcamp local data (favorites, saved playlists)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BandcampStorage {
    /// Storage format version for migrations
    #[serde(default)]
    pub version: u32,
    /// User's favorite tracks (stored locally)
    #[serde(default)]
    pub favorite_tracks: Vec<StoredTrack>,
    /// User's favorite albums (stored locally)
    #[serde(default)]
    pub favorite_albums: Vec<StoredAlbum>,
    /// User's favorite artists (stored locally)
    #[serde(default)]
    pub favorite_artists: Vec<StoredArtist>,
    /// Local playlists
    #[serde(default)]
    pub saved_playlists: Vec<SavedPlaylist>,
    /// Track URLs within local playlists (playlist_id -> track_urls)
    #[serde(default)]
    pub local_playlist_tracks: Vec<LocalPlaylistTracks>,
}

/// A track stored in local favorites
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTrack {
    /// Full Bandcamp track URL (serves as unique ID)
    pub url: String,
    /// Numeric track ID from yt-dlp
    pub track_id: String,
    pub title: String,
    pub artist: String,
    /// Artist's Bandcamp subdomain (e.g., "phoebebridgers")
    pub artist_subdomain: String,
    pub album: String,
    /// Album URL if available
    pub album_url: Option<String>,
    pub duration_seconds: u32,
    pub thumbnail_url: Option<String>,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub added_at: DateTime<Utc>,
}

/// An album stored in local favorites
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAlbum {
    /// Full album URL (serves as unique ID)
    pub url: String,
    /// Album ID from yt-dlp
    pub album_id: String,
    pub title: String,
    pub artist: String,
    /// Artist's Bandcamp subdomain
    pub artist_subdomain: String,
    pub num_tracks: u32,
    pub thumbnail_url: Option<String>,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub added_at: DateTime<Utc>,
}

/// An artist stored in local favorites
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredArtist {
    /// Artist's Bandcamp subdomain (e.g., "phoebebridgers")
    pub subdomain: String,
    pub name: String,
    /// Full artist URL (e.g., "https://phoebebridgers.bandcamp.com")
    pub url: String,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub added_at: DateTime<Utc>,
}

/// A local playlist
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPlaylist {
    /// Playlist ID (local-xxxxx for local playlists)
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub num_tracks: usize,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub added_at: DateTime<Utc>,
}

/// Track URLs for a local playlist
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalPlaylistTracks {
    pub playlist_id: String,
    pub track_urls: Vec<String>,
}

impl BandcampStorage {
    const CURRENT_VERSION: u32 = 1;

    /// Get the storage file path
    fn storage_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
        Ok(config_dir.join("drift").join("bandcamp_data.toml"))
    }

    /// Load storage from disk, returning default if not found
    pub fn load() -> Result<Self> {
        let path = Self::storage_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)?;
        let mut storage: Self = toml::from_str(&contents)?;

        // Handle migrations if needed
        if storage.version < Self::CURRENT_VERSION {
            storage.migrate();
        }

        Ok(storage)
    }

    /// Save storage to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::storage_path()?;

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = toml::to_string_pretty(self)?;
        fs::write(&path, contents)?;
        Ok(())
    }

    /// Migrate storage to current version
    fn migrate(&mut self) {
        self.version = Self::CURRENT_VERSION;
    }

    // === Favorite Tracks ===

    /// Add a track to favorites
    pub fn add_favorite_track(&mut self, track: StoredTrack) {
        // Don't add duplicates (compare by URL)
        if !self.favorite_tracks.iter().any(|t| t.url == track.url) {
            self.favorite_tracks.push(track);
        }
    }

    /// Remove a track from favorites by URL
    pub fn remove_favorite_track(&mut self, track_url: &str) -> bool {
        let len_before = self.favorite_tracks.len();
        self.favorite_tracks.retain(|t| t.url != track_url);
        self.favorite_tracks.len() < len_before
    }

    /// Check if a track is in favorites
    #[allow(dead_code)]
    pub fn is_favorite_track(&self, track_url: &str) -> bool {
        self.favorite_tracks.iter().any(|t| t.url == track_url)
    }

    /// Find a cached track by URL
    pub fn find_track(&self, track_url: &str) -> Option<&StoredTrack> {
        self.favorite_tracks.iter().find(|t| t.url == track_url)
    }

    // === Favorite Albums ===

    /// Add an album to favorites
    #[allow(dead_code)]
    pub fn add_favorite_album(&mut self, album: StoredAlbum) {
        if !self.favorite_albums.iter().any(|a| a.url == album.url) {
            self.favorite_albums.push(album);
        }
    }

    /// Remove an album from favorites by URL
    #[allow(dead_code)]
    pub fn remove_favorite_album(&mut self, album_url: &str) -> bool {
        let len_before = self.favorite_albums.len();
        self.favorite_albums.retain(|a| a.url != album_url);
        self.favorite_albums.len() < len_before
    }

    // === Favorite Artists ===

    /// Add an artist to favorites
    #[allow(dead_code)]
    pub fn add_favorite_artist(&mut self, artist: StoredArtist) {
        if !self.favorite_artists.iter().any(|a| a.subdomain == artist.subdomain) {
            self.favorite_artists.push(artist);
        }
    }

    /// Remove an artist from favorites by subdomain
    #[allow(dead_code)]
    pub fn remove_favorite_artist(&mut self, subdomain: &str) -> bool {
        let len_before = self.favorite_artists.len();
        self.favorite_artists.retain(|a| a.subdomain != subdomain);
        self.favorite_artists.len() < len_before
    }

    // === Local Playlists ===

    /// Create a new local playlist
    pub fn create_local_playlist(&mut self, name: &str, description: Option<&str>) -> SavedPlaylist {
        let id = format!("local-{}", uuid_simple());
        let playlist = SavedPlaylist {
            id: id.clone(),
            title: name.to_string(),
            description: description.map(|s| s.to_string()),
            num_tracks: 0,
            added_at: Utc::now(),
        };
        self.saved_playlists.push(playlist.clone());
        self.local_playlist_tracks.push(LocalPlaylistTracks {
            playlist_id: id,
            track_urls: Vec::new(),
        });
        playlist
    }

    /// Update a local playlist's metadata
    pub fn update_playlist(
        &mut self,
        playlist_id: &str,
        title: Option<&str>,
        description: Option<&str>,
    ) -> bool {
        if let Some(playlist) = self
            .saved_playlists
            .iter_mut()
            .find(|p| p.id == playlist_id)
        {
            if let Some(t) = title {
                playlist.title = t.to_string();
            }
            if let Some(d) = description {
                playlist.description = Some(d.to_string());
            }
            true
        } else {
            false
        }
    }

    /// Remove a playlist by ID
    pub fn remove_playlist(&mut self, playlist_id: &str) -> bool {
        let len_before = self.saved_playlists.len();
        self.saved_playlists.retain(|p| p.id != playlist_id);
        self.local_playlist_tracks.retain(|lpt| lpt.playlist_id != playlist_id);
        self.saved_playlists.len() < len_before
    }

    /// Get track URLs for a local playlist
    pub fn get_local_playlist_tracks(&self, playlist_id: &str) -> Vec<String> {
        self.local_playlist_tracks
            .iter()
            .find(|lpt| lpt.playlist_id == playlist_id)
            .map(|lpt| lpt.track_urls.clone())
            .unwrap_or_default()
    }

    /// Add tracks to a local playlist
    pub fn add_tracks_to_local_playlist(&mut self, playlist_id: &str, track_urls: &[String]) -> bool {
        if let Some(lpt) = self
            .local_playlist_tracks
            .iter_mut()
            .find(|lpt| lpt.playlist_id == playlist_id)
        {
            for url in track_urls {
                if !lpt.track_urls.contains(url) {
                    lpt.track_urls.push(url.clone());
                }
            }
            // Update track count in saved_playlists
            if let Some(playlist) = self
                .saved_playlists
                .iter_mut()
                .find(|p| p.id == playlist_id)
            {
                playlist.num_tracks = lpt.track_urls.len();
            }
            true
        } else {
            false
        }
    }

    /// Remove tracks from a local playlist by indices
    pub fn remove_tracks_from_local_playlist(
        &mut self,
        playlist_id: &str,
        indices: &[usize],
    ) -> bool {
        if let Some(lpt) = self
            .local_playlist_tracks
            .iter_mut()
            .find(|lpt| lpt.playlist_id == playlist_id)
        {
            // Remove in reverse order to maintain indices
            let mut sorted_indices: Vec<_> = indices.to_vec();
            sorted_indices.sort_unstable_by(|a, b| b.cmp(a));
            for idx in sorted_indices {
                if idx < lpt.track_urls.len() {
                    lpt.track_urls.remove(idx);
                }
            }
            // Update track count
            if let Some(playlist) = self
                .saved_playlists
                .iter_mut()
                .find(|p| p.id == playlist_id)
            {
                playlist.num_tracks = lpt.track_urls.len();
            }
            true
        } else {
            false
        }
    }
}

/// Generate a simple UUID-like string for local playlist IDs
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:x}{:x}", duration.as_secs(), duration.subsec_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_favorite_tracks() {
        let mut storage = BandcampStorage::default();

        let track = StoredTrack {
            url: "https://artist.bandcamp.com/track/test".to_string(),
            track_id: "123".to_string(),
            title: "Test Track".to_string(),
            artist: "Test Artist".to_string(),
            artist_subdomain: "artist".to_string(),
            album: "Test Album".to_string(),
            album_url: None,
            duration_seconds: 180,
            thumbnail_url: None,
            added_at: Utc::now(),
        };

        storage.add_favorite_track(track.clone());
        assert!(storage.is_favorite_track("https://artist.bandcamp.com/track/test"));
        assert_eq!(storage.favorite_tracks.len(), 1);

        // No duplicates
        storage.add_favorite_track(track);
        assert_eq!(storage.favorite_tracks.len(), 1);

        // Remove
        assert!(storage.remove_favorite_track("https://artist.bandcamp.com/track/test"));
        assert!(!storage.is_favorite_track("https://artist.bandcamp.com/track/test"));
    }

    #[test]
    fn test_local_playlist() {
        let mut storage = BandcampStorage::default();

        let playlist = storage.create_local_playlist("My Playlist", Some("Description"));
        assert!(playlist.id.starts_with("local-"));

        // Add tracks
        storage.add_tracks_to_local_playlist(
            &playlist.id,
            &[
                "https://artist.bandcamp.com/track/song1".to_string(),
                "https://artist.bandcamp.com/track/song2".to_string(),
            ],
        );
        let tracks = storage.get_local_playlist_tracks(&playlist.id);
        assert_eq!(tracks.len(), 2);

        // Remove track at index 0
        storage.remove_tracks_from_local_playlist(&playlist.id, &[0]);
        let tracks = storage.get_local_playlist_tracks(&playlist.id);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0], "https://artist.bandcamp.com/track/song2");
    }

    #[test]
    fn test_favorite_albums() {
        let mut storage = BandcampStorage::default();

        let album = StoredAlbum {
            url: "https://artist.bandcamp.com/album/test".to_string(),
            album_id: "456".to_string(),
            title: "Test Album".to_string(),
            artist: "Test Artist".to_string(),
            artist_subdomain: "artist".to_string(),
            num_tracks: 10,
            thumbnail_url: None,
            added_at: Utc::now(),
        };

        storage.add_favorite_album(album.clone());
        assert_eq!(storage.favorite_albums.len(), 1);

        // No duplicates
        storage.add_favorite_album(album);
        assert_eq!(storage.favorite_albums.len(), 1);

        // Remove
        assert!(storage.remove_favorite_album("https://artist.bandcamp.com/album/test"));
        assert_eq!(storage.favorite_albums.len(), 0);
    }
}
