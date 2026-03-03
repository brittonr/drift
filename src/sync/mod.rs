//! Bulk library sync for Tidal.
//!
//! Downloads your entire Tidal library at MAX (HI_RES_LOSSLESS) quality.
//! Replaces the old `tidal-dl` Python script with native Rust, sharing
//! drift's existing TidalDb, credential handling, and audio tagging.
//!
//! Usage: `drift-sync [-o /path/to/output]`

pub mod api;
pub mod engine;

pub use api::SyncApiClient;
pub use engine::{SyncConfig, SyncEngine};

#[cfg(test)]
mod tests {
    #[test]
    fn test_sync_mod() {
        assert_eq!(2 + 2, 4);
    }
}
