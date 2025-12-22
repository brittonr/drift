use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::path::PathBuf;

use crate::tidal::{Track, Playlist};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadStatus {
    Pending,
    Downloading,
    Completed,
    Failed,
    Paused,
}

impl DownloadStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Downloading => "downloading",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Paused => "paused",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "downloading" => Self::Downloading,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "paused" => Self::Paused,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRecord {
    pub track_id: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: u32,
    pub album_cover_id: Option<String>,
    pub file_path: Option<String>,
    pub status: DownloadStatus,
    pub progress_bytes: u64,
    pub total_bytes: u64,
    pub error_message: Option<String>,
}

impl From<&Track> for DownloadRecord {
    fn from(track: &Track) -> Self {
        Self {
            track_id: track.id,
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_seconds: track.duration_seconds,
            album_cover_id: track.album_cover_id.clone(),
            file_path: None,
            status: DownloadStatus::Pending,
            progress_bytes: 0,
            total_bytes: 0,
            error_message: None,
        }
    }
}

impl From<&DownloadRecord> for Track {
    fn from(record: &DownloadRecord) -> Self {
        Self {
            id: record.track_id,
            title: record.title.clone(),
            artist: record.artist.clone(),
            album: record.album.clone(),
            duration_seconds: record.duration_seconds,
            album_cover_id: record.album_cover_id.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyncedPlaylist {
    pub playlist_id: String,
    pub name: String,
    pub track_count: usize,
    pub synced_count: usize,
    pub last_synced: Option<String>,
}

pub struct DownloadDb {
    conn: Connection,
}

impl DownloadDb {
    pub fn new() -> Result<Self> {
        let db_path = Self::get_db_path()?;
        let conn = Connection::open(&db_path)
            .context("Failed to open download database")?;
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
        Ok(data_dir.join("downloads.db"))
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS downloads (
                track_id INTEGER PRIMARY KEY,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                album TEXT NOT NULL,
                duration_seconds INTEGER NOT NULL,
                album_cover_id TEXT,
                file_path TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                progress_bytes INTEGER DEFAULT 0,
                total_bytes INTEGER DEFAULT 0,
                error_message TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE INDEX IF NOT EXISTS idx_status ON downloads(status);
            CREATE INDEX IF NOT EXISTS idx_album ON downloads(album);
            CREATE INDEX IF NOT EXISTS idx_artist ON downloads(artist);

            CREATE TABLE IF NOT EXISTS synced_playlists (
                playlist_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                track_count INTEGER DEFAULT 0,
                last_synced DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS playlist_tracks (
                playlist_id TEXT NOT NULL,
                track_id INTEGER NOT NULL,
                position INTEGER NOT NULL,
                PRIMARY KEY (playlist_id, track_id),
                FOREIGN KEY (playlist_id) REFERENCES synced_playlists(playlist_id) ON DELETE CASCADE,
                FOREIGN KEY (track_id) REFERENCES downloads(track_id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_playlist_tracks ON playlist_tracks(playlist_id);"
        ).context("Failed to initialize database schema")?;
        Ok(())
    }

    pub fn queue_download(&self, track: &Track) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO downloads
             (track_id, title, artist, album, duration_seconds, album_cover_id, status, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', CURRENT_TIMESTAMP)",
            params![
                track.id as i64,
                track.title,
                track.artist,
                track.album,
                track.duration_seconds,
                track.album_cover_id,
            ],
        ).context("Failed to queue download")?;
        Ok(())
    }

    pub fn update_progress(&self, track_id: u64, progress: u64, total: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE downloads
             SET progress_bytes = ?1, total_bytes = ?2, status = 'downloading', updated_at = CURRENT_TIMESTAMP
             WHERE track_id = ?3",
            params![progress as i64, total as i64, track_id as i64],
        ).context("Failed to update progress")?;
        Ok(())
    }

    pub fn mark_completed(&self, track_id: u64, file_path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE downloads
             SET status = 'completed', file_path = ?1, error_message = NULL, updated_at = CURRENT_TIMESTAMP
             WHERE track_id = ?2",
            params![file_path, track_id as i64],
        ).context("Failed to mark completed")?;
        Ok(())
    }

    pub fn mark_failed(&self, track_id: u64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE downloads
             SET status = 'failed', error_message = ?1, updated_at = CURRENT_TIMESTAMP
             WHERE track_id = ?2",
            params![error, track_id as i64],
        ).context("Failed to mark failed")?;
        Ok(())
    }

    pub fn mark_paused(&self, track_id: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE downloads
             SET status = 'paused', updated_at = CURRENT_TIMESTAMP
             WHERE track_id = ?1",
            params![track_id as i64],
        ).context("Failed to mark paused")?;
        Ok(())
    }

    pub fn get_pending(&self) -> Result<Vec<DownloadRecord>> {
        self.get_by_status("pending")
    }

    pub fn get_downloading(&self) -> Result<Vec<DownloadRecord>> {
        self.get_by_status("downloading")
    }

    pub fn get_completed(&self) -> Result<Vec<DownloadRecord>> {
        self.get_by_status("completed")
    }

    pub fn get_failed(&self) -> Result<Vec<DownloadRecord>> {
        self.get_by_status("failed")
    }

    fn get_by_status(&self, status: &str) -> Result<Vec<DownloadRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT track_id, title, artist, album, duration_seconds, album_cover_id,
                    file_path, status, progress_bytes, total_bytes, error_message
             FROM downloads
             WHERE status = ?1
             ORDER BY updated_at DESC"
        )?;

        let records = stmt.query_map([status], |row| {
            Ok(DownloadRecord {
                track_id: row.get::<_, i64>(0)? as u64,
                title: row.get(1)?,
                artist: row.get(2)?,
                album: row.get(3)?,
                duration_seconds: row.get(4)?,
                album_cover_id: row.get(5)?,
                file_path: row.get(6)?,
                status: DownloadStatus::from_str(&row.get::<_, String>(7)?),
                progress_bytes: row.get::<_, i64>(8)? as u64,
                total_bytes: row.get::<_, i64>(9)? as u64,
                error_message: row.get(10)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to fetch download records")?;

        Ok(records)
    }

    pub fn get_all(&self) -> Result<Vec<DownloadRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT track_id, title, artist, album, duration_seconds, album_cover_id,
                    file_path, status, progress_bytes, total_bytes, error_message
             FROM downloads
             ORDER BY
                CASE status
                    WHEN 'downloading' THEN 1
                    WHEN 'pending' THEN 2
                    WHEN 'paused' THEN 3
                    WHEN 'failed' THEN 4
                    WHEN 'completed' THEN 5
                END,
                updated_at DESC"
        )?;

        let records = stmt.query_map([], |row| {
            Ok(DownloadRecord {
                track_id: row.get::<_, i64>(0)? as u64,
                title: row.get(1)?,
                artist: row.get(2)?,
                album: row.get(3)?,
                duration_seconds: row.get(4)?,
                album_cover_id: row.get(5)?,
                file_path: row.get(6)?,
                status: DownloadStatus::from_str(&row.get::<_, String>(7)?),
                progress_bytes: row.get::<_, i64>(8)? as u64,
                total_bytes: row.get::<_, i64>(9)? as u64,
                error_message: row.get(10)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to fetch all download records")?;

        Ok(records)
    }

    pub fn is_downloaded(&self, track_id: u64) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM downloads WHERE track_id = ?1 AND status = 'completed'",
                [track_id as i64],
                |_| Ok(()),
            )
            .is_ok()
    }

    pub fn get_local_path(&self, track_id: u64) -> Option<String> {
        self.conn
            .query_row(
                "SELECT file_path FROM downloads WHERE track_id = ?1 AND status = 'completed'",
                [track_id as i64],
                |row| row.get(0),
            )
            .ok()
    }

    pub fn delete_download(&self, track_id: u64) -> Result<Option<String>> {
        // Get file path before deleting
        let file_path: Option<String> = self.conn
            .query_row(
                "SELECT file_path FROM downloads WHERE track_id = ?1",
                [track_id as i64],
                |row| row.get(0),
            )
            .ok();

        self.conn.execute(
            "DELETE FROM downloads WHERE track_id = ?1",
            [track_id as i64],
        ).context("Failed to delete download")?;

        Ok(file_path)
    }

    pub fn get_download_count(&self) -> Result<(usize, usize, usize)> {
        let pending: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM downloads WHERE status IN ('pending', 'downloading')",
            [],
            |row| row.get(0),
        )?;
        let completed: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM downloads WHERE status = 'completed'",
            [],
            |row| row.get(0),
        )?;
        let failed: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM downloads WHERE status = 'failed'",
            [],
            |row| row.get(0),
        )?;

        Ok((pending as usize, completed as usize, failed as usize))
    }

    pub fn retry_failed(&self, track_id: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE downloads
             SET status = 'pending', error_message = NULL, progress_bytes = 0, updated_at = CURRENT_TIMESTAMP
             WHERE track_id = ?1 AND status = 'failed'",
            [track_id as i64],
        ).context("Failed to retry download")?;
        Ok(())
    }

    pub fn clear_completed(&self) -> Result<Vec<String>> {
        // Get all file paths first
        let mut stmt = self.conn.prepare(
            "SELECT file_path FROM downloads WHERE status = 'completed' AND file_path IS NOT NULL"
        )?;
        let paths: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        self.conn.execute(
            "DELETE FROM downloads WHERE status = 'completed'",
            [],
        )?;

        Ok(paths)
    }

    // Playlist sync methods

    pub fn sync_playlist(&self, playlist: &Playlist, tracks: &[Track]) -> Result<usize> {
        // Insert or update playlist
        self.conn.execute(
            "INSERT OR REPLACE INTO synced_playlists (playlist_id, name, track_count, last_synced)
             VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)",
            params![playlist.id, playlist.title, tracks.len()],
        )?;

        // Get existing track IDs for this playlist
        let mut stmt = self.conn.prepare(
            "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1"
        )?;
        let existing_ids: std::collections::HashSet<u64> = stmt
            .query_map([&playlist.id], |row| Ok(row.get::<_, i64>(0)? as u64))?
            .filter_map(|r| r.ok())
            .collect();

        // Find new tracks
        let mut new_count = 0;
        for (pos, track) in tracks.iter().enumerate() {
            if !existing_ids.contains(&track.id) {
                // Queue the download
                self.queue_download(track)?;

                // Link to playlist
                self.conn.execute(
                    "INSERT OR REPLACE INTO playlist_tracks (playlist_id, track_id, position)
                     VALUES (?1, ?2, ?3)",
                    params![playlist.id, track.id as i64, pos],
                )?;

                new_count += 1;
            }
        }

        Ok(new_count)
    }

    pub fn get_synced_playlists(&self) -> Result<Vec<SyncedPlaylist>> {
        let mut stmt = self.conn.prepare(
            "SELECT sp.playlist_id, sp.name, sp.track_count, sp.last_synced,
                    (SELECT COUNT(*) FROM playlist_tracks pt
                     JOIN downloads d ON pt.track_id = d.track_id
                     WHERE pt.playlist_id = sp.playlist_id AND d.status = 'completed') as synced_count
             FROM synced_playlists sp
             ORDER BY sp.last_synced DESC"
        )?;

        let playlists = stmt
            .query_map([], |row| {
                Ok(SyncedPlaylist {
                    playlist_id: row.get(0)?,
                    name: row.get(1)?,
                    track_count: row.get::<_, i64>(2)? as usize,
                    last_synced: row.get(3)?,
                    synced_count: row.get::<_, i64>(4)? as usize,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(playlists)
    }

    pub fn get_playlist_new_tracks(&self, playlist_id: &str, current_tracks: &[Track]) -> Result<Vec<Track>> {
        // Get track IDs we already have for this playlist
        let mut stmt = self.conn.prepare(
            "SELECT track_id FROM playlist_tracks WHERE playlist_id = ?1"
        )?;
        let existing_ids: std::collections::HashSet<u64> = stmt
            .query_map([playlist_id], |row| Ok(row.get::<_, i64>(0)? as u64))?
            .filter_map(|r| r.ok())
            .collect();

        // Return tracks not in our database
        let new_tracks: Vec<Track> = current_tracks
            .iter()
            .filter(|t| !existing_ids.contains(&t.id))
            .cloned()
            .collect();

        Ok(new_tracks)
    }

    pub fn is_playlist_synced(&self, playlist_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM synced_playlists WHERE playlist_id = ?1",
                [playlist_id],
                |_| Ok(()),
            )
            .is_ok()
    }

    pub fn remove_synced_playlist(&self, playlist_id: &str) -> Result<()> {
        // Remove playlist tracks links (downloads remain)
        self.conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            [playlist_id],
        )?;

        // Remove playlist
        self.conn.execute(
            "DELETE FROM synced_playlists WHERE playlist_id = ?1",
            [playlist_id],
        )?;

        Ok(())
    }

    pub fn get_downloaded_track_ids(&self) -> Result<std::collections::HashSet<u64>> {
        let mut stmt = self.conn.prepare(
            "SELECT track_id FROM downloads WHERE status = 'completed'"
        )?;
        let ids = stmt
            .query_map([], |row| Ok(row.get::<_, i64>(0)? as u64))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }
}
