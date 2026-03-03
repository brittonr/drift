//! History deduplication.
//!
//! When a user plays a track, the client writes a history entry keyed by
//! timestamp. Network retries or UI double-taps can produce duplicate
//! entries for the same track within a short window.
//!
//! This module detects those duplicates so the caller (a cluster plugin
//! or client-side code) can delete the older copies.

use crate::HistoryRecord;

/// Find duplicate history entries within a time window.
///
/// Given a newly written entry and a list of recent entries, returns the
/// keys of older entries that are duplicates (same `track_id`, within
/// `dedup_window_ms` of the new entry).
///
/// The new entry itself is never returned for deletion.
///
/// # Arguments
///
/// * `new_key` — KV key of the newly written entry
/// * `new_entry` — The newly written history record
/// * `recent_entries` — Recent entries from KV scan: `(key, record)` pairs
/// * `dedup_window_ms` — Maximum time delta to consider a duplicate (default: 10_000)
///
/// # Examples
///
/// ```
/// use drift_plugin::{HistoryRecord, dedup};
///
/// let new = HistoryRecord {
///     track_id: "42".into(), title: "Song".into(), artist: "A".into(),
///     album: "B".into(), duration_seconds: 200, cover_art_id: None,
///     service: "tidal".into(), played_at_ms: 1700000010000,
/// };
/// let old = HistoryRecord {
///     track_id: "42".into(), title: "Song".into(), artist: "A".into(),
///     album: "B".into(), duration_seconds: 200, cover_art_id: None,
///     service: "tidal".into(), played_at_ms: 1700000005000,
/// };
///
/// let to_delete = dedup::find_duplicates(
///     "drift:u:history:01700000010000",
///     &new,
///     &[("drift:u:history:01700000005000".into(), old)],
///     10_000,
/// );
/// assert_eq!(to_delete, vec!["drift:u:history:01700000005000"]);
/// ```
pub fn find_duplicates(
    new_key: &str,
    new_entry: &HistoryRecord,
    recent_entries: &[(String, HistoryRecord)],
    dedup_window_ms: u64,
) -> Vec<String> {
    let mut to_delete = Vec::new();

    for (key, entry) in recent_entries {
        // Skip the new entry itself
        if key == new_key {
            continue;
        }

        // Same track?
        if entry.track_id != new_entry.track_id {
            continue;
        }

        // Within dedup window?
        let delta = if new_entry.played_at_ms >= entry.played_at_ms {
            new_entry.played_at_ms - entry.played_at_ms
        } else {
            entry.played_at_ms - new_entry.played_at_ms
        };

        if delta <= dedup_window_ms {
            to_delete.push(key.clone());
        }
    }

    to_delete
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_DEDUP_WINDOW_MS;

    fn record(track_id: &str, played_at_ms: u64) -> HistoryRecord {
        HistoryRecord {
            track_id: track_id.into(),
            title: "Title".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_seconds: 200,
            cover_art_id: None,
            service: "tidal".into(),
            played_at_ms,
        }
    }

    #[test]
    fn no_duplicates_different_tracks() {
        let new = record("track-1", 1000);
        let recent = vec![("k1".into(), record("track-2", 999))];
        let result = find_duplicates("k-new", &new, &recent, DEFAULT_DEDUP_WINDOW_MS);
        assert!(result.is_empty());
    }

    #[test]
    fn exact_duplicate() {
        let new = record("track-1", 1000);
        let recent = vec![("k-old".into(), record("track-1", 995))];
        let result = find_duplicates("k-new", &new, &recent, DEFAULT_DEDUP_WINDOW_MS);
        assert_eq!(result, vec!["k-old"]);
    }

    #[test]
    fn outside_dedup_window() {
        let new = record("track-1", 20_000);
        let recent = vec![("k-old".into(), record("track-1", 1_000))];
        let result = find_duplicates("k-new", &new, &recent, DEFAULT_DEDUP_WINDOW_MS);
        assert!(result.is_empty()); // 19s > 10s window
    }

    #[test]
    fn at_exact_boundary() {
        let new = record("track-1", 10_000);
        let recent = vec![("k-old".into(), record("track-1", 0))];
        let result = find_duplicates("k-new", &new, &recent, 10_000);
        assert_eq!(result, vec!["k-old"]); // exactly at boundary = still dedup
    }

    #[test]
    fn one_past_boundary() {
        let new = record("track-1", 10_001);
        let recent = vec![("k-old".into(), record("track-1", 0))];
        let result = find_duplicates("k-new", &new, &recent, 10_000);
        assert!(result.is_empty()); // 10,001ms > 10,000ms window
    }

    #[test]
    fn skips_self() {
        let new = record("track-1", 1000);
        let recent = vec![("k-new".into(), record("track-1", 1000))];
        let result = find_duplicates("k-new", &new, &recent, DEFAULT_DEDUP_WINDOW_MS);
        assert!(result.is_empty()); // don't delete ourselves
    }

    #[test]
    fn multiple_duplicates() {
        let new = record("track-1", 5000);
        let recent = vec![
            ("k1".into(), record("track-1", 4000)),  // 1s ago — dedup
            ("k2".into(), record("track-2", 4500)),  // different track — skip
            ("k3".into(), record("track-1", 4900)),  // 100ms ago — dedup
        ];
        let result = find_duplicates("k-new", &new, &recent, DEFAULT_DEDUP_WINDOW_MS);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"k1".to_string()));
        assert!(result.contains(&"k3".to_string()));
    }

    #[test]
    fn empty_recent_entries() {
        let new = record("track-1", 1000);
        let result = find_duplicates("k-new", &new, &[], DEFAULT_DEDUP_WINDOW_MS);
        assert!(result.is_empty());
    }

    #[test]
    fn reverse_time_order() {
        // New entry is older than recent (clock skew or out-of-order writes)
        let new = record("track-1", 1000);
        let recent = vec![("k-future".into(), record("track-1", 1005))];
        let result = find_duplicates("k-new", &new, &recent, DEFAULT_DEDUP_WINDOW_MS);
        assert_eq!(result, vec!["k-future"]); // absolute delta is 5ms
    }
}
