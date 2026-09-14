//! 客户端剪贴板读写 (Win32)

use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;

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
                if EmptyClipboard().is_err() {
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
                SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(h.0)).is_ok()
            })();
            let _ = CloseClipboard();
            if ok {
                return true;
            }
            thread::sleep(Duration::from_millis(15)); // 内部失败也重试
        }
        false
    }
}
