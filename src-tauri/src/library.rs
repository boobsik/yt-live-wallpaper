use std::time::Duration;

use chrono::Utc;
use serde_json::Value;
use url::Url;

use crate::model::{normalize_fit, normalize_quality, WallpaperEntry};

/// Extracts an 11-character YouTube video id from any common URL shape:
/// `watch?v=`, `youtu.be/`, `shorts/`, `embed/`, `live/`, with or without scheme.
pub fn parse_video_id(input: &str) -> Option<String> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }
    let lower = raw.to_ascii_lowercase();
    let candidate = if lower.starts_with("http://") || lower.starts_with("https://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let parsed = Url::parse(&candidate).ok()?;
    let host = parsed
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let id = match host.as_str() {
        "youtu.be" => parsed.path_segments()?.next()?.to_string(),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtube-nocookie.com" => {
            let segments: Vec<&str> = parsed
                .path_segments()?
                .filter(|segment| !segment.is_empty())
                .collect();
            if parsed.path().trim_end_matches('/') == "/watch" {
                parsed
                    .query_pairs()
                    .find(|(key, _)| key == "v")
                    .map(|(_, value)| value.to_string())?
            } else if segments.len() >= 2
                && matches!(segments[0], "shorts" | "embed" | "live" | "v")
            {
                segments[1].to_string()
            } else {
                return None;
            }
        }
        _ => return None,
    };
    if is_video_id(&id) {
        Some(id)
    } else {
        None
    }
}

pub fn is_video_id(id: &str) -> bool {
    id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn canonical_url(video_id: &str) -> String {
    format!("https://www.youtube.com/watch?v={video_id}")
}

pub fn thumbnail_url(video_id: &str) -> String {
    format!("https://i.ytimg.com/vi/{video_id}/hqdefault.jpg")
}

#[derive(Debug, Clone)]
pub struct Meta {
    pub title: String,
    pub thumbnail_url: String,
    pub channel: Option<String>,
}

/// Pulls the title/thumbnail from YouTube oEmbed. Network failures are
/// non-fatal: the wallpaper is still added with a readable fallback title.
pub fn fetch_meta(video_id: &str) -> Meta {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(7))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) yt-live-wallpaper/0.1")
        .build();

    if let Ok(client) = client {
        let response = client
            .get("https://www.youtube.com/oembed")
            .query(&[
                ("url", canonical_url(video_id)),
                ("format", "json".to_string()),
            ])
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json::<Value>());

        if let Ok(json) = response {
            let title = json
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .unwrap_or("YouTube video")
                .to_string();
            let thumb = json
                .get("thumbnail_url")
                .and_then(Value::as_str)
                .filter(|thumb| !thumb.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| thumbnail_url(video_id));
            let channel = json
                .get("author_name")
                .and_then(Value::as_str)
                .map(str::to_string);
            return Meta {
                title,
                thumbnail_url: thumb,
                channel,
            };
        }
    }

    Meta {
        title: format!("YouTube · {video_id}"),
        thumbnail_url: thumbnail_url(video_id),
        channel: None,
    }
}

pub fn new_entry(video_id: String, url: String, quality: &str, fit: &str, meta: Meta) -> WallpaperEntry {
    let now = Utc::now().to_rfc3339();
    let stamp = Utc::now().timestamp_millis();
    WallpaperEntry {
        id: format!("{stamp:x}-{video_id}"),
        video_id,
        url,
        title: meta.title,
        thumbnail_url: meta.thumbnail_url,
        quality: normalize_quality(quality).to_string(),
        fit_mode: normalize_fit(fit).to_string(),
        added_at: now,
        last_used_at: None,
        channel: meta.channel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_every_supported_url_shape() {
        let id = "dQw4w9WgXcQ";
        for input in [
            format!("https://www.youtube.com/watch?v={id}"),
            format!("https://www.youtube.com/watch?v={id}&list=PL123"),
            format!("https://youtu.be/{id}?t=42"),
            format!("https://www.youtube.com/shorts/{id}"),
            format!("https://www.youtube.com/embed/{id}"),
            format!("https://www.youtube.com/live/{id}"),
            format!("https://m.youtube.com/watch?v={id}"),
            format!("https://music.youtube.com/watch?v={id}"),
            format!("https://www.youtube-nocookie.com/embed/{id}"),
            format!("www.youtube.com/watch?v={id}"),
            format!("youtu.be/{id}"),
            format!("HTTPS://YOUTUBE.COM/watch?v={id}"),
        ] {
            assert_eq!(parse_video_id(&input).as_deref(), Some(id), "{input}");
        }
    }

    #[test]
    fn rejects_other_hosts_and_bad_ids() {
        for input in [
            "https://example.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=too-short",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ!",
            "https://www.youtube.com/channel/UC123",
            "not a url",
            "",
        ] {
            assert_eq!(parse_video_id(input), None, "{input}");
        }
    }

    #[test]
    fn canonical_and_thumbnail_urls_are_stable() {
        let id = "dQw4w9WgXcQ";
        assert_eq!(
            canonical_url(id),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        assert_eq!(
            thumbnail_url(id),
            "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"
        );
    }
}
