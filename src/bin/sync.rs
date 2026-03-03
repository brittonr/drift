//! drift-sync — Download your entire Tidal library at MAX quality.
//!
//! Native Rust replacement for the tidal-dl Python script.
//! Shares drift's credential handling, TidalDb (redb), and audio tagging.
//!
//! Usage:
//!   drift-sync                         # default output: ~/Music/Tidal/
//!   drift-sync -o /path/to/output      # custom output directory
//!   drift-sync --help                  # show help

use anyhow::Result;
use std::path::PathBuf;

use drift::config::Config;
use drift::storage::DriftStorage;
use drift::sync::{SyncApiClient, SyncConfig, SyncEngine};
use drift::tidal_db::TidalDb;

fn print_help() {
    println!("drift-sync — Download your entire Tidal library at MAX quality.");
    println!();
    println!("USAGE:");
    println!("  drift-sync [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("  -o, --output <DIR>   Output directory (default: ~/Music/Tidal/)");
    println!("  -h, --help           Show this help message");
    println!();
    println!("Downloads all favorite albums, favorite tracks, and playlists");
    println!("at HI_RES_LOSSLESS quality (with automatic fallback).");
    println!();
    println!("Credentials are loaded from:");
    println!("  ~/.config/drift/credentials.json     (drift native)");
    println!("  ~/.config/tidal-tui/credentials.json  (legacy tidal-dl)");
    println!();
    println!("The download database (.tidal-dl.redb) is shared with tidal-dl,");
    println!("so previously downloaded tracks are automatically skipped.");
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Help flag
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(());
    }

    // Parse output directory
    let output_dir = if let Some(idx) = args.iter().position(|a| a == "-o" || a == "--output") {
        args.get(idx + 1)
            .map(|p| PathBuf::from(p))
            .unwrap_or_else(|| {
                eprintln!("Error: -o/--output requires a directory argument");
                std::process::exit(1);
            })
    } else {
        SyncConfig::default().output_dir
    };

    println!("╔══════════════════════════════════════════╗");
    println!("║   drift-sync — MAX quality library sync  ║");
    println!("╚══════════════════════════════════════════╝");
    println!();

    // Load credentials
    let api = SyncApiClient::load()?;
    println!("✓ Loaded credentials (user_id: {})", api.user_id());

    // Create output directory
    std::fs::create_dir_all(&output_dir)?;
    println!("✓ Output directory: {}", output_dir.display());

    // Open redb-backed download history (shared with tidal-dl)
    let db_path = output_dir.join(".tidal-dl.redb");
    let db = TidalDb::create(&db_path)?;

    let track_count = db.track_count()?;
    let unavail_count = db.unavailable_count()?;
    print!("  ✓ Download history: {} tracks in redb", track_count);
    if unavail_count > 0 {
        print!(", {} marked unavailable", unavail_count);
    }
    println!();

    // One-time migration: import old JSON history if it exists
    let json_history = output_dir.join(".tidal-dl-history.json");
    if json_history.exists() {
        if track_count == 0 {
            println!("  ↻ Migrating JSON history to redb...");
            let imported = db.import_json(&json_history.to_string_lossy())?;
            println!("  ✓ Imported {} tracks", imported);
        }
        let backup = json_history.with_extension("json.bak");
        std::fs::rename(&json_history, &backup)?;
        println!(
            "  ✓ Old history backed up to {}",
            backup.file_name().unwrap_or_default().to_string_lossy()
        );
    }

    // Optionally connect to Aspen storage for cross-device sync
    let aspen_storage: Option<Box<dyn DriftStorage>> = {
        let drift_config = Config::load().unwrap_or_default();
        if drift_config.storage.backend == "aspen" {
            #[cfg(feature = "aspen")]
            {
                if let Some(ticket) = drift_config.storage.cluster_ticket.as_deref() {
                    let user_id = drift_config
                        .storage
                        .user_id
                        .unwrap_or_else(|| {
                            hostname::get()
                                .map(|h| h.to_string_lossy().into_owned())
                                .unwrap_or_else(|_| "drift-sync".to_string())
                        });
                    println!("  Connecting to Aspen cluster as '{}'...", user_id);
                    match drift::storage::aspen::AspenStorage::connect(ticket, &user_id).await {
                        Ok(s) => {
                            println!("  ✓ Connected to Aspen cluster");
                            Some(Box::new(s) as Box<dyn DriftStorage>)
                        }
                        Err(e) => {
                            println!("  ⚠ Aspen connection failed: {}, continuing local-only", e);
                            None
                        }
                    }
                } else {
                    println!("  ⚠ Aspen backend configured but no cluster_ticket, skipping");
                    None
                }
            }
            #[cfg(not(feature = "aspen"))]
            {
                println!("  ⚠ Aspen backend configured but 'aspen' feature not enabled, skipping");
                None
            }
        } else {
            None
        }
    };

    // Run the sync
    let config = SyncConfig { output_dir };
    let mut engine = SyncEngine::new(api, db, config, aspen_storage);
    engine.run().await?;

    Ok(())
}
