//! Download engine for bulk library sync.
//!
//! Orchestrates downloading all favorite albums, tracks, and playlists
//! from Tidal at maximum quality. Handles content-addressed dedup via
//! BLAKE3 hashing, album-level completion caching, unavailable track
//! caching, and metadata tagging.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::storage::DriftStorage;
use crate::tidal_db::TidalDb;

use super::api::{SyncAlbum, SyncApiClient, SyncPlaylist, SyncTrack};

// ── Configuration ────────────────────────────────────────────────────────────

/// Configuration for the sync engine.
pub struct SyncConfig {
    pub output_dir: PathBuf,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            output_dir: dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Music")
                .join("Tidal"),
        }
    }
}

// ── Statistics ───────────────────────────────────────────────────────────────

#[derive(Default)]
struct SyncStats {
    downloaded: u64,
    skipped: u64,
    failed: u64,
    total_bytes: u64,
}

// ── Engine ───────────────────────────────────────────────────────────────────

pub struct SyncEngine {
    api: SyncApiClient,
    db: TidalDb,
    config: SyncConfig,
    stats: SyncStats,
    /// Track IDs processed this session (avoids re-checking redb).
    seen_this_run: HashSet<String>,
    /// Optional Aspen storage for cross-device history sync.
    storage: Option<Box<dyn DriftStorage>>,
}

impl SyncEngine {
    pub fn new(
        api: SyncApiClient,
        db: TidalDb,
        config: SyncConfig,
        storage: Option<Box<dyn DriftStorage>>,
    ) -> Self {
        Self {
            api,
            db,
            config,
            stats: SyncStats::default(),
            seen_this_run: HashSet::new(),
            storage,
        }
    }

    /// Run the full library sync: albums → favorites → playlists.
    pub async fn run(&mut self) -> Result<()> {
        let start = Instant::now();

        // ── Fetch library ────────────────────────────────────────────────
        println!("\n━━━ Fetching library ━━━");

        print!("  Fetching favorite albums... ");
        let albums = self.api.get_favorite_albums().await;
        println!("{} albums", albums.len());

        // Proactive refresh — album pagination may have burned through rate limits
        self.api.refresh_token().await?;

        print!("  Fetching favorite tracks... ");
        let fav_tracks = self.api.get_favorite_tracks().await;
        println!("{} tracks", fav_tracks.len());

        self.api.refresh_token().await?;

        print!("  Fetching playlists... ");
        let playlists = self.api.get_playlists().await;
        println!("{} playlists", playlists.len());

        let total_tracks: u64 = albums.iter().map(|a| a.num_tracks as u64).sum::<u64>()
            + fav_tracks.len() as u64
            + playlists.iter().map(|p| p.num_tracks as u64).sum::<u64>();
        println!("\n  Total: ~{} tracks to process", total_tracks);

        // ── Download ─────────────────────────────────────────────────────
        println!("\n━━━ Downloading albums ━━━");
        self.download_albums(&albums).await;

        println!("\n━━━ Downloading favorite tracks ━━━");
        self.api.refresh_token().await?;
        self.download_favorites(&fav_tracks).await;

        println!("\n━━━ Downloading playlists ━━━");
        self.api.refresh_token().await?;
        self.download_playlists(&playlists).await;

        // ── Summary ──────────────────────────────────────────────────────
        let elapsed = start.elapsed();
        let mins = elapsed.as_secs() / 60;
        let secs = elapsed.as_secs() % 60;

        println!("\n━━━ Done ━━━");
        println!(
            "  Downloaded: {} tracks ({})",
            self.stats.downloaded,
            fmt_bytes(self.stats.total_bytes)
        );
        println!(
            "  Skipped:    {} (already existed)",
            self.stats.skipped
        );
        println!("  Failed:     {}", self.stats.failed);
        println!("  Time:       {}m {}s", mins, secs);
        println!("  Location:   {}", self.config.output_dir.display());

        Ok(())
    }

    // ── Album download ───────────────────────────────────────────────────

    async fn download_albums(&mut self, albums: &[SyncAlbum]) {
        let mut cached = 0u64;

        for (i, album) in albums.iter().enumerate() {
            let idx = i + 1;

            // Album-level cache — skip API call if complete AND track count matches.
            if let Ok(Some(num)) = self.db.check_album(&album.id) {
                if album.num_tracks > 0 && num > 0 && num == album.num_tracks {
                    cached += 1;
                    if cached % 500 == 0 {
                        println!(
                            "  ... {} albums skipped (cached), at [{}/{}]",
                            cached,
                            idx,
                            albums.len()
                        );
                    }
                    continue;
                }
            }

            let artist_dir = sanitize(&album.artist);
            let album_dir = sanitize(&album.title);
            let dest = self.config.output_dir.join(&artist_dir).join(&album_dir);

            println!(
                "\n  [{}/{}] {} — {} ({} tracks)",
                idx,
                albums.len(),
                album.artist,
                album.title,
                album.num_tracks
            );

            let tracks = self.api.get_album_tracks(&album.id).await;
            if tracks.is_empty() {
                println!("    ⚠ No tracks found (delisted?) — caching to skip next run");
                let _ = self.db.mark_album(&album.id, 0);
                continue;
            }

            // Batch pre-check all track IDs in one redb transaction
            let track_ids: Vec<&str> =
                tracks.iter().map(|t| t.id.as_str()).collect();
            let known = self.db.check_batch(&track_ids).unwrap_or_default();
            let unavail = self
                .db
                .check_unavailable_batch(&track_ids)
                .unwrap_or_default();

            let failed_before = self.stats.failed;
            let mut new = 0u64;
            for (j, track) in tracks.iter().enumerate() {
                if self
                    .download_track(
                        track,
                        &dest,
                        (j + 1) as u32,
                        tracks.len() as u32,
                        false,
                        &known,
                        &unavail,
                    )
                    .await
                {
                    new += 1;
                }
            }

            if new == 0 {
                println!("    ✓ Already complete");
            }

            // Mark album complete if no failures and all tracks accounted for
            let album_failed = self.stats.failed - failed_before;
            let known_count = track_ids
                .iter()
                .filter(|id| known.contains(**id))
                .count() as u64;
            if album_failed == 0 && known_count + new == tracks.len() as u64 {
                let _ = self.db.mark_album(&album.id, tracks.len() as u32);
            }
        }

        if cached > 0 {
            println!(
                "\n  ✓ {} albums skipped (cached as complete)",
                cached
            );
        }
    }

    // ── Favorite tracks download ─────────────────────────────────────────

    async fn download_favorites(&mut self, tracks: &[SyncTrack]) {
        if tracks.is_empty() {
            return;
        }

        let track_ids: Vec<&str> =
            tracks.iter().map(|t| t.id.as_str()).collect();
        let known = self.db.check_batch(&track_ids).unwrap_or_default();
        let unavail = self
            .db
            .check_unavailable_batch(&track_ids)
            .unwrap_or_default();

        let already: HashSet<&str> = known
            .iter()
            .map(|s| s.as_str())
            .chain(unavail.iter().map(|s| s.as_str()))
            .collect();
        let new_count = track_ids.iter().filter(|id| !already.contains(**id)).count();

        print!(
            "\n  Favorite Tracks ({} tracks, {} new",
            tracks.len(),
            new_count
        );
        let unavail_overlap = track_ids
            .iter()
            .filter(|id| unavail.contains(**id))
            .count();
        if unavail_overlap > 0 {
            print!(", {} unavailable", unavail_overlap);
        }
        println!(")");

        if new_count == 0 {
            println!("    ✓ All tracks already downloaded");
            self.stats.skipped += tracks.len() as u64;
            for t in tracks {
                self.seen_this_run.insert(t.id.clone());
            }
            return;
        }

        let dest = self.config.output_dir.join("_Favorites");
        for (i, track) in tracks.iter().enumerate() {
            let track_dest = dest.join(sanitize(&track.artist));
            self.download_track(
                track,
                &track_dest,
                (i + 1) as u32,
                tracks.len() as u32,
                true,
                &known,
                &unavail,
            )
            .await;
        }
    }

    // ── Playlist download ────────────────────────────────────────────────

    async fn download_playlists(&mut self, playlists: &[SyncPlaylist]) {
        for (i, pl) in playlists.iter().enumerate() {
            let idx = i + 1;
            let dest = self
                .config
                .output_dir
                .join("_Playlists")
                .join(sanitize(&pl.title));

            println!(
                "\n  [{}/{}] Playlist: {} ({} tracks)",
                idx,
                playlists.len(),
                pl.title,
                pl.num_tracks
            );

            let tracks = self.api.get_playlist_tracks(&pl.id).await;
            if tracks.is_empty() {
                println!("    ⚠ No tracks found");
                continue;
            }

            let track_ids: Vec<&str> =
                tracks.iter().map(|t| t.id.as_str()).collect();
            let known = self.db.check_batch(&track_ids).unwrap_or_default();
            let unavail = self
                .db
                .check_unavailable_batch(&track_ids)
                .unwrap_or_default();

            for (j, track) in tracks.iter().enumerate() {
                self.download_track(
                    track,
                    &dest,
                    (j + 1) as u32,
                    tracks.len() as u32,
                    true,
                    &known,
                    &unavail,
                )
                .await;
            }
        }
    }

    // ── Single track download ────────────────────────────────────────────

    /// Download a single track. Returns `true` if downloaded, `false` if skipped.
    ///
    /// Deduplicates by:
    ///   1. Track ID (pre-download, via batch check)
    ///   2. Existing file on disk (any extension)
    ///   3. BLAKE3 content hash (post-download, cross-directory)
    async fn download_track(
        &mut self,
        track: &SyncTrack,
        dest_dir: &Path,
        index: u32,
        total: u32,
        use_index_as_num: bool,
        known_ids: &HashSet<String>,
        unavailable_ids: &HashSet<String>,
    ) -> bool {
        // Fast path — skip if already processed this run
        if self.seen_this_run.contains(&track.id) {
            self.stats.skipped += 1;
            return false;
        }

        // Skip if batch pre-check says we have it
        if known_ids.contains(&track.id) {
            self.stats.skipped += 1;
            self.seen_this_run.insert(track.id.clone());
            return false;
        }

        // Skip tracks known to be unavailable (cached for 7 days)
        if unavailable_ids.contains(&track.id) {
            self.stats.skipped += 1;
            self.seen_this_run.insert(track.id.clone());
            return false;
        }

        let track_num = if use_index_as_num {
            index
        } else if track.track_number > 0 {
            track.track_number
        } else {
            index
        };

        let title = sanitize(&track.title);
        let prefix = format!("{:02} - {}", track_num, title);

        // Check for existing files with any extension
        if dest_dir.exists() {
            for ext in &["flac", "m4a", "mp3"] {
                let existing = dest_dir.join(format!("{}.{}", prefix, ext));
                if existing.exists() {
                    self.stats.skipped += 1;
                    // Hash and record so cross-directory dedup works
                    if let Ok(hash) = hash_file(&existing) {
                        let _ = self.db.put(
                            &track.id,
                            &hash,
                            &existing.to_string_lossy(),
                            &track.artist,
                            &track.title,
                        );
                    }
                    self.seen_this_run.insert(track.id.clone());
                    return false;
                }
            }
        }

        // Get stream URL with quality cascade
        let (url, codec) = match self.api.get_stream_url(&track.id).await {
            Ok(result) => result,
            Err(e) => {
                println!("    ✗ {}: {}", prefix, e);
                let _ = self.db.mark_unavailable(&track.id);
                self.seen_this_run.insert(track.id.clone());
                self.stats.failed += 1;
                return false;
            }
        };

        let ext = file_extension(&codec);
        let dest = dest_dir.join(format!("{}.{}", prefix, ext));

        // Create destination directory
        if let Err(e) = std::fs::create_dir_all(dest_dir) {
            println!("    ✗ Failed to create directory: {}", e);
            self.stats.failed += 1;
            return false;
        }

        // Download with streaming BLAKE3 hash
        let progress = if total > 0 {
            format!("[{}/{}]", index, total)
        } else {
            String::new()
        };
        println!(
            "    ↓ {} {} - {} [{}]",
            progress, track.artist, track.title, codec
        );

        tokio::time::sleep(self.api.download_delay()).await;

        match self.download_and_hash(&url, &dest).await {
            Ok((content_hash, downloaded)) => {
                // Post-download content dedup — if identical content exists, discard
                if let Ok(Some((_, existing_path))) = self.db.check_hash(&content_hash) {
                    let existing_name = Path::new(&existing_path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    println!(
                        "    ≡ Duplicate content (matches {}), removing",
                        existing_name
                    );
                    let _ = std::fs::remove_file(&dest);
                    self.stats.skipped += 1;
                    let _ = self.db.put(
                        &track.id,
                        &content_hash,
                        &existing_path,
                        &track.artist,
                        &track.title,
                    );
                    self.seen_this_run.insert(track.id.clone());
                    return false;
                }

                self.stats.downloaded += 1;
                self.stats.total_bytes += downloaded;

                // Tag the file with metadata
                tag_file(&dest, track, &ext);

                // Record in redb — ACID write
                let _ = self.db.put(
                    &track.id,
                    &content_hash,
                    &dest.to_string_lossy(),
                    &track.artist,
                    &track.title,
                );

                // Record to Aspen storage if connected
                if let Some(ref storage) = self.storage {
                    let drift_track = crate::service::Track {
                        id: track.id.clone(),
                        title: track.title.clone(),
                        artist: track.artist.clone(),
                        album: track.album.clone(),
                        duration_seconds: 0,
                        cover_art: crate::service::CoverArt::None,
                        service: crate::service::ServiceType::Tidal,
                    };
                    if let Err(e) = storage.record_play(&drift_track).await {
                        eprintln!("    ⚠ Aspen sync: {}", e);
                    }
                }

                self.seen_this_run.insert(track.id.clone());

                true
            }
            Err(e) => {
                println!("    ✗ Download failed: {}", e);
                if dest.exists() {
                    let _ = std::fs::remove_file(&dest);
                }
                self.stats.failed += 1;
                false
            }
        }
    }

    /// Download a file while computing its BLAKE3 hash in a single streaming pass.
    /// Returns (hash_hex, bytes_downloaded).
    async fn download_and_hash(
        &self,
        url: &str,
        dest: &Path,
    ) -> Result<(String, u64)> {
        let resp = self
            .api
            .http()
            .get(url)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .context("Download request failed")?;

        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("HTTP error: {}", resp.status()));
        }

        let mut file = File::create(dest)
            .await
            .context("Failed to create output file")?;
        let mut hasher = blake3::Hasher::new();
        let mut downloaded = 0u64;

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("Stream error during download")?;
            file.write_all(&chunk).await?;
            hasher.update(&chunk);
            downloaded += chunk.len() as u64;
        }

        file.flush().await?;

        let hash = hasher.finalize().to_hex().to_string();
        Ok((hash, downloaded))
    }
}

// ── Utility functions ────────────────────────────────────────────────────────

/// Sanitize a string for use as a filename.
fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .collect();
    let trimmed = s.trim_matches(|c: char| c == '.' || c == ' ');
    if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Determine file extension from codec/quality string.
fn file_extension(codec: &str) -> String {
    let lower = codec.to_lowercase();
    if lower.contains("flac") || lower.contains("lossless") || lower.contains("hi_res") {
        "flac".to_string()
    } else if lower.contains("aac") || lower.contains("mp4") {
        "m4a".to_string()
    } else if lower.contains("mp3") {
        "mp3".to_string()
    } else {
        // Default to FLAC for high quality
        "flac".to_string()
    }
}

/// Compute BLAKE3 hash of a file (streaming, 64KB chunks).
fn hash_file(path: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Tag a downloaded file with metadata.
fn tag_file(path: &Path, track: &SyncTrack, ext: &str) {
    match ext {
        "flac" => tag_flac(path, track),
        "mp3" => tag_mp3(path, track),
        // M4A tagging not yet supported in Rust
        _ => {}
    }
}

fn tag_flac(path: &Path, track: &SyncTrack) {
    match metaflac::Tag::read_from_path(path) {
        Ok(mut tag) => {
            tag.set_vorbis("TITLE", vec![&track.title]);
            tag.set_vorbis("ARTIST", vec![&track.artist]);
            tag.set_vorbis("ALBUM", vec![&track.album]);
            tag.set_vorbis("ALBUMARTIST", vec![&track.album_artist]);
            if track.track_number > 0 {
                tag.set_vorbis("TRACKNUMBER", vec![&track.track_number.to_string()]);
            }
            if track.volume_number > 0 {
                tag.set_vorbis("DISCNUMBER", vec![&track.volume_number.to_string()]);
            }
            if let Err(e) = tag.save() {
                eprintln!("    ⚠ FLAC tagging failed: {}", e);
            }
        }
        Err(e) => {
            eprintln!("    ⚠ Could not read FLAC tags: {}", e);
        }
    }
}

fn tag_mp3(path: &Path, track: &SyncTrack) {
    use id3::{Tag, TagLike, Version};

    let mut tag = Tag::new();
    tag.set_title(&track.title);
    tag.set_artist(&track.artist);
    tag.set_album(&track.album);
    tag.set_album_artist(&track.album_artist);
    if track.track_number > 0 {
        tag.set_track(track.track_number);
    }
    if track.volume_number > 0 {
        tag.set_disc(track.volume_number);
    }
    if let Err(e) = tag.write_to_path(path, Version::Id3v24) {
        eprintln!("    ⚠ MP3 tagging failed: {}", e);
    }
}

/// Format a byte count for display.
fn fmt_bytes(n: u64) -> String {
    let mut n = n as f64;
    for unit in &["B", "KB", "MB", "GB", "TB"] {
        if n < 1024.0 {
            return format!("{:.1} {}", n, unit);
        }
        n /= 1024.0;
    }
    format!("{:.1} PB", n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_sync_config_default() {
        let config = SyncConfig::default();
        assert!(config.output_dir.ends_with("Music/Tidal") || 
                config.output_dir.ends_with("Music\\Tidal"));
    }

    #[test]
    fn test_sanitize_basic() {
        assert_eq!(sanitize("Normal Name"), "Normal Name");
        assert_eq!(sanitize("Album 2023"), "Album 2023");
        assert_eq!(sanitize("Name-with-dash"), "Name-with-dash");
    }

    #[test]
    fn test_sanitize_forbidden_chars() {
        assert_eq!(sanitize("Track: The Beginning"), "Track_ The Beginning");
        assert_eq!(sanitize("Artist/Band"), "Artist_Band");
        assert_eq!(sanitize("File<>Name"), "File__Name");
        assert_eq!(sanitize("Track|Mix"), "Track_Mix");
        assert_eq!(sanitize("Song?"), "Song_");
        assert_eq!(sanitize("Track*Name"), "Track_Name");
        assert_eq!(sanitize("Path\\Name"), "Path_Name");
        assert_eq!(sanitize("Quote\"Name"), "Quote_Name");
    }

    #[test]
    fn test_sanitize_dots_and_spaces() {
        assert_eq!(sanitize("  Name with spaces  "), "Name with spaces");
        assert_eq!(sanitize("...dots..."), "dots");
        assert_eq!(sanitize(". . ."), "_"); // All dots and spaces trim to empty
        assert_eq!(sanitize("   "), "_");
    }

    #[test]
    fn test_sanitize_empty_and_edge_cases() {
        assert_eq!(sanitize(""), "_");
        assert_eq!(sanitize("   "), "_");
        assert_eq!(sanitize("..."), "_");
        assert_eq!(sanitize("."), "_");
    }

    #[test]
    fn test_sanitize_unicode() {
        assert_eq!(sanitize("Café"), "Café");
        assert_eq!(sanitize("日本語"), "日本語");
        assert_eq!(sanitize("Émilie"), "Émilie");
        assert_eq!(sanitize("Pokémon"), "Pokémon");
    }

    #[test]
    fn test_sanitize_mixed() {
        assert_eq!(
            sanitize("Track: \"Best of 2023\" (Remix)"),
            "Track_ _Best of 2023_ (Remix)"
        );
        assert_eq!(
            sanitize("Artist - Album / Track #1"),
            "Artist - Album _ Track #1"
        );
    }

    #[test]
    fn test_file_extension_flac() {
        assert_eq!(file_extension("FLAC"), "flac");
        assert_eq!(file_extension("flac"), "flac");
        assert_eq!(file_extension("LOSSLESS"), "flac");
        assert_eq!(file_extension("HI_RES_LOSSLESS"), "flac");
        assert_eq!(file_extension("HI_RES"), "flac");
    }

    #[test]
    fn test_file_extension_m4a() {
        assert_eq!(file_extension("AAC"), "m4a");
        assert_eq!(file_extension("aac"), "m4a");
        assert_eq!(file_extension("MP4"), "m4a");
        assert_eq!(file_extension("mp4"), "m4a");
    }

    #[test]
    fn test_file_extension_mp3() {
        assert_eq!(file_extension("MP3"), "mp3");
        assert_eq!(file_extension("mp3"), "mp3");
    }

    #[test]
    fn test_file_extension_unknown_defaults_to_flac() {
        assert_eq!(file_extension("UNKNOWN"), "flac");
        assert_eq!(file_extension(""), "flac");
        assert_eq!(file_extension("OGG"), "flac");
    }

    #[test]
    fn test_file_extension_case_insensitive() {
        assert_eq!(file_extension("FLaC"), "flac");
        assert_eq!(file_extension("AaC"), "m4a");
        assert_eq!(file_extension("Mp3"), "mp3");
    }

    #[test]
    fn test_hash_file_basic() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"test content").unwrap();
        temp_file.flush().unwrap();
        
        let hash = hash_file(temp_file.path()).unwrap();
        assert_eq!(hash.len(), 64); // BLAKE3 produces 32 bytes = 64 hex chars
        
        // Hash should be deterministic
        let hash2 = hash_file(temp_file.path()).unwrap();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_hash_file_empty() {
        let temp_file = NamedTempFile::new().unwrap();
        let hash = hash_file(temp_file.path()).unwrap();
        assert_eq!(hash.len(), 64);
        
        // BLAKE3 hash of empty file
        let expected = blake3::hash(b"").to_hex().to_string();
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_hash_file_large() {
        let mut temp_file = NamedTempFile::new().unwrap();
        // Write more than the 64KB buffer size to test streaming
        let large_data = vec![0u8; 150_000];
        temp_file.write_all(&large_data).unwrap();
        temp_file.flush().unwrap();
        
        let hash = hash_file(temp_file.path()).unwrap();
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn test_hash_file_different_content() {
        let mut file1 = NamedTempFile::new().unwrap();
        file1.write_all(b"content A").unwrap();
        file1.flush().unwrap();
        
        let mut file2 = NamedTempFile::new().unwrap();
        file2.write_all(b"content B").unwrap();
        file2.flush().unwrap();
        
        let hash1 = hash_file(file1.path()).unwrap();
        let hash2 = hash_file(file2.path()).unwrap();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_file_nonexistent() {
        let result = hash_file(Path::new("/nonexistent/file.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn test_fmt_bytes_bytes() {
        assert_eq!(fmt_bytes(0), "0.0 B");
        assert_eq!(fmt_bytes(1), "1.0 B");
        assert_eq!(fmt_bytes(999), "999.0 B");
    }

    #[test]
    fn test_fmt_bytes_kilobytes() {
        assert_eq!(fmt_bytes(1024), "1.0 KB");
        assert_eq!(fmt_bytes(2048), "2.0 KB");
        assert_eq!(fmt_bytes(1536), "1.5 KB");
        assert_eq!(fmt_bytes(102400), "100.0 KB");
    }

    #[test]
    fn test_fmt_bytes_megabytes() {
        assert_eq!(fmt_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(fmt_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(fmt_bytes(1536 * 1024), "1.5 MB");
    }

    #[test]
    fn test_fmt_bytes_gigabytes() {
        assert_eq!(fmt_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(fmt_bytes(10 * 1024 * 1024 * 1024), "10.0 GB");
        assert_eq!(fmt_bytes(1536 * 1024 * 1024), "1.5 GB");
    }

    #[test]
    fn test_fmt_bytes_terabytes() {
        assert_eq!(fmt_bytes(1024u64 * 1024 * 1024 * 1024), "1.0 TB");
        assert_eq!(fmt_bytes(5 * 1024u64 * 1024 * 1024 * 1024), "5.0 TB");
    }

    #[test]
    fn test_fmt_bytes_petabytes() {
        assert_eq!(fmt_bytes(1024u64 * 1024 * 1024 * 1024 * 1024), "1.0 PB");
    }

    #[test]
    fn test_fmt_bytes_precision() {
        // Test that we get proper decimal formatting
        assert_eq!(fmt_bytes(1500), "1.5 KB");
        assert_eq!(fmt_bytes(1024 + 512), "1.5 KB");
        assert_eq!(fmt_bytes(2560), "2.5 KB");
    }

    #[test]
    fn test_sync_stats_default() {
        let stats = SyncStats::default();
        assert_eq!(stats.downloaded, 0);
        assert_eq!(stats.skipped, 0);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.total_bytes, 0);
    }

    #[test]
    fn test_sync_album_fields() {
        let album = SyncAlbum {
            id: "album-123".to_string(),
            title: "Test Album".to_string(),
            artist: "Test Artist".to_string(),
            num_tracks: 12,
        };
        
        assert_eq!(album.id, "album-123");
        assert_eq!(album.title, "Test Album");
        assert_eq!(album.artist, "Test Artist");
        assert_eq!(album.num_tracks, 12);
    }

    #[test]
    fn test_sync_track_fields() {
        let track = SyncTrack {
            id: "track-456".to_string(),
            title: "Test Track".to_string(),
            artist: "Track Artist".to_string(),
            album: "Track Album".to_string(),
            album_artist: "Album Artist".to_string(),
            duration_seconds: 180,
            track_number: 3,
            volume_number: 1,
        };
        
        assert_eq!(track.id, "track-456");
        assert_eq!(track.title, "Test Track");
        assert_eq!(track.artist, "Track Artist");
        assert_eq!(track.album, "Track Album");
        assert_eq!(track.album_artist, "Album Artist");
        assert_eq!(track.duration_seconds, 180);
        assert_eq!(track.track_number, 3);
        assert_eq!(track.volume_number, 1);
    }

    #[test]
    fn test_sync_playlist_fields() {
        let playlist = SyncPlaylist {
            id: "playlist-789".to_string(),
            title: "My Playlist".to_string(),
            num_tracks: 25,
        };
        
        assert_eq!(playlist.id, "playlist-789");
        assert_eq!(playlist.title, "My Playlist");
        assert_eq!(playlist.num_tracks, 25);
    }

    // Test filename construction logic
    #[test]
    fn test_track_filename_construction() {
        let track = SyncTrack {
            id: "1".to_string(),
            title: "Song: The Beginning".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            album_artist: "Album Artist".to_string(),
            duration_seconds: 200,
            track_number: 5,
            volume_number: 1,
        };
        
        let sanitized_title = sanitize(&track.title);
        let expected_prefix = format!("{:02} - {}", track.track_number, sanitized_title);
        assert_eq!(expected_prefix, "05 - Song_ The Beginning");
    }

    #[test]
    fn test_track_path_construction() {
        let artist = "The Beatles";
        let album = "Abbey Road";
        let track_title = "Come Together";
        
        let artist_dir = sanitize(artist);
        let album_dir = sanitize(album);
        let track_file = sanitize(track_title);
        
        assert_eq!(artist_dir, "The Beatles");
        assert_eq!(album_dir, "Abbey Road");
        assert_eq!(track_file, "Come Together");
        
        // Full path would be: output_dir / artist_dir / album_dir / "01 - track_file.flac"
    }

    #[test]
    fn test_problematic_artist_names() {
        // Real-world artist names that have caused issues
        assert_eq!(sanitize("AC/DC"), "AC_DC");
        assert_eq!(sanitize("R.E.M."), "R.E.M");
        assert_eq!(sanitize("?uestlove"), "_uestlove");
        assert_eq!(sanitize("Panic! at the Disco"), "Panic! at the Disco"); // ! is not forbidden
    }

    #[test]
    fn test_problematic_album_names() {
        assert_eq!(sanitize("What's Going On?"), "What's Going On_");
        assert_eq!(sanitize("...And Justice for All"), "And Justice for All");
        assert_eq!(sanitize("OK Computer"), "OK Computer");
        assert_eq!(sanitize("The Dark Side of the Moon"), "The Dark Side of the Moon");
    }

    #[test]
    fn test_codec_to_extension_mapping() {
        // Test the codec mapping used in file extension determination
        let codecs = vec![
            ("HI_RES_LOSSLESS", "flac"),
            ("HI_RES", "flac"),
            ("LOSSLESS", "flac"),
            ("AAC", "m4a"),
            ("MP3", "mp3"),
            ("DOLBY_ATMOS", "flac"), // Unknown should default to flac
        ];
        
        for (codec, expected_ext) in codecs {
            assert_eq!(file_extension(codec), expected_ext);
        }
    }

    #[test]
    fn test_track_number_formatting() {
        // Test zero-padding logic
        assert_eq!(format!("{:02}", 1), "01");
        assert_eq!(format!("{:02}", 9), "09");
        assert_eq!(format!("{:02}", 10), "10");
        assert_eq!(format!("{:02}", 99), "99");
        assert_eq!(format!("{:02}", 100), "100"); // Still works for triple digits
    }

    #[test]
    fn test_volume_number_in_filename() {
        // Multi-disc albums should use disc/volume number
        let track = SyncTrack {
            id: "1".to_string(),
            title: "Track".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            album_artist: "Album Artist".to_string(),
            duration_seconds: 200,
            track_number: 3,
            volume_number: 2,
        };
        
        assert_eq!(track.volume_number, 2);
        // The actual implementation would need to use volume_number in path construction
        // e.g., "Disc 2/03 - Track.flac"
    }
}
