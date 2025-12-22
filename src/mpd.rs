use anyhow::Result;
use std::time::Duration;
use std::process::Command;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct QueueItem {
    pub position: usize,
    pub artist: String,
    pub title: String,
    pub duration: String,
}

#[derive(Debug, Clone)]
pub struct CurrentSong {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration: Duration,
    pub elapsed: Duration,
}

// Helper to parse duration from "MM:SS" format
fn parse_duration(time_str: &str) -> Duration {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() == 2 {
        let minutes = parts[0].parse::<u64>().unwrap_or(0);
        let seconds = parts[1].parse::<u64>().unwrap_or(0);
        Duration::from_secs(minutes * 60 + seconds)
    } else {
        Duration::from_secs(0)
    }
}

pub struct MpdController {
    _is_connected: bool,
    host: String,
    port: u16,
}

impl MpdController {
    pub async fn new(debug_log: &mut VecDeque<String>) -> Result<Self> {
        Self::with_config("localhost", 6600, debug_log).await
    }

    pub async fn with_config(host: &str, port: u16, debug_log: &mut VecDeque<String>) -> Result<Self> {
        // Check if MPD is running
        let output = Command::new("mpc")
            .arg("-h")
            .arg(host)
            .arg("-p")
            .arg(port.to_string())
            .arg("status")
            .output()?;

        let is_connected = output.status.success();
        if is_connected {
            debug_log.push_back(format!("✓ Connected to MPD at {}:{}", host, port));
            let status_str = String::from_utf8_lossy(&output.stdout);
            for line in status_str.lines().take(2) {
                debug_log.push_back(format!("  {}", line));
            }
        } else {
            debug_log.push_back(format!("✗ Could not connect to MPD at {}:{}", host, port));
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("  Error: {}", error));
        }

        Ok(Self {
            _is_connected: is_connected,
            host: host.to_string(),
            port,
        })
    }

    /// Build mpc command with host/port args
    fn mpc_cmd(&self) -> Command {
        let mut cmd = Command::new("mpc");
        cmd.arg("-h").arg(&self.host);
        cmd.arg("-p").arg(self.port.to_string());
        cmd
    }


    pub async fn add_track(&mut self, url: &str, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back(format!("Executing: mpc add \"{}\"", &url[..100.min(url.len())]));

        let output = self.mpc_cmd()
            .arg("add")
            .arg(url)
            .output()?;

        if output.status.success() {
            debug_log.push_back("✓ Track added to MPD queue".to_string());
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.is_empty() {
                debug_log.push_back(format!("  MPD response: {}", stdout.trim()));
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            debug_log.push_back(format!("✗ Failed to add track"));
            if !stderr.is_empty() {
                debug_log.push_back(format!("  Error: {}", stderr.trim()));
            }
            if !stdout.is_empty() {
                debug_log.push_back(format!("  Output: {}", stdout.trim()));
            }
            // Return an error so the caller knows it failed
            return Err(anyhow::anyhow!("Failed to add track to MPD: {}", stderr));
        }
        Ok(())
    }

    pub async fn play(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc play".to_string());

        let output = self.mpc_cmd().arg("play").output()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("[playing]") {
                debug_log.push_back("✓ Playback started".to_string());
                // Log the first line (track info)
                if let Some(track_line) = stdout.lines().next() {
                    debug_log.push_back(format!("  Now playing: {}", track_line));
                }
            } else {
                debug_log.push_back("⚠ Play command sent but status unclear".to_string());
                debug_log.push_back(format!("  Output: {}", stdout.lines().next().unwrap_or("")));
            }
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to start playback: {}", error));
        }
        Ok(())
    }

    pub async fn play_position(&mut self, position: usize, debug_log: &mut VecDeque<String>) -> Result<()> {
        // MPD positions are 1-based, so add 1
        let pos = position + 1;
        debug_log.push_back(format!("Executing: mpc play {}", pos));

        let output = self.mpc_cmd()
            .arg("play")
            .arg(pos.to_string())
            .output()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            debug_log.push_back(format!("✓ Playing track at position {}", pos));
            if let Some(track_line) = stdout.lines().next() {
                debug_log.push_back(format!("  Now playing: {}", track_line));
            }
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to play position {}: {}", pos, error));
        }
        Ok(())
    }

    pub async fn pause(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc pause".to_string());

        let output = self.mpc_cmd().arg("pause").output()?;

        if output.status.success() {
            debug_log.push_back("✓ Playback paused".to_string());
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to pause: {}", error));
        }
        Ok(())
    }

    pub async fn stop(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc stop".to_string());

        let output = self.mpc_cmd().arg("stop").output()?;

        if output.status.success() {
            debug_log.push_back("✓ Playback stopped".to_string());
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to stop: {}", error));
        }
        Ok(())
    }

    pub async fn next(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc next".to_string());

        let output = self.mpc_cmd().arg("next").output()?;

        if output.status.success() {
            debug_log.push_back("✓ Skipped to next track".to_string());
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(track_line) = stdout.lines().next() {
                debug_log.push_back(format!("  Now playing: {}", track_line));
            }
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to skip: {}", error));
        }
        Ok(())
    }

    pub async fn previous(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc prev".to_string());

        let output = self.mpc_cmd().arg("prev").output()?;

        if output.status.success() {
            debug_log.push_back("✓ Went to previous track".to_string());
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(track_line) = stdout.lines().next() {
                debug_log.push_back(format!("  Now playing: {}", track_line));
            }
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to go back: {}", error));
        }
        Ok(())
    }

    pub async fn get_status(&mut self, _debug_log: &mut VecDeque<String>) -> Result<PlayerStatus> {
        let output = self.mpc_cmd().arg("status").output()?;
        let status_str = String::from_utf8_lossy(&output.stdout);

        let is_playing = status_str.contains("[playing]");
        let current_track = if status_str.lines().count() > 1 {
            Some(status_str.lines().next().unwrap_or("Unknown").to_string())
        } else {
            None
        };

        // Parse volume if present
        let volume = if let Some(volume_line) = status_str.lines().find(|l| l.contains("volume:")) {
            // Extract just the volume value (format: "volume: 65%   repeat: off...")
            volume_line
                .split("volume:")
                .nth(1)
                .and_then(|s| s.trim().split_whitespace().next())
                .and_then(|v| v.trim_end_matches('%').parse::<u8>().ok())
        } else {
            Some(50)
        };

        // Parse playback modes from the status line
        // Format: "volume: 50%   repeat: off   random: off   single: off   consume: off"
        let repeat = status_str.contains("repeat: on");
        let random = status_str.contains("random: on");
        let single = status_str.contains("single: on");

        Ok(PlayerStatus {
            is_playing,
            current_track,
            volume,
            elapsed: None,
            duration: None,
            repeat,
            random,
            single,
        })
    }

    pub async fn set_volume(&mut self, volume: u8, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back(format!("Executing: mpc volume {}", volume));

        let output = self.mpc_cmd()
            .arg("volume")
            .arg(volume.to_string())
            .output()?;

        if output.status.success() {
            debug_log.push_back(format!("✓ Volume set to {}%", volume));
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to set volume: {}", error));
        }
        Ok(())
    }

    pub async fn volume_up(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc volume +5".to_string());

        let output = self.mpc_cmd()
            .arg("volume")
            .arg("+5")
            .output()?;

        if output.status.success() {
            debug_log.push_back("✓ Volume increased".to_string());
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to increase volume: {}", error));
        }
        Ok(())
    }

    pub async fn volume_down(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc volume -5".to_string());

        let output = self.mpc_cmd()
            .arg("volume")
            .arg("-5")
            .output()?;

        if output.status.success() {
            debug_log.push_back("✓ Volume decreased".to_string());
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to decrease volume: {}", error));
        }
        Ok(())
    }

    pub async fn seek_forward(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc seek +10".to_string());

        let output = self.mpc_cmd()
            .arg("seek")
            .arg("+10")
            .output()?;

        if output.status.success() {
            debug_log.push_back("✓ Seeked forward 10s".to_string());
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to seek: {}", error));
        }
        Ok(())
    }

    pub async fn seek_backward(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc seek -10".to_string());

        let output = self.mpc_cmd()
            .arg("seek")
            .arg("-10")
            .output()?;

        if output.status.success() {
            debug_log.push_back("✓ Seeked backward 10s".to_string());
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to seek: {}", error));
        }
        Ok(())
    }

    pub async fn seek_to(&mut self, seconds: u32, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back(format!("Executing: mpc seek {}", seconds));

        let output = self.mpc_cmd()
            .arg("seek")
            .arg(seconds.to_string())
            .output()?;

        if output.status.success() {
            debug_log.push_back(format!("✓ Seeked to {}s", seconds));
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to seek: {}", error));
        }
        Ok(())
    }

    pub async fn toggle_repeat(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc repeat".to_string());

        let output = self.mpc_cmd()
            .arg("repeat")
            .output()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("On") {
                debug_log.push_back("✓ Repeat enabled".to_string());
            } else {
                debug_log.push_back("✓ Repeat disabled".to_string());
            }
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to toggle repeat: {}", error));
        }
        Ok(())
    }

    pub async fn toggle_random(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc random".to_string());

        let output = self.mpc_cmd()
            .arg("random")
            .output()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("On") {
                debug_log.push_back("✓ Random/shuffle enabled".to_string());
            } else {
                debug_log.push_back("✓ Random/shuffle disabled".to_string());
            }
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to toggle random: {}", error));
        }
        Ok(())
    }

    pub async fn toggle_single(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Executing: mpc single".to_string());

        let output = self.mpc_cmd()
            .arg("single")
            .output()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("On") {
                debug_log.push_back("✓ Single track repeat enabled".to_string());
            } else {
                debug_log.push_back("✓ Single track repeat disabled".to_string());
            }
        } else {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to toggle single: {}", error));
        }
        Ok(())
    }

    // Debug helper to check current MPD queue
    pub async fn debug_queue(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        debug_log.push_back("Checking MPD queue...".to_string());

        let output = self.mpc_cmd().arg("playlist").output()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let count = stdout.lines().count();
            debug_log.push_back(format!("  Queue has {} tracks", count));
            if count > 0 {
                if let Some(first) = stdout.lines().next() {
                    debug_log.push_back(format!("  First: {}", &first[..100.min(first.len())]));
                }
            }
        }
        Ok(())
    }

    // Get the current queue
    pub async fn get_queue(&mut self) -> Result<Vec<QueueItem>> {
        // First try to get with metadata
        let output = self.mpc_cmd()
            .args(&["playlist", "-f", "%artist%|||%title%|||%album%|||%time%"])
            .output()?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // If we got metadata, use it
        if stdout.contains("|||") && !stdout.trim().is_empty() {
            let mut queue = Vec::new();
            for (i, line) in stdout.lines().enumerate() {
                let parts: Vec<&str> = line.split("|||").collect();
                if parts.len() >= 3 {
                    queue.push(QueueItem {
                        position: i + 1,
                        artist: parts[0].to_string(),
                        title: parts[1].to_string(),
                        duration: parts.get(3).unwrap_or(&"0:00").to_string(),
                    });
                }
            }
            return Ok(queue);
        }

        // No metadata available, just get the raw playlist
        let output = self.mpc_cmd().arg("playlist").output()?;
        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut queue = Vec::new();

        for (i, line) in stdout.lines().enumerate() {
            if !line.trim().is_empty() {
                // Extract filename or show URL excerpt for streaming
                let display_name = if line.starts_with("http") {
                    format!("Stream {}", i + 1)
                } else {
                    line.rsplit('/').next().unwrap_or(line).to_string()
                };

                queue.push(QueueItem {
                    position: i + 1,
                    artist: "Unknown".to_string(),
                    title: display_name,
                    duration: "0:00".to_string(),
                });
            }
        }

        Ok(queue)
    }

    // Get just timing info (elapsed/duration) from MPD status
    pub async fn get_timing_info(&mut self) -> Result<(Duration, Duration)> {
        let status_output = self.mpc_cmd().arg("status").output()?;
        let status_str = String::from_utf8_lossy(&status_output.stdout);

        let mut elapsed = Duration::from_secs(0);
        let mut total = Duration::from_secs(0);

        // Parse elapsed/duration from status line
        // Format: "[playing] #1/5   0:45/3:20 (22%)"
        for line in status_str.lines() {
            if line.contains("/") && line.contains(":") {
                if let Some(time_part) = line.split_whitespace().nth(2) {
                    let times: Vec<&str> = time_part.split('/').collect();
                    if times.len() == 2 {
                        elapsed = parse_duration(times[0]);
                        total = parse_duration(times[1]);
                        break;
                    }
                }
            }
        }

        Ok((elapsed, total))
    }

    /// Get current queue position (0-indexed) and elapsed time
    /// Returns (position, elapsed_seconds) or None if not playing
    pub async fn get_playback_position(&mut self) -> Result<Option<(usize, u32)>> {
        let status_output = self.mpc_cmd().arg("status").output()?;
        let status_str = String::from_utf8_lossy(&status_output.stdout);

        // Format: "[playing] #1/5   0:45/3:20 (22%)"
        for line in status_str.lines() {
            if line.contains("[playing]") || line.contains("[paused]") {
                // Extract position: "#1/5" -> 1 (then convert to 0-indexed)
                if let Some(pos_part) = line.split('#').nth(1) {
                    if let Some(pos_str) = pos_part.split('/').next() {
                        if let Ok(pos) = pos_str.parse::<usize>() {
                            // Extract elapsed time
                            let mut elapsed_secs = 0u32;
                            if let Some(time_part) = line.split_whitespace().nth(2) {
                                if let Some(elapsed_str) = time_part.split('/').next() {
                                    let duration = parse_duration(elapsed_str);
                                    elapsed_secs = duration.as_secs() as u32;
                                }
                            }
                            // Convert to 0-indexed
                            return Ok(Some((pos.saturating_sub(1), elapsed_secs)));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    // Get detailed current playing info
    pub async fn get_current_song(&mut self) -> Result<Option<CurrentSong>> {
        let output = self.mpc_cmd()
            .args(&["current", "-f", "%artist%|||%title%|||%album%|||%time%"])
            .output()?;

        if !output.status.success() || output.stdout.is_empty() {
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = stdout.trim().split("|||").collect();

        if parts.len() >= 4 {
            // Get status for elapsed/duration
            let status_output = self.mpc_cmd().arg("status").output()?;
            let status_str = String::from_utf8_lossy(&status_output.stdout);

            let mut elapsed = Duration::from_secs(0);
            let mut total = Duration::from_secs(0);

            // Parse elapsed/duration from second line
            for line in status_str.lines() {
                if line.contains("/") && line.contains(":") {
                    // Format: "[playing] #1/5   0:45/3:20 (22%)"
                    if let Some(time_part) = line.split_whitespace().nth(2) {
                        let times: Vec<&str> = time_part.split('/').collect();
                        if times.len() == 2 {
                            elapsed = parse_duration(times[0]);
                            total = parse_duration(times[1]);
                        }
                    }
                }
            }

            Ok(Some(CurrentSong {
                artist: parts[0].to_string(),
                title: parts[1].to_string(),
                album: parts[2].to_string(),
                duration: total,
                elapsed,
            }))
        } else {
            Ok(None)
        }
    }

    // Clear the queue
    pub async fn clear_queue(&mut self, debug_log: &mut VecDeque<String>) -> Result<()> {
        let output = self.mpc_cmd().arg("clear").output()?;

        if output.status.success() {
            debug_log.push_back("✓ Queue cleared".to_string());
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to clear queue: {}", stderr));
        }

        Ok(())
    }

    /// Get the number of remaining tracks in the queue (after current position)
    pub async fn get_remaining_queue_count(&mut self) -> Result<usize> {
        let status_output = self.mpc_cmd().arg("status").output()?;
        let status_str = String::from_utf8_lossy(&status_output.stdout);

        // Get queue length
        let playlist_output = self.mpc_cmd().arg("playlist").output()?;
        let playlist_str = String::from_utf8_lossy(&playlist_output.stdout);
        let queue_len = playlist_str.lines().count();

        if queue_len == 0 {
            return Ok(0);
        }

        // Parse current position from status
        // Format: "[playing] #1/5   0:45/3:20 (22%)"
        for line in status_str.lines() {
            if line.contains("[playing]") || line.contains("[paused]") {
                if let Some(pos_part) = line.split('#').nth(1) {
                    if let Some(pos_str) = pos_part.split('/').next() {
                        if let Ok(pos) = pos_str.parse::<usize>() {
                            // pos is 1-indexed, queue_len is count
                            // remaining = total - current position
                            return Ok(queue_len.saturating_sub(pos));
                        }
                    }
                }
            }
        }

        // If not playing, all tracks are "remaining"
        Ok(queue_len)
    }

    // Remove track from queue by position
    pub async fn remove_from_queue(&mut self, position: usize, debug_log: &mut VecDeque<String>) -> Result<()> {
        let output = self.mpc_cmd()
            .arg("del")
            .arg(position.to_string())
            .output()?;

        if output.status.success() {
            debug_log.push_back(format!("✓ Removed track at position {}", position));
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug_log.push_back(format!("✗ Failed to remove track: {}", stderr));
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct PlayerStatus {
    pub is_playing: bool,
    pub current_track: Option<String>,
    pub volume: Option<u8>,
    pub elapsed: Option<Duration>,
    pub duration: Option<Duration>,
    pub repeat: bool,
    pub random: bool,
    pub single: bool,
}