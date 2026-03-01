//! Cross-device sync handling.
//!
//! When using the Aspen storage backend, other devices may update the
//! queue or history. `poll_sync` checks for these remote changes and
//! applies them to the local app state.

use super::App;
use crate::service::Track;
use crate::storage::SyncEvent;

impl App {
    /// Poll the storage backend for remote changes and apply them.
    ///
    /// Called once per second from the main event loop. Local-only
    /// storage returns empty (no-op). Aspen storage checks for
    /// queue/history updates from other devices.
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
                SyncEvent::HistoryChanged(entries) => {
                    let count = entries.len();
                    self.history_entries = entries;
                    self.add_debug(format!(
                        "⟳ History synced from remote ({} entries)",
                        count
                    ));
                }
            }
        }
    }
}
