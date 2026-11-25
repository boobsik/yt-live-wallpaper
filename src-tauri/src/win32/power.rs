//! Smart power saving: detects a fullscreen foreground application (game,
//! video player, presentation) or any maximized window on the wallpaper's
//! monitor, so the wallpaper can be paused instantly.

use std::ffi::c_void;
use std::mem::size_of;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed, GWL_EXSTYLE, GWL_STYLE,
    WS_CHILD, WS_EX_TOOLWINDOW,
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

/// Common filters for both detectors. Own windows, shell windows, tool windows
/// and cloaked/minimised windows are never treated as a covering application.
fn eligible_window(hwnd: HWND, own_hwnds: &[isize]) -> bool {
    unsafe {
        if hwnd.0.is_null()
            || own_hwnds.contains(&(hwnd.0 as isize))
            || !IsWindowVisible(hwnd).as_bool()
            || IsIconic(hwnd).as_bool()
        {
            return false;
        }

        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if style & WS_CHILD.0 != 0 || ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return false;
        }

        if desktop_class(&window_class(hwnd)) {
            return false;
        }

        // UWP windows that are minimised live on as cloaked windows.
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        );
        cloaked == 0
    }
}

fn monitor_info(monitor: HMONITOR) -> Option<MONITORINFO> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(monitor, &mut info).as_bool().then_some(info) }
}

/// True when the foreground window is a borderless/exclusive fullscreen window
/// belonging to another process and covering its whole monitor.
///
/// `own_hwnds` contains the app's own windows (main + wallpaper) so they are
/// never misdetected as fullscreen — that caused the "pause after 1 second" bug.
pub fn fullscreen_foreground(own_hwnds: &[isize]) -> bool {
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() || !eligible_window(foreground, own_hwnds) {
            return false;
        }

        let mut rect = RECT::default();
        if GetWindowRect(foreground, &mut rect).is_err() {
            return false;
        }
        let Some(info) = monitor_info(MonitorFromWindow(
            foreground,
            MONITOR_DEFAULTTONEAREST,
        )) else {
            return false;
        };

        // A 2 px tolerance keeps 1 px border quirks from defeating the check.
        rect.left <= info.rcMonitor.left + 2
            && rect.top <= info.rcMonitor.top + 2
            && rect.right >= info.rcMonitor.right - 2
            && rect.bottom >= info.rcMonitor.bottom - 2
    }
}

/// Fraction of the monitor's work area covered by a window rectangle. The
/// intersection is clipped to `rcWork` so off-screen coordinates cannot inflate
/// the result.
fn covered_fraction(rect: &RECT, work: &RECT) -> f64 {
    let left = rect.left.max(work.left);
    let top = rect.top.max(work.top);
    let right = rect.right.min(work.right);
    let bottom = rect.bottom.min(work.bottom);
    if right <= left || bottom <= top {
        return 0.0;
    }
    let total = (work.right - work.left) as i64 * (work.bottom - work.top) as i64;
    if total <= 0 {
        return 0.0;
    }
    ((right - left) as i64 * (bottom - top) as i64) as f64 / total as f64
}

struct CoverCtx<'a> {
    own_hwnds: &'a [isize],
    monitor: HMONITOR,
    found: bool,
}

unsafe extern "system" fn enum_cover_proc(top: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut CoverCtx<'_>);
    if !eligible_window(top, ctx.own_hwnds) {
        return BOOL(1);
    }
    unsafe {
        if MonitorFromWindow(top, MONITOR_DEFAULTTONEAREST) != ctx.monitor {
            return BOOL(1);
        }
        let mut rect = RECT::default();
        if GetWindowRect(top, &mut rect).is_err() {
            return BOOL(1);
        }
        let Some(info) = monitor_info(ctx.monitor) else {
            return BOOL(0);
        };
        let covers = IsZoomed(top).as_bool() || covered_fraction(&rect, &info.rcWork) >= 0.95;
        if covers {
            ctx.found = true;
            return BOOL(0);
        }
    }
    BOOL(1)
}

/// True when any visible top-level window on the wallpaper's monitor is
/// maximized (`IsZoomed`) or covers at least 95% of that monitor's work area.
/// Foreground is irrelevant: a maximized window behind another window still
/// hides the desktop.
pub fn covering_window(own_hwnds: &[isize], wallpaper: HWND) -> bool {
    unsafe {
        let monitor = MonitorFromWindow(wallpaper, MONITOR_DEFAULTTOPRIMARY);
        let mut ctx = CoverCtx {
            own_hwnds,
            monitor,
            found: false,
        };
        let _ = EnumWindows(
            Some(enum_cover_proc),
            LPARAM(&mut ctx as *mut CoverCtx<'_> as isize),
        );
        ctx.found
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::RECT;

    use super::covered_fraction;

    #[test]
    fn work_area_coverage_clips_to_the_monitor() {
        let work = RECT {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        };
        let outside = RECT {
            left: -100,
            top: 0,
            right: 100,
            bottom: 100,
        };
        assert!((covered_fraction(&outside, &work) - 1.0).abs() < f64::EPSILON);

        let small = RECT {
            left: 10,
            top: 10,
            right: 50,
            bottom: 50,
        };
        assert!((covered_fraction(&small, &work) - 0.16).abs() < f64::EPSILON);
    }
}
