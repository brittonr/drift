use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

/// Theme configuration for the application
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    /// Theme name for identification
    pub name: String,

    /// Primary accent color (borders, highlights)
    pub primary: String,

    /// Secondary accent color (albums, downloads)
    pub secondary: String,

    /// Success/playing state color
    pub success: String,

    /// Warning/selected state color
    pub warning: String,

    /// Error state color
    pub error: String,

    /// Primary text color
    pub text: String,

    /// Secondary/muted text color
    pub text_muted: String,

    /// Disabled/placeholder text color
    pub text_disabled: String,

    /// Border color for focused elements
    pub border_focused: String,

    /// Border color for normal elements
    pub border_normal: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            primary: "Cyan".to_string(),
            secondary: "Magenta".to_string(),
            success: "Green".to_string(),
            warning: "Yellow".to_string(),
            error: "Red".to_string(),
            text: "White".to_string(),
            text_muted: "Gray".to_string(),
            text_disabled: "DarkGray".to_string(),
            border_focused: "Cyan".to_string(),
            border_normal: "DarkGray".to_string(),
        }
    }
}

impl Theme {
    /// Parse a color string to ratatui Color
    pub fn parse_color(color_str: &str) -> Color {
        match color_str.trim() {
            "Reset" => Color::Reset,
            "Black" => Color::Black,
            "Red" => Color::Red,
            "Green" => Color::Green,
            "Yellow" => Color::Yellow,
            "Blue" => Color::Blue,
            "Magenta" => Color::Magenta,
            "Cyan" => Color::Cyan,
            "Gray" | "Grey" => Color::Gray,
            "DarkGray" | "DarkGrey" => Color::DarkGray,
            "LightRed" => Color::LightRed,
            "LightGreen" => Color::LightGreen,
            "LightYellow" => Color::LightYellow,
            "LightBlue" => Color::LightBlue,
            "LightMagenta" => Color::LightMagenta,
            "LightCyan" => Color::LightCyan,
            "White" => Color::White,
            s if s.starts_with('#') => {
                if let Some((r, g, b)) = parse_hex_color(s) {
                    Color::Rgb(r, g, b)
                } else {
                    Color::Reset
                }
            }
            s => {
                if let Ok(index) = s.parse::<u8>() {
                    Color::Indexed(index)
                } else {
                    Color::Reset
                }
            }
        }
    }

    // Color accessors
    pub fn primary(&self) -> Color {
        Self::parse_color(&self.primary)
    }

    pub fn secondary(&self) -> Color {
        Self::parse_color(&self.secondary)
    }

    pub fn success(&self) -> Color {
        Self::parse_color(&self.success)
    }

    pub fn warning(&self) -> Color {
        Self::parse_color(&self.warning)
    }

    pub fn error(&self) -> Color {
        Self::parse_color(&self.error)
    }

    pub fn text(&self) -> Color {
        Self::parse_color(&self.text)
    }

    pub fn text_muted(&self) -> Color {
        Self::parse_color(&self.text_muted)
    }

    pub fn text_disabled(&self) -> Color {
        Self::parse_color(&self.text_disabled)
    }

    pub fn border_focused(&self) -> Color {
        Self::parse_color(&self.border_focused)
    }

    pub fn border_normal(&self) -> Color {
        Self::parse_color(&self.border_normal)
    }

    /// Border highlight color (alias for border_focused)
    pub fn border_highlight(&self) -> Color {
        Self::parse_color(&self.border_focused)
    }

    /// Background color (dark terminals default to black)
    pub fn background(&self) -> Color {
        Color::Black
    }

    // Style helpers
    pub fn track_style(&self, is_selected: bool, is_playing: bool) -> Style {
        match (is_selected, is_playing) {
            (true, true) => Style::default()
                .fg(self.success())
                .add_modifier(Modifier::BOLD),
            (true, false) => Style::default()
                .fg(self.warning())
                .add_modifier(Modifier::BOLD),
            (false, true) => Style::default().fg(self.success()),
            (false, false) => Style::default(),
        }
    }

    pub fn highlight_style(&self) -> Style {
        Style::default()
            .fg(self.warning())
            .add_modifier(Modifier::BOLD)
    }

    pub fn border_style(&self, focused: bool) -> Style {
        if focused {
            Style::default().fg(self.border_focused())
        } else {
            Style::default().fg(self.border_normal())
        }
    }

    // Built-in theme presets
    pub fn catppuccin_mocha() -> Self {
        Self {
            name: "catppuccin-mocha".to_string(),
            primary: "#89b4fa".to_string(),    // Blue
            secondary: "#cba6f7".to_string(),  // Mauve
            success: "#a6e3a1".to_string(),    // Green
            warning: "#f9e2af".to_string(),    // Yellow
            error: "#f38ba8".to_string(),      // Red
            text: "#cdd6f4".to_string(),       // Text
            text_muted: "#a6adc8".to_string(), // Subtext0
            text_disabled: "#6c7086".to_string(), // Overlay0
            border_focused: "#89b4fa".to_string(),
            border_normal: "#585b70".to_string(), // Surface2
        }
    }

    pub fn dracula() -> Self {
        Self {
            name: "dracula".to_string(),
            primary: "#8be9fd".to_string(),    // Cyan
            secondary: "#ff79c6".to_string(),  // Pink
            success: "#50fa7b".to_string(),    // Green
            warning: "#f1fa8c".to_string(),    // Yellow
            error: "#ff5555".to_string(),      // Red
            text: "#f8f8f2".to_string(),       // Foreground
            text_muted: "#6272a4".to_string(), // Comment
            text_disabled: "#44475a".to_string(), // Current Line
            border_focused: "#bd93f9".to_string(), // Purple
            border_normal: "#44475a".to_string(),
        }
    }

    pub fn nord() -> Self {
        Self {
            name: "nord".to_string(),
            primary: "#88c0d0".to_string(),    // Nord8 (cyan)
            secondary: "#b48ead".to_string(),  // Nord15 (purple)
            success: "#a3be8c".to_string(),    // Nord14 (green)
            warning: "#ebcb8b".to_string(),    // Nord13 (yellow)
            error: "#bf616a".to_string(),      // Nord11 (red)
            text: "#eceff4".to_string(),       // Nord6
            text_muted: "#d8dee9".to_string(), // Nord4
            text_disabled: "#4c566a".to_string(), // Nord3
            border_focused: "#81a1c1".to_string(), // Nord9
            border_normal: "#3b4252".to_string(), // Nord1
        }
    }

    pub fn gruvbox() -> Self {
        Self {
            name: "gruvbox".to_string(),
            primary: "#83a598".to_string(),    // Blue
            secondary: "#d3869b".to_string(),  // Purple
            success: "#b8bb26".to_string(),    // Green
            warning: "#fabd2f".to_string(),    // Yellow
            error: "#fb4934".to_string(),      // Red
            text: "#ebdbb2".to_string(),       // fg
            text_muted: "#a89984".to_string(), // gray
            text_disabled: "#665c54".to_string(), // bg2
            border_focused: "#fe8019".to_string(), // Orange
            border_normal: "#504945".to_string(), // bg1
        }
    }

    pub fn tokyo_night() -> Self {
        Self {
            name: "tokyo-night".to_string(),
            primary: "#7aa2f7".to_string(),    // Blue
            secondary: "#bb9af7".to_string(),  // Magenta
            success: "#9ece6a".to_string(),    // Green
            warning: "#e0af68".to_string(),    // Yellow
            error: "#f7768e".to_string(),      // Red
            text: "#c0caf5".to_string(),       // Foreground
            text_muted: "#565f89".to_string(), // Comment
            text_disabled: "#414868".to_string(), // Terminal black
            border_focused: "#7dcfff".to_string(), // Cyan
            border_normal: "#3b4261".to_string(),
        }
    }

    pub fn from_preset(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "default" => Some(Self::default()),
            "catppuccin" | "catppuccin-mocha" => Some(Self::catppuccin_mocha()),
            "dracula" => Some(Self::dracula()),
            "nord" => Some(Self::nord()),
            "gruvbox" => Some(Self::gruvbox()),
            "tokyo-night" | "tokyonight" => Some(Self::tokyo_night()),
            _ => None,
        }
    }
}

fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;

    Some((r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_named_colors() {
        assert!(matches!(Theme::parse_color("Cyan"), Color::Cyan));
        assert!(matches!(Theme::parse_color("Red"), Color::Red));
        assert!(matches!(Theme::parse_color("Reset"), Color::Reset));
    }

    #[test]
    fn test_parse_hex_colors() {
        assert!(matches!(
            Theme::parse_color("#ff0000"),
            Color::Rgb(255, 0, 0)
        ));
        assert!(matches!(
            Theme::parse_color("#00ff00"),
            Color::Rgb(0, 255, 0)
        ));
    }

    #[test]
    fn test_parse_indexed_colors() {
        assert!(matches!(Theme::parse_color("42"), Color::Indexed(42)));
    }

    #[test]
    fn test_theme_presets() {
        assert!(Theme::from_preset("catppuccin").is_some());
        assert!(Theme::from_preset("dracula").is_some());
        assert!(Theme::from_preset("nord").is_some());
        assert!(Theme::from_preset("gruvbox").is_some());
        assert!(Theme::from_preset("tokyo-night").is_some());
        assert!(Theme::from_preset("nonexistent").is_none());
    }

    #[test]
    fn test_theme_serialization() {
        let theme = Theme::default();
        let toml = toml::to_string_pretty(&theme).unwrap();
        let deserialized: Theme = toml::from_str(&toml).unwrap();
        assert_eq!(theme.name, deserialized.name);
        assert_eq!(theme.primary, deserialized.primary);
    }
}
