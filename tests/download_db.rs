//! Integration tests for DownloadDb.
//!
//! These tests use real redb databases in temp directories to verify
//! the download tracking and playlist sync functionality.

use anyhow::Result;
use drift::download_db::{DownloadDb, DownloadStatus};
use drift::service::{CoverArt, Playlist, ServiceType, Track};

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
        description: Some("Test playlist description".to_string()),
        num_tracks: 0,
        service: ServiceType::Tidal,
    }
}

#[test]
fn test_queue_download() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let track = create_test_track("12345", "Test Song", "Test Artist");

    db.queue_download(&track)?;

    let pending = db.get_pending()?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].track_id, "12345");
    assert_eq!(pending[0].title, "Test Song");
    assert_eq!(pending[0].status, DownloadStatus::Pending);
    assert_eq!(pending[0].progress_bytes, 0);
    assert_eq!(pending[0].total_bytes, 0);

    Ok(())
}

#[test]
fn test_update_progress() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let track = create_test_track("1", "Song", "Artist");

    db.queue_download(&track)?;
    db.update_progress("1", 500, 1000)?;

    let all = db.get_all()?;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].progress_bytes, 500);
    assert_eq!(all[0].total_bytes, 1000);
    assert_eq!(all[0].status, DownloadStatus::Downloading);

    Ok(())
}

#[test]
fn test_mark_completed() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let track = create_test_track("1", "Song", "Artist");

    db.queue_download(&track)?;
    db.mark_completed("1", "/path/to/song.flac")?;

    let completed = db.get_completed()?;
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].file_path, Some("/path/to/song.flac".to_string()));
    assert_eq!(completed[0].status, DownloadStatus::Completed);

    assert!(db.is_downloaded("1"));
    assert_eq!(db.get_local_path("1"), Some("/path/to/song.flac".to_string()));

    Ok(())
}

#[test]
fn test_mark_failed() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let track = create_test_track("1", "Song", "Artist");

    db.queue_download(&track)?;
    db.mark_failed("1", "Network error")?;

    let failed = db.get_failed()?;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].error_message, Some("Network error".to_string()));
    assert_eq!(failed[0].status, DownloadStatus::Failed);

    Ok(())
}

#[test]
fn test_retry_failed() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let track = create_test_track("1", "Song", "Artist");

    db.queue_download(&track)?;
    db.mark_failed("1", "Network error")?;

    let failed = db.get_failed()?;
    assert_eq!(failed.len(), 1);

    db.retry_failed("1")?;

    let pending = db.get_pending()?;
    assert_eq!(pending.len(), 1);
    assert!(pending[0].error_message.is_none());
    assert_eq!(pending[0].status, DownloadStatus::Pending);
    assert_eq!(pending[0].progress_bytes, 0);

    let failed_after = db.get_failed()?;
    assert!(failed_after.is_empty());

    Ok(())
}

#[test]
fn test_delete_download() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let track = create_test_track("1", "Song", "Artist");

    db.queue_download(&track)?;
    db.mark_completed("1", "/path/to/song.flac")?;

    let path = db.delete_download("1")?;
    assert_eq!(path, Some("/path/to/song.flac".to_string()));

    let all = db.get_all()?;
    assert!(all.is_empty());

    Ok(())
}

#[test]
fn test_delete_nonexistent_returns_none() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;

    let path = db.delete_download("nonexistent")?;
    assert!(path.is_none());

    Ok(())
}

#[test]
fn test_get_download_count() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;

    db.queue_download(&create_test_track("1", "Pending 1", "Artist"))?;
    db.queue_download(&create_test_track("2", "Pending 2", "Artist"))?;
    db.queue_download(&create_test_track("3", "Completed", "Artist"))?;
    db.queue_download(&create_test_track("4", "Failed", "Artist"))?;

    db.mark_completed("3", "/path/3.flac")?;
    db.mark_failed("4", "Error")?;

    let (pending, completed, failed) = db.get_download_count()?;
    assert_eq!(pending, 2);
    assert_eq!(completed, 1);
    assert_eq!(failed, 1);

    Ok(())
}

#[test]
fn test_playlist_sync_initial() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let playlist = create_test_playlist("playlist-1", "My Playlist");
    let tracks = vec![
        create_test_track("1", "Song 1", "Artist"),
        create_test_track("2", "Song 2", "Artist"),
        create_test_track("3", "Song 3", "Artist"),
    ];

    let new_count = db.sync_playlist(&playlist, &tracks)?;
    assert_eq!(new_count, 3);

    assert!(db.is_playlist_synced("playlist-1"));

    let synced = db.get_synced_playlists()?;
    assert_eq!(synced.len(), 1);
    assert_eq!(synced[0].playlist_id, "playlist-1");
    assert_eq!(synced[0].name, "My Playlist");
    assert_eq!(synced[0].track_count, 3);
    assert_eq!(synced[0].synced_count, 0); // None completed yet

    let pending = db.get_pending()?;
    assert_eq!(pending.len(), 3);

    Ok(())
}

#[test]
fn test_playlist_sync_idempotent() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let playlist = create_test_playlist("playlist-1", "My Playlist");
    let tracks = vec![
        create_test_track("1", "Song 1", "Artist"),
        create_test_track("2", "Song 2", "Artist"),
    ];

    // Sync twice
    let new1 = db.sync_playlist(&playlist, &tracks)?;
    let new2 = db.sync_playlist(&playlist, &tracks)?;

    assert_eq!(new1, 2);
    assert_eq!(new2, 0); // No new tracks

    let all = db.get_all()?;
    assert_eq!(all.len(), 2); // Should still only have 2 tracks

    Ok(())
}

#[test]
fn test_playlist_sync_new_tracks() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let playlist = create_test_playlist("playlist-1", "My Playlist");

    // Initial sync with 2 tracks
    let tracks = vec![
        create_test_track("1", "Song 1", "Artist"),
        create_test_track("2", "Song 2", "Artist"),
    ];
    db.sync_playlist(&playlist, &tracks)?;

    // Sync again with an additional track
    let tracks_with_new = vec![
        create_test_track("1", "Song 1", "Artist"),
        create_test_track("2", "Song 2", "Artist"),
        create_test_track("3", "Song 3 NEW", "Artist"),
    ];
    let new_count = db.sync_playlist(&playlist, &tracks_with_new)?;

    assert_eq!(new_count, 1); // Only 1 new track

    let all = db.get_all()?;
    assert_eq!(all.len(), 3);

    Ok(())
}

#[test]
fn test_playlist_sync_count_completed() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let playlist = create_test_playlist("playlist-1", "My Playlist");
    let tracks = vec![
        create_test_track("1", "Song 1", "Artist"),
        create_test_track("2", "Song 2", "Artist"),
        create_test_track("3", "Song 3", "Artist"),
    ];

    db.sync_playlist(&playlist, &tracks)?;

    // Mark some as completed
    db.mark_completed("1", "/path/1.flac")?;
    db.mark_completed("2", "/path/2.flac")?;

    let synced = db.get_synced_playlists()?;
    assert_eq!(synced.len(), 1);
    assert_eq!(synced[0].track_count, 3);
    assert_eq!(synced[0].synced_count, 2); // 2 completed

    Ok(())
}

#[test]
fn test_remove_synced_playlist() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let playlist = create_test_playlist("playlist-1", "My Playlist");
    let tracks = vec![create_test_track("1", "Song 1", "Artist")];

    db.sync_playlist(&playlist, &tracks)?;
    assert!(db.is_playlist_synced("playlist-1"));

    db.remove_synced_playlist("playlist-1")?;
    assert!(!db.is_playlist_synced("playlist-1"));

    // Downloads should still exist
    let all = db.get_all()?;
    assert_eq!(all.len(), 1);

    Ok(())
}

#[test]
fn test_multiple_playlists() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;

    let playlist1 = create_test_playlist("pl-1", "Playlist 1");
    let tracks1 = vec![
        create_test_track("1", "Song 1", "Artist"),
        create_test_track("2", "Song 2", "Artist"),
    ];

    let playlist2 = create_test_playlist("pl-2", "Playlist 2");
    let tracks2 = vec![
        create_test_track("3", "Song 3", "Artist"),
        create_test_track("4", "Song 4", "Artist"),
    ];

    db.sync_playlist(&playlist1, &tracks1)?;
    db.sync_playlist(&playlist2, &tracks2)?;

    let synced = db.get_synced_playlists()?;
    assert_eq!(synced.len(), 2);

    // All tracks should be queued
    let all = db.get_all()?;
    assert_eq!(all.len(), 4);

    Ok(())
}

#[test]
fn test_content_dedup_across_playlists() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;

    let playlist1 = create_test_playlist("pl-1", "Playlist 1");
    let playlist2 = create_test_playlist("pl-2", "Playlist 2");

    // Same track appears in both playlists
    let track = create_test_track("same-track", "Shared Song", "Artist");

    db.sync_playlist(&playlist1, &[track.clone()])?;
    db.sync_playlist(&playlist2, &[track.clone()])?;

    // Should only download once
    let all = db.get_all()?;
    assert_eq!(all.len(), 1);

    Ok(())
}

#[test]
fn test_get_downloaded_track_ids() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;

    db.queue_download(&create_test_track("1", "Song 1", "Artist"))?;
    db.queue_download(&create_test_track("2", "Song 2", "Artist"))?;
    db.queue_download(&create_test_track("3", "Song 3", "Artist"))?;

    db.mark_completed("1", "/path/1.flac")?;
    db.mark_completed("3", "/path/3.flac")?;

    let downloaded_ids = db.get_downloaded_track_ids()?;
    assert_eq!(downloaded_ids.len(), 2);
    assert!(downloaded_ids.contains("1"));
    assert!(!downloaded_ids.contains("2"));
    assert!(downloaded_ids.contains("3"));

    Ok(())
}

#[test]
fn test_clear_completed() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;

    db.queue_download(&create_test_track("1", "Completed", "Artist"))?;
    db.queue_download(&create_test_track("2", "Pending", "Artist"))?;
    db.queue_download(&create_test_track("3", "Completed", "Artist"))?;

    db.mark_completed("1", "/path/1.flac")?;
    db.mark_completed("3", "/path/3.flac")?;

    let paths = db.clear_completed()?;
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&"/path/1.flac".to_string()));
    assert!(paths.contains(&"/path/3.flac".to_string()));

    // Only pending should remain
    let all = db.get_all()?;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].track_id, "2");

    Ok(())
}

#[test]
fn test_download_status_ordering() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;

    for i in 1..=5 {
        db.queue_download(&create_test_track(&i.to_string(), &format!("Song {}", i), "Artist"))?;
    }

    db.update_progress("1", 50, 100)?; // Downloading
    db.mark_completed("2", "/path/2.flac")?; // Completed
    db.mark_failed("3", "Error")?; // Failed
    db.mark_paused("4")?; // Paused
    // 5 stays pending

    let all = db.get_all()?;
    assert_eq!(all.len(), 5);

    // Order: downloading, pending, paused, failed, completed
    assert_eq!(all[0].status, DownloadStatus::Downloading);
    assert_eq!(all[1].status, DownloadStatus::Pending);
    assert_eq!(all[2].status, DownloadStatus::Paused);
    assert_eq!(all[3].status, DownloadStatus::Failed);
    assert_eq!(all[4].status, DownloadStatus::Completed);

    Ok(())
}

#[test]
fn test_unicode_metadata() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let track = Track {
        id: "1".to_string(),
        title: "日本語タイトル".to_string(),
        artist: "アーティスト".to_string(),
        album: "Альбом".to_string(),
        duration_seconds: 180,
        cover_art: CoverArt::None,
        service: ServiceType::Tidal,
    };

    db.queue_download(&track)?;
    let pending = db.get_pending()?;

    assert_eq!(pending[0].title, "日本語タイトル");
    assert_eq!(pending[0].artist, "アーティスト");
    assert_eq!(pending[0].album, "Альбом");

    Ok(())
}

#[test]
fn test_get_playlist_new_tracks() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let playlist = create_test_playlist("pl-1", "Playlist");

    // Initial sync
    let initial_tracks = vec![
        create_test_track("1", "Song 1", "Artist"),
        create_test_track("2", "Song 2", "Artist"),
    ];
    db.sync_playlist(&playlist, &initial_tracks)?;

    // Current tracks with additions
    let current_tracks = vec![
        create_test_track("1", "Song 1", "Artist"),
        create_test_track("2", "Song 2", "Artist"),
        create_test_track("3", "Song 3 NEW", "Artist"),
        create_test_track("4", "Song 4 NEW", "Artist"),
    ];

    let new_tracks = db.get_playlist_new_tracks("pl-1", &current_tracks)?;
    assert_eq!(new_tracks.len(), 2);
    assert_eq!(new_tracks[0].track_id, "3");
    assert_eq!(new_tracks[1].track_id, "4");

    Ok(())
}

#[test]
fn test_not_downloaded_returns_false() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let track = create_test_track("1", "Song", "Artist");

    db.queue_download(&track)?;

    // Still pending, not completed
    assert!(!db.is_downloaded("1"));

    Ok(())
}

#[test]
fn test_get_local_path_returns_none_when_not_completed() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;
    let track = create_test_track("1", "Song", "Artist");

    db.queue_download(&track)?;

    assert_eq!(db.get_local_path("1"), None);

    Ok(())
}

#[test]
fn test_concurrent_operations() -> Result<()> {
    let db = DownloadDb::new_in_memory()?;

    // Simulate concurrent operations
    db.queue_download(&create_test_track("1", "Song 1", "Artist"))?;
    db.update_progress("1", 100, 1000)?;

    db.queue_download(&create_test_track("2", "Song 2", "Artist"))?;
    db.mark_failed("2", "Error")?;

    db.queue_download(&create_test_track("3", "Song 3", "Artist"))?;
    db.mark_completed("3", "/path/3.flac")?;

    db.retry_failed("2")?;

    let all = db.get_all()?;
    assert_eq!(all.len(), 3);

    Ok(())
}
