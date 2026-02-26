//! tidal-db — JSON-lines RPC server wrapping the shared TidalDb library.
//!
//! Runs as a co-process for the tidal-dl Python script.
//! Usage: tidal-db serve <db_path>

use std::io::{self, BufRead, Write};
use std::path::Path;

use drift::tidal_db::TidalDb;
use serde_json::{json, Value};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args[1] != "serve" {
        eprintln!("Usage: tidal-db serve <db_path>");
        std::process::exit(1);
    }

    let db = TidalDb::create(Path::new(&args[2])).unwrap_or_else(|e| {
        eprintln!("Failed to open redb at {}: {}", args[2], e);
        std::process::exit(1);
    });

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if !l.is_empty() => l,
            Ok(_) => continue,
            Err(_) => break,
        };

        let resp = match serde_json::from_str::<Value>(&line) {
            Ok(cmd) => handle(&db, &cmd),
            Err(e) => json!({"error": e.to_string()}),
        };

        serde_json::to_writer(&mut out, &resp).unwrap();
        out.write_all(b"\n").unwrap();
        out.flush().unwrap();
    }
}

fn handle(db: &TidalDb, cmd: &Value) -> Value {
    match cmd.get("cmd").and_then(|v| v.as_str()) {
        Some("check") => cmd_check(db, cmd),
        Some("check_batch") => cmd_check_batch(db, cmd),
        Some("check_hash") => cmd_check_hash(db, cmd),
        Some("put") => cmd_put(db, cmd),
        Some("check_album") => cmd_check_album(db, cmd),
        Some("mark_album") => cmd_mark_album(db, cmd),
        Some("stats") => cmd_stats(db),
        Some("import") => cmd_import(db, cmd),
        Some("prune") => cmd_prune(db),
        _ => json!({"error": "unknown command"}),
    }
}

fn cmd_check(db: &TidalDb, cmd: &Value) -> Value {
    let track_id = cmd["track_id"].as_str().unwrap_or("");
    match db.check(track_id) {
        Ok(Some(rec)) => json!({"exists": true, "path": rec.path, "hash": rec.hash}),
        Ok(None) => json!({"exists": false}),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_check_batch(db: &TidalDb, cmd: &Value) -> Value {
    let track_ids: Vec<&str> = cmd["track_ids"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    match db.check_batch(&track_ids) {
        Ok(found) => json!({"found": found}),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_check_hash(db: &TidalDb, cmd: &Value) -> Value {
    let hash = cmd["hash"].as_str().unwrap_or("");
    match db.check_hash(hash) {
        Ok(Some((track_id, path))) => json!({"exists": true, "track_id": track_id, "path": path}),
        Ok(None) => json!({"exists": false}),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_put(db: &TidalDb, cmd: &Value) -> Value {
    let track_id = cmd["track_id"].as_str().unwrap_or("");
    let hash = cmd["hash"].as_str().unwrap_or("");
    let path = cmd["path"].as_str().unwrap_or("");
    let artist = cmd["artist"].as_str().unwrap_or("");
    let title = cmd["title"].as_str().unwrap_or("");
    match db.put(track_id, hash, path, artist, title) {
        Ok(()) => json!({"ok": true}),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_check_album(db: &TidalDb, cmd: &Value) -> Value {
    let album_id = cmd["album_id"].as_str().unwrap_or("");
    match db.check_album(album_id) {
        Ok(complete) => json!({"complete": complete}),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_mark_album(db: &TidalDb, cmd: &Value) -> Value {
    let album_id = cmd["album_id"].as_str().unwrap_or("");
    match db.mark_album(album_id) {
        Ok(()) => json!({"ok": true}),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_stats(db: &TidalDb) -> Value {
    match db.track_count() {
        Ok(count) => json!({"count": count}),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_import(db: &TidalDb, cmd: &Value) -> Value {
    let json_path = cmd["json_path"].as_str().unwrap_or("");
    match db.import_json(json_path) {
        Ok(imported) => json!({"imported": imported}),
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn cmd_prune(db: &TidalDb) -> Value {
    match db.prune() {
        Ok(pruned) => json!({"pruned": pruned}),
        Err(e) => json!({"error": e.to_string()}),
    }
}
