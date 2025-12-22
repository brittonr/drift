use ratatui::{
    layout::Alignment,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::cava::CavaVisualizer;

pub fn render_visualizer(
    f: &mut Frame,
    visualizer: Option<&CavaVisualizer>,
    is_playing: bool,
    area: ratatui::layout::Rect,
) {
    if let Some(viz) = visualizer {
        let bars = viz.draw_bars();

        let mut lines = vec![];

        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(bars, Style::default().fg(Color::Cyan)),
        ]));

        lines.push(Line::from(vec![
            Span::styled("  Bass ", Style::default().fg(Color::DarkGray)),
            Span::raw("                    "),
            Span::styled("Treble", Style::default().fg(Color::DarkGray)),
        ]));

        let visualizer_widget = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title("Audio Visualizer [Space+v: toggle]")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(if is_playing {
                        Color::Green
                    } else {
                        Color::DarkGray
                    })),
            );

        f.render_widget(visualizer_widget, area);
    }
}
