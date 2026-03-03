//! Search cache TTL management.
//!
//! Search results are cached in the cluster KV with a timestamp.
//! This module checks whether entries have expired and identifies
//! stale cache keys for deletion.

use crate::CachedSearch;

/// Check if a cached search entry has expired.
///
/// # Arguments
///
/// * `cached_at_ms` — When the entry was cached (Unix epoch milliseconds)
/// * `now_ms` — Current time (Unix epoch milliseconds)
/// * `ttl_ms` — Time-to-live in milliseconds
///
/// # Examples
///
/// ```
/// use drift_plugin::ttl;
///
/// // Cached 2 hours ago, TTL is 1 hour → expired
/// assert!(ttl::is_expired(0, 7_200_000, 3_600_000));
///
/// // Cached 30 minutes ago, TTL is 1 hour → not expired
/// assert!(!ttl::is_expired(0, 1_800_000, 3_600_000));
/// ```
pub fn is_expired(cached_at_ms: u64, now_ms: u64, ttl_ms: u64) -> bool {
    if now_ms < cached_at_ms {
        // Clock skew — treat as not expired
        return false;
    }
    (now_ms - cached_at_ms) > ttl_ms
}

/// Given a list of search cache entries, return the keys that have expired.
///
/// # Arguments
///
/// * `entries` — `(key, CachedSearch)` pairs from a KV scan
/// * `now_ms` — Current time
/// * `ttl_ms` — Cache TTL
///
/// # Examples
///
/// ```
/// use drift_plugin::{CachedSearch, ttl};
///
/// let entries = vec![
///     ("k1".into(), CachedSearch { results_json: "{}".into(), cached_at_ms: 0 }),
///     ("k2".into(), CachedSearch { results_json: "{}".into(), cached_at_ms: 5_000_000 }),
/// ];
/// let expired = ttl::find_expired(&entries, 4_000_000, 3_600_000);
/// assert_eq!(expired, vec!["k1"]); // k1 cached 4M ms ago > 3.6M TTL
/// ```
pub fn find_expired(
    entries: &[(String, CachedSearch)],
    now_ms: u64,
    ttl_ms: u64,
) -> Vec<String> {
    entries
        .iter()
        .filter(|(_, cached)| is_expired(cached.cached_at_ms, now_ms, ttl_ms))
        .map(|(key, _)| key.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_CACHE_TTL_MS;

    #[test]
    fn not_expired_within_ttl() {
        assert!(!is_expired(1000, 2000, DEFAULT_CACHE_TTL_MS));
    }

    #[test]
    fn expired_past_ttl() {
        let cached_at = 0;
        let now = DEFAULT_CACHE_TTL_MS + 1;
        assert!(is_expired(cached_at, now, DEFAULT_CACHE_TTL_MS));
    }

    #[test]
    fn exact_boundary_not_expired() {
        // At exactly TTL, not expired (> not >=)
        assert!(!is_expired(0, DEFAULT_CACHE_TTL_MS, DEFAULT_CACHE_TTL_MS));
    }

    #[test]
    fn one_past_boundary_expired() {
        assert!(is_expired(0, DEFAULT_CACHE_TTL_MS + 1, DEFAULT_CACHE_TTL_MS));
    }

    #[test]
    fn clock_skew_not_expired() {
        // now < cached_at — treat as not expired
        assert!(!is_expired(5000, 1000, 100));
    }

    #[test]
    fn zero_ttl_always_expired() {
        assert!(is_expired(0, 1, 0));
    }

    #[test]
    fn find_expired_mixed() {
        let now = 10_000;
        let ttl = 5_000;
        let entries = vec![
            ("fresh".into(), CachedSearch { results_json: "{}".into(), cached_at_ms: 8_000 }),  // 2s old
            ("stale".into(), CachedSearch { results_json: "{}".into(), cached_at_ms: 1_000 }),  // 9s old
            ("borderline".into(), CachedSearch { results_json: "{}".into(), cached_at_ms: 5_000 }),  // exactly 5s
        ];
        let expired = find_expired(&entries, now, ttl);
        assert_eq!(expired, vec!["stale"]); // only stale is past TTL
    }

    #[test]
    fn find_expired_all_fresh() {
        let entries = vec![
            ("a".into(), CachedSearch { results_json: "{}".into(), cached_at_ms: 9_000 }),
            ("b".into(), CachedSearch { results_json: "{}".into(), cached_at_ms: 9_500 }),
        ];
        let expired = find_expired(&entries, 10_000, 5_000);
        assert!(expired.is_empty());
    }

    #[test]
    fn find_expired_all_stale() {
        let entries = vec![
            ("a".into(), CachedSearch { results_json: "{}".into(), cached_at_ms: 0 }),
            ("b".into(), CachedSearch { results_json: "{}".into(), cached_at_ms: 1_000 }),
        ];
        let expired = find_expired(&entries, 100_000, 5_000);
        assert_eq!(expired.len(), 2);
    }

    #[test]
    fn find_expired_empty() {
        let expired = find_expired(&[], 10_000, 5_000);
        assert!(expired.is_empty());
    }
}
