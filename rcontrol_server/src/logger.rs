//! 极简文件日志 (windows 子系统程序无控制台)

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG: Mutex<Option<File>> = Mutex::new(None);
static VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn init(tag: &str) {
    let dir = rcontrol_common::config::server_data_dir().join("logs");
    let _ = fs::create_dir_all(&dir);
    let path: PathBuf = dir.join(format!("{}.log", tag));
    // 超过 4MB 轮转
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() > 4 * 1024 * 1024 {
            let _ = fs::remove_file(&path);
        }
    }
    let f = OpenOptions::new().create(true).append(true).open(&path).ok();
    *LOG.lock().unwrap() = f;
    VERBOSE.store(std::env::var("RCONTROL_VERBOSE").is_ok(), std::sync::atomic::Ordering::Relaxed);
}

pub fn log_line(level: &str, msg: &str) {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let line = format!("[{} {}] {}\r\n", fmt_ts(ts as u64), level, msg);
    if let Ok(mut g) = LOG.lock() {
        if let Some(f) = g.as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }
    if VERBOSE.load(std::sync::atomic::Ordering::Relaxed) {
        print!("{}", line);
    }
}

/// unix 毫秒 -> "HH:MM:SS.mmm" (本地时间, 用 Win32 API 转换)
fn fmt_ts(unix_ms: u64) -> String {
    use windows::Win32::Foundation::*;
    let ms = unix_ms as i64;
    unsafe {
        let mut st = SYSTEMTIME::default();
        let mut ft = FILETIME::default();
        // unix epoch -> windows epoch (11644473600s)
        let win_ticks = (ms + 11644473600_000).wrapping_mul(10_000);
        ft.dwLowDateTime = (win_ticks & 0xFFFF_FFFF) as u32;
        ft.dwHighDateTime = (win_ticks >> 32) as u32;
        let _ = windows::Win32::System::Time::FileTimeToSystemTime(&ft, &mut st);
        format!(
            "{:02}:{:02}:{:02}.{:03}",
            st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
        )
    }
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::logger::log_line("INFO", &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::logger::log_line("ERROR", &format!($($arg)*)) };
}
