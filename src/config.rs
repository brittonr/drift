use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const CONFIG_FILE_NAME: &str = "config.toml";

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mpd: MpdConfig,
    pub playback: PlaybackConfig,
    pub ui: UiConfig,
    pub downloads: DownloadsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mpd: MpdConfig::default(),
            playback: PlaybackConfig::default(),
            ui: UiConfig::default(),
            downloads: DownloadsConfig::default(),
        }
    }
}

/// MPD connection settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MpdConfig {
    /// MPD host address
    pub host: String,
    /// MPD port
    pub port: u16,
}

impl Default for MpdConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 6600,
        }
    }
}

/// Playback preferences
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackConfig {
    /// Default volume (0-100)
    pub default_volume: u8,
    /// Audio quality: "low", "high", "lossless", "master"
    pub audio_quality: String,
    /// Resume playback on startup
    pub resume_on_startup: bool,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            default_volume: 80,
            audio_quality: "high".to_string(),
            resume_on_startup: true,
        }
    }
}

/// UI customization
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// Show audio visualizer
    pub show_visualizer: bool,
    /// Show album art
    pub show_album_art: bool,
    /// Number of visualizer bars
    pub visualizer_bars: u8,
    /// Status check interval in milliseconds
    pub status_interval_ms: u64,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_visualizer: true,
            show_album_art: true,
            visualizer_bars: 20,
            status_interval_ms: 200,
        }
    }
}

/// Downloads settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadsConfig {
    /// Maximum concurrent downloads
    pub max_concurrent: usize,
    /// Download directory (empty = default cache dir)
    pub download_dir: Option<String>,
    /// Auto-tag downloaded files with metadata
    pub auto_tag: bool,
}

impl Default for DownloadsConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 2,
            download_dir: None,
            auto_tag: true,
        }
    }
}

impl Config {
    /// Get the configuration file path
    pub fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Failed to get config directory")?
            .join("tidal-tui");

        fs::create_dir_all(&config_dir)
            .context("Failed to create config directory")?;

        Ok(config_dir.join(CONFIG_FILE_NAME))
    }

    /// Load configuration from file, or create default if not exists
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;

        if path.exists() {
            let contents = fs::read_to_string(&path)
                .context("Failed to read config file")?;

            let config: Config = toml::from_str(&contents)
                .context("Failed to parse config file")?;

            Ok(config)
        } else {
            // Create default config and save it
            let config = Config::default();
            config.save()?;
            Ok(config)
        }
    }

    /// Save configuration to file
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;

        let contents = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;

        fs::write(&path, contents)
            .context("Failed to write config file")?;

        Ok(())
    }

    /// Generate example config content for documentation
    pub fn example_config() -> String {
        let config = Config::default();
        toml::to_string_pretty(&config).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();

        assert_eq!(config.mpd.host, "localhost");
        assert_eq!(config.mpd.port, 6600);
        assert_eq!(config.playback.default_volume, 80);
        assert_eq!(config.playback.audio_quality, "high");
        assert!(config.playback.resume_on_startup);
        assert!(config.ui.show_visualizer);
        assert!(config.ui.show_album_art);
        assert_eq!(config.ui.visualizer_bars, 20);
        assert_eq!(config.downloads.max_concurrent, 2);
        assert!(config.downloads.auto_tag);
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();

        assert_eq!(config.mpd.host, deserialized.mpd.host);
        assert_eq!(config.mpd.port, deserialized.mpd.port);
        assert_eq!(config.playback.default_volume, deserialized.playback.default_volume);
    }

    #[test]
    fn test_partial_config_uses_defaults() {
        let partial_toml = r#"
[mpd]
host = "192.168.1.100"
"#;

        let config: Config = toml::from_str(partial_toml).unwrap();

        // Custom value
        assert_eq!(config.mpd.host, "192.168.1.100");
        // Default values
        assert_eq!(config.mpd.port, 6600);
        assert_eq!(config.playback.default_volume, 80);
        assert!(config.ui.show_visualizer);
    }

    #[test]
    fn test_full_config_parsing() {
        let full_toml = r#"
[mpd]
host = "remote-server"
port = 6601

[playback]
default_volume = 50
audio_quality = "lossless"
resume_on_startup = false

[ui]
show_visualizer = false
show_album_art = true
visualizer_bars = 30
status_interval_ms = 500

[downloads]
max_concurrent = 4
download_dir = "/custom/path"
auto_tag = false
"#;

        let config: Config = toml::from_str(full_toml).unwrap();

        assert_eq!(config.mpd.host, "remote-server");
        assert_eq!(config.mpd.port, 6601);
        assert_eq!(config.playback.default_volume, 50);
        assert_eq!(config.playback.audio_quality, "lossless");
        assert!(!config.playback.resume_on_startup);
        assert!(!config.ui.show_visualizer);
        assert!(config.ui.show_album_art);
        assert_eq!(config.ui.visualizer_bars, 30);
        assert_eq!(config.ui.status_interval_ms, 500);
        assert_eq!(config.downloads.max_concurrent, 4);
        assert_eq!(config.downloads.download_dir, Some("/custom/path".to_string()));
        assert!(!config.downloads.auto_tag);
    }

    #[test]
    fn test_example_config_is_valid() {
        let example = Config::example_config();
        let parsed: Result<Config, _> = toml::from_str(&example);
        assert!(parsed.is_ok(), "Example config should be valid TOML");
    }

    #[test]
    fn test_invalid_toml_returns_error() {
        let invalid_toml = "this is not valid [[ toml";
        let result: Result<Config, _> = toml::from_str(invalid_toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_with_unknown_fields_is_ignored() {
        let toml_with_extra = r#"
[mpd]
host = "localhost"
unknown_field = "should be ignored"

[unknown_section]
foo = "bar"
"#;

        // This should not fail - unknown fields are ignored by default with serde
        let result: Result<Config, _> = toml::from_str(toml_with_extra);
        // Note: by default serde will error on unknown fields unless we use #[serde(deny_unknown_fields)]
        // Since we didn't add that, this should succeed
        assert!(result.is_ok());
    }
}
