//! 鼠标/键盘注入 (SendInput, 扫描码模式) + SAS(Ctrl+Alt+Del) 发送
//! 注入前需附加到当前输入桌面才能作用于锁屏/登录界面

use crate::desktop::DesktopState;
use rcontrol_common::proto::{KeyboardEvent, MouseEvent};
use std::sync::mpsc::Receiver;
use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS, MOUSEINPUT,
    MOUSE_EVENT_FLAGS,
};

// MOUSEEVENTF_* / KEYEVENTF_* (newtype 不支持 |, 直接用原始数值)
const MF_MOVE: u32 = 0x0001;
const MF_LEFTDOWN: u32 = 0x0002;
const MF_LEFTUP: u32 = 0x0004;
const MF_RIGHTDOWN: u32 = 0x0008;
const MF_RIGHTUP: u32 = 0x0010;
const MF_MIDDLEDOWN: u32 = 0x0040;
const MF_MIDDLEUP: u32 = 0x0080;
const MF_WHEEL: u32 = 0x0800;
const MF_HWHEEL: u32 = 0x1000;
const MF_VIRTUALDESK: u32 = 0x4000;
const MF_ABSOLUTE: u32 = 0x8000;
const KF_EXTENDEDKEY: u32 = 0x0001;
const KF_KEYUP: u32 = 0x0002;
const KF_SCANCODE: u32 = 0x0008;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN};

pub enum InputEvent {
    Mouse(MouseEvent),
    Key(KeyboardEvent),
    Sas,
}

// proto::MOUSE_* 常量为 u32
use rcontrol_common::proto;

/// 输入注入线程: 独占一个线程, 每次注入前确保已附加到当前输入桌面
/// frame_size 返回当前编码帧尺寸 (画质压缩降采样时帧 < 屏幕, 客户端坐标是帧坐标)
pub fn input_loop(
    rx: Receiver<InputEvent>,
    stop: &dyn Fn() -> bool,
    frame_size: &dyn Fn() -> (u32, u32),
) {
    let mut desktop = DesktopState::new();
    let mut batch: Vec<InputEvent> = Vec::with_capacity(64);
    loop {
        if stop() {
            return;
        }
        // 阻塞等第一个事件, 然后尽量收割积压
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(ev) => batch.push(ev),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // 空闲时也定期检查桌面切换
                desktop.ensure(Duration::from_millis(500));
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
        while batch.len() < 64 {
            match rx.try_recv() {
                Ok(ev) => batch.push(ev),
                Err(_) => break,
            }
        }
        // 注入前附加到输入桌面
        desktop.ensure(Duration::from_millis(200));
        let fsize = frame_size();
        for ev in batch.drain(..) {
            match ev {
                InputEvent::Mouse(m) => inject_mouse(&m, fsize),
                InputEvent::Key(k) => inject_key(&k),
                InputEvent::Sas => {
                    send_sas();
                }
            }
        }
    }
}

fn inject_mouse(ev: &MouseEvent, frame_size: (u32, u32)) {
    unsafe {
        let (vw, vh) = (
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        );
        // 客户端坐标基于它看到的帧 (可能被 max_width 降采样, 且以虚拟屏幕左上角为 0,0)
        // 换算成虚拟桌面绝对坐标后再归一化
        let (fx, fy) = {
            let (fw, fh) = (frame_size.0.max(1) as f64, frame_size.1.max(1) as f64);
            (
                (ev.x.clamp(0.0, fw - 1.0) * vw as f64 / fw) as i32,
                (ev.y.clamp(0.0, fh - 1.0) * vh as f64 / fh) as i32,
            )
        };
        let norm = |v: i32, size: i32| -> i32 { (v.clamp(0, size - 1) * 65535) / (size - 1).max(1) };
        let mut inputs: Vec<INPUT> = Vec::new();
        let mk = |flags: u32, dx: i32, dy: i32, data: i32| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data as u32,
                    dwFlags: MOUSE_EVENT_FLAGS(flags),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let move_flags = MF_MOVE | MF_ABSOLUTE | MF_VIRTUALDESK;
        let (nx, ny) = (norm(fx, vw), norm(fy, vh));
        match ev.mask {
            proto::MOUSE_MOVE => {
                inputs.push(mk(move_flags, nx, ny, 0));
            }
            proto::MOUSE_LEFT_DOWN | proto::MOUSE_LEFT_UP => {
                let down = ev.mask == proto::MOUSE_LEFT_DOWN;
                inputs.push(mk(move_flags | if down { MF_LEFTDOWN } else { MF_LEFTUP }, nx, ny, 0));
            }
            proto::MOUSE_RIGHT_DOWN | proto::MOUSE_RIGHT_UP => {
                let down = ev.mask == proto::MOUSE_RIGHT_DOWN;
                inputs.push(mk(move_flags | if down { MF_RIGHTDOWN } else { MF_RIGHTUP }, nx, ny, 0));
            }
            proto::MOUSE_MIDDLE_DOWN | proto::MOUSE_MIDDLE_UP => {
                let down = ev.mask == proto::MOUSE_MIDDLE_DOWN;
                inputs.push(mk(move_flags | if down { MF_MIDDLEDOWN } else { MF_MIDDLEUP }, nx, ny, 0));
            }
            proto::MOUSE_WHEEL_V => {
                inputs.push(mk(MF_WHEEL, 0, 0, ev.wheel * 120));
            }
            proto::MOUSE_WHEEL_H => {
                inputs.push(mk(MF_HWHEEL, 0, 0, ev.wheel * 120));
            }
            _ => {}
        }
        if !inputs.is_empty() {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }
}

fn inject_key(ev: &KeyboardEvent) {
    // bit9 (0x100) = E0 扩展键
    let scan = ev.scancode & 0xff;
    let extended = ev.scancode & 0x100 != 0;
    if scan == 0 {
        return;
    }
    let mut flags = KF_SCANCODE;
    if extended {
        flags |= KF_EXTENDEDKEY;
    }
    if !ev.down {
        flags |= KF_KEYUP;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
            ki: KEYBDINPUT {
                wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(0),
                wScan: scan as u16,
                dwFlags: KEYBD_EVENT_FLAGS(flags),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

/// 发送 SAS (Ctrl+Alt+Del)。要求 SYSTEM 权限。
/// sas.dll 的未文档化导出; 注册表 SoftwareSASGeneration 需允许 Services 生成 SAS。
pub fn send_sas() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE,
        KEY_READ, KEY_WRITE, REG_DWORD, REG_SAM_FLAGS, REG_VALUE_TYPE,
    };
    use windows::Win32::Foundation::BOOL;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    // gnu (Win7) 工具链没有 sas 的导入库, 静态 #[link] 链不过; 运行时解析
    let send_sas_api = match unsafe { resolve_send_sas() } {
        Some(f) => f,
        None => return false,
    };


    unsafe {
        const SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\System";
        let subkey = wide(SUBKEY);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey.as_ptr()),
            0,
            REG_SAM_FLAGS(KEY_READ.0 | KEY_WRITE.0),
            &mut hkey,
        )
        .0
            != 0
        {
            crate::logger::log_line("ERROR", "send_sas: open Policies\\System failed (need SYSTEM)");
            return false;
        }
        // 读旧值
        let vname = wide("SoftwareSASGeneration");
        let mut old: (u32, bool) = (0, false); // (值, 是否存在)
        let mut dtype = REG_VALUE_TYPE(0);
        let mut dsize = 4u32;
        let mut buf = [0u8; 4];
        if RegQueryValueExW(hkey, PCWSTR(vname.as_ptr()), None, Some(&mut dtype), Some(buf.as_mut_ptr()), Some(&mut dsize)).0 == 0
            && dtype == REG_DWORD
        {
            old = (u32::from_le_bytes(buf), true);
        }
        // 若不允许服务生成 SAS, 临时改为 1 (Services)
        let need_restore = old.0 != 1 && old.0 != 3;
        if need_restore {
            let _ = RegSetValueExW(hkey, PCWSTR(vname.as_ptr()), 0, REG_DWORD, Some(&1u32.to_le_bytes()));
        }
        send_sas_api(BOOL(0));
        if need_restore {
            if old.1 {
                let _ = RegSetValueExW(hkey, PCWSTR(vname.as_ptr()), 0, REG_DWORD, Some(&old.0.to_le_bytes()));
            } else {
                // 原值不存在: 恢复为 0 (默认禁止)
                let _ = RegSetValueExW(hkey, PCWSTR(vname.as_ptr()), 0, REG_DWORD, Some(&0u32.to_le_bytes()));
            }
        }
        let _ = RegCloseKey(hkey);
        crate::logger::log_line("INFO", "send_sas: SendSAS(FALSE) sent");
        true
    }
}

/// 运行时解析 sas.dll!SendSAS。模块保持加载不释放 (worker 生命周期内只调一次, 无害)。
unsafe fn resolve_send_sas() -> Option<unsafe extern "system" fn(windows::Win32::Foundation::BOOL)> {
    use windows::core::{PCSTR, PCWSTR};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    let mut name = [0u16; 9];
    for (i, c) in "sas.dll".encode_utf16().enumerate() {
        name[i] = c;
    }
    let h = match LoadLibraryW(PCWSTR(name.as_ptr())) {
        Ok(h) => h,
        Err(_) => {
            crate::logger::log_line("ERROR", "send_sas: LoadLibraryW(sas.dll) failed");
            return None;
        }
    };
    match GetProcAddress(h, PCSTR(b"SendSAS\0".as_ptr())) {
        Some(f) => Some(std::mem::transmute(f)),
        None => {
            crate::logger::log_line("ERROR", "send_sas: sas.dll 无 SendSAS 导出");
            None
        }
    }
}
