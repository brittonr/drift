//! History pruning.
//!
//! Each user accumulates history entries over time. This module identifies
//! entries to delete when the count exceeds a configured maximum, keeping
//! the most recent entries.

/// Given history keys sorted newest-first, return keys to delete.
///
/// Keys are expected to be in reverse chronological order (newest first).
/// Everything beyond `max_entries` is returned for deletion.
///
/// # Arguments
///
/// * `keys` — History keys, newest first
/// * `max_entries` — Maximum number of entries to keep
///
/// # Examples
///
/// ```
/// use drift_plugin::prune;
///
/// let keys: Vec<String> = (0..10).rev().map(|i| format!("key-{}", i)).collect();
/// let to_delete = prune::keys_to_prune(&keys, 7);
/// assert_eq!(to_delete.len(), 3);
/// ```
pub fn keys_to_prune(keys: &[String], max_entries: usize) -> Vec<String> {
    if keys.len() <= max_entries {
        return Vec::new();
    }
    keys[max_entries..].to_vec()
}

/// Determine how many entries to prune for a given count.
///
/// Returns 0 if count is at or below max.
///
/// # Examples
///
/// ```
/// use drift_plugin::prune;
///
/// assert_eq!(prune::excess_count(500, 500), 0);
/// assert_eq!(prune::excess_count(510, 500), 10);
/// assert_eq!(prune::excess_count(100, 500), 0);
/// ```
pub fn excess_count(current: usize, max_entries: usize) -> usize {
    current.saturating_sub(max_entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_MAX_HISTORY_ENTRIES;

    #[test]
    fn under_limit_no_pruning() {
        let keys: Vec<String> = (0..10).map(|i| format!("k{}", i)).collect();
        assert!(keys_to_prune(&keys, DEFAULT_MAX_HISTORY_ENTRIES).is_empty());
    }

    #[test]
    fn at_limit_no_pruning() {
        let keys: Vec<String> = (0..DEFAULT_MAX_HISTORY_ENTRIES)
            .map(|i| format!("k{}", i))
            .collect();
        assert!(keys_to_prune(&keys, DEFAULT_MAX_HISTORY_ENTRIES).is_empty());
    }

    #[test]
    fn over_limit_prunes_tail() {
        let keys: Vec<String> = (0..10).map(|i| format!("k{}", i)).collect();
        let pruned = keys_to_prune(&keys, 7);
        assert_eq!(pruned.len(), 3);
        assert_eq!(pruned, vec!["k7", "k8", "k9"]);
    }

    #[test]
    fn one_over_limit() {
        let keys: Vec<String> = (0..6).map(|i| format!("k{}", i)).collect();
        let pruned = keys_to_prune(&keys, 5);
        assert_eq!(pruned, vec!["k5"]);
    }

    #[test]
    fn empty_keys() {
        let pruned = keys_to_prune(&[], 500);
        assert!(pruned.is_empty());
    }

    #[test]
    fn max_zero_prunes_all() {
        let keys: Vec<String> = (0..3).map(|i| format!("k{}", i)).collect();
        let pruned = keys_to_prune(&keys, 0);
        assert_eq!(pruned.len(), 3);
    }

    #[test]
    fn excess_count_at_limit() {
        assert_eq!(excess_count(500, 500), 0);
    }

    #[test]
    fn excess_count_over() {
        assert_eq!(excess_count(510, 500), 10);
    }

    #[test]
    fn excess_count_under() {
        assert_eq!(excess_count(100, 500), 0);
    }

    #[test]
    fn excess_count_zero() {
        assert_eq!(excess_count(0, 500), 0);
    }
}
