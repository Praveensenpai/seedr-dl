use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

fn default_download_threads() -> usize {
    8
}

/// Application settings stored in ~/.config/seedr-dl/config.json
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub jellyfin_media_dir: PathBuf,
    pub gemini_api_key: Option<String>,
    pub delete_after_download: bool,
    #[serde(default = "default_download_threads")]
    pub download_threads: usize,
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        Self {
            jellyfin_media_dir: Path::new(&home).join("jellyfin/media"),
            gemini_api_key: None,
            delete_after_download: false,
            download_threads: default_download_threads(),
        }
    }
}

/// Authentication credentials stored in ~/.config/seedr-dl/auth.json
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Auth {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

/// Returns application config directory (~/.config/seedr-dl).
pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    Path::new(&home).join(".config/seedr-dl")
}

/// Returns base application cache directory (~/.cache/seedr-dl).
pub fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    Path::new(&home).join(".cache/seedr-dl")
}

/// Returns downloads cache directory (~/.cache/seedr-dl/downloads).
pub fn downloads_dir() -> PathBuf {
    cache_dir().join("downloads")
}

pub fn load_config() -> Result<Config> {
    let path = config_dir().join("config.json");
    if !path.exists() {
        return Ok(Config::default());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;
    let config: Config =
        serde_json::from_str(&content).with_context(|| "Failed to parse config.json")?;
    Ok(config)
}

pub fn save_config(config: &Config) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join("config.json");
    let content = serde_json::to_string_pretty(config)?;
    fs::write(&path, content)?;
    Ok(())
}

pub fn load_auth() -> Result<Option<Auth>> {
    let path = config_dir().join("auth.json");
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)?;
    let auth: Auth = serde_json::from_str(&content)?;
    Ok(Some(auth))
}

pub fn save_auth(auth: &Auth) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join("auth.json");
    let content = serde_json::to_string_pretty(auth)?;
    fs::write(&path, content)?;
    Ok(())
}

pub fn get_gemini_key(config: &Config) -> Option<String> {
    if let Ok(key) = std::env::var("GEMINI_API_KEY") {
        if !key.trim().is_empty() {
            return Some(key.trim().to_string());
        }
    }
    config
        .gemini_api_key
        .clone()
        .filter(|k| !k.trim().is_empty())
}
