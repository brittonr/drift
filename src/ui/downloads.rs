use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::download_db::{DownloadRecord, DownloadStatus};
use crate::downloads::format_bytes;

pub struct DownloadsViewState<'a> {
    pub download_records: &'a [DownloadRecord],
    pub selected_download: usize,
    pub offline_mode: bool,
    pub pending_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
}

pub fn render_downloads_view(
    f: &mut Frame,
    state: &DownloadsViewState,
    area: Rect,
) -> Rect {
    if state.download_records.is_empty() {
        let empty_msg = Paragraph::new("No downloads\n\nPress 'O' on a track to download it\nPress 'b' to return to browse mode")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title(format!("Downloads [o: offline {}]",
                        if state.offline_mode { "ON" } else { "OFF" }))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Magenta)),
            );
        f.render_widget(empty_msg, area);
        return area;
    }

    let items: Vec<ListItem> = state
        .download_records
        .iter()
        .enumerate()
        .map(|(i, record)| {
            let style = if i == state.selected_download {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let status_icon = match record.status {
                DownloadStatus::Pending => "...",
                DownloadStatus::Downloading => {
                    let progress = if record.total_bytes > 0 {
                        (record.progress_bytes as f64 / record.total_bytes as f64 * 100.0) as u8
                    } else {
                        0
                    };
                    return ListItem::new(format!(
                        "[{:>3}%] {} - {} ({})",
                        progress,
                        record.artist,
                        record.title,
                        format_bytes(record.progress_bytes)
                    )).style(style.fg(Color::Cyan));
                }
                DownloadStatus::Completed => "[OK]",
                DownloadStatus::Failed => "[X]",
                DownloadStatus::Paused => "[||]",
            };

            let status_color = match record.status {
                DownloadStatus::Completed => Color::Green,
                DownloadStatus::Failed => Color::Red,
                DownloadStatus::Downloading => Color::Cyan,
                DownloadStatus::Paused => Color::Yellow,
                DownloadStatus::Pending => Color::DarkGray,
            };

            let content = format!(
                "{} {} - {}",
                status_icon,
                record.artist,
                record.title,
            );

            ListItem::new(content).style(style.fg(status_color))
        })
        .collect();

    let title = format!(
        "Downloads [{}p {}ok {}fail] [o: offline {} | x: delete | R: retry | b: back]",
        state.pending_count, state.completed_count, state.failed_count,
        if state.offline_mode { "ON" } else { "OFF" }
    );

    let downloads_list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Magenta)),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    f.render_stateful_widget(
        downloads_list,
        area,
        &mut ListState::default().with_selected(Some(state.selected_download)),
    );

    area
}
