use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use dirs::config_dir;

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedCredentials {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub user_id: Option<i64>,
}

pub struct TidalAuth;

impl TidalAuth {
    pub fn config_path() -> Result<PathBuf> {
        let mut path = config_dir()
            .ok_or_else(|| anyhow!("Could not find config directory"))?;
        path.push("drift");
        fs::create_dir_all(&path)?;
        path.push("credentials.json");
        Ok(path)
    }

    pub fn load_credentials() -> Result<SavedCredentials> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Err(anyhow!("No saved credentials found"));
        }

        let contents = fs::read_to_string(&path)?;
        let creds: SavedCredentials = serde_json::from_str(&contents)?;
        Ok(creds)
    }

    pub fn save_credentials(creds: &SavedCredentials) -> Result<()> {
        let path = Self::config_path()?;
        let contents = serde_json::to_string_pretty(creds)?;
        fs::write(&path, contents)?;
        Ok(())
    }
}