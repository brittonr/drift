use anyhow::{Context, Result};
use image::DynamicImage;
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use std::collections::HashMap;
use std::path::PathBuf;

/// Handles downloading and caching album art
pub struct AlbumArtCache {
    cache_dir: PathBuf,
    /// In-memory cache of loaded images
    images: HashMap<String, DynamicImage>,
    /// Protocol handler for rendering images
    picker: Option<Picker>,
    /// Current image protocol state (for rendering)
    current_protocol: Option<Box<dyn StatefulProtocol>>,
}

impl AlbumArtCache {
    /// Create a new album art cache
    pub fn new() -> Result<Self> {
        let cache_dir = dirs::cache_dir()
            .context("Failed to get cache directory")?
            .join("tidal-tui")
            .join("album-art");

        // Create cache directory if it doesn't exist
        std::fs::create_dir_all(&cache_dir)
            .context("Failed to create album art cache directory")?;

        // Try to detect terminal capabilities
        // Try to get font size from terminal, fallback to default if it fails
        let mut picker = Picker::from_termios()
            .ok()
            .or_else(|| Some(Picker::new((8, 16))));

        // Attempt to detect graphics protocol
        if let Some(ref mut p) = picker {
            let _ = p.guess_protocol();
        }

        Ok(Self {
            cache_dir,
            images: HashMap::new(),
            picker,
            current_protocol: None,
        })
    }

    /// Get the file path for a cached cover ID
    fn get_cache_path(&self, cover_id: &str, size: u32) -> PathBuf {
        self.cache_dir.join(format!("{}_{}.jpg", cover_id, size))
    }

    /// Download and cache album art, returns the loaded image
    pub async fn get_album_art(&mut self, cover_id: &str, size: u32) -> Result<&DynamicImage> {
        let cache_key = format!("{}_{}", cover_id, size);

        // Check if already in memory
        if self.images.contains_key(&cache_key) {
            return Ok(self.images.get(&cache_key).unwrap());
        }

        let cache_path = self.get_cache_path(cover_id, size);

        // Check if file exists on disk
        let image = if cache_path.exists() {
            // Load from disk cache
            image::open(&cache_path)
                .context("Failed to load cached album art")?
        } else {
            // Download from Tidal
            let url = crate::tidal::TidalClient::get_album_cover_url(cover_id, size);
            let response = reqwest::get(&url)
                .await
                .context("Failed to download album art")?;

            let bytes = response.bytes()
                .await
                .context("Failed to read album art bytes")?;

            // Save to disk cache
            std::fs::write(&cache_path, &bytes)
                .context("Failed to write album art to cache")?;

            // Load the image
            image::load_from_memory(&bytes)
                .context("Failed to decode album art")?
        };

        // Store in memory cache
        self.images.insert(cache_key.clone(), image);

        Ok(self.images.get(&cache_key).unwrap())
    }

    /// Set the current image to display (creates protocol for rendering)
    pub fn set_current_image(&mut self, cover_id: &str, size: u32) -> Result<()> {
        let cache_key = format!("{}_{}", cover_id, size);

        if let Some(image) = self.images.get(&cache_key) {
            if let Some(ref mut picker) = self.picker {
                // Create a resize protocol for this image
                let protocol = picker.new_resize_protocol(image.clone());
                self.current_protocol = Some(protocol);
            }
        }

        Ok(())
    }

    /// Get the current protocol for rendering
    pub fn get_protocol_mut(&mut self) -> Option<&mut Box<dyn StatefulProtocol>> {
        self.current_protocol.as_mut()
    }

    /// Check if graphics are supported
    pub fn is_supported(&self) -> bool {
        self.picker.is_some()
    }

    /// Clear the in-memory cache (keeps disk cache)
    pub fn clear_memory_cache(&mut self) {
        self.images.clear();
        self.current_protocol = None;
    }

    /// Get the size of the in-memory cache
    pub fn memory_cache_size(&self) -> usize {
        self.images.len()
    }

    /// Get a cached image if available (non-blocking)
    pub fn get_cached(&self, cover_id: &str, size: u32) -> Option<&DynamicImage> {
        let cache_key = format!("{}_{}", cover_id, size);
        self.images.get(&cache_key)
    }

    /// Check if an image is in the cache
    pub fn has_cached(&self, cover_id: &str, size: u32) -> bool {
        let cache_key = format!("{}_{}", cover_id, size);
        self.images.contains_key(&cache_key)
    }
}
