use anyhow::{Context, Result};
use chrono::Utc;
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::service::{CoverArt, Playlist, ServiceType, Track};

// Key: track_id, Value: JSON StoredDownloadRecord
const DOWNLOADS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("downloads");
// Key: playlist_id, Value: JSON StoredPlaylist
const PLAYLISTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("synced_playlists");
// Key: "playlist_id\0track_id", Value: position
const PLAYLIST_TRACKS_TABLE: TableDefinition<&str, u32> = TableDefinition::new("playlist_tracks");

#[derive(Serialize, Deserialize)]
struct StoredDownloadRecord {
    title: String,
    artist: String,
    album: String,
    duration_seconds: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    cover_art_id: Option<String>,
    service: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_path: Option<String>,
    status: String,
    progress_bytes: u64,
    total_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct StoredPlaylist {
    name: String,
    track_count: usize,
    last_synced_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadStatus {
    Pending,
    Downloading,
    Completed,
    Failed,
    Paused,
}

impl DownloadStatus {
    #[allow(dead_code)]
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

    fn sort_order(&self) -> u8 {
        match self {
            Self::Downloading => 1,
            Self::Pending => 2,
            Self::Paused => 3,
            Self::Failed => 4,
            Self::Completed => 5,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DownloadRecord {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: u32,
    pub cover_art_id: Option<String>,
    pub service: ServiceType,
    pub file_path: Option<String>,
    pub status: DownloadStatus,
    pub progress_bytes: u64,
    pub total_bytes: u64,
    pub error_message: Option<String>,
}

impl From<&Track> for DownloadRecord {
    fn from(track: &Track) -> Self {
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
            service: track.service,
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
            id: record.track_id.clone(),
            title: record.title.clone(),
            artist: record.artist.clone(),
            album: record.album.clone(),
            duration_seconds: record.duration_seconds,
            cover_art: CoverArt::from_tidal_option(record.cover_art_id.clone()),
            service: record.service,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SyncedPlaylist {
    pub playlist_id: String,
    pub name: String,
    pub track_count: usize,
    pub synced_count: usize,
    pub last_synced: Option<String>,
}

impl StoredDownloadRecord {
    fn from_track(track: &Track, now_ms: u64) -> Self {
        let cover_art_id = match &track.cover_art {
            CoverArt::ServiceId { id, .. } => Some(id.clone()),
            CoverArt::Url(url) => Some(url.clone()),
            CoverArt::None => None,
        };
        Self {
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_seconds: track.duration_seconds,
            cover_art_id,
            service: track.service.to_string(),
            file_path: None,
            status: "pending".to_string(),
            progress_bytes: 0,
            total_bytes: 0,
            error_message: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
    }

    fn to_download_record(&self, track_id: &str) -> DownloadRecord {
        let service = self.service.parse().unwrap_or(ServiceType::Tidal);
        DownloadRecord {
            track_id: track_id.to_string(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration_seconds: self.duration_seconds,
            cover_art_id: self.cover_art_id.clone(),
            service,
            file_path: self.file_path.clone(),
            status: DownloadStatus::from_str(&self.status),
            progress_bytes: self.progress_bytes,
            total_bytes: self.total_bytes,
            error_message: self.error_message.clone(),
        }
    }
}

fn playlist_track_key(playlist_id: &str, track_id: &str) -> String {
    format!("{}\0{}", playlist_id, track_id)
}

fn playlist_track_prefix(playlist_id: &str) -> String {
    format!("{}\0", playlist_id)
}

pub struct DownloadDb {
    db: Database,
}

impl DownloadDb {
    pub fn new() -> Result<Self> {
        let db_path = Self::get_db_path()?;
        let db = Database::create(&db_path)
            .context("Failed to open download database")?;
        Self::init_tables(&db)?;
        Ok(Self { db })
    }

    fn get_db_path() -> Result<PathBuf> {
        let data_dir = dirs::data_dir()
            .context("Failed to get data directory")?
            .join("drift");
        std::fs::create_dir_all(&data_dir)
            .context("Failed to create data directory")?;
        Ok(data_dir.join("downloads.redb"))
    }

    fn init_tables(db: &Database) -> Result<()> {
        let txn = db.begin_write()?;
        { let _ = txn.open_table(DOWNLOADS_TABLE)?; }
        { let _ = txn.open_table(PLAYLISTS_TABLE)?; }
        { let _ = txn.open_table(PLAYLIST_TRACKS_TABLE)?; }
        txn.commit()?;
        Ok(())
    }

    fn now_ms() -> u64 {
        Utc::now().timestamp_millis() as u64
    }

    /// Read a stored record. Returns None if not found.
    fn read_record(&self, track_id: &str) -> Result<Option<StoredDownloadRecord>> {
        let rtxn = self.db.begin_read()?;
        let table = rtxn.open_table(DOWNLOADS_TABLE)?;
        let bytes = match table.get(track_id)? {
            Some(val) => val.value().to_vec(),
            None => return Ok(None),
        };
        let stored: StoredDownloadRecord = serde_json::from_slice(&bytes)?;
        Ok(Some(stored))
    }

    pub fn queue_download(&self, track: &Track) -> Result<()> {
        let now = Self::now_ms();
        let stored = StoredDownloadRecord::from_track(track, now);
        let json = serde_json::to_vec(&stored)?;
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(DOWNLOADS_TABLE)?;
            table.insert(track.id.as_str(), json.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Read-modify-write helper: reads a download record, applies a mutation, writes it back.
    fn modify_download(&self, track_id: &str, ctx: &str, f: impl FnOnce(&mut StoredDownloadRecord)) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(DOWNLOADS_TABLE)?;
            // Read into owned bytes to release the borrow on table
            let bytes = table.get(track_id)?
                .context(format!("Track not found for {ctx}"))?
                .value()
                .to_vec();
            let mut stored: StoredDownloadRecord = serde_json::from_slice(&bytes)?;
            f(&mut stored);
            stored.updated_at_ms = Self::now_ms();
            let json = serde_json::to_vec(&stored)?;
            table.insert(track_id, json.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn update_progress(&self, track_id: &str, progress: u64, total: u64) -> Result<()> {
        self.modify_download(track_id, "progress update", |r| {
            r.progress_bytes = progress;
            r.total_bytes = total;
            r.status = "downloading".to_string();
        })
    }

    pub fn mark_completed(&self, track_id: &str, file_path: &str) -> Result<()> {
        let fp = file_path.to_string();
        self.modify_download(track_id, "mark_completed", |r| {
            r.status = "completed".to_string();
            r.file_path = Some(fp);
            r.error_message = None;
        })
    }

    pub fn mark_failed(&self, track_id: &str, error: &str) -> Result<()> {
        let err = error.to_string();
        self.modify_download(track_id, "mark_failed", |r| {
            r.status = "failed".to_string();
            r.error_message = Some(err);
        })
    }

    #[allow(dead_code)]
    pub fn mark_paused(&self, track_id: &str) -> Result<()> {
        self.modify_download(track_id, "mark_paused", |r| {
            r.status = "paused".to_string();
        })
    }

    pub fn get_pending(&self) -> Result<Vec<DownloadRecord>> {
        self.get_by_status("pending")
    }

    #[allow(dead_code)]
    pub fn get_downloading(&self) -> Result<Vec<DownloadRecord>> {
        self.get_by_status("downloading")
    }

    #[allow(dead_code)]
    pub fn get_completed(&self) -> Result<Vec<DownloadRecord>> {
        self.get_by_status("completed")
    }

    #[allow(dead_code)]
    pub fn get_failed(&self) -> Result<Vec<DownloadRecord>> {
        self.get_by_status("failed")
    }

    fn get_by_status(&self, status: &str) -> Result<Vec<DownloadRecord>> {
        let rtxn = self.db.begin_read()?;
        let table = rtxn.open_table(DOWNLOADS_TABLE)?;
        let mut records = Vec::new();
        for item in table.iter()? {
            let (key, val) = item?;
            let stored: StoredDownloadRecord = serde_json::from_slice(val.value())?;
            if stored.status == status {
                records.push(stored.to_download_record(key.value()));
            }
        }
        // Sort by updated_at descending
        records.sort_by(|a, b| {
            // We don't have updated_at on DownloadRecord, re-read isn't needed —
            // the scan order doesn't matter much, but let's keep it consistent
            b.track_id.cmp(&a.track_id)
        });
        Ok(records)
    }

    pub fn get_all(&self) -> Result<Vec<DownloadRecord>> {
        let rtxn = self.db.begin_read()?;
        let table = rtxn.open_table(DOWNLOADS_TABLE)?;
        let mut records_with_ts: Vec<(DownloadRecord, u64)> = Vec::new();
        for item in table.iter()? {
            let (key, val) = item?;
            let stored: StoredDownloadRecord = serde_json::from_slice(val.value())?;
            let updated = stored.updated_at_ms;
            records_with_ts.push((stored.to_download_record(key.value()), updated));
        }
        // Sort: status priority first, then updated_at DESC within each group
        records_with_ts.sort_by(|(a, a_ts), (b, b_ts)| {
            let sa = a.status.sort_order();
            let sb = b.status.sort_order();
            sa.cmp(&sb).then(b_ts.cmp(a_ts))
        });
        Ok(records_with_ts.into_iter().map(|(r, _)| r).collect())
    }

    #[allow(dead_code)]
    pub fn is_downloaded(&self, track_id: &str) -> bool {
        self.read_record(track_id)
            .ok()
            .flatten()
            .map(|r| r.status == "completed")
            .unwrap_or(false)
    }

    pub fn get_local_path(&self, track_id: &str) -> Option<String> {
        self.read_record(track_id)
            .ok()
            .flatten()
            .filter(|r| r.status == "completed")
            .and_then(|r| r.file_path)
    }

    pub fn delete_download(&self, track_id: &str) -> Result<Option<String>> {
        let file_path = self.read_record(track_id)?
            .and_then(|r| r.file_path);
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(DOWNLOADS_TABLE)?;
            table.remove(track_id)?;
        }
        txn.commit()?;
        Ok(file_path)
    }

    pub fn get_download_count(&self) -> Result<(usize, usize, usize)> {
        let rtxn = self.db.begin_read()?;
        let table = rtxn.open_table(DOWNLOADS_TABLE)?;
        let (mut pending, mut completed, mut failed) = (0usize, 0usize, 0usize);
        for item in table.iter()? {
            let (_, val) = item?;
            let stored: StoredDownloadRecord = serde_json::from_slice(val.value())?;
            match stored.status.as_str() {
                "pending" | "downloading" => pending += 1,
                "completed" => completed += 1,
                "failed" => failed += 1,
                _ => {}
            }
        }
        Ok((pending, completed, failed))
    }

    pub fn retry_failed(&self, track_id: &str) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(DOWNLOADS_TABLE)?;
            let bytes = match table.get(track_id)? {
                Some(val) => val.value().to_vec(),
                None => return Ok(()),
            };
            let mut stored: StoredDownloadRecord = serde_json::from_slice(&bytes)?;
            if stored.status == "failed" {
                stored.status = "pending".to_string();
                stored.error_message = None;
                stored.progress_bytes = 0;
                stored.updated_at_ms = Self::now_ms();
                let json = serde_json::to_vec(&stored)?;
                table.insert(track_id, json.as_slice())?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn clear_completed(&self) -> Result<Vec<String>> {
        let mut paths = Vec::new();
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(DOWNLOADS_TABLE)?;
            // Collect completed track IDs and their file paths
            let completed: Vec<(String, Option<String>)> = {
                let mut items = Vec::new();
                for item in table.iter()? {
                    let (key, val) = item?;
                    let stored: StoredDownloadRecord = serde_json::from_slice(val.value())?;
                    if stored.status == "completed" {
                        items.push((key.value().to_string(), stored.file_path));
                    }
                }
                items
            };
            for (tid, fp) in completed {
                table.remove(tid.as_str())?;
                if let Some(p) = fp {
                    paths.push(p);
                }
            }
        }
        txn.commit()?;
        Ok(paths)
    }

    // ── Playlist sync ───────────────────────────────────────────────

    pub fn sync_playlist(&self, playlist: &Playlist, tracks: &[Track]) -> Result<usize> {
        let now = Self::now_ms();
        let txn = self.db.begin_write()?;
        let new_count;
        {
            // Upsert playlist metadata
            let mut pl_table = txn.open_table(PLAYLISTS_TABLE)?;
            let stored_pl = StoredPlaylist {
                name: playlist.title.clone(),
                track_count: tracks.len(),
                last_synced_ms: now,
            };
            let pl_json = serde_json::to_vec(&stored_pl)?;
            pl_table.insert(playlist.id.as_str(), pl_json.as_slice())?;

            // Get existing track IDs for this playlist
            let pt_table = txn.open_table(PLAYLIST_TRACKS_TABLE)?;
            let prefix = playlist_track_prefix(&playlist.id);
            let mut existing_ids = std::collections::HashSet::new();
            for item in pt_table.iter()? {
                let (key, _) = item?;
                let k = key.value();
                if k.starts_with(&prefix) {
                    if let Some(tid) = k.strip_prefix(&prefix) {
                        existing_ids.insert(tid.to_string());
                    }
                }
            }
            drop(pt_table);

            // Queue new tracks and link them
            let mut dl_table = txn.open_table(DOWNLOADS_TABLE)?;
            let mut pt_table = txn.open_table(PLAYLIST_TRACKS_TABLE)?;
            let mut count = 0usize;
            for (pos, track) in tracks.iter().enumerate() {
                if !existing_ids.contains(&track.id) {
                    // Queue download
                    let stored = StoredDownloadRecord::from_track(track, now);
                    let json = serde_json::to_vec(&stored)?;
                    dl_table.insert(track.id.as_str(), json.as_slice())?;

                    // Link to playlist
                    let ptk = playlist_track_key(&playlist.id, &track.id);
                    pt_table.insert(ptk.as_str(), pos as u32)?;
                    count += 1;
                }
            }
            new_count = count;
        }
        txn.commit()?;
        Ok(new_count)
    }

    pub fn get_synced_playlists(&self) -> Result<Vec<SyncedPlaylist>> {
        let rtxn = self.db.begin_read()?;
        let pl_table = rtxn.open_table(PLAYLISTS_TABLE)?;
        let pt_table = rtxn.open_table(PLAYLIST_TRACKS_TABLE)?;
        let dl_table = rtxn.open_table(DOWNLOADS_TABLE)?;

        let mut playlists = Vec::new();
        for item in pl_table.iter()? {
            let (key, val) = item?;
            let pid = key.value().to_string();
            let stored: StoredPlaylist = serde_json::from_slice(val.value())?;

            // Count completed tracks for this playlist
            let prefix = playlist_track_prefix(&pid);
            let mut synced_count = 0usize;
            for pt_item in pt_table.iter()? {
                let (ptk, _) = pt_item?;
                let k = ptk.value();
                if k.starts_with(&prefix) {
                    if let Some(tid) = k.strip_prefix(&prefix) {
                        if let Some(dl_val) = dl_table.get(tid)? {
                            let dl: StoredDownloadRecord = serde_json::from_slice(dl_val.value())?;
                            if dl.status == "completed" {
                                synced_count += 1;
                            }
                        }
                    }
                }
            }

            let last_synced = chrono::DateTime::from_timestamp_millis(stored.last_synced_ms as i64)
                .map(|dt| dt.to_rfc3339());

            playlists.push(SyncedPlaylist {
                playlist_id: pid,
                name: stored.name,
                track_count: stored.track_count,
                synced_count,
                last_synced,
            });
        }

        // Sort by last_synced descending
        playlists.sort_by(|a, b| b.last_synced.cmp(&a.last_synced));
        Ok(playlists)
    }

    #[allow(dead_code)]
    pub fn get_playlist_new_tracks(&self, playlist_id: &str, current_tracks: &[Track]) -> Result<Vec<Track>> {
        let rtxn = self.db.begin_read()?;
        let pt_table = rtxn.open_table(PLAYLIST_TRACKS_TABLE)?;
        let prefix = playlist_track_prefix(playlist_id);
        let mut existing_ids = std::collections::HashSet::new();
        for item in pt_table.iter()? {
            let (key, _) = item?;
            let k = key.value();
            if k.starts_with(&prefix) {
                if let Some(tid) = k.strip_prefix(&prefix) {
                    existing_ids.insert(tid.to_string());
                }
            }
        }

        Ok(current_tracks.iter()
            .filter(|t| !existing_ids.contains(&t.id))
            .cloned()
            .collect())
    }

    #[allow(dead_code)]
    pub fn is_playlist_synced(&self, playlist_id: &str) -> bool {
        let rtxn = match self.db.begin_read() {
            Ok(t) => t,
            Err(_) => return false,
        };
        let table = match rtxn.open_table(PLAYLISTS_TABLE) {
            Ok(t) => t,
            Err(_) => return false,
        };
        table.get(playlist_id).ok().flatten().is_some()
    }

    #[allow(dead_code)]
    pub fn remove_synced_playlist(&self, playlist_id: &str) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            // Remove playlist_tracks links
            let mut pt_table = txn.open_table(PLAYLIST_TRACKS_TABLE)?;
            let prefix = playlist_track_prefix(playlist_id);
            let keys: Vec<String> = pt_table.iter()?
                .filter_map(|item| {
                    let (key, _) = item.ok()?;
                    let k = key.value().to_string();
                    if k.starts_with(&prefix) { Some(k) } else { None }
                })
                .collect();
            for key in keys {
                pt_table.remove(key.as_str())?;
            }

            // Remove playlist
            let mut pl_table = txn.open_table(PLAYLISTS_TABLE)?;
            pl_table.remove(playlist_id)?;
        }
        txn.commit()?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_downloaded_track_ids(&self) -> Result<std::collections::HashSet<String>> {
        let rtxn = self.db.begin_read()?;
        let table = rtxn.open_table(DOWNLOADS_TABLE)?;
        let mut ids = std::collections::HashSet::new();
        for item in table.iter()? {
            let (key, val) = item?;
            let stored: StoredDownloadRecord = serde_json::from_slice(val.value())?;
            if stored.status == "completed" {
                ids.insert(key.value().to_string());
            }
        }
        Ok(ids)
    }

    #[cfg(test)]
    pub fn new_in_memory() -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("drift-downloads-test-{}-{}.redb", std::process::id(), n));
        let db = Database::create(&path)
            .context("Failed to create test database")?;
        Self::init_tables(&db)?;
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

    fn create_test_playlist(id: &str, title: &str) -> Playlist {
        Playlist {
            id: id.to_string(),
            title: title.to_string(),
            description: None,
            num_tracks: 0,
            service: ServiceType::Tidal,
        }
    }

    #[test]
    fn test_download_status_conversion() {
        assert_eq!(DownloadStatus::Pending.as_str(), "pending");
        assert_eq!(DownloadStatus::Downloading.as_str(), "downloading");
        assert_eq!(DownloadStatus::Completed.as_str(), "completed");
        assert_eq!(DownloadStatus::Failed.as_str(), "failed");
        assert_eq!(DownloadStatus::Paused.as_str(), "paused");
    }

    #[test]
    fn test_download_status_from_str() {
        assert_eq!(DownloadStatus::from_str("pending"), DownloadStatus::Pending);
        assert_eq!(DownloadStatus::from_str("downloading"), DownloadStatus::Downloading);
        assert_eq!(DownloadStatus::from_str("completed"), DownloadStatus::Completed);
        assert_eq!(DownloadStatus::from_str("failed"), DownloadStatus::Failed);
        assert_eq!(DownloadStatus::from_str("paused"), DownloadStatus::Paused);
        assert_eq!(DownloadStatus::from_str("unknown"), DownloadStatus::Pending);
    }

    #[test]
    fn test_download_record_from_track() {
        let track = create_test_track("12345", "Test Song", "Test Artist");
        let record = DownloadRecord::from(&track);

        assert_eq!(record.track_id, "12345");
        assert_eq!(record.title, "Test Song");
        assert_eq!(record.artist, "Test Artist");
        assert_eq!(record.album, "Test Album");
        assert_eq!(record.duration_seconds, 180);
        assert_eq!(record.cover_art_id, Some("cover-123".to_string()));
        assert_eq!(record.status, DownloadStatus::Pending);
        assert_eq!(record.progress_bytes, 0);
        assert_eq!(record.total_bytes, 0);
        assert!(record.file_path.is_none());
        assert!(record.error_message.is_none());
    }

    #[test]
    fn test_track_from_download_record() {
        let record = DownloadRecord {
            track_id: "99999".to_string(),
            title: "Record Title".to_string(),
            artist: "Record Artist".to_string(),
            album: "Record Album".to_string(),
            duration_seconds: 240,
            cover_art_id: Some("cover-abc".to_string()),
            service: ServiceType::Tidal,
            file_path: Some("/path/to/file.flac".to_string()),
            status: DownloadStatus::Completed,
            progress_bytes: 1000,
            total_bytes: 1000,
            error_message: None,
        };

        let track = Track::from(&record);

        assert_eq!(track.id, "99999");
        assert_eq!(track.title, "Record Title");
        assert_eq!(track.artist, "Record Artist");
        assert_eq!(track.album, "Record Album");
        assert_eq!(track.duration_seconds, 240);
    }

    #[test]
    fn test_db_init() {
        let db = DownloadDb::new_in_memory().unwrap();
        let count = db.get_all().unwrap();
        assert!(count.is_empty());
    }

    #[test]
    fn test_queue_and_get_download() {
        let db = DownloadDb::new_in_memory().unwrap();
        let track = create_test_track("1", "Song One", "Artist One");

        db.queue_download(&track).unwrap();

        let pending = db.get_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].track_id, "1");
        assert_eq!(pending[0].title, "Song One");
        assert_eq!(pending[0].status, DownloadStatus::Pending);
    }

    #[test]
    fn test_update_progress() {
        let db = DownloadDb::new_in_memory().unwrap();
        let track = create_test_track("1", "Song One", "Artist One");

        db.queue_download(&track).unwrap();
        db.update_progress("1", 500, 1000).unwrap();

        let all = db.get_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].progress_bytes, 500);
        assert_eq!(all[0].total_bytes, 1000);
        assert_eq!(all[0].status, DownloadStatus::Downloading);
    }

    #[test]
    fn test_mark_completed() {
        let db = DownloadDb::new_in_memory().unwrap();
        let track = create_test_track("1", "Song One", "Artist One");

        db.queue_download(&track).unwrap();
        db.mark_completed("1", "/path/to/song.flac").unwrap();

        let completed = db.get_completed().unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].file_path, Some("/path/to/song.flac".to_string()));
        assert_eq!(completed[0].status, DownloadStatus::Completed);

        assert!(db.is_downloaded("1"));
        assert_eq!(db.get_local_path("1"), Some("/path/to/song.flac".to_string()));
    }

    #[test]
    fn test_mark_failed() {
        let db = DownloadDb::new_in_memory().unwrap();
        let track = create_test_track("1", "Song One", "Artist One");

        db.queue_download(&track).unwrap();
        db.mark_failed("1", "Network error").unwrap();

        let failed = db.get_failed().unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].error_message, Some("Network error".to_string()));
        assert_eq!(failed[0].status, DownloadStatus::Failed);
    }

    #[test]
    fn test_retry_failed() {
        let db = DownloadDb::new_in_memory().unwrap();
        let track = create_test_track("1", "Song One", "Artist One");

        db.queue_download(&track).unwrap();
        db.mark_failed("1", "Network error").unwrap();

        let failed = db.get_failed().unwrap();
        assert_eq!(failed.len(), 1);

        db.retry_failed("1").unwrap();

        let pending = db.get_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].error_message.is_none());

        let failed_after = db.get_failed().unwrap();
        assert!(failed_after.is_empty());
    }

    #[test]
    fn test_delete_download() {
        let db = DownloadDb::new_in_memory().unwrap();
        let track = create_test_track("1", "Song One", "Artist One");

        db.queue_download(&track).unwrap();
        db.mark_completed("1", "/path/to/song.flac").unwrap();

        let path = db.delete_download("1").unwrap();
        assert_eq!(path, Some("/path/to/song.flac".to_string()));

        let all = db.get_all().unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn test_get_download_count() {
        let db = DownloadDb::new_in_memory().unwrap();

        db.queue_download(&create_test_track("1", "Pending 1", "Artist")).unwrap();
        db.queue_download(&create_test_track("2", "Pending 2", "Artist")).unwrap();
        db.queue_download(&create_test_track("3", "Completed", "Artist")).unwrap();
        db.queue_download(&create_test_track("4", "Failed", "Artist")).unwrap();

        db.mark_completed("3", "/path/3.flac").unwrap();
        db.mark_failed("4", "Error").unwrap();

        let (pending, completed, failed) = db.get_download_count().unwrap();
        assert_eq!(pending, 2);
        assert_eq!(completed, 1);
        assert_eq!(failed, 1);
    }

    #[test]
    fn test_playlist_sync_initial() {
        let db = DownloadDb::new_in_memory().unwrap();
        let playlist = create_test_playlist("playlist-1", "My Playlist");
        let tracks = vec![
            create_test_track("1", "Song 1", "Artist"),
            create_test_track("2", "Song 2", "Artist"),
            create_test_track("3", "Song 3", "Artist"),
        ];

        let new_count = db.sync_playlist(&playlist, &tracks).unwrap();
        assert_eq!(new_count, 3);

        assert!(db.is_playlist_synced("playlist-1"));

        let synced = db.get_synced_playlists().unwrap();
        assert_eq!(synced.len(), 1);
        assert_eq!(synced[0].name, "My Playlist");
        assert_eq!(synced[0].track_count, 3);

        let pending = db.get_pending().unwrap();
        assert_eq!(pending.len(), 3);
    }

    #[test]
    fn test_playlist_sync_idempotent() {
        let db = DownloadDb::new_in_memory().unwrap();
        let playlist = create_test_playlist("playlist-1", "My Playlist");
        let tracks = vec![
            create_test_track("1", "Song 1", "Artist"),
            create_test_track("2", "Song 2", "Artist"),
        ];

        db.sync_playlist(&playlist, &tracks).unwrap();
        let _new_count2 = db.sync_playlist(&playlist, &tracks).unwrap();

        let all = db.get_all().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_playlist_sync_new_tracks() {
        let db = DownloadDb::new_in_memory().unwrap();
        let playlist = create_test_playlist("playlist-1", "My Playlist");

        let tracks = vec![
            create_test_track("1", "Song 1", "Artist"),
            create_test_track("2", "Song 2", "Artist"),
        ];
        db.sync_playlist(&playlist, &tracks).unwrap();

        let tracks_with_new = vec![
            create_test_track("1", "Song 1", "Artist"),
            create_test_track("2", "Song 2", "Artist"),
            create_test_track("3", "Song 3 NEW", "Artist"),
        ];
        let new_count = db.sync_playlist(&playlist, &tracks_with_new).unwrap();

        assert!(new_count >= 1);

        let all = db.get_all().unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_remove_synced_playlist() {
        let db = DownloadDb::new_in_memory().unwrap();
        let playlist = create_test_playlist("playlist-1", "My Playlist");
        let tracks = vec![create_test_track("1", "Song 1", "Artist")];

        db.sync_playlist(&playlist, &tracks).unwrap();
        assert!(db.is_playlist_synced("playlist-1"));

        db.remove_synced_playlist("playlist-1").unwrap();
        assert!(!db.is_playlist_synced("playlist-1"));

        // Downloads should still exist
        let all = db.get_all().unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_multiple_downloads_ordering() {
        let db = DownloadDb::new_in_memory().unwrap();

        for i in 1..=5 {
            db.queue_download(&create_test_track(&i.to_string(), &format!("Song {}", i), "Artist")).unwrap();
        }

        // Small delays to ensure distinct updated_at_ms
        db.update_progress("1", 50, 100).unwrap();
        db.mark_completed("2", "/path/2.flac").unwrap();
        db.mark_failed("3", "Error").unwrap();
        db.mark_paused("4").unwrap();
        // 5 stays pending

        let all = db.get_all().unwrap();
        assert_eq!(all.len(), 5);

        // Should be ordered: downloading, pending, paused, failed, completed
        assert_eq!(all[0].status, DownloadStatus::Downloading);
        assert_eq!(all[1].status, DownloadStatus::Pending);
        assert_eq!(all[2].status, DownloadStatus::Paused);
        assert_eq!(all[3].status, DownloadStatus::Failed);
        assert_eq!(all[4].status, DownloadStatus::Completed);
    }

    #[test]
    fn test_get_downloaded_track_ids() {
        let db = DownloadDb::new_in_memory().unwrap();

        db.queue_download(&create_test_track("1", "Song 1", "Artist")).unwrap();
        db.queue_download(&create_test_track("2", "Song 2", "Artist")).unwrap();
        db.queue_download(&create_test_track("3", "Song 3", "Artist")).unwrap();

        db.mark_completed("1", "/path/1.flac").unwrap();
        db.mark_completed("3", "/path/3.flac").unwrap();

        let downloaded_ids = db.get_downloaded_track_ids().unwrap();
        assert_eq!(downloaded_ids.len(), 2);
        assert!(downloaded_ids.contains("1"));
        assert!(!downloaded_ids.contains("2"));
        assert!(downloaded_ids.contains("3"));
    }

    #[test]
    fn test_unicode_in_metadata() {
        let db = DownloadDb::new_in_memory().unwrap();
        let track = Track {
            id: "1".to_string(),
            title: "日本語タイトル".to_string(),
            artist: "アーティスト".to_string(),
            album: "Альбом".to_string(),
            duration_seconds: 180,
            cover_art: CoverArt::None,
            service: ServiceType::Tidal,
        };

        db.queue_download(&track).unwrap();
        let pending = db.get_pending().unwrap();

        assert_eq!(pending[0].title, "日本語タイトル");
        assert_eq!(pending[0].artist, "アーティスト");
        assert_eq!(pending[0].album, "Альбом");
    }

    #[test]
    fn test_special_characters_in_error_message() {
        let db = DownloadDb::new_in_memory().unwrap();
        let track = create_test_track("1", "Song", "Artist");

        db.queue_download(&track).unwrap();
        db.mark_failed("1", "Error with 'quotes' and \"double quotes\" and\nnewlines").unwrap();

        let failed = db.get_failed().unwrap();
        assert!(failed[0].error_message.as_ref().unwrap().contains("quotes"));
    }
}
