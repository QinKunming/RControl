//! 桌面(Desktop)附加工具: 锁屏/登录界面时输入桌面会切换到 winlogon 安全桌面,
//! 捕获与输入注入线程必须先附加到当前输入桌面才能工作 (参考 rustdesk 做法)

use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, SetThreadDesktop, HDESK,
    DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, DESKTOP_CREATEMENU, DESKTOP_CREATEWINDOW,
    DESKTOP_ENUMERATE, DESKTOP_HOOKCONTROL, DESKTOP_JOURNALPLAYBACK, DESKTOP_JOURNALRECORD,
    DESKTOP_READOBJECTS, DESKTOP_SWITCHDESKTOP, DESKTOP_WRITEOBJECTS, UOI_NAME,
};

/// 线程本地的桌面附加状态。
/// 用法: 捕获/输入线程持有它, 每帧(或每次注入前)调 ensure()。
pub struct DesktopState {
    desk_name: String,
    hdesk: Option<HDESK>,
    last_check: Instant,
}

impl DesktopState {
    pub fn new() -> Self {
        DesktopState { desk_name: String::new(), hdesk: None, last_check: Instant::now() - Duration::from_secs(10) }
    }

    /// 是否已附加到当前输入桌面; 距上次检查超过 min_interval 则重新检查
    pub fn ensure(&mut self, min_interval: Duration) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_check) < min_interval {
            return !self.desk_name.is_empty();
        }
        self.last_check = now;
        self.force_attach()
    }

    /// 强制重新打开输入桌面并比较, 变化时切换线程桌面
    pub fn force_attach(&mut self) -> bool {
        unsafe {
            let desired = DESKTOP_ACCESS_FLAGS(
                DESKTOP_CREATEMENU.0
                    | DESKTOP_CREATEWINDOW.0
                    | DESKTOP_ENUMERATE.0
                    | DESKTOP_HOOKCONTROL.0
                    | DESKTOP_JOURNALPLAYBACK.0
                    | DESKTOP_JOURNALRECORD.0
                    | DESKTOP_READOBJECTS.0
                    | DESKTOP_SWITCHDESKTOP.0
                    | DESKTOP_WRITEOBJECTS.0,
            );
            let desk = match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, desired) {
                Ok(d) => d,
                Err(_) => return false,
            };
            let name = desktop_name(desk);
            if name != self.desk_name {
                if SetThreadDesktop(desk).is_err() {
                    let _ = CloseDesktop(desk);
                    return false;
                }
                // 切换成功: 关闭旧句柄, 保留新句柄
                if let Some(old) = self.hdesk.take() {
                    let _ = CloseDesktop(old);
                }
                self.hdesk = Some(desk);
                let was_secure = self.is_secure();
                self.desk_name = name;
                if was_secure != self.is_secure() {
                    crate::logger::log_line(
                        "INFO",
                        &format!("desktop switched -> {} (secure={})", self.desk_name, self.is_secure()),
                    );
                }
            } else {
                let _ = CloseDesktop(desk);
            }
            true
        }
    }

    /// 安全桌面 = 非 Default 桌面 (锁屏/登录/UAC)
    pub fn is_secure(&self) -> bool {
        !self.desk_name.is_empty() && self.desk_name != "Default"
    }

    pub fn name(&self) -> &str {
        &self.desk_name
    }
}

unsafe fn desktop_name(desk: HDESK) -> String {
    let mut buf = [0u16; 128];
    let mut needed = 0u32;
    let ok = GetUserObjectInformationW(
        HANDLE(desk.0),
        UOI_NAME,
        Some(buf.as_mut_ptr().cast()),
        (buf.len() * 2) as u32,
        Some(&mut needed),
    )
    .is_ok();
    if !ok {
        return String::new();
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// 让本进程用物理像素坐标 (仅调用一次)
pub fn set_process_dpi_aware() {
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetProcessDPIAware();
    }
}

/// 忽略未使用警告的辅助 (CloseHandle 在部分编译单元未用)
#[allow(dead_code)]
pub fn close_handle(h: HANDLE) {
    unsafe {
        let _ = CloseHandle(h);
    }
}

#[allow(dead_code)]
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[allow(dead_code)]
pub fn pcwstr(s: &str) -> PCWSTR {
    PCWSTR::from_raw(s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>().as_ptr())
}
