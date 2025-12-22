use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::path::PathBuf;

use crate::service::{CoverArt, ServiceType, Track};

const MAX_HISTORY_SIZE: usize = 500;
const DEDUP_WINDOW_SECONDS: i64 = 10;

#[derive(Debug, Clone)]
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

pub struct HistoryDb {
    conn: Connection,
}

impl HistoryDb {
    pub fn new() -> Result<Self> {
        let db_path = Self::get_db_path()?;
        let conn = Connection::open(&db_path)
            .context("Failed to open history database")?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn get_db_path() -> Result<PathBuf> {
        let data_dir = dirs::data_dir()
            .context("Failed to get data directory")?
            .join("tidal-tui");
        std::fs::create_dir_all(&data_dir)
            .context("Failed to create data directory")?;
        Ok(data_dir.join("history.db"))
    }

    fn init_schema(&self) -> Result<()> {
        // Check if we need to migrate from old schema (track_id INTEGER)
        // If old schema exists, drop it (fresh start approach)
        let has_old_schema: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('playback_history') WHERE name = 'track_id' AND type = 'INTEGER'",
            [],
            |row| Ok(row.get::<_, i64>(0)? > 0),
        ).unwrap_or(false);

        if has_old_schema {
            self.conn.execute("DROP TABLE IF EXISTS playback_history", [])
                .context("Failed to drop old history table")?;
        }

        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS playback_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                track_id TEXT NOT NULL,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                album TEXT NOT NULL,
                duration_seconds INTEGER NOT NULL,
                cover_art_id TEXT,
                service TEXT NOT NULL DEFAULT 'tidal',
                played_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE INDEX IF NOT EXISTS idx_played_at ON playback_history(played_at DESC);
            CREATE INDEX IF NOT EXISTS idx_track_id ON playback_history(track_id);"
        ).context("Failed to initialize history schema")?;
        Ok(())
    }

    pub fn record_play(&self, track: &Track) -> Result<()> {
        // Check for duplicate play within dedup window
        let recent_play: Option<i64> = self.conn.query_row(
            "SELECT id FROM playback_history
             WHERE track_id = ?1
             AND played_at > datetime('now', ?2)
             LIMIT 1",
            params![
                &track.id,
                format!("-{} seconds", DEDUP_WINDOW_SECONDS)
            ],
            |row| row.get(0),
        ).ok();

        if recent_play.is_some() {
            // Skip recording, this is a duplicate
            return Ok(());
        }

        // Extract cover art ID if available
        let cover_art_id = match &track.cover_art {
            CoverArt::ServiceId { id, .. } => Some(id.as_str()),
            CoverArt::Url(url) => Some(url.as_str()),
            CoverArt::None => None,
        };

        self.conn.execute(
            "INSERT INTO playback_history
             (track_id, title, artist, album, duration_seconds, cover_art_id, service, played_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
            params![
                &track.id,
                track.title,
                track.artist,
                track.album,
                track.duration_seconds,
                cover_art_id,
                track.service.to_string(),
            ],
        ).context("Failed to record play")?;

        // Prune old entries if we exceed max size
        self.prune_old_entries()?;

        Ok(())
    }

    pub fn get_recent(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, track_id, title, artist, album, duration_seconds, cover_art_id, service, played_at
             FROM playback_history
             ORDER BY played_at DESC
             LIMIT ?1"
        )?;

        let entries = stmt.query_map([limit as i64], |row| {
            let played_at_str: String = row.get(8)?;
            let played_at = DateTime::parse_from_rfc3339(&played_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| {
                    // Fallback: try SQLite datetime format
                    chrono::NaiveDateTime::parse_from_str(&played_at_str, "%Y-%m-%d %H:%M:%S")
                        .map(|ndt| ndt.and_utc())
                        .unwrap_or_else(|_| Utc::now())
                });

            let service_str: String = row.get(7)?;
            let service = service_str.parse().unwrap_or(ServiceType::Tidal);

            Ok(HistoryEntry {
                id: row.get(0)?,
                track_id: row.get(1)?,
                title: row.get(2)?,
                artist: row.get(3)?,
                album: row.get(4)?,
                duration_seconds: row.get(5)?,
                cover_art_id: row.get(6)?,
                service,
                played_at,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to fetch history entries")?;

        Ok(entries)
    }

    pub fn clear_history(&self) -> Result<()> {
        self.conn.execute("DELETE FROM playback_history", [])
            .context("Failed to clear history")?;
        Ok(())
    }

    fn prune_old_entries(&self) -> Result<()> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM playback_history",
            [],
            |row| row.get(0),
        )?;

        if count as usize > MAX_HISTORY_SIZE {
            let to_delete = count as usize - MAX_HISTORY_SIZE;
            self.conn.execute(
                "DELETE FROM playback_history
                 WHERE id IN (
                     SELECT id FROM playback_history
                     ORDER BY played_at ASC
                     LIMIT ?1
                 )",
                [to_delete as i64],
            )?;
        }

        Ok(())
    }

    #[cfg(test)]
    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .context("Failed to open in-memory database")?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
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

        // Insert with slight delay simulation by inserting in order
        for i in 1..=5 {
            db.conn.execute(
                "INSERT INTO playback_history
                 (track_id, title, artist, album, duration_seconds, service, played_at)
                 VALUES (?1, ?2, 'Artist', 'Album', 180, 'tidal', datetime('now', ?3))",
                params![i.to_string(), format!("Song {}", i), format!("-{} seconds", 6 - i)],
            ).unwrap();
        }

        let entries = db.get_recent(10).unwrap();
        assert_eq!(entries.len(), 5);
        // Most recent should be first
        assert_eq!(entries[0].track_id, "5");
        assert_eq!(entries[4].track_id, "1");
    }

    #[test]
    fn test_limit() {
        let db = HistoryDb::new_in_memory().unwrap();

        for i in 1..=10 {
            db.conn.execute(
                "INSERT INTO playback_history
                 (track_id, title, artist, album, duration_seconds, service)
                 VALUES (?1, ?2, 'Artist', 'Album', 180, 'tidal')",
                params![i.to_string(), format!("Song {}", i)],
            ).unwrap();
        }

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

        // Insert more than MAX_HISTORY_SIZE entries
        for i in 1..=(MAX_HISTORY_SIZE + 10) {
            db.conn.execute(
                "INSERT INTO playback_history
                 (track_id, title, artist, album, duration_seconds, service, played_at)
                 VALUES (?1, ?2, 'Artist', 'Album', 180, 'tidal', datetime('now', ?3))",
                params![i.to_string(), format!("Song {}", i), format!("-{} seconds", MAX_HISTORY_SIZE + 10 - i)],
            ).unwrap();
        }

        // Trigger prune
        db.prune_old_entries().unwrap();

        let count: i64 = db.conn.query_row(
            "SELECT COUNT(*) FROM playback_history",
            [],
            |row| row.get(0),
        ).unwrap();

        assert_eq!(count as usize, MAX_HISTORY_SIZE);
    }
}
