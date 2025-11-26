mod tidal;
mod mpd;
mod cava;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::{io, time::Duration, collections::VecDeque};
use tidal::{TidalClient, Playlist, Track, SearchResults};
use mpd::MpdController;
use cava::CavaVisualizer;

#[derive(PartialEq, Clone, Copy)]
enum ViewMode {
    Browse,
    Search,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum SearchTab {
    Tracks,
    Albums,
    Artists,
}

struct App {
    // View state
    view_mode: ViewMode,
    selected_tab: usize,

    // Browse mode data
    playlists: Vec<Playlist>,
    tracks: Vec<Track>,
    selected_playlist: usize,
    selected_track: usize,

    // Search mode data
    search_query: String,
    search_results: Option<SearchResults>,
    search_tab: SearchTab,
    selected_search_track: usize,
    selected_search_album: usize,
    selected_search_artist: usize,
    is_searching: bool,

    // Playback state
    is_playing: bool,
    current_track: Option<Track>,
    current_song: Option<mpd::CurrentSong>,
    queue: Vec<mpd::QueueItem>,
    local_queue: Vec<Track>,  // Store actual track metadata
    selected_queue_item: usize,
    show_queue: bool,

    // Core components
    tidal_client: TidalClient,
    mpd_controller: MpdController,
    debug_log: VecDeque<String>,
    visualizer: Option<CavaVisualizer>,
    show_visualizer: bool,

    // Helix-style key command state
    pending_key: Option<char>,  // For multi-key commands like 'g' prefix
    space_pressed: bool,  // For Space-prefixed commands
}

impl App {
    async fn new() -> Result<Self> {
        let mut debug_log = VecDeque::new();
        debug_log.push_back("Starting Tidal TUI...".to_string());

        // Initialize Tidal client
        debug_log.push_back("Initializing Tidal client...".to_string());
        let mut tidal_client = TidalClient::new().await?;

        if tidal_client.config.is_some() {
            debug_log.push_back("✓ Tidal credentials loaded".to_string());
        } else {
            debug_log.push_back("⚠ No Tidal credentials - demo mode".to_string());
        }

        // Initialize MPD controller
        debug_log.push_back("Connecting to MPD...".to_string());
        let mpd_controller = MpdController::new(&mut debug_log).await?;

        // Load initial playlists (will auto-refresh token if needed)
        debug_log.push_back("Fetching playlists...".to_string());
        let playlists = tidal_client.get_playlists().await?;
        debug_log.push_back(format!("✓ Loaded {} playlists", playlists.len()));

        // Load tracks from first playlist if available
        let tracks = if !playlists.is_empty() {
            debug_log.push_back(format!("Loading tracks from '{}'...", playlists[0].title));
            let tracks = tidal_client.get_tracks(&playlists[0].id).await?;
            debug_log.push_back(format!("✓ Loaded {} tracks", tracks.len()));
            tracks
        } else {
            Vec::new()
        };

        // Try to initialize visualizer
        let mut visualizer = match CavaVisualizer::new() {
            Ok(mut v) => {
                debug_log.push_back("✓ Visualizer initialized".to_string());
                // Start the cava process
                match v.start() {
                    Ok(_) => {
                        debug_log.push_back("✓ Cava process started".to_string());
                        Some(v)
                    }
                    Err(e) => {
                        debug_log.push_back(format!("⚠ Could not start cava: {}", e));
                        None
                    }
                }
            }
            Err(e) => {
                debug_log.push_back(format!("⚠ Could not initialize visualizer: {}", e));
                None
            }
        };

        Ok(Self {
            view_mode: ViewMode::Browse,
            selected_tab: 0,
            playlists,
            tracks,
            selected_playlist: 0,
            selected_track: 0,
            search_query: String::new(),
            search_results: None,
            search_tab: SearchTab::Tracks,
            selected_search_track: 0,
            selected_search_album: 0,
            selected_search_artist: 0,
            is_searching: false,
            is_playing: false,
            current_track: None,
            current_song: None,
            queue: Vec::new(),
            local_queue: Vec::new(),
            selected_queue_item: 0,
            show_queue: false,
            tidal_client,
            mpd_controller,
            debug_log,
            visualizer,
            show_visualizer: true,
            pending_key: None,
            space_pressed: false,
        })
    }

    fn add_debug(&mut self, msg: String) {
        // Write to debug file
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/tidal-tui-debug.log")
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

    async fn search(&mut self) -> Result<()> {
        if self.search_query.trim().is_empty() {
            self.add_debug("Search query is empty".to_string());
            return Ok(());
        }

        self.add_debug(format!("Searching for: {}", self.search_query));
        self.is_searching = true;

        // Search will auto-refresh token if needed
        match self.tidal_client.search(&self.search_query, 20).await {
            Ok(results) => {
                let track_count = results.tracks.len();
                let album_count = results.albums.len();
                let artist_count = results.artists.len();
                self.add_debug(format!("✓ Found {} tracks, {} albums, {} artists",
                    track_count, album_count, artist_count));

                self.search_results = Some(results);
                self.selected_search_track = 0;
                self.selected_search_album = 0;
                self.selected_search_artist = 0;
            }
            Err(e) => {
                self.add_debug(format!("✗ Search failed: {}", e));
            }
        }

        self.is_searching = false;
        Ok(())
    }

    async fn load_playlist(&mut self, index: usize) -> Result<()> {
        if index < self.playlists.len() {
            let playlist_title = self.playlists[index].title.clone();
            let playlist_id = self.playlists[index].id.clone();
            self.add_debug(format!("Loading playlist: {}", playlist_title));

            // Get tracks will auto-refresh token if needed
            self.tracks = self.tidal_client.get_tracks(&playlist_id).await?;
            self.selected_track = 0;

            self.add_debug(format!("✓ Loaded {} tracks", self.tracks.len()));
        }
        Ok(())
    }

    async fn play_track(&mut self, track: Track) -> Result<()> {
        self.add_debug(format!("Playing: {} - {}", track.artist, track.title));

        // Get streaming URL (will auto-refresh token if needed)
        self.add_debug(format!("Getting stream URL for track ID {}...", track.id));
        let stream_url = match self.tidal_client.get_stream_url(&track.id.to_string()).await {
            Ok(url) => {
                self.add_debug(format!("✓ Got URL: {}...", &url[..50.min(url.len())]));
                url
            }
            Err(e) => {
                self.add_debug(format!("✗ Failed to get URL: {}", e));
                return Err(e);
            }
        };

        // Clear MPD queue and add new track
        self.add_debug("Clearing MPD queue...".to_string());
        if let Err(e) = self.mpd_controller.clear_queue(&mut self.debug_log).await {
            self.add_debug(format!("✗ Clear failed: {}", e));
            return Err(e);
        }
        // Also clear local queue when playing a new track directly
        self.local_queue.clear();

        self.add_debug("Adding track to MPD...".to_string());
        if let Err(e) = self.mpd_controller.add_track(&stream_url, &mut self.debug_log).await {
            self.add_debug(format!("✗ Add failed: {}", e));
            return Err(e);
        }

        self.add_debug("Starting playback...".to_string());
        if let Err(e) = self.mpd_controller.play(&mut self.debug_log).await {
            self.add_debug(format!("✗ Play failed: {}", e));
            return Err(e);
        }

        self.is_playing = true;
        self.current_track = Some(track);
        self.add_debug("✓ Playback started".to_string());
        Ok(())
    }

    async fn play_selected_track(&mut self) -> Result<()> {
        let track = if self.view_mode == ViewMode::Browse {
            if self.selected_track < self.tracks.len() {
                self.tracks[self.selected_track].clone()
            } else {
                return Ok(());
            }
        } else {
            // Search mode
            if let Some(ref results) = self.search_results {
                if self.search_tab == SearchTab::Tracks && self.selected_search_track < results.tracks.len() {
                    results.tracks[self.selected_search_track].clone()
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        };

        self.play_track(track).await
    }

    async fn add_selected_track_to_queue(&mut self) -> Result<()> {
        let track = if self.view_mode == ViewMode::Browse {
            if self.selected_track < self.tracks.len() {
                self.tracks[self.selected_track].clone()
            } else {
                return Ok(());
            }
        } else {
            // Search mode
            if let Some(ref results) = self.search_results {
                if self.search_tab == SearchTab::Tracks && self.selected_search_track < results.tracks.len() {
                    results.tracks[self.selected_search_track].clone()
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        };

        self.add_track_to_queue(track).await
    }

    async fn add_track_to_queue(&mut self, track: Track) -> Result<()> {
        self.add_debug(format!("Adding to queue: {} - {}", track.artist, track.title));

        // Get streaming URL (will auto-refresh token if needed)
        self.add_debug(format!("Getting stream URL for track ID {}...", track.id));
        let stream_url = match self.tidal_client.get_stream_url(&track.id.to_string()).await {
            Ok(url) => {
                self.add_debug(format!("✓ Got URL: {}...", &url[..50.min(url.len())]));
                url
            }
            Err(e) => {
                self.add_debug(format!("✗ Failed to get URL: {}", e));
                return Err(e);
            }
        };

        // Add to MPD queue (without clearing)
        self.add_debug("Adding to MPD queue...".to_string());
        if let Err(e) = self.mpd_controller.add_track(&stream_url, &mut self.debug_log).await {
            self.add_debug(format!("✗ Add to MPD failed: {}", e));
            return Err(e);
        }

        self.add_debug(format!("✓ Added to queue: {}", track.title));

        // Add to local queue for metadata tracking
        self.local_queue.push(track.clone());

        // Check actual queue status
        if let Ok(queue) = self.mpd_controller.get_queue().await {
            self.add_debug(format!("  Queue now has {} tracks", queue.len()));
            self.queue = queue;
        }

        // Only update current_track if this is the first track
        if self.current_track.is_none() {
            self.current_track = Some(track);
        }

        // If nothing is playing, start playback
        let status = self.mpd_controller.get_status(&mut self.debug_log).await?;
        if !status.is_playing {
            self.add_debug("No playback detected, starting...".to_string());
            if let Err(e) = self.mpd_controller.play(&mut self.debug_log).await {
                self.add_debug(format!("✗ Play failed: {}", e));
                return Err(e);
            }
            self.is_playing = true;
        } else {
            self.add_debug("Playback already active, track queued".to_string());
        }

        Ok(())
    }

    async fn add_all_tracks_to_queue(&mut self) -> Result<()> {
        let tracks_to_add = if self.view_mode == ViewMode::Browse {
            if self.selected_tab == 1 && !self.tracks.is_empty() {
                // Add all tracks from current playlist
                self.tracks.clone()
            } else {
                return Ok(());
            }
        } else {
            // Search mode - add all search results
            if let Some(ref results) = self.search_results {
                if self.search_tab == SearchTab::Tracks && !results.tracks.is_empty() {
                    results.tracks.clone()
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        };

        self.add_debug(format!("Adding {} tracks to queue...", tracks_to_add.len()));

        // Check if we need to start playback after adding
        let was_playing = self.mpd_controller.get_status(&mut self.debug_log).await?.is_playing;

        let mut added_count = 0;
        for (i, track) in tracks_to_add.iter().enumerate() {
            self.add_debug(format!("[{}/{}] {} - {}", i+1, tracks_to_add.len(), track.artist, track.title));

            // Get streaming URL
            match self.tidal_client.get_stream_url(&track.id.to_string()).await {
                Ok(url) => {
                    // Add to MPD queue
                    if let Err(e) = self.mpd_controller.add_track(&url, &mut self.debug_log).await {
                        self.add_debug(format!("  ✗ Failed to add: {}", e));
                    } else {
                        // Add to local queue for metadata tracking
                        self.local_queue.push(track.clone());
                        added_count += 1;
                    }
                }
                Err(e) => {
                    self.add_debug(format!("  ✗ Failed to get URL: {}", e));
                }
            }
        }

        self.add_debug(format!("✓ Added {}/{} tracks to queue", added_count, tracks_to_add.len()));

        // Update queue display
        if let Ok(queue) = self.mpd_controller.get_queue().await {
            self.queue = queue;
            self.add_debug(format!("Queue now has {} total tracks", self.queue.len()));
        }

        // Start playback if nothing was playing
        if !was_playing && added_count > 0 {
            self.add_debug("Starting playback...".to_string());
            if let Err(e) = self.mpd_controller.play(&mut self.debug_log).await {
                self.add_debug(format!("✗ Play failed: {}", e));
            } else {
                self.is_playing = true;
            }
        }

        Ok(())
    }

    async fn toggle_playback(&mut self) -> Result<()> {
        if self.is_playing {
            self.add_debug("Pausing playback...".to_string());
            self.mpd_controller.pause(&mut self.debug_log).await?;
        } else {
            self.add_debug("Resuming playback...".to_string());
            self.mpd_controller.play(&mut self.debug_log).await?;
        }
        self.is_playing = !self.is_playing;
        Ok(())
    }

    async fn check_mpd_status(&mut self) -> Result<()> {
        let status = self.mpd_controller.get_status(&mut self.debug_log).await?;

        if status.is_playing != self.is_playing {
            self.is_playing = status.is_playing;
            self.add_debug(format!("Playback state: {}", if self.is_playing { "playing" } else { "paused" }));
        }

        // Update current song info from MPD status (elapsed/duration)
        // Since MPD plays raw URLs without metadata, we combine MPD timing info
        // with our stored Track metadata
        if let Some(ref track) = self.current_track {
            // Get elapsed and duration from MPD status
            match self.mpd_controller.get_timing_info().await {
                Ok((elapsed, duration)) => {
                    self.current_song = Some(mpd::CurrentSong {
                        artist: track.artist.clone(),
                        title: track.title.clone(),
                        album: track.album.clone(),
                        elapsed,
                        duration,
                    });
                }
                Err(e) => {
                    self.add_debug(format!("Failed to get timing info: {}", e));
                }
            }
        } else if self.current_song.is_some() {
            // No current_track but we have current_song, clear it
            self.current_song = None;
        }

        // Update queue if it's visible
        if self.show_queue {
            if let Ok(queue) = self.mpd_controller.get_queue().await {
                self.queue = queue;
            }
        }

        Ok(())
    }

    // Helix-style navigation helpers
    fn move_down(&mut self) {
        if self.show_queue && !self.queue.is_empty() {
            self.selected_queue_item = (self.selected_queue_item + 1).min(self.queue.len() - 1);
        } else if self.view_mode == ViewMode::Browse {
            if self.selected_tab == 0 && !self.playlists.is_empty() {
                self.selected_playlist = (self.selected_playlist + 1).min(self.playlists.len() - 1);
            } else if self.selected_tab == 1 && !self.tracks.is_empty() {
                self.selected_track = (self.selected_track + 1).min(self.tracks.len() - 1);
            }
        } else if let Some(ref results) = self.search_results {
            match self.search_tab {
                SearchTab::Tracks if !results.tracks.is_empty() => {
                    self.selected_search_track = (self.selected_search_track + 1).min(results.tracks.len() - 1);
                }
                SearchTab::Albums if !results.albums.is_empty() => {
                    self.selected_search_album = (self.selected_search_album + 1).min(results.albums.len() - 1);
                }
                SearchTab::Artists if !results.artists.is_empty() => {
                    self.selected_search_artist = (self.selected_search_artist + 1).min(results.artists.len() - 1);
                }
                _ => {}
            }
        }
    }

    fn move_up(&mut self) {
        if self.show_queue && !self.queue.is_empty() {
            if self.selected_queue_item > 0 {
                self.selected_queue_item -= 1;
            }
        } else if self.view_mode == ViewMode::Browse {
            if self.selected_tab == 0 && self.selected_playlist > 0 {
                self.selected_playlist -= 1;
            } else if self.selected_tab == 1 && self.selected_track > 0 {
                self.selected_track -= 1;
            }
        } else if let Some(ref results) = self.search_results {
            match self.search_tab {
                SearchTab::Tracks if self.selected_search_track > 0 => {
                    self.selected_search_track -= 1;
                }
                SearchTab::Albums if self.selected_search_album > 0 => {
                    self.selected_search_album -= 1;
                }
                SearchTab::Artists if self.selected_search_artist > 0 => {
                    self.selected_search_artist -= 1;
                }
                _ => {}
            }
        }
    }

    fn move_left(&mut self) {
        if self.view_mode == ViewMode::Browse && self.selected_tab > 0 {
            self.selected_tab = 0;
            self.add_debug("Switched to playlists panel".to_string());
        }
    }

    fn move_right(&mut self) {
        if self.view_mode == ViewMode::Browse && self.selected_tab < 1 {
            self.selected_tab = 1;
            self.add_debug("Switched to tracks panel".to_string());
        }
    }

    fn jump_to_top(&mut self) {
        if self.show_queue {
            self.selected_queue_item = 0;
        } else if self.view_mode == ViewMode::Browse {
            if self.selected_tab == 0 {
                self.selected_playlist = 0;
            } else {
                self.selected_track = 0;
            }
        } else {
            match self.search_tab {
                SearchTab::Tracks => self.selected_search_track = 0,
                SearchTab::Albums => self.selected_search_album = 0,
                SearchTab::Artists => self.selected_search_artist = 0,
            }
        }
    }

    fn jump_to_end(&mut self) {
        if self.show_queue && !self.queue.is_empty() {
            self.selected_queue_item = self.queue.len() - 1;
        } else if self.view_mode == ViewMode::Browse {
            if self.selected_tab == 0 && !self.playlists.is_empty() {
                self.selected_playlist = self.playlists.len() - 1;
            } else if self.selected_tab == 1 && !self.tracks.is_empty() {
                self.selected_track = self.tracks.len() - 1;
            }
        } else if let Some(ref results) = self.search_results {
            match self.search_tab {
                SearchTab::Tracks if !results.tracks.is_empty() => {
                    self.selected_search_track = results.tracks.len() - 1;
                }
                SearchTab::Albums if !results.albums.is_empty() => {
                    self.selected_search_album = results.albums.len() - 1;
                }
                SearchTab::Artists if !results.artists.is_empty() => {
                    self.selected_search_artist = results.artists.len() - 1;
                }
                _ => {}
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = match App::new().await {
        Ok(app) => app,
        Err(e) => {
            // Cleanup terminal before showing error
            disable_raw_mode()?;
            execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
            eprintln!("Failed to initialize app: {}", e);
            return Err(e);
        }
    };

    // Run the app
    let res = run_app(&mut terminal, &mut app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("{:?}", err);
    }

    Ok(())
}

async fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    let mut last_status_check = std::time::Instant::now();

    loop {
        // Check MPD status periodically (before drawing)
        if last_status_check.elapsed() > Duration::from_secs(1) {
            if let Err(e) = app.check_mpd_status().await {
                app.add_debug(format!("MPD status check error: {}", e));
            }
            last_status_check = std::time::Instant::now();
        }

        terminal.draw(|f| ui(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                // Handle search input mode separately
                if app.is_searching {
                    match key.code {
                        KeyCode::Enter => {
                            app.is_searching = false;
                            if let Err(e) = app.search().await {
                                app.add_debug(format!("✗ Search error: {}", e));
                            }
                        }
                        KeyCode::Esc => {
                            app.is_searching = false;
                            app.search_query.clear();
                        }
                        KeyCode::Backspace => {
                            app.search_query.pop();
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.search_query.push(c);
                        }
                        _ => {}
                    }
                    continue;
                }

                // Helix-style keybindings in normal mode
                // Handle Space-prefixed commands
                if app.space_pressed {
                    match key.code {
                        KeyCode::Char('q') => {
                            app.space_pressed = false;
                            return Ok(());
                        }
                        KeyCode::Char('p') => {
                            app.space_pressed = false;
                            if let Err(e) = app.toggle_playback().await {
                                app.add_debug(format!("✗ Error toggling playback: {}", e));
                            }
                        }
                        KeyCode::Char('n') => {
                            app.space_pressed = false;
                            app.add_debug("Next track".to_string());
                            if let Err(e) = app.mpd_controller.next(&mut app.debug_log).await {
                                app.add_debug(format!("✗ Next failed: {}", e));
                            }
                        }
                        KeyCode::Char('b') => {
                            app.space_pressed = false;
                            app.add_debug("Previous track".to_string());
                            if let Err(e) = app.mpd_controller.previous(&mut app.debug_log).await {
                                app.add_debug(format!("✗ Previous failed: {}", e));
                            }
                        }
                        KeyCode::Char('v') => {
                            app.space_pressed = false;
                            app.show_visualizer = !app.show_visualizer;
                            app.add_debug(format!("Visualizer {}", if app.show_visualizer { "enabled" } else { "disabled" }));
                        }
                        KeyCode::Char('c') => {
                            app.space_pressed = false;
                            app.debug_log.clear();
                            app.add_debug("Debug log cleared".to_string());
                        }
                        KeyCode::Char('e') => {
                            app.space_pressed = false;
                            let export_path = "/tmp/tidal-tui-export.log";
                            let mut content = String::new();
                            for line in &app.debug_log {
                                content.push_str(line);
                                content.push('\n');
                            }
                            if let Err(e) = std::fs::write(export_path, content) {
                                app.add_debug(format!("✗ Failed to export log: {}", e));
                            } else {
                                app.add_debug(format!("✓ Debug log exported to {}", export_path));
                            }
                        }
                        _ => {
                            app.space_pressed = false;
                        }
                    }
                    continue;
                }

                // Handle 'g' prefix for jump commands
                if let Some('g') = app.pending_key {
                    match key.code {
                        KeyCode::Char('g') => {
                            app.pending_key = None;
                            app.jump_to_top();
                        }
                        KeyCode::Char('e') => {
                            app.pending_key = None;
                            app.jump_to_end();
                        }
                        _ => {
                            app.pending_key = None;
                        }
                    }
                    continue;
                }

                // Main helix-style commands
                match key.code {
                    // Navigation
                    KeyCode::Char('h') => app.move_left(),
                    KeyCode::Char('j') => app.move_down(),
                    KeyCode::Char('k') => app.move_up(),
                    KeyCode::Char('l') => app.move_right(),

                    // Jump commands (prefix)
                    KeyCode::Char('g') => {
                        app.pending_key = Some('g');
                    }

                    // Space prefix for commands
                    KeyCode::Char(' ') => {
                        app.space_pressed = true;
                    }

                    // Enter: load playlist or play track
                    KeyCode::Enter => {
                        // If queue is visible, play from queue position instead
                        if app.show_queue && !app.local_queue.is_empty() && app.selected_queue_item < app.local_queue.len() {
                            app.add_debug(format!("Playing from queue position {}", app.selected_queue_item + 1));
                            if let Err(e) = app.mpd_controller.play_position(app.selected_queue_item, &mut app.debug_log).await {
                                app.add_debug(format!("✗ Failed to play from queue: {}", e));
                            }
                        } else if app.view_mode == ViewMode::Browse {
                            if app.selected_tab == 0 {
                                if let Err(e) = app.load_playlist(app.selected_playlist).await {
                                    app.add_debug(format!("✗ Error loading playlist: {}", e));
                                }
                            } else if app.selected_tab == 1 {
                                if let Err(e) = app.play_selected_track().await {
                                    app.add_debug(format!("✗ Error playing track: {}", e));
                                }
                            }
                        } else if app.search_tab == SearchTab::Tracks {
                            if let Err(e) = app.play_selected_track().await {
                                app.add_debug(format!("✗ Error playing track: {}", e));
                            }
                        }
                    }

                    // y: yank/add to queue
                    KeyCode::Char('y') => {
                        if let Err(e) = app.add_selected_track_to_queue().await {
                            app.add_debug(format!("✗ Failed to add track: {}", e));
                        }
                    }

                    // Y: yank all
                    KeyCode::Char('Y') => {
                        if let Err(e) = app.add_all_tracks_to_queue().await {
                            app.add_debug(format!("✗ Failed to add tracks: {}", e));
                        }
                    }

                    // p: play selected
                    KeyCode::Char('p') => {
                        // If queue is visible, play from queue position
                        if app.show_queue && !app.local_queue.is_empty() && app.selected_queue_item < app.local_queue.len() {
                            app.add_debug(format!("Playing from queue position {}", app.selected_queue_item + 1));
                            if let Err(e) = app.mpd_controller.play_position(app.selected_queue_item, &mut app.debug_log).await {
                                app.add_debug(format!("✗ Failed to play from queue: {}", e));
                            }
                        } else if app.view_mode == ViewMode::Browse && app.selected_tab == 1 {
                            if let Err(e) = app.play_selected_track().await {
                                app.add_debug(format!("✗ Error playing track: {}", e));
                            }
                        } else if app.view_mode == ViewMode::Search && app.search_tab == SearchTab::Tracks {
                            if let Err(e) = app.play_selected_track().await {
                                app.add_debug(format!("✗ Error playing track: {}", e));
                            }
                        }
                    }

                    // d: delete/remove from queue
                    KeyCode::Char('d') => {
                        if app.show_queue && !app.local_queue.is_empty() {
                            if app.selected_queue_item < app.local_queue.len() {
                                if let Err(e) = app.mpd_controller.remove_from_queue(app.selected_queue_item, &mut app.debug_log).await {
                                    app.add_debug(format!("✗ Failed to remove track: {}", e));
                                } else {
                                    app.local_queue.remove(app.selected_queue_item);
                                    if app.selected_queue_item > 0 && app.selected_queue_item >= app.local_queue.len() {
                                        app.selected_queue_item -= 1;
                                    }
                                    app.add_debug(format!("✓ Removed track from queue, {} remaining", app.local_queue.len()));
                                }
                            }
                        }
                    }

                    // D: clear entire queue
                    KeyCode::Char('D') => {
                        if let Err(e) = app.mpd_controller.clear_queue(&mut app.debug_log).await {
                            app.add_debug(format!("✗ Failed to clear queue: {}", e));
                        } else {
                            app.queue.clear();
                            app.local_queue.clear();
                            app.add_debug("✓ Queue cleared".to_string());
                        }
                    }

                    // /: search
                    KeyCode::Char('/') => {
                        app.view_mode = ViewMode::Search;
                        app.is_searching = true;
                        app.search_query.clear();
                        app.add_debug("Search mode activated".to_string());
                    }

                    // b: browse mode
                    KeyCode::Char('b') => {
                        app.view_mode = ViewMode::Browse;
                        app.add_debug("Browse mode activated".to_string());
                    }

                    // w: toggle queue (think "window")
                    KeyCode::Char('w') => {
                        app.show_queue = !app.show_queue;
                        if app.show_queue {
                            match app.mpd_controller.get_queue().await {
                                Ok(queue) => {
                                    app.queue = queue;
                                    app.add_debug(format!("✓ Queue loaded: {} tracks", app.queue.len()));
                                }
                                Err(e) => {
                                    app.add_debug(format!("✗ Failed to load queue: {}", e));
                                }
                            }
                        }
                        app.add_debug(format!("Queue {}", if app.show_queue { "shown" } else { "hidden" }));
                    }

                    // Tab: cycle through tabs in current view
                    KeyCode::Tab => {
                        if app.view_mode == ViewMode::Browse {
                            app.selected_tab = (app.selected_tab + 1) % 2;
                            app.add_debug(format!("Switched to {} panel",
                                if app.selected_tab == 0 { "playlists" } else { "tracks" }));
                        } else {
                            app.search_tab = match app.search_tab {
                                SearchTab::Tracks => SearchTab::Albums,
                                SearchTab::Albums => SearchTab::Artists,
                                SearchTab::Artists => SearchTab::Tracks,
                            };
                            app.add_debug(format!("Switched to {:?} results", app.search_tab));
                        }
                    }

                    _ => {}
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &App) {
    // Main layout - adjust based on what's visible
    let mut constraints = vec![
        Constraint::Length(3),     // Header
        Constraint::Length(9),     // Now Playing (always visible, larger)
    ];

    // Add visualizer if enabled
    if app.show_visualizer && app.visualizer.is_some() {
        constraints.push(Constraint::Length(7)); // Visualizer
    }

    // Main content area (will be split horizontally if queue is shown)
    constraints.push(Constraint::Percentage(50)); // Main content area

    // Debug panel
    constraints.push(Constraint::Percentage(25)); // Debug panel
    constraints.push(Constraint::Length(3));     // Status bar

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(f.area());

    // Header - shows mode and connection status
    let header_text = format!(
        "{} - {} Mode",
        if app.tidal_client.config.is_some() {
            "Tidal TUI - Connected"
        } else {
            "Tidal TUI - Demo Mode"
        },
        match app.view_mode {
            ViewMode::Browse => "Browse",
            ViewMode::Search => "Search",
        }
    );

    let header = Paragraph::new(header_text)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        );
    let mut chunk_index = 0;
    f.render_widget(header, main_chunks[chunk_index]);
    chunk_index += 1;

    // Now Playing (always visible)
    render_now_playing(f, app, main_chunks[chunk_index]);
    chunk_index += 1;

    // Visualizer panel (if enabled)
    if app.show_visualizer && app.visualizer.is_some() {
        render_visualizer(f, app, main_chunks[chunk_index]);
        chunk_index += 1;
    }

    // Main content area - split horizontally if queue is shown
    let content_area = main_chunks[chunk_index];

    if app.show_queue {
        // Split the content area horizontally
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(60), // Main content
                Constraint::Percentage(40), // Queue
            ])
            .split(content_area);

        // Render main content on the left
        if app.view_mode == ViewMode::Browse {
            render_browse_view(f, app, content_chunks[0]);
        } else {
            render_search_view(f, app, content_chunks[0]);
        }

        // Render queue on the right
        render_queue(f, app, content_chunks[1]);
    } else {
        // Full width for main content when queue is hidden
        if app.view_mode == ViewMode::Browse {
            render_browse_view(f, app, content_area);
        } else {
            render_search_view(f, app, content_area);
        }
    }
    chunk_index += 1;

    // Debug panel
    let debug_text: String = app.debug_log
        .iter()
        .rev()
        .take(10)
        .rev()
        .map(|s| s.clone())
        .collect::<Vec<_>>()
        .join("\n");

    let debug_panel = Paragraph::new(debug_text)
        .style(Style::default().fg(Color::Gray))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Debug Log [Space+e: export | Space+c: clear]")
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    f.render_widget(debug_panel, main_chunks[chunk_index]);
    chunk_index += 1;

    // Build status bar based on current mode (no longer showing playback status)
    let status_bar = if app.is_searching {
        Paragraph::new(Line::from(vec![
            Span::styled("INSERT MODE", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" | "),
            Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": search | "),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": cancel"),
        ]))
    } else if app.space_pressed {
        Paragraph::new(Line::from(vec![
            Span::styled("SPACE", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" + "),
            Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": quit | "),
            Span::styled("p", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": pause | "),
            Span::styled("n", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": next | "),
            Span::styled("b", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": prev | "),
            Span::styled("v", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": visualizer | "),
            Span::styled("c", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": clear log | "),
            Span::styled("e", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": export"),
        ]))
    } else if app.pending_key == Some('g') {
        Paragraph::new(Line::from(vec![
            Span::styled("g", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" + "),
            Span::styled("g", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": top | "),
            Span::styled("e", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": end"),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled("NORMAL", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(" | "),
            Span::styled("hjkl", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": move | "),
            Span::styled("gg/ge", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": top/end | "),
            Span::styled("y/Y", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": add/all | "),
            Span::styled("p", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": play | "),
            Span::styled("d/D", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": del/clear | "),
            Span::styled("w", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": queue | "),
            Span::styled("Space", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(": cmd"),
        ]))
    };

    let status_bar = status_bar.block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded),
    );
    f.render_widget(status_bar, main_chunks[chunk_index]);
}

fn render_visualizer(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if let Some(ref viz) = app.visualizer {
        let bars = viz.draw_bars();

        // Create visualization text with proper spacing
        let mut lines = vec![];

        // Add the bars
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(bars, Style::default().fg(Color::Cyan)),
        ]));

        // Add frequency labels
        lines.push(Line::from(vec![
            Span::styled("  Bass ", Style::default().fg(Color::DarkGray)),
            Span::raw("                    "),
            Span::styled("Treble", Style::default().fg(Color::DarkGray)),
        ]));

        let visualizer = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title("♫ Audio Visualizer [Space+v: toggle]")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(if app.is_playing {
                        Color::Green
                    } else {
                        Color::DarkGray
                    })),
            );

        f.render_widget(visualizer, area);
    }
}

fn render_now_playing(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let mut lines = vec![];

    if let Some(ref song) = app.current_song {
        // Status icon
        let status_icon = if app.is_playing { "▶" } else { "⏸" };
        let status_color = if app.is_playing { Color::Green } else { Color::Yellow };

        // Title and artist line
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", status_icon), Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
            Span::styled(&song.title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]));

        // Artist line
        lines.push(Line::from(vec![
            Span::raw("   Artist: "),
            Span::styled(&song.artist, Style::default().fg(Color::Cyan)),
        ]));

        // Album line
        lines.push(Line::from(vec![
            Span::raw("   Album:  "),
            Span::styled(&song.album, Style::default().fg(Color::Magenta)),
        ]));

        // Empty line for spacing
        lines.push(Line::from(""));

        // Calculate progress
        let elapsed_secs = song.elapsed.as_secs();
        let total_secs = song.duration.as_secs();
        let progress = if total_secs > 0 {
            (elapsed_secs as f64 / total_secs as f64).min(1.0)
        } else {
            0.0
        };

        // Calculate progress bar width based on available space
        // Account for time stamps and padding
        let bar_width = area.width.saturating_sub(20).max(40) as usize;
        let filled = (progress * bar_width as f64) as usize;
        let empty = bar_width.saturating_sub(filled);

        // Use Unicode block characters for smooth progress bar
        let filled_str = "━".repeat(filled);
        let empty_str = "─".repeat(empty);

        // Progress bar line with owned strings
        lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled(format!("{:02}:{:02}", elapsed_secs / 60, elapsed_secs % 60), Style::default().fg(Color::Gray)),
            Span::raw(" "),
            Span::styled(filled_str, Style::default().fg(Color::Cyan)),
            Span::styled(empty_str, Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(format!("{:02}:{:02}", total_secs / 60, total_secs % 60), Style::default().fg(Color::Gray)),
            Span::raw(format!(" ({}%)", (progress * 100.0) as u8)),
        ]));

        // Queue info
        let queue_info = if app.local_queue.len() > 1 {
            format!("   {} tracks in queue", app.local_queue.len())
        } else if app.local_queue.len() == 1 {
            "   1 track in queue".to_string()
        } else {
            "   No tracks in queue".to_string()
        };
        lines.push(Line::from(vec![
            Span::styled(queue_info, Style::default().fg(Color::DarkGray)),
        ]));

    } else {
        // No track playing
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("   No track playing", Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("   Press ", Style::default().fg(Color::DarkGray)),
            Span::styled("p", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" or ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(" to play a track", Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(""));
    }

    let border_style = if app.is_playing {
        Style::default().fg(Color::Green)
    } else if app.current_song.is_some() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if app.is_playing {
        "♫ Now Playing"
    } else if app.current_song.is_some() {
        "♫ Paused"
    } else {
        "♫ Player"
    };

    let now_playing = Paragraph::new(lines)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style),
        );

    f.render_widget(now_playing, area);
}

fn render_queue(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    if app.local_queue.is_empty() {
        // Show empty queue message
        let empty_msg = Paragraph::new("Queue is empty\n\nPress 'y' to add selected track\nPress 'Y' to add all tracks")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title("Queue (0 tracks) [y: add | Y: add all | w: hide]")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan)),
            );
        f.render_widget(empty_msg, area);
        return;
    }

    let mut items = vec![];

    for (i, track) in app.local_queue.iter().enumerate() {
        let style = if i == app.selected_queue_item {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        // Format duration
        let duration_str = format!("{}:{:02}", track.duration_seconds / 60, track.duration_seconds % 60);

        let content = format!(
            "{:2}. {} - {} [{}]",
            i + 1,
            track.artist,
            track.title,
            duration_str
        );

        items.push(ListItem::new(content).style(style));
    }

    let queue_list = List::new(items)
        .block(
            Block::default()
                .title(format!("Queue ({} tracks) [p/Enter: play | y: add | d: remove | D: clear]", app.local_queue.len()))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(if app.show_queue {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                }),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    f.render_stateful_widget(
        queue_list,
        area,
        &mut ratatui::widgets::ListState::default().with_selected(Some(app.selected_queue_item)),
    );
}

fn render_browse_view(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)].as_ref())
        .split(area);

    // Left panel - Playlists
    let playlists: Vec<ListItem> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(i, playlist)| {
            let style = if i == app.selected_playlist && app.selected_tab == 0 {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let display = TidalClient::format_playlist_display(playlist);
            ListItem::new(display).style(style)
        })
        .collect();

    let playlists_widget = List::new(playlists)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Playlists [h/l: switch | Enter: load]")
                .border_style(if app.selected_tab == 0 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        );
    f.render_widget(playlists_widget, content_chunks[0]);

    // Right panel - Tracks
    let tracks: Vec<ListItem> = app
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let style = if i == app.selected_track && app.selected_tab == 1 {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let display = TidalClient::format_track_display(track);
            ListItem::new(display).style(style)
        })
        .collect();

    let tracks_widget = List::new(tracks)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Tracks [p/Enter: play | y: add to queue]")
                .border_style(if app.selected_tab == 1 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        );
    f.render_widget(tracks_widget, content_chunks[1]);
}

fn render_search_view(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let search_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)].as_ref())
        .split(area);

    // Search input box
    let search_input = Paragraph::new(app.search_query.as_str())
        .style(if app.is_searching {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if app.is_searching {
                    "Search (Enter to search, Esc to cancel)"
                } else {
                    "Search (/ to search again)"
                })
                .border_style(if app.is_searching {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
        );
    f.render_widget(search_input, search_chunks[0]);

    // Search results
    if let Some(ref results) = app.search_results {
        let results_area = search_chunks[1];

        match app.search_tab {
            SearchTab::Tracks => {
                let items: Vec<ListItem> = results
                    .tracks
                    .iter()
                    .enumerate()
                    .map(|(i, track)| {
                        let style = if i == app.selected_search_track {
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        let display = TidalClient::format_track_display(track);
                        ListItem::new(display).style(style)
                    })
                    .collect();

                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!("Tracks ({}) [Tab: cycle results | p: play | y: add]", results.tracks.len()))
                            .border_style(Style::default().fg(Color::Cyan)),
                    );
                f.render_widget(list, results_area);
            }
            SearchTab::Albums => {
                let items: Vec<ListItem> = results
                    .albums
                    .iter()
                    .enumerate()
                    .map(|(i, album)| {
                        let style = if i == app.selected_search_album {
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        let display = format!("{} - {} ({} tracks)", album.artist, album.title, album.num_tracks);
                        ListItem::new(display).style(style)
                    })
                    .collect();

                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!("Albums ({}) [Tab: cycle results]", results.albums.len()))
                            .border_style(Style::default().fg(Color::Magenta)),
                    );
                f.render_widget(list, results_area);
            }
            SearchTab::Artists => {
                let items: Vec<ListItem> = results
                    .artists
                    .iter()
                    .enumerate()
                    .map(|(i, artist)| {
                        let style = if i == app.selected_search_artist {
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        ListItem::new(artist.name.clone()).style(style)
                    })
                    .collect();

                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!("Artists ({}) [Tab: cycle results]", results.artists.len()))
                            .border_style(Style::default().fg(Color::Green)),
                    );
                f.render_widget(list, results_area);
            }
        }
    } else {
        let empty_msg = if app.search_query.is_empty() {
            "Type to search for tracks, albums, and artists"
        } else {
            "No results. Press Enter to search."
        };

        let empty = Paragraph::new(empty_msg)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Search Results"),
            );
        f.render_widget(empty, search_chunks[1]);
    }
}