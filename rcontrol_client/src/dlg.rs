//! 主控端: 被控端清单管理 + 连接窗口 (原生 Win32) + 弹窗辅助
//!
//! 清单持久化在主控端 exe 同目录 hosts.txt, 每行: 名称\t地址\t口令\t只读(0/1)

use rcontrol_common::config::{load_client_config, save_client_config};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontIndirectW, DEFAULT_CHARSET,
    DEFAULT_PITCH, FF_DONTCARE, FW_NORMAL, GetDC, GetDeviceCaps, HBRUSH, LOGPIXELSY, LOGFONTW,
    OUT_DEFAULT_PRECIS, ReleaseDC, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, INITCOMMONCONTROLSEX, INITCOMMONCONTROLSEX_ICC, ICC_LISTVIEW_CLASSES,
    LVCOLUMNW, LVCOLUMNW_MASK, LVCF_FMT, LVCF_TEXT, LVCF_WIDTH, LVCFMT_LEFT, LVIF_TEXT,
    LIST_VIEW_ITEM_FLAGS, LIST_VIEW_ITEM_STATE_FLAGS, LVM_INSERTCOLUMNW, LVM_INSERTITEMW, LVM_SETITEMW,
    LVITEMW, LVNI_SELECTED, LVM_DELETEALLITEMS, LVM_GETNEXTITEM, NM_CLICK, NM_DBLCLK, NMHDR,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const WS_CHILD_V: u32 = 0x4000_0000 | 0x1000_0000;
const STYLE_EDIT_PLAIN: u32 = WS_CHILD_V | 0x0080_0000 | 0x0080;
const STYLE_EDIT_PWD: u32 = WS_CHILD_V | 0x0080_0000 | 0x0080 | 0x0020_0000; // ES_PASSWORD
const STYLE_BTN: u32 = WS_CHILD_V | 0x0001_0000;
const STYLE_CHK: u32 = WS_CHILD_V | 0x0003;
const STYLE_STATIC: u32 = WS_CHILD_V;
const STYLE_GROUP: u32 = WS_CHILD_V | 0x0007; // BS_GROUPBOX
const STYLE_LBL_R: u32 = WS_CHILD_V | 0x0002; // SS_RIGHT (与输入框成列)
const STYLE_LV: u32 = WS_CHILD_V | 0x0001_0000 | 0x0001 /*LVS_REPORT*/ | 0x0008 /*LVS_SHOWSELALWAYS*/ | 0x0800 /*LVS_NOSORTHEADER*/;
const LVS_EX_FULLROWSELECT: u32 = 0x20;
const LVS_EX_DOUBLEBUFFER: u32 = 0x1_0000;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1000 + 54;
const LVM_SETITEMSTATE: u32 = 0x1000 + 43;
const LVIF_STATE: u32 = 8;
const LVIS_FOCUSED: u32 = 1;
const LVIS_SELECTED: u32 = 2;
const WS_EX_STATICEDGE: u32 = 0x0002_0000;
const SS_SUNKEN: u32 = 0x1000;

// 控件 ID
const ID_LV: i32 = 110;
const ID_NAME: i32 = 111;
const ID_ADDR: i32 = 112;
const ID_PWD: i32 = 113;
const ID_CHK_VIEW: i32 = 114;
const ID_BTN_CONN: i32 = 1;
const ID_BTN_ADD: i32 = 2;
const ID_BTN_UPD: i32 = 3;
const ID_BTN_DEL: i32 = 4;
const ID_BTN_QUIT: i32 = 5;

/// 全局字体 (Segoe UI 9pt, 一次创建)
static FONT: std::sync::OnceLock<isize> = std::sync::OnceLock::new();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

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

pub unsafe fn create_control(
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

pub unsafe fn get_text(hwnd: HWND) -> String {
    let len = SendMessageW(hwnd, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0 as usize;
    if len == 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len + 1];
    SendMessageW(hwnd, WM_GETTEXT, WPARAM(len + 1), LPARAM(buf.as_mut_ptr() as isize));
    String::from_utf16_lossy(&buf[..len])
}

pub unsafe fn set_text(hwnd: HWND, text: &str) {
    let v = wide(text);
    SetWindowTextW(hwnd, PCWSTR::from_raw(v.as_ptr()));
}

pub fn alert(text: &str) {
    unsafe {
        let t = wide(text);
        let c = wide("RControl");
        MessageBoxW(None, PCWSTR::from_raw(t.as_ptr()), PCWSTR::from_raw(c.as_ptr()), MB_OK | MB_ICONWARNING);
    }
}

// ---------------- 被控端清单持久化 ----------------

#[derive(Clone)]
struct Host {
    name: String,
    addr: String,
    pwd: String,
    view_only: bool,
}

/// 清单文件 = 主控端 exe 同目录 hosts.txt
fn hosts_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("hosts.txt")
}

fn clean_field(s: &str) -> String {
    s.replace(['\t', '\r', '\n'], " ")
}

fn load_hosts() -> Vec<Host> {
    let mut out = Vec::new();
    if let Ok(text) = std::fs::read_to_string(hosts_path()) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() >= 2 && !f[1].trim().is_empty() {
                out.push(Host {
                    name: f[0].trim().to_string(),
                    addr: f[1].trim().to_string(),
                    pwd: if f.len() > 2 { f[2].to_string() } else { String::new() },
                    view_only: f.len() > 3 && f[3].trim() == "1",
                });
            }
        }
    }
    // 首次使用: 从旧版配置的"上次地址"预填一条, 不让清单空着
    if out.is_empty() {
        let cfg = load_client_config();
        if !cfg.last_addr.is_empty() {
            out.push(Host {
                name: "上次连接".into(),
                addr: cfg.last_addr.clone(),
                pwd: String::new(),
                view_only: false,
            });
        }
    }
    out
}

fn save_hosts(hosts: &[Host]) -> bool {
    let mut text = String::from("# RControl 被控端清单 (名称/地址/口令/只读)\r\n");
    for h in hosts {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\r\n",
            clean_field(&h.name),
            clean_field(&h.addr),
            clean_field(&h.pwd),
            if h.view_only { 1 } else { 0 }
        ));
    }
    std::fs::write(hosts_path(), text).is_ok()
}

// ---------------- 连接结果 ----------------

pub struct ConnectInfo {
    pub addr: String,
    pub password: String,
    pub view_only: bool,
}

/// 对话框结果 (静态槽, 避免 SetWindowLongPtr hack)
static RESULT: std::sync::Mutex<Option<ConnectInfo>> = std::sync::Mutex::new(None);

struct DlgState {
    list: HWND,
    name: HWND,
    addr: HWND,
    pwd: HWND,
    chk_view: HWND,
    status: HWND,
    hosts: Vec<Host>,
}

unsafe fn state_of(hwnd: HWND) -> &'static mut DlgState {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DlgState;
    &mut *p
}

unsafe fn lv_add_col(lv: HWND, i: i32, title: &str, w: i32) {
    let mut t = wide(title);
    let col = LVCOLUMNW {
        mask: LVCOLUMNW_MASK(LVCF_FMT.0 | LVCF_WIDTH.0 | LVCF_TEXT.0),
        fmt: LVCFMT_LEFT,
        cx: w,
        pszText: PWSTR(t.as_mut_ptr()),
        ..Default::default()
    };
    SendMessageW(lv, LVM_INSERTCOLUMNW, WPARAM(i as usize), LPARAM(&col as *const _ as isize));
}

unsafe fn lv_insert(lv: HWND, i: i32, name: &str) {
    let mut w = wide(name);
    let it = LVITEMW {
        mask: LIST_VIEW_ITEM_FLAGS(LVIF_TEXT.0),
        iItem: i,
        pszText: PWSTR(w.as_mut_ptr()),
        ..Default::default()
    };
    SendMessageW(lv, LVM_INSERTITEMW, WPARAM(0), LPARAM(&it as *const _ as isize));
}

unsafe fn lv_set_text(lv: HWND, i: i32, sub: i32, text: &str) {
    let mut w = wide(text);
    let it = LVITEMW {
        mask: LIST_VIEW_ITEM_FLAGS(LVIF_TEXT.0),
        iItem: i,
        iSubItem: sub,
        pszText: PWSTR(w.as_mut_ptr()),
        ..Default::default()
    };
    SendMessageW(lv, LVM_SETITEMW, WPARAM(0), LPARAM(&it as *const _ as isize));
}

unsafe fn lv_selected(lv: HWND) -> i32 {
    SendMessageW(lv, LVM_GETNEXTITEM, WPARAM(u32::MAX as usize), LPARAM(LVNI_SELECTED as isize)).0 as i32
}

/// 重建清单视图 (保留原选中行: 更新/删除提示依赖选中状态)
unsafe fn fill_list(hwnd: HWND) {
    let g = state_of(hwnd);
    let sel = lv_selected(g.list);
    SendMessageW(g.list, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
    for (i, h) in g.hosts.iter().enumerate() {
        lv_insert(g.list, i as i32, &h.name);
        lv_set_text(g.list, i as i32, 1, &h.addr);
        lv_set_text(g.list, i as i32, 2, if h.view_only { "是" } else { "—" });
    }
    if sel >= 0 && (sel as usize) < g.hosts.len() {
        let it = LVITEMW {
            mask: LIST_VIEW_ITEM_FLAGS(LVIF_STATE),
            iItem: sel,
            state: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED | LVIS_FOCUSED),
            stateMask: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED | LVIS_FOCUSED),
            ..Default::default()
        };
        SendMessageW(g.list, LVM_SETITEMSTATE, WPARAM(sel as usize), LPARAM(&it as *const _ as isize));
    }
}

unsafe fn set_status(hwnd: HWND, text: &str) {
    let g = state_of(hwnd);
    set_text(g.status, text);
}

/// 选中行 -> 填充表单
unsafe fn sel_to_form(hwnd: HWND) -> bool {
    let g = state_of(hwnd);
    let i = lv_selected(g.list);
    if i < 0 || i as usize >= g.hosts.len() {
        return false;
    }
    let h = g.hosts[i as usize].clone();
    set_text(g.name, &h.name);
    set_text(g.addr, &h.addr);
    set_text(g.pwd, &h.pwd);
    SendMessageW(g.chk_view, BM_SETCHECK as u32, WPARAM(h.view_only as usize), LPARAM(0));
    true
}

/// 从表单读一个 Host (不校验)
unsafe fn form_to_host(hwnd: HWND) -> Host {
    let g = state_of(hwnd);
    Host {
        name: get_text(g.name).trim().to_string(),
        addr: get_text(g.addr).trim().to_string(),
        pwd: get_text(g.pwd),
        view_only: SendMessageW(g.chk_view, BM_GETCHECK as u32, WPARAM(0), LPARAM(0)).0 != 0,
    }
}

fn normalize_addr(addr: &str) -> String {
    if addr.contains(':') {
        addr.to_string()
    } else {
        format!("{}:3333", addr)
    }
}

/// 校验表单: 出错时返回提示
fn validate(h: &Host) -> Result<(), String> {
    if h.name.is_empty() {
        return Err("请填写名称 (用于在清单中区分被控端)".into());
    }
    if h.addr.is_empty() {
        return Err("请填写被控端地址 (IP:端口)".into());
    }
    if h.addr.starts_with(':') || h.addr.ends_with(':') {
        return Err("地址格式无效, 应为 IP:端口".into());
    }
    Ok(())
}

unsafe fn on_connect(hwnd: HWND) {
    let h = form_to_host(hwnd);
    if h.addr.is_empty() || h.pwd.is_empty() {
        set_status(hwnd, "连接需要地址与口令 (在下方表单或清单中双击)");
        return;
    }
    let addr_fmt = normalize_addr(&h.addr);
    let mut cfg = load_client_config();
    cfg.last_addr = addr_fmt.clone();
    save_client_config(&cfg);
    *RESULT.lock().unwrap() = Some(ConnectInfo { addr: addr_fmt, password: h.pwd, view_only: h.view_only });
    let _ = DestroyWindow(hwnd);
}

unsafe fn on_add(hwnd: HWND) {
    let h = form_to_host(hwnd);
    if let Err(e) = validate(&h) {
        set_status(hwnd, &e);
        return;
    }
    let g = state_of(hwnd);
    let n = g.hosts.len();
    g.hosts.push(Host { addr: normalize_addr(&h.addr), ..h });
    if save_hosts(&g.hosts) {
        fill_list(hwnd);
        set_status(hwnd, &format!("已新增并保存 (共 {} 台) -> {}", n + 1, hosts_path().display()));
    } else {
        set_status(hwnd, &format!("保存失败: {} (请检查软件目录写权限)", hosts_path().display()));
    }
}

unsafe fn on_update(hwnd: HWND) {
    let g = state_of(hwnd);
    let i = lv_selected(g.list);
    if i < 0 || i as usize >= g.hosts.len() {
        set_status(hwnd, "先在清单中选中要修改的一条");
        return;
    }
    let h = form_to_host(hwnd);
    if let Err(e) = validate(&h) {
        set_status(hwnd, &e);
        return;
    }
    g.hosts[i as usize] = Host { addr: normalize_addr(&h.addr), ..h };
    if save_hosts(&g.hosts) {
        fill_list(hwnd);
        set_status(hwnd, &format!("已更新并保存 (共 {} 台)", g.hosts.len()));
    } else {
        set_status(hwnd, "保存失败 (请检查软件目录写权限)");
    }
}

unsafe fn on_delete(hwnd: HWND) {
    let g = state_of(hwnd);
    let i = lv_selected(g.list);
    if i < 0 || i as usize >= g.hosts.len() {
        set_status(hwnd, "先在清单中选中要删除的一条");
        return;
    }
    let name = g.hosts[i as usize].name.clone();
    let t = wide(&format!("从清单中删除 \"{}\" 吗?", name));
    let c = wide("确认");
    if MessageBoxW(hwnd, PCWSTR::from_raw(t.as_ptr()), PCWSTR::from_raw(c.as_ptr()), MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
    g.hosts.remove(i as usize);
    if save_hosts(&g.hosts) {
        fill_list(hwnd);
        set_status(hwnd, &format!("已删除并保存 (剩 {} 台)", g.hosts.len()));
    } else {
        set_status(hwnd, "保存失败 (请检查软件目录写权限)");
    }
}

/// 运行主控端主窗口 (被控端清单 + 连接表单): 点"连接"返回 Some, 退出/关闭返回 None
pub fn run() -> Option<ConnectInfo> {
    *RESULT.lock().unwrap() = None;
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: INITCOMMONCONTROLSEX_ICC(ICC_LISTVIEW_CLASSES.0),
        };
        let _ = InitCommonControlsEx(&icc);

        let s = dpi_scale();
        // Segoe UI 9pt (与文件传输窗口一致)
        let mut face = [0u16; 32];
        for (i, c) in "Segoe UI".encode_utf16().enumerate() {
            face[i] = c;
        }
        let dpi = (96.0 * s) as i32;
        let lf = LOGFONTW {
            lfHeight: -((9 * dpi + 36) / 72),
            lfWeight: FW_NORMAL.0 as i32,
            lfCharSet: DEFAULT_CHARSET,
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
        let class_name = wide("RControlClientDlg");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(dlg_proc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as isize as *mut _),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let title = wide("RControl 主控端 - 被控端清单");
        // 客户区目标 620x418 (按 DPI 缩放)
        let style = WS_OVERLAPPED | WS_SYSMENU | WS_CAPTION | WS_MINIMIZEBOX;
        let mut rc = RECT { left: 0, top: 0, right: sc(620, s), bottom: sc(418, s) };
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
        .expect("create dialog");
        let _ = hwnd;

        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // 回车 = 连接, Esc = 退出
            if msg.message == WM_KEYDOWN && msg.wParam.0 == 0x0d {
                let _ = PostMessageW(hwnd, WM_COMMAND, WPARAM(ID_BTN_CONN as usize), LPARAM(0));
                continue;
            }
            if msg.message == WM_KEYDOWN && msg.wParam.0 == 0x1b {
                let _ = PostMessageW(hwnd, WM_COMMAND, WPARAM(ID_BTN_QUIT as usize), LPARAM(0));
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    RESULT.lock().unwrap().take()
}

unsafe extern "system" fn dlg_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            // 列对齐规则: 标签右缘 76/280, 输入框 x 82/286
            let s = dpi_scale();

            // ---- 清单 ----
            create_control(hwnd, "STATIC", "被控端清单 (双击一台直接连接):", STYLE_STATIC, sc(16, s), sc(12, s), sc(400, s), sc(20, s), 900);
            let list = create_control(hwnd, "SysListView32", "", STYLE_LV, sc(16, s), sc(36, s), sc(588, s), sc(168, s), ID_LV);
            lv_add_col(list, 0, "名称", sc(190, s));
            lv_add_col(list, 1, "地址", sc(260, s));
            lv_add_col(list, 2, "仅查看", sc(90, s));
            SendMessageW(
                list,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                WPARAM(0),
                LPARAM((LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize),
            );

            // ---- 连接参数表单 (标签右对齐成列) ----
            create_control(hwnd, "BUTTON", "连接参数", STYLE_GROUP, sc(12, s), sc(214, s), sc(616, s), sc(92, s), 910);
            create_control(hwnd, "STATIC", "名称", STYLE_LBL_R, sc(24, s), sc(242, s), sc(48, s), sc(20, s), 901);
            let name = create_control(hwnd, "EDIT", "", STYLE_EDIT_PLAIN, sc(82, s), sc(238, s), sc(150, s), sc(24, s), ID_NAME);
            create_control(hwnd, "STATIC", "地址", STYLE_LBL_R, sc(240, s), sc(242, s), sc(40, s), sc(20, s), 902);
            let addr = create_control(hwnd, "EDIT", "", STYLE_EDIT_PLAIN, sc(286, s), sc(238, s), sc(200, s), sc(24, s), ID_ADDR);
            let chk_view = create_control(hwnd, "BUTTON", "仅查看 (不发键鼠)", STYLE_CHK, sc(500, s), sc(240, s), sc(130, s), sc(22, s), ID_CHK_VIEW);

            create_control(hwnd, "STATIC", "口令", STYLE_LBL_R, sc(24, s), sc(274, s), sc(48, s), sc(20, s), 903);
            let pwd = create_control(hwnd, "EDIT", "", STYLE_EDIT_PWD, sc(82, s), sc(270, s), sc(150, s), sc(24, s), ID_PWD);
            create_control(hwnd, "STATIC", "地址格式 IP:端口, 不填端口默认 3333", STYLE_STATIC, sc(286, s), sc(274, s), sc(320, s), sc(20, s), 904);

            // ---- 按钮 ----
            create_control(hwnd, "BUTTON", "连接", STYLE_BTN, sc(12, s), sc(318, s), sc(110, s), sc(32, s), ID_BTN_CONN);
            create_control(hwnd, "BUTTON", "新增到清单", STYLE_BTN, sc(130, s), sc(318, s), sc(100, s), sc(32, s), ID_BTN_ADD);
            create_control(hwnd, "BUTTON", "更新选中", STYLE_BTN, sc(238, s), sc(318, s), sc(100, s), sc(32, s), ID_BTN_UPD);
            create_control(hwnd, "BUTTON", "删除选中", STYLE_BTN, sc(346, s), sc(318, s), sc(100, s), sc(32, s), ID_BTN_DEL);
            create_control(hwnd, "BUTTON", "退出", STYLE_BTN, sc(524, s), sc(318, s), sc(80, s), sc(32, s), ID_BTN_QUIT);

            // ---- 状态 ----
            let status = create_control_ex(
                hwnd,
                "STATIC",
                "",
                STYLE_STATIC | SS_SUNKEN,
                WS_EX_STATICEDGE,
                sc(12, s),
                sc(358, s),
                sc(616, s),
                sc(22, s),
                905,
            );
            create_control(hwnd, "STATIC", "清单保存在主控端软件目录 hosts.txt", STYLE_STATIC, sc(12, s), sc(388, s), sc(500, s), sc(20, s), 906);

            let hosts = load_hosts();
            let state = Box::new(DlgState { list, name, addr, pwd, chk_view, status, hosts });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            fill_list(hwnd);
            if !state_of(hwnd).hosts.is_empty() {
                set_status(hwnd, &format!("共 {} 台被控端, 双击清单直接连接", state_of(hwnd).hosts.len()));
            }
            LRESULT(0)
        }
        WM_NOTIFY => {
            let nm = &*(lp.0 as *const NMHDR);
            if nm.idFrom == ID_LV as usize && (nm.code == NM_CLICK || nm.code == NM_DBLCLK) {
                sel_to_form(hwnd);
                if nm.code == NM_DBLCLK {
                    on_connect(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wp.0 & 0xffff) as i32;
            if wp.0 >> 16 == 0 {
                match id {
                    ID_BTN_CONN => on_connect(hwnd),
                    ID_BTN_ADD => on_add(hwnd),
                    ID_BTN_UPD => on_update(hwnd),
                    ID_BTN_DEL => on_delete(hwnd),
                    ID_BTN_QUIT => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DlgState;
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

/// 带扩展样式创建控件 (状态栏凹陷边框)
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
