//! Playlist synchronization and merge logic.
//!
//! Synced playlists are stored in the cluster KV:
//!
//! ```text
//! drift:{user}:playlist:{id}       → JSON SyncedPlaylist
//! drift:{user}:playlist_index      → JSON PlaylistIndex
//! ```
//!
//! # Conflict Resolution
//!
//! - **Metadata** (title, description, visibility): Last-Writer-Wins (LWW) via
//!   lamport clock, with wall-clock tiebreaker
//! - **Tracks**: OR-set union — merge all tracks by `(id, service)` key,
//!   keeping the earlier `added_at_ms` when both have the same track
//!
//! # Own Echo Detection
//!
//! When a write returns from the cluster and triggers a watch callback,
//! we skip merging if `remote.device_id == local_device_id` — this is our
//! own write echoing back.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Types ────────────────────────────────────────────────────────────────────

/// A synced playlist with conflict-free merge metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncedPlaylist {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub tracks: Vec<SyncedTrackRef>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub lamport_clock: u64,
    pub device_id: String,
    #[serde(default)]
    pub visibility: PlaylistVisibility,
}

/// A track reference in a synced playlist.
///
/// Tracks are identified by `(id, service)` for OR-set merge logic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncedTrackRef {
    pub id: String,
    pub service: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_art_url: Option<String>,
    pub added_at_ms: u64,
    pub added_by: String,
}

/// Index of all playlists for a user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaylistIndex {
    pub playlists: Vec<PlaylistIndexEntry>,
    pub updated_at_ms: u64,
    pub lamport_clock: u64,
    pub device_id: String,
}

/// Summary entry in the playlist index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaylistIndexEntry {
    pub id: String,
    pub title: String,
    pub track_count: usize,
    pub updated_at_ms: u64,
    pub visibility: PlaylistVisibility,
}

/// Playlist visibility setting.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PlaylistVisibility {
    Private,
    Shared,
}

impl Default for PlaylistVisibility {
    fn default() -> Self {
        Self::Shared
    }
}

/// Result of merging two playlists.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaylistMergeResult {
    /// Remote is strictly newer — accept it as-is
    AcceptRemote(SyncedPlaylist),
    /// Local is strictly newer — keep it
    KeepLocal,
    /// Merge required — contains the merged playlist
    Merged(SyncedPlaylist),
}

// ── Key helpers ──────────────────────────────────────────────────────────────

/// Build a playlist key: `drift:{user}:playlist:{playlist_id}`
///
/// # Examples
///
/// ```
/// use drift_plugin::playlist::playlist_key;
/// assert_eq!(playlist_key("alice", "favorites"), "drift:alice:playlist:favorites");
/// ```
pub fn playlist_key(user: &str, playlist_id: &str) -> String {
    format!("drift:{}:playlist:{}", user, playlist_id)
}

/// Build a playlist index key: `drift:{user}:playlist_index`
///
/// # Examples
///
/// ```
/// use drift_plugin::playlist::playlist_index_key;
/// assert_eq!(playlist_index_key("alice"), "drift:alice:playlist_index");
/// ```
pub fn playlist_index_key(user: &str) -> String {
    format!("drift:{}:playlist_index", user)
}

/// Check if a key is a playlist key (NOT the playlist_index).
///
/// # Examples
///
/// ```
/// use drift_plugin::playlist::is_playlist_key;
/// assert!(is_playlist_key("drift:alice:playlist:favorites"));
/// assert!(!is_playlist_key("drift:alice:playlist_index"));
/// assert!(!is_playlist_key("drift:alice:history:123"));
/// ```
pub fn is_playlist_key(key: &str) -> bool {
    let parts: Vec<&str> = key.splitn(4, ':').collect();
    parts.len() == 4 && parts[0] == "drift" && parts[2] == "playlist"
}

/// Extract the playlist ID from a playlist key.
///
/// # Examples
///
/// ```
/// use drift_plugin::playlist::extract_playlist_id;
/// assert_eq!(extract_playlist_id("drift:alice:playlist:favorites"), Some("favorites"));
/// assert_eq!(extract_playlist_id("drift:alice:playlist_index"), None);
/// assert_eq!(extract_playlist_id("drift:alice:history:123"), None);
/// ```
pub fn extract_playlist_id(key: &str) -> Option<&str> {
    let parts: Vec<&str> = key.splitn(4, ':').collect();
    if parts.len() == 4 && parts[0] == "drift" && parts[2] == "playlist" {
        Some(parts[3])
    } else {
        None
    }
}

// ── Merge logic ──────────────────────────────────────────────────────────────

/// Merge two playlists with conflict-free semantics.
///
/// # Own Echo Detection
///
/// If `remote.device_id == local_device_id`, this is our own write echoing back.
/// Return `KeepLocal` immediately.
///
/// # Metadata Merge (LWW)
///
/// - Title, description, visibility: use the version with higher `lamport_clock`
/// - On clock tie, use higher `updated_at_ms`
///
/// # Track Merge (OR-set)
///
/// - Union all tracks by `(id, service)` key
/// - When both have the same track, keep the one with earlier `added_at_ms`
///
/// # Result Clock
///
/// The merged playlist gets `max(local.lamport_clock, remote.lamport_clock) + 1`.
///
/// # Examples
///
/// ```
/// use drift_plugin::playlist::{merge_playlist, SyncedPlaylist, SyncedTrackRef, PlaylistVisibility};
///
/// let local = SyncedPlaylist {
///     id: "pl1".into(),
///     title: "Old Title".into(),
///     description: None,
///     tracks: vec![],
///     created_at_ms: 1000,
///     updated_at_ms: 1000,
///     lamport_clock: 1,
///     device_id: "dev-a".into(),
///     visibility: PlaylistVisibility::Shared,
/// };
///
/// let remote = SyncedPlaylist {
///     id: "pl1".into(),
///     title: "New Title".into(),
///     description: Some("Updated".into()),
///     tracks: vec![],
///     created_at_ms: 1000,
///     updated_at_ms: 2000,
///     lamport_clock: 2,
///     device_id: "dev-b".into(),
///     visibility: PlaylistVisibility::Private,
/// };
///
/// let result = merge_playlist(&local, &remote, "dev-a");
/// // Remote is strictly newer
/// ```
pub fn merge_playlist(
    local: &SyncedPlaylist,
    remote: &SyncedPlaylist,
    local_device_id: &str,
) -> PlaylistMergeResult {
    // Skip own echo
    if remote.device_id == local_device_id {
        return PlaylistMergeResult::KeepLocal;
    }

    // Decide metadata winner
    let remote_metadata_wins = if remote.lamport_clock > local.lamport_clock {
        true
    } else if remote.lamport_clock < local.lamport_clock {
        false
    } else {
        // Clock tie — wall-clock tiebreaker
        remote.updated_at_ms >= local.updated_at_ms
    };

    // Merge tracks: OR-set union
    let merged_tracks = merge_track_lists(&local.tracks, &remote.tracks);

    // Check if we can just accept remote as-is
    let remote_tracks_identical = tracks_equal(&remote.tracks, &merged_tracks);
    if remote_metadata_wins && remote_tracks_identical {
        return PlaylistMergeResult::AcceptRemote(remote.clone());
    }

    // Check if local is unchanged
    let local_tracks_identical = tracks_equal(&local.tracks, &merged_tracks);
    let local_metadata_wins = !remote_metadata_wins;
    if local_metadata_wins && local_tracks_identical {
        return PlaylistMergeResult::KeepLocal;
    }

    // Need to merge
    let (title, description, visibility) = if remote_metadata_wins {
        (
            remote.title.clone(),
            remote.description.clone(),
            remote.visibility,
        )
    } else {
        (
            local.title.clone(),
            local.description.clone(),
            local.visibility,
        )
    };

    let merged = SyncedPlaylist {
        id: local.id.clone(),
        title,
        description,
        tracks: merged_tracks,
        created_at_ms: local.created_at_ms, // Keep original creation time
        updated_at_ms: u64::max(local.updated_at_ms, remote.updated_at_ms),
        lamport_clock: u64::max(local.lamport_clock, remote.lamport_clock) + 1,
        device_id: local_device_id.to_string(),
        visibility,
    };

    PlaylistMergeResult::Merged(merged)
}

/// Merge two track lists using OR-set semantics.
///
/// Union all tracks by `(id, service)` key. When both lists have the same track,
/// keep the one with earlier `added_at_ms` (first to add wins).
fn merge_track_lists(local: &[SyncedTrackRef], remote: &[SyncedTrackRef]) -> Vec<SyncedTrackRef> {
    let mut by_key: HashMap<(String, String), SyncedTrackRef> = HashMap::new();

    // Add local tracks
    for track in local {
        let key = (track.id.clone(), track.service.clone());
        by_key.insert(key, track.clone());
    }

    // Merge remote tracks
    for track in remote {
        let key = (track.id.clone(), track.service.clone());
        by_key
            .entry(key)
            .and_modify(|existing| {
                // Keep the earlier added_at_ms
                if track.added_at_ms < existing.added_at_ms {
                    *existing = track.clone();
                }
            })
            .or_insert_with(|| track.clone());
    }

    // Return sorted by added_at_ms for deterministic ordering
    let mut merged: Vec<_> = by_key.into_values().collect();
    merged.sort_by_key(|t| t.added_at_ms);
    merged
}

/// Check if two track lists are equal (order-independent).
fn tracks_equal(a: &[SyncedTrackRef], b: &[SyncedTrackRef]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut a_sorted = a.to_vec();
    let mut b_sorted = b.to_vec();
    a_sorted.sort_by(|x, y| (&x.id, &x.service).cmp(&(&y.id, &y.service)));
    b_sorted.sort_by(|x, y| (&x.id, &x.service).cmp(&(&y.id, &y.service)));

    a_sorted == b_sorted
}

/// Merge two playlist indexes.
///
/// # Algorithm
///
/// - Union all entries by playlist ID
/// - For entries in both, keep the one with higher `updated_at_ms`
/// - Result gets `max(local.lamport_clock, remote.lamport_clock) + 1`
///
/// # Own Echo Detection
///
/// If `remote.device_id == local_device_id`, return local unchanged.
///
/// # Examples
///
/// ```
/// use drift_plugin::playlist::{merge_playlist_index, PlaylistIndex, PlaylistIndexEntry, PlaylistVisibility};
///
/// let local = PlaylistIndex {
///     playlists: vec![
///         PlaylistIndexEntry {
///             id: "pl1".into(),
///             title: "Favorites".into(),
///             track_count: 10,
///             updated_at_ms: 1000,
///             visibility: PlaylistVisibility::Shared,
///         },
///     ],
///     updated_at_ms: 1000,
///     lamport_clock: 1,
///     device_id: "dev-a".into(),
/// };
///
/// let remote = PlaylistIndex {
///     playlists: vec![
///         PlaylistIndexEntry {
///             id: "pl2".into(),
///             title: "Workout".into(),
///             track_count: 5,
///             updated_at_ms: 2000,
///             visibility: PlaylistVisibility::Private,
///         },
///     ],
///     updated_at_ms: 2000,
///     lamport_clock: 2,
///     device_id: "dev-b".into(),
/// };
///
/// let merged = merge_playlist_index(&local, &remote, "dev-a");
/// assert_eq!(merged.playlists.len(), 2); // Union of both
/// ```
pub fn merge_playlist_index(
    local: &PlaylistIndex,
    remote: &PlaylistIndex,
    local_device_id: &str,
) -> PlaylistIndex {
    // Skip own echo
    if remote.device_id == local_device_id {
        return local.clone();
    }

    let mut by_id: HashMap<String, PlaylistIndexEntry> = HashMap::new();

    // Add local entries
    for entry in &local.playlists {
        by_id.insert(entry.id.clone(), entry.clone());
    }

    // Merge remote entries
    for entry in &remote.playlists {
        by_id
            .entry(entry.id.clone())
            .and_modify(|existing| {
                // Keep the newer entry
                if entry.updated_at_ms > existing.updated_at_ms {
                    *existing = entry.clone();
                }
            })
            .or_insert_with(|| entry.clone());
    }

    // Return sorted by ID for deterministic ordering
    let mut playlists: Vec<_> = by_id.into_values().collect();
    playlists.sort_by(|a, b| a.id.cmp(&b.id));

    PlaylistIndex {
        playlists,
        updated_at_ms: u64::max(local.updated_at_ms, remote.updated_at_ms),
        lamport_clock: u64::max(local.lamport_clock, remote.lamport_clock) + 1,
        device_id: local_device_id.to_string(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Serde roundtrip ──────────────────────────────────────────────────────

    #[test]
    fn synced_playlist_serde_roundtrip() {
        let playlist = SyncedPlaylist {
            id: "pl1".into(),
            title: "Favorites".into(),
            description: Some("My favorite tracks".into()),
            tracks: vec![SyncedTrackRef {
                id: "track1".into(),
                service: "tidal".into(),
                title: "Song".into(),
                artist: "Artist".into(),
                album: "Album".into(),
                duration_seconds: 240,
                cover_art_url: Some("https://example.com/cover.jpg".into()),
                added_at_ms: 1000,
                added_by: "dev-a".into(),
            }],
            created_at_ms: 1000,
            updated_at_ms: 2000,
            lamport_clock: 5,
            device_id: "dev-a".into(),
            visibility: PlaylistVisibility::Private,
        };

        let json = serde_json::to_string(&playlist).unwrap();
        let decoded: SyncedPlaylist = serde_json::from_str(&json).unwrap();
        assert_eq!(playlist, decoded);
    }

    #[test]
    fn synced_track_ref_serde_roundtrip() {
        let track = SyncedTrackRef {
            id: "42".into(),
            service: "spotify".into(),
            title: "Test".into(),
            artist: "Test Artist".into(),
            album: "Test Album".into(),
            duration_seconds: 180,
            cover_art_url: None,
            added_at_ms: 1700000000000,
            added_by: "device-1".into(),
        };

        let json = serde_json::to_string(&track).unwrap();
        let decoded: SyncedTrackRef = serde_json::from_str(&json).unwrap();
        assert_eq!(track, decoded);
    }

    #[test]
    fn playlist_index_serde_roundtrip() {
        let index = PlaylistIndex {
            playlists: vec![
                PlaylistIndexEntry {
                    id: "pl1".into(),
                    title: "Favorites".into(),
                    track_count: 10,
                    updated_at_ms: 1000,
                    visibility: PlaylistVisibility::Shared,
                },
                PlaylistIndexEntry {
                    id: "pl2".into(),
                    title: "Workout".into(),
                    track_count: 5,
                    updated_at_ms: 2000,
                    visibility: PlaylistVisibility::Private,
                },
            ],
            updated_at_ms: 2000,
            lamport_clock: 3,
            device_id: "dev-a".into(),
        };

        let json = serde_json::to_string(&index).unwrap();
        let decoded: PlaylistIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(index, decoded);
    }

    #[test]
    fn playlist_visibility_serde() {
        let priv_json = serde_json::to_string(&PlaylistVisibility::Private).unwrap();
        assert_eq!(priv_json, r#""private""#);

        let shared_json = serde_json::to_string(&PlaylistVisibility::Shared).unwrap();
        assert_eq!(shared_json, r#""shared""#);

        let decoded_priv: PlaylistVisibility = serde_json::from_str(r#""private""#).unwrap();
        assert_eq!(decoded_priv, PlaylistVisibility::Private);

        let decoded_shared: PlaylistVisibility = serde_json::from_str(r#""shared""#).unwrap();
        assert_eq!(decoded_shared, PlaylistVisibility::Shared);
    }

    #[test]
    fn playlist_visibility_default() {
        assert_eq!(PlaylistVisibility::default(), PlaylistVisibility::Shared);

        // Test that #[serde(default)] works
        let json = r#"{"id":"pl1","title":"Test","tracks":[],"created_at_ms":1000,"updated_at_ms":1000,"lamport_clock":1,"device_id":"dev-a"}"#;
        let playlist: SyncedPlaylist = serde_json::from_str(json).unwrap();
        assert_eq!(playlist.visibility, PlaylistVisibility::Shared);
    }

    // ── Key helpers ──────────────────────────────────────────────────────────

    #[test]
    fn playlist_key_valid() {
        assert_eq!(
            playlist_key("alice", "favorites"),
            "drift:alice:playlist:favorites"
        );
        assert_eq!(playlist_key("bob", "123"), "drift:bob:playlist:123");
    }

    #[test]
    fn playlist_index_key_valid() {
        assert_eq!(playlist_index_key("alice"), "drift:alice:playlist_index");
        assert_eq!(playlist_index_key("bob"), "drift:bob:playlist_index");
    }

    #[test]
    fn is_playlist_key_valid() {
        assert!(is_playlist_key("drift:alice:playlist:favorites"));
        assert!(is_playlist_key("drift:bob:playlist:123"));
    }

    #[test]
    fn is_playlist_key_invalid() {
        assert!(!is_playlist_key("drift:alice:playlist_index"));
        assert!(!is_playlist_key("drift:alice:history:123"));
        assert!(!is_playlist_key("drift:alice:queue"));
        assert!(!is_playlist_key("other:alice:playlist:fav"));
        assert!(!is_playlist_key("drift:alice"));
    }

    #[test]
    fn extract_playlist_id_valid() {
        assert_eq!(
            extract_playlist_id("drift:alice:playlist:favorites"),
            Some("favorites")
        );
        assert_eq!(
            extract_playlist_id("drift:bob:playlist:abc123"),
            Some("abc123")
        );
    }

    #[test]
    fn extract_playlist_id_invalid() {
        assert_eq!(extract_playlist_id("drift:alice:playlist_index"), None);
        assert_eq!(extract_playlist_id("drift:alice:history:123"), None);
        assert_eq!(extract_playlist_id("drift:alice:queue"), None);
        assert_eq!(extract_playlist_id("other:key"), None);
    }

    // ── merge_playlist ───────────────────────────────────────────────────────

    fn make_playlist(
        id: &str,
        title: &str,
        lamport: u64,
        updated: u64,
        device: &str,
        tracks: Vec<SyncedTrackRef>,
    ) -> SyncedPlaylist {
        SyncedPlaylist {
            id: id.into(),
            title: title.into(),
            description: None,
            tracks,
            created_at_ms: 1000,
            updated_at_ms: updated,
            lamport_clock: lamport,
            device_id: device.into(),
            visibility: PlaylistVisibility::Shared,
        }
    }

    fn make_track(id: &str, service: &str, added_at: u64, added_by: &str) -> SyncedTrackRef {
        SyncedTrackRef {
            id: id.into(),
            service: service.into(),
            title: "Title".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_seconds: 200,
            cover_art_url: None,
            added_at_ms: added_at,
            added_by: added_by.into(),
        }
    }

    #[test]
    fn merge_playlist_own_echo_ignored() {
        let local = make_playlist("pl1", "Title", 1, 1000, "dev-a", vec![]);
        let remote = make_playlist("pl1", "Different", 2, 2000, "dev-a", vec![]);

        let result = merge_playlist(&local, &remote, "dev-a");
        assert_eq!(result, PlaylistMergeResult::KeepLocal);
    }

    #[test]
    fn merge_playlist_remote_newer_wins() {
        let local = make_playlist("pl1", "Old Title", 1, 1000, "dev-a", vec![]);
        let remote = make_playlist("pl1", "New Title", 2, 2000, "dev-b", vec![]);

        let result = merge_playlist(&local, &remote, "dev-a");
        match result {
            PlaylistMergeResult::AcceptRemote(p) => {
                assert_eq!(p.title, "New Title");
                assert_eq!(p.lamport_clock, 2);
            }
            _ => panic!("Expected AcceptRemote"),
        }
    }

    #[test]
    fn merge_playlist_local_newer_wins() {
        let local = make_playlist("pl1", "New Title", 2, 2000, "dev-a", vec![]);
        let remote = make_playlist("pl1", "Old Title", 1, 1000, "dev-b", vec![]);

        let result = merge_playlist(&local, &remote, "dev-a");
        assert_eq!(result, PlaylistMergeResult::KeepLocal);
    }

    #[test]
    fn merge_playlist_clock_tie_wall_clock_tiebreaker() {
        let local = make_playlist("pl1", "Title A", 1, 1000, "dev-a", vec![]);
        let remote = make_playlist("pl1", "Title B", 1, 2000, "dev-b", vec![]);

        let result = merge_playlist(&local, &remote, "dev-a");
        match result {
            PlaylistMergeResult::AcceptRemote(p) => {
                assert_eq!(p.title, "Title B"); // Remote has newer wall clock
            }
            _ => panic!("Expected AcceptRemote"),
        }
    }

    #[test]
    fn merge_playlist_track_union() {
        let local = make_playlist(
            "pl1",
            "Title",
            1,
            1000,
            "dev-a",
            vec![make_track("t1", "tidal", 1000, "dev-a")],
        );
        let remote = make_playlist(
            "pl1",
            "Title",
            1,
            1000,
            "dev-b",
            vec![make_track("t2", "spotify", 2000, "dev-b")],
        );

        let result = merge_playlist(&local, &remote, "dev-a");
        match result {
            PlaylistMergeResult::Merged(p) => {
                assert_eq!(p.tracks.len(), 2);
                assert_eq!(p.tracks[0].id, "t1"); // Earlier added_at_ms
                assert_eq!(p.tracks[1].id, "t2");
                assert_eq!(p.lamport_clock, 2); // max(1, 1) + 1
            }
            _ => panic!("Expected Merged, got {:?}", result),
        }
    }

    #[test]
    fn merge_playlist_duplicate_track_keep_earlier() {
        let local = make_playlist(
            "pl1",
            "Title",
            2,
            2000,
            "dev-a",
            vec![make_track("t1", "tidal", 2000, "dev-a")],
        );
        let remote = make_playlist(
            "pl1",
            "Title",
            1,
            1000,
            "dev-b",
            vec![make_track("t1", "tidal", 1000, "dev-b")],
        );

        let result = merge_playlist(&local, &remote, "dev-a");
        match result {
            PlaylistMergeResult::Merged(p) => {
                assert_eq!(p.tracks.len(), 1);
                assert_eq!(p.tracks[0].added_at_ms, 1000); // Keep earlier
                assert_eq!(p.tracks[0].added_by, "dev-b");
                assert_eq!(p.lamport_clock, 3); // max(2, 1) + 1
            }
            _ => panic!("Expected Merged, got {:?}", result),
        }
    }

    #[test]
    fn merge_playlist_concurrent_different_tracks() {
        let local = make_playlist(
            "pl1",
            "Title A",
            1,
            1000,
            "dev-a",
            vec![make_track("t1", "tidal", 1000, "dev-a")],
        );
        let remote = make_playlist(
            "pl1",
            "Title B",
            2,
            2000,
            "dev-b",
            vec![make_track("t2", "spotify", 2000, "dev-b")],
        );

        let result = merge_playlist(&local, &remote, "dev-a");
        match result {
            PlaylistMergeResult::Merged(p) => {
                assert_eq!(p.title, "Title B"); // Remote metadata wins (higher clock)
                assert_eq!(p.tracks.len(), 2); // Union of tracks
                assert_eq!(p.lamport_clock, 3); // max(1, 2) + 1
            }
            _ => panic!("Expected Merged"),
        }
    }

    #[test]
    fn merge_playlist_visibility_merge() {
        let local = make_playlist("pl1", "Title", 1, 1000, "dev-a", vec![]);
        let mut remote = make_playlist("pl1", "Title", 2, 2000, "dev-b", vec![]);
        remote.visibility = PlaylistVisibility::Private;

        let result = merge_playlist(&local, &remote, "dev-a");
        match result {
            PlaylistMergeResult::AcceptRemote(p) => {
                assert_eq!(p.visibility, PlaylistVisibility::Private);
            }
            _ => panic!("Expected AcceptRemote"),
        }
    }

    // ── merge_playlist_index ─────────────────────────────────────────────────

    fn make_index(
        entries: Vec<PlaylistIndexEntry>,
        lamport: u64,
        updated: u64,
        device: &str,
    ) -> PlaylistIndex {
        PlaylistIndex {
            playlists: entries,
            updated_at_ms: updated,
            lamport_clock: lamport,
            device_id: device.into(),
        }
    }

    fn make_entry(id: &str, title: &str, updated: u64) -> PlaylistIndexEntry {
        PlaylistIndexEntry {
            id: id.into(),
            title: title.into(),
            track_count: 10,
            updated_at_ms: updated,
            visibility: PlaylistVisibility::Shared,
        }
    }

    #[test]
    fn merge_index_own_echo_ignored() {
        let local = make_index(vec![make_entry("pl1", "Title", 1000)], 1, 1000, "dev-a");
        let remote = make_index(
            vec![make_entry("pl2", "Different", 2000)],
            2,
            2000,
            "dev-a",
        );

        let result = merge_playlist_index(&local, &remote, "dev-a");
        assert_eq!(result, local); // Unchanged
    }

    #[test]
    fn merge_index_union() {
        let local = make_index(vec![make_entry("pl1", "Favorites", 1000)], 1, 1000, "dev-a");
        let remote = make_index(vec![make_entry("pl2", "Workout", 2000)], 2, 2000, "dev-b");

        let result = merge_playlist_index(&local, &remote, "dev-a");
        assert_eq!(result.playlists.len(), 2);
        assert_eq!(result.playlists[0].id, "pl1"); // Sorted by ID
        assert_eq!(result.playlists[1].id, "pl2");
        assert_eq!(result.lamport_clock, 3); // max(1, 2) + 1
        assert_eq!(result.device_id, "dev-a");
    }

    #[test]
    fn merge_index_dedup_by_id() {
        let local = make_index(
            vec![make_entry("pl1", "Old Title", 1000)],
            1,
            1000,
            "dev-a",
        );
        let remote = make_index(
            vec![make_entry("pl1", "New Title", 2000)],
            2,
            2000,
            "dev-b",
        );

        let result = merge_playlist_index(&local, &remote, "dev-a");
        assert_eq!(result.playlists.len(), 1);
        assert_eq!(result.playlists[0].title, "New Title"); // Remote newer
        assert_eq!(result.playlists[0].updated_at_ms, 2000);
    }

    #[test]
    fn merge_index_conflict_resolution_local_newer() {
        let local = make_index(
            vec![make_entry("pl1", "Newer Title", 3000)],
            1,
            3000,
            "dev-a",
        );
        let remote = make_index(
            vec![make_entry("pl1", "Older Title", 2000)],
            2,
            2000,
            "dev-b",
        );

        let result = merge_playlist_index(&local, &remote, "dev-a");
        assert_eq!(result.playlists.len(), 1);
        assert_eq!(result.playlists[0].title, "Newer Title"); // Local newer
        assert_eq!(result.playlists[0].updated_at_ms, 3000);
    }

    #[test]
    fn merge_index_empty_local() {
        let local = make_index(vec![], 1, 1000, "dev-a");
        let remote = make_index(vec![make_entry("pl1", "Title", 2000)], 2, 2000, "dev-b");

        let result = merge_playlist_index(&local, &remote, "dev-a");
        assert_eq!(result.playlists.len(), 1);
        assert_eq!(result.playlists[0].id, "pl1");
    }

    #[test]
    fn merge_index_empty_remote() {
        let local = make_index(vec![make_entry("pl1", "Title", 1000)], 1, 1000, "dev-a");
        let remote = make_index(vec![], 2, 2000, "dev-b");

        let result = merge_playlist_index(&local, &remote, "dev-a");
        assert_eq!(result.playlists.len(), 1);
        assert_eq!(result.playlists[0].id, "pl1");
    }
}
