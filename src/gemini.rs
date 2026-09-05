use anyhow::Result;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Movie,
    Show,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub media_type: MediaType,
    pub title: String,
    pub year: Option<u32>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub relative_folder: String,
    pub clean_filename: String,
}

#[derive(Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
struct GeminiConfig {
    response_mime_type: String,
}

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiConfig,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiCandidateContent>,
}

#[derive(Deserialize)]
struct GeminiCandidateContent {
    parts: Option<Vec<GeminiCandidatePart>>,
}

#[derive(Deserialize)]
struct GeminiCandidatePart {
    text: Option<String>,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
}

pub async fn parse_media(raw_name: &str, api_key: Option<&str>) -> MediaInfo {
    if let Some(key) = api_key {
        if let Ok(info) = query_gemini(raw_name, key).await {
            return info;
        }
    }
    parse_with_regex(raw_name)
}

async fn query_gemini(raw_name: &str, api_key: &str) -> Result<MediaInfo> {
    let prompt = format!(
        r#"Analyze this media release filename for a Jellyfin media server: "{raw_name}"
Return strictly valid JSON with this schema:
{{
  "media_type": "movie" | "show",
  "title": "Clean Canonical Title",
  "year": 2023 or null,
  "season": 1 or null,
  "episode": 5 or null,
  "relative_folder": "movies/Title (Year)" or "shows/Title (Year)/Season 01",
  "clean_filename": "Title (Year).ext" or "Title (Year) - S01E05.ext"
}}"#
    );

    let req_body = GeminiRequest {
        contents: vec![GeminiContent {
            parts: vec![GeminiPart { text: prompt }],
        }],
        generation_config: GeminiConfig {
            response_mime_type: "application/json".to_string(),
        },
    };

    let client = Client::new();
    for model in [
        "gemini-3.5-flash-lite",
        "gemini-2.5-flash",
        "gemini-1.5-flash",
    ] {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}"
        );
        let Ok(resp) = client.post(&url).json(&req_body).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(gemini_resp) = resp.json::<GeminiResponse>().await else {
            continue;
        };
        let text_opt = gemini_resp
            .candidates
            .and_then(|c| c.into_iter().next())
            .and_then(|c| c.content)
            .and_then(|c| c.parts)
            .and_then(|p| p.into_iter().next())
            .and_then(|p| p.text);

        if let Some(text) = text_opt {
            if let Ok(parsed) = serde_json::from_str::<MediaInfo>(&text) {
                return Ok(parsed);
            }
        }
    }

    anyhow::bail!("Gemini API models returned error or invalid JSON")
}

pub fn parse_with_regex(raw_name: &str) -> MediaInfo {
    let ext = Path::new(raw_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    let name_no_ext = Path::new(raw_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(raw_name);

    let show_re = Regex::new(r"(?i)[._ -]S(\d{1,2})E(\d{1,2})").ok();
    let year_re = Regex::new(r"(?:19|20)\d{2}").ok();

    if let Some(caps) = show_re.as_ref().and_then(|re| re.captures(name_no_ext)) {
        let season: u32 = caps
            .get(1)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(1);
        let episode: u32 = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(1);
        let match_start = caps.get(0).map_or(name_no_ext.len(), |m| m.start());
        let raw_title = &name_no_ext[..match_start];
        let title = clean_title(raw_title);

        let folder = format!("shows/{title}/Season {season:02}");
        let clean_filename = format!("{title} - S{season:02}E{episode:02}{ext}");

        MediaInfo {
            media_type: MediaType::Show,
            title,
            year: None,
            season: Some(season),
            episode: Some(episode),
            relative_folder: folder,
            clean_filename,
        }
    } else {
        let year: Option<u32> = year_re
            .as_ref()
            .and_then(|re| re.find(name_no_ext))
            .and_then(|m| m.as_str().parse().ok());
        let title_part = if let Some(y_match) = year_re.as_ref().and_then(|re| re.find(name_no_ext))
        {
            &name_no_ext[..y_match.start()]
        } else {
            name_no_ext
        };
        let title = clean_title(title_part);

        let (folder, clean_filename) = if let Some(y) = year {
            (
                format!("movies/{title} ({y})"),
                format!("{title} ({y}){ext}"),
            )
        } else {
            (format!("movies/{title}"), format!("{title}{ext}"))
        };

        MediaInfo {
            media_type: MediaType::Movie,
            title,
            year,
            season: None,
            episode: None,
            relative_folder: folder,
            clean_filename,
        }
    }
}

fn clean_title(s: &str) -> String {
    let replaced = s.replace(['.', '_', '-'], " ");
    let cleaned = replaced
        .split_whitespace()
        .filter(|w| {
            let lower = w.to_lowercase();
            !matches!(
                lower.as_str(),
                "1080p"
                    | "720p"
                    | "2160p"
                    | "4k"
                    | "uhd"
                    | "bluray"
                    | "webrip"
                    | "web-dl"
                    | "x264"
                    | "x265"
                    | "hevc"
                    | "dts"
                    | "aac"
                    | "rarbg"
                    | "yts"
                    | "www"
                    | "tamilmv"
                    | "1tamilmv"
                    | "ing"
            )
        })
        .collect::<Vec<&str>>()
        .join(" ");

    let trimmed = cleaned.trim_matches(|c: char| {
        c.is_whitespace() || c == '(' || c == ')' || c == '[' || c == ']' || c == '-' || c == '_'
    });

    if trimmed.is_empty() {
        s.trim().to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_movie() {
        let raw = "Oppenheimer.2023.1080p.BluRay.x264.mkv";
        let info = parse_with_regex(raw);
        assert_eq!(info.media_type, MediaType::Movie);
        assert_eq!(info.title, "Oppenheimer");
        assert_eq!(info.year, Some(2023));
        assert_eq!(info.relative_folder, "movies/Oppenheimer (2023)");
        assert_eq!(info.clean_filename, "Oppenheimer (2023).mkv");
    }

    #[test]
    fn test_parse_tv_show() {
        let raw = "Breaking.Bad.S01E05.720p.WEB-DL.mkv";
        let info = parse_with_regex(raw);
        assert_eq!(info.media_type, MediaType::Show);
        assert_eq!(info.title, "Breaking Bad");
        assert_eq!(info.season, Some(1));
        assert_eq!(info.episode, Some(5));
        assert_eq!(info.relative_folder, "shows/Breaking Bad/Season 01");
        assert_eq!(info.clean_filename, "Breaking Bad - S01E05.mkv");
    }

    #[test]
    fn test_parse_movie_with_site_prefix() {
        let raw = "www 1TamilMV ing Jana Nayagan ( (2026).mkv";
        let info = parse_with_regex(raw);
        assert_eq!(info.media_type, MediaType::Movie);
        assert_eq!(info.title, "Jana Nayagan");
        assert_eq!(info.year, Some(2026));
        assert_eq!(info.relative_folder, "movies/Jana Nayagan (2026)");
        assert_eq!(info.clean_filename, "Jana Nayagan (2026).mkv");
    }
}
