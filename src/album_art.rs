use anyhow::{Context, Result};
use image::DynamicImage;
use lru::LruCache;
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use std::num::NonZeroUsize;
use std::path::PathBuf;

/// Handles downloading and caching album art
pub struct AlbumArtCache {
    cache_dir: PathBuf,
    /// In-memory LRU cache of loaded images (bounded to prevent memory exhaustion)
    images: LruCache<String, DynamicImage>,
    /// Protocol handler for rendering images
    picker: Option<Picker>,
    /// Current image protocol state (for rendering)
    current_protocol: Option<Box<dyn StatefulProtocol>>,
}

impl AlbumArtCache {
    /// Create a new album art cache with specified capacity
    pub fn new(capacity: usize) -> Result<Self> {
        let cache_dir = dirs::cache_dir()
            .context("Failed to get cache directory")?
            .join("drift")
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

        // Use at least 10 entries to avoid degenerate cache behavior
        let cap = NonZeroUsize::new(capacity.max(10))
            .expect("capacity should be non-zero");

        Ok(Self {
            cache_dir,
            images: LruCache::new(cap),
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

        // Check if already in memory (use contains to avoid borrow issues)
        if self.images.contains(&cache_key) {
            return Ok(self.images.get(&cache_key).expect("key exists after contains check"));
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

        // Store in memory cache (LRU will evict oldest if at capacity)
        self.images.put(cache_key.clone(), image);

        Ok(self.images.get(&cache_key).expect("just inserted"))
    }

    /// Download and cache album art from a URL (for YouTube/Bandcamp)
    pub async fn get_album_art_from_url(&mut self, url: &str, size: u32) -> Result<&DynamicImage> {
        // Create a sanitized cache key from the URL
        let url_hash = Self::hash_url(url);
        let cache_key = format!("url_{}_{}", url_hash, size);

        // Check if already in memory
        if self.images.contains(&cache_key) {
            return Ok(self.images.get(&cache_key).expect("key exists after contains check"));
        }

        let cache_path = self.cache_dir.join(format!("{}.jpg", cache_key));

        // Check if file exists on disk
        let image = if cache_path.exists() {
            image::open(&cache_path)
                .context("Failed to load cached album art")?
        } else {
            // Download from URL
            let response = reqwest::get(url)
                .await
                .context("Failed to download album art from URL")?;

            let bytes = response.bytes()
                .await
                .context("Failed to read album art bytes")?;

            // Save to disk cache
            std::fs::write(&cache_path, &bytes)
                .context("Failed to write album art to cache")?;

            // Load and optionally resize the image
            let mut img = image::load_from_memory(&bytes)
                .context("Failed to decode album art")?;

            // Resize if needed (YouTube thumbnails can be large)
            if img.width() > size || img.height() > size {
                img = img.thumbnail(size, size);
            }

            img
        };

        // Store in memory cache (LRU will evict oldest if at capacity)
        self.images.put(cache_key.clone(), image);

        Ok(self.images.get(&cache_key).expect("just inserted"))
    }

    /// Simple hash function for URLs to create safe filenames
    fn hash_url(url: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Check if a URL-based image is cached
    pub fn has_url_cached(&self, url: &str, size: u32) -> bool {
        let url_hash = Self::hash_url(url);
        let cache_key = format!("url_{}_{}", url_hash, size);
        self.images.contains(&cache_key)
    }

    /// Set current image from URL cache
    pub fn set_current_image_from_url(&mut self, url: &str, size: u32) -> Result<()> {
        let url_hash = Self::hash_url(url);
        let cache_key = format!("url_{}_{}", url_hash, size);

        if let Some(image) = self.images.peek(&cache_key) {
            if let Some(ref mut picker) = self.picker {
                let protocol = picker.new_resize_protocol(image.clone());
                self.current_protocol = Some(protocol);
            }
        }

        Ok(())
    }

    /// Set the current image to display (creates protocol for rendering)
    pub fn set_current_image(&mut self, cover_id: &str, size: u32) -> Result<()> {
        let cache_key = format!("{}_{}", cover_id, size);

        if let Some(image) = self.images.peek(&cache_key) {
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

    /// Get a cached image if available (non-blocking, doesn't update LRU order)
    pub fn get_cached(&self, cover_id: &str, size: u32) -> Option<&DynamicImage> {
        let cache_key = format!("{}_{}", cover_id, size);
        self.images.peek(&cache_key)
    }

    /// Check if an image is in the cache
    pub fn has_cached(&self, cover_id: &str, size: u32) -> bool {
        let cache_key = format!("{}_{}", cover_id, size);
        self.images.contains(&cache_key)
    }

    /// Check if cover art is cached (handles CoverArt enum)
    pub fn has_cover_cached(&self, cover: &crate::service::CoverArt, size: u32) -> bool {
        match cover {
            crate::service::CoverArt::ServiceId { id, .. } => self.has_cached(id, size),
            crate::service::CoverArt::Url(url) => self.has_url_cached(url, size),
            crate::service::CoverArt::None => false,
        }
    }

    /// Set current image from CoverArt enum
    pub fn set_current_from_cover(&mut self, cover: &crate::service::CoverArt, size: u32) -> Result<bool> {
        match cover {
            crate::service::CoverArt::ServiceId { id, .. } => {
                if self.has_cached(id, size) {
                    self.set_current_image(id, size)?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            crate::service::CoverArt::Url(url) => {
                if self.has_url_cached(url, size) {
                    self.set_current_image_from_url(url, size)?;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            crate::service::CoverArt::None => Ok(false),
        }
    }

    /// Get the cover art identifier for async loading
    pub fn get_cover_source(cover: &crate::service::CoverArt) -> Option<CoverSource> {
        match cover {
            crate::service::CoverArt::ServiceId { id, .. } => Some(CoverSource::TidalId(id.clone())),
            crate::service::CoverArt::Url(url) => Some(CoverSource::Url(url.clone())),
            crate::service::CoverArt::None => None,
        }
    }
}

/// Cover art source for async loading
#[derive(Debug, Clone)]
pub enum CoverSource {
    TidalId(String),
    Url(String),
}
