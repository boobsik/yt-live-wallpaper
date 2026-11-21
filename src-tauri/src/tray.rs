use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

use crate::runtime;

/// System tray: left click opens the library, menu mirrors the desktop actions.
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Открыть библиотеку", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Пауза / Возобновить", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Остановить обои", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &pause, &stop, &separator, &quit])?;

    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .icon_as_template(false)
        .tooltip("yt-live-wallpaper")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => runtime::show_main(app),
            "pause" => {
                runtime::toggle_pause(app);
            }
            "stop" => runtime::stop(app),
            "quit" => {
                runtime::shutdown(app);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                runtime::show_main(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}
