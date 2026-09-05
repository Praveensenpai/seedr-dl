use crate::config::Auth;
use anyhow::{bail, Context, Result};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use serde::Deserialize;
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

#[derive(Debug, Deserialize, Clone)]
pub struct SeedrTorrent {
    pub id: u64,
    pub name: String,
    pub progress: Option<f64>,
    pub size: Option<u64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SeedrFolder {
    pub id: u64,
    pub name: String,
    pub size: Option<u64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SeedrFile {
    pub id: Option<u64>,
    pub folder_file_id: Option<u64>,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
struct GenericResponse {
    #[serde(default)]
    result: Option<serde_json::Value>,
    user_torrent_id: Option<u64>,
    url: Option<String>,
    reason_phrase: Option<String>,
    error: Option<String>,
}

use std::net::SocketAddr;

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
    pub fn new(token: String) -> Self {
        Self {
            client: build_seedr_client(30),
            token,
        }
    }

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
                bail!(
                    "Not enough space in your Seedr cloud account! Run 'seedr-dl' to download and delete existing files, or free space on seedr.cc."
                );
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

    pub async fn wait_for_caching(
        &self,
        torrent_id: u64,
        name: &str,
    ) -> Result<Option<SeedrFolder>> {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
                .template("{spinner:.cyan} {msg}")?,
        );
        pb.set_message(format!("Seedr cloud caching: {name}"));

        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let list = self.list_root().await?;

            if let Some(folder) = list.folders.iter().find(|f| f.name == name) {
                pb.finish_with_message(format!(
                    "{} Seedr cloud caching complete!",
                    "✔".green().bold()
                ));
                return Ok(Some(folder.clone()));
            }

            if let Some(torrent) = list.torrents.iter().find(|t| t.id == torrent_id) {
                let pct = torrent.progress.unwrap_or(0.0);
                pb.set_message(format!("Seedr cloud caching: {name} ({pct:.1}%)"));
            } else {
                // Torrent disappeared from downloading list, check folders again
                if let Some(folder) = list
                    .folders
                    .iter()
                    .find(|f| f.name.contains(name) || name.contains(&f.name))
                {
                    pb.finish_with_message(format!(
                        "{} Seedr cloud caching complete!",
                        "✔".green().bold()
                    ));
                    return Ok(Some(folder.clone()));
                }
                pb.finish_with_message(format!("{} Ready in Seedr cloud.", "✔".green().bold()));
                return Ok(None);
            }
        }
    }

    pub async fn list_root(&self) -> Result<ListContentsResponse> {
        let url = format!(
            "{SEEDR_RESOURCE_URL}?func=list_contents&access_token={}",
            self.token
        );
        let resp = self.client.get(&url).send().await?;
        let data: ListContentsResponse = resp.json().await?;
        Ok(data)
    }

    pub async fn list_folder(&self, folder_id: u64) -> Result<ListContentsResponse> {
        let url = format!(
            "{SEEDR_RESOURCE_URL}?func=list_contents&folder_id={folder_id}&access_token={}",
            self.token
        );
        let resp = self.client.get(&url).send().await?;
        let data: ListContentsResponse = resp.json().await?;
        Ok(data)
    }

    pub async fn get_download_url(&self, file_id: u64) -> Result<String> {
        let url = format!(
            "{SEEDR_RESOURCE_URL}?func=fetch_file&folder_file_id={file_id}&access_token={}",
            self.token
        );
        let resp = self.client.get(&url).send().await?;
        let res: GenericResponse = resp.json().await?;
        res.url.context("Download URL not found in Seedr response")
    }

    pub async fn delete_folder(&self, folder_id: u64) -> Result<()> {
        let delete_arr = format!("[{{\"type\":\"folder\",\"id\":{folder_id}}}]");
        let params = [
            ("func", "delete"),
            ("delete_arr", &delete_arr),
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
