use anyhow::{Context, Result};
use chrono::Utc;
use redb::{Database, ReadableTable, TableDefinition};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use crate::service::{Album, Artist, Playlist, Track};

/// Key: "playlists", "playlist_tracks:{id}", "favorites", "album_tracks:{id}", "artist_data:{id}"
/// Value: JSON bytes of CacheEntry<T>
const METADATA_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata_cache");

/// Internal wrapper for cached data with timestamp.
#[derive(Serialize, Deserialize)]
struct CacheEntry<T> {
    data: T,
    updated_at_ms: u64,
}

/// Staleness info returned with cached data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    /// Data is fresh (within TTL).
    Fresh,
    /// Data exists but is older than TTL. Caller should refresh in background.
    Stale,
}

/// Result of a cache lookup — includes the data and whether it's stale.
#[derive(Debug)]
pub struct CacheHit<T> {
    pub data: T,
    pub status: CacheStatus,
}

pub struct MetadataCache {
    db: Database,
    ttl: Duration,
}

impl MetadataCache {
    /// Create a new metadata cache with the given TTL.
    /// Stores data in the system data directory.
    pub fn new(ttl: Duration) -> Result<Self> {
        let db_path = Self::default_path()?;

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        let db = Database::create(&db_path)
            .with_context(|| format!("Failed to create database at {}", db_path.display()))?;

        Ok(Self { db, ttl })
    }

    /// Create an in-memory cache for testing.
    pub fn new_in_memory(ttl: Duration) -> Result<Self> {
        let db = Database::builder()
            .create_with_backend(redb::backends::InMemoryBackend::new())
            .context("Failed to create in-memory database")?;

        Ok(Self { db, ttl })
    }

    fn default_path() -> Result<PathBuf> {
        let data_dir = dirs::data_dir().context("Failed to determine system data directory")?;
        Ok(data_dir.join("drift").join("metadata.redb"))
    }

    /// Get a cached entry and check staleness.
    fn get_entry<T>(&self, key: &str) -> Result<Option<CacheHit<T>>>
    where
        T: DeserializeOwned,
    {
        let txn = self
            .db
            .begin_read()
            .context("Failed to begin read transaction")?;

        // Table might not exist yet - this is OK, just return None
        let table = match txn.open_table(METADATA_TABLE) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        let value = match table.get(key)? {
            Some(v) => v.value().to_vec(),
            None => return Ok(None),
        };

        let entry: CacheEntry<T> =
            serde_json::from_slice(&value).context("Failed to deserialize cache entry")?;

        let now_ms = Utc::now().timestamp_millis() as u64;
        let ttl_ms = self.ttl.as_millis() as u64;
        let age_ms = now_ms.saturating_sub(entry.updated_at_ms);

        let status = if age_ms <= ttl_ms {
            CacheStatus::Fresh
        } else {
            CacheStatus::Stale
        };

        Ok(Some(CacheHit {
            data: entry.data,
            status,
        }))
    }

    /// Set a cached entry with current timestamp.
    fn set_entry<T>(&self, key: &str, data: &T) -> Result<()>
    where
        T: Serialize + ?Sized,
    {
        let entry = CacheEntry {
            data,
            updated_at_ms: Utc::now().timestamp_millis() as u64,
        };

        let json =
            serde_json::to_vec(&entry).context("Failed to serialize cache entry")?;

        let txn = self
            .db
            .begin_write()
            .context("Failed to begin write transaction")?;

        {
            let mut table = txn
                .open_table(METADATA_TABLE)
                .context("Failed to open metadata table")?;

            table
                .insert(key, json.as_slice())
                .context("Failed to insert cache entry")?;
        }

        txn.commit().context("Failed to commit transaction")?;

        Ok(())
    }

    // Playlists (keyed by "playlists")

    pub fn get_playlists(&self) -> Result<Option<CacheHit<Vec<Playlist>>>> {
        self.get_entry("playlists")
    }

    pub fn set_playlists(&self, playlists: &[Playlist]) -> Result<()> {
        self.set_entry("playlists", playlists)
    }

    // Playlist tracks (keyed by playlist_id)

    pub fn get_playlist_tracks(
        &self,
        playlist_id: &str,
    ) -> Result<Option<CacheHit<Vec<Track>>>> {
        let key = format!("playlist_tracks:{}", playlist_id);
        self.get_entry(&key)
    }

    pub fn set_playlist_tracks(&self, playlist_id: &str, tracks: &[Track]) -> Result<()> {
        let key = format!("playlist_tracks:{}", playlist_id);
        self.set_entry(&key, tracks)
    }

    // Favorites (keyed by "favorites")

    pub fn get_favorites(
        &self,
    ) -> Result<Option<CacheHit<(Vec<Track>, Vec<Album>, Vec<Artist>)>>> {
        self.get_entry("favorites")
    }

    pub fn set_favorites(
        &self,
        tracks: &[Track],
        albums: &[Album],
        artists: &[Artist],
    ) -> Result<()> {
        self.set_entry("favorites", &(tracks, albums, artists))
    }

    // Album tracks (keyed by album_id)

    pub fn get_album_tracks(&self, album_id: &str) -> Result<Option<CacheHit<Vec<Track>>>> {
        let key = format!("album_tracks:{}", album_id);
        self.get_entry(&key)
    }

    pub fn set_album_tracks(&self, album_id: &str, tracks: &[Track]) -> Result<()> {
        let key = format!("album_tracks:{}", album_id);
        self.set_entry(&key, tracks)
    }

    // Artist data (keyed by artist_id)

    pub fn get_artist_data(
        &self,
        artist_id: &str,
    ) -> Result<Option<CacheHit<(Vec<Track>, Vec<Album>)>>> {
        let key = format!("artist_data:{}", artist_id);
        self.get_entry(&key)
    }

    pub fn set_artist_data(
        &self,
        artist_id: &str,
        tracks: &[Track],
        albums: &[Album],
    ) -> Result<()> {
        let key = format!("artist_data:{}", artist_id);
        self.set_entry(&key, &(tracks, albums))
    }

    /// Invalidate a specific cache entry.
    pub fn invalidate(&self, table_key: &str) -> Result<()> {
        let txn = self
            .db
            .begin_write()
            .context("Failed to begin write transaction")?;

        {
            let mut table = txn
                .open_table(METADATA_TABLE)
                .context("Failed to open metadata table")?;

            table
                .remove(table_key)
                .context("Failed to remove cache entry")?;
        }

        txn.commit().context("Failed to commit transaction")?;

        Ok(())
    }

    /// Clear all cached data.
    pub fn clear(&self) -> Result<()> {
        let txn = self
            .db
            .begin_write()
            .context("Failed to begin write transaction")?;

        {
            let mut table = txn
                .open_table(METADATA_TABLE)
                .context("Failed to open metadata table")?;

            // Collect all keys first to avoid borrowing issues
            let keys: Vec<String> = table
                .iter()?
                .filter_map(|entry| entry.ok())
                .map(|(k, _)| k.value().to_string())
                .collect();

            for key in keys {
                table.remove(key.as_str())?;
            }
        }

        txn.commit().context("Failed to commit transaction")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{CoverArt, ServiceType};
    use std::thread::sleep;

    fn make_test_track(id: &str) -> Track {
        Track {
            id: id.to_string(),
            title: format!("Track {}", id),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration_seconds: 180,
            cover_art: CoverArt::None,
            service: ServiceType::Tidal,
        }
    }

    fn make_test_playlist(id: &str) -> Playlist {
        Playlist {
            id: id.to_string(),
            title: format!("Playlist {}", id),
            description: Some("Test playlist".to_string()),
            num_tracks: 10,
            service: ServiceType::Tidal,
        }
    }

    fn make_test_album(id: &str) -> Album {
        Album {
            id: id.to_string(),
            title: format!("Album {}", id),
            artist: "Artist".to_string(),
            num_tracks: 12,
            cover_art: CoverArt::None,
            service: ServiceType::Tidal,
        }
    }

    fn make_test_artist(id: &str) -> Artist {
        Artist {
            id: id.to_string(),
            name: format!("Artist {}", id),
            service: ServiceType::Tidal,
        }
    }

    #[test]
    fn test_set_get_playlists() {
        let cache = MetadataCache::new_in_memory(Duration::from_secs(3600)).unwrap();
        let playlists = vec![make_test_playlist("1"), make_test_playlist("2")];

        cache.set_playlists(&playlists).unwrap();
        let result = cache.get_playlists().unwrap();

        assert!(result.is_some());
        let hit = result.unwrap();
        assert_eq!(hit.data.len(), 2);
        assert_eq!(hit.data[0].id, "1");
        assert_eq!(hit.data[1].id, "2");
        assert_eq!(hit.status, CacheStatus::Fresh);
    }

    #[test]
    fn test_set_get_playlist_tracks() {
        let cache = MetadataCache::new_in_memory(Duration::from_secs(3600)).unwrap();
        let tracks = vec![
            make_test_track("t1"),
            make_test_track("t2"),
            make_test_track("t3"),
        ];

        cache
            .set_playlist_tracks("playlist_123", &tracks)
            .unwrap();
        let result = cache.get_playlist_tracks("playlist_123").unwrap();

        assert!(result.is_some());
        let hit = result.unwrap();
        assert_eq!(hit.data.len(), 3);
        assert_eq!(hit.data[0].id, "t1");
        assert_eq!(hit.status, CacheStatus::Fresh);
    }

    #[test]
    fn test_set_get_favorites() {
        let cache = MetadataCache::new_in_memory(Duration::from_secs(3600)).unwrap();
        let tracks = vec![make_test_track("t1")];
        let albums = vec![make_test_album("a1")];
        let artists = vec![make_test_artist("ar1")];

        cache.set_favorites(&tracks, &albums, &artists).unwrap();
        let result = cache.get_favorites().unwrap();

        assert!(result.is_some());
        let hit = result.unwrap();
        let (cached_tracks, cached_albums, cached_artists) = hit.data;
        assert_eq!(cached_tracks.len(), 1);
        assert_eq!(cached_albums.len(), 1);
        assert_eq!(cached_artists.len(), 1);
        assert_eq!(cached_tracks[0].id, "t1");
        assert_eq!(cached_albums[0].id, "a1");
        assert_eq!(cached_artists[0].id, "ar1");
        assert_eq!(hit.status, CacheStatus::Fresh);
    }

    #[test]
    fn test_set_get_album_tracks() {
        let cache = MetadataCache::new_in_memory(Duration::from_secs(3600)).unwrap();
        let tracks = vec![make_test_track("t1"), make_test_track("t2")];

        cache.set_album_tracks("album_456", &tracks).unwrap();
        let result = cache.get_album_tracks("album_456").unwrap();

        assert!(result.is_some());
        let hit = result.unwrap();
        assert_eq!(hit.data.len(), 2);
        assert_eq!(hit.data[0].id, "t1");
        assert_eq!(hit.status, CacheStatus::Fresh);
    }

    #[test]
    fn test_set_get_artist_data() {
        let cache = MetadataCache::new_in_memory(Duration::from_secs(3600)).unwrap();
        let tracks = vec![make_test_track("t1")];
        let albums = vec![make_test_album("a1"), make_test_album("a2")];

        cache
            .set_artist_data("artist_789", &tracks, &albums)
            .unwrap();
        let result = cache.get_artist_data("artist_789").unwrap();

        assert!(result.is_some());
        let hit = result.unwrap();
        let (cached_tracks, cached_albums) = hit.data;
        assert_eq!(cached_tracks.len(), 1);
        assert_eq!(cached_albums.len(), 2);
        assert_eq!(cached_tracks[0].id, "t1");
        assert_eq!(cached_albums[0].id, "a1");
        assert_eq!(hit.status, CacheStatus::Fresh);
    }

    #[test]
    fn test_cache_miss_returns_none() {
        let cache = MetadataCache::new_in_memory(Duration::from_secs(3600)).unwrap();

        assert!(cache.get_playlists().unwrap().is_none());
        assert!(cache.get_playlist_tracks("nonexistent").unwrap().is_none());
        assert!(cache.get_favorites().unwrap().is_none());
        assert!(cache.get_album_tracks("nonexistent").unwrap().is_none());
        assert!(cache.get_artist_data("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_staleness_fresh() {
        let cache = MetadataCache::new_in_memory(Duration::from_secs(3600)).unwrap();
        let playlists = vec![make_test_playlist("1")];

        cache.set_playlists(&playlists).unwrap();
        let result = cache.get_playlists().unwrap();

        assert!(result.is_some());
        let hit = result.unwrap();
        assert_eq!(hit.status, CacheStatus::Fresh);
    }

    #[test]
    fn test_staleness_stale() {
        // Create cache with very short TTL
        let cache = MetadataCache::new_in_memory(Duration::from_millis(10)).unwrap();
        let playlists = vec![make_test_playlist("1")];

        cache.set_playlists(&playlists).unwrap();

        // Wait for TTL to expire
        sleep(Duration::from_millis(50));

        let result = cache.get_playlists().unwrap();

        assert!(result.is_some());
        let hit = result.unwrap();
        assert_eq!(hit.status, CacheStatus::Stale);
        assert_eq!(hit.data.len(), 1); // Data still returned even if stale
    }

    #[test]
    fn test_invalidate() {
        let cache = MetadataCache::new_in_memory(Duration::from_secs(3600)).unwrap();
        let playlists = vec![make_test_playlist("1")];

        cache.set_playlists(&playlists).unwrap();
        assert!(cache.get_playlists().unwrap().is_some());

        cache.invalidate("playlists").unwrap();
        assert!(cache.get_playlists().unwrap().is_none());
    }

    #[test]
    fn test_clear() {
        let cache = MetadataCache::new_in_memory(Duration::from_secs(3600)).unwrap();

        // Set multiple entries
        cache
            .set_playlists(&vec![make_test_playlist("1")])
            .unwrap();
        cache
            .set_playlist_tracks("p1", &vec![make_test_track("t1")])
            .unwrap();
        cache
            .set_favorites(&vec![make_test_track("t1")], &vec![], &vec![])
            .unwrap();
        cache
            .set_album_tracks("a1", &vec![make_test_track("t1")])
            .unwrap();
        cache
            .set_artist_data("ar1", &vec![make_test_track("t1")], &vec![])
            .unwrap();

        // Verify all exist
        assert!(cache.get_playlists().unwrap().is_some());
        assert!(cache.get_playlist_tracks("p1").unwrap().is_some());
        assert!(cache.get_favorites().unwrap().is_some());
        assert!(cache.get_album_tracks("a1").unwrap().is_some());
        assert!(cache.get_artist_data("ar1").unwrap().is_some());

        // Clear all
        cache.clear().unwrap();

        // Verify all gone
        assert!(cache.get_playlists().unwrap().is_none());
        assert!(cache.get_playlist_tracks("p1").unwrap().is_none());
        assert!(cache.get_favorites().unwrap().is_none());
        assert!(cache.get_album_tracks("a1").unwrap().is_none());
        assert!(cache.get_artist_data("ar1").unwrap().is_none());
    }
}
