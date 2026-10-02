use crate::config::Auth;
use anyhow::{bail, Context, Result};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;

const SEEDR_TOKEN_URL: &str = "https://www.seedr.cc/oauth_test/token.php";
const SEEDR_RESOURCE_URL: &str = "https://www.seedr.cc/oauth_test/resource.php";

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Active torrent being downloaded/cached in Seedr cloud.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SeedrTorrent {
    pub id: u64,
    pub name: String,
    pub progress: Option<f64>,
    pub size: Option<u64>,
    #[serde(default)]
    pub download_rate: Option<u64>,
    #[serde(default)]
    pub seeders: Option<u32>,
    #[serde(default)]
    pub hash: Option<String>,
}

/// Completed cloud folder stored in Seedr account.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SeedrFolder {
    pub id: u64,
    pub name: String,
    pub size: Option<u64>,
}

/// Individual file inside a Seedr cloud folder.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SeedrFile {
    pub id: Option<u64>,
    pub folder_file_id: Option<u64>,
    pub name: String,
    pub size: u64,
}

/// Response returned from listing folder or root cloud contents.
#[derive(Debug, Serialize, Deserialize)]
pub struct ListContentsResponse {
    #[serde(default)]
    pub space_max: Option<u64>,
    #[serde(default)]
    pub space_used: Option<u64>,
    #[serde(default)]
    pub torrents: Vec<SeedrTorrent>,
    #[serde(default)]
    pub folders: Vec<SeedrFolder>,
    #[serde(default)]
    pub files: Vec<SeedrFile>,
}

/// Query parameters for waiting for cloud caching.
#[derive(Debug, Clone)]
pub struct CachingQuery<'a> {
    pub torrent_id: u64,
    pub name: Option<&'a str>,
    pub previous_folders: &'a [u64],
}

#[derive(Debug, Deserialize)]
struct GenericResponse {
    #[serde(default)]
    result: Option<serde_json::Value>,
    user_torrent_id: Option<u64>,
    url: Option<String>,
    reason_phrase: Option<String>,
    error: Option<String>,
}

/// Client interacting with the Seedr.cc API.
pub struct SeedrClient {
    client: Client,
    token: String,
}

fn build_seedr_client(timeout_secs: u64) -> Client {
    let seedr_addr = SocketAddr::from(([95, 211, 204, 172], 443));
    Client::builder()
        .resolve("www.seedr.cc", seedr_addr)
        .resolve("seedr.cc", seedr_addr)
        .resolve("stream.seedr.cc", seedr_addr)
        .resolve("direct.seedr.cc", seedr_addr)
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .unwrap_or_default()
}

impl SeedrClient {
    /// Creates a new `SeedrClient` with the given access token.
    #[must_use]
    pub fn new(token: String) -> Self {
        Self {
            client: build_seedr_client(30),
            token,
        }
    }

    /// Authenticates with Seedr.cc using email and password.
    ///
    /// # Errors
    /// Returns an error if the network request fails or credentials are rejected.
    pub async fn login(email: &str, pass: &str) -> Result<Auth> {
        let client = build_seedr_client(30);
        let params = [
            ("grant_type", "password"),
            ("client_id", "seedr_chrome"),
            ("type", "login"),
            ("username", email),
            ("password", pass),
        ];

        let resp = client
            .post(SEEDR_TOKEN_URL)
            .form(&params)
            .send()
            .await
            .context("Failed to connect to Seedr auth endpoint")?;

        let data: TokenResponse = resp.json().await.context("Failed to parse auth response")?;

        if let Some(err) = data.error {
            let desc = data.error_description.unwrap_or_default();
            bail!("Seedr login failed: {err} - {desc}");
        }

        let access_token = data.access_token.context("No access token in response")?;
        Ok(Auth {
            access_token,
            refresh_token: data.refresh_token,
        })
    }

    /// Adds a magnet link to the user's Seedr account.
    ///
    /// # Errors
    /// Returns an error if Seedr rejects the magnet or has insufficient space.
    pub async fn add_magnet(&self, magnet: &str) -> Result<u64> {
        let params = [
            ("func", "add_torrent"),
            ("torrent_magnet", magnet),
            ("access_token", &self.token),
        ];

        let resp = self
            .client
            .post(SEEDR_RESOURCE_URL)
            .form(&params)
            .send()
            .await
            .context("Failed to add magnet to Seedr")?;

        let res: GenericResponse = resp.json().await?;

        if let Some(ref reason) = res.reason_phrase {
            if reason.contains("not_enough_space") {
                bail!("Not enough space in your Seedr cloud account! Free space on seedr.cc.");
            }
            bail!("Seedr rejected magnet: {reason}");
        }

        if let Some(ref err) = res.error {
            bail!("Seedr error: {err}");
        }

        if let Some(id) = res.user_torrent_id {
            return Ok(id);
        }

        bail!("Seedr rejected magnet: {:?}", res.result)
    }

    /// Polls until the torrent finishes caching into a folder on Seedr cloud.
    ///
    /// # Errors
    /// Returns an error if communication with Seedr fails.
    pub async fn wait_for_caching(
        &self,
        query: &CachingQuery<'_>,
        mut on_progress: Option<&mut (dyn FnMut(&SeedrTorrent) + Send)>,
    ) -> Result<Option<SeedrFolder>> {
        let pb = ProgressBar::new_spinner();
        let display_name = query.name.unwrap_or("Torrent");
        pb.set_style(
            ProgressStyle::default_spinner()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
                .template("{spinner:.cyan} {msg}")?,
        );
        pb.set_message(format!("Seedr cloud caching: {display_name}"));

        let mut resolved_name = query.name.map(ToString::to_string);
        let mut poll_secs = 5;
        let mut not_found_attempts = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(poll_secs)).await;
            let list = self.list_root().await?;

            if let Some(torrent) = list.torrents.iter().find(|t| t.id == query.torrent_id) {
                not_found_attempts = 0;
                resolved_name = Some(torrent.name.clone());
                let pct = torrent.progress.unwrap_or(0.0);
                poll_secs = Self::calculate_adaptive_poll_secs(torrent.download_rate.unwrap_or(0));
                pb.set_message(format!(
                    "Seedr cloud caching: {} ({pct:.1}%) [poll: {poll_secs}s]",
                    torrent.name
                ));
                if let Some(ref mut cb) = on_progress {
                    cb(torrent);
                }
                continue;
            }

            let is_match = |f: &SeedrFolder| {
                !query.previous_folders.contains(&f.id)
                    || resolved_name.as_ref().is_some_and(|target| {
                        f.name == *target || f.name.contains(target) || target.contains(&f.name)
                    })
            };
            let candidates: Vec<&SeedrFolder> =
                list.folders.iter().filter(|f| is_match(f)).collect();
            if let Some(folder) = candidates.into_iter().max_by_key(|f| f.size) {
                pb.finish_with_message(format!(
                    "{} Seedr cloud caching complete!",
                    "✔".green().bold()
                ));
                return Ok(Some(folder.clone()));
            }

            not_found_attempts += 1;
            if not_found_attempts < 12 {
                poll_secs = 3;
                continue;
            }

            pb.finish_with_message(format!("{} Ready in Seedr cloud.", "✔".green().bold()));
            return Ok(list.folders.into_iter().max_by_key(|f| f.size));
        }
    }

    const fn calculate_adaptive_poll_secs(rate: u64) -> u64 {
        match rate {
            0..=149_999 => 60,
            150_000..=499_999 => 30,
            500_000..=1_999_999 => 15,
            _ => 5,
        }
    }

    /// Lists the root contents (folders and active torrents) in Seedr cloud.
    ///
    /// # Errors
    /// Returns an error if the network request or response parsing fails.
    pub async fn list_root(&self) -> Result<ListContentsResponse> {
        let url = format!(
            "{SEEDR_RESOURCE_URL}?func=list_contents&access_token={}",
            self.token
        );
        let resp = self.client.get(&url).send().await?;
        let data: ListContentsResponse = resp.json().await?;
        Ok(data)
    }

    /// Lists the files inside a specific Seedr cloud folder.
    ///
    /// # Errors
    /// Returns an error if the folder contents cannot be retrieved.
    pub async fn list_folder(&self, folder_id: u64) -> Result<ListContentsResponse> {
        let url = format!(
            "https://www.seedr.cc/api/folder/{folder_id}?access_token={}",
            self.token
        );
        let resp = self.client.get(&url).send().await?;
        let data: ListContentsResponse = resp.json().await?;
        Ok(data)
    }

    /// Retrieves the direct HTTP download URL for a file in Seedr cloud.
    ///
    /// # Errors
    /// Returns an error if URL retrieval fails.
    pub async fn get_download_url(&self, file_id: u64) -> Result<String> {
        let file_str = file_id.to_string();
        let params = [
            ("func", "fetch_file"),
            ("folder_file_id", &file_str),
            ("access_token", &self.token),
        ];
        let resp = self
            .client
            .post(SEEDR_RESOURCE_URL)
            .form(&params)
            .send()
            .await?;
        let res: GenericResponse = resp.json().await?;
        res.url.context("Download URL not found in Seedr response")
    }

    /// Deletes a folder by ID from the Seedr cloud.
    ///
    /// # Errors
    /// Returns an error if the deletion request fails.
    pub async fn delete_folder(&self, folder_id: u64) -> Result<()> {
        let delete_arr = format!("[{{\"type\":\"folder\",\"id\":{folder_id}}}]");
        self.post_delete(&delete_arr).await
    }

    /// Deletes or cancels an active torrent by ID from the Seedr cloud.
    ///
    /// # Errors
    /// Returns an error if the cancellation request fails.
    pub async fn delete_torrent(&self, torrent_id: u64) -> Result<()> {
        let delete_arr = format!("[{{\"type\":\"torrent\",\"id\":{torrent_id}}}]");
        self.post_delete(&delete_arr).await
    }

    /// Deletes all specified folders from the Seedr cloud.
    ///
    /// # Errors
    /// Returns an error if batch deletion fails.
    pub async fn delete_all_folders(&self, folders: &[SeedrFolder]) -> Result<()> {
        if folders.is_empty() {
            return Ok(());
        }
        let items: Vec<String> = folders
            .iter()
            .map(|f| format!("{{\"type\":\"folder\",\"id\":{}}}", f.id))
            .collect();
        self.post_delete(&format!("[{}]", items.join(","))).await
    }

    async fn post_delete(&self, delete_arr: &str) -> Result<()> {
        let params = [
            ("func", "delete"),
            ("delete_arr", delete_arr),
            ("access_token", &self.token),
        ];
        self.client
            .post(SEEDR_RESOURCE_URL)
            .form(&params)
            .send()
            .await?;
        Ok(())
    }
}

fn decode_pct(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Extracts the display name (`dn=`) parameter from a magnet link if present.
#[must_use]
pub fn extract_magnet_name(magnet_or_url: &str) -> Option<String> {
    let sub = &magnet_or_url[magnet_or_url.find("dn=")? + 3..];
    let end = sub.find('&').unwrap_or(sub.len());
    Some(decode_pct(&sub[..end]))
}

/// Extracts the BTIH hash from a magnet link if present.
#[must_use]
pub fn extract_btih_hash(magnet: &str) -> Option<String> {
    let prefix = "xt=urn:btih:";
    let idx = magnet.find(prefix)?;
    let sub = &magnet[idx + prefix.len()..];
    let end = sub.find('&').unwrap_or(sub.len());
    Some(sub[..end].to_lowercase())
}

#[cfg(test)]
mod tests;
