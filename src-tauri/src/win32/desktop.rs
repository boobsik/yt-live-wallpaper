//! Win32 desktop integration: the WorkerW trick, window parenting and the
//! native wallpaper restore path.
//!
//! Explorer hosts the desktop in `Progman`. Sending `0x052C` makes it spawn a
//! `WorkerW` sibling; the wallpaper is then a child window of that WorkerW,
//! which renders behind `SHELLDLL_DefView` (the desktop icons).

use std::ffi::c_void;

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetParent, GetSystemMetrics,
    GetWindowLongPtrW, IsWindow, SendMessageTimeoutW, SetParent, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, SystemParametersInfoW, GWL_EXSTYLE, GWL_STYLE, HWND_BOTTOM, SMTO_NORMAL,
    SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, SM_CXSCREEN,
    SM_CYSCREEN, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WS_CAPTION, WS_CHILD, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
    WS_POPUP, WS_SYSMENU,
    WS_THICKFRAME,
};

const WM_SPAWN_WORKERW: u32 = 0x052C;

struct EnumCtx {
    progman: HWND,
    shell_parent: Option<HWND>,
    shell_view: Option<HWND>,
}

unsafe extern "system" fn enum_windows_proc(top: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumCtx);
    if top == ctx.progman {
        return BOOL(1);
    }

    // SHELLDLL_DefView may live directly under Progman (common on Windows 11
    // 24H2+) or under a top-level WorkerW (classic layout). We record whichever
    // window actually owns the icon view.
    let shell = FindWindowExW(Some(top), None, w!("SHELLDLL_DefView"), PCWSTR::null())
        .unwrap_or_default();
    if !shell.0.is_null() {
        ctx.shell_parent = Some(top);
        ctx.shell_view = Some(shell);
        return BOOL(0); // stop: the icon host is found
    }
    BOOL(1)
}

fn find_child_workerw(parent: HWND) -> Option<HWND> {
    unsafe { FindWindowExW(Some(parent), None, w!("WorkerW"), PCWSTR::null()).ok() }
        .filter(|hwnd| !hwnd.0.is_null())
}

/// Finds the window that should host the wallpaper behind desktop icons.
///
/// Windows 11 24H2+ often keeps `SHELLDLL_DefView` as a child of `Progman` and
/// places a child `WorkerW` behind it. Classic builds place `SHELLDLL_DefView`
/// inside a top-level `WorkerW` and the wallpaper host is the next top-level
/// `WorkerW`. We handle both so icons stay clickable above the wallpaper.
pub fn find_workerw() -> Option<HWND> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null()).ok()?;
        if progman.0.is_null() {
            return None;
        }

        // Ask Explorer to split/create the wallpaper WorkerW. 0xD, 0xD, 1 is the
        // variant that reliably creates the child WorkerW on modern Windows 11.
        let mut result = 0usize;
        let _ = SendMessageTimeoutW(
            progman,
            WM_SPAWN_WORKERW,
            WPARAM(0xD),
            LPARAM(0x1),
            SMTO_NORMAL,
            1000,
            Some(&mut result),
        );

        let mut ctx = EnumCtx {
            progman,
            shell_parent: None,
            shell_view: None,
        };
        let _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM(&mut ctx as *mut EnumCtx as isize),
        );

        let shell_parent = ctx.shell_parent.unwrap_or(progman);
        let shell_view = ctx.shell_view.unwrap_or_else(|| {
            FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), PCWSTR::null())
                .unwrap_or_default()
        });

        // Windows 11 24H2+: SHELLDLL_DefView is inside Progman; the wallpaper
        // WorkerW is a child of Progman, behind the icon view.
        if shell_parent == progman {
            if let Some(workerw) = find_child_workerw(progman) {
                return Some(workerw);
            }
        }

        // Classic layout: SHELLDLL_DefView is inside a top-level WorkerW; the
        // wallpaper host is the next top-level WorkerW in z-order.
        if !shell_view.0.is_null() {
            if let Ok(next) =
                FindWindowExW(None, Some(shell_parent), w!("WorkerW"), PCWSTR::null())
            {
                if !next.0.is_null() {
                    return Some(next);
                }
            }
        }

        find_child_workerw(progman).or(Some(shell_parent))
    }
}

/// Reparents the wallpaper HWND into the WorkerW and makes it fill the desktop.
/// `SetWindowPos` with `HWND_BOTTOM` keeps the wallpaper behind `SHELLDLL_DefView`
/// so desktop icons remain visible and clickable.
pub fn attach(hwnd: HWND, workerw: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let stripped = style
            & !(WS_POPUP.0
                | WS_CAPTION.0
                | WS_THICKFRAME.0
                | WS_MINIMIZEBOX.0
                | WS_MAXIMIZEBOX.0
                | WS_SYSMENU.0);
        SetWindowLongPtrW(hwnd, GWL_STYLE, (stripped | WS_CHILD.0) as isize);

        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        // WS_EX_TRANSPARENT lets mouse input pass through the wallpaper window to
        // the icon view above it; WS_EX_NOACTIVATE prevents stealing focus.
        let ex_style = (ex_style | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_TRANSPARENT.0)
            & !WS_EX_APPWINDOW.0;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);

        if SetParent(hwnd, Some(workerw)).is_err() {
            return;
        }

        let (width, height) = screen_size();
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_BOTTOM),
            0,
            0,
            width,
            height,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

pub fn parent_of(hwnd: HWND) -> HWND {
    unsafe { GetParent(hwnd).unwrap_or_default() }
}

pub fn is_valid_window(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    unsafe { IsWindow(Some(HWND(hwnd as *mut c_void))) }.as_bool()
}

pub fn show_raw(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

pub fn hide_raw(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// Physical size of the primary display.
pub fn screen_size() -> (i32, i32) {
    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        (width.max(320), height.max(240))
    }
}

pub fn desktop_class(class: &str) -> bool {
    matches!(
        class,
        "Progman"
            | "WorkerW"
            | "SHELLDLL_DefView"
            | "SysListView32"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "Windows.UI.Core.CoreWindow"
            | "MultitaskingViewFrame"
            | "ForegroundStaging"
            | "XamlExplorerHostIslandWindow"
    ) || class.starts_with("WorkerW")
}

/// Snapshot of the current desktop wallpaper, stored so it can be restored when
/// the live wallpaper is stopped or the app quits.
pub fn capture_wallpaper() -> String {
    let mut buffer = vec![0u16; 32_768];
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buffer.len() as u32,
            Some(buffer.as_mut_ptr() as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_err() {
        return String::new();
    }
    let len = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

/// Applies a static Windows wallpaper. Returns false when the path is empty or
/// no longer exists, so we never blank the desktop on restore.
pub fn set_wallpaper(path: &str) -> bool {
    let path = path.trim();
    if path.is_empty() || !std::path::Path::new(path).is_file() {
        return false;
    }
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(wide.as_ptr() as *mut c_void),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
        .is_ok()
    }
}

/// Stock Windows wallpaper used when the captured path no longer exists
/// (Spotlight rotates its files, external drives get unplugged, ...).
pub fn fallback_wallpaper() -> Option<String> {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    let candidates = [
        format!(r"{root}\Web\Wallpaper\Windows\img0.jpg"),
        format!(r"{root}\Web\Wallpaper\Windows\img19.jpg"),
        format!(r"{root}\Web\Wallpaper\Themes\Windows 11\img0.jpg"),
    ];
    candidates
        .into_iter()
        .find(|candidate| std::path::Path::new(candidate).is_file())
}
