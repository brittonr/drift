use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::app::state::DialogMode;
use crate::tidal::Playlist;

pub struct DialogRenderState<'a> {
    pub mode: &'a DialogMode,
    pub input_text: &'a str,
    pub selected_index: usize,
    pub playlists: &'a [Playlist],
}

pub fn render_dialog(f: &mut Frame, state: &DialogRenderState, area: Rect) {
    match state.mode {
        DialogMode::None => {}
        DialogMode::CreatePlaylist => {
            render_text_input_dialog(
                f,
                "Create New Playlist",
                "Enter playlist name:",
                state.input_text,
                area,
            );
        }
        DialogMode::AddToPlaylist { track_title, .. } => {
            render_playlist_selector_dialog(
                f,
                &format!("Add to Playlist: {}", truncate_str(track_title, 30)),
                state.playlists,
                state.selected_index,
                area,
            );
        }
        DialogMode::RenamePlaylist { playlist_title, .. } => {
            render_text_input_dialog(
                f,
                &format!("Rename: {}", truncate_str(playlist_title, 25)),
                "Enter new name:",
                state.input_text,
                area,
            );
        }
        DialogMode::ConfirmDeletePlaylist { playlist_title, .. } => {
            render_confirm_dialog(
                f,
                "Delete Playlist",
                &format!("Are you sure you want to delete '{}'?", playlist_title),
                area,
            );
        }
    }
}

fn render_text_input_dialog(
    f: &mut Frame,
    title: &str,
    prompt: &str,
    input: &str,
    area: Rect,
) {
    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_height = 7;
    let popup_x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!(" {} ", title))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    f.render_widget(block.clone(), popup_area);

    let inner = block.inner(popup_area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    // Prompt
    let prompt_line = Paragraph::new(prompt)
        .style(Style::default().fg(Color::White));
    f.render_widget(prompt_line, chunks[0]);

    // Input field with cursor
    let input_display = format!("{}_", input);
    let input_field = Paragraph::new(input_display)
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    f.render_widget(input_field, chunks[1]);

    // Help text
    let help_text = Paragraph::new("Enter: confirm | Esc: cancel")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    f.render_widget(help_text, chunks[3]);
}

fn render_playlist_selector_dialog(
    f: &mut Frame,
    title: &str,
    playlists: &[Playlist],
    selected: usize,
    area: Rect,
) {
    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_height = 15.min(area.height.saturating_sub(4));
    let popup_x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!(" {} ", title))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    f.render_widget(block.clone(), popup_area);

    let inner = block.inner(popup_area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    // Playlist items
    let items: Vec<ListItem> = playlists
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let style = if i == selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let text = format!(
                "{} ({} tracks)",
                truncate_str(&p.title, 35),
                p.num_tracks
            );
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();

    if items.is_empty() {
        let empty_msg = Paragraph::new("No playlists available")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        f.render_widget(empty_msg, chunks[0]);
    } else {
        let list = List::new(items);
        f.render_widget(list, chunks[0]);
    }

    // Help text
    let help_text = Paragraph::new("j/k: select | Enter: add | Esc: cancel")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    f.render_widget(help_text, chunks[1]);
}

fn render_confirm_dialog(
    f: &mut Frame,
    title: &str,
    message: &str,
    area: Rect,
) {
    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_height = 7;
    let popup_x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let popup_y = area.y + (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!(" {} ", title))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Red));

    f.render_widget(block.clone(), popup_area);

    let inner = block.inner(popup_area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(inner);

    // Message
    let msg = Paragraph::new(message)
        .style(Style::default().fg(Color::White))
        .alignment(Alignment::Center);
    f.render_widget(msg, chunks[0]);

    // Help text
    let help_text = Paragraph::new("Enter/y: confirm | Esc/n: cancel")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    f.render_widget(help_text, chunks[1]);
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}
