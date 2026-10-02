//! Application configuration and authentication credential persistence.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

fn default_download_threads() -> usize {
    2
}

/// Application settings stored in ~/.config/seedr-dl/config.json
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Directory where downloaded files are saved.
    pub download_dir: PathBuf,
    /// Whether to delete cloud files after downloading.
    pub delete_after_download: bool,
    /// Number of concurrent download worker threads.
    #[serde(default = "default_download_threads")]
    pub download_threads: usize,
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let default_dir = Path::new(&home).join("Downloads");
        Self {
            download_dir: if default_dir.exists() {
                default_dir
            } else {
                PathBuf::from(".")
            },
            delete_after_download: false,
            download_threads: default_download_threads(),
        }
    }
}

/// Authentication credentials stored in ~/.config/seedr-dl/auth.json
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Auth {
    /// Seedr API access token.
    pub access_token: String,
    /// Seedr API refresh token.
    pub refresh_token: Option<String>,
}

/// Returns application config directory (~/.config/seedr-dl).
#[must_use]
pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    Path::new(&home).join(".config/seedr-dl")
}

/// Returns base application cache directory (~/.cache/seedr-dl).
#[must_use]
pub fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    Path::new(&home).join(".cache/seedr-dl")
}

/// Returns downloads cache directory (~/.cache/seedr-dl/downloads).
#[must_use]
pub fn downloads_dir() -> PathBuf {
    cache_dir().join("downloads")
}

/// Loads configuration from disk.
///
/// # Errors
/// Returns an error if reading or parsing the config file fails.
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

/// Saves configuration to disk.
///
/// # Errors
/// Returns an error if directory creation or file writing fails.
pub fn save_config(config: &Config) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join("config.json");
    let content = serde_json::to_string_pretty(config)?;
    fs::write(&path, content)?;
    Ok(())
}

/// Loads authentication credentials from disk if present.
///
/// # Errors
/// Returns an error if reading or parsing auth.json fails.
pub fn load_auth() -> Result<Option<Auth>> {
    let path = config_dir().join("auth.json");
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)?;
    let auth: Auth = serde_json::from_str(&content)?;
    Ok(Some(auth))
}

/// Saves authentication credentials to disk.
///
/// # Errors
/// Returns an error if directory creation or file writing fails.
pub fn save_auth(auth: &Auth) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join("auth.json");
    let content = serde_json::to_string_pretty(auth)?;
    fs::write(&path, content)?;
    Ok(())
}



/// Interactively prompts the user for Seedr credentials, logs in, and saves auth token.
///
/// # Errors
/// Returns an error if terminal IO fails, credentials are blank, or login fails.
pub async fn interactive_auth() -> Result<Auth> {
    use crate::seedr::SeedrClient;
    use colored::Colorize;
    use std::io::{self, Write};

    print!("  Enter Seedr email: ");
    io::stdout().flush()?;
    let mut email = String::new();
    io::stdin().read_line(&mut email)?;

    print!("  Enter Seedr password: ");
    io::stdout().flush()?;
    let mut pass = String::new();
    io::stdin().read_line(&mut pass)?;

    let email = email.trim();
    let pass = pass.trim();
    if email.is_empty() || pass.is_empty() {
        anyhow::bail!("Email and password cannot be empty");
    }

    println!("  {} Authenticating with Seedr.cc...", "•".dimmed());
    let auth = SeedrClient::login(email, pass).await?;
    save_auth(&auth)?;
    println!(
        "  {} Successfully authenticated and saved token!",
        "✔".green().bold()
    );
    Ok(auth)
}
