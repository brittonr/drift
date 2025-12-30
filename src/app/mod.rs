pub mod state;
mod playback;
mod navigation;
mod downloads;
mod queue;

use std::collections::VecDeque;

use anyhow::Result;
use ratatui::layout::Rect;

use crate::album_art::AlbumArtCache;
use crate::cava::CavaVisualizer;
use crate::config::Config;
use crate::download_db::DownloadRecord;
use crate::history_db::{HistoryDb, HistoryEntry};
use crate::mpd::{CurrentSong, MpdController, QueueItem};
use crate::queue_persistence::{self, PersistedQueue};
use crate::search::{ResultScorer, SearchHistory};
use crate::service::{Album, Artist, CoverArt, MixedPlaylistStorage, MultiServiceManager, MusicService, Playlist, SearchResults, Track};
use crate::downloads::{DownloadEvent, DownloadManager};
use crate::video::MpvController;

pub use state::{
    AlbumDetailState, ArtistDetailState, BrowseState, ClickableAreas, DialogMode, DialogState,
    DownloadsState, HelpState, KeyState, LibraryState, PlaybackState, SearchState, StatusMessage,
    ViewMode,
};

pub struct App {
    // View state
    pub view_mode: ViewMode,

    // Browse mode data
    pub playlists: Vec<Playlist>,
    pub tracks: Vec<Track>,
    pub browse: BrowseState,

    // Search mode data
    pub search: SearchState,
    pub search_results: Option<SearchResults>,
    pub search_history: SearchHistory,

    // Playback state
    pub playback: PlaybackState,
    pub current_track: Option<Track>,
    pub current_song: Option<CurrentSong>,
    pub queue: Vec<QueueItem>,
    pub local_queue: Vec<Track>,

    // Core components
    pub music_service: MultiServiceManager,
    pub mixed_playlists: MixedPlaylistStorage,
    pub mpd_controller: MpdController,
    pub debug_log: VecDeque<String>,
    pub visualizer: Option<CavaVisualizer>,
    pub show_visualizer: bool,
    pub album_art_cache: AlbumArtCache,

    // Helix-style key command state
    pub key_state: KeyState,

    // Queue persistence
    pub pending_restore: Option<PersistedQueue>,

    // Mouse support
    pub clickable_areas: ClickableAreas,

    // Downloads
    pub download_manager: Option<DownloadManager>,
    pub download_event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<DownloadEvent>>,
    pub download_records: Vec<DownloadRecord>,
    pub downloads: DownloadsState,

    // Library/Favorites
    pub library: LibraryState,
    pub favorite_tracks: Vec<Track>,
    pub favorite_albums: Vec<Album>,
    pub favorite_artists: Vec<Artist>,

    // Artist/Album detail views
    pub artist_detail: ArtistDetailState,
    pub album_detail: AlbumDetailState,
    pub navigation_history: Vec<ViewMode>,

    // Playback history
    pub history_db: Option<HistoryDb>,
    pub history_entries: Vec<HistoryEntry>,

    // Configuration
    pub config: Config,

    // Help panel
    pub show_help: bool,
    pub help: HelpState,

    // Debug log visibility (hidden by default)
    pub show_debug: bool,

    // Playlist dialogs
    pub dialog: DialogState,

    // Status bar message (for displaying errors/info)
    pub status_message: Option<StatusMessage>,

    // Video playback controller (for YouTube video mode)
    pub video_controller: Option<MpvController>,
}

impl App {
    pub async fn new() -> Result<Self> {
        let mut debug_log = VecDeque::new();
        debug_log.push_back("Starting Tidal TUI...".to_string());

        // Load configuration
        let config = match Config::load() {
            Ok(cfg) => {
                debug_log.push_back("Configuration loaded".to_string());
                debug_log.push_back(format!("  MPD: {}:{}", cfg.mpd.host, cfg.mpd.port));
                debug_log.push_back(format!("  Audio quality: {}", cfg.playback.audio_quality));
                cfg
            }
            Err(e) => {
                debug_log.push_back(format!("Failed to load config: {}, using defaults", e));
                Config::default()
            }
        };

        // Initialize multi-service manager
        debug_log.push_back("Initializing music services...".to_string());
        let mut music_service = MultiServiceManager::new(&config).await?;
        music_service.set_audio_quality(&config.playback.audio_quality);

        // Log enabled services
        for service in music_service.enabled_services() {
            debug_log.push_back(format!("  {} enabled", service));
        }

        // Log any initialization errors
        for (service, error) in music_service.init_errors() {
            debug_log.push_back(format!("  {} unavailable: {}", service, error));
        }

        debug_log.push_back(format!("Primary service: {}", music_service.primary_service()));

        // Load mixed playlist storage
        let mixed_playlists = MixedPlaylistStorage::load().unwrap_or_default();
        if !mixed_playlists.playlists.is_empty() {
            debug_log.push_back(format!("Loaded {} mixed playlists", mixed_playlists.playlists.len()));
        }

        // Initialize MPD controller with config
        debug_log.push_back("Connecting to MPD...".to_string());
        let mpd_controller = MpdController::with_config(
            &config.mpd.host,
            config.mpd.port,
            &mut debug_log
        ).await?;

        // Load initial playlists
        debug_log.push_back("Fetching playlists...".to_string());
        let playlists = music_service.get_playlists().await?;
        debug_log.push_back(format!("Loaded {} playlists", playlists.len()));

        // Load tracks from first playlist if available
        let tracks = if !playlists.is_empty() {
            debug_log.push_back(format!("Loading tracks from '{}'...", playlists[0].title));
            let tracks = music_service.get_playlist_tracks(&playlists[0].id).await?;
            debug_log.push_back(format!("Loaded {} tracks", tracks.len()));
            tracks
        } else {
            Vec::new()
        };

        // Try to initialize visualizer
        let visualizer = if config.ui.show_visualizer {
            match CavaVisualizer::new() {
                Ok(mut v) => {
                    debug_log.push_back("Visualizer initialized".to_string());
                    match v.start() {
                        Ok(_) => {
                            debug_log.push_back("Cava process started".to_string());
                            Some(v)
                        }
                        Err(e) => {
                            debug_log.push_back(format!("Could not start cava: {}", e));
                            None
                        }
                    }
                }
                Err(e) => {
                    debug_log.push_back(format!("Could not initialize visualizer: {}", e));
                    None
                }
            }
        } else {
            debug_log.push_back("Visualizer disabled in config".to_string());
            None
        };

        // Initialize album art cache
        let album_art_cache = AlbumArtCache::new()?;
        debug_log.push_back("Album art cache initialized".to_string());

        // Load persisted queue
        let (local_queue, pending_restore) = if config.playback.resume_on_startup {
            match queue_persistence::load_queue() {
                Ok(Some(persisted)) => {
                    debug_log.push_back(format!("Found {} tracks in saved queue", persisted.tracks.len()));
                    let tracks: Vec<Track> = persisted.tracks.iter().map(Track::from).collect();
                    (tracks, Some(persisted))
                }
                Ok(None) => {
                    debug_log.push_back("No saved queue found".to_string());
                    (Vec::new(), None)
                }
                Err(e) => {
                    debug_log.push_back(format!("Failed to load queue: {}", e));
                    (Vec::new(), None)
                }
            }
        } else {
            debug_log.push_back("Queue resume disabled in config".to_string());
            (Vec::new(), None)
        };

        // Initialize download manager
        let (download_manager, download_event_rx, download_records) =
            match DownloadManager::with_config(&config.downloads) {
                Ok((dm, rx)) => {
                    let records = dm.get_all_downloads().unwrap_or_default();
                    debug_log.push_back(format!("Download manager initialized ({} downloads, max {})",
                        records.len(), config.downloads.max_concurrent));
                    (Some(dm), Some(rx), records)
                }
                Err(e) => {
                    debug_log.push_back(format!("Could not initialize downloads: {}", e));
                    (None, None, Vec::new())
                }
            };

        // Initialize playback history database
        let (history_db, history_entries) = match HistoryDb::new() {
            Ok(db) => {
                let entries = db.get_recent(100).unwrap_or_default();
                debug_log.push_back(format!("History database initialized ({} entries)", entries.len()));
                (Some(db), entries)
            }
            Err(e) => {
                debug_log.push_back(format!("Could not initialize history: {}", e));
                (None, Vec::new())
            }
        };

        // Initialize video controller if mpv is available
        let video_controller = if MpvController::is_available() {
            debug_log.push_back("mpv found - video mode available (press 'V' to toggle)".to_string());
            Some(MpvController::new(&config.video))
        } else {
            debug_log.push_back("mpv not found - video mode disabled".to_string());
            None
        };

        let default_volume = config.playback.default_volume;
        let show_visualizer = config.ui.show_visualizer;

        // Load search history
        let search_history = SearchHistory::load(config.search.history_size);
        if !search_history.entries.is_empty() {
            debug_log.push_back(format!("Loaded {} search history entries", search_history.entries.len()));
        }

        Ok(Self {
            view_mode: ViewMode::Browse,
            playlists,
            tracks,
            browse: BrowseState::default(),
            search: SearchState::new(),
            search_results: None,
            search_history,
            playback: PlaybackState {
                volume: default_volume,
                ..Default::default()
            },
            current_track: None,
            current_song: None,
            queue: Vec::new(),
            local_queue,
            music_service,
            mixed_playlists,
            mpd_controller,
            debug_log,
            visualizer,
            show_visualizer,
            album_art_cache,
            key_state: KeyState::default(),
            pending_restore,
            clickable_areas: ClickableAreas::default(),
            download_manager,
            download_event_rx,
            download_records,
            downloads: DownloadsState::default(),
            library: LibraryState::default(),
            favorite_tracks: Vec::new(),
            favorite_albums: Vec::new(),
            favorite_artists: Vec::new(),
            artist_detail: ArtistDetailState::default(),
            album_detail: AlbumDetailState::default(),
            navigation_history: Vec::new(),
            history_db,
            history_entries,
            config,
            show_help: false,
            help: HelpState::default(),
            show_debug: false,
            dialog: DialogState::default(),
            status_message: None,
            video_controller,
        })
    }

    pub fn add_debug(&mut self, msg: String) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/drift-debug.log")
        {
            use std::io::Write;
            let timestamp = chrono::Local::now().format("%H:%M:%S");
            writeln!(file, "[{}] {}", timestamp, msg).ok();
        }

        self.debug_log.push_back(msg);
        while self.debug_log.len() > 100 {
            self.debug_log.pop_front();
        }
    }

    pub fn set_status_error(&mut self, msg: String) {
        self.status_message = Some(StatusMessage {
            message: msg.clone(),
            is_error: true,
            timestamp: std::time::Instant::now(),
        });
        self.add_debug(msg);
    }

    pub fn set_status_info(&mut self, msg: String) {
        self.status_message = Some(StatusMessage {
            message: msg,
            is_error: false,
            timestamp: std::time::Instant::now(),
        });
    }

    pub fn clear_expired_status(&mut self) {
        if let Some(ref msg) = self.status_message {
            if msg.timestamp.elapsed() > std::time::Duration::from_secs(5) {
                self.status_message = None;
            }
        }
    }

    pub fn update_clickable_areas(
        &mut self,
        left: Option<Rect>,
        right: Option<Rect>,
        queue: Option<Rect>,
        progress: Option<Rect>,
    ) {
        self.clickable_areas.left_list = left;
        self.clickable_areas.right_list = right;
        self.clickable_areas.queue_list = queue;
        self.clickable_areas.progress_bar = progress;
    }

    pub async fn load_playlist(&mut self, index: usize) -> Result<()> {
        if index < self.playlists.len() {
            let playlist_title = self.playlists[index].title.clone();
            let playlist_id = self.playlists[index].id.clone();
            self.add_debug(format!("Loading playlist: {}", playlist_title));

            self.tracks = self.music_service.get_playlist_tracks(&playlist_id).await?;
            self.browse.selected_track = 0;

            self.add_debug(format!("Loaded {} tracks", self.tracks.len()));
        }
        Ok(())
    }

    pub async fn search(&mut self) -> Result<()> {
        if self.search.query.trim().is_empty() {
            self.add_debug("Search query is empty".to_string());
            return Ok(());
        }

        let query = self.search.query.clone();
        let max_results = self.config.search.max_results;
        let page = self.search.page;

        self.add_debug(format!("Searching for: {} (page {}, limit {})", query, page + 1, max_results));
        self.search.is_active = true;

        match self.music_service.search(&query, max_results).await {
            Ok(mut results) => {
                let track_count = results.tracks.len();
                let album_count = results.albums.len();
                let artist_count = results.artists.len();
                let total_count = track_count + album_count + artist_count;

                // Score and sort results for relevance
                ResultScorer::score_results(&mut results, &query);

                self.add_debug(format!("Found {} tracks, {} albums, {} artists (scored & sorted)",
                    track_count, album_count, artist_count));

                // Check if more results might be available (heuristic)
                self.search.has_more = track_count >= max_results
                    || album_count >= max_results
                    || artist_count >= max_results;

                self.search_results = Some(results);
                self.search.selected_track = 0;
                self.search.selected_album = 0;
                self.search.selected_artist = 0;

                // Record search in history
                self.search_history.add(&query, total_count);
                let _ = self.search_history.save();
            }
            Err(e) => {
                self.add_debug(format!("Search failed: {}", e));
                self.set_status_error(format!("Search failed: {}", e));
            }
        }

        self.search.is_active = false;
        Ok(())
    }

    pub async fn load_favorites(&mut self) {
        self.add_debug("Loading favorites from Tidal...".to_string());

        match self.music_service.get_favorite_tracks().await {
            Ok(tracks) => {
                let count = tracks.len();
                self.favorite_tracks = tracks;
                self.add_debug(format!("Loaded {} favorite tracks", count));
            }
            Err(e) => {
                self.add_debug(format!("Failed to load favorite tracks: {}", e));
            }
        }

        match self.music_service.get_favorite_albums().await {
            Ok(albums) => {
                let count = albums.len();
                self.favorite_albums = albums;
                self.add_debug(format!("Loaded {} favorite albums", count));
            }
            Err(e) => {
                self.add_debug(format!("Failed to load favorite albums: {}", e));
            }
        }

        match self.music_service.get_favorite_artists().await {
            Ok(artists) => {
                let count = artists.len();
                self.favorite_artists = artists;
                self.add_debug(format!("Loaded {} favorite artists", count));
            }
            Err(e) => {
                self.add_debug(format!("Failed to load favorite artists: {}", e));
            }
        }

        self.library.loaded = true;
        self.library.selected_track = 0;
        self.library.selected_album = 0;
        self.library.selected_artist = 0;
    }

    pub fn is_playlist_synced(&self, playlist_id: &str) -> bool {
        if let Some(ref dm) = self.download_manager {
            dm.is_playlist_synced(playlist_id)
        } else {
            false
        }
    }

    pub fn get_synced_playlist_ids(&self) -> std::collections::HashSet<String> {
        let mut synced = std::collections::HashSet::new();
        if let Some(ref dm) = self.download_manager {
            if let Ok(playlists) = dm.get_synced_playlists() {
                for playlist in playlists {
                    synced.insert(playlist.playlist_id);
                }
            }
        }
        synced
    }

    pub async fn remove_favorite_track(&mut self, index: usize) {
        if index >= self.favorite_tracks.len() {
            return;
        }

        let track = self.favorite_tracks[index].clone();
        self.add_debug(format!("Removing from favorites: {}", track.title));

        match self.music_service.remove_favorite_track(&track.id).await {
            Ok(()) => {
                self.add_debug(format!("Removed '{}' from favorites", track.title));
                self.favorite_tracks.remove(index);
                if self.library.selected_track > 0 && self.library.selected_track >= self.favorite_tracks.len() {
                    self.library.selected_track = self.favorite_tracks.len().saturating_sub(1);
                }
            }
            Err(e) => {
                self.add_debug(format!("Failed to remove from favorites: {}", e));
            }
        }
    }

    pub async fn add_favorite_track(&mut self, track: Track) {
        self.add_debug(format!("Adding to favorites: {}", track.title));

        match self.music_service.add_favorite_track(&track.id).await {
            Ok(()) => {
                self.add_debug(format!("Added '{}' to favorites", track.title));
            }
            Err(e) => {
                self.add_debug(format!("Failed to add to favorites: {}", e));
            }
        }
    }

    pub fn get_selected_track(&self) -> Option<Track> {
        match self.view_mode {
            ViewMode::Browse => {
                if self.browse.selected_tab == 1 && self.browse.selected_track < self.tracks.len() {
                    Some(self.tracks[self.browse.selected_track].clone())
                } else {
                    None
                }
            }
            ViewMode::Search => {
                if let Some(ref results) = self.search_results {
                    if self.search.tab == crate::ui::SearchTab::Tracks
                        && self.search.selected_track < results.tracks.len()
                    {
                        Some(results.tracks[self.search.selected_track].clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Push current view to navigation history and switch to new view
    pub fn push_view(&mut self, new_mode: ViewMode) {
        self.navigation_history.push(self.view_mode);
        self.view_mode = new_mode;
    }

    /// Pop from navigation history and restore previous view
    pub fn pop_view(&mut self) {
        if let Some(previous) = self.navigation_history.pop() {
            self.view_mode = previous;
        }
    }

    /// Load artist detail data
    pub async fn load_artist_detail(&mut self, artist: Artist) {
        self.artist_detail.artist = Some(artist.clone());
        self.artist_detail.selected_track = 0;
        self.artist_detail.selected_album = 0;
        self.artist_detail.selected_panel = 0;
        self.artist_detail.top_tracks.clear();
        self.artist_detail.albums.clear();

        self.add_debug(format!("Loading artist: {}", artist.name));

        // Fetch top tracks
        match self.music_service.get_artist_top_tracks(&artist.id).await {
            Ok(tracks) => {
                self.add_debug(format!("Loaded {} top tracks", tracks.len()));
                self.artist_detail.top_tracks = tracks;
            }
            Err(e) => {
                self.add_debug(format!("Failed to load top tracks: {}", e));
            }
        }

        // Fetch albums
        match self.music_service.get_artist_albums(&artist.id).await {
            Ok(albums) => {
                self.add_debug(format!("Loaded {} albums", albums.len()));
                self.artist_detail.albums = albums;
            }
            Err(e) => {
                self.add_debug(format!("Failed to load albums: {}", e));
            }
        }
    }

    /// Load album detail data
    pub async fn load_album_detail(&mut self, album: Album) {
        self.album_detail.album = Some(album.clone());
        self.album_detail.selected_track = 0;
        self.album_detail.tracks.clear();

        self.add_debug(format!("Loading album: {} - {}", album.artist, album.title));

        match self.music_service.get_album_tracks(&album.id).await {
            Ok(tracks) => {
                self.add_debug(format!("Loaded {} tracks", tracks.len()));
                self.album_detail.tracks = tracks;
            }
            Err(e) => {
                self.add_debug(format!("Failed to load album tracks: {}", e));
            }
        }
    }

    /// Record a track to playback history
    pub fn record_history(&mut self, track: &Track) {
        if let Some(ref db) = self.history_db {
            match db.record_play(track) {
                Ok(()) => {
                    // Refresh the cached list
                    self.history_entries = db.get_recent(100).unwrap_or_default();
                }
                Err(e) => {
                    self.add_debug(format!("Failed to record history: {}", e));
                }
            }
        }
    }

    // ========== Playlist Management ==========

    /// Open the "Create Playlist" dialog
    pub fn open_create_playlist_dialog(&mut self) {
        self.dialog.mode = DialogMode::CreatePlaylist;
        self.dialog.input_text.clear();
        self.add_debug("Create playlist dialog opened".to_string());
    }

    /// Open the "Add to Playlist" dialog for a given track
    pub fn open_add_to_playlist_dialog(&mut self, track: &Track) {
        self.dialog.mode = DialogMode::AddToPlaylist {
            track_id: track.id.clone(),
            track_title: track.title.clone(),
        };
        self.dialog.selected_index = 0;
        self.add_debug(format!("Add to playlist dialog for: {}", track.title));
    }

    /// Open the "Rename Playlist" dialog
    pub fn open_rename_playlist_dialog(&mut self, playlist: &Playlist) {
        self.dialog.mode = DialogMode::RenamePlaylist {
            playlist_id: playlist.id.clone(),
            playlist_title: playlist.title.clone(),
        };
        self.dialog.input_text = playlist.title.clone();
        self.add_debug(format!("Rename playlist dialog for: {}", playlist.title));
    }

    /// Open the "Delete Playlist" confirmation dialog
    pub fn open_delete_playlist_dialog(&mut self, playlist: &Playlist) {
        self.dialog.mode = DialogMode::ConfirmDeletePlaylist {
            playlist_id: playlist.id.clone(),
            playlist_title: playlist.title.clone(),
        };
        self.add_debug(format!("Delete playlist dialog for: {}", playlist.title));
    }

    /// Close any open dialog
    pub fn close_dialog(&mut self) {
        self.dialog.mode = DialogMode::None;
        self.dialog.input_text.clear();
        self.dialog.selected_index = 0;
    }

    /// Check if any dialog is open
    pub fn is_dialog_open(&self) -> bool {
        self.dialog.mode != DialogMode::None
    }

    /// Create a new playlist with the current input text
    pub async fn create_playlist_from_dialog(&mut self) {
        let name = self.dialog.input_text.trim().to_string();
        if name.is_empty() {
            self.add_debug("Playlist name cannot be empty".to_string());
            return;
        }

        self.add_debug(format!("Creating playlist: {}", name));

        match self.music_service.create_playlist(&name, None).await {
            Ok(playlist) => {
                self.add_debug(format!("Created playlist: {}", playlist.title));
                self.playlists.insert(0, playlist);
                self.close_dialog();
            }
            Err(e) => {
                self.add_debug(format!("Failed to create playlist: {}", e));
            }
        }
    }

    /// Add a track to the selected playlist
    pub async fn add_track_to_playlist_from_dialog(&mut self) {
        let (track_id, playlist_id) = match &self.dialog.mode {
            DialogMode::AddToPlaylist { track_id, .. } => {
                if self.dialog.selected_index < self.playlists.len() {
                    (track_id.clone(), self.playlists[self.dialog.selected_index].id.clone())
                } else {
                    self.add_debug("No playlist selected".to_string());
                    return;
                }
            }
            _ => return,
        };

        let playlist_title = self.playlists[self.dialog.selected_index].title.clone();
        self.add_debug(format!("Adding track to playlist: {}", playlist_title));

        match self.music_service.add_tracks_to_playlist(&playlist_id, &[track_id]).await {
            Ok(()) => {
                self.add_debug(format!("Added track to '{}'", playlist_title));
                // Update track count in local state
                if let Some(playlist) = self.playlists.iter_mut().find(|p| p.id == playlist_id) {
                    playlist.num_tracks += 1;
                }
                self.close_dialog();
            }
            Err(e) => {
                self.add_debug(format!("Failed to add track: {}", e));
            }
        }
    }

    /// Rename the playlist with the current input text
    pub async fn rename_playlist_from_dialog(&mut self) {
        let (playlist_id, new_title) = match &self.dialog.mode {
            DialogMode::RenamePlaylist { playlist_id, .. } => {
                let title = self.dialog.input_text.trim().to_string();
                if title.is_empty() {
                    self.add_debug("Playlist name cannot be empty".to_string());
                    return;
                }
                (playlist_id.clone(), title)
            }
            _ => return,
        };

        self.add_debug(format!("Renaming playlist to: {}", new_title));

        match self.music_service.update_playlist(&playlist_id, Some(&new_title), None).await {
            Ok(()) => {
                self.add_debug(format!("Renamed playlist to: {}", new_title));
                // Update local state
                if let Some(playlist) = self.playlists.iter_mut().find(|p| p.id == playlist_id) {
                    playlist.title = new_title;
                }
                self.close_dialog();
            }
            Err(e) => {
                self.add_debug(format!("Failed to rename playlist: {}", e));
            }
        }
    }

    /// Delete the playlist after confirmation
    pub async fn delete_playlist_from_dialog(&mut self) {
        let (playlist_id, playlist_title) = match &self.dialog.mode {
            DialogMode::ConfirmDeletePlaylist { playlist_id, playlist_title } => {
                (playlist_id.clone(), playlist_title.clone())
            }
            _ => return,
        };

        self.add_debug(format!("Deleting playlist: {}", playlist_title));

        match self.music_service.delete_playlist(&playlist_id).await {
            Ok(()) => {
                self.add_debug("Playlist deleted".to_string());
                // Remove from local state
                self.playlists.retain(|p| p.id != playlist_id);
                // Reset selection if needed
                if self.browse.selected_playlist >= self.playlists.len() && !self.playlists.is_empty() {
                    self.browse.selected_playlist = self.playlists.len() - 1;
                }
                self.close_dialog();
            }
            Err(e) => {
                self.add_debug(format!("Failed to delete playlist: {}", e));
            }
        }
    }

    /// Remove a track from the current playlist (when viewing tracks in browse mode)
    pub async fn remove_track_from_current_playlist(&mut self) {
        if self.view_mode != ViewMode::Browse || self.browse.selected_tab != 1 {
            return;
        }

        if self.browse.selected_playlist >= self.playlists.len() {
            return;
        }

        let playlist = &self.playlists[self.browse.selected_playlist];
        if playlist.id.starts_with("demo-") {
            self.add_debug("Cannot modify demo playlists".to_string());
            return;
        }

        if self.browse.selected_track >= self.tracks.len() {
            return;
        }

        let playlist_id = playlist.id.clone();
        let track_index = self.browse.selected_track;
        let track_title = self.tracks[track_index].title.clone();

        self.add_debug(format!("Removing '{}' from playlist", track_title));

        match self.music_service.remove_tracks_from_playlist(&playlist_id, &[track_index]).await {
            Ok(()) => {
                self.add_debug(format!("Removed '{}' from playlist", track_title));
                // Remove from local state
                self.tracks.remove(track_index);
                // Update playlist track count
                if let Some(p) = self.playlists.iter_mut().find(|p| p.id == playlist_id) {
                    p.num_tracks = p.num_tracks.saturating_sub(1);
                }
                // Adjust selection if needed
                if self.browse.selected_track >= self.tracks.len() && !self.tracks.is_empty() {
                    self.browse.selected_track = self.tracks.len() - 1;
                }
            }
            Err(e) => {
                self.add_debug(format!("Failed to remove track: {}", e));
            }
        }
    }

    /// Prefetch album art for the currently selected search result
    pub async fn prefetch_search_preview_art(&mut self) {
        // Only prefetch when preview is enabled and in search mode
        if !self.search.show_preview || self.view_mode != ViewMode::Search {
            return;
        }

        // Get the cover art for the selected item
        let cover_art = match self.search.tab {
            crate::ui::SearchTab::Tracks => {
                self.search_results.as_ref().and_then(|r| {
                    // Apply service filter
                    let filtered: Vec<_> = r.tracks.iter()
                        .filter(|t| self.search.service_filter.map_or(true, |s| t.service == s))
                        .collect();
                    filtered.get(self.search.selected_track).map(|t| t.cover_art.clone())
                })
            }
            crate::ui::SearchTab::Albums => {
                self.search_results.as_ref().and_then(|r| {
                    let filtered: Vec<_> = r.albums.iter()
                        .filter(|a| self.search.service_filter.map_or(true, |s| a.service == s))
                        .collect();
                    filtered.get(self.search.selected_album).map(|a| a.cover_art.clone())
                })
            }
            crate::ui::SearchTab::Artists => {
                // Artists don't have cover art in our model
                None
            }
        };

        // Load the cover art if not cached
        if let Some(cover) = cover_art {
            match &cover {
                CoverArt::ServiceId { id, .. } => {
                    if !self.album_art_cache.has_cached(id, 320) {
                        if let Err(e) = self.album_art_cache.get_album_art(id, 320).await {
                            self.add_debug(format!("Preview art load failed: {}", e));
                        }
                    }
                }
                CoverArt::Url(url) => {
                    if !self.album_art_cache.has_url_cached(url, 320) {
                        if let Err(e) = self.album_art_cache.get_album_art_from_url(url, 320).await {
                            self.add_debug(format!("Preview art load failed: {}", e));
                        }
                    }
                }
                CoverArt::None => {}
            }
        }
    }
}
