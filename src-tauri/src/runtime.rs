//! Orchestration: library -> playback resolution -> WorkerW window, plus the
//! smart-pause / stream-refresh background workers.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::Utc;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::HWND;

use crate::model::{
    normalize_fit, normalize_quality, Playback, Settings, WallpaperEntry, YtdlpInfo,
};
use crate::storage;
use crate::win32::{desktop, power};
use crate::ytdlp;

pub const MAIN_LABEL: &str = "main";
pub const WALLPAPER_LABEL: &str = "wallpaper";

/// WebView2 switches that keep a background window rendering (Chromium would
/// otherwise throttle/occlude a reparented window) and allow muted autoplay.
///
/// WebView2 permits only one environment per user-data folder, and every window
/// must be created with identical options. This string must stay in sync with
/// `tauri.conf.json > app.windows[0].additionalBrowserArgs`.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion,IntensiveWakeUpThrottling --autoplay-policy=no-user-gesture-required --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";

pub static RUNNING: AtomicBool = AtomicBool::new(true);

/// A detector value only changes after the same reading appears twice in a row.
#[derive(Default)]
struct FlipFlop {
    stable: bool,
    pending: Option<bool>,
}

impl FlipFlop {
    fn observe(&mut self, value: bool) -> bool {
        if self.stable == value {
            self.pending = None;
        } else if self.pending == Some(value) {
            self.stable = value;
            self.pending = None;
        } else {
            self.pending = Some(value);
        }
        self.stable
    }
}

#[derive(Default)]
struct PauseDetector {
    fullscreen: FlipFlop,
    maximized: FlipFlop,
}

pub struct Inner {
    pub settings: Settings,
    pub library: Vec<WallpaperEntry>,
    pub playback: Playback,
    pub ytdlp: YtdlpInfo,
    pub user_paused: bool,
    pub fullscreen: bool,
    pub maximized: bool,
    pub original_wallpaper: String,
    pub stream_since: Option<Instant>,
    pub refreshing: bool,
    pub shutting_down: bool,
}

impl Inner {
    pub fn effective_paused(&self) -> bool {
        self.user_paused
            || (self.settings.pause_fullscreen && self.fullscreen)
            || (self.settings.pause_maximized && self.maximized)
    }

    pub fn pause_reason(&self) -> Option<String> {
        if !self.effective_paused() {
            None
        } else if self.settings.pause_fullscreen && self.fullscreen {
            Some("fullscreen".to_string())
        } else if self.settings.pause_maximized && self.maximized {
            Some("maximized".to_string())
        } else {
            Some("user".to_string())
        }
    }

    pub fn sync_pause(&mut self) -> bool {
        let paused = self.effective_paused();
        let reason = self.pause_reason();
        let changed = self.playback.paused != paused || self.playback.pause_reason != reason;
        self.playback.paused = paused;
        self.playback.pause_reason = reason;
        changed
    }
}

pub struct AppState {
    pub inner: Mutex<Inner>,
    detector: Mutex<PauseDetector>,
}

impl AppState {
    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }
}

pub fn new_state(app: &AppHandle) -> AppState {
    let mut settings: Settings =
        storage::load_json(&storage::settings_path(app), Settings::default);
    settings.sanitize();
    let mut library: Vec<WallpaperEntry> =
        storage::load_json(&storage::library_path(app), Vec::new);
    for entry in &mut library {
        entry.quality = normalize_quality(&entry.quality).to_string();
        entry.fit_mode = normalize_fit(&entry.fit_mode).to_string();
    }
    let original_wallpaper = desktop::capture_wallpaper();
    AppState {
        detector: Mutex::new(PauseDetector::default()),
        inner: Mutex::new(Inner {
            settings,
            library,
            playback: Playback::default(),
            ytdlp: YtdlpInfo::default(),
            user_paused: false,
            fullscreen: false,
            maximized: false,
            original_wallpaper,
            stream_since: None,
            refreshing: false,
            shutting_down: false,
        }),
    }
}

pub fn persist_settings(app: &AppHandle, settings: &Settings) {
    let _ = storage::save_json(&storage::settings_path(app), settings);
}

pub fn persist_library(app: &AppHandle, library: &[WallpaperEntry]) {
    let _ = storage::save_json(&storage::library_path(app), &library.to_vec());
}

// ---------------------------------------------------------------------------
// Wallpaper window
// ---------------------------------------------------------------------------

/// Creates the borderless WebView2 window used as the live wallpaper. It stays
/// hidden until a wallpaper is applied; creation must happen on the UI thread
/// (called from `setup`).
pub fn ensure_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(window) = app.get_webview_window(WALLPAPER_LABEL) {
        return Ok(window);
    }
    let (width, height) = desktop::screen_size();
    WebviewWindowBuilder::new(
        app,
        WALLPAPER_LABEL,
        WebviewUrl::App("wallpaper.html".into()),
    )
    .title("yt-live-wallpaper")
    .inner_size(width as f64, height as f64)
    .position(0.0, 0.0)
    .decorations(false)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .skip_taskbar(true)
    .shadow(false)
    .focused(false)
    .visible(false)
    .additional_browser_args(BROWSER_ARGS)
    .build()
    .map_err(|error| format!("не удалось создать окно обоев: {error}"))
}

fn raw_hwnd(window: &WebviewWindow) -> Option<HWND> {
    let hwnd = window.hwnd().ok()?;
    let raw = hwnd.0 as isize;
    if raw == 0 {
        return None;
    }
    Some(HWND(raw as *mut c_void))
}

fn attach_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(WALLPAPER_LABEL) else {
        return;
    };
    let Some(hwnd) = raw_hwnd(&window) else {
        return;
    };
    match desktop::find_workerw() {
        Some(workerw) => desktop::attach(hwnd, workerw),
        None => desktop::show_raw(hwnd),
    }
    // Let tao/wry update the WebView2 controller visibility. Reparenting with
    // raw SetParent alone can leave the controller hidden.
    let _ = window.show();
}

fn hide_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WALLPAPER_LABEL) {
        if let Some(hwnd) = raw_hwnd(&window) {
            // Raw hide is immediate (the tray exit path may not pump the loop).
            desktop::hide_raw(hwnd);
        }
        let _ = window.hide();
    }
}

/// Shows the wallpaper only after the renderer confirms playback. Stale events
/// (from an old stream or a pause that arrived during the grace period) are
/// ignored.
pub fn reveal_window(app: &AppHandle, id: Option<String>) {
    let active = {
        let state = app.state::<AppState>();
        let inner = state.lock();
        inner.playback.is_active()
            && !inner.playback.paused
            && id.as_deref() == inner.playback.id.as_deref()
    };
    if !active {
        return;
    }
    if let Some(window) = app.get_webview_window(WALLPAPER_LABEL) {
        if let Some(hwnd) = raw_hwnd(&window) {
            desktop::show_raw(hwnd);
        }
        let _ = window.show();
    }
}

fn emit_play(app: &AppHandle, playback: &Playback) {
    if let Some(window) = app.get_webview_window(WALLPAPER_LABEL) {
        let _ = window.emit("wallpaper:play", playback.clone());
    }
    let _ = app.emit("state:changed", playback.clone());
}

fn emit_pause(app: &AppHandle, playback: &Playback) {
    #[derive(Clone, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct PausePayload {
        paused: bool,
        reason: Option<String>,
    }
    let payload = PausePayload {
        paused: playback.paused,
        reason: playback.pause_reason.clone(),
    };
    storage::append_log(
        app,
        &format!(
            "diag: pause paused={} reason={}",
            playback.paused,
            playback.pause_reason.as_deref().unwrap_or("none")
        ),
    );
    // The iframe can briefly show its own controls while pausing. Hide the
    // wallpaper first, then let the renderer call pauseVideo().
    if playback.paused && playback.engine == "embed" {
        hide_window(app);
    }
    if let Some(window) = app.get_webview_window(WALLPAPER_LABEL) {
        let _ = window.emit("wallpaper:pause", payload);
    }
    let _ = app.emit("state:changed", playback.clone());
}

fn emit_library(app: &AppHandle) {
    let library = { app.state::<AppState>().lock().library.clone() };
    let _ = app.emit("library:changed", library);
}

// ---------------------------------------------------------------------------
// Apply / stop
// ---------------------------------------------------------------------------

pub async fn apply_entry(
    app: &AppHandle,
    id: &str,
    quality_override: Option<String>,
    force_embed: bool,
) -> Result<Playback, String> {
    let state = app.state::<AppState>();
    let (entry, quality, settings) = {
        let inner = state.lock();
        let entry = inner
            .library
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| "Обои не найдены в библиотеке".to_string())?;
        let quality = quality_override
            .as_deref()
            .map(normalize_quality)
            .unwrap_or(&entry.quality)
            .to_string();
        (entry, quality, inner.settings.clone())
    };

    storage::append_log(
        app,
        &format!(
            "diag: apply video={} quality={} force_embed={force_embed}",
            entry.video_id, quality
        ),
    );

    // Direct, ad-free stream through yt-dlp (video-only: wallpapers are muted).
    let mut resolved: Result<String, String> = Err("yt-dlp недоступен".to_string());
    let mut fallback_reason: Option<String> = None;
    if force_embed {
        fallback_reason =
            Some("поток yt-dlp не играет: принудительный переход на embed".to_string());
    } else if let Some(exe) = ytdlp::locate(&settings.ytdlp_path) {
        let name = exe
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "yt-dlp".to_string());
        storage::append_log(app, &format!("diag: найден {name}"));
        let url = entry.url.clone();
        let quality_job = quality.clone();
        resolved = match tauri::async_runtime::spawn_blocking(move || {
            ytdlp::resolve(&exe, &url, &quality_job)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => Err(error.to_string()),
        };
    } else {
        fallback_reason = Some(if settings.ytdlp_path.trim().is_empty() {
            "yt-dlp не найден (папка приложения и PATH)".to_string()
        } else {
            "yt-dlp не найден по указанному пути".to_string()
        });
    }

    let (engine, reason, stream_url) = match resolved {
        Ok(url) => (
            "direct",
            format!("прямой поток yt-dlp ({quality})"),
            Some(url),
        ),
        Err(error) => {
            let reason = fallback_reason.unwrap_or_else(|| format!("ошибка yt-dlp: {error}"));
            ("embed", reason, None)
        }
    };
    storage::append_log(app, &format!("diag: engine={engine} reason={reason}"));

    let mut playback = Playback {
        id: Some(entry.id.clone()),
        video_id: Some(entry.video_id.clone()),
        title: Some(entry.title.clone()),
        thumbnail_url: Some(entry.thumbnail_url.clone()),
        quality: quality.clone(),
        fit: entry.fit_mode.clone(),
        engine: engine.to_string(),
        engine_reason: Some(reason),
        stream_url,
        paused: false,
        pause_reason: None,
        stream_width: 0,
        stream_height: 0,
        fps: 0.0,
    };

    {
        let mut inner = state.lock();
        playback.paused = inner.effective_paused();
        playback.pause_reason = inner.pause_reason();
        inner.playback = playback.clone();
        inner.stream_since = Some(Instant::now());
        if let Some(stored) = inner.library.iter_mut().find(|stored| stored.id == id) {
            stored.quality = quality;
            stored.last_used_at = Some(Utc::now().to_rfc3339());
        }
        let snapshot = inner.library.clone();
        drop(inner);
        persist_library(app, &snapshot);
    }

    ensure_window(app)?;
    attach_window(app);
    // Both engines stay hidden until the renderer reports a real PLAYING event.
    // This also keeps YouTube's first-frame controls off the desktop.
    hide_window(app);
    emit_play(app, &playback);
    emit_library(app);
    Ok(playback)
}

pub fn toggle_pause(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    let (paused, playback) = {
        let mut inner = state.lock();
        if !inner.playback.is_active() {
            return false;
        }
        inner.user_paused = !inner.user_paused;
        inner.sync_pause();
        (inner.playback.paused, inner.playback.clone())
    };
    emit_pause(app, &playback);
    paused
}

pub fn stop(app: &AppHandle) {
    let (playback, was_active) = {
        let state = app.state::<AppState>();
        let mut inner = state.lock();
        let was_active = inner.playback.is_active();
        inner.playback = Playback {
            quality: inner.settings.quality.clone(),
            fit: inner.settings.fit.clone(),
            ..Playback::default()
        };
        inner.user_paused = false;
        inner.stream_since = None;
        (inner.playback.clone(), was_active)
    };

    if let Some(window) = app.get_webview_window(WALLPAPER_LABEL) {
        let _ = window.emit("wallpaper:stop", ());
    }
    hide_window(app);
    if was_active {
        restore_desktop(app);
    }
    let _ = app.emit("state:changed", playback);
}

/// Puts the user's original static wallpaper back. Falls back to the stock
/// Windows wallpaper so a missing file can never leave a black desktop.
pub fn restore_desktop(app: &AppHandle) {
    let original = { app.state::<AppState>().lock().original_wallpaper.clone() };
    let restored = desktop::set_wallpaper(&original);
    if !restored {
        if let Some(fallback) = desktop::fallback_wallpaper() {
            desktop::set_wallpaper(&fallback);
        }
    }
}

pub fn set_fit(app: &AppHandle, mode: &str) -> Result<Playback, String> {
    let fit = normalize_fit(mode).to_string();
    let state = app.state::<AppState>();
    let playback = {
        let mut inner = state.lock();
        inner.settings.fit = fit.clone();
        inner.playback.fit = fit.clone();
        if let Some(active_id) = inner.playback.id.clone() {
            if let Some(entry) = inner.library.iter_mut().find(|entry| entry.id == active_id) {
                entry.fit_mode = fit.clone();
            }
        }
        let settings = inner.settings.clone();
        let snapshot = inner.library.clone();
        drop(inner);
        persist_settings(app, &settings);
        persist_library(app, &snapshot);
        app.state::<AppState>().lock().playback.clone()
    };
    if let Some(window) = app.get_webview_window(WALLPAPER_LABEL) {
        let _ = window.emit("wallpaper:fit", fit);
    }
    let _ = app.emit("state:changed", playback.clone());
    Ok(playback)
}

pub fn apply_settings(app: &AppHandle, mut settings: Settings) -> Result<Settings, String> {
    settings.sanitize();
    let previously_enabled = {
        let state = app.state::<AppState>();
        let inner = state.lock();
        inner.settings.autostart
    };
    if settings.autostart != previously_enabled
        || (settings.autostart && !crate::autostart::is_enabled())
    {
        crate::autostart::set_enabled(settings.autostart)?;
    }

    let playback = {
        let state = app.state::<AppState>();
        let mut inner = state.lock();
        inner.settings = settings.clone();
        inner.sync_pause();
        inner.playback.clone()
    };
    persist_settings(app, &settings);
    emit_pause(app, &playback);
    Ok(settings)
}

// ---------------------------------------------------------------------------
// Smart pause / stream refresh workers
// ---------------------------------------------------------------------------

fn set_cover_state(app: &AppHandle, fullscreen: bool, maximized: bool) {
    let state = app.state::<AppState>();
    let (changed, playback) = {
        let mut inner = state.lock();
        if inner.fullscreen == fullscreen && inner.maximized == maximized {
            (false, inner.playback.clone())
        } else {
            inner.fullscreen = fullscreen;
            inner.maximized = maximized;
            inner.sync_pause();
            (true, inner.playback.clone())
        }
    };
    if changed {
        emit_pause(app, &playback);
    }
}

fn verify_attachment(app: &AppHandle) {
    let (active, shutting_down) = {
        let state = app.state::<AppState>();
        let inner = state.lock();
        (inner.playback.is_active(), inner.shutting_down)
    };
    if !active || shutting_down {
        return;
    }
    let Some(window) = app.get_webview_window(WALLPAPER_LABEL) else {
        return;
    };
    let Some(hwnd) = raw_hwnd(&window) else {
        return;
    };
    if !desktop::is_valid_window(hwnd.0 as isize) {
        return;
    }
    let parent = desktop::parent_of(hwnd);
    if parent.0.is_null() || !desktop::is_valid_window(parent.0 as isize) {
        // Explorer restarted and took WorkerW with it: re-attach.
        attach_window(app);
    }
}

fn own_hwnds(app: &AppHandle) -> Vec<isize> {
    let mut hwnds = Vec::with_capacity(2);
    for label in [MAIN_LABEL, WALLPAPER_LABEL] {
        if let Some(window) = app.get_webview_window(label) {
            if let Some(hwnd) = raw_hwnd(&window) {
                hwnds.push(hwnd.0 as isize);
            }
        }
    }
    hwnds
}

pub fn start_power_worker(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let mut tick: u32 = 0;
        while RUNNING.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(700));
            if !RUNNING.load(Ordering::Relaxed) {
                break;
            }
            tick = tick.wrapping_add(1);
            let (pause_fullscreen, pause_maximized, shutting_down) = {
                let state = app.state::<AppState>();
                let inner = state.lock();
                (
                    inner.settings.pause_fullscreen,
                    inner.settings.pause_maximized,
                    inner.shutting_down,
                )
            };
            if !shutting_down {
                // Never treat our own windows as a covering app: that used to
                // pause the wallpaper ~1s after launch. We only pause/resume
                // playback — the wallpaper window stays attached to WorkerW.
                let own = own_hwnds(&app);
                let wallpaper = app
                    .get_webview_window(WALLPAPER_LABEL)
                    .and_then(|window| raw_hwnd(&window))
                    .unwrap_or_default();
                let fullscreen = pause_fullscreen && power::fullscreen_foreground(&own);
                let maximized = pause_maximized && power::covering_window(&own, wallpaper);

                // A single stray reading (window being created/destroyed) must
                // not flicker the wallpaper; require two identical samples.
                let state = app.state::<AppState>();
                let mut detector = state
                    .detector
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let stable_fullscreen = detector.fullscreen.observe(fullscreen);
                let stable_maximized = detector.maximized.observe(maximized);
                drop(detector);
                set_cover_state(&app, stable_fullscreen, stable_maximized);
            }
            if tick % 8 == 0 {
                verify_attachment(&app);
            }
        }
    });
}

pub fn start_stream_refresh(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        // YouTube direct URLs expire after a few hours. Refresh well before
        // that while the wallpaper keeps running.
        while RUNNING.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(600));
            if !RUNNING.load(Ordering::Relaxed) {
                break;
            }
            let id = {
                let state = app.state::<AppState>();
                let mut inner = state.lock();
                let due = inner.playback.engine == "direct"
                    && inner
                        .stream_since
                        .map(|since| since.elapsed() > Duration::from_secs(9_900))
                        .unwrap_or(false)
                    && !inner.refreshing
                    && inner.playback.id.is_some();
                if due {
                    inner.refreshing = true;
                    inner.playback.id.clone()
                } else {
                    None
                }
            };
            if let Some(id) = id {
                let _ = tauri::async_runtime::block_on(apply_entry(&app, &id, None, false));
                let state = app.state::<AppState>();
                state.lock().refreshing = false;
            }
        }
    });
}

/// Falls back to the YouTube embed when a direct googlevideo URL dies.
pub async fn stream_failed(app: &AppHandle) -> Result<Playback, String> {
    let id = {
        let state = app.state::<AppState>();
        let inner = state.lock();
        if inner.playback.engine != "direct" {
            return Err("embed уже используется".to_string());
        }
        inner.playback.id.clone()
    };
    let Some(id) = id else {
        return Err("нет активных обоев".to_string());
    };
    storage::append_log(app, "diag: прямой поток не играет — fallback на embed");
    apply_entry(app, &id, None, true).await
}

// ---------------------------------------------------------------------------
// Main window helpers / shutdown
// ---------------------------------------------------------------------------

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn minimize_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_LABEL) {
        let _ = window.minimize();
    }
}

pub fn hide_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_LABEL) {
        let _ = window.hide();
    }
}

/// Restores the static wallpaper and stops the workers. Idempotent: the tray
/// exit path and `RunEvent::Exit` can both call it.
pub fn shutdown(app: &AppHandle) {
    let active = {
        let state = app.state::<AppState>();
        let mut inner = state.lock();
        if inner.shutting_down {
            return;
        }
        inner.shutting_down = true;
        inner.playback.is_active()
    };
    RUNNING.store(false, Ordering::Relaxed);
    hide_window(app);
    if active {
        restore_desktop(app);
    }
    if let Some(window) = app.get_webview_window(WALLPAPER_LABEL) {
        let _ = window.emit("wallpaper:stop", ());
    }
}

/// Applies the most recently used wallpaper on startup, without stealing focus.
pub fn auto_apply_last(app: &AppHandle) {
    let last = {
        let state = app.state::<AppState>();
        let inner = state.lock();
        inner
            .library
            .iter()
            .filter(|entry| entry.last_used_at.is_some())
            .max_by(|a, b| a.last_used_at.cmp(&b.last_used_at))
            .map(|entry| entry.id.clone())
    };
    if let Some(id) = last {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = apply_entry(&app, &id, None, false).await;
        });
    } else {
        storage::append_log(app, "diag: запуск без активных обоев — движок не выбран");
    }
}

/// Persists a freshly checked yt-dlp status.
pub fn set_ytdlp(app: &AppHandle, info: YtdlpInfo) {
    {
        let state = app.state::<AppState>();
        state.lock().ytdlp = info.clone();
    }
    let _ = app.emit("ytdlp:changed", info);
}

pub fn refresh_ytdlp(app: &AppHandle) {
    let custom = { app.state::<AppState>().lock().settings.ytdlp_path.clone() };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(info) = tauri::async_runtime::spawn_blocking(move || ytdlp::info(&custom)).await {
            set_ytdlp(&app, info);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::FlipFlop;

    #[test]
    fn debounce_needs_two_identical_readings() {
        let mut flip = FlipFlop::default();
        assert!(!flip.observe(true));
        assert!(flip.observe(true));
        assert!(flip.observe(false));
        assert!(flip.observe(true));
        assert!(flip.observe(false));
        assert!(!flip.observe(false));
    }
}
