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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_track(id: u64, title: &str, artist: &str) -> PersistedTrack {
        PersistedTrack {
            id,
            title: title.to_string(),
            artist: artist.to_string(),
            album: "Test Album".to_string(),
            duration_seconds: 180,
            album_cover_id: Some("cover-123".to_string()),
        }
    }

    #[test]
    fn test_persisted_queue_new() {
        let queue = PersistedQueue::new();

        assert_eq!(queue.version, 1);
        assert!(queue.tracks.is_empty());
        assert!(queue.current_position.is_none());
        assert!(queue.elapsed_seconds.is_none());
    }

    #[test]
    fn test_persisted_track_serialization() {
        let track = create_test_track(12345, "Test Song", "Test Artist");
        let serialized = toml::to_string_pretty(&track).unwrap();

        assert!(serialized.contains("id = 12345"));
        assert!(serialized.contains("title = \"Test Song\""));
        assert!(serialized.contains("artist = \"Test Artist\""));
        assert!(serialized.contains("album = \"Test Album\""));
        assert!(serialized.contains("duration_seconds = 180"));
        assert!(serialized.contains("album_cover_id = \"cover-123\""));
    }

    #[test]
    fn test_persisted_track_deserialization() {
        let toml_str = r#"
id = 99999
title = "Deserialized Track"
artist = "Some Artist"
album = "Some Album"
duration_seconds = 240
album_cover_id = "abc-def"
"#;

        let track: PersistedTrack = toml::from_str(toml_str).unwrap();

        assert_eq!(track.id, 99999);
        assert_eq!(track.title, "Deserialized Track");
        assert_eq!(track.artist, "Some Artist");
        assert_eq!(track.album, "Some Album");
        assert_eq!(track.duration_seconds, 240);
        assert_eq!(track.album_cover_id, Some("abc-def".to_string()));
    }

    #[test]
    fn test_persisted_track_without_cover_id() {
        let toml_str = r#"
id = 11111
title = "No Cover Track"
artist = "Artist"
album = "Album"
duration_seconds = 120
"#;

        let track: PersistedTrack = toml::from_str(toml_str).unwrap();

        assert_eq!(track.id, 11111);
        assert!(track.album_cover_id.is_none());
    }

    #[test]
    fn test_persisted_queue_serialization_roundtrip() {
        let mut queue = PersistedQueue::new();
        queue.tracks.push(create_test_track(1, "Song One", "Artist A"));
        queue.tracks.push(create_test_track(2, "Song Two", "Artist B"));
        queue.current_position = Some(1);
        queue.elapsed_seconds = Some(45);

        let serialized = toml::to_string_pretty(&queue).unwrap();
        let deserialized: PersistedQueue = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.version, queue.version);
        assert_eq!(deserialized.tracks.len(), 2);
        assert_eq!(deserialized.tracks[0].title, "Song One");
        assert_eq!(deserialized.tracks[1].title, "Song Two");
        assert_eq!(deserialized.current_position, Some(1));
        assert_eq!(deserialized.elapsed_seconds, Some(45));
    }

    #[test]
    fn test_empty_queue_serialization() {
        let queue = PersistedQueue::new();
        let serialized = toml::to_string_pretty(&queue).unwrap();

        assert!(serialized.contains("version = 1"));
        assert!(serialized.contains("tracks = []"));
        // Optional fields should not appear when None
        assert!(!serialized.contains("current_position"));
        assert!(!serialized.contains("elapsed_seconds"));
    }

    #[test]
    fn test_queue_with_position_only() {
        let toml_str = r#"
version = 1
tracks = []
current_position = 5
"#;

        let queue: PersistedQueue = toml::from_str(toml_str).unwrap();

        assert_eq!(queue.current_position, Some(5));
        assert!(queue.elapsed_seconds.is_none());
    }

    #[test]
    fn test_queue_version_preserved() {
        let toml_str = r#"
version = 2
tracks = []
"#;

        let queue: PersistedQueue = toml::from_str(toml_str).unwrap();
        assert_eq!(queue.version, 2);
    }

    #[test]
    fn test_special_characters_in_track_title() {
        let track = PersistedTrack {
            id: 1,
            title: "Track with \"quotes\" and 'apostrophes'".to_string(),
            artist: "Artist with\nnewline".to_string(),
            album: "Album with\ttab".to_string(),
            duration_seconds: 60,
            album_cover_id: None,
        };

        let serialized = toml::to_string_pretty(&track).unwrap();
        let deserialized: PersistedTrack = toml::from_str(&serialized).unwrap();

        assert_eq!(track.title, deserialized.title);
        assert_eq!(track.artist, deserialized.artist);
        assert_eq!(track.album, deserialized.album);
    }

    #[test]
    fn test_unicode_in_track_metadata() {
        let track = PersistedTrack {
            id: 1,
            title: "日本語タイトル".to_string(),
            artist: "アーティスト名".to_string(),
            album: "Альбом на русском".to_string(),
            duration_seconds: 300,
            album_cover_id: None,
        };

        let serialized = toml::to_string_pretty(&track).unwrap();
        let deserialized: PersistedTrack = toml::from_str(&serialized).unwrap();

        assert_eq!(track.title, deserialized.title);
        assert_eq!(track.artist, deserialized.artist);
        assert_eq!(track.album, deserialized.album);
    }

    #[test]
    fn test_large_queue() {
        let mut queue = PersistedQueue::new();
        for i in 0..100 {
            queue.tracks.push(create_test_track(i, &format!("Track {}", i), "Artist"));
        }
        queue.current_position = Some(50);

        let serialized = toml::to_string_pretty(&queue).unwrap();
        let deserialized: PersistedQueue = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.tracks.len(), 100);
        assert_eq!(deserialized.tracks[99].title, "Track 99");
        assert_eq!(deserialized.current_position, Some(50));
    }

    #[test]
    fn test_invalid_toml_returns_error() {
        let invalid_toml = "this is not valid [[ toml syntax";
        let result: Result<PersistedQueue, _> = toml::from_str(invalid_toml);
        assert!(result.is_err());
    }
}
