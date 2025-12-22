use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Semaphore};

use crate::config::DownloadsConfig;
use crate::download_db::{DownloadDb, DownloadRecord, SyncedPlaylist};
use crate::tidal::{Playlist, TidalClient, Track};

const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 2;

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Started { track_id: u64, title: String },
    Progress { track_id: u64, downloaded: u64, total: u64 },
    Completed { track_id: u64, path: String },
    Failed { track_id: u64, error: String },
    QueueUpdated,
    PlaylistSynced { playlist_id: String, name: String, new_tracks: usize },
}

pub struct DownloadManager {
    db: DownloadDb,
    download_dir: PathBuf,
    semaphore: Arc<Semaphore>,
    event_tx: mpsc::UnboundedSender<DownloadEvent>,
    is_paused: bool,
}

impl DownloadManager {
    pub fn new() -> Result<(Self, mpsc::UnboundedReceiver<DownloadEvent>)> {
        Self::with_config(&DownloadsConfig::default())
    }

    pub fn with_config(config: &DownloadsConfig) -> Result<(Self, mpsc::UnboundedReceiver<DownloadEvent>)> {
        let download_dir = Self::get_download_dir(config)?;
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let max_concurrent = if config.max_concurrent > 0 {
            config.max_concurrent
        } else {
            DEFAULT_MAX_CONCURRENT_DOWNLOADS
        };

        let manager = Self {
            db: DownloadDb::new()?,
            download_dir,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            event_tx,
            is_paused: false,
        };

        Ok((manager, event_rx))
    }

    fn get_download_dir(config: &DownloadsConfig) -> Result<PathBuf> {
        // Use custom download dir if specified, otherwise use cache dir
        let download_dir = if let Some(ref custom_dir) = config.download_dir {
            PathBuf::from(custom_dir)
        } else {
            dirs::cache_dir()
                .context("Failed to get cache directory")?
                .join("tidal-tui")
                .join("downloads")
        };

        std::fs::create_dir_all(&download_dir)
            .context("Failed to create downloads directory")?;
        Ok(download_dir)
    }

    pub fn queue_track(&self, track: &Track) -> Result<()> {
        self.db.queue_download(track)?;
        let _ = self.event_tx.send(DownloadEvent::QueueUpdated);
        Ok(())
    }

    pub fn queue_tracks(&self, tracks: &[Track]) -> Result<usize> {
        let mut count = 0;
        for track in tracks {
            if self.db.queue_download(track).is_ok() {
                count += 1;
            }
        }
        let _ = self.event_tx.send(DownloadEvent::QueueUpdated);
        Ok(count)
    }

    pub fn is_downloaded(&self, track_id: u64) -> bool {
        self.db.is_downloaded(track_id)
    }

    pub fn get_local_path(&self, track_id: u64) -> Option<String> {
        self.db.get_local_path(track_id)
    }

    pub fn get_all_downloads(&self) -> Result<Vec<DownloadRecord>> {
        self.db.get_all()
    }

    pub fn get_pending_downloads(&self) -> Result<Vec<DownloadRecord>> {
        self.db.get_pending()
    }

    pub fn get_completed_downloads(&self) -> Result<Vec<DownloadRecord>> {
        self.db.get_completed()
    }

    pub fn get_download_counts(&self) -> Result<(usize, usize, usize)> {
        self.db.get_download_count()
    }

    pub fn delete_download(&self, track_id: u64) -> Result<()> {
        if let Some(path) = self.db.delete_download(track_id)? {
            // Try to delete the file
            if let Err(e) = std::fs::remove_file(&path) {
                // File might not exist, that's ok
                if e.kind() != std::io::ErrorKind::NotFound {
                    return Err(anyhow::anyhow!("Failed to delete file: {}", e));
                }
            }
        }
        let _ = self.event_tx.send(DownloadEvent::QueueUpdated);
        Ok(())
    }

    pub fn retry_failed(&self, track_id: u64) -> Result<()> {
        self.db.retry_failed(track_id)?;
        let _ = self.event_tx.send(DownloadEvent::QueueUpdated);
        Ok(())
    }

    pub fn pause(&mut self) {
        self.is_paused = true;
    }

    pub fn resume(&mut self) {
        self.is_paused = false;
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused
    }

    pub async fn process_next_download(
        &self,
        tidal: &mut TidalClient,
        debug_log: &mut VecDeque<String>,
    ) -> Result<bool> {
        if self.is_paused {
            return Ok(false);
        }

        // Try to acquire a permit
        let permit = match self.semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return Ok(false), // All slots busy
        };

        // Get next pending download
        let pending = self.db.get_pending()?;
        let record = match pending.first() {
            Some(r) => r.clone(),
            None => {
                drop(permit);
                return Ok(false);
            }
        };

        let track = Track::from(&record);
        debug_log.push_back(format!("Starting download: {} - {}", track.artist, track.title));

        let _ = self.event_tx.send(DownloadEvent::Started {
            track_id: track.id,
            title: track.title.clone(),
        });

        // Perform the download
        match self.download_track(&track, tidal, debug_log).await {
            Ok(path) => {
                debug_log.push_back(format!("Download complete: {}", track.title));
                let _ = self.event_tx.send(DownloadEvent::Completed {
                    track_id: track.id,
                    path,
                });
            }
            Err(e) => {
                let error = e.to_string();
                debug_log.push_back(format!("Download failed: {} - {}", track.title, error));
                self.db.mark_failed(track.id, &error)?;
                let _ = self.event_tx.send(DownloadEvent::Failed {
                    track_id: track.id,
                    error,
                });
            }
        }

        drop(permit);
        Ok(true)
    }

    async fn download_track(
        &self,
        track: &Track,
        tidal: &mut TidalClient,
        debug_log: &mut VecDeque<String>,
    ) -> Result<String> {
        // Get stream URL (time-limited, must download immediately)
        debug_log.push_back(format!("Getting stream URL for: {}", track.title));
        let stream_url = tidal.get_stream_url(&track.id.to_string()).await?;

        // Determine file path
        let file_path = self.get_download_path(track);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Download the file
        debug_log.push_back(format!("Downloading to: {}", file_path.display()));

        let client = reqwest::Client::new();
        let response = client.get(&stream_url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("HTTP error: {}", response.status()));
        }

        let total_size = response.content_length().unwrap_or(0);
        self.db.update_progress(track.id, 0, total_size)?;

        let mut file = File::create(&file_path).await?;
        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut last_progress_update: u64 = 0;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            // Update progress every ~256KB
            if downloaded - last_progress_update > 256 * 1024 {
                self.db.update_progress(track.id, downloaded, total_size)?;
                let _ = self.event_tx.send(DownloadEvent::Progress {
                    track_id: track.id,
                    downloaded,
                    total: total_size,
                });
                last_progress_update = downloaded;
            }
        }

        file.flush().await?;
        drop(file);

        // Tag the file with metadata
        self.tag_file(&file_path, track)?;

        // Mark complete in database
        let path_str = file_path.to_string_lossy().to_string();
        self.db.mark_completed(track.id, &path_str)?;

        Ok(path_str)
    }

    fn get_download_path(&self, track: &Track) -> PathBuf {
        let artist = sanitize_filename(&track.artist);
        let album = sanitize_filename(&track.album);
        let title = sanitize_filename(&track.title);

        // Use FLAC extension (Tidal streams are typically FLAC for lossless)
        self.download_dir
            .join(&artist)
            .join(&album)
            .join(format!("{}.flac", title))
    }

    fn tag_file(&self, path: &PathBuf, track: &Track) -> Result<()> {
        let extension = path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match extension.as_str() {
            "flac" => self.tag_flac(path, track),
            "mp3" | "m4a" => self.tag_mp3(path, track),
            _ => Ok(()), // Unknown format, skip tagging
        }
    }

    fn tag_flac(&self, path: &PathBuf, track: &Track) -> Result<()> {
        match metaflac::Tag::read_from_path(path) {
            Ok(mut tag) => {
                tag.set_vorbis("TITLE", vec![&track.title]);
                tag.set_vorbis("ARTIST", vec![&track.artist]);
                tag.set_vorbis("ALBUM", vec![&track.album]);
                tag.save().context("Failed to save FLAC tags")?;
            }
            Err(e) => {
                // File might not be valid FLAC, just log and continue
                tracing::warn!("Could not tag FLAC file: {}", e);
            }
        }
        Ok(())
    }

    fn tag_mp3(&self, path: &PathBuf, track: &Track) -> Result<()> {
        use id3::{Tag, TagLike, Version};

        let mut tag = Tag::new();
        tag.set_title(&track.title);
        tag.set_artist(&track.artist);
        tag.set_album(&track.album);

        tag.write_to_path(path, Version::Id3v24)
            .context("Failed to write ID3 tags")?;
        Ok(())
    }

    pub fn get_cache_size(&self) -> Result<u64> {
        let mut total = 0u64;
        if self.download_dir.exists() {
            Self::calculate_dir_size(&self.download_dir, &mut total)?;
        }
        Ok(total)
    }

    fn calculate_dir_size(dir: &std::path::Path, total: &mut u64) -> Result<()> {
        if dir.is_dir() {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    Self::calculate_dir_size(&path, total)?;
                } else {
                    *total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
        Ok(())
    }

    pub fn clear_all_downloads(&self) -> Result<()> {
        // Delete all files
        if self.download_dir.exists() {
            std::fs::remove_dir_all(&self.download_dir)?;
            std::fs::create_dir_all(&self.download_dir)?;
        }

        // Clear database
        self.db.clear_completed()?;

        let _ = self.event_tx.send(DownloadEvent::QueueUpdated);
        Ok(())
    }

    // Playlist sync methods

    pub fn sync_playlist(&self, playlist: &Playlist, tracks: &[Track]) -> Result<usize> {
        let new_count = self.db.sync_playlist(playlist, tracks)?;

        let _ = self.event_tx.send(DownloadEvent::PlaylistSynced {
            playlist_id: playlist.id.clone(),
            name: playlist.title.clone(),
            new_tracks: new_count,
        });
        let _ = self.event_tx.send(DownloadEvent::QueueUpdated);

        Ok(new_count)
    }

    pub fn get_synced_playlists(&self) -> Result<Vec<SyncedPlaylist>> {
        self.db.get_synced_playlists()
    }

    pub fn is_playlist_synced(&self, playlist_id: &str) -> bool {
        self.db.is_playlist_synced(playlist_id)
    }

    pub fn remove_synced_playlist(&self, playlist_id: &str) -> Result<()> {
        self.db.remove_synced_playlist(playlist_id)?;
        let _ = self.event_tx.send(DownloadEvent::QueueUpdated);
        Ok(())
    }

    pub fn get_playlist_new_tracks(&self, playlist_id: &str, current_tracks: &[Track]) -> Result<Vec<Track>> {
        self.db.get_playlist_new_tracks(playlist_id, current_tracks)
    }

    pub fn get_downloaded_track_ids(&self) -> Result<std::collections::HashSet<u64>> {
        self.db.get_downloaded_track_ids()
    }
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

// Format bytes for display
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
