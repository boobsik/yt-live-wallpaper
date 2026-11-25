use chrono::Utc;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::library;
use crate::model::{
    normalize_fit, normalize_quality, AppView, Playback, Settings, Telemetry, WallpaperEntry,
    YtdlpInfo,
};
use crate::runtime::{self, AppState};
use crate::storage;
use crate::win32::stats;
use crate::ytdlp;

#[tauri::command]
pub fn get_app_state(app: AppHandle) -> AppView {
    let state = app.state::<AppState>();
    let inner = state.lock();
    AppView {
        playback: inner.playback.clone(),
        settings: inner.settings.clone(),
        ytdlp: inner.ytdlp.clone(),
        library: inner.library.clone(),
    }
}

#[tauri::command]
pub fn get_library(app: AppHandle) -> Vec<WallpaperEntry> {
    app.state::<AppState>().lock().library.clone()
}

#[tauri::command]
pub fn get_playback(app: AppHandle) -> Playback {
    app.state::<AppState>().lock().playback.clone()
}

#[tauri::command]
pub async fn add_wallpaper(
    app: AppHandle,
    url: String,
    quality: String,
    fit: String,
) -> Result<WallpaperEntry, String> {
    let video_id = library::parse_video_id(&url)
        .ok_or_else(|| "Не удалось найти VIDEO_ID в ссылке YouTube".to_string())?;
    let canonical = library::canonical_url(&video_id);
    let quality = normalize_quality(&quality).to_string();
    let fit_mode = normalize_fit(&fit).to_string();

    let meta_video_id = video_id.clone();
    let meta = tauri::async_runtime::spawn_blocking(move || library::fetch_meta(&meta_video_id))
        .await
        .map_err(|error| error.to_string())?;

    let id = {
        let state = app.state::<AppState>();
        let mut inner = state.lock();
        let now = Utc::now().to_rfc3339();
        let id = if let Some(existing) = inner
            .library
            .iter_mut()
            .find(|entry| entry.video_id == video_id)
        {
            existing.title = meta.title;
            existing.thumbnail_url = meta.thumbnail_url;
            existing.quality = quality;
            existing.fit_mode = fit_mode;
            existing.last_used_at = Some(now);
            if existing.channel.is_none() {
                existing.channel = meta.channel;
            }
            existing.id.clone()
        } else {
            let entry = library::new_entry(video_id, canonical, &quality, &fit_mode, meta);
            let id = entry.id.clone();
            inner.library.insert(0, entry);
            id
        };
        let snapshot = inner.library.clone();
        drop(inner);
        let _ = storage::save_json(&storage::library_path(&app), &snapshot);
        id
    };

    // Applying emits library:changed, so the grid refreshes in one shot.
    runtime::apply_entry(&app, &id, None, false).await?;

    let state = app.state::<AppState>();
    let entry = state
        .lock()
        .library
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
        .ok_or_else(|| "Запись пропала из библиотеки".to_string())?;
    Ok(entry)
}

#[tauri::command]
pub async fn apply_wallpaper(app: AppHandle, id: String) -> Result<Playback, String> {
    runtime::apply_entry(&app, &id, None, false).await
}

#[tauri::command]
pub fn remove_wallpaper(app: AppHandle, id: String) -> Result<Vec<WallpaperEntry>, String> {
    let active = {
        let state = app.state::<AppState>();
        let is_active = state.lock().playback.id.as_deref() == Some(id.as_str());
        is_active
    };
    if active {
        runtime::stop(&app);
    }
    let snapshot = {
        let state = app.state::<AppState>();
        let mut inner = state.lock();
        inner.library.retain(|entry| entry.id != id);
        inner.library.clone()
    };
    storage::save_json(&storage::library_path(&app), &snapshot)?;
    let _ = app.emit("library:changed", snapshot.clone());
    Ok(snapshot)
}

#[tauri::command]
pub fn toggle_pause(app: AppHandle) -> bool {
    runtime::toggle_pause(&app)
}

#[tauri::command]
pub fn stop_wallpaper(app: AppHandle) {
    runtime::stop(&app);
}

#[tauri::command]
pub async fn set_fit_mode(app: AppHandle, mode: String) -> Result<Playback, String> {
    runtime::set_fit(&app, &mode)
}

#[tauri::command]
pub async fn set_quality(app: AppHandle, quality: String) -> Result<Playback, String> {
    let normalized = normalize_quality(&quality).to_string();
    let active = {
        let state = app.state::<AppState>();
        let mut inner = state.lock();
        inner.settings.quality = normalized.clone();
        let settings = inner.settings.clone();
        drop(inner);
        runtime::persist_settings(&app, &settings);
        app.state::<AppState>().lock().playback.id.clone()
    };
    if let Some(id) = active {
        runtime::apply_entry(&app, &id, Some(normalized), false).await
    } else {
        let playback = {
            let state = app.state::<AppState>();
            let mut inner = state.lock();
            inner.playback.quality = normalized;
            inner.playback.clone()
        };
        let _ = app.emit("state:changed", playback.clone());
        Ok(playback)
    }
}

#[tauri::command]
pub async fn get_runtime_stats(app: AppHandle) -> Telemetry {
    let mut telemetry = tauri::async_runtime::spawn_blocking(stats::sample)
        .await
        .unwrap_or_default();
    let playback = app.state::<AppState>().lock().playback.clone();
    telemetry.engine = playback.engine;
    telemetry.engine_reason = playback.engine_reason;
    telemetry
}

#[tauri::command]
pub fn wallpaper_playing(app: AppHandle, id: Option<String>) {
    runtime::reveal_window(&app, id);
}

#[tauri::command]
pub fn update_settings(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    runtime::apply_settings(&app, settings)
}

#[tauri::command]
pub async fn check_ytdlp(app: AppHandle, path: String) -> YtdlpInfo {
    let info = tauri::async_runtime::spawn_blocking(move || ytdlp::info(&path))
        .await
        .unwrap_or_default();
    runtime::set_ytdlp(&app, info.clone());
    info
}

#[tauri::command]
pub fn report_stream(app: AppHandle, width: u32, height: u32, fps: f64) {
    let playback = {
        let state = app.state::<AppState>();
        let mut inner = state.lock();
        if inner.playback.is_active() {
            inner.playback.stream_width = width.min(16_384);
            inner.playback.stream_height = height.min(16_384);
            if fps.is_finite() && fps > 0.0 && fps <= 240.0 {
                inner.playback.fps = (fps * 10.0).round() / 10.0;
            }
        }
        inner.playback.clone()
    };
    let _ = app.emit("state:changed", playback);
}

#[tauri::command]
pub async fn stream_failed(app: AppHandle) -> Result<Playback, String> {
    runtime::stream_failed(&app).await
}

#[tauri::command]
pub fn minimize_main(app: AppHandle) {
    runtime::minimize_main(&app);
}

#[tauri::command]
pub fn hide_main(app: AppHandle) {
    runtime::hide_main(&app);
}

#[tauri::command]
pub fn show_main_window(app: AppHandle) {
    runtime::show_main(&app);
}

#[tauri::command]
pub fn emit_note(app: AppHandle, message: String) {
    #[derive(Clone, Serialize)]
    struct Note {
        message: String,
    }
    let _ = app.emit("note", Note { message });
}
