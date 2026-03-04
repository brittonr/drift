//! Cross-device sync handling.
//!
//! When using the local-first storage backend with Aspen replication,
//! other devices may update the queue or history. `poll_sync` checks
//! for these remote changes and merges them into local app state
//! using CRDT semantics — never replacing wholesale.

use super::App;
use crate::service::Track;
use crate::storage::SyncEvent;

impl App {
    /// Poll the storage backend for remote changes and merge them.
    ///
    /// Called once per second from the main event loop. Local-first
    /// storage returns empty until the replication task detects
    /// remote changes and writes them to the local store.
    pub async fn poll_sync(&mut self) {
        let events = match self.storage.poll_changes().await {
            Ok(events) => events,
            Err(e) => {
                // Don't spam — only log if we haven't recently
                self.add_debug(format!("Sync poll error: {e}"));
                return;
            }
        };

        for event in events {
            match event {
                SyncEvent::QueueChanged(persisted) => {
                    let track_count = persisted.tracks.len();
                    self.local_queue = persisted.tracks.iter().map(Track::from).collect();

                    self.add_debug(format!(
                        "⟳ Queue synced from remote ({} tracks)",
                        track_count
                    ));
                }
                SyncEvent::HistoryChanged(new_entries) => {
                    // Merge — append new entries, don't replace
                    let new_count = new_entries.len();
                    if new_count > 0 {
                        self.history_entries.extend(new_entries);
                        // Sort by played_at descending (most recent first)
                        self.history_entries.sort_by(|a, b| b.played_at.cmp(&a.played_at));
                        // Cap at 500 entries
                        self.history_entries.truncate(500);
                        self.add_debug(format!(
                            "⟳ History: merged {} new entries from remote",
                            new_count
                        ));
                    }
                }
            }
        }
    }
}
