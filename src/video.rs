use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::config::VideoConfig;

#[derive(Debug, Serialize)]
struct MpvCommand {
    command: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct MpvResponse {
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    error: String,
}

// MpvStatus is populated by get_status; is_idle is prepared for future use
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MpvStatus {
    pub is_playing: bool,
    pub elapsed: Duration,
    pub duration: Duration,
    pub is_idle: bool,
}

impl Default for MpvStatus {
    fn default() -> Self {
        Self {
            is_playing: false,
            elapsed: Duration::from_secs(0),
            duration: Duration::from_secs(0),
            is_idle: true,
        }
    }
}

pub struct MpvController {
    socket_path: PathBuf,
    process: Option<Child>,
    config: VideoConfig,
}

impl MpvController {
    pub fn new(config: &VideoConfig) -> Self {
        Self {
            socket_path: PathBuf::from(&config.socket_path),
            process: None,
            config: config.clone(),
        }
    }

    /// Check if mpv is installed and available
    pub fn is_available() -> bool {
        Command::new("mpv")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Start mpv with a video URL
    pub async fn start(&mut self, url: &str, debug_log: &mut VecDeque<String>) -> Result<()> {
        // Stop any existing instance first
        self.stop(debug_log).await.ok();

        // Clean up old socket if it exists
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path).ok();
        }

        debug_log.push_back(format!("Starting mpv with URL: {}...", &url[..50.min(url.len())]));

        let mut cmd = Command::new(&self.config.mpv_path);

        // IPC socket for control
        cmd.arg(format!("--input-ipc-server={}", self.socket_path.display()));

        // Keep window open when video ends (allows seeking back)
        cmd.arg("--keep-open=yes");

        // Force a window even for audio-only content
        cmd.arg("--force-window=yes");

        // Hardware acceleration
        cmd.arg(format!("--hwdec={}", self.config.hwdec));

        // Window geometry if configured
        if let Some(ref geometry) = self.config.window_geometry {
            cmd.arg(format!("--geometry={}", geometry));
        }

        // Fullscreen if configured
        if self.config.fullscreen {
            cmd.arg("--fullscreen");
        }

        // The URL to play
        cmd.arg(url);

        // Don't block terminal
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        let child = cmd.spawn().context("Failed to spawn mpv process")?;
        self.process = Some(child);

        debug_log.push_back("mpv process started".to_string());

        // Wait for socket to become available
        for i in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self.socket_path.exists() {
                debug_log.push_back(format!("mpv IPC socket ready after {}ms", (i + 1) * 100));
                return Ok(());
            }
        }

        debug_log.push_back("Warning: mpv socket not ready after 5s".to_string());
        Ok(())
    }

    /// Stop mpv playback and kill the process
    pub async fn stop(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        // Try graceful quit via IPC first
        if self.socket_path.exists() {
            if let Ok(mut stream) = UnixStream::connect(&self.socket_path).await {
                let cmd = MpvCommand {
                    command: vec![Value::String("quit".to_string())],
                };
                let mut json = serde_json::to_string(&cmd)?;
                json.push('\n');
                stream.write_all(json.as_bytes()).await.ok();
                debug_log.push_back("Sent quit command to mpv".to_string());
            }
        }

        // Kill process if still running
        if let Some(ref mut child) = self.process {
            child.kill().ok();
            child.wait().ok();
            debug_log.push_back("mpv process terminated".to_string());
        }
        self.process = None;

        // Clean up socket
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path).ok();
        }

        Ok(())
    }

    /// Pause playback
    #[allow(dead_code)]
    pub async fn pause(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("mpv: pause".to_string());
        self.set_property("pause", Value::Bool(true)).await
    }

    /// Resume playback
    #[allow(dead_code)]
    pub async fn resume(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("mpv: resume".to_string());
        self.set_property("pause", Value::Bool(false)).await
    }

    /// Toggle pause state
    pub async fn toggle_pause(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("mpv: toggle pause".to_string());
        self.send_command(&["cycle", "pause"]).await?;
        Ok(())
    }

    /// Seek forward by seconds
    #[allow(dead_code)]
    pub async fn seek_forward(&mut self, seconds: i64, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back(format!("mpv: seek +{}s", seconds));
        self.send_command(&["seek", &seconds.to_string(), "relative"]).await?;
        Ok(())
    }

    /// Seek backward by seconds
    #[allow(dead_code)]
    pub async fn seek_backward(&mut self, seconds: i64, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back(format!("mpv: seek -{}s", seconds));
        self.send_command(&["seek", &(-seconds).to_string(), "relative"]).await?;
        Ok(())
    }

    /// Seek to absolute position in seconds
    #[allow(dead_code)]
    pub async fn seek_to(&mut self, seconds: f64, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back(format!("mpv: seek to {}s", seconds));
        self.send_command(&["seek", &seconds.to_string(), "absolute"]).await?;
        Ok(())
    }

    /// Set volume (0-100)
    #[allow(dead_code)]
    pub async fn set_volume(&mut self, volume: u8, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back(format!("mpv: volume {}", volume));
        self.set_property("volume", Value::Number(volume.into())).await
    }

    /// Get current playback status
    pub async fn get_status(&mut self) -> Result<MpvStatus> {
        if !self.socket_path.exists() {
            return Ok(MpvStatus::default());
        }

        let paused = self.get_property("pause").await
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let idle = self.get_property("idle-active").await
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let elapsed = self.get_property("time-pos").await
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let duration = self.get_property("duration").await
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        Ok(MpvStatus {
            is_playing: !paused && !idle,
            elapsed: Duration::from_secs_f64(elapsed),
            duration: Duration::from_secs_f64(duration),
            is_idle: idle,
        })
    }

    /// Check if mpv process is still running
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.process {
            match child.try_wait() {
                Ok(None) => true,  // Still running
                Ok(Some(_)) => {
                    self.process = None;
                    false
                }
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// Send a command to mpv via IPC
    async fn send_command(&mut self, args: &[&str]) -> Result<Option<Value>> {
        if !self.socket_path.exists() {
            return Err(anyhow::anyhow!("mpv socket not available"));
        }

        let mut stream = UnixStream::connect(&self.socket_path).await
            .context("Failed to connect to mpv socket")?;

        let cmd = MpvCommand {
            command: args.iter().map(|s| Value::String(s.to_string())).collect(),
        };

        let mut json = serde_json::to_string(&cmd)?;
        json.push('\n');

        stream.write_all(json.as_bytes()).await?;

        // Read response
        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await?;

        let response: MpvResponse = serde_json::from_str(&response_line)
            .unwrap_or(MpvResponse { data: None, error: "parse error".to_string() });

        if response.error != "success" && !response.error.is_empty() {
            return Err(anyhow::anyhow!("mpv error: {}", response.error));
        }

        Ok(response.data)
    }

    /// Get a property from mpv
    async fn get_property(&mut self, name: &str) -> Option<Value> {
        self.send_command(&["get_property", name]).await.ok().flatten()
    }

    /// Set a property on mpv
    #[allow(dead_code)]
    async fn set_property(&mut self, name: &str, value: Value) -> Result<()> {
        if !self.socket_path.exists() {
            return Err(anyhow::anyhow!("mpv socket not available"));
        }

        let mut stream = UnixStream::connect(&self.socket_path).await
            .context("Failed to connect to mpv socket")?;

        let cmd_array = vec![
            Value::String("set_property".to_string()),
            Value::String(name.to_string()),
            value,
        ];

        let cmd = serde_json::json!({ "command": cmd_array });
        let mut json = serde_json::to_string(&cmd)?;
        json.push('\n');

        stream.write_all(json.as_bytes()).await?;

        // Read response
        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await?;

        Ok(())
    }
}

impl Drop for MpvController {
    fn drop(&mut self) {
        // Try to clean up mpv process
        if let Some(ref mut child) = self.process {
            child.kill().ok();
        }
        // Clean up socket file
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path).ok();
        }
    }
}
