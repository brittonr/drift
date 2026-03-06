//! Integration tests for LocalStorage through the DriftStorage trait.
//!
//! Tests history recording and search caching which use isolated temp
//! storage via `new_for_test()`. Queue persistence and search history
//! use global config paths and are tested via unit tests instead.

use anyhow::Result;
use drift::service::{Album, Artist, CoverArt, SearchResults, ServiceType, Track};
use drift::storage::local::LocalStorage;
use drift::storage::DriftStorage;
use std::time::Duration;

fn create_test_track(id: &str, title: &str, artist: &str, service: ServiceType) -> Track {
    Track {
        id: id.to_string(),
        title: title.to_string(),
        artist: artist.to_string(),
        album: "Test Album".to_string(),
        duration_seconds: 180,
        cover_art: CoverArt::tidal("cover-123".to_string()),
        service,
    }
}

fn create_search_results(
    num_tracks: usize,
    num_albums: usize,
    num_artists: usize,
) -> SearchResults {
    let tracks = (0..num_tracks)
        .map(|i| {
            create_test_track(
                &format!("track-{}", i),
                &format!("Track {}", i),
                "Artist",
                ServiceType::Tidal,
            )
        })
        .collect();

    let albums = (0..num_albums)
        .map(|i| Album {
            id: format!("album-{}", i),
            title: format!("Album {}", i),
            artist: "Artist".to_string(),
            num_tracks: 10,
            cover_art: CoverArt::tidal("cover-123".to_string()),
            service: ServiceType::Tidal,
        })
        .collect();

    let artists = (0..num_artists)
        .map(|i| Artist {
            id: format!("artist-{}", i),
            name: format!("Artist {}", i),
            service: ServiceType::Tidal,
        })
        .collect();

    SearchResults {
        tracks,
        albums,
        artists,
    }
}

// ── Backend name ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_backend_name() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    assert_eq!(storage.backend_name(), "local");
    Ok(())
}

// ── History: record + retrieve ───────────────────────────────────────

#[tokio::test]
async fn test_record_play_and_get_history_roundtrip() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    let track = create_test_track("12345", "Test Song", "Test Artist", ServiceType::Tidal);

    storage.record_play(&track).await?;

    let history = storage.get_history(10).await?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].track_id, "12345");
    assert_eq!(history[0].title, "Test Song");
    assert_eq!(history[0].artist, "Test Artist");
    assert_eq!(history[0].service, ServiceType::Tidal);
    Ok(())
}

#[tokio::test]
async fn test_history_ordering_most_recent_first() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    for (id, title) in [("1", "First"), ("2", "Second"), ("3", "Third")] {
        let track = create_test_track(id, title, "Artist", ServiceType::Tidal);
        storage.record_play(&track).await?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let history = storage.get_history(10).await?;
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].track_id, "3");
    assert_eq!(history[1].track_id, "2");
    assert_eq!(history[2].track_id, "1");
    Ok(())
}

#[tokio::test]
async fn test_history_dedup_within_10_seconds() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    let track = create_test_track("12345", "Same Song", "Artist", ServiceType::Tidal);

    storage.record_play(&track).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    storage.record_play(&track).await?;

    let history = storage.get_history(10).await?;
    assert_eq!(history.len(), 1, "duplicate within 10s should be deduped");
    Ok(())
}

#[tokio::test]
async fn test_history_with_different_services() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    let tracks = [
        create_test_track("1", "Tidal Song", "A", ServiceType::Tidal),
        create_test_track("2", "YouTube Song", "A", ServiceType::YouTube),
        create_test_track("3", "Bandcamp Song", "A", ServiceType::Bandcamp),
    ];

    for track in &tracks {
        storage.record_play(track).await?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let history = storage.get_history(10).await?;
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].service, ServiceType::Bandcamp);
    assert_eq!(history[1].service, ServiceType::YouTube);
    assert_eq!(history[2].service, ServiceType::Tidal);
    Ok(())
}

#[tokio::test]
async fn test_history_limit() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    for i in 1..=20 {
        let track = create_test_track(
            &i.to_string(),
            &format!("Song {}", i),
            "Artist",
            ServiceType::Tidal,
        );
        storage.record_play(&track).await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let history = storage.get_history(5).await?;
    assert_eq!(history.len(), 5);
    assert_eq!(history[0].title, "Song 20");
    Ok(())
}

#[tokio::test]
async fn test_unicode_in_track_metadata() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    let track = Track {
        id: "unicode-1".to_string(),
        title: "日本語タイトル".to_string(),
        artist: "アーティスト名".to_string(),
        album: "Альбом на русском".to_string(),
        duration_seconds: 200,
        cover_art: CoverArt::None,
        service: ServiceType::Tidal,
    };

    storage.record_play(&track).await?;

    let history = storage.get_history(10).await?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].title, "日本語タイトル");
    assert_eq!(history[0].artist, "アーティスト名");
    assert_eq!(history[0].album, "Альбом на русском");
    Ok(())
}

#[tokio::test]
async fn test_empty_history() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    let history = storage.get_history(10).await?;
    assert!(history.is_empty());
    Ok(())
}

// ── Search cache ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_cache_search_and_get_roundtrip() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    let results = create_search_results(5, 3, 2);
    storage.cache_search("test query", None, &results).await?;

    let cached = storage.get_cached_search("test query", None).await?;
    assert!(cached.is_some());

    let cached = cached.unwrap();
    assert_eq!(cached.tracks.len(), 5);
    assert_eq!(cached.albums.len(), 3);
    assert_eq!(cached.artists.len(), 2);
    Ok(())
}

#[tokio::test]
async fn test_search_cache_miss_returns_none() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    let cached = storage.get_cached_search("unknown", None).await?;
    assert!(cached.is_none());
    Ok(())
}

#[tokio::test]
async fn test_search_cache_with_service_filter() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    let results = create_search_results(3, 0, 0);

    storage
        .cache_search("filtered", Some(ServiceType::Tidal), &results)
        .await?;

    // Same filter → hit
    let cached = storage
        .get_cached_search("filtered", Some(ServiceType::Tidal))
        .await?;
    assert!(cached.is_some());

    // Different filter → miss
    let missed = storage
        .get_cached_search("filtered", Some(ServiceType::YouTube))
        .await?;
    assert!(missed.is_none());

    // No filter → miss
    let missed = storage.get_cached_search("filtered", None).await?;
    assert!(missed.is_none());
    Ok(())
}

#[tokio::test]
async fn test_search_cache_expired_returns_none() -> Result<()> {
    let storage = LocalStorage::new_for_test(1)?; // 1-second TTL

    let results = create_search_results(2, 0, 0);
    storage.cache_search("expiring", None, &results).await?;

    // Fresh → hit
    let cached = storage.get_cached_search("expiring", None).await?;
    assert!(cached.is_some());

    // Wait for expiry
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Expired → miss
    let expired = storage.get_cached_search("expiring", None).await?;
    assert!(expired.is_none());
    Ok(())
}

#[tokio::test]
async fn test_search_cache_case_insensitive() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    let results = create_search_results(3, 0, 0);

    storage.cache_search("Hello World", None, &results).await?;

    assert!(storage.get_cached_search("hello world", None).await?.is_some());
    assert!(storage.get_cached_search("HELLO WORLD", None).await?.is_some());
    assert!(storage.get_cached_search("HeLLo WoRLd", None).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn test_search_cache_whitespace_normalization() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    let results = create_search_results(2, 0, 0);

    storage.cache_search("test query", None, &results).await?;

    assert!(storage
        .get_cached_search("  test query  ", None)
        .await?
        .is_some());
    assert!(storage
        .get_cached_search("\ttest query\n", None)
        .await?
        .is_some());
    Ok(())
}

// ── Combined operations ──────────────────────────────────────────────

#[tokio::test]
async fn test_multiple_sequential_operations() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    // Record plays
    for i in 1..=5 {
        let track = create_test_track(
            &i.to_string(),
            &format!("Song {}", i),
            "Artist",
            ServiceType::Tidal,
        );
        storage.record_play(&track).await?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Cache searches
    storage
        .cache_search("q1", None, &create_search_results(3, 0, 0))
        .await?;
    storage
        .cache_search("q2", Some(ServiceType::Tidal), &create_search_results(5, 2, 1))
        .await?;

    // Verify
    let history = storage.get_history(10).await?;
    assert_eq!(history.len(), 5);

    assert!(storage.get_cached_search("q1", None).await?.is_some());
    assert!(storage
        .get_cached_search("q2", Some(ServiceType::Tidal))
        .await?
        .is_some());
    Ok(())
}

// ── poll_changes (local always returns empty) ────────────────────────

#[tokio::test]
async fn test_poll_changes_returns_empty() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    let changes = storage.poll_changes().await?;
    assert!(changes.is_empty());
    Ok(())
}

// ── Playlist storage ─────────────────────────────────────────────────

use drift::storage::{PlaylistVisibility, SyncedPlaylist, SyncedTrackRef, PlaylistIndexEntry};

fn create_test_playlist(id: &str, title: &str) -> SyncedPlaylist {
    SyncedPlaylist {
        id: id.to_string(),
        title: title.to_string(),
        description: None,
        tracks: Vec::new(),
        created_at_ms: 1000,
        updated_at_ms: 1000,
        lamport_clock: 1,
        device_id: "test-device".to_string(),
        visibility: PlaylistVisibility::Shared,
    }
}

fn create_test_track_ref(id: &str, service: &str) -> SyncedTrackRef {
    SyncedTrackRef {
        id: id.to_string(),
        service: service.to_string(),
        title: format!("Track {}", id),
        artist: "Artist".to_string(),
        album: "Album".to_string(),
        duration_seconds: 200,
        cover_art_url: None,
        added_at_ms: 1000,
        added_by: "test-device".to_string(),
    }
}

#[tokio::test]
async fn test_save_and_load_playlist() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    let mut playlist = create_test_playlist("pl1", "My Favorites");
    playlist.tracks.push(create_test_track_ref("t1", "tidal"));
    playlist.tracks.push(create_test_track_ref("t2", "youtube"));

    storage.save_playlist(&playlist).await?;

    let loaded = storage.load_playlist("pl1").await?;
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.id, "pl1");
    assert_eq!(loaded.title, "My Favorites");
    assert_eq!(loaded.tracks.len(), 2);
    assert_eq!(loaded.tracks[0].id, "t1");
    assert_eq!(loaded.tracks[1].id, "t2");
    Ok(())
}

#[tokio::test]
async fn test_load_playlist_not_found() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    let loaded = storage.load_playlist("nonexistent").await?;
    assert!(loaded.is_none());
    Ok(())
}

#[tokio::test]
async fn test_list_playlists_empty() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    let playlists = storage.list_playlists().await?;
    assert!(playlists.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_list_playlists_after_save() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    let pl1 = create_test_playlist("pl1", "Favorites");
    let mut pl2 = create_test_playlist("pl2", "Workout");
    pl2.tracks.push(create_test_track_ref("t1", "tidal"));
    pl2.tracks.push(create_test_track_ref("t2", "tidal"));

    storage.save_playlist(&pl1).await?;
    storage.save_playlist(&pl2).await?;

    let playlists = storage.list_playlists().await?;
    assert_eq!(playlists.len(), 2);

    // Find by id (order not guaranteed)
    let pl1_entry = playlists.iter().find(|p| p.id == "pl1").unwrap();
    assert_eq!(pl1_entry.title, "Favorites");
    assert_eq!(pl1_entry.track_count, 0);

    let pl2_entry = playlists.iter().find(|p| p.id == "pl2").unwrap();
    assert_eq!(pl2_entry.title, "Workout");
    assert_eq!(pl2_entry.track_count, 2);
    Ok(())
}

#[tokio::test]
async fn test_update_existing_playlist() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    let mut playlist = create_test_playlist("pl1", "Original");
    storage.save_playlist(&playlist).await?;

    // Update title and add tracks
    playlist.title = "Updated Title".to_string();
    playlist.tracks.push(create_test_track_ref("t1", "tidal"));
    playlist.updated_at_ms = 2000;
    storage.save_playlist(&playlist).await?;

    let loaded = storage.load_playlist("pl1").await?.unwrap();
    assert_eq!(loaded.title, "Updated Title");
    assert_eq!(loaded.tracks.len(), 1);

    // Index should also be updated
    let index = storage.list_playlists().await?;
    assert_eq!(index.len(), 1);
    assert_eq!(index[0].title, "Updated Title");
    assert_eq!(index[0].track_count, 1);
    Ok(())
}

#[tokio::test]
async fn test_delete_playlist() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    storage.save_playlist(&create_test_playlist("pl1", "First")).await?;
    storage.save_playlist(&create_test_playlist("pl2", "Second")).await?;

    assert_eq!(storage.list_playlists().await?.len(), 2);

    storage.delete_playlist("pl1").await?;

    assert_eq!(storage.list_playlists().await?.len(), 1);
    assert!(storage.load_playlist("pl1").await?.is_none());
    assert!(storage.load_playlist("pl2").await?.is_some());
    Ok(())
}

#[tokio::test]
async fn test_delete_nonexistent_playlist() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;
    // Should not error
    storage.delete_playlist("nonexistent").await?;
    Ok(())
}

#[tokio::test]
async fn test_playlist_visibility() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    let mut playlist = create_test_playlist("pl1", "Private Playlist");
    playlist.visibility = PlaylistVisibility::Private;
    storage.save_playlist(&playlist).await?;

    let loaded = storage.load_playlist("pl1").await?.unwrap();
    assert_eq!(loaded.visibility, PlaylistVisibility::Private);

    let index = storage.list_playlists().await?;
    assert_eq!(index[0].visibility, PlaylistVisibility::Private);
    Ok(())
}

#[tokio::test]
async fn test_playlist_with_unicode() -> Result<()> {
    let storage = LocalStorage::new_for_test(3600)?;

    let mut playlist = create_test_playlist("pl-jp", "日本の音楽");
    playlist.description = Some("お気に入りの曲".to_string());
    playlist.tracks.push(SyncedTrackRef {
        id: "t1".to_string(),
        service: "tidal".to_string(),
        title: "桜".to_string(),
        artist: "アーティスト".to_string(),
        album: "アルバム".to_string(),
        duration_seconds: 300,
        cover_art_url: None,
        added_at_ms: 1000,
        added_by: "test".to_string(),
    });
    storage.save_playlist(&playlist).await?;

    let loaded = storage.load_playlist("pl-jp").await?.unwrap();
    assert_eq!(loaded.title, "日本の音楽");
    assert_eq!(loaded.description, Some("お気に入りの曲".to_string()));
    assert_eq!(loaded.tracks[0].title, "桜");
    Ok(())
}
