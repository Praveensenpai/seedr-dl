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
    /// Unique identifier for torrent.
    pub id: u64,
    /// Release name of torrent.
    pub name: String,
    /// Cache progress percentage.
    pub progress: Option<f64>,
    /// File size in bytes.
    pub size: Option<u64>,
}

/// Completed cloud folder stored in Seedr account.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SeedrFolder {
    /// Folder identifier.
    pub id: u64,
    /// Folder display name.
    pub name: String,
    /// Folder size in bytes.
    pub size: Option<u64>,
}

/// Individual file inside a Seedr cloud folder.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SeedrFile {
    /// File ID.
    pub id: Option<u64>,
    /// Specific folder file ID for direct downloads.
    pub folder_file_id: Option<u64>,
    /// File name.
    pub name: String,
    /// Size in bytes.
    pub size: u64,
}

/// Response returned from listing folder or root cloud contents.
#[derive(Debug, Serialize, Deserialize)]
pub struct ListContentsResponse {
    /// Maximum storage available on account.
    #[serde(default)]
    pub space_max: Option<u64>,
    /// Total storage used on account.
    #[serde(default)]
    pub space_used: Option<u64>,
    /// Active caching torrents.
    #[serde(default)]
    pub torrents: Vec<SeedrTorrent>,
    /// Completed cloud folders.
    #[serde(default)]
    pub folders: Vec<SeedrFolder>,
    /// Files inside the requested folder.
    #[serde(default)]
    pub files: Vec<SeedrFile>,
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

        if res
            .result
            .as_ref()
            .is_some_and(|r| r == true || r == "true")
        {
            if let Some(id) = res.user_torrent_id {
                return Ok(id);
            }
        }

        bail!("Seedr rejected magnet: {:?}", res.result)
    }

    /// Polls until the torrent finishes caching into a folder on Seedr cloud.
    ///
    /// # Errors
    /// Returns an error if communication with Seedr fails.
    pub async fn wait_for_caching(
        &self,
        torrent_id: u64,
        name: Option<&str>,
        previous_folders: &[u64],
    ) -> Result<Option<SeedrFolder>> {
        let pb = ProgressBar::new_spinner();
        let display_name = name.unwrap_or("Torrent");
        pb.set_style(
            ProgressStyle::default_spinner()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
                .template("{spinner:.cyan} {msg}")?,
        );
        pb.set_message(format!("Seedr cloud caching: {display_name}"));

        let mut resolved_name = name.map(ToString::to_string);
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let list = self.list_root().await?;

            if let Some(torrent) = list.torrents.iter().find(|t| t.id == torrent_id) {
                resolved_name = Some(torrent.name.clone());
                let pct = torrent.progress.unwrap_or(0.0);
                pb.set_message(format!("Seedr cloud caching: {} ({pct:.1}%)", torrent.name));
                continue;
            }

            if let Some(folder) = list.folders.iter().find(|f| !previous_folders.contains(&f.id)) {
                pb.finish_with_message(format!("{} Seedr cloud caching complete!", "✔".green().bold()));
                return Ok(Some(folder.clone()));
            }

            if let Some(ref target_name) = resolved_name {
                if let Some(folder) = list
                    .folders
                    .iter()
                    .find(|f| f.name == *target_name || f.name.contains(target_name) || target_name.contains(&f.name))
                {
                    pb.finish_with_message(format!("{} Seedr cloud caching complete!", "✔".green().bold()));
                    return Ok(Some(folder.clone()));
                }
            }

            pb.finish_with_message(format!("{} Ready in Seedr cloud.", "✔".green().bold()));
            return Ok(list.folders.into_iter().next());
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
        let delete_arr = format!("[{}]", items.join(","));
        self.post_delete(&delete_arr).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seedr_not_enough_space_response() -> Result<(), Box<dyn std::error::Error>> {
        let json = r#"{"result":false,"reason_phrase":"not_enough_space_added_to_wishlist"}"#;
        let res: GenericResponse = serde_json::from_str(json)?;
        assert!(res.reason_phrase.is_some_and(|r| r.contains("not_enough_space")));
        Ok(())
    }
}
