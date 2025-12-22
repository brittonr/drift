use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::tidal::Track;

const QUEUE_FILE_NAME: &str = "queue.toml";
const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTrack {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_cover_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedQueue {
    pub version: u32,
    pub tracks: Vec<PersistedTrack>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_position: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_seconds: Option<u32>,
}

impl PersistedQueue {
    pub fn new() -> Self {
        Self {
            version: CURRENT_VERSION,
            tracks: Vec::new(),
            current_position: None,
            elapsed_seconds: None,
        }
    }

    pub fn from_tracks(tracks: &[Track], position: Option<usize>, elapsed: Option<u32>) -> Self {
        Self {
            version: CURRENT_VERSION,
            tracks: tracks.iter().map(PersistedTrack::from).collect(),
            current_position: position,
            elapsed_seconds: elapsed,
        }
    }
}

impl From<&Track> for PersistedTrack {
    fn from(track: &Track) -> Self {
        Self {
            id: track.id,
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_seconds: track.duration_seconds,
            album_cover_id: track.album_cover_id.clone(),
        }
    }
}

impl From<&PersistedTrack> for Track {
    fn from(pt: &PersistedTrack) -> Self {
        Self {
            id: pt.id,
            title: pt.title.clone(),
            artist: pt.artist.clone(),
            album: pt.album.clone(),
            duration_seconds: pt.duration_seconds,
            album_cover_id: pt.album_cover_id.clone(),
        }
    }
}

fn get_queue_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .context("Failed to get config directory")?
        .join("tidal-tui");

    fs::create_dir_all(&config_dir)
        .context("Failed to create config directory")?;

    Ok(config_dir.join(QUEUE_FILE_NAME))
}

pub fn save_queue(queue: &PersistedQueue) -> Result<()> {
    let path = get_queue_path()?;
    let contents = toml::to_string_pretty(queue)
        .context("Failed to serialize queue to TOML")?;
    fs::write(&path, contents)
        .context("Failed to write queue file")?;
    Ok(())
}

pub fn load_queue() -> Result<Option<PersistedQueue>> {
    let path = get_queue_path()?;

    if !path.exists() {
        return Ok(None);
    }

    let contents = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: Could not read queue file: {}", e);
            return Ok(None);
        }
    };

    match toml::from_str(&contents) {
        Ok(queue) => Ok(Some(queue)),
        Err(e) => {
            eprintln!("Warning: Queue file corrupt, starting fresh: {}", e);
            Ok(None)
        }
    }
}
