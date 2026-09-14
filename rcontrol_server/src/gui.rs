//! 服务端设置 GUI (原生 Win32, 无额外依赖)

use crate::service_manager as sm;
use rcontrol_common::config::{load_server_config, save_server_config, ServerConfig, server_data_dir};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontIndirectW, DEFAULT_PITCH,
    FF_DONTCARE, FW_NORMAL, GetDC, GetDeviceCaps, HBRUSH, LOGPIXELSY, LOGFONTW,
    OUT_DEFAULT_PRECIS, ReleaseDC, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

// 样式常量 (直接用数值避免 newtype 混用)
const WS_CHILD_V: u32 = 0x4000_0000 | 0x1000_0000; // WS_CHILD|WS_VISIBLE
const STYLE_EDIT: u32 = WS_CHILD_V | 0x0080_0000 | 0x0080; // |WS_BORDER|ES_AUTOHSCROLL
const STYLE_BTN: u32 = WS_CHILD_V | 0x0001_0000; // |WS_TABSTOP
const STYLE_CHK: u32 = WS_CHILD_V | 0x0003; // BS_AUTOCHECKBOX
const STYLE_STATIC: u32 = WS_CHILD_V;
const STYLE_GROUP: u32 = WS_CHILD_V | 0x0007; // BS_GROUPBOX
const STYLE_LBL_R: u32 = WS_CHILD_V | 0x0002; // SS_RIGHT (标签右对齐, 与输入框成列)
const WS_EX_STATICEDGE: u32 = 0x0002_0000;

/// 全局字体 (Segoe UI 9pt, 一次创建)
static FONT: std::sync::OnceLock<isize> = std::sync::OnceLock::new();

/// DPI 缩放系数
fn dpi_scale() -> f32 {
    unsafe {
        let hdc = GetDC(None);
        let dpi = if hdc.is_invalid() { 96 } else { GetDeviceCaps(hdc, LOGPIXELSY) };
        if !hdc.is_invalid() {
            ReleaseDC(None, hdc);
        }
        dpi as f32 / 96.0
    }
}

fn sc(v: i32, s: f32) -> i32 {
    (v as f32 * s) as i32
}

// 控件 ID
const ID_PORT: i32 = 100;
const ID_PWD: i32 = 101;
const ID_FPS: i32 = 102;
const ID_QUALITY: i32 = 103;
const ID_MAXW: i32 = 104;
const ID_CHK_FILE: i32 = 105;
const ID_CHK_CLIP: i32 = 106;
const ID_BTN_SAVE: i32 = 1;
const ID_BTN_INSTALL: i32 = 2;
const ID_BTN_STOP_SVC: i32 = 3;
const ID_BTN_UNINSTALL: i32 = 4;
const ID_BTN_TEMP_RUN: i32 = 5;
const ID_BTN_TEMP_STOP: i32 = 6;
const ID_BTN_COPY: i32 = 7;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct GuiState {
    main: HWND,
    port: HWND,
    pwd: HWND,
    fps: HWND,
    quality: HWND,
    maxw: HWND,
    chk_file: HWND,
    chk_clip: HWND,
    status: HWND,
}

unsafe fn create_control(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: i32,
) -> HWND {
    let class_w = wide(class);
    let text_w = wide(text);
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR::from_raw(class_w.as_ptr()),
        PCWSTR::from_raw(text_w.as_ptr()),
        WINDOW_STYLE(style),
        x,
        y,
        w,
        h,
        parent,
        HMENU(id as isize as *mut _),
        HINSTANCE(hinst.0),
        None,
    )
    .expect("create control");
    if let Some(f) = FONT.get() {
        SendMessageW(hwnd, WM_SETFONT, WPARAM(*f as usize), LPARAM(1));
    }
    hwnd
}

unsafe fn get_text(hwnd: HWND) -> String {
    let len = SendMessageW(hwnd, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0 as usize;
    if len == 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len + 1];
    SendMessageW(hwnd, WM_GETTEXT, WPARAM(len + 1), LPARAM(buf.as_mut_ptr() as isize));
    String::from_utf16_lossy(&buf[..len])
}

unsafe fn set_text(hwnd: HWND, text: &str) {
    let v = wide(text);
    SetWindowTextW(hwnd, PCWSTR::from_raw(v.as_ptr()));
}

pub fn run() {
    unsafe {
        // DPI 感知: 保证高 DPI 屏上字体与布局按物理像素渲染, 不模糊不折叠
        let _ = SetProcessDPIAware();
        let s = dpi_scale();

        // Segoe UI 9pt (与主控端一致)
        let mut face = [0u16; 32];
        for (i, c) in "Segoe UI".encode_utf16().enumerate() {
            face[i] = c;
        }
        let dpi = (96.0 * s) as i32;
        let lf = LOGFONTW {
            lfHeight: -((9 * dpi + 36) / 72),
            lfWeight: FW_NORMAL.0 as i32,
            lfCharSet: windows::Win32::Graphics::Gdi::DEFAULT_CHARSET,
            lfQuality: CLEARTYPE_QUALITY,
            lfPitchAndFamily: DEFAULT_PITCH.0 | FF_DONTCARE.0,
            lfOutPrecision: OUT_DEFAULT_PRECIS,
            lfClipPrecision: CLIP_DEFAULT_PRECIS,
            lfFaceName: face,
            ..Default::default()
        };
        let font = CreateFontIndirectW(&lf);
        let _ = FONT.set(font.0 as isize);

        let hinst = GetModuleHandleW(None).expect("module handle");
        let class_name = wide("RControlServerGui");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as isize as *mut _),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let title = wide("RControl 被控端设置");
        // 客户区目标 640x444 (按 DPI 缩放), 用 AdjustWindowRect 反推整窗尺寸避免内容被边框裁掉
        let style = WS_OVERLAPPED | WS_SYSMENU | WS_CAPTION | WS_MINIMIZEBOX;
        let mut rc = RECT { left: 0, top: 0, right: sc(640, s), bottom: sc(444, s) };
        if AdjustWindowRect(&mut rc, style, BOOL(0)).is_err() {
            rc.right += sc(16, s);
            rc.bottom += sc(40, s);
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rc.right - rc.left,
            rc.bottom - rc.top,
            None,
            None,
            HINSTANCE(hinst.0),
            None,
        )
        .expect("create window");

        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        let _ = SetTimer(hwnd, 1, 2000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn state_of(hwnd: HWND) -> &'static mut GuiState {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GuiState;
    &mut *p
}

/// 带扩展样式创建控件 (状态框的凹陷边框)
unsafe fn create_control_ex(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    ex: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: i32,
) -> HWND {
    let class_w = wide(class);
    let text_w = wide(text);
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE(ex),
        PCWSTR::from_raw(class_w.as_ptr()),
        PCWSTR::from_raw(text_w.as_ptr()),
        WINDOW_STYLE(style),
        x,
        y,
        w,
        h,
        parent,
        HMENU(id as isize as *mut _),
        HINSTANCE(hinst.0),
        None,
    )
    .expect("create control ex");
    if let Some(f) = FONT.get() {
        SendMessageW(hwnd, WM_SETFONT, WPARAM(*f as usize), LPARAM(1));
    }
    hwnd
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            // 列对齐规则 (基准 96dpi, 全部乘 DPI 系数):
            //   标签右缘 82/242/422, 输入框 x 88/248/428, 复选框与输入框同列
            let s = dpi_scale();

            // ---- 分组: 连接参数 ----
            create_control(hwnd, "BUTTON", "连接参数", STYLE_GROUP, sc(12, s), sc(10, s), sc(616, s), sc(84, s), 910);
            create_control(hwnd, "STATIC", "端口", STYLE_LBL_R, sc(24, s), sc(42, s), sc(58, s), sc(20, s), 900);
            let port_e = create_control(hwnd, "EDIT", "3333", STYLE_EDIT, sc(88, s), sc(38, s), sc(70, s), sc(24, s), ID_PORT);
            create_control(hwnd, "STATIC", "访问口令", STYLE_LBL_R, sc(172, s), sc(42, s), sc(70, s), sc(20, s), 902);
            let pwd = create_control(hwnd, "EDIT", "", STYLE_EDIT, sc(248, s), sc(38, s), sc(150, s), sc(24, s), ID_PWD);
            create_control(hwnd, "BUTTON", "复制口令", STYLE_BTN, sc(412, s), sc(36, s), sc(96, s), sc(28, s), ID_BTN_COPY);
            create_control(hwnd, "BUTTON", "保存配置", STYLE_BTN, sc(518, s), sc(36, s), sc(96, s), sc(28, s), ID_BTN_SAVE);

            // ---- 分组: 画面与权限 ----
            create_control(hwnd, "BUTTON", "画面与权限", STYLE_GROUP, sc(12, s), sc(102, s), sc(616, s), sc(112, s), 911);
            create_control(hwnd, "STATIC", "帧率", STYLE_LBL_R, sc(24, s), sc(134, s), sc(58, s), sc(20, s), 903);
            let fps = create_control(hwnd, "EDIT", "30", STYLE_EDIT, sc(88, s), sc(130, s), sc(60, s), sc(24, s), ID_FPS);
            create_control(hwnd, "STATIC", "画质", STYLE_LBL_R, sc(172, s), sc(134, s), sc(70, s), sc(20, s), 904);
            let quality = create_control(hwnd, "EDIT", "70", STYLE_EDIT, sc(248, s), sc(130, s), sc(60, s), sc(24, s), ID_QUALITY);
            create_control(hwnd, "STATIC", "最大宽度", STYLE_LBL_R, sc(352, s), sc(134, s), sc(70, s), sc(20, s), 905);
            let maxw = create_control(hwnd, "EDIT", "0", STYLE_EDIT, sc(428, s), sc(130, s), sc(70, s), sc(24, s), ID_MAXW);
            create_control(hwnd, "STATIC", "0=原始分辨率", STYLE_STATIC, sc(504, s), sc(134, s), sc(110, s), sc(20, s), 907);
            let chk_file =
                create_control(hwnd, "BUTTON", "允许文件传输", STYLE_CHK, sc(88, s), sc(162, s), sc(130, s), sc(22, s), ID_CHK_FILE);
            let chk_clip = create_control(
                hwnd,
                "BUTTON",
                "允许剪贴板同步",
                STYLE_CHK,
                sc(248, s),
                sc(162, s),
                sc(150, s),
                sc(22, s),
                ID_CHK_CLIP,
            );

            // ---- 运行控制按钮 ----
            create_control(hwnd, "BUTTON", "安装服务并启动", STYLE_BTN, sc(12, s), sc(226, s), sc(130, s), sc(30, s), ID_BTN_INSTALL);
            create_control(hwnd, "BUTTON", "停止服务", STYLE_BTN, sc(150, s), sc(226, s), sc(90, s), sc(30, s), ID_BTN_STOP_SVC);
            create_control(hwnd, "BUTTON", "卸载服务", STYLE_BTN, sc(248, s), sc(226, s), sc(90, s), sc(30, s), ID_BTN_UNINSTALL);
            create_control(
                hwnd,
                "BUTTON",
                "临时运行(当前用户)",
                STYLE_BTN,
                sc(346, s),
                sc(226, s),
                sc(140, s),
                sc(30, s),
                ID_BTN_TEMP_RUN,
            );
            create_control(hwnd, "BUTTON", "停止临时运行", STYLE_BTN, sc(494, s), sc(226, s), sc(116, s), sc(30, s), ID_BTN_TEMP_STOP);

            // ---- 状态区 ----
            create_control(hwnd, "STATIC", "运行状态 (每 2 秒刷新):", STYLE_STATIC, sc(12, s), sc(264, s), sc(300, s), sc(20, s), 906);
            let status = create_control_ex(
                hwnd,
                "STATIC",
                "",
                STYLE_STATIC | 0x1000, // SS_SUNKEN 凹陷效果
                WS_EX_STATICEDGE,
                sc(12, s),
                sc(286, s),
                sc(616, s),
                sc(148, s),
                999,
            );

            let state = Box::new(GuiState {
                main: hwnd,
                port: port_e,
                pwd,
                fps,
                quality,
                maxw,
                chk_file,
                chk_clip,
                status,
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            fill_from_config(hwnd);
            refresh_status(hwnd);
            LRESULT(0)
        }
        WM_TIMER => {
            refresh_status(hwnd);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wp.0 & 0xffff) as i32;
            if wp.0 >> 16 == 0 {
                match id {
                    ID_BTN_SAVE => on_save(hwnd),
                    ID_BTN_INSTALL => on_install(hwnd),
                    ID_BTN_STOP_SVC => on_stop_service(hwnd),
                    ID_BTN_UNINSTALL => on_uninstall(hwnd),
                    ID_BTN_TEMP_RUN => on_temp_run(hwnd),
                    ID_BTN_TEMP_STOP => on_temp_stop(hwnd),
                    ID_BTN_COPY => on_copy_pwd(hwnd),
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GuiState;
            if !p.is_null() {
                drop(Box::from_raw(p));
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn read_config_from_ui(hwnd: HWND) -> Result<ServerConfig, String> {
    let g = state_of(hwnd);
    let port: u16 = get_text(g.port).trim().parse().map_err(|_| "端口无效 (1-65535)")?;
    if port == 0 {
        return Err("端口无效".into());
    }
    let pwd = get_text(g.pwd);
    if pwd.chars().count() < 4 {
        return Err("口令至少 4 个字符".into());
    }
    let fps: u32 = get_text(g.fps).trim().parse().unwrap_or(30).clamp(1, 60);
    let quality: u32 = get_text(g.quality).trim().parse().unwrap_or(70).clamp(10, 95);
    // 0 = 原始分辨率 (1:1 显示); 非零值限 640..3840; 输入无效时保留原值
    let maxw: u32 = match get_text(g.maxw).trim().parse::<u32>() {
        Ok(0) => 0,
        Ok(v) => v.clamp(640, 3840),
        Err(_) => load_server_config().max_width,
    };
    let allow_file = SendMessageW(g.chk_file, BM_GETCHECK as u32, WPARAM(0), LPARAM(0)).0 != 0;
    let allow_clipboard = SendMessageW(g.chk_clip, BM_GETCHECK as u32, WPARAM(0), LPARAM(0)).0 != 0;
    let mut old = load_server_config();
    old.port = port;
    old.password = pwd;
    old.max_fps = fps;
    old.quality = quality;
    old.max_width = maxw;
    old.allow_file = allow_file;
    old.allow_clipboard = allow_clipboard;
    Ok(old)
}

unsafe fn fill_from_config(hwnd: HWND) {
    let g = state_of(hwnd);
    let cfg = load_server_config();
    set_text(g.port, &cfg.port.to_string());
    set_text(g.pwd, &cfg.password);
    set_text(g.fps, &cfg.max_fps.to_string());
    set_text(g.quality, &cfg.quality.to_string());
    set_text(g.maxw, &cfg.max_width.to_string());
    SendMessageW(g.chk_file, BM_SETCHECK as u32, WPARAM(cfg.allow_file as usize), LPARAM(0));
    SendMessageW(g.chk_clip, BM_SETCHECK as u32, WPARAM(cfg.allow_clipboard as usize), LPARAM(0));
}

unsafe fn msgbox(text: &str, caption: &str, err: bool) {
    let t = wide(text);
    let c = wide(caption);
    let flags = if err { MB_OK | MB_ICONERROR } else { MB_OK | MB_ICONINFORMATION };
    MessageBoxW(None, PCWSTR::from_raw(t.as_ptr()), PCWSTR::from_raw(c.as_ptr()), flags);
}

unsafe fn on_save(hwnd: HWND) {
    let r = read_config_from_ui(hwnd).and_then(|c| save_server_config(&c).map_err(|e| e.to_string()));
    match r {
        Ok(()) => msgbox("配置已保存。\n端口/口令变化会使工作进程自动重启生效。", "RControl", false),
        Err(e) => msgbox(&e, "保存失败", true),
    }
    refresh_status(hwnd);
}

unsafe fn on_install(hwnd: HWND) {
    let cfg = match read_config_from_ui(hwnd) {
        Ok(c) => c,
        Err(e) => {
            msgbox(&e, "配置无效", true);
            return;
        }
    };
    if let Err(e) = save_server_config(&cfg) {
        msgbox(&e.to_string(), "保存配置失败", true);
        return;
    }
    let _ = sm::remove_firewall_rule();
    if let Err(e) = sm::add_firewall_rule(cfg.port) {
        msgbox(&format!("添加防火墙规则失败: {}\n请手动放行 TCP {}", e, cfg.port), "警告", true);
    }
    match sm::install_service().and_then(|_| sm::start_service()) {
        Ok(()) => msgbox(
            "服务已安装并启动 (开机自启)。\n现已支持锁屏/登录界面的远程控制。",
            "RControl",
            false,
        ),
        Err(e) => msgbox(&e, "安装服务失败", true),
    }
    refresh_status(hwnd);
}

unsafe fn on_stop_service(hwnd: HWND) {
    if let Err(e) = sm::stop_service() {
        msgbox(&e, "停止服务失败", true);
    }
    refresh_status(hwnd);
}

unsafe fn on_uninstall(hwnd: HWND) {
    let _ = sm::remove_firewall_rule();
    match sm::uninstall_service() {
        Ok(()) => msgbox("服务已卸载。", "RControl", false),
        Err(e) => msgbox(&e, "卸载服务失败", true),
    }
    refresh_status(hwnd);
}

unsafe fn on_temp_run(hwnd: HWND) {
    let cfg = match read_config_from_ui(hwnd) {
        Ok(c) => c,
        Err(e) => {
            msgbox(&e, "配置无效", true);
            return;
        }
    };
    if let Err(e) = save_server_config(&cfg) {
        msgbox(&e.to_string(), "保存配置失败", true);
        return;
    }
    match sm::spawn_user_worker() {
        Ok(pid) => msgbox(
            &format!("已临时启动 (PID {})。\n注意: 临时模式不支持锁屏/登录界面控制与 Ctrl+Alt+Del 发送。", pid),
            "RControl",
            false,
        ),
        Err(e) => msgbox(&e, "启动失败", true),
    }
    refresh_status(hwnd);
}

unsafe fn on_temp_stop(hwnd: HWND) {
    let killed = kill_user_workers();
    if killed > 0 {
        msgbox(&format!("已停止 {} 个临时进程", killed), "RControl", false);
    } else {
        msgbox("没有找到临时运行的进程", "RControl", false);
    }
    refresh_status(hwnd);
}

fn kill_user_workers() -> usize {
    use std::os::windows::process::CommandExt;
    let path = server_data_dir().join("status.json");
    let mut killed = 0;
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if v.get("user_mode").and_then(|b| b.as_bool()).unwrap_or(false) {
                if let Some(pid) = v.get("pid").and_then(|p| p.as_u64()) {
                    let out = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/F"])
                        .creation_flags(0x0800_0000)
                        .output();
                    if let Ok(o) = out {
                        if o.status.success() {
                            killed += 1;
                        }
                    }
                }
            }
        }
    }
    killed
}

unsafe fn on_copy_pwd(hwnd: HWND) {
    let g = state_of(hwnd);
    let pwd = get_text(g.pwd);
    if !pwd.is_empty() && crate::clip::set_clipboard_text(&pwd) {
        msgbox("口令已复制到剪贴板", "RControl", false);
    }
}

#[derive(serde::Deserialize)]
struct WorkerStatus {
    ts: u64,
    pid: u32,
    session: u32,
    system_priv: bool,
    user_mode: bool,
    port: u16,
    connected: bool,
    on_secure: bool,
    hostname: String,
    #[serde(default)]
    last_error: String,
}

fn read_worker_status() -> Option<WorkerStatus> {
    let path = server_data_dir().join("status.json");
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
}

unsafe fn refresh_status(hwnd: HWND) {
    let g = state_of(hwnd);
    let svc = match sm::query_service() {
        sm::ServiceStateInfo::NotInstalled => "未安装",
        sm::ServiceStateInfo::Stopped => "已停止",
        sm::ServiceStateInfo::StartPending => "启动中",
        sm::ServiceStateInfo::Running => "运行中",
        sm::ServiceStateInfo::Other => "其他状态",
    };
    let mut text = format!("服务状态: {}\r\n", svc);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    match read_worker_status() {
        Some(st) if now.saturating_sub(st.ts) < 5 => {
            text += &format!(
                "工作进程: 运行中 (PID {}, 会话 {}, {}模式, {})\r\n监听端口: {}    当前连接: {}\r\n锁屏/登录界面控制: {}\r\n",
                st.pid,
                st.session,
                if st.user_mode { "用户临时" } else { "服务" },
                if st.system_priv { "SYSTEM 权限" } else { "普通权限" },
                st.port,
                if st.connected { "已连接" } else { "无" },
                if st.system_priv { "支持" } else { "不支持 (需安装服务)" },
            );
            if !st.last_error.is_empty() {
                text += &format!("最近错误: {}\r\n", st.last_error);
            }
        }
        _ => {
            text += "工作进程: 未运行\r\n";
        }
    }
    text += "\r\n使用说明:\r\n1. 保存配置后点击 [安装服务并启动], 开机自启并支持锁屏/登录界面远程控制。\r\n2. 主控端 (rcontrol_client.exe) 连接本机 IP + 端口, 输入访问口令。\r\n3. 主控端按 Ctrl+Shift+Del 可向被控端发送 Ctrl+Alt+Del。";
    set_text(g.status, &text);
}
