use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::model::{quality_max_height, YtdlpInfo};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Finds yt-dlp: explicit setting first, then next to the exe, then PATH.
/// An explicit but invalid path is respected (returns None) so the UI can show
/// "not found" instead of silently using a different binary.
pub fn locate(custom: &str) -> Option<PathBuf> {
    let custom = custom.trim();
    if !custom.is_empty() {
        let path = PathBuf::from(custom);
        if path.is_file() {
            return Some(path);
        }
        if path.is_dir() {
            let nested = path.join("yt-dlp.exe");
            if nested.is_file() {
                return Some(nested);
            }
        }
        return None;
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in ["yt-dlp.exe", "yt-dlp"] {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    for name in ["yt-dlp.exe", "yt-dlp"] {
        if let Some(found) = which(name) {
            return Some(found);
        }
    }
    None
}

fn which(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn info(custom: &str) -> YtdlpInfo {
    let Some(path) = locate(custom) else {
        return YtdlpInfo::default();
    };
    let version = run_capture(&path, &["--version"], Duration::from_secs(8))
        .ok()
        .and_then(|stdout| stdout.lines().next().map(str::trim).map(str::to_string))
        .filter(|line| !line.is_empty());
    YtdlpInfo {
        found: true,
        path: Some(path.to_string_lossy().to_string()),
        version,
    }
}

/// Resolves a direct, video-only stream URL. Video-only is intentional: the
/// wallpaper is muted, so there is no need to download a second audio track,
/// and adaptive formats are what make 1440p/4K possible at all.
pub fn resolve(exe: &Path, url: &str, quality: &str) -> Result<String, String> {
    let height = quality_max_height(quality);
    // Prefer H.264 for hardware decoding, then fall back to any codec.
    let format = format!(
        "bv*[height<={height}][vcodec^=avc1]/bv*[height<={height}]/b[height<={height}]/bv*/b"
    );
    let args = [
        "-f",
        &format,
        "-g",
        "--no-playlist",
        "--no-warnings",
        "--no-progress",
        "--no-cache-dir",
        "--socket-timeout",
        "15",
        "--retries",
        "2",
        url,
    ];
    let stdout = run_capture(exe, &args, Duration::from_secs(30))
        .map_err(|error| format!("yt-dlp: {error}"))?;
    stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("http://") || line.starts_with("https://"))
        .map(str::to_string)
        .ok_or_else(|| "yt-dlp не вернул URL потока".to_string())
}

fn run_capture(exe: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "превышено время ожидания ({} с)",
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(error) => return Err(error.to_string()),
        }
    }

    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("неизвестная ошибка");
        return Err(last.trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
