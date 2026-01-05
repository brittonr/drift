use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Storage structure for YouTube local data (favorites, saved playlists)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct YouTubeStorage {
    /// Storage format version for migrations
    #[serde(default)]
    pub version: u32,
    /// User's favorite tracks (stored locally)
    #[serde(default)]
    pub favorite_tracks: Vec<StoredTrack>,
    /// User's favorite channels/artists (stored locally)
    #[serde(default)]
    pub favorite_channels: Vec<StoredChannel>,
    /// Saved YouTube playlist URLs and local playlists
    #[serde(default)]
    pub saved_playlists: Vec<SavedPlaylist>,
    /// Track IDs within local playlists (playlist_id -> track_ids)
    #[serde(default)]
    pub local_playlist_tracks: Vec<LocalPlaylistTracks>,
}

/// A track stored in local favorites
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTrack {
    /// YouTube video ID
    pub id: String,
    pub title: String,
    /// Channel ID (artist)
    pub channel_id: String,
    /// Channel name (artist name)
    pub channel_name: String,
    pub duration_seconds: u32,
    pub thumbnail_url: Option<String>,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub added_at: DateTime<Utc>,
}

/// A channel/artist stored in local favorites
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredChannel {
    /// Channel ID
    pub id: String,
    /// Channel name
    pub name: String,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub added_at: DateTime<Utc>,
}

/// A saved playlist (either a YouTube URL or a local playlist)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPlaylist {
    /// Playlist ID (PLxxxxx for YouTube, local-xxxxx for local)
    pub id: String,
    /// Full URL for YouTube playlists, empty for local
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub num_tracks: usize,
    pub thumbnail_url: Option<String>,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub added_at: DateTime<Utc>,
    /// true = local playlist, false = saved YouTube playlist URL
    pub is_user_created: bool,
}

/// Track IDs for a local playlist
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalPlaylistTracks {
    pub playlist_id: String,
    pub track_ids: Vec<String>,
}

impl YouTubeStorage {
    const CURRENT_VERSION: u32 = 1;

    /// Get the storage file path
    fn storage_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
        Ok(config_dir.join("drift").join("youtube_data.toml"))
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
        // Don't add duplicates
        if !self.favorite_tracks.iter().any(|t| t.id == track.id) {
            self.favorite_tracks.push(track);
        }
    }

    /// Remove a track from favorites by ID
    pub fn remove_favorite_track(&mut self, track_id: &str) -> bool {
        let len_before = self.favorite_tracks.len();
        self.favorite_tracks.retain(|t| t.id != track_id);
        self.favorite_tracks.len() < len_before
    }

    /// Check if a track is in favorites
    pub fn is_favorite_track(&self, track_id: &str) -> bool {
        self.favorite_tracks.iter().any(|t| t.id == track_id)
    }

    // === Favorite Channels/Artists ===

    /// Add a channel to favorites
    pub fn add_favorite_channel(&mut self, channel: StoredChannel) {
        if !self.favorite_channels.iter().any(|c| c.id == channel.id) {
            self.favorite_channels.push(channel);
        }
    }

    /// Remove a channel from favorites by ID
    pub fn remove_favorite_channel(&mut self, channel_id: &str) -> bool {
        let len_before = self.favorite_channels.len();
        self.favorite_channels.retain(|c| c.id != channel_id);
        self.favorite_channels.len() < len_before
    }

    // === Saved Playlists ===

    /// Add a saved YouTube playlist
    pub fn add_saved_playlist(&mut self, playlist: SavedPlaylist) {
        if !self.saved_playlists.iter().any(|p| p.id == playlist.id) {
            self.saved_playlists.push(playlist);
        }
    }

    /// Remove a playlist by ID
    pub fn remove_saved_playlist(&mut self, playlist_id: &str) -> bool {
        let len_before = self.saved_playlists.len();
        self.saved_playlists.retain(|p| p.id != playlist_id);
        // Also remove associated tracks if local playlist
        self.local_playlist_tracks
            .retain(|lpt| lpt.playlist_id != playlist_id);
        self.saved_playlists.len() < len_before
    }

    /// Create a new local playlist
    pub fn create_local_playlist(&mut self, name: &str, description: Option<&str>) -> SavedPlaylist {
        let id = format!("local-{}", uuid_simple());
        let playlist = SavedPlaylist {
            id: id.clone(),
            url: String::new(),
            title: name.to_string(),
            description: description.map(|s| s.to_string()),
            num_tracks: 0,
            thumbnail_url: None,
            added_at: Utc::now(),
            is_user_created: true,
        };
        self.saved_playlists.push(playlist.clone());
        self.local_playlist_tracks.push(LocalPlaylistTracks {
            playlist_id: id,
            track_ids: Vec::new(),
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
            .find(|p| p.id == playlist_id && p.is_user_created)
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

    /// Get track IDs for a local playlist
    pub fn get_local_playlist_tracks(&self, playlist_id: &str) -> Vec<String> {
        self.local_playlist_tracks
            .iter()
            .find(|lpt| lpt.playlist_id == playlist_id)
            .map(|lpt| lpt.track_ids.clone())
            .unwrap_or_default()
    }

    /// Add tracks to a local playlist
    pub fn add_tracks_to_local_playlist(&mut self, playlist_id: &str, track_ids: &[String]) -> bool {
        if let Some(lpt) = self
            .local_playlist_tracks
            .iter_mut()
            .find(|lpt| lpt.playlist_id == playlist_id)
        {
            for track_id in track_ids {
                if !lpt.track_ids.contains(track_id) {
                    lpt.track_ids.push(track_id.clone());
                }
            }
            // Update track count in saved_playlists
            if let Some(playlist) = self
                .saved_playlists
                .iter_mut()
                .find(|p| p.id == playlist_id)
            {
                playlist.num_tracks = lpt.track_ids.len();
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
                if idx < lpt.track_ids.len() {
                    lpt.track_ids.remove(idx);
                }
            }
            // Update track count
            if let Some(playlist) = self
                .saved_playlists
                .iter_mut()
                .find(|p| p.id == playlist_id)
            {
                playlist.num_tracks = lpt.track_ids.len();
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
    format!(
        "{:x}{:x}",
        duration.as_secs(),
        duration.subsec_nanos()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_favorite_tracks() {
        let mut storage = YouTubeStorage::default();

        let track = StoredTrack {
            id: "test123".to_string(),
            title: "Test Track".to_string(),
            channel_id: "UC123".to_string(),
            channel_name: "Test Channel".to_string(),
            duration_seconds: 180,
            thumbnail_url: None,
            added_at: Utc::now(),
        };

        storage.add_favorite_track(track.clone());
        assert!(storage.is_favorite_track("test123"));
        assert_eq!(storage.favorite_tracks.len(), 1);

        // No duplicates
        storage.add_favorite_track(track);
        assert_eq!(storage.favorite_tracks.len(), 1);

        // Remove
        assert!(storage.remove_favorite_track("test123"));
        assert!(!storage.is_favorite_track("test123"));
    }

    #[test]
    fn test_local_playlist() {
        let mut storage = YouTubeStorage::default();

        let playlist = storage.create_local_playlist("My Playlist", Some("Description"));
        assert!(playlist.is_user_created);
        assert!(playlist.id.starts_with("local-"));

        // Add tracks
        storage.add_tracks_to_local_playlist(&playlist.id, &["track1".to_string(), "track2".to_string()]);
        let tracks = storage.get_local_playlist_tracks(&playlist.id);
        assert_eq!(tracks.len(), 2);

        // Remove track at index 0
        storage.remove_tracks_from_local_playlist(&playlist.id, &[0]);
        let tracks = storage.get_local_playlist_tracks(&playlist.id);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0], "track2");
    }
}
