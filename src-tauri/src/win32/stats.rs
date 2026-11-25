//! Win32 telemetry: working set and CPU time for the app and every WebView2
//! process in its process tree. No external crates, no WMI, no PowerShell.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
};

use crate::model::Telemetry;

struct CpuSample {
    at: Instant,
    ticks: HashMap<u32, u64>,
}

static LAST_SAMPLE: OnceLock<Mutex<Option<CpuSample>>> = OnceLock::new();
static CORES: OnceLock<f64> = OnceLock::new();

fn cores() -> f64 {
    *CORES.get_or_init(|| {
        std::thread::available_parallelism()
            .map(|value| value.get() as f64)
            .unwrap_or(8.0)
    })
}

fn filetime_ticks(time: FILETIME) -> u64 {
    ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64
}

fn cpu_ticks(handle: HANDLE) -> Option<u64> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user).ok()?;
    }
    Some(filetime_ticks(kernel) + filetime_ticks(user))
}

fn working_set(handle: HANDLE) -> u64 {
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            handle,
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    };
    if ok.is_err() {
        0
    } else {
        counters.WorkingSetSize as u64
    }
}

struct ProcessRow {
    pid: u32,
    parent: u32,
    name: String,
}

fn process_snapshot() -> Vec<ProcessRow> {
    let mut rows = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return rows;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|&unit| unit == 0)
                    .unwrap_or(entry.szExeFile.len());
                rows.push(ProcessRow {
                    pid: entry.th32ProcessID,
                    parent: entry.th32ParentProcessID,
                    name: String::from_utf16_lossy(&entry.szExeFile[..end]).to_ascii_lowercase(),
                });
                if entry.th32ProcessID == 0 {
                    break;
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    rows
}

/// PIDs of our own process plus every descendant (WebView2 browser process,
/// renderers, GPU process, utility processes...).
fn tree_pids(rows: &[ProcessRow], root: u32) -> Vec<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for row in rows {
        children.entry(row.parent).or_default().push(row.pid);
    }
    let mut result = vec![root];
    let mut index = 0;
    while index < result.len() {
        let pid = result[index];
        index += 1;
        if let Some(list) = children.get(&pid) {
            for child in list {
                if *child != 0 && !result.contains(child) {
                    result.push(*child);
                }
            }
        }
    }
    result
}

pub fn sample() -> Telemetry {
    let rows = process_snapshot();
    let own_pid = std::process::id();
    let pids = tree_pids(&rows, own_pid);

    let mut node_bytes = 0u64;
    let mut webview_bytes = 0u64;
    let mut ticks: HashMap<u32, u64> = HashMap::new();
    let mut counted = 0u32;

    for pid in &pids {
        let name = rows
            .iter()
            .find(|row| row.pid == *pid)
            .map(|row| row.name.as_str())
            .unwrap_or_default();
        let is_webview = name == "msedgewebview2.exe";
        let is_self = *pid == own_pid;
        if !is_webview && !is_self {
            continue;
        }
        let access = PROCESS_QUERY_INFORMATION | PROCESS_VM_READ;
        let Ok(handle) = (unsafe { OpenProcess(access, false, *pid) }) else {
            continue;
        };
        let bytes = working_set(handle);
        if is_self {
            node_bytes += bytes;
        } else {
            webview_bytes += bytes;
        }
        if let Some(value) = cpu_ticks(handle) {
            ticks.insert(*pid, value);
        }
        let _ = unsafe { CloseHandle(handle) };
        counted += 1;
    }

    let now = Instant::now();
    let mut cpu = 0.0f64;
    let lock = LAST_SAMPLE.get_or_init(|| Mutex::new(None));
    {
        let mut guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(previous) = guard.as_ref() {
            let wall = now.duration_since(previous.at).as_secs_f64();
            if wall > 0.05 {
                let mut delta = 0u64;
                for (pid, current) in &ticks {
                    if let Some(old) = previous.ticks.get(pid) {
                        delta += current.saturating_sub(*old);
                    }
                }
                // FILETIME is 100 ns per tick.
                let seconds = delta as f64 / 10_000_000.0;
                cpu = (seconds / wall / cores() * 100.0).clamp(0.0, 100.0);
            }
        }
        *guard = Some(CpuSample { at: now, ticks });
    }

    Telemetry {
        node_ram_mb: round1(node_bytes as f64 / 1_048_576.0),
        webview_ram_mb: round1(webview_bytes as f64 / 1_048_576.0),
        cpu: round1(cpu),
        processes: counted,
        sampled_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0),
        engine: "idle".to_string(),
        engine_reason: None,
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}
