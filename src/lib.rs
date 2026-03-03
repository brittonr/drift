// Public modules for the library crate (used by drift-sync and other binaries).
//
// The main TUI binary (src/main.rs) declares its own private modules.
// This library exports only data types and storage abstractions.

pub mod album_art;
pub mod app;
pub mod cava;
pub mod config;
pub mod download_db;
pub mod downloads;
pub mod handlers;
pub mod history_db;
pub mod mpd;
pub mod queue_persistence;
pub mod search;
pub mod search_cache;
pub mod service;
pub mod storage;
pub mod sync;
pub mod tidal_db;
pub mod ui;
pub mod video;
