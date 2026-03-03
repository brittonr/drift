//! Drift server-side plugin for Aspen clusters.
//!
//! Pure data-processing logic for history dedup, search cache TTL,
//! and history pruning. Designed to run as a WASM plugin on Aspen
//! clusters, but also callable from client-side code.
//!
//! # Modules
//!
//! - [`dedup`]: Detect and remove duplicate history plays within a time window
//! - [`ttl`]: Check search cache expiration
//! - [`prune`]: Trim history entries beyond a maximum count
//!
//! # Key Schema
//!
//! ```text
//! drift:{user}:history:{timestamp_ms:020}  → JSON HistoryRecord
//! drift:{user}:queue                        → JSON PersistedQueue
//! drift:{user}:search:{hash}              → JSON CachedSearch
//! drift:{user}:search_history              → JSON SearchHistory
//! ```

pub mod dedup;
pub mod prune;
pub mod ttl;

use serde::{Deserialize, Serialize};

// ── Shared types ─────────────────────────────────────────────────────────────

/// A play history record stored in the cluster KV.
///
/// Matches the `HistoryRecord` in `drift/src/storage/aspen.rs`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryRecord {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover_art_id: Option<String>,
    pub service: String,
    pub played_at_ms: u64,
}

/// A cached search result with timestamp.
///
/// Matches the `CachedSearch` in `drift/src/storage/aspen.rs`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CachedSearch {
    #[serde(rename = "r")]
    pub results_json: String,
    #[serde(rename = "t")]
    pub cached_at_ms: u64,
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Default dedup window: plays of the same track within this window are duplicates.
pub const DEFAULT_DEDUP_WINDOW_MS: u64 = 10_000; // 10 seconds

/// Default search cache TTL.
pub const DEFAULT_CACHE_TTL_MS: u64 = 3_600_000; // 1 hour

/// Default maximum history entries per user.
pub const DEFAULT_MAX_HISTORY_ENTRIES: usize = 500;

// ── Key parsing helpers ──────────────────────────────────────────────────────

/// Extract the user from a drift KV key.
///
/// Keys follow `drift:{user}:{rest}`. Returns `None` if the key doesn't
/// match the expected format.
///
/// # Examples
///
/// ```
/// assert_eq!(drift_plugin::extract_user("drift:alice:history:00001"), Some("alice"));
/// assert_eq!(drift_plugin::extract_user("other:key"), None);
/// ```
pub fn extract_user(key: &str) -> Option<&str> {
    let rest = key.strip_prefix("drift:")?;
    rest.split(':').next()
}

/// Check if a key is a history entry key.
///
/// # Examples
///
/// ```
/// assert!(drift_plugin::is_history_key("drift:alice:history:00000001700000000"));
/// assert!(!drift_plugin::is_history_key("drift:alice:queue"));
/// ```
pub fn is_history_key(key: &str) -> bool {
    // drift:{user}:history:{timestamp}
    let parts: Vec<&str> = key.splitn(4, ':').collect();
    parts.len() == 4 && parts[0] == "drift" && parts[2] == "history"
}

/// Check if a key is a search cache key.
///
/// # Examples
///
/// ```
/// assert!(drift_plugin::is_search_key("drift:alice:search:abc123"));
/// assert!(!drift_plugin::is_search_key("drift:alice:history:00001"));
/// ```
pub fn is_search_key(key: &str) -> bool {
    let parts: Vec<&str> = key.splitn(4, ':').collect();
    parts.len() == 4 && parts[0] == "drift" && parts[2] == "search"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_user_valid() {
        assert_eq!(extract_user("drift:alice:history:001"), Some("alice"));
        assert_eq!(extract_user("drift:bob:queue"), Some("bob"));
        assert_eq!(extract_user("drift:host-01:search:abc"), Some("host-01"));
    }

    #[test]
    fn extract_user_invalid() {
        assert_eq!(extract_user("other:key"), None);
        assert_eq!(extract_user("drift"), None);
        assert_eq!(extract_user(""), None);
    }

    #[test]
    fn is_history_key_valid() {
        assert!(is_history_key("drift:alice:history:00000001700000000"));
        assert!(is_history_key("drift:bob:history:12345"));
    }

    #[test]
    fn is_history_key_invalid() {
        assert!(!is_history_key("drift:alice:queue"));
        assert!(!is_history_key("drift:alice:search:abc"));
        assert!(!is_history_key("other:alice:history:123"));
    }

    #[test]
    fn is_search_key_valid() {
        assert!(is_search_key("drift:alice:search:abc123"));
        assert!(is_search_key("drift:bob:search:deadbeef"));
    }

    #[test]
    fn is_search_key_invalid() {
        assert!(!is_search_key("drift:alice:history:123"));
        assert!(!is_search_key("drift:alice:queue"));
    }

    #[test]
    fn history_record_serde_roundtrip() {
        let record = HistoryRecord {
            track_id: "12345".into(),
            title: "Test Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_seconds: 240,
            cover_art_id: Some("cover-id".into()),
            service: "tidal".into(),
            played_at_ms: 1700000000000,
        };
        let json = serde_json::to_string(&record).unwrap();
        let decoded: HistoryRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record, decoded);
    }

    #[test]
    fn cached_search_field_renames() {
        let cached = CachedSearch {
            results_json: r#"{"tracks":[]}"#.into(),
            cached_at_ms: 1700000000000,
        };
        let json = serde_json::to_string(&cached).unwrap();
        assert!(json.contains(r#""r":"#));
        assert!(json.contains(r#""t":"#));
        assert!(!json.contains("results_json"));
        assert!(!json.contains("cached_at_ms"));
    }
}
