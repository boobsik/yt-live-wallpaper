mod autostart;
mod commands;
mod library;
mod model;
mod runtime;
mod storage;
mod tray;
mod win32;
mod ytdlp;

use std::time::Duration;

use tauri::{Manager, RunEvent, WindowEvent};

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            runtime::show_main(app);
        }))
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::get_library,
            commands::get_playback,
            commands::add_wallpaper,
            commands::apply_wallpaper,
            commands::remove_wallpaper,
            commands::toggle_pause,
            commands::stop_wallpaper,
            commands::set_fit_mode,
            commands::set_quality,
            commands::get_runtime_stats,
            commands::update_settings,
            commands::check_ytdlp,
            commands::report_stream,
            commands::wallpaper_playing,
            commands::stream_failed,
            commands::minimize_main,
            commands::hide_main,
            commands::show_main_window,
            commands::emit_note,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let state = runtime::new_state(&handle);
            let settings = state.lock().settings.clone();
            app.manage(state);

            // The wallpaper window is created once, hidden, on the UI thread.
            runtime::ensure_window(&handle)?;
            tray::setup(&handle)?;
            runtime::start_power_worker(&handle);
            runtime::start_stream_refresh(&handle);
            runtime::refresh_ytdlp(&handle);

            let minimized = settings.start_minimized
                || std::env::args().any(|argument| argument == "--minimized");
            if minimized {
                runtime::hide_main(&handle);
                // Windows from the config are attached slightly after setup on
                // some machines; make the tray-first start deterministic.
                let retry_handle = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(350));
                    runtime::hide_main(&retry_handle);
                });
            }

            // WebView2 needs a moment to finish its first navigation; applying
            // before that is harmless because the page reads `get_playback`
            // when it becomes ready.
            let auto_apply_handle = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(900));
                runtime::auto_apply_last(&auto_apply_handle);
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != runtime::MAIN_LABEL {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Closing the manager keeps the wallpaper alive in the tray.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build yt-live-wallpaper");

    app.run(|app_handle, event| {
        if matches!(event, RunEvent::Exit) {
            runtime::shutdown(app_handle);
        }
    });
}
