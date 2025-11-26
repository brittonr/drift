use std::process::{Command, Stdio, Child};
use std::sync::{Arc, Mutex};
use std::thread;
use anyhow::Result;

pub struct CavaVisualizer {
    bars: Arc<Mutex<Vec<u8>>>,
    process: Option<Child>,
}

impl CavaVisualizer {
    pub fn new() -> Result<Self> {
        let bars = Arc::new(Mutex::new(vec![0; 20]));

        Ok(Self {
            bars,
            process: None,
        })
    }

    pub fn start(&mut self) -> Result<()> {
        // Start cava process
        let bars_clone = Arc::clone(&self.bars);

        // Try to find cava in PATH or use a common nix store path
        let cava_cmd = which::which("cava")
            .unwrap_or_else(|_| std::path::PathBuf::from("cava"));

        let mut child = Command::new(cava_cmd)
            .arg("-p")
            .arg("/home/brittonr/git/tidal-tui/cava_config")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdout = child.stdout.take().expect("Failed to get cava stdout");

        // Spawn thread to read cava output
        thread::spawn(move || {
            use std::io::Read;
            let mut reader = stdout;
            let mut buffer = [0u8; 20];

            loop {
                // Cava outputs raw bytes continuously (20 bytes per frame)
                match reader.read_exact(&mut buffer) {
                    Ok(_) => {
                        // Convert raw bytes (0-255) to bar heights (0-7 range)
                        let values: Vec<u8> = buffer.iter()
                            .map(|&b| {
                                // Scale from 0-255 to 0-7
                                (b / 32).min(7)
                            })
                            .collect();

                        if let Ok(mut bars) = bars_clone.lock() {
                            *bars = values;
                        }
                    }
                    Err(_) => {
                        // Cava process ended or error occurred
                        break;
                    }
                }
            }
        });

        self.process = Some(child);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
    }

    pub fn get_bars(&self) -> Vec<u8> {
        match self.bars.lock() {
            Ok(bars) => bars.clone(),
            Err(_) => vec![0; 20],
        }
    }


    pub fn draw_bars(&self) -> String {
        let bars = self.get_bars();
        let bar_chars = vec!['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

        bars.iter()
            .map(|&height| {
                let index = (height as usize).min(bar_chars.len() - 1);
                bar_chars[index]
            })
            .collect::<String>()
    }
}

impl Drop for CavaVisualizer {
    fn drop(&mut self) {
        self.stop();
    }
}