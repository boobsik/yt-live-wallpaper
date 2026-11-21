use serde::{Deserialize, Serialize};

/// Persisted user preferences (`settings.json` in app data dir).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Pause wallpaper automatically when a fullscreen app/game is in foreground.
    pub smart_pause: bool,
    /// Launch with Windows (HKCU Run key).
    pub autostart: bool,
    /// Start hidden in the tray.
    pub start_minimized: bool,
    /// Optional explicit path to yt-dlp.exe (empty = auto-detect).
    pub ytdlp_path: String,
    /// Default quality for new wallpapers: auto | 4k | 1440p | 1080p.
    pub quality: String,
    /// Default fit mode: cover | contain | fill.
    pub fit: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            smart_pause: true,
            autostart: false,
            start_minimized: false,
            ytdlp_path: String::new(),
            quality: "1080p".to_string(),
            fit: "cover".to_string(),
        }
    }
}

impl Settings {
    /// Keeps string enums inside the supported set, so a hand-edited settings
    /// file can never put the player into an unknown state.
    pub fn sanitize(&mut self) {
        self.quality = normalize_quality(&self.quality).to_string();
        self.fit = normalize_fit(&self.fit).to_string();
        self.ytdlp_path = self.ytdlp_path.trim().to_string();
    }
}

/// One wallpaper in the local library.
///
/// Serialized with snake_case keys on purpose: `library.json` is a documented,
/// hand-editable file (`video_id`, `thumbnail_url`, `fit_mode`, ...).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WallpaperEntry {
    pub id: String,
    pub video_id: String,
    pub url: String,
    pub title: String,
    pub thumbnail_url: String,
    #[serde(default = "default_quality")]
    pub quality: String,
    #[serde(default = "default_fit")]
    pub fit_mode: String,
    pub added_at: String,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
}

fn default_quality() -> String {
    "1080p".to_string()
}

fn default_fit() -> String {
    "cover".to_string()
}

/// Everything the player windows need to render the current wallpaper.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playback {
    pub id: Option<String>,
    pub video_id: Option<String>,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
    pub quality: String,
    pub fit: String,
    /// "idle" | "direct" (yt-dlp stream) | "embed" (YouTube IFrame API)
    pub engine: String,
    pub stream_url: Option<String>,
    pub paused: bool,
    /// "user" | "fullscreen" | null
    pub pause_reason: Option<String>,
    pub stream_width: u32,
    pub stream_height: u32,
    pub fps: f64,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            id: None,
            video_id: None,
            title: None,
            thumbnail_url: None,
            quality: "1080p".to_string(),
            fit: "cover".to_string(),
            engine: "idle".to_string(),
            stream_url: None,
            paused: false,
            pause_reason: None,
            stream_width: 0,
            stream_height: 0,
            fps: 0.0,
        }
    }
}

impl Playback {
    pub fn is_active(&self) -> bool {
        self.engine != "idle" && self.id.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtdlpInfo {
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

/// Win32 telemetry sample (own process + WebView2 process tree).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetry {
    pub node_ram_mb: f64,
    pub webview_ram_mb: f64,
    pub cpu: f64,
    pub processes: u32,
    pub sampled_at: u64,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            node_ram_mb: 0.0,
            webview_ram_mb: 0.0,
            cpu: 0.0,
            processes: 0,
            sampled_at: 0,
        }
    }
}

/// Snapshot returned to the library UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppView {
    pub playback: Playback,
    pub settings: Settings,
    pub ytdlp: YtdlpInfo,
    pub library: Vec<WallpaperEntry>,
}

pub fn normalize_quality(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "auto" | "best" => "auto",
        "4k" | "2160" | "2160p" => "4k",
        "1440" | "1440p" | "2k" => "1440p",
        "720" | "720p" => "720p",
        _ => "1080p",
    }
}

pub fn normalize_fit(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "contain" => "contain",
        "fill" | "stretch" => "fill",
        _ => "cover",
    }
}

pub fn quality_max_height(raw: &str) -> u32 {
    match normalize_quality(raw) {
        "auto" => 4320,
        "4k" => 2160,
        "1440p" => 1440,
        "720p" => 720,
        _ => 1080,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_normalization_handles_aliases() {
        assert_eq!(normalize_quality("Auto"), "auto");
        assert_eq!(normalize_quality("2160p"), "4k");
        assert_eq!(normalize_quality("1440"), "1440p");
        assert_eq!(normalize_quality("720p"), "720p");
        assert_eq!(normalize_quality("worst"), "1080p");
    }

    #[test]
    fn quality_heights_follow_the_labels() {
        assert_eq!(quality_max_height("auto"), 4320);
        assert_eq!(quality_max_height("4k"), 2160);
        assert_eq!(quality_max_height("1440p"), 1440);
        assert_eq!(quality_max_height("1080p"), 1080);
        assert_eq!(quality_max_height("unknown"), 1080);
    }

    #[test]
    fn fit_normalization_is_bounded() {
        assert_eq!(normalize_fit("cover"), "cover");
        assert_eq!(normalize_fit("Contain"), "contain");
        assert_eq!(normalize_fit("stretch"), "fill");
        assert_eq!(normalize_fit("nonsense"), "cover");
    }
}
