//! CRDT-style merge functions for local-first sync.
//!
//! These are pure functions that determine how to reconcile local and remote
//! state. They never touch storage directly — the caller applies the result.

use crate::history_db::HistoryEntry;
use crate::queue_persistence::PersistedQueue;

/// Result of merging two queue states.
#[derive(Debug)]
pub enum QueueMergeResult {
    /// Remote queue is newer — accept it.
    AcceptRemote(PersistedQueue),
    /// Local queue is newer or identical — keep it.
    KeepLocal,
}

/// Merge two queue states using Lamport clock + wall-clock tiebreaker.
///
/// Rules:
/// 1. Higher Lamport clock wins.
/// 2. Same clock but different device: wall-clock tiebreaker.
/// 3. Same device: this is our own echo, ignore.
/// 4. Remote clock = 0 means pre-CRDT queue — accept if local is also 0.
pub fn merge_queue(
    local: &PersistedQueue,
    remote: &PersistedQueue,
    local_device_id: &str,
) -> QueueMergeResult {
    // Skip our own echoes
    if !remote.device_id.is_empty() && remote.device_id == local_device_id {
        return QueueMergeResult::KeepLocal;
    }

    // Pre-CRDT queues (lamport_clock = 0): accept remote if it has tracks
    // and local doesn't, otherwise keep local.
    if local.lamport_clock == 0 && remote.lamport_clock == 0 {
        if local.tracks.is_empty() && !remote.tracks.is_empty() {
            return QueueMergeResult::AcceptRemote(remote.clone());
        }
        return QueueMergeResult::KeepLocal;
    }

    if remote.lamport_clock > local.lamport_clock {
        return QueueMergeResult::AcceptRemote(remote.clone());
    }

    if remote.lamport_clock == local.lamport_clock {
        // Concurrent edit — wall-clock tiebreaker
        if remote.updated_at_ms > local.updated_at_ms {
            return QueueMergeResult::AcceptRemote(remote.clone());
        }
    }

    QueueMergeResult::KeepLocal
}

/// Merge remote history entries into local, returning only genuinely new entries.
///
/// Uses set-union semantics: entries are unique by (track_id, played_at timestamp).
/// This is naturally conflict-free since two devices playing different tracks
/// at different times produce non-overlapping entries.
pub fn merge_history(
    local: &[HistoryEntry],
    remote: &[HistoryEntry],
) -> Vec<HistoryEntry> {
    use std::collections::HashSet;

    // Build a set of (track_id, played_at_ms) from local entries
    let local_keys: HashSet<(String, i64)> = local
        .iter()
        .map(|e| (e.track_id.clone(), e.played_at.timestamp_millis()))
        .collect();

    // Return remote entries not present locally
    remote
        .iter()
        .filter(|e| {
            let key = (e.track_id.clone(), e.played_at.timestamp_millis());
            !local_keys.contains(&key)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue_persistence::PersistedTrack;
    use crate::service::ServiceType;
    use chrono::{DateTime, Utc};

    fn make_queue(
        device_id: &str,
        lamport: u64,
        updated_ms: u64,
        track_count: usize,
    ) -> PersistedQueue {
        let tracks: Vec<PersistedTrack> = (0..track_count)
            .map(|i| PersistedTrack {
                id: format!("t{}", i),
                title: format!("Track {}", i),
                artist: "Artist".to_string(),
                album: "Album".to_string(),
                duration_seconds: 180,
                cover_art_id: None,
                service: "tidal".to_string(),
            })
            .collect();
        PersistedQueue {
            version: 3,
            tracks,
            current_position: None,
            elapsed_seconds: None,
            device_id: device_id.to_string(),
            lamport_clock: lamport,
            updated_at_ms: updated_ms,
        }
    }

    fn make_history_entry(track_id: &str, played_at: DateTime<Utc>) -> HistoryEntry {
        HistoryEntry {
            id: 0,
            track_id: track_id.to_string(),
            title: format!("Track {}", track_id),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration_seconds: 180,
            cover_art_id: None,
            service: ServiceType::Tidal,
            played_at,
        }
    }

    #[test]
    fn test_remote_newer_lamport_wins() {
        let local = make_queue("device-a", 5, 1000, 3);
        let remote = make_queue("device-b", 7, 900, 5);
        match merge_queue(&local, &remote, "device-a") {
            QueueMergeResult::AcceptRemote(q) => {
                assert_eq!(q.tracks.len(), 5);
                assert_eq!(q.lamport_clock, 7);
            }
            QueueMergeResult::KeepLocal => panic!("should accept remote"),
        }
    }

    #[test]
    fn test_local_newer_lamport_keeps() {
        let local = make_queue("device-a", 10, 1000, 3);
        let remote = make_queue("device-b", 5, 2000, 5);
        assert!(matches!(
            merge_queue(&local, &remote, "device-a"),
            QueueMergeResult::KeepLocal
        ));
    }

    #[test]
    fn test_same_lamport_wall_clock_tiebreaker() {
        let local = make_queue("device-a", 5, 1000, 3);
        let remote = make_queue("device-b", 5, 2000, 5);
        match merge_queue(&local, &remote, "device-a") {
            QueueMergeResult::AcceptRemote(q) => {
                assert_eq!(q.updated_at_ms, 2000);
            }
            QueueMergeResult::KeepLocal => panic!("should accept remote (newer wall-clock)"),
        }
    }

    #[test]
    fn test_same_device_echo_ignored() {
        let local = make_queue("device-a", 5, 1000, 3);
        let remote = make_queue("device-a", 7, 2000, 5);
        assert!(matches!(
            merge_queue(&local, &remote, "device-a"),
            QueueMergeResult::KeepLocal
        ));
    }

    #[test]
    fn test_pre_crdt_empty_local_accepts_remote() {
        let local = make_queue("", 0, 0, 0);
        let remote = make_queue("", 0, 0, 3);
        match merge_queue(&local, &remote, "device-a") {
            QueueMergeResult::AcceptRemote(q) => assert_eq!(q.tracks.len(), 3),
            QueueMergeResult::KeepLocal => panic!("should accept remote"),
        }
    }

    #[test]
    fn test_history_merge_disjoint() {
        let t1 = Utc::now();
        let t2 = t1 + chrono::Duration::seconds(60);
        let t3 = t1 + chrono::Duration::seconds(120);

        let local = vec![make_history_entry("a", t1)];
        let remote = vec![
            make_history_entry("b", t2),
            make_history_entry("c", t3),
        ];

        let new = merge_history(&local, &remote);
        assert_eq!(new.len(), 2);
        assert_eq!(new[0].track_id, "b");
        assert_eq!(new[1].track_id, "c");
    }

    #[test]
    fn test_history_merge_overlap() {
        let t1 = Utc::now();
        let t2 = t1 + chrono::Duration::seconds(60);

        let local = vec![
            make_history_entry("a", t1),
            make_history_entry("b", t2),
        ];
        let remote = vec![
            make_history_entry("a", t1), // duplicate
            make_history_entry("c", t2), // different track, same time
        ];

        let new = merge_history(&local, &remote);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].track_id, "c");
    }

    #[test]
    fn test_history_merge_empty_remote() {
        let t1 = Utc::now();
        let local = vec![make_history_entry("a", t1)];
        let new = merge_history(&local, &[]);
        assert!(new.is_empty());
    }

    #[test]
    fn test_history_merge_empty_local() {
        let t1 = Utc::now();
        let remote = vec![make_history_entry("a", t1)];
        let new = merge_history(&[], &remote);
        assert_eq!(new.len(), 1);
    }
}
