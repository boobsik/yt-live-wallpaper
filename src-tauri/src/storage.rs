use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tauri::{AppHandle, Manager};

/// `%APPDATA%\com.ytlivewallpaper.desktop` (or the platform equivalent).
pub fn data_dir(app: &AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn library_path(app: &AppHandle) -> PathBuf {
    data_dir(app).join("library.json")
}

pub fn settings_path(app: &AppHandle) -> PathBuf {
    data_dir(app).join("settings.json")
}

pub fn log_path(app: &AppHandle) -> PathBuf {
    data_dir(app).join("engine.log")
}

/// Appends one timestamped line to `engine.log`. Diagnostics are best-effort:
/// a full disk must never stop playback.
pub fn append_log(app: &AppHandle, message: &str) {
    let line = format!("[{}] {}\n", Utc::now().to_rfc3339(), message);
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(app))
    {
        let _ = file.write_all(line.as_bytes());
    }
}

pub fn load_json<T: DeserializeOwned, F: FnOnce() -> T>(path: &Path, fallback: F) -> T {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| fallback()),
        Err(_) => fallback(),
    }
}

/// Writes through a temporary file so a crash can never truncate the library.
pub fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path.parent().ok_or("invalid data path")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let raw = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}
