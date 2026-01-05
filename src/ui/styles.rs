use ratatui::style::{Color, Modifier, Style};

use crate::service::ServiceType;

/// Indicator prefix for currently playing track
pub const PLAYING_INDICATOR: &str = ">> ";
/// Padding to align non-playing tracks with playing ones
pub const PLAYING_PADDING: &str = "   ";

/// Service badge strings for display
pub fn service_badge(service: ServiceType) -> &'static str {
    match service {
        ServiceType::Tidal => "[T]",
        ServiceType::YouTube => "[Y]",
        ServiceType::Bandcamp => "[B]",
    }
}

/// Get the color associated with a service
#[allow(dead_code)]
pub fn service_color(service: ServiceType) -> Color {
    match service {
        ServiceType::Tidal => Color::Cyan,
        ServiceType::YouTube => Color::Red,
        ServiceType::Bandcamp => Color::Magenta,
    }
}

/// Determine the style for a track in a list
#[allow(dead_code)]
pub fn track_style(is_selected: bool, is_playing: bool) -> Style {
    match (is_selected, is_playing) {
        (true, true) => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        (true, false) => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        (false, true) => Style::default().fg(Color::Green),
        (false, false) => Style::default(),
    }
}

/// Check if a track is currently playing
pub fn is_track_playing(track_id: &str, current_track_id: Option<&str>) -> bool {
    current_track_id.map(|id| id == track_id).unwrap_or(false)
}

/// Format track display with optional playing indicator
pub fn format_track_with_indicator(display: String, is_playing: bool) -> String {
    if is_playing {
        format!("{}{}", PLAYING_INDICATOR, display)
    } else {
        format!("{}{}", PLAYING_PADDING, display)
    }
}
