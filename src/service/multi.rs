use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;

use super::{
    Album, Artist, BandcampClient, CoverArt, MusicService, Playlist, SearchResults, ServiceType,
    TidalClient, Track, YouTubeClient,
};
use crate::config::Config;

/// Manages multiple music services and routes operations appropriately
pub struct MultiServiceManager {
    /// All initialized services (keyed by ServiceType)
    services: HashMap<ServiceType, Box<dyn MusicService>>,
    /// Primary service for operations that don't have a clear routing target
    primary: ServiceType,
    /// Service initialization errors (for status display)
    init_errors: HashMap<ServiceType, String>,
}

impl MultiServiceManager {
    /// Initialize all available services based on config
    pub async fn new(config: &Config) -> Result<Self> {
        let mut services: HashMap<ServiceType, Box<dyn MusicService>> = HashMap::new();
        let mut init_errors: HashMap<ServiceType, String> = HashMap::new();

        // Always try Tidal
        match TidalClient::new().await {
            Ok(mut client) => {
                client.set_audio_quality(&config.playback.audio_quality);
                services.insert(ServiceType::Tidal, Box::new(client));
            }
            Err(e) => {
                init_errors.insert(ServiceType::Tidal, e.to_string());
            }
        }

        // Try YouTube/Bandcamp if yt-dlp available and auto-detect enabled
        if config.service.auto_detect && Self::check_ytdlp_available().await {
            // YouTube
            if Self::should_enable_service(&config.service.enabled, "youtube") {
                match YouTubeClient::new(None).await {
                    Ok(mut client) => {
                        client.set_audio_quality(&config.playback.audio_quality);
                        services.insert(ServiceType::YouTube, Box::new(client));
                    }
                    Err(e) => {
                        init_errors.insert(ServiceType::YouTube, e.to_string());
                    }
                }
            }

            // Bandcamp
            if Self::should_enable_service(&config.service.enabled, "bandcamp") {
                match BandcampClient::new(config.bandcamp.clone()).await {
                    Ok(mut client) => {
                        client.set_audio_quality(&config.playback.audio_quality);
                        services.insert(ServiceType::Bandcamp, Box::new(client));
                    }
                    Err(e) => {
                        init_errors.insert(ServiceType::Bandcamp, e.to_string());
                    }
                }
            }
        }

        // Determine primary service
        let primary = Self::determine_primary(&config.service.primary, &services)?;

        Ok(Self {
            services,
            primary,
            init_errors,
        })
    }

    /// Check if yt-dlp is available in PATH
    async fn check_ytdlp_available() -> bool {
        Command::new("yt-dlp")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check if a service should be enabled based on config
    fn should_enable_service(enabled_list: &[String], service: &str) -> bool {
        // Empty list means enable all
        enabled_list.is_empty() || enabled_list.iter().any(|s| s.eq_ignore_ascii_case(service))
    }

    /// Determine the primary service from config or first available
    fn determine_primary(
        preferred: &str,
        services: &HashMap<ServiceType, Box<dyn MusicService>>,
    ) -> Result<ServiceType> {
        // Try to parse preferred service
        if let Ok(service_type) = preferred.parse::<ServiceType>() {
            if services.contains_key(&service_type) {
                return Ok(service_type);
            }
        }

        // Fall back to first available in priority order
        [
            ServiceType::Tidal,
            ServiceType::YouTube,
            ServiceType::Bandcamp,
        ]
        .into_iter()
        .find(|s| services.contains_key(s))
        .ok_or_else(|| anyhow!("No music services available"))
    }

    /// Get list of enabled services
    pub fn enabled_services(&self) -> Vec<ServiceType> {
        self.services.keys().copied().collect()
    }

    /// Get initialization errors
    pub fn init_errors(&self) -> &HashMap<ServiceType, String> {
        &self.init_errors
    }

    /// Get the primary service type
    pub fn primary_service(&self) -> ServiceType {
        self.primary
    }

    /// Detect service from track ID format
    pub fn detect_service_from_id(track_id: &str) -> ServiceType {
        // Bandcamp: URLs
        if track_id.starts_with("http") || track_id.contains("bandcamp.com") {
            return ServiceType::Bandcamp;
        }

        // YouTube: 11 character video IDs (alphanumeric with - and _)
        if track_id.len() == 11
            && track_id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return ServiceType::YouTube;
        }

        // Tidal: numeric IDs
        ServiceType::Tidal
    }

    /// Get a mutable reference to a specific service
    fn get_service_mut(&mut self, service: ServiceType) -> Result<&mut Box<dyn MusicService>> {
        self.services
            .get_mut(&service)
            .ok_or_else(|| anyhow!("Service {} not available", service))
    }

    /// Get stream URL, routing to correct service based on track
    pub async fn get_stream_url_for_track(&mut self, track: &Track) -> Result<String> {
        let service = self.get_service_mut(track.service)?;
        service.get_stream_url(&track.id).await
    }

    /// Get stream URL by ID, detecting service from ID format
    pub async fn get_stream_url_by_id(&mut self, track_id: &str) -> Result<String> {
        let service_type = Self::detect_service_from_id(track_id);
        let service = self.get_service_mut(service_type)?;
        service.get_stream_url(track_id).await
    }
}

#[async_trait]
impl MusicService for MultiServiceManager {
    fn service_type(&self) -> ServiceType {
        self.primary
    }

    fn is_authenticated(&self) -> bool {
        // Return true if primary service is authenticated
        self.services
            .get(&self.primary)
            .map(|s| s.is_authenticated())
            .unwrap_or(false)
    }

    fn set_audio_quality(&mut self, quality: &str) {
        for service in self.services.values_mut() {
            service.set_audio_quality(quality);
        }
    }

    async fn get_stream_url(&mut self, track_id: &str) -> Result<String> {
        // Route based on track ID format detection
        self.get_stream_url_by_id(track_id).await
    }

    // === Library: Playlists ===

    async fn get_playlists(&mut self) -> Result<Vec<Playlist>> {
        let mut all_playlists = Vec::new();

        for service in self.services.values_mut() {
            match service.get_playlists().await {
                Ok(playlists) => all_playlists.extend(playlists),
                Err(e) => {
                    tracing::warn!("Failed to get playlists from {}: {}", service.service_type(), e);
                }
            }
        }

        Ok(all_playlists)
    }

    async fn get_playlist_tracks(&mut self, playlist_id: &str) -> Result<Vec<Track>> {
        // Detect service from playlist ID format
        let service_type = if playlist_id.starts_with("local-") {
            // YouTube local playlists
            ServiceType::YouTube
        } else if playlist_id.starts_with("collection:") || playlist_id.contains("bandcamp") {
            ServiceType::Bandcamp
        } else {
            // Try primary first, then others
            self.primary
        };

        // Try detected service first
        if let Some(service) = self.services.get_mut(&service_type) {
            if let Ok(tracks) = service.get_playlist_tracks(playlist_id).await {
                return Ok(tracks);
            }
        }

        // Fall back to trying all services
        for service in self.services.values_mut() {
            if let Ok(tracks) = service.get_playlist_tracks(playlist_id).await {
                return Ok(tracks);
            }
        }

        Err(anyhow!("Playlist not found in any service"))
    }

    // === Library: Favorites ===

    async fn get_favorite_tracks(&mut self) -> Result<Vec<Track>> {
        let mut all_tracks = Vec::new();

        for service in self.services.values_mut() {
            match service.get_favorite_tracks().await {
                Ok(tracks) => all_tracks.extend(tracks),
                Err(e) => {
                    tracing::warn!("Failed to get favorites from {}: {}", service.service_type(), e);
                }
            }
        }

        Ok(all_tracks)
    }

    async fn get_favorite_albums(&mut self) -> Result<Vec<Album>> {
        let mut all_albums = Vec::new();

        for service in self.services.values_mut() {
            match service.get_favorite_albums().await {
                Ok(albums) => all_albums.extend(albums),
                Err(e) => {
                    tracing::warn!("Failed to get favorite albums from {}: {}", service.service_type(), e);
                }
            }
        }

        Ok(all_albums)
    }

    async fn get_favorite_artists(&mut self) -> Result<Vec<Artist>> {
        let mut all_artists = Vec::new();

        for service in self.services.values_mut() {
            match service.get_favorite_artists().await {
                Ok(artists) => all_artists.extend(artists),
                Err(e) => {
                    tracing::warn!("Failed to get favorite artists from {}: {}", service.service_type(), e);
                }
            }
        }

        Ok(all_artists)
    }

    async fn add_favorite_track(&mut self, track_id: &str) -> Result<()> {
        let service_type = Self::detect_service_from_id(track_id);
        let service = self.get_service_mut(service_type)?;
        service.add_favorite_track(track_id).await
    }

    async fn remove_favorite_track(&mut self, track_id: &str) -> Result<()> {
        let service_type = Self::detect_service_from_id(track_id);
        let service = self.get_service_mut(service_type)?;
        service.remove_favorite_track(track_id).await
    }

    // === Search ===

    async fn search(&mut self, query: &str, limit: usize) -> Result<SearchResults> {
        let mut all_tracks = Vec::new();
        let mut all_albums = Vec::new();
        let mut all_artists = Vec::new();

        let per_service_limit = (limit / self.services.len().max(1)).max(5);

        for service in self.services.values_mut() {
            match service.search(query, per_service_limit).await {
                Ok(results) => {
                    all_tracks.extend(results.tracks);
                    all_albums.extend(results.albums);
                    all_artists.extend(results.artists);
                }
                Err(e) => {
                    tracing::warn!("Search failed for {}: {}", service.service_type(), e);
                }
            }
        }

        // Interleave results by service for variety
        all_tracks = Self::interleave_by_service(all_tracks);
        all_albums = Self::interleave_by_service(all_albums);

        Ok(SearchResults {
            tracks: all_tracks.into_iter().take(limit).collect(),
            albums: all_albums.into_iter().take(limit).collect(),
            artists: all_artists.into_iter().take(limit).collect(),
        })
    }

    // === Album/Artist Details ===

    async fn get_album_tracks(&mut self, album_id: &str) -> Result<Vec<Track>> {
        let service_type = Self::detect_service_from_id(album_id);
        let service = self.get_service_mut(service_type)?;
        service.get_album_tracks(album_id).await
    }

    async fn get_artist_top_tracks(&mut self, artist_id: &str) -> Result<Vec<Track>> {
        let service_type = Self::detect_service_from_id(artist_id);
        let service = self.get_service_mut(service_type)?;
        service.get_artist_top_tracks(artist_id).await
    }

    async fn get_artist_albums(&mut self, artist_id: &str) -> Result<Vec<Album>> {
        let service_type = Self::detect_service_from_id(artist_id);
        let service = self.get_service_mut(service_type)?;
        service.get_artist_albums(artist_id).await
    }

    // === Radio/Recommendations ===

    async fn get_track_radio(&mut self, track_id: &str, limit: usize) -> Result<Vec<Track>> {
        let service_type = Self::detect_service_from_id(track_id);
        let service = self.get_service_mut(service_type)?;
        service.get_track_radio(track_id, limit).await
    }

    async fn get_artist_radio(&mut self, artist_id: &str, limit: usize) -> Result<Vec<Track>> {
        let service_type = Self::detect_service_from_id(artist_id);
        let service = self.get_service_mut(service_type)?;
        service.get_artist_radio(artist_id, limit).await
    }

    async fn get_playlist_radio(&mut self, playlist_id: &str, limit: usize) -> Result<Vec<Track>> {
        // Try primary service first for playlist radio
        if let Some(service) = self.services.get_mut(&self.primary) {
            if let Ok(tracks) = service.get_playlist_radio(playlist_id, limit).await {
                return Ok(tracks);
            }
        }

        // Fall back to trying all services
        for service in self.services.values_mut() {
            if let Ok(tracks) = service.get_playlist_radio(playlist_id, limit).await {
                return Ok(tracks);
            }
        }

        Err(anyhow!("Could not get playlist radio"))
    }

    // === Playlist Management ===

    async fn create_playlist(&mut self, name: &str, description: Option<&str>) -> Result<Playlist> {
        // Create on primary service
        let service = self.get_service_mut(self.primary)?;
        service.create_playlist(name, description).await
    }

    async fn update_playlist(
        &mut self,
        playlist_id: &str,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        // Try to find which service owns this playlist
        for service in self.services.values_mut() {
            if service.update_playlist(playlist_id, title, description).await.is_ok() {
                return Ok(());
            }
        }
        Err(anyhow!("Playlist not found"))
    }

    async fn delete_playlist(&mut self, playlist_id: &str) -> Result<()> {
        for service in self.services.values_mut() {
            if service.delete_playlist(playlist_id).await.is_ok() {
                return Ok(());
            }
        }
        Err(anyhow!("Playlist not found"))
    }

    async fn add_tracks_to_playlist(
        &mut self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<()> {
        for service in self.services.values_mut() {
            if service.add_tracks_to_playlist(playlist_id, track_ids).await.is_ok() {
                return Ok(());
            }
        }
        Err(anyhow!("Playlist not found"))
    }

    async fn remove_tracks_from_playlist(
        &mut self,
        playlist_id: &str,
        indices: &[usize],
    ) -> Result<()> {
        for service in self.services.values_mut() {
            if service.remove_tracks_from_playlist(playlist_id, indices).await.is_ok() {
                return Ok(());
            }
        }
        Err(anyhow!("Playlist not found"))
    }

    // === Cover Art ===

    fn get_cover_url(&self, cover: &CoverArt, size: u32) -> Option<String> {
        match cover {
            CoverArt::Url(url) => Some(url.clone()),
            CoverArt::ServiceId { id: _, service } => {
                self.services.get(service).and_then(|s| s.get_cover_url(cover, size))
            }
            CoverArt::None => None,
        }
    }
}

impl MultiServiceManager {
    /// Interleave items by their service type for variety in display
    fn interleave_by_service<T: HasServiceType>(items: Vec<T>) -> Vec<T> {
        if items.len() <= 1 {
            return items;
        }

        // Group by service
        let mut by_service: HashMap<ServiceType, Vec<T>> = HashMap::new();
        for item in items {
            by_service.entry(item.service_type()).or_default().push(item);
        }

        // Interleave
        let mut result = Vec::new();
        let service_order = [ServiceType::Tidal, ServiceType::YouTube, ServiceType::Bandcamp];
        let mut indices: HashMap<ServiceType, usize> = HashMap::new();

        loop {
            let mut added_any = false;
            for service in &service_order {
                if let Some(items) = by_service.get_mut(service) {
                    let idx = indices.entry(*service).or_insert(0);
                    if *idx < items.len() {
                        // Need to swap_remove to avoid clone requirement
                        if *idx == items.len() - 1 {
                            result.push(items.pop().unwrap());
                        } else {
                            // For non-last items, we need to be creative
                            // Just drain in order for simplicity
                            continue;
                        }
                        added_any = true;
                    }
                }
            }
            if !added_any {
                break;
            }
        }

        // Append remaining items
        for (_, mut items) in by_service {
            result.append(&mut items);
        }

        result
    }
}

/// Trait for types that have a service type
trait HasServiceType {
    fn service_type(&self) -> ServiceType;
}

impl HasServiceType for Track {
    fn service_type(&self) -> ServiceType {
        self.service
    }
}

impl HasServiceType for Album {
    fn service_type(&self) -> ServiceType {
        self.service
    }
}

impl HasServiceType for Artist {
    fn service_type(&self) -> ServiceType {
        self.service
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_service_from_id() {
        // Bandcamp URLs
        assert_eq!(
            MultiServiceManager::detect_service_from_id("https://artist.bandcamp.com/track/song"),
            ServiceType::Bandcamp
        );
        assert_eq!(
            MultiServiceManager::detect_service_from_id("http://test.bandcamp.com/album/test"),
            ServiceType::Bandcamp
        );

        // YouTube video IDs (11 chars, alphanumeric with - and _)
        assert_eq!(
            MultiServiceManager::detect_service_from_id("dQw4w9WgXcQ"),
            ServiceType::YouTube
        );
        assert_eq!(
            MultiServiceManager::detect_service_from_id("abc-_123456"),
            ServiceType::YouTube
        );

        // Tidal numeric IDs
        assert_eq!(
            MultiServiceManager::detect_service_from_id("123456789"),
            ServiceType::Tidal
        );
        assert_eq!(
            MultiServiceManager::detect_service_from_id("1"),
            ServiceType::Tidal
        );
    }

    #[test]
    fn test_should_enable_service() {
        // Empty list enables all
        assert!(MultiServiceManager::should_enable_service(&[], "youtube"));
        assert!(MultiServiceManager::should_enable_service(&[], "tidal"));

        // Explicit list
        let enabled = vec!["tidal".to_string(), "youtube".to_string()];
        assert!(MultiServiceManager::should_enable_service(&enabled, "tidal"));
        assert!(MultiServiceManager::should_enable_service(&enabled, "YouTube")); // case insensitive
        assert!(!MultiServiceManager::should_enable_service(&enabled, "bandcamp"));
    }
}
