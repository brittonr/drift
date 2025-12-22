use ratatui::layout::Rect;

use crate::tidal::{Album, Artist, Track};
use crate::ui::{LibraryTab, SearchTab};

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
