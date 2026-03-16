use ratatui::{
    layout::Rect,
    Frame,
};

use rat_widgets::{ConfirmDialog, InputDialog, SelectList};

use crate::app::state::DialogMode;
use crate::service::Playlist;
use super::theme::Theme;

pub struct DialogRenderState<'a> {
    pub mode: &'a DialogMode,
    pub input_text: &'a str,
    pub selected_index: usize,
    pub playlists: &'a [Playlist],
}

pub fn render_dialog(f: &mut Frame, state: &DialogRenderState, area: Rect, theme: &Theme) {
    let wt = theme.to_widget_theme();

    match state.mode {
        DialogMode::None => {}
        DialogMode::CreatePlaylist => {
            let mut dlg = InputDialog::new("Create New Playlist");
            dlg.value = state.input_text.to_string();
            dlg.render_themed(f, area, &wt);
        }
        DialogMode::AddToPlaylist { track_title, .. } => {
            let items: Vec<String> = state
                .playlists
                .iter()
                .map(|p| {
                    format!(
                        "{} ({} tracks)",
                        truncate_str(&p.title, 35),
                        p.num_tracks
                    )
                })
                .collect();
            let mut dlg = SelectList::new(
                format!("Add to Playlist: {}", truncate_str(track_title, 30)),
                items,
            );
            dlg.selected = state.selected_index;
            dlg.render_themed(f, area, &wt);
        }
        DialogMode::RenamePlaylist { playlist_title, .. } => {
            let mut dlg = InputDialog::new(format!(
                "Rename: {}",
                truncate_str(playlist_title, 25)
            ));
            dlg.value = state.input_text.to_string();
            dlg.render_themed(f, area, &wt);
        }
        DialogMode::ConfirmDeletePlaylist { playlist_title, .. } => {
            let dlg = ConfirmDialog::new(format!(
                "Are you sure you want to delete '{}'?",
                playlist_title
            ));
            dlg.render_themed(f, area, &wt);
        }
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}
