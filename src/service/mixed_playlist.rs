use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use super::{CoverArt, Playlist, ServiceType, Track};

/// Storage for mixed-service playlists (playlists containing tracks from multiple services)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MixedPlaylistStorage {
    /// Storage format version
    #[serde(default)]
    pub version: u32,
    /// Mixed playlists
    #[serde(default)]
    pub playlists: Vec<MixedPlaylist>,
}

/// A playlist that can contain tracks from any service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixedPlaylist {
    /// Unique ID (mixed-{timestamp}{nanos})
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub tracks: Vec<MixedTrackRef>,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub updated_at: DateTime<Utc>,
}

/// Reference to a track with cached metadata for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixedTrackRef {
    /// Track ID (format varies by service)
    pub id: String,
    /// Which service this track belongs to
    pub service: ServiceType,
    /// Cached metadata for display without fetching
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: u32,
    pub cover_art_url: Option<String>,
}

impl MixedPlaylistStorage {
    const CURRENT_VERSION: u32 = 1;
    const FILE_NAME: &'static str = "mixed_playlists.toml";

    /// Get the storage file path
    fn storage_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
        Ok(config_dir.join("drift").join(Self::FILE_NAME))
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

    /// Create a new mixed playlist
    pub fn create_playlist(&mut self, title: &str, description: Option<&str>) -> MixedPlaylist {
        let id = format!("mixed-{}", uuid_simple());
        let now = Utc::now();
        let playlist = MixedPlaylist {
            id: id.clone(),
            title: title.to_string(),
            description: description.map(|s| s.to_string()),
            tracks: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        self.playlists.push(playlist.clone());
        playlist
    }

    /// Add a track to a mixed playlist
    pub fn add_track(&mut self, playlist_id: &str, track: &Track) -> bool {
        if let Some(playlist) = self.playlists.iter_mut().find(|p| p.id == playlist_id) {
            // Don't add duplicates
            if playlist.tracks.iter().any(|t| t.id == track.id && t.service == track.service) {
                return false;
            }

            let cover_url = match &track.cover_art {
                CoverArt::Url(url) => Some(url.clone()),
                CoverArt::ServiceId { id, .. } => Some(id.clone()),
                CoverArt::None => None,
            };

            playlist.tracks.push(MixedTrackRef {
                id: track.id.clone(),
                service: track.service,
                title: track.title.clone(),
                artist: track.artist.clone(),
                album: track.album.clone(),
                duration_seconds: track.duration_seconds,
                cover_art_url: cover_url,
            });
            playlist.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Remove tracks from a playlist by indices
    pub fn remove_tracks(&mut self, playlist_id: &str, indices: &[usize]) -> bool {
        if let Some(playlist) = self.playlists.iter_mut().find(|p| p.id == playlist_id) {
            // Remove in reverse order to maintain indices
            let mut sorted_indices: Vec<_> = indices.to_vec();
            sorted_indices.sort_unstable_by(|a, b| b.cmp(a));
            for idx in sorted_indices {
                if idx < playlist.tracks.len() {
                    playlist.tracks.remove(idx);
                }
            }
            playlist.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Update playlist metadata
    pub fn update_playlist(
        &mut self,
        playlist_id: &str,
        title: Option<&str>,
        description: Option<&str>,
    ) -> bool {
        if let Some(playlist) = self.playlists.iter_mut().find(|p| p.id == playlist_id) {
            if let Some(t) = title {
                playlist.title = t.to_string();
            }
            if let Some(d) = description {
                playlist.description = Some(d.to_string());
            }
            playlist.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Delete a playlist
    pub fn delete_playlist(&mut self, playlist_id: &str) -> bool {
        let len_before = self.playlists.len();
        self.playlists.retain(|p| p.id != playlist_id);
        self.playlists.len() < len_before
    }

    /// Check if a playlist ID belongs to a mixed playlist
    pub fn is_mixed_playlist(&self, playlist_id: &str) -> bool {
        playlist_id.starts_with("mixed-") || self.playlists.iter().any(|p| p.id == playlist_id)
    }

    /// Get a playlist by ID
    pub fn get_playlist(&self, playlist_id: &str) -> Option<&MixedPlaylist> {
        self.playlists.iter().find(|p| p.id == playlist_id)
    }

    /// Convert all playlists to service::Playlist format
    pub fn to_playlists(&self) -> Vec<Playlist> {
        self.playlists
            .iter()
            .map(|mp| Playlist {
                id: mp.id.clone(),
                title: mp.title.clone(),
                description: mp.description.clone(),
                num_tracks: mp.tracks.len(),
                // Mixed playlists don't belong to a single service
                // Use Tidal as a placeholder (could add a Mixed variant later)
                service: ServiceType::Tidal,
            })
            .collect()
    }

    /// Convert track refs to Track format
    pub fn get_tracks(&self, playlist_id: &str) -> Vec<Track> {
        self.get_playlist(playlist_id)
            .map(|p| {
                p.tracks
                    .iter()
                    .map(|tr| Track {
                        id: tr.id.clone(),
                        title: tr.title.clone(),
                        artist: tr.artist.clone(),
                        album: tr.album.clone(),
                        duration_seconds: tr.duration_seconds,
                        cover_art: tr
                            .cover_art_url
                            .clone()
                            .map(CoverArt::Url)
                            .unwrap_or(CoverArt::None),
                        service: tr.service,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Generate a simple UUID-like string for playlist IDs
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

    fn create_test_track(service: ServiceType) -> Track {
        Track {
            id: format!("test-{}", service),
            title: format!("Test Track from {}", service),
            artist: "Test Artist".to_string(),
            album: "Test Album".to_string(),
            duration_seconds: 180,
            cover_art: CoverArt::None,
            service,
        }
    }

    #[test]
    fn test_create_mixed_playlist() {
        let mut storage = MixedPlaylistStorage::default();
        let playlist = storage.create_playlist("My Mixed Playlist", Some("Description"));

        assert!(playlist.id.starts_with("mixed-"));
        assert_eq!(playlist.title, "My Mixed Playlist");
        assert_eq!(playlist.description, Some("Description".to_string()));
        assert!(playlist.tracks.is_empty());
        assert_eq!(storage.playlists.len(), 1);
    }

    #[test]
    fn test_add_tracks_from_multiple_services() {
        let mut storage = MixedPlaylistStorage::default();
        let playlist = storage.create_playlist("Mixed", None);

        let tidal_track = create_test_track(ServiceType::Tidal);
        let youtube_track = create_test_track(ServiceType::YouTube);
        let bandcamp_track = create_test_track(ServiceType::Bandcamp);

        assert!(storage.add_track(&playlist.id, &tidal_track));
        assert!(storage.add_track(&playlist.id, &youtube_track));
        assert!(storage.add_track(&playlist.id, &bandcamp_track));

        let tracks = storage.get_tracks(&playlist.id);
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].service, ServiceType::Tidal);
        assert_eq!(tracks[1].service, ServiceType::YouTube);
        assert_eq!(tracks[2].service, ServiceType::Bandcamp);
    }

    #[test]
    fn test_no_duplicate_tracks() {
        let mut storage = MixedPlaylistStorage::default();
        let playlist = storage.create_playlist("Mixed", None);

        let track = create_test_track(ServiceType::Tidal);

        assert!(storage.add_track(&playlist.id, &track));
        assert!(!storage.add_track(&playlist.id, &track)); // Duplicate

        let tracks = storage.get_tracks(&playlist.id);
        assert_eq!(tracks.len(), 1);
    }

    #[test]
    fn test_remove_tracks() {
        let mut storage = MixedPlaylistStorage::default();
        let playlist = storage.create_playlist("Mixed", None);

        storage.add_track(&playlist.id, &create_test_track(ServiceType::Tidal));
        storage.add_track(&playlist.id, &create_test_track(ServiceType::YouTube));
        storage.add_track(&playlist.id, &create_test_track(ServiceType::Bandcamp));

        assert!(storage.remove_tracks(&playlist.id, &[1])); // Remove YouTube

        let tracks = storage.get_tracks(&playlist.id);
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].service, ServiceType::Tidal);
        assert_eq!(tracks[1].service, ServiceType::Bandcamp);
    }

    #[test]
    fn test_delete_playlist() {
        let mut storage = MixedPlaylistStorage::default();
        let playlist = storage.create_playlist("Mixed", None);

        assert_eq!(storage.playlists.len(), 1);
        assert!(storage.delete_playlist(&playlist.id));
        assert_eq!(storage.playlists.len(), 0);
    }

    #[test]
    fn test_is_mixed_playlist() {
        let mut storage = MixedPlaylistStorage::default();
        let playlist = storage.create_playlist("Mixed", None);

        assert!(storage.is_mixed_playlist(&playlist.id));
        assert!(storage.is_mixed_playlist("mixed-anything"));
        assert!(!storage.is_mixed_playlist("regular-playlist"));
    }

    #[test]
    fn test_to_playlists() {
        let mut storage = MixedPlaylistStorage::default();
        let playlist = storage.create_playlist("Mixed", Some("Desc"));
        storage.add_track(&playlist.id, &create_test_track(ServiceType::Tidal));
        storage.add_track(&playlist.id, &create_test_track(ServiceType::YouTube));

        let playlists = storage.to_playlists();
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].title, "Mixed");
        assert_eq!(playlists[0].num_tracks, 2);
    }
}
