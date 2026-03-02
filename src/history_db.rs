use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::service::{CoverArt, ServiceType, Track};

const MAX_HISTORY_SIZE: usize = 500;
const DEDUP_WINDOW_SECONDS: i64 = 10;

const HISTORY_TABLE: TableDefinition<u64, &[u8]> = TableDefinition::new("playback_history");

/// Serialized form stored as JSON bytes in redb.
#[derive(Serialize, Deserialize)]
struct StoredEntry {
    track_id: String,
    title: String,
    artist: String,
    album: String,
    duration_seconds: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    cover_art_id: Option<String>,
    service: String,
    played_at_ms: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HistoryEntry {
    pub id: i64,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: u32,
    pub cover_art_id: Option<String>,
    pub service: ServiceType,
    pub played_at: DateTime<Utc>,
}

impl From<&HistoryEntry> for Track {
    fn from(entry: &HistoryEntry) -> Self {
        Track {
            id: entry.track_id.clone(),
            title: entry.title.clone(),
            artist: entry.artist.clone(),
            album: entry.album.clone(),
            duration_seconds: entry.duration_seconds,
            cover_art: CoverArt::from_tidal_option(entry.cover_art_id.clone()),
            service: entry.service,
        }
    }
}

impl StoredEntry {
    fn from_track(track: &Track, now_ms: u64) -> Self {
        let cover_art_id = match &track.cover_art {
            CoverArt::ServiceId { id, .. } => Some(id.clone()),
            CoverArt::Url(url) => Some(url.clone()),
            CoverArt::None => None,
        };
        Self {
            track_id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_seconds: track.duration_seconds,
            cover_art_id,
            service: track.service.to_string(),
            played_at_ms: now_ms,
        }
    }

    fn to_history_entry(&self, key: u64) -> HistoryEntry {
        let played_at = DateTime::from_timestamp_millis(self.played_at_ms as i64)
            .unwrap_or_else(Utc::now);
        let service = self.service.parse().unwrap_or(ServiceType::Tidal);
        HistoryEntry {
            id: key as i64,
            track_id: self.track_id.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration_seconds: self.duration_seconds,
            cover_art_id: self.cover_art_id.clone(),
            service,
            played_at,
        }
    }
}

pub struct HistoryDb {
    db: Database,
}

impl HistoryDb {
    pub fn new() -> Result<Self> {
        let db_path = Self::get_db_path()?;
        let db = Database::create(&db_path)
            .context("Failed to open history database")?;
        // Ensure table exists
        let txn = db.begin_write()?;
        { let _ = txn.open_table(HISTORY_TABLE)?; }
        txn.commit()?;
        Ok(Self { db })
    }

    fn get_db_path() -> Result<PathBuf> {
        let data_dir = dirs::data_dir()
            .context("Failed to get data directory")?
            .join("drift");
        std::fs::create_dir_all(&data_dir)
            .context("Failed to create data directory")?;
        Ok(data_dir.join("history.redb"))
    }

    pub fn record_play(&self, track: &Track) -> Result<()> {
        let now_ms = Utc::now().timestamp_millis() as u64;
        let cutoff_ms = now_ms.saturating_sub((DEDUP_WINDOW_SECONDS * 1000) as u64);

        // Check for recent duplicate
        {
            let rtxn = self.db.begin_read()?;
            let table = rtxn.open_table(HISTORY_TABLE)?;
            // Scan entries from cutoff to now
            let range = table.range(cutoff_ms..=now_ms)?;
            for entry in range {
                let (_, val) = entry?;
                if let Ok(stored) = serde_json::from_slice::<StoredEntry>(val.value()) {
                    if stored.track_id == track.id {
                        return Ok(()); // Dedup — skip
                    }
                }
            }
        }

        // Insert
        let stored = StoredEntry::from_track(track, now_ms);
        let json = serde_json::to_vec(&stored)?;
        {
            let txn = self.db.begin_write()?;
            {
                let mut table = txn.open_table(HISTORY_TABLE)?;
                // Ensure unique key (if two plays at same ms, bump)
                let mut key = now_ms;
                while table.get(key)?.is_some() {
                    key += 1;
                }
                table.insert(key, json.as_slice())?;
            }
            txn.commit()?;
        }

        self.prune_old_entries()?;
        Ok(())
    }

    pub fn get_recent(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let rtxn = self.db.begin_read()?;
        let table = rtxn.open_table(HISTORY_TABLE)?;
        let mut entries = Vec::with_capacity(limit);
        // Reverse iterate (newest first)
        for item in table.iter()?.rev() {
            if entries.len() >= limit {
                break;
            }
            let (key, val) = item?;
            if let Ok(stored) = serde_json::from_slice::<StoredEntry>(val.value()) {
                entries.push(stored.to_history_entry(key.value()));
            }
        }
        Ok(entries)
    }

    #[allow(dead_code)]
    pub fn clear_history(&self) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(HISTORY_TABLE)?;
            // Collect all keys then delete
            let keys: Vec<u64> = table.iter()?
                .map(|r| r.map(|(k, _)| k.value()))
                .collect::<std::result::Result<_, _>>()?;
            for key in keys {
                table.remove(key)?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    fn prune_old_entries(&self) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(HISTORY_TABLE)?;
            let count = table.len()? as usize;
            if count > MAX_HISTORY_SIZE {
                let to_delete = count - MAX_HISTORY_SIZE;
                // Collect oldest keys (forward iteration = ascending order)
                let keys: Vec<u64> = table.iter()?
                    .take(to_delete)
                    .map(|r| r.map(|(k, _)| k.value()))
                    .collect::<std::result::Result<_, _>>()?;
                for key in keys {
                    table.remove(key)?;
                }
            }
        }
        txn.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn new_in_memory() -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("drift-history-test-{}-{}.redb", std::process::id(), n));
        let db = Database::create(&path)
            .context("Failed to create test database")?;
        let txn = db.begin_write()?;
        { let _ = txn.open_table(HISTORY_TABLE)?; }
        txn.commit()?;
        Ok(Self { db })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_track(id: &str, title: &str, artist: &str) -> Track {
        Track {
            id: id.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            album: "Test Album".to_string(),
            duration_seconds: 180,
            cover_art: CoverArt::tidal("cover-123".to_string()),
            service: ServiceType::Tidal,
        }
    }

    #[test]
    fn test_db_init() {
        let db = HistoryDb::new_in_memory().unwrap();
        let entries = db.get_recent(100).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_record_and_get() {
        let db = HistoryDb::new_in_memory().unwrap();
        let track = create_test_track("1", "Song One", "Artist One");

        db.record_play(&track).unwrap();

        let entries = db.get_recent(10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].track_id, "1");
        assert_eq!(entries[0].title, "Song One");
        assert_eq!(entries[0].artist, "Artist One");
    }

    #[test]
    fn test_ordering_most_recent_first() {
        let db = HistoryDb::new_in_memory().unwrap();

        // Insert entries with different timestamps
        let txn = db.db.begin_write().unwrap();
        {
            let mut table = txn.open_table(HISTORY_TABLE).unwrap();
            let base_ms = Utc::now().timestamp_millis() as u64;
            for i in 1u64..=5 {
                let stored = StoredEntry {
                    track_id: i.to_string(),
                    title: format!("Song {}", i),
                    artist: "Artist".to_string(),
                    album: "Album".to_string(),
                    duration_seconds: 180,
                    cover_art_id: None,
                    service: "tidal".to_string(),
                    played_at_ms: base_ms - (6 - i) * 1000, // older entries have lower timestamps
                };
                let json = serde_json::to_vec(&stored).unwrap();
                table.insert(stored.played_at_ms, json.as_slice()).unwrap();
            }
        }
        txn.commit().unwrap();

        let entries = db.get_recent(10).unwrap();
        assert_eq!(entries.len(), 5);
        // Most recent should be first (Song 5 has highest timestamp)
        assert_eq!(entries[0].track_id, "5");
        assert_eq!(entries[4].track_id, "1");
    }

    #[test]
    fn test_limit() {
        let db = HistoryDb::new_in_memory().unwrap();

        let txn = db.db.begin_write().unwrap();
        {
            let mut table = txn.open_table(HISTORY_TABLE).unwrap();
            let base_ms = Utc::now().timestamp_millis() as u64;
            for i in 1u64..=10 {
                let stored = StoredEntry {
                    track_id: i.to_string(),
                    title: format!("Song {}", i),
                    artist: "Artist".to_string(),
                    album: "Album".to_string(),
                    duration_seconds: 180,
                    cover_art_id: None,
                    service: "tidal".to_string(),
                    played_at_ms: base_ms + i,
                };
                let json = serde_json::to_vec(&stored).unwrap();
                table.insert(stored.played_at_ms, json.as_slice()).unwrap();
            }
        }
        txn.commit().unwrap();

        let entries = db.get_recent(5).unwrap();
        assert_eq!(entries.len(), 5);
    }

    #[test]
    fn test_clear_history() {
        let db = HistoryDb::new_in_memory().unwrap();
        let track = create_test_track("1", "Song One", "Artist One");

        db.record_play(&track).unwrap();
        assert_eq!(db.get_recent(10).unwrap().len(), 1);

        db.clear_history().unwrap();
        assert!(db.get_recent(10).unwrap().is_empty());
    }

    #[test]
    fn test_track_from_history_entry() {
        let entry = HistoryEntry {
            id: 1,
            track_id: "12345".to_string(),
            title: "Test Song".to_string(),
            artist: "Test Artist".to_string(),
            album: "Test Album".to_string(),
            duration_seconds: 240,
            cover_art_id: Some("cover-abc".to_string()),
            service: ServiceType::Tidal,
            played_at: Utc::now(),
        };

        let track = Track::from(&entry);

        assert_eq!(track.id, "12345");
        assert_eq!(track.title, "Test Song");
        assert_eq!(track.artist, "Test Artist");
        assert_eq!(track.album, "Test Album");
        assert_eq!(track.duration_seconds, 240);
    }

    #[test]
    fn test_unicode_metadata() {
        let db = HistoryDb::new_in_memory().unwrap();
        let track = Track {
            id: "1".to_string(),
            title: "日本語タイトル".to_string(),
            artist: "アーティスト".to_string(),
            album: "Альбом".to_string(),
            duration_seconds: 180,
            cover_art: CoverArt::None,
            service: ServiceType::Tidal,
        };

        db.record_play(&track).unwrap();
        let entries = db.get_recent(10).unwrap();

        assert_eq!(entries[0].title, "日本語タイトル");
        assert_eq!(entries[0].artist, "アーティスト");
        assert_eq!(entries[0].album, "Альбом");
    }

    #[test]
    fn test_prune_old_entries() {
        let db = HistoryDb::new_in_memory().unwrap();

        // Insert more than MAX_HISTORY_SIZE entries directly
        let txn = db.db.begin_write().unwrap();
        {
            let mut table = txn.open_table(HISTORY_TABLE).unwrap();
            let base_ms = 1_000_000_000_000u64; // some base timestamp
            for i in 0..(MAX_HISTORY_SIZE + 10) as u64 {
                let stored = StoredEntry {
                    track_id: i.to_string(),
                    title: format!("Song {}", i),
                    artist: "Artist".to_string(),
                    album: "Album".to_string(),
                    duration_seconds: 180,
                    cover_art_id: None,
                    service: "tidal".to_string(),
                    played_at_ms: base_ms + i,
                };
                let json = serde_json::to_vec(&stored).unwrap();
                table.insert(stored.played_at_ms, json.as_slice()).unwrap();
            }
        }
        txn.commit().unwrap();

        // Trigger prune
        db.prune_old_entries().unwrap();

        let rtxn = db.db.begin_read().unwrap();
        let table = rtxn.open_table(HISTORY_TABLE).unwrap();
        assert_eq!(table.len().unwrap() as usize, MAX_HISTORY_SIZE);
    }
}
