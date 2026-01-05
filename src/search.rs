// Enhanced search module with history, debouncing, fuzzy filtering, and parallel search

use anyhow::Result;
use chrono::{DateTime, Utc};
use nucleo::{Config as NucleoConfig, Matcher, Utf32Str};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::time::timeout;

use crate::config::SearchConfig;
use crate::service::{Album, Artist, MusicService, SearchResults, ServiceType, Track};

/// Search history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHistoryEntry {
    pub query: String,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub searched_at: DateTime<Utc>,
    pub result_count: usize,
}

/// Persistent search history storage
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchHistory {
    #[serde(default)]
    pub entries: VecDeque<SearchHistoryEntry>,
    #[serde(default)]
    pub max_size: usize,
}

impl SearchHistory {
    const FILE_NAME: &'static str = "search_history.toml";

    fn storage_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
        Ok(config_dir.join("drift").join(Self::FILE_NAME))
    }

    /// Load search history from disk
    pub fn load(max_size: usize) -> Self {
        let path = match Self::storage_path() {
            Ok(p) => p,
            Err(_) => return Self::new(max_size),
        };

        if !path.exists() {
            return Self::new(max_size);
        }

        match fs::read_to_string(&path) {
            Ok(contents) => {
                let mut history: SearchHistory = toml::from_str(&contents).unwrap_or_default();
                history.max_size = max_size;
                // Trim to max size
                while history.entries.len() > max_size {
                    history.entries.pop_back();
                }
                history
            }
            Err(_) => Self::new(max_size),
        }
    }

    /// Create new empty history
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            max_size,
        }
    }

    /// Save search history to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::storage_path()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = toml::to_string_pretty(self)?;
        fs::write(&path, contents)?;
        Ok(())
    }

    /// Add a search query to history
    pub fn add(&mut self, query: &str, result_count: usize) {
        // Don't add empty queries or duplicates of the most recent
        if query.trim().is_empty() {
            return;
        }

        // Remove existing entry with same query to move it to front
        self.entries.retain(|e| e.query.to_lowercase() != query.to_lowercase());

        // Add to front
        self.entries.push_front(SearchHistoryEntry {
            query: query.to_string(),
            searched_at: Utc::now(),
            result_count,
        });

        // Trim to max size
        while self.entries.len() > self.max_size {
            self.entries.pop_back();
        }
    }

    /// Get suggestions matching a prefix
    pub fn get_suggestions(&self, prefix: &str) -> Vec<&str> {
        if prefix.is_empty() {
            return self.entries.iter().take(10).map(|e| e.query.as_str()).collect();
        }

        let prefix_lower = prefix.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.query.to_lowercase().starts_with(&prefix_lower))
            .take(10)
            .map(|e| e.query.as_str())
            .collect()
    }

    /// Clear all history
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Enhanced search state with debouncing and history
// EnhancedSearchState is prepared for enhanced search UX
#[allow(dead_code)]
pub struct EnhancedSearchState {
    /// Current search query
    pub query: String,
    /// Last query that was actually searched
    pub last_searched_query: String,
    /// Time of last keystroke (for debouncing)
    pub last_keystroke: Option<Instant>,
    /// Whether a search is in progress
    pub searching: bool,
    /// Search history
    pub history: SearchHistory,
    /// History suggestion index (-1 = none selected)
    pub history_index: i32,
    /// Fuzzy matcher for local filtering
    matcher: Matcher,
    /// Filter query for local results
    pub filter_query: String,
    /// Is filter mode active
    pub filter_active: bool,
}

#[allow(dead_code)]
impl EnhancedSearchState {
    pub fn new(config: &SearchConfig) -> Self {
        Self {
            query: String::new(),
            last_searched_query: String::new(),
            last_keystroke: None,
            searching: false,
            history: SearchHistory::load(config.history_size),
            history_index: -1,
            matcher: Matcher::new(NucleoConfig::DEFAULT),
            filter_query: String::new(),
            filter_active: false,
        }
    }

    /// Record a keystroke for debouncing
    pub fn keystroke(&mut self) {
        self.last_keystroke = Some(Instant::now());
        self.history_index = -1; // Reset history navigation
    }

    /// Check if debounce period has passed
    pub fn should_search(&self, debounce_ms: u64, min_chars: usize) -> bool {
        if self.query.len() < min_chars {
            return false;
        }
        if self.query == self.last_searched_query {
            return false;
        }
        if let Some(last) = self.last_keystroke {
            last.elapsed() >= Duration::from_millis(debounce_ms)
        } else {
            false
        }
    }

    /// Navigate history up
    pub fn history_up(&mut self) {
        if self.history.entries.is_empty() {
            return;
        }
        let max_index = self.history.entries.len() as i32 - 1;
        self.history_index = (self.history_index + 1).min(max_index);
        if let Some(entry) = self.history.entries.get(self.history_index as usize) {
            self.query = entry.query.clone();
        }
    }

    /// Navigate history down
    pub fn history_down(&mut self) {
        if self.history_index > 0 {
            self.history_index -= 1;
            if let Some(entry) = self.history.entries.get(self.history_index as usize) {
                self.query = entry.query.clone();
            }
        } else if self.history_index == 0 {
            self.history_index = -1;
            self.query.clear();
        }
    }

    /// Record a completed search
    pub fn record_search(&mut self, result_count: usize) {
        self.last_searched_query = self.query.clone();
        self.history.add(&self.query, result_count);
        // Auto-save history (ignore errors)
        let _ = self.history.save();
    }

    /// Fuzzy match a string against the filter query
    pub fn fuzzy_matches(&mut self, text: &str) -> Option<u32> {
        if self.filter_query.is_empty() {
            return Some(0);
        }

        let mut haystack_buf = Vec::new();
        let mut needle_buf = Vec::new();
        let haystack = Utf32Str::new(text, &mut haystack_buf);
        let needle = Utf32Str::new(&self.filter_query, &mut needle_buf);

        self.matcher.fuzzy_match(haystack, needle).map(|score| score as u32)
    }

    /// Filter and score a list of tracks
    pub fn filter_tracks(&mut self, tracks: &[Track]) -> Vec<(usize, Track, u32)> {
        if self.filter_query.is_empty() {
            return tracks
                .iter()
                .enumerate()
                .map(|(i, t)| (i, t.clone(), 0))
                .collect();
        }

        let mut results: Vec<_> = tracks
            .iter()
            .enumerate()
            .filter_map(|(i, track)| {
                let search_text = format!("{} {} {}", track.artist, track.title, track.album);
                self.fuzzy_matches(&search_text)
                    .map(|score| (i, track.clone(), score))
            })
            .collect();

        // Sort by score (higher is better)
        results.sort_by(|a, b| b.2.cmp(&a.2));
        results
    }

    /// Filter and score a list of albums
    pub fn filter_albums(&mut self, albums: &[Album]) -> Vec<(usize, Album, u32)> {
        if self.filter_query.is_empty() {
            return albums
                .iter()
                .enumerate()
                .map(|(i, a)| (i, a.clone(), 0))
                .collect();
        }

        let mut results: Vec<_> = albums
            .iter()
            .enumerate()
            .filter_map(|(i, album)| {
                let search_text = format!("{} {}", album.artist, album.title);
                self.fuzzy_matches(&search_text)
                    .map(|score| (i, album.clone(), score))
            })
            .collect();

        results.sort_by(|a, b| b.2.cmp(&a.2));
        results
    }

    /// Filter and score a list of artists
    pub fn filter_artists(&mut self, artists: &[Artist]) -> Vec<(usize, Artist, u32)> {
        if self.filter_query.is_empty() {
            return artists
                .iter()
                .enumerate()
                .map(|(i, a)| (i, a.clone(), 0))
                .collect();
        }

        let mut results: Vec<_> = artists
            .iter()
            .enumerate()
            .filter_map(|(i, artist)| {
                self.fuzzy_matches(&artist.name)
                    .map(|score| (i, artist.clone(), score))
            })
            .collect();

        results.sort_by(|a, b| b.2.cmp(&a.2));
        results
    }
}

/// Enhanced parallel search with timeout and progress tracking
// ParallelSearcher is prepared for multi-service parallel search
#[allow(dead_code)]
pub struct ParallelSearcher {
    timeout_secs: u64,
}

/// Search progress information
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SearchProgress {
    pub services_completed: usize,
    pub services_total: usize,
    pub tracks_found: usize,
    pub albums_found: usize,
    pub artists_found: usize,
    pub errors: Vec<(ServiceType, String)>,
}

#[allow(dead_code)]
impl ParallelSearcher {
    pub fn new(timeout_secs: u64) -> Self {
        Self { timeout_secs }
    }

    /// Search a single service with timeout
    async fn search_service<S: MusicService + ?Sized>(
        service: &mut S,
        query: &str,
        limit: usize,
        timeout_duration: Duration,
    ) -> Result<SearchResults, (ServiceType, String)> {
        let service_type = service.service_type();

        match timeout(timeout_duration, service.search(query, limit)).await {
            Ok(Ok(results)) => Ok(results),
            Ok(Err(e)) => Err((service_type, e.to_string())),
            Err(_) => Err((service_type, "Search timed out".to_string())),
        }
    }
}

/// Score and rank search results for relevance
pub struct ResultScorer;

impl ResultScorer {
    /// Score a track based on query match quality
    pub fn score_track(track: &Track, query: &str) -> u32 {
        let mut score = 0u32;
        let query_lower = query.to_lowercase();
        let title_lower = track.title.to_lowercase();
        let artist_lower = track.artist.to_lowercase();

        // Exact title match
        if title_lower == query_lower {
            score += 1000;
        }
        // Title starts with query
        else if title_lower.starts_with(&query_lower) {
            score += 500;
        }
        // Title contains query
        else if title_lower.contains(&query_lower) {
            score += 200;
        }

        // Artist exact match
        if artist_lower == query_lower {
            score += 800;
        }
        // Artist starts with query
        else if artist_lower.starts_with(&query_lower) {
            score += 400;
        }
        // Artist contains query
        else if artist_lower.contains(&query_lower) {
            score += 150;
        }

        // Bonus for popular services (Tidal typically has better metadata)
        match track.service {
            ServiceType::Tidal => score += 50,
            ServiceType::YouTube => score += 30,
            ServiceType::Bandcamp => score += 40,
        }

        // Penalize very short or very long tracks (likely not the main song)
        if track.duration_seconds < 60 {
            score = score.saturating_sub(100);
        } else if track.duration_seconds > 600 {
            score = score.saturating_sub(50);
        }

        score
    }

    /// Score an album based on query match quality
    pub fn score_album(album: &Album, query: &str) -> u32 {
        let mut score = 0u32;
        let query_lower = query.to_lowercase();
        let title_lower = album.title.to_lowercase();
        let artist_lower = album.artist.to_lowercase();

        // Exact title match
        if title_lower == query_lower {
            score += 1000;
        } else if title_lower.starts_with(&query_lower) {
            score += 500;
        } else if title_lower.contains(&query_lower) {
            score += 200;
        }

        // Artist match
        if artist_lower == query_lower {
            score += 800;
        } else if artist_lower.starts_with(&query_lower) {
            score += 400;
        } else if artist_lower.contains(&query_lower) {
            score += 150;
        }

        // Prefer albums with more tracks (likely full albums, not singles)
        if album.num_tracks >= 8 {
            score += 100;
        } else if album.num_tracks >= 4 {
            score += 50;
        }

        match album.service {
            ServiceType::Tidal => score += 50,
            ServiceType::YouTube => score += 20,
            ServiceType::Bandcamp => score += 40,
        }

        score
    }

    /// Score an artist based on query match quality
    pub fn score_artist(artist: &Artist, query: &str) -> u32 {
        let mut score = 0u32;
        let query_lower = query.to_lowercase();
        let name_lower = artist.name.to_lowercase();

        // Exact name match
        if name_lower == query_lower {
            score += 1000;
        } else if name_lower.starts_with(&query_lower) {
            score += 500;
        } else if name_lower.contains(&query_lower) {
            score += 200;
        }

        match artist.service {
            ServiceType::Tidal => score += 50,
            ServiceType::YouTube => score += 30,
            ServiceType::Bandcamp => score += 40,
        }

        score
    }

    /// Sort tracks by score
    pub fn sort_tracks(tracks: &mut [Track], query: &str) {
        tracks.sort_by(|a, b| {
            let score_a = Self::score_track(a, query);
            let score_b = Self::score_track(b, query);
            score_b.cmp(&score_a) // Descending
        });
    }

    /// Sort albums by score
    pub fn sort_albums(albums: &mut [Album], query: &str) {
        albums.sort_by(|a, b| {
            let score_a = Self::score_album(a, query);
            let score_b = Self::score_album(b, query);
            score_b.cmp(&score_a)
        });
    }

    /// Sort artists by score
    pub fn sort_artists(artists: &mut [Artist], query: &str) {
        artists.sort_by(|a, b| {
            let score_a = Self::score_artist(a, query);
            let score_b = Self::score_artist(b, query);
            score_b.cmp(&score_a)
        });
    }

    /// Score and sort all results
    pub fn score_results(results: &mut SearchResults, query: &str) {
        Self::sort_tracks(&mut results.tracks, query);
        Self::sort_albums(&mut results.albums, query);
        Self::sort_artists(&mut results.artists, query);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_history_add_and_dedupe() {
        let mut history = SearchHistory::new(5);
        history.add("test query", 10);
        history.add("another query", 20);
        history.add("test query", 15); // Duplicate should move to front

        assert_eq!(history.entries.len(), 2);
        assert_eq!(history.entries[0].query, "test query");
        assert_eq!(history.entries[0].result_count, 15);
    }

    #[test]
    fn test_search_history_max_size() {
        let mut history = SearchHistory::new(3);
        for i in 0..5 {
            history.add(&format!("query {}", i), i);
        }

        assert_eq!(history.entries.len(), 3);
        assert_eq!(history.entries[0].query, "query 4"); // Most recent
    }

    #[test]
    fn test_search_history_suggestions() {
        let mut history = SearchHistory::new(10);
        history.add("taylor swift", 100);
        history.add("taylor dayne", 50);
        history.add("beatles", 75);

        let suggestions = history.get_suggestions("taylor");
        assert_eq!(suggestions.len(), 2);
        assert!(suggestions.contains(&"taylor swift"));
        assert!(suggestions.contains(&"taylor dayne"));
    }

    #[test]
    fn test_result_scorer_track() {
        let track = Track {
            id: "1".to_string(),
            title: "Anti-Hero".to_string(),
            artist: "Taylor Swift".to_string(),
            album: "Midnights".to_string(),
            duration_seconds: 200,
            cover_art: crate::service::CoverArt::None,
            service: ServiceType::Tidal,
        };

        let score1 = ResultScorer::score_track(&track, "Anti-Hero");
        let score2 = ResultScorer::score_track(&track, "Taylor Swift");
        let score3 = ResultScorer::score_track(&track, "random query");

        assert!(score1 > score3);
        assert!(score2 > score3);
    }

    #[test]
    fn test_result_scorer_sorts_correctly() {
        let mut tracks = vec![
            Track {
                id: "1".to_string(),
                title: "Random Song".to_string(),
                artist: "Unknown".to_string(),
                album: "Album".to_string(),
                duration_seconds: 200,
                cover_art: crate::service::CoverArt::None,
                service: ServiceType::YouTube,
            },
            Track {
                id: "2".to_string(),
                title: "Anti-Hero".to_string(),
                artist: "Taylor Swift".to_string(),
                album: "Midnights".to_string(),
                duration_seconds: 200,
                cover_art: crate::service::CoverArt::None,
                service: ServiceType::Tidal,
            },
        ];

        ResultScorer::sort_tracks(&mut tracks, "Anti-Hero");
        assert_eq!(tracks[0].title, "Anti-Hero");
    }
}
