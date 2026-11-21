use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "YtLiveWallpaper";

/// Registers/unregisters the app in the current user's Run key.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(RUN_KEY)
        .map_err(|e| format!("registry: {e}"))?;
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let value = format!("\"{}\" --minimized", exe.display());
        key.set_value(VALUE_NAME, &value)
            .map_err(|e| format!("registry: {e}"))?;
    } else {
        let _ = key.delete_value(VALUE_NAME);
    }
    Ok(())
}

pub fn is_enabled() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey(RUN_KEY)
        .ok()
        .and_then(|key| key.get_value::<String, _>(VALUE_NAME).ok())
        .is_some()
}
