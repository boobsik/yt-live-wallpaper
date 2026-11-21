//! Smart power saving: detects a fullscreen foreground application (game,
//! video player, presentation) so the wallpaper can be paused instantly.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_STYLE, WS_CHILD,
};

use super::desktop::desktop_class;

fn window_class(hwnd: HWND) -> String {
    let mut buffer = [0u16; 256];
    let copied = unsafe { GetClassNameW(hwnd, &mut buffer) };
    if copied <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..copied as usize])
}

/// True when the foreground window is a borderless/exclusive fullscreen window
/// belonging to another process and covering its whole monitor.
///
/// `own_hwnds` contains the app's own windows (main + wallpaper) so they are
/// never misdetected as fullscreen — that caused the "pause after 1 second" bug.
pub fn fullscreen_foreground(own_hwnds: &[isize]) -> bool {
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() {
            return false;
        }
        if own_hwnds.contains(&(foreground.0 as isize)) {
            return false;
        }
        if !IsWindowVisible(foreground).as_bool() || IsIconic(foreground).as_bool() {
            return false;
        }

        let style = GetWindowLongPtrW(foreground, GWL_STYLE) as u32;
        if style & WS_CHILD.0 != 0 {
            return false;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(foreground, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return false;
        }

        if desktop_class(&window_class(foreground)) {
            return false;
        }

        // UWP windows that are minimised live on as cloaked windows.
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            foreground,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        );
        if cloaked != 0 {
            return false;
        }

        let mut rect = RECT::default();
        if GetWindowRect(foreground, &mut rect).is_err() {
            return false;
        }

        let monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return false;
        }

        // A 2 px tolerance keeps 1 px border quirks from defeating the check.
        rect.left <= info.rcMonitor.left + 2
            && rect.top <= info.rcMonitor.top + 2
            && rect.right >= info.rcMonitor.right - 2
            && rect.bottom >= info.rcMonitor.bottom - 2
    }
}
