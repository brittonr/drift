use super::*;

const PLAYED_AT_MS: u64 = 1_700_000_000_000;

fn record() -> drift_plugin::HistoryRecord {
    drift_plugin::HistoryRecord {
        track_id: "song".into(),
        title: "Song".into(),
        artist: "Artist".into(),
        album: "Album".into(),
        duration_seconds: 0,
        cover_art_id: None,
        service: "tidal".into(),
        played_at_ms: PLAYED_AT_MS,
    }
}

#[test]
fn imported_history_keeps_time_and_deduplicates_replay() {
    let db = HistoryDb::new_in_memory().unwrap();
    db.import_records(&[record()]).unwrap();
    db.import_records(&[record()]).unwrap();
    let entries = db.get_recent(1).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].played_at.timestamp_millis() as u64, PLAYED_AT_MS);
}

#[test]
fn invalid_batch_does_not_publish_its_valid_prefix() {
    let db = HistoryDb::new_in_memory().unwrap();
    let mut invalid = record();
    invalid.service = "unknown".into();
    assert!(db.import_records(&[record(), invalid]).is_err());
    assert!(db.get_recent(1).unwrap().is_empty());
    let mut invalid = record();
    invalid.played_at_ms = u64::MAX;
    assert!(db.import_records(&[invalid]).is_err());
    assert!(db.get_recent(1).unwrap().is_empty());
}
