//! Integration tests for LocalStorage through the DriftStorage trait.
//!
//! These tests use REAL storage (temp redb files, real directories) to verify
//! the actual I/O path works correctly.

use anyhow::Result;
use chrono::Utc;
use drift::history_db::HistoryEntry;
use drift::queue_persistence::PersistedQueue;
use drift::search::SearchHistory;
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

fn create_search_results(num_tracks: usize, num_albums: usize, num_artists: usize) -> SearchResults {
    let tracks = (0..num_tracks)
        .map(|i| create_test_track(&format!("track-{}", i), &format!("Track {}", i), "Artist", ServiceType::Tidal))
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

    SearchResults { tracks, albums, artists }
}

#[tokio::test]
async fn test_backend_name() -> Result<()> {
    let storage = LocalStorage::new(3600)?;
    assert_eq!(storage.backend_name(), "local");
    Ok(())
}

#[tokio::test]
async fn test_record_play_and_get_history_roundtrip() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    let track = create_test_track("12345", "Test Song", "Test Artist", ServiceType::Tidal);

    // Record play
    storage.record_play(&track).await?;

    // Get history
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
    let storage = LocalStorage::new(3600)?;

    // Record three plays with small delays to ensure different timestamps
    let track1 = create_test_track("1", "First Song", "Artist", ServiceType::Tidal);
    storage.record_play(&track1).await?;

    tokio::time::sleep(Duration::from_millis(50)).await;

    let track2 = create_test_track("2", "Second Song", "Artist", ServiceType::Tidal);
    storage.record_play(&track2).await?;

    tokio::time::sleep(Duration::from_millis(50)).await;

    let track3 = create_test_track("3", "Third Song", "Artist", ServiceType::Tidal);
    storage.record_play(&track3).await?;

    // Get history
    let history = storage.get_history(10).await?;

    assert_eq!(history.len(), 3);
    // Most recent first
    assert_eq!(history[0].track_id, "3");
    assert_eq!(history[1].track_id, "2");
    assert_eq!(history[2].track_id, "1");

    Ok(())
}

#[tokio::test]
async fn test_history_dedup_within_10_seconds() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    let track = create_test_track("12345", "Same Song", "Artist", ServiceType::Tidal);

    // Record same track twice within 10 seconds
    storage.record_play(&track).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    storage.record_play(&track).await?;

    // Should only have one entry
    let history = storage.get_history(10).await?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].track_id, "12345");

    Ok(())
}

#[tokio::test]
async fn test_history_allows_duplicate_after_dedup_window() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    let track = create_test_track("12345", "Same Song", "Artist", ServiceType::Tidal);

    // Record, then wait more than 10 seconds, then record again
    storage.record_play(&track).await?;

    // Wait 11 seconds (beyond dedup window)
    tokio::time::sleep(Duration::from_secs(11)).await;

    storage.record_play(&track).await?;

    // Should have two entries
    let history = storage.get_history(10).await?;
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].track_id, "12345");
    assert_eq!(history[1].track_id, "12345");

    Ok(())
}

#[tokio::test]
async fn test_save_and_load_queue_roundtrip() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    let track1 = create_test_track("1", "Queue Track 1", "Artist", ServiceType::Tidal);
    let track2 = create_test_track("2", "Queue Track 2", "Artist", ServiceType::YouTube);

    let mut queue = PersistedQueue::new();
    queue.tracks.push(track1.into());
    queue.tracks.push(track2.into());
    queue.current_position = Some(1);
    queue.elapsed_seconds = Some(45);

    // Save queue
    storage.save_queue(&queue).await?;

    // Load queue
    let loaded = storage.load_queue().await?;
    assert!(loaded.is_some());

    let loaded_queue = loaded.unwrap();
    assert_eq!(loaded_queue.tracks.len(), 2);
    assert_eq!(loaded_queue.tracks[0].title, "Queue Track 1");
    assert_eq!(loaded_queue.tracks[1].title, "Queue Track 2");
    assert_eq!(loaded_queue.current_position, Some(1));
    assert_eq!(loaded_queue.elapsed_seconds, Some(45));

    Ok(())
}

#[tokio::test]
async fn test_load_queue_returns_none_when_empty() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    // Without saving anything, load should return None
    let loaded = storage.load_queue().await?;
    assert!(loaded.is_none());

    Ok(())
}

#[tokio::test]
async fn test_cache_search_and_get_cached_search_roundtrip() -> Result<()> {
    let storage = LocalStorage::new(3600)?; // 1 hour TTL

    let query = "test query";
    let results = create_search_results(5, 3, 2);

    // Cache search
    storage.cache_search(query, None, &results).await?;

    // Get cached search
    let cached = storage.get_cached_search(query, None).await?;
    assert!(cached.is_some());

    let cached_results = cached.unwrap();
    assert_eq!(cached_results.tracks.len(), 5);
    assert_eq!(cached_results.albums.len(), 3);
    assert_eq!(cached_results.artists.len(), 2);

    Ok(())
}

#[tokio::test]
async fn test_search_cache_miss_returns_none() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    // Query that was never cached
    let cached = storage.get_cached_search("unknown query", None).await?;
    assert!(cached.is_none());

    Ok(())
}

#[tokio::test]
async fn test_search_cache_with_service_filter() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    let query = "filtered query";
    let results = create_search_results(3, 0, 0);

    // Cache with filter
    storage.cache_search(query, Some(ServiceType::Tidal), &results).await?;

    // Should get results with same filter
    let cached = storage.get_cached_search(query, Some(ServiceType::Tidal)).await?;
    assert!(cached.is_some());

    // Different filter should miss
    let cached_different = storage.get_cached_search(query, Some(ServiceType::YouTube)).await?;
    assert!(cached_different.is_none());

    // No filter should also miss
    let cached_no_filter = storage.get_cached_search(query, None).await?;
    assert!(cached_no_filter.is_none());

    Ok(())
}

#[tokio::test]
async fn test_search_cache_expired_returns_none() -> Result<()> {
    let storage = LocalStorage::new(1)?; // 1 second TTL

    let query = "expiring query";
    let results = create_search_results(2, 0, 0);

    storage.cache_search(query, None, &results).await?;

    // Should get results immediately
    let cached = storage.get_cached_search(query, None).await?;
    assert!(cached.is_some());

    // Wait for cache to expire
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Should now return None
    let cached_expired = storage.get_cached_search(query, None).await?;
    assert!(cached_expired.is_none());

    Ok(())
}

#[tokio::test]
async fn test_save_and_load_search_history() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    let mut history = SearchHistory::new(10);
    history.add("first query", 5);
    history.add("second query", 10);
    history.add("third query", 3);

    // Save search history
    storage.save_search_history(&history).await?;

    // Load search history
    let loaded = storage.load_search_history(10).await?;

    assert_eq!(loaded.entries.len(), 3);
    assert_eq!(loaded.entries[0].query, "third query"); // Most recent first
    assert_eq!(loaded.entries[1].query, "second query");
    assert_eq!(loaded.entries[2].query, "first query");

    Ok(())
}

#[tokio::test]
async fn test_load_search_history_returns_empty_when_none() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    let loaded = storage.load_search_history(10).await?;
    assert_eq!(loaded.entries.len(), 0);

    Ok(())
}

#[tokio::test]
async fn test_multiple_sequential_operations() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    // Record multiple plays
    for i in 1..=5 {
        let track = create_test_track(&i.to_string(), &format!("Song {}", i), "Artist", ServiceType::Tidal);
        storage.record_play(&track).await?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Save a queue
    let track1 = create_test_track("100", "Queue Track", "Artist", ServiceType::Tidal);
    let queue = PersistedQueue::from_tracks(&[track1], Some(0), None);
    storage.save_queue(&queue).await?;

    // Cache some searches
    storage.cache_search("query 1", None, &create_search_results(3, 0, 0)).await?;
    storage.cache_search("query 2", Some(ServiceType::Tidal), &create_search_results(5, 2, 1)).await?;

    // Save search history
    let mut search_history = SearchHistory::new(10);
    search_history.add("query 1", 3);
    search_history.add("query 2", 8);
    storage.save_search_history(&search_history).await?;

    // Verify all operations
    let history = storage.get_history(10).await?;
    assert_eq!(history.len(), 5);

    let loaded_queue = storage.load_queue().await?;
    assert!(loaded_queue.is_some());

    let cached1 = storage.get_cached_search("query 1", None).await?;
    assert!(cached1.is_some());

    let cached2 = storage.get_cached_search("query 2", Some(ServiceType::Tidal)).await?;
    assert!(cached2.is_some());

    let loaded_search_history = storage.load_search_history(10).await?;
    assert_eq!(loaded_search_history.entries.len(), 2);

    Ok(())
}

#[tokio::test]
async fn test_history_with_different_services() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    let tidal_track = create_test_track("1", "Tidal Song", "Artist", ServiceType::Tidal);
    let youtube_track = create_test_track("2", "YouTube Song", "Artist", ServiceType::YouTube);
    let bandcamp_track = create_test_track("3", "Bandcamp Song", "Artist", ServiceType::Bandcamp);

    storage.record_play(&tidal_track).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    storage.record_play(&youtube_track).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    storage.record_play(&bandcamp_track).await?;

    let history = storage.get_history(10).await?;
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].service, ServiceType::Bandcamp);
    assert_eq!(history[1].service, ServiceType::YouTube);
    assert_eq!(history[2].service, ServiceType::Tidal);

    Ok(())
}

#[tokio::test]
async fn test_history_limit() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    // Record 20 plays
    for i in 1..=20 {
        let track = create_test_track(&i.to_string(), &format!("Song {}", i), "Artist", ServiceType::Tidal);
        storage.record_play(&track).await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Request only 5
    let history = storage.get_history(5).await?;
    assert_eq!(history.len(), 5);

    // Most recent should be Song 20
    assert_eq!(history[0].title, "Song 20");

    Ok(())
}

#[tokio::test]
async fn test_unicode_in_track_metadata() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

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
async fn test_empty_queue() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    let empty_queue = PersistedQueue::new();
    storage.save_queue(&empty_queue).await?;

    let loaded = storage.load_queue().await?;
    assert!(loaded.is_some());

    let loaded_queue = loaded.unwrap();
    assert_eq!(loaded_queue.tracks.len(), 0);
    assert!(loaded_queue.current_position.is_none());
    assert!(loaded_queue.elapsed_seconds.is_none());

    Ok(())
}

#[tokio::test]
async fn test_search_cache_case_insensitive() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    let results = create_search_results(3, 0, 0);
    storage.cache_search("Hello World", None, &results).await?;

    // Should get same results with different case
    let cached1 = storage.get_cached_search("hello world", None).await?;
    let cached2 = storage.get_cached_search("HELLO WORLD", None).await?;
    let cached3 = storage.get_cached_search("HeLLo WoRLd", None).await?;

    assert!(cached1.is_some());
    assert!(cached2.is_some());
    assert!(cached3.is_some());

    Ok(())
}

#[tokio::test]
async fn test_search_cache_whitespace_normalization() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    let results = create_search_results(2, 0, 0);
    storage.cache_search("test query", None, &results).await?;

    // Should match with extra whitespace
    let cached1 = storage.get_cached_search("  test query  ", None).await?;
    let cached2 = storage.get_cached_search("\ttest query\n", None).await?;

    assert!(cached1.is_some());
    assert!(cached2.is_some());

    Ok(())
}

#[tokio::test]
async fn test_overwrite_queue() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    // Save initial queue
    let track1 = create_test_track("1", "First Track", "Artist", ServiceType::Tidal);
    let queue1 = PersistedQueue::from_tracks(&[track1], Some(0), None);
    storage.save_queue(&queue1).await?;

    // Overwrite with new queue
    let track2 = create_test_track("2", "Second Track", "Artist", ServiceType::YouTube);
    let track3 = create_test_track("3", "Third Track", "Artist", ServiceType::Bandcamp);
    let queue2 = PersistedQueue::from_tracks(&[track2, track3], Some(1), Some(30));
    storage.save_queue(&queue2).await?;

    // Should get the latest queue
    let loaded = storage.load_queue().await?;
    assert!(loaded.is_some());

    let loaded_queue = loaded.unwrap();
    assert_eq!(loaded_queue.tracks.len(), 2);
    assert_eq!(loaded_queue.tracks[0].title, "Second Track");
    assert_eq!(loaded_queue.tracks[1].title, "Third Track");
    assert_eq!(loaded_queue.current_position, Some(1));
    assert_eq!(loaded_queue.elapsed_seconds, Some(30));

    Ok(())
}

#[tokio::test]
async fn test_poll_changes_returns_empty() -> Result<()> {
    let storage = LocalStorage::new(3600)?;

    // LocalStorage should always return empty changes
    let changes = storage.poll_changes().await?;
    assert!(changes.is_empty());

    Ok(())
}
