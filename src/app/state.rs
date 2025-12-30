use ratatui::layout::Rect;
use std::time::Instant;

use crate::service::{Album, Artist, ServiceType, Track};
use crate::ui::{LibraryTab, SearchTab};

/// Status message for display in the status bar
pub struct StatusMessage {
    pub message: String,
    pub is_error: bool,
    pub timestamp: Instant,
}

#[derive(Clone)]
pub enum RadioSeed {
    Track(String),
    Playlist(String),
    Artist(String),
    Album(String),
}

#[derive(PartialEq, Clone, Copy)]
pub enum ViewMode {
    Browse,
    Search,
    Downloads,
    Library,
    ArtistDetail,
    AlbumDetail,
}

#[derive(Default, Clone, Copy)]
pub struct ClickableAreas {
    pub left_list: Option<Rect>,
    pub right_list: Option<Rect>,
    pub queue_list: Option<Rect>,
    pub progress_bar: Option<Rect>,
}

/// Browse mode state
#[derive(Default)]
pub struct BrowseState {
    pub selected_playlist: usize,
    pub selected_track: usize,
    pub selected_tab: usize,
}

/// Search mode state
#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub tab: SearchTab,
    pub selected_track: usize,
    pub selected_album: usize,
    pub selected_artist: usize,
    pub is_active: bool,
    /// Filter query for fuzzy filtering current results
    pub filter_query: String,
    /// Is filter mode active (Ctrl+F to toggle)
    pub filter_active: bool,
    /// History navigation index (-1 = none)
    pub history_index: i32,
    /// Show history suggestions popup
    pub show_suggestions: bool,
    /// Current page for pagination (0-indexed)
    pub page: usize,
    /// Whether more results are available
    pub has_more: bool,
    /// Service filter (None = all services, Some = specific service)
    pub service_filter: Option<ServiceType>,
    /// Show preview panel with album art (default: true)
    pub show_preview: bool,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            show_preview: true,
            ..Default::default()
        }
    }
}

impl Default for SearchTab {
    fn default() -> Self {
        SearchTab::Tracks
    }
}

/// Library/Favorites state
#[derive(Default)]
pub struct LibraryState {
    pub tab: LibraryTab,
    pub selected_track: usize,
    pub selected_album: usize,
    pub selected_artist: usize,
    pub selected_history: usize,
    pub loaded: bool,
}

impl Default for LibraryTab {
    fn default() -> Self {
        LibraryTab::Tracks
    }
}

/// Downloads state
#[derive(Default)]
pub struct DownloadsState {
    pub selected: usize,
    pub offline_mode: bool,
}

/// Playback state
pub struct PlaybackState {
    pub is_playing: bool,
    pub volume: u8,
    pub repeat_mode: bool,
    pub random_mode: bool,
    pub single_mode: bool,
    pub selected_queue_item: usize,
    pub show_queue: bool,
    pub queue_dirty: bool,
    pub radio_seed: Option<RadioSeed>,
    pub radio_fetching: bool,
    /// Video mode enabled (YouTube content plays in mpv window)
    pub video_mode: bool,
}

impl PlaybackState {
    pub fn radio_mode(&self) -> bool {
        self.radio_seed.is_some()
    }
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            is_playing: false,
            volume: 80,
            repeat_mode: false,
            random_mode: false,
            single_mode: false,
            selected_queue_item: 0,
            show_queue: false,
            queue_dirty: false,
            radio_seed: None,
            radio_fetching: false,
            video_mode: false,
        }
    }
}

/// Helix-style key command state
#[derive(Default)]
pub struct KeyState {
    pub pending_key: Option<char>,
    pub space_pressed: bool,
}

/// Artist detail view state
#[derive(Default)]
pub struct ArtistDetailState {
    pub artist: Option<Artist>,
    pub top_tracks: Vec<Track>,
    pub albums: Vec<Album>,
    pub selected_track: usize,
    pub selected_album: usize,
    pub selected_panel: usize, // 0 = top tracks, 1 = albums
}

/// Album detail view state
#[derive(Default)]
pub struct AlbumDetailState {
    pub album: Option<Album>,
    pub tracks: Vec<Track>,
    pub selected_track: usize,
}

/// Help panel state
#[derive(Default)]
pub struct HelpState {
    pub scroll_offset: usize,
}

/// Dialog mode for text input and playlist selection
#[derive(Clone, PartialEq)]
pub enum DialogMode {
    /// Not showing any dialog
    None,
    /// Creating a new playlist - text input for name
    CreatePlaylist,
    /// Adding a track to a playlist - selecting which playlist
    AddToPlaylist {
        track_id: String,
        track_title: String,
    },
    /// Renaming a playlist - text input for new name
    RenamePlaylist {
        playlist_id: String,
        playlist_title: String,
    },
    /// Confirming playlist deletion
    ConfirmDeletePlaylist {
        playlist_id: String,
        playlist_title: String,
    },
}

impl Default for DialogMode {
    fn default() -> Self {
        DialogMode::None
    }
}

/// State for dialog inputs
#[derive(Default)]
pub struct DialogState {
    pub mode: DialogMode,
    pub input_text: String,
    pub selected_index: usize,
}
