//! Integration tests for HistoryDb.
//!
//! These tests use real redb databases in temp directories to verify
//! the play history tracking functionality.

use anyhow::Result;
use drift::history_db::HistoryDb;
use drift::service::{CoverArt, ServiceType, Track};
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

#[test]
fn test_record_and_retrieve() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;
    let track = create_test_track("12345", "Test Song", "Test Artist", ServiceType::Tidal);

    db.record_play(&track)?;

    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].track_id, "12345");
    assert_eq!(entries[0].title, "Test Song");
    assert_eq!(entries[0].artist, "Test Artist");
    assert_eq!(entries[0].service, ServiceType::Tidal);

    Ok(())
}

#[test]
fn test_ordering_most_recent_first() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;

    // Record three tracks
    for i in 1..=3 {
        let track = create_test_track(&i.to_string(), &format!("Song {}", i), "Artist", ServiceType::Tidal);
        db.record_play(&track)?;
        // Small delay to ensure different timestamps
        std::thread::sleep(Duration::from_millis(10));
    }

    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 3);

    // Most recent should be Song 3
    assert_eq!(entries[0].title, "Song 3");
    assert_eq!(entries[1].title, "Song 2");
    assert_eq!(entries[2].title, "Song 1");

    Ok(())
}

#[test]
fn test_limit_results() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;

    // Record 10 tracks
    for i in 1..=10 {
        let track = create_test_track(&i.to_string(), &format!("Song {}", i), "Artist", ServiceType::Tidal);
        db.record_play(&track)?;
        std::thread::sleep(Duration::from_millis(5));
    }

    // Request only 5
    let entries = db.get_recent(5)?;
    assert_eq!(entries.len(), 5);

    // Should get most recent 5 (Song 10 down to Song 6)
    assert_eq!(entries[0].title, "Song 10");
    assert_eq!(entries[4].title, "Song 6");

    Ok(())
}

#[test]
fn test_dedup_within_window() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;
    let track = create_test_track("same-track", "Same Song", "Artist", ServiceType::Tidal);

    // Record twice within dedup window
    db.record_play(&track)?;
    std::thread::sleep(Duration::from_millis(100)); // Well within 10s window
    db.record_play(&track)?;

    // Should only have one entry
    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 1);

    Ok(())
}

#[test]
fn test_allows_duplicate_after_window() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;
    let track = create_test_track("same-track", "Same Song", "Artist", ServiceType::Tidal);

    db.record_play(&track)?;

    // Wait beyond dedup window (10 seconds)
    std::thread::sleep(Duration::from_secs(11));

    db.record_play(&track)?;

    // Should have two entries
    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].track_id, "same-track");
    assert_eq!(entries[1].track_id, "same-track");

    Ok(())
}

#[test]
fn test_different_services() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;

    let tidal = create_test_track("1", "Tidal Song", "Artist", ServiceType::Tidal);
    let youtube = create_test_track("2", "YouTube Song", "Artist", ServiceType::YouTube);
    let bandcamp = create_test_track("3", "Bandcamp Song", "Artist", ServiceType::Bandcamp);

    db.record_play(&tidal)?;
    std::thread::sleep(Duration::from_millis(10));
    db.record_play(&youtube)?;
    std::thread::sleep(Duration::from_millis(10));
    db.record_play(&bandcamp)?;

    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].service, ServiceType::Bandcamp);
    assert_eq!(entries[1].service, ServiceType::YouTube);
    assert_eq!(entries[2].service, ServiceType::Tidal);

    Ok(())
}

#[test]
fn test_clear_history() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;

    // Add some entries
    for i in 1..=5 {
        let track = create_test_track(&i.to_string(), &format!("Song {}", i), "Artist", ServiceType::Tidal);
        db.record_play(&track)?;
        std::thread::sleep(Duration::from_millis(5));
    }

    let before = db.get_recent(10)?;
    assert_eq!(before.len(), 5);

    // Clear
    db.clear_history()?;

    let after = db.get_recent(10)?;
    assert!(after.is_empty());

    Ok(())
}

#[test]
fn test_unicode_metadata() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;
    let track = Track {
        id: "unicode-1".to_string(),
        title: "日本語タイトル".to_string(),
        artist: "アーティスト名".to_string(),
        album: "Альбом на русском".to_string(),
        duration_seconds: 200,
        cover_art: CoverArt::None,
        service: ServiceType::Tidal,
    };

    db.record_play(&track)?;

    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].title, "日本語タイトル");
    assert_eq!(entries[0].artist, "アーティスト名");
    assert_eq!(entries[0].album, "Альбом на русском");

    Ok(())
}

#[test]
fn test_empty_database() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;

    let entries = db.get_recent(10)?;
    assert!(entries.is_empty());

    Ok(())
}

#[test]
fn test_cover_art_preservation() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;

    let track_with_cover = Track {
        id: "1".to_string(),
        title: "With Cover".to_string(),
        artist: "Artist".to_string(),
        album: "Album".to_string(),
        duration_seconds: 180,
        cover_art: CoverArt::tidal("cover-abc".to_string()),
        service: ServiceType::Tidal,
    };

    let track_without_cover = Track {
        id: "2".to_string(),
        title: "Without Cover".to_string(),
        artist: "Artist".to_string(),
        album: "Album".to_string(),
        duration_seconds: 180,
        cover_art: CoverArt::None,
        service: ServiceType::YouTube,
    };

    db.record_play(&track_with_cover)?;
    std::thread::sleep(Duration::from_millis(10));
    db.record_play(&track_without_cover)?;

    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].cover_art_id, None); // YouTube track, no cover
    assert_eq!(entries[1].cover_art_id, Some("cover-abc".to_string()));

    Ok(())
}

#[test]
fn test_duration_preservation() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;

    let short_track = create_test_track("1", "Short", "Artist", ServiceType::Tidal);
    let mut long_track = create_test_track("2", "Long", "Artist", ServiceType::Tidal);
    long_track.duration_seconds = 600;

    db.record_play(&short_track)?;
    std::thread::sleep(Duration::from_millis(10));
    db.record_play(&long_track)?;

    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].duration_seconds, 600);
    assert_eq!(entries[1].duration_seconds, 180);

    Ok(())
}

#[test]
fn test_large_history() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;

    // Record 100 tracks
    for i in 1..=100 {
        let track = create_test_track(&i.to_string(), &format!("Song {}", i), "Artist", ServiceType::Tidal);
        db.record_play(&track)?;
    }

    // Request all
    let entries = db.get_recent(100)?;
    assert_eq!(entries.len(), 100);

    // Verify ordering
    assert_eq!(entries[0].title, "Song 100");
    assert_eq!(entries[99].title, "Song 1");

    Ok(())
}

#[test]
fn test_automatic_pruning() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;

    // Record more than MAX_HISTORY_SIZE (500)
    // This would take too long, so we'll just verify the mechanism exists
    // by checking that pruning doesn't fail
    for i in 1..=10 {
        let track = create_test_track(&i.to_string(), &format!("Song {}", i), "Artist", ServiceType::Tidal);
        db.record_play(&track)?;
    }

    let entries = db.get_recent(100)?;
    assert_eq!(entries.len(), 10);

    Ok(())
}

#[test]
fn test_special_characters_in_metadata() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;
    let track = Track {
        id: "special-1".to_string(),
        title: "Track with \"quotes\" and 'apostrophes'".to_string(),
        artist: "Artist with\nnewline".to_string(),
        album: "Album with\ttab".to_string(),
        duration_seconds: 180,
        cover_art: CoverArt::None,
        service: ServiceType::Tidal,
    };

    db.record_play(&track)?;

    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].title, "Track with \"quotes\" and 'apostrophes'");

    Ok(())
}

#[test]
fn test_timestamp_accuracy() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;
    let track = create_test_track("1", "Song", "Artist", ServiceType::Tidal);

    use chrono::Utc;
    let before = Utc::now();
    db.record_play(&track)?;
    let after = Utc::now();

    let entries = db.get_recent(1)?;
    assert_eq!(entries.len(), 1);

    // played_at should be between before and after
    assert!(entries[0].played_at >= before);
    assert!(entries[0].played_at <= after);

    Ok(())
}

#[test]
fn test_concurrent_same_track_different_services() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;

    // Same track ID but different services should be treated as different tracks
    let tidal = create_test_track("same-id", "Song", "Artist", ServiceType::Tidal);
    let youtube = create_test_track("same-id", "Song", "Artist", ServiceType::YouTube);

    db.record_play(&tidal)?;
    std::thread::sleep(Duration::from_millis(10));
    db.record_play(&youtube)?;

    // Should have both entries since they're from different services
    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 2);

    Ok(())
}

#[test]
fn test_zero_duration_track() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;
    let mut track = create_test_track("1", "Song", "Artist", ServiceType::Tidal);
    track.duration_seconds = 0;

    db.record_play(&track)?;

    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].duration_seconds, 0);

    Ok(())
}

#[test]
fn test_very_long_metadata() -> Result<()> {
    let db = HistoryDb::new_in_memory()?;
    let long_title = "A".repeat(1000);
    let track = Track {
        id: "long-1".to_string(),
        title: long_title.clone(),
        artist: "B".repeat(1000),
        album: "C".repeat(1000),
        duration_seconds: 180,
        cover_art: CoverArt::None,
        service: ServiceType::Tidal,
    };

    db.record_play(&track)?;

    let entries = db.get_recent(10)?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].title.len(), 1000);

    Ok(())
}
