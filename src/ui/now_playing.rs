use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::album_art::AlbumArtCache;
use crate::app::state::RadioSeed;
use crate::cava::CavaVisualizer;
use crate::mpd::CurrentSong;
use crate::service::{CoverArt, Track};

use super::styles::service_badge;
use super::theme::Theme;

pub struct NowPlayingState<'a> {
    pub current_track: Option<&'a Track>,
    pub current_song: Option<&'a CurrentSong>,
    pub is_playing: bool,
    pub volume: u8,
    pub repeat_mode: bool,
    pub random_mode: bool,
    pub single_mode: bool,
    pub radio_seed: Option<RadioSeed>,
    pub local_queue_len: usize,
    pub album_art_cache: &'a mut AlbumArtCache,
    pub visualizer: Option<&'a CavaVisualizer>,
    pub video_mode: bool,
}

pub fn render_now_playing(
    f: &mut Frame,
    state: &mut NowPlayingState,
    area: Rect,
    theme: &Theme,
) -> Option<Rect> {
    // Split vertically if visualizer is present
    let (player_area, visualizer_area) = if state.visualizer.is_some() {
        let vsplit = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(7),      // Player info
                Constraint::Length(5),   // Visualizer
            ])
            .split(area);
        (vsplit[0], Some(vsplit[1]))
    } else {
        (area, None)
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(20),
            Constraint::Min(40),
        ])
        .split(player_area);

    let art_area = chunks[0];
    let info_area = chunks[1];

    let has_album_art = if let Some(track) = state.current_track {
        let cover_id = match &track.cover_art {
            CoverArt::ServiceId { id, .. } => Some(id.as_str()),
            CoverArt::Url(url) => Some(url.as_str()),
            CoverArt::None => None,
        };
        if let Some(cover_id) = cover_id {
            if state.album_art_cache.has_cached(cover_id, 320) {
                let _ = state.album_art_cache.set_current_image(cover_id, 320);

                if let Some(protocol) = state.album_art_cache.get_protocol_mut() {
                    use ratatui_image::StatefulImage;
                    let image_widget = StatefulImage::new(None);
                    f.render_stateful_widget(image_widget, art_area, protocol);
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    if !has_album_art {
        let placeholder_lines = vec![
            Line::from(""),
            Line::from(""),
            Line::from(vec![
                Span::styled("       ", Style::default().fg(theme.text_disabled()).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("        ", Style::default().fg(theme.text_disabled())),
            ]),
            Line::from(vec![
                Span::styled("       ", Style::default().fg(theme.text_disabled())),
            ]),
        ];

        let placeholder = Paragraph::new(placeholder_lines)
            .alignment(Alignment::Center);

        f.render_widget(placeholder, art_area);
    }

    let mut lines = vec![];
    let mut progress_bar_area: Option<Rect> = None;

    if let Some(song) = state.current_song {
        let status_icon = if state.is_playing { ">" } else { "||" };
        let status_color = if state.is_playing { theme.success() } else { theme.warning() };

        // Get service badge from current track if available
        let service_prefix = state.current_track
            .map(|t| format!("{} ", service_badge(t.service)))
            .unwrap_or_default();

        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", status_icon), Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}{}", service_prefix, &song.title), Style::default().fg(theme.text()).add_modifier(Modifier::BOLD)),
        ]));

        lines.push(Line::from(vec![
            Span::raw("   Artist: "),
            Span::styled(&song.artist, Style::default().fg(theme.primary())),
        ]));

        lines.push(Line::from(vec![
            Span::raw("   Album:  "),
            Span::styled(&song.album, Style::default().fg(theme.secondary())),
        ]));

        lines.push(Line::from(""));

        let elapsed_secs = song.elapsed.as_secs();
        let total_secs = song.duration.as_secs();
        let progress = if total_secs > 0 {
            (elapsed_secs as f64 / total_secs as f64).min(1.0)
        } else {
            0.0
        };

        let bar_width = info_area.width.saturating_sub(20).max(40) as usize;
        let filled = (progress * bar_width as f64) as usize;
        let empty = bar_width.saturating_sub(filled);

        let filled_str = "=".repeat(filled);
        let empty_str = "-".repeat(empty);

        let progress_bar_x = info_area.x + 10;
        let progress_bar_y = info_area.y + 5;
        progress_bar_area = Some(Rect::new(
            progress_bar_x,
            progress_bar_y,
            bar_width as u16,
            1,
        ));

        lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled(format!("{:02}:{:02}", elapsed_secs / 60, elapsed_secs % 60), Style::default().fg(theme.text_muted())),
            Span::raw(" "),
            Span::styled(filled_str, Style::default().fg(theme.primary())),
            Span::styled(empty_str, Style::default().fg(theme.text_disabled())),
            Span::raw(" "),
            Span::styled(format!("{:02}:{:02}", total_secs / 60, total_secs % 60), Style::default().fg(theme.text_muted())),
            Span::raw(format!(" ({}%)", (progress * 100.0) as u8)),
        ]));

        let queue_info = if state.local_queue_len > 1 {
            format!("{} tracks in queue", state.local_queue_len)
        } else if state.local_queue_len == 1 {
            "1 track in queue".to_string()
        } else {
            "No tracks in queue".to_string()
        };

        let mut modes = Vec::new();
        if state.video_mode {
            modes.push("VIDEO");
        }
        if state.repeat_mode {
            modes.push("repeat");
        }
        if state.single_mode {
            modes.push("single");
        }
        if state.random_mode {
            modes.push("shuffle");
        }
        match &state.radio_seed {
            Some(RadioSeed::Track(_)) => modes.push("radio"),
            Some(RadioSeed::Playlist(_)) => modes.push("mix"),
            Some(RadioSeed::Artist(_)) => modes.push("artist radio"),
            Some(RadioSeed::Album(_)) => modes.push("album radio"),
            None => {}
        }
        let modes_str = if modes.is_empty() {
            String::new()
        } else {
            format!(" | {}", modes.join(", "))
        };

        lines.push(Line::from(vec![
            Span::styled(format!("   Vol: {}%  |  {}{}", state.volume, queue_info, modes_str),
                Style::default().fg(theme.text_disabled())),
        ]));

    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("   No track playing", Style::default().fg(theme.text_disabled()).add_modifier(Modifier::ITALIC)),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("   Press ", Style::default().fg(theme.text_disabled())),
            Span::styled("p", Style::default().fg(theme.primary()).add_modifier(Modifier::BOLD)),
            Span::styled(" or ", Style::default().fg(theme.text_disabled())),
            Span::styled("Enter", Style::default().fg(theme.primary()).add_modifier(Modifier::BOLD)),
            Span::styled(" to play a track", Style::default().fg(theme.text_disabled())),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(""));
    }

    let border_style = if state.is_playing {
        Style::default().fg(theme.success())
    } else if state.current_song.is_some() {
        Style::default().fg(theme.warning())
    } else {
        Style::default().fg(theme.border_normal())
    };

    let title = if state.is_playing {
        "Now Playing"
    } else if state.current_song.is_some() {
        "Paused"
    } else {
        "Player"
    };

    let now_playing = Paragraph::new(lines)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style),
        );

    f.render_widget(now_playing, info_area);

    // Render visualizer if present
    if let (Some(viz), Some(viz_area)) = (state.visualizer, visualizer_area) {
        let bars = viz.draw_bars();

        let viz_lines = vec![
            Line::from(vec![
                Span::raw("  "),
                Span::styled(bars, Style::default().fg(theme.primary())),
            ]),
            Line::from(vec![
                Span::styled("  Bass ", Style::default().fg(theme.text_disabled())),
                Span::raw("                    "),
                Span::styled("Treble", Style::default().fg(theme.text_disabled())),
            ]),
        ];

        let visualizer_widget = Paragraph::new(viz_lines)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title("Visualizer [Space+v: toggle]")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(if state.is_playing {
                        theme.success()
                    } else {
                        theme.text_disabled()
                    })),
            );

        f.render_widget(visualizer_widget, viz_area);
    }

    progress_bar_area
}
