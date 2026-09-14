//! 服务端剪贴板文本同步

use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// 读剪贴板文本 (重试若干次, 剪贴板常被别的进程占用)
pub fn get_clipboard_text() -> Option<String> {
    unsafe {
        for _ in 0..5 {
            if OpenClipboard(None).is_err() {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            let result = (|| {
                let h = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
                let ptr = GlobalLock(HGLOBAL(h.0)) as *const u16;
                if ptr.is_null() {
                    return None;
                }
                let mut len = 0usize;
                while *ptr.add(len) != 0 && len < 16 * 1024 * 1024 {
                    len += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                let _ = GlobalUnlock(HGLOBAL(h.0));
                Some(s)
            })();
            let _ = CloseClipboard();
            return result;
        }
        None
    }
}

/// 最近一次由对端同步写入的文本 (轮询时跳过, 避免回显)
static LAST_LOCAL_SET: Mutex<String> = Mutex::new(String::new());

/// 写剪贴板文本
pub fn set_clipboard_text(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * 2;
    unsafe {
        for _ in 0..5 {
            if OpenClipboard(None).is_err() {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            let ok = (|| -> bool {
                if let Err(e) = EmptyClipboard() {
                    crate::logger::log_line("WARN", &format!("set_clip: EmptyClipboard err {:?}", e));
                    return false;
                }
                let h = match GlobalAlloc(GMEM_MOVEABLE, bytes) {
                    Ok(h) => h,
                    Err(_) => return false,
                };
                let ptr = GlobalLock(h) as *mut u16;
                if ptr.is_null() {
                    return false;
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
                let _ = GlobalUnlock(h);
                match SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(h.0)) {
                    Ok(_) => {
                        // 写入对端同步标记要在 CloseClipboard 之前, 消除与轮询线程的回显竞态窗口
                        *LAST_LOCAL_SET.lock().unwrap() = text.to_string();
                        true
                    }
                    Err(e) => {
                        crate::logger::log_line("WARN", &format!("set_clip: SetClipboardData err {:?}", e));
                        false
                    }
                }
            })();
            let _ = CloseClipboard();
            if ok {
                return true;
            }
            thread::sleep(Duration::from_millis(15)); // 内部失败也重试
        }
        crate::logger::log_line("WARN", "set_clip: 写入失败 (重试耗尽)");
        false
    }
}

/// 剪贴板轮询线程: 本地剪贴板变化时通过 tx 发给客户端
pub fn clipboard_poll_loop(
    tx: std::sync::mpsc::Sender<String>,
    stop: &dyn Fn() -> bool,
    is_secure: impl Fn() -> bool + Send + 'static,
) {
    let mut last_sent = String::new();
    loop {
        if stop() {
            return;
        }
        thread::sleep(Duration::from_millis(800));
        // 安全桌面(锁屏)上剪贴板不可用
        if is_secure() {
            continue;
        }
        if let Some(cur) = get_clipboard_text() {
            let just_set = LAST_LOCAL_SET.lock().unwrap().clone();
            if cur != last_sent && cur != just_set && !cur.is_empty() {
                last_sent = cur.clone();
                if tx.send(cur).is_err() {
                    return;
                }
            }
        }
    }
}
