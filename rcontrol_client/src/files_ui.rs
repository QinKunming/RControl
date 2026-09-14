//! 文件传输窗口: 双栏 (左=远程被控端, 右=本地主控端), ListView 报表视图
//! 传输在 worker 线程执行, UI 通过状态字符串 + 列表邮箱刷新
//!
//! 修复史: 旧版用 LISTBOX 且样式误写 (缺 LBS_NOTIFY=0x1, 放进去的是
//! WS_TABSTOP|WS_GROUP), 双击通知从不发往父窗口 → 无法进入目录 →
//! 远程栏停在驱动器列表 (无文件可选), 上传目标拼成相对路径 → 全部功能失效。
//! 现改用 SysListView32 (LVS_REPORT), 通知走 WM_NOTIFY。

use crate::conn::Client;
use rcontrol_common::proto::*;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, GetDC, GetDeviceCaps, ReleaseDC, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY,
    DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_NORMAL, HFONT, LOGFONTW, LOGPIXELSY,
    OUT_DEFAULT_PRECIS,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::Shell::{
    SHGetFileInfoW, SHFILEINFOW, SHGFI_FLAGS, SHGFI_SMALLICON, SHGFI_SYSICONINDEX, SHGFI_TYPENAME,
    SHGFI_USEFILEATTRIBUTES,
};
use windows::Win32::UI::WindowsAndMessaging::*;

// ---- 控件 ID ----
// 远程栏 1000 段 / 本地栏 2000 段 / 中间按钮 3000 段
const ID_ED_R: i32 = 1001;
const ID_UP_R: i32 = 1002;
const ID_GO_R: i32 = 1003;
const ID_REF_R: i32 = 1004;
const ID_LV_R: i32 = 1005;
const ID_MK_R: i32 = 1006;
const ID_RM_R: i32 = 1007;
const ID_RN_R: i32 = 1008;
const ID_ED_L: i32 = 2001;
const ID_UP_L: i32 = 2002;
const ID_GO_L: i32 = 2003;
const ID_REF_L: i32 = 2004;
const ID_LV_L: i32 = 2005;
const ID_MK_L: i32 = 2006;
const ID_RM_L: i32 = 2007;
const ID_RN_L: i32 = 2008;
const ID_DL: i32 = 3000; // → 下载到本地 (远程栏选中 → 本地栏当前目录)
const ID_UL: i32 = 3001; // ← 上传到远程

const WS_CHILD_V: u32 = 0x4000_0000 | 0x1000_0000;
const STYLE_EDIT_PLAIN: u32 = WS_CHILD_V | 0x0080_0000 | 0x0080; // WS_BORDER|ES_AUTOHSCROLL
const STYLE_BTN: u32 = WS_CHILD_V | 0x0001_0000; // WS_TABSTOP
const STYLE_STATIC: u32 = WS_CHILD_V;
const STYLE_LV: u32 = WS_CHILD_V | 0x0001_0000 | LVS_REPORT | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER;

const COL_NAME: usize = 0;
const COL_SIZE: usize = 1;
const COL_TIME: usize = 2;
const COL_TYPE: usize = 3;

/// 一条目录项 (结构化, 不再从显示文本反解析)
#[derive(Clone)]
struct Entr {
    name: String,
    is_dir: bool,
    size: u64,
    mtime: u64,
    readonly: bool,
}

impl Entr {
    fn dotdot() -> Entr {
        Entr { name: "..".into(), is_dir: true, size: 0, mtime: 0, readonly: false }
    }
    fn from_file(e: &FileEntry) -> Entr {
        Entr { name: e.name.clone(), is_dir: e.is_dir, size: e.size, mtime: e.mtime, readonly: e.is_readonly }
    }
}

fn sort_entries(v: &mut Vec<Entr>) {
    v.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
}

/// UI 线程 -> worker 的异步任务
enum Task {
    ListRemote(String),
    ListLocal(String),
    Download { remote: String, local: String },
    Upload { local: String, remote: String },
    Mkdir { remote: bool, path: String },
    Remove { remote: bool, path: String },
    Rename { remote: bool, old: String, new: String },
}

/// worker -> UI 的结果
#[derive(Default)]
struct UiUpdate {
    status: Option<String>,
    /// (是否远程, 路径, 条目)
    listing: Option<(bool, String, Vec<Entr>)>,
    error: Option<String>,
    /// 完成的传输任务数 (用于解除按钮禁用)
    transfers_done: u32,
}

struct Pane {
    path: String,
    entries: Vec<Entr>,
    lv: HWND,
    ed: HWND,
}

struct FmState {
    panes: [Pane; 2], // [0]=远程 [1]=本地
    btn_dl: HWND,
    btn_ul: HWND,
    status: HWND,
    /// 排队中的传输任务数 (worker 串行处理)
    pending: u32,
    dpi: i32,
    tx: Sender<Task>,
    rx: Receiver<UiUpdate>,
}

static OPEN_ONCE: Mutex<bool> = Mutex::new(false);
/// 窗口字体 (Segoe UI, 按 DPI 缩放)
static FONT: OnceLock<isize> = OnceLock::new();

/// 打开文件管理器 (已打开则忽略)
pub fn open(client: Arc<Client>) {
    {
        let mut g = OPEN_ONCE.lock().unwrap();
        if *g {
            return;
        }
        *g = true;
    }
    std::thread::spawn(move || {
        run_window(client);
        *OPEN_ONCE.lock().unwrap() = false;
    });
}

// ---------------- 工具 ----------------

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

fn fmt_size(n: u64) -> String {
    if n >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else if n >= 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{} B", n)
    }
}

/// unix 秒 -> 本地时区 "yyyy-MM-dd HH:mm" (SystemTimeToTzSpecificLocalTime 处理夏令时)
fn fmt_time(unix: u64) -> String {
    if unix == 0 {
        return String::new();
    }
    unsafe {
        let v = (unix as u64).saturating_add(11_644_473_600).saturating_mul(10_000_000);
        let ft = windows::Win32::Foundation::FILETIME {
            dwLowDateTime: v as u32,
            dwHighDateTime: (v >> 32) as u32,
        };
        let mut utc = windows::Win32::Foundation::SYSTEMTIME::default();
        if FileTimeToSystemTime(&ft, &mut utc).is_err() {
            return String::new();
        }
        let mut loc = windows::Win32::Foundation::SYSTEMTIME::default();
        if SystemTimeToTzSpecificLocalTime(None, &utc, &mut loc).is_err() {
            loc = utc; // 极端情况退回 UTC
        }
        format!("{:04}-{:02}-{:02} {:02}:{:02}", loc.wYear, loc.wMonth, loc.wDay, loc.wHour, loc.wMinute)
    }
}

fn join_path(base: &str, name: &str) -> String {
    if base.is_empty() {
        return name.to_string(); // 驱动器列表: name 是 "C:\"
    }
    let t = base.trim_end_matches('\\');
    format!("{}\\{}", t, name)
}

/// 盘根 -> "" (驱动器列表); "" -> None
fn parent_path(p: &str) -> Option<String> {
    if p.is_empty() {
        return None;
    }
    if p.len() >= 2 && p.ends_with('\\') && p.as_bytes()[1] == b':' {
        return Some(String::new());
    }
    let t = p.trim_end_matches('\\');
    if t.len() <= 3 && t.as_bytes().get(1) == Some(&b':') {
        return Some(format!("{}\\", t[..2].to_string()));
    }
    match t.rfind('\\') {
        Some(i) => Some(t[..i].to_string()),
        None => Some(String::new()),
    }
}

fn default_local_dir() -> String {
    std::env::var_os("USERPROFILE")
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| "C:\\".into())
}

fn path_label(is_remote: bool, path: &str) -> String {
    if is_remote {
        if path.is_empty() {
            "[远程] 驱动器".into()
        } else {
            format!("[远程] {}", path)
        }
    } else if path.is_empty() {
        "[本地] 驱动器".into()
    } else {
        format!("[本地] {}", path)
    }
}

// ---------------- Win32 小工具 ----------------

unsafe fn make_ctl(parent: HWND, class: &str, text: &str, style: u32, id: i32) -> HWND {
    let cw = to_wide(class);
    let tw = to_wide(text);
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR::from_raw(cw.as_ptr()),
        PCWSTR::from_raw(tw.as_ptr()),
        WINDOW_STYLE(style),
        0,
        0,
        10,
        10,
        parent,
        HMENU(id as isize as *mut _),
        HINSTANCE(hinst.0),
        None,
    )
    .expect("create ctl");
    if let Some(f) = font_hfont() {
        SendMessageW(hwnd, WM_SETFONT, WPARAM(f.0 as usize), LPARAM(1));
    }
    hwnd
}

fn font_hfont() -> Option<HFONT> {
    FONT.get().map(|v| HFONT(*v as *mut _))
}

/// GetDlgItem (0.58 返回 Result)
fn dlg_item(hwnd: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(hwnd, id).unwrap_or_default() }
}

unsafe fn set_wtext(hwnd: HWND, text: &str) {
    let v = to_wide(text);
    SetWindowTextW(hwnd, PCWSTR::from_raw(v.as_ptr()));
}

unsafe fn get_wtext(hwnd: HWND) -> String {
    let len = SendMessageW(hwnd, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0 as usize;
    if len == 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len + 1];
    SendMessageW(hwnd, WM_GETTEXT, WPARAM(len + 1), LPARAM(buf.as_mut_ptr() as isize));
    String::from_utf16_lossy(&buf[..len])
}

unsafe fn lv_count(lv: HWND) -> i32 {
    SendMessageW(lv, LVM_GETITEMCOUNT, WPARAM(0), LPARAM(0)).0 as i32
}

/// 全部选中项索引
unsafe fn lv_selected(lv: HWND) -> Vec<i32> {
    let mut out = Vec::new();
    let mut i = SendMessageW(lv, LVM_GETNEXTITEM, WPARAM(u32::MAX as usize), LPARAM(LVNI_SELECTED as isize)).0;
    while i >= 0 && out.len() < 4096 {
        out.push(i as i32);
        i = SendMessageW(lv, LVM_GETNEXTITEM, WPARAM(i as usize), LPARAM(LVNI_SELECTED as isize)).0;
    }
    out
}

unsafe fn lv_clear(lv: HWND) {
    SendMessageW(lv, LVM_DELETEALLITEMS, WPARAM(0), LPARAM(0));
}

unsafe fn lv_insert(lv: HWND, i: i32, name: &str, icon: i32) {
    let mut w = to_wide(name);
    let it = LVITEMW {
        mask: LIST_VIEW_ITEM_FLAGS(LVIF_TEXT.0 | LVIF_IMAGE.0),
        iItem: i,
        iImage: icon,
        pszText: PWSTR(w.as_mut_ptr()),
        ..Default::default()
    };
    SendMessageW(lv, LVM_INSERTITEMW, WPARAM(0), LPARAM(&it as *const _ as isize));
}

unsafe fn lv_set_text(lv: HWND, i: i32, sub: i32, text: &str) {
    let mut w = to_wide(text);
    let it = LVITEMW {
        mask: LIST_VIEW_ITEM_FLAGS(LVIF_TEXT.0),
        iItem: i,
        iSubItem: sub,
        pszText: PWSTR(w.as_mut_ptr()),
        ..Default::default()
    };
    SendMessageW(lv, LVM_SETITEMW, WPARAM(0), LPARAM(&it as *const _ as isize));
}

unsafe fn lv_add_col(lv: HWND, i: i32, title: &str, w: i32) {
    let mut t = to_wide(title);
    let col = LVCOLUMNW {
        mask: LVCOLUMNW_MASK(LVCF_FMT.0 | LVCF_WIDTH.0 | LVCF_TEXT.0),
        fmt: LVCFMT_LEFT,
        cx: w,
        pszText: PWSTR(t.as_mut_ptr()),
        ..Default::default()
    };
    SendMessageW(lv, LVM_INSERTCOLUMNW, WPARAM(i as usize), LPARAM(&col as *const _ as isize));
}

unsafe fn lv_set_col_w(lv: HWND, i: usize, w: i32) {
    SendMessageW(lv, LVM_SETCOLUMNWIDTH, WPARAM(i), LPARAM(w as isize));
}

/// 取系统小图标 imagelist (整个进程共享一份)
unsafe fn sys_image_list() -> HIMAGELIST {
    let w = to_wide("C:\\");
    let mut sfi = SHFILEINFOW::default();
    let flags = SHGFI_FLAGS(SHGFI_SYSICONINDEX.0 | SHGFI_SMALLICON.0);
    let r = SHGetFileInfoW(
        PCWSTR::from_raw(w.as_ptr()),
        FILE_ATTRIBUTE_NORMAL,
        Some(&mut sfi),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        flags,
    );
    HIMAGELIST(r as isize)
}

/// (图标索引, 类型描述)。路径不必真实存在 (SHGFI_USEFILEATTRIBUTES 按扩展名取)
unsafe fn icon_and_type(name: &str, is_dir: bool) -> (i32, String) {
    let w = to_wide(name);
    let mut sfi = SHFILEINFOW::default();
    let attr = if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
    let flags = SHGFI_FLAGS(SHGFI_SYSICONINDEX.0 | SHGFI_SMALLICON.0 | SHGFI_USEFILEATTRIBUTES.0 | SHGFI_TYPENAME.0);
    let ok = SHGetFileInfoW(
        PCWSTR::from_raw(w.as_ptr()),
        attr,
        Some(&mut sfi),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        flags,
    );
    if ok != 0 {
        let t = wide_to_string(&sfi.szTypeName);
        if !t.is_empty() {
            return (sfi.iIcon, t);
        }
        return (sfi.iIcon, if is_dir { "文件夹".into() } else { "文件".into() });
    }
    (0, if is_dir { "文件夹".into() } else { "文件".into() })
}

/// 填充列表 (条目已含 "..")
unsafe fn fill_list(p: &mut Pane) {
    lv_clear(p.lv);
    for (i, e) in p.entries.iter().enumerate() {
        let ii = i as i32;
        if e.name == ".." {
            lv_insert(p.lv, ii, "..", icon_and_type("folder", true).0);
            lv_set_text(p.lv, ii, COL_SIZE as i32, "");
            lv_set_text(p.lv, ii, COL_TIME as i32, "");
            lv_set_text(p.lv, ii, COL_TYPE as i32, "上级目录");
            continue;
        }
        let (icon, typ) = icon_and_type(&e.name, e.is_dir);
        lv_insert(p.lv, ii, &e.name, icon);
        if e.is_dir {
            lv_set_text(p.lv, ii, COL_SIZE as i32, "");
        } else {
            let ro = if e.readonly { " (只读)" } else { "" };
            lv_set_text(p.lv, ii, COL_SIZE as i32, &format!("{}{}", fmt_size(e.size), ro));
        }
        lv_set_text(p.lv, ii, COL_TIME as i32, &fmt_time(e.mtime));
        lv_set_text(p.lv, ii, COL_TYPE as i32, &typ);
    }
}

// ---------------- 窗口 ----------------

/// 调试日志: 仅当环境变量 RCONTROL_FM_DEBUG 存在时写 %TEMP%\rcontrol_fm_debug.log (排查 F9 打不开等问题用)
fn dbg_log(s: &str) {
    if std::env::var_os("RCONTROL_FM_DEBUG").is_none() {
        return;
    }
    use std::io::Write;
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::temp_dir().join("rcontrol_fm_debug.log"))
        .and_then(|mut f| f.write_all(format!("{}\n", s).as_bytes()));
}

fn run_window(client: Arc<Client>) {
    dbg_log("run_window enter");
    let (tx, rx) = channel::<UiUpdate>();
    let (task_tx, task_rx) = channel::<Task>();
    {
        let client = client.clone();
        let tx = tx.clone();
        std::thread::spawn(move || worker(client, task_rx, tx));
    }

    unsafe {
        let hinst = GetModuleHandleW(None).expect("module handle");

        // DPI (进程已 DPI Aware, 坐标全是物理像素)
        let hdc = GetDC(None);
        let dpi = if hdc.is_invalid() { 96 } else { GetDeviceCaps(hdc, LOGPIXELSY) };
        if !hdc.is_invalid() {
            ReleaseDC(None, hdc);
        }
        let s = dpi as f32 / 96.0;
        // Segoe UI 9pt
        let mut face = [0u16; 32];
        for (i, c) in "Segoe UI".encode_utf16().enumerate() {
            face[i] = c;
        }
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

        // 通用控件 v6 (manifest 已声明), ListView/StatusBar
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: INITCOMMONCONTROLSEX_ICC(ICC_LISTVIEW_CLASSES.0 | ICC_BAR_CLASSES.0),
        };
        let _ = InitCommonControlsEx(&icc);

        let class_name = to_wide("RControlFiles");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(fm_proc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH((windows::Win32::Graphics::Gdi::COLOR_WINDOW.0 + 1) as isize as *mut _),
            ..Default::default()
        };
        RegisterClassW(&wc);
        dbg_log("class registered");
        let title = to_wide("RControl 文件传输");
        let (ww, wh) = ((1060.0 * s) as i32, (660.0 * s) as i32);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            ww,
            wh,
            None,
            None,
            HINSTANCE(hinst.0),
            None,
        )
        .expect("create files window");
        dbg_log("window created");

        let himl = sys_image_list();
        let state = Box::new(FmState {
            panes: [
                Pane { path: String::new(), entries: Vec::new(), lv: HWND::default(), ed: HWND::default() },
                Pane { path: default_local_dir(), entries: Vec::new(), lv: HWND::default(), ed: HWND::default() },
            ],
            btn_dl: HWND::default(),
            btn_ul: HWND::default(),
            status: HWND::default(),
            pending: 0,
            dpi,
            tx: task_tx,
            rx,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
        dbg_log("state set");
        init_controls(hwnd, himl);
        dbg_log("controls created");
        set_status(hwnd, "提示: 双击进入目录, 选中文件后 [→ 下载到本地] / [← 上传到远程]; 暂不支持直接传输文件夹 (请先压缩)");
        let mut r = RECT::default();
        let _ = GetClientRect(hwnd, &mut r);
        layout(hwnd, r.right, r.bottom);
        let _ = SetTimer(hwnd, 1, 120, None);
        dbg_log("timer set, showing");
        ShowWindow(hwnd, SW_SHOW);
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);

        let mut msg = MSG::default();
        dbg_log("entering message loop");
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // 回车 = 路径栏"转到" (焦点在路径编辑框时)
            if msg.message == WM_KEYDOWN && msg.wParam.0 == VK_RETURN.0 as usize {
                let focus = GetFocus();
                let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if p != 0 {
                    let g = &mut *(p as *mut FmState);
                    if focus == g.panes[0].ed {
                        go_from_edit(hwnd, g, 0);
                        continue;
                    }
                    if focus == g.panes[1].ed {
                        go_from_edit(hwnd, g, 1);
                        continue;
                    }
                }
            }
            // Esc 关闭文件窗口 (不转发: 这是本线程自己的消息循环)
            if msg.message == WM_KEYDOWN && msg.wParam.0 == VK_ESCAPE.0 as usize {
                PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some(f) = font_hfont() {
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(f.0));
        }
    }
}

/// 创建全部子控件 (位置由 layout 决定)
unsafe fn init_controls(hwnd: HWND, himl: HIMAGELIST) {
    for remote in [true, false] {
        let ids = if remote {
            [ID_ED_R, ID_UP_R, ID_GO_R, ID_REF_R, ID_LV_R, ID_MK_R, ID_RM_R, ID_RN_R]
        } else {
            [ID_ED_L, ID_UP_L, ID_GO_L, ID_REF_L, ID_LV_L, ID_MK_L, ID_RM_L, ID_RN_L]
        };
        let title = if remote { "远程 (被控端)" } else { "本地 (主控端)" };
        let _ = make_ctl(hwnd, "STATIC", title, STYLE_STATIC, ids[0] - 1);
        let ed = make_ctl(hwnd, "EDIT", "", STYLE_EDIT_PLAIN, ids[0]);
        let _ = make_ctl(hwnd, "BUTTON", "↑ 上级", STYLE_BTN, ids[1]);
        let _ = make_ctl(hwnd, "BUTTON", "转到", STYLE_BTN, ids[2]);
        let _ = make_ctl(hwnd, "BUTTON", "刷新", STYLE_BTN, ids[3]);
        let lv = make_ctl(hwnd, "SysListView32", "", STYLE_LV, ids[4]);
        SendMessageW(lv, LVM_SETEXTENDEDLISTVIEWSTYLE, WPARAM(0), LPARAM((LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize));
        SendMessageW(lv, LVM_SETIMAGELIST, WPARAM(LVSIL_SMALL as usize), LPARAM(himl.0));
        lv_add_col(lv, 0, "名称", 260);
        lv_add_col(lv, 1, "大小", 90);
        lv_add_col(lv, 2, "修改时间", 140);
        lv_add_col(lv, 3, "类型", 130);
        let _ = make_ctl(hwnd, "BUTTON", "新建文件夹", STYLE_BTN, ids[5]);
        let _ = make_ctl(hwnd, "BUTTON", "删除", STYLE_BTN, ids[6]);
        let _ = make_ctl(hwnd, "BUTTON", "重命名", STYLE_BTN, ids[7]);
        let g = state_of(hwnd);
        let i = if remote { 0 } else { 1 };
        g.panes[i].lv = lv;
        g.panes[i].ed = ed;
        set_wtext(g.panes[i].ed, &path_label(remote, &g.panes[i].path));
    }
    let g = state_of(hwnd);
    g.btn_dl = make_ctl(hwnd, "BUTTON", "→ 下载到本地", STYLE_BTN, ID_DL);
    g.btn_ul = make_ctl(hwnd, "BUTTON", "← 上传到远程", STYLE_BTN, ID_UL);
    g.status = make_ctl(hwnd, "msctls_statusbar32", "", WS_CHILD_V, 9100);
}

/// 全窗口布局 (WM_SIZE / 初始)
unsafe fn layout(hwnd: HWND, cx: i32, cy: i32) {
    if cx <= 0 || cy <= 0 {
        return;
    }
    let g = state_of(hwnd);
    let s = g.dpi as f32 / 96.0;
    let sc = |v: i32| (v as f32 * s) as i32;

    // 状态栏先贴底
    SendMessageW(g.status, WM_SIZE, WPARAM(0), LPARAM(cx as isize | ((cy as isize) << 16)));
    let mut sr = RECT::default();
    let _ = GetWindowRect(g.status, &mut sr);
    let sb_h = (sr.bottom - sr.top).max(sc(22));
    let avail_h = cy - sb_h;

    let m = sc(10);
    let center_w = sc(150);
    let pane_w = ((cx - 2 * m - center_w - 2 * sc(8)) / 2).max(sc(320));
    let px = [m, m + pane_w + sc(8) + center_w + sc(8)];
    let cx_center = m + pane_w + sc(8);

    let label_y = m;
    let label_h = sc(20);
    let row_y = label_y + label_h + sc(2);
    let row_h = sc(26);
    let lv_y = row_y + row_h + sc(6);
    let btn_h = sc(30);
    let btn_y = avail_h - btn_h - m;
    let lv_h = (btn_y - sc(6) - lv_y).max(sc(120));

    let up_w = sc(64);
    let go_w = sc(56);
    let ref_w = sc(56);
    let gap = sc(4);
    let ed_w = pane_w - up_w - go_w - ref_w - 3 * gap;

    for i in 0..2usize {
        let p = &g.panes[i];
        let x = px[i];
        let id0 = ID_ED_R + (i as i32) * 1000;
        let _ = MoveWindow(dlg_item(hwnd, id0 - 1), x, label_y, pane_w, label_h, true);
        let _ = MoveWindow(p.ed, x, row_y, ed_w, row_h, true);
        let _ = MoveWindow(dlg_item(hwnd, id0 + 1), x + ed_w + gap, row_y, up_w, row_h, true);
        let _ = MoveWindow(dlg_item(hwnd, id0 + 2), x + ed_w + gap + up_w + gap, row_y, go_w, row_h, true);
        let _ = MoveWindow(dlg_item(hwnd, id0 + 3), x + ed_w + gap + up_w + gap + go_w + gap, row_y, ref_w, row_h, true);
        let _ = MoveWindow(p.lv, x, lv_y, pane_w, lv_h, true);
        let mut bx = x;
        for (off, w) in [(0i32, 100i32), (1, 64), (2, 76)] {
            let _ = MoveWindow(dlg_item(hwnd, id0 + 5 + off), bx, btn_y, sc(w), btn_h, true);
            bx += sc(w) + gap;
        }
        // 列宽: 名称自适应剩余宽度
        let w_size = sc(90);
        let w_time = sc(140);
        let w_type = sc(130);
        let w_name = (pane_w - w_size - w_time - w_type - sc(26)).max(sc(120));
        lv_set_col_w(p.lv, COL_NAME, w_name);
        lv_set_col_w(p.lv, COL_SIZE, w_size);
        lv_set_col_w(p.lv, COL_TIME, w_time);
        lv_set_col_w(p.lv, COL_TYPE, w_type);
    }

    let mid = lv_y + lv_h / 2;
    let _ = MoveWindow(g.btn_dl, cx_center, mid - sc(21) - sc(5), center_w, sc(34), true);
    let _ = MoveWindow(g.btn_ul, cx_center, mid + sc(5), center_w, sc(34), true);
}

unsafe fn state_of(hwnd: HWND) -> &'static mut FmState {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut FmState;
    &mut *p
}

unsafe fn set_status(hwnd: HWND, text: &str) {
    let g = state_of(hwnd);
    let w = to_wide(text);
    SendMessageW(g.status, SB_SETTEXTW, WPARAM(0), LPARAM(w.as_ptr() as isize));
}

/// 请求刷新目录 (立即更新路径栏, 内容等 worker 回包)
unsafe fn navigate(hwnd: HWND, remote: bool, path: &str) {
    let g = state_of(hwnd);
    let i = if remote { 0 } else { 1 };
    let _ = g.tx.send(if remote { Task::ListRemote(path.to_string()) } else { Task::ListLocal(path.to_string()) });
    set_wtext(g.panes[i].ed, &path_label(remote, path));
}

/// 从路径编辑框读路径并跳转 (规范化: 去空白; "C:" 补 "\")
unsafe fn go_from_edit(hwnd: HWND, g: &mut FmState, i: usize) {
    let raw = get_wtext(g.panes[i].ed).trim().to_string();
    let raw = raw.strip_prefix(if i == 0 { "[远程] " } else { "[本地] " }).map(|s| s.trim().to_string()).unwrap_or(raw);
    let path = if raw.len() == 2 && raw.as_bytes()[1] == b':' { format!("{}\\", raw) } else { raw };
    navigate(hwnd, i == 0, &path);
}

/// 双击/回车打开条目
unsafe fn open_entry(hwnd: HWND, i_pane: usize, idx: i32) {
    if idx < 0 {
        return;
    }
    let g = state_of(hwnd);
    let p = &g.panes[i_pane];
    if idx as usize >= p.entries.len() {
        return;
    }
    let e = p.entries[idx as usize].clone();
    let remote = i_pane == 0;
    if e.name == ".." {
        if let Some(par) = parent_path(&p.path) {
            navigate(hwnd, remote, &par);
        }
    } else if e.is_dir {
        navigate(hwnd, remote, &join_path(&p.path, &e.name));
    } else {
        set_status(hwnd, if remote { "远程文件: 选中后点 [→ 下载到本地]" } else { "本地文件: 选中后点 [← 上传到远程]" });
    }
}

fn confirm(hwnd: HWND, text: &str) -> bool {
    let t = to_wide(text);
    let c = to_wide("确认");
    unsafe {
        MessageBoxW(hwnd, PCWSTR::from_raw(t.as_ptr()), PCWSTR::from_raw(c.as_ptr()), MB_OKCANCEL | MB_ICONQUESTION) == IDOK
    }
}

/// 选中条目中的普通文件 (排除 ".." 与目录)
fn selected_files(g: &FmState, i_pane: usize) -> (Vec<Entr>, usize) {
    let p = &g.panes[i_pane];
    let mut files = Vec::new();
    let mut dirs = 0;
    unsafe {
        for idx in lv_selected(p.lv) {
            if let Some(e) = p.entries.get(idx as usize) {
                if e.is_dir {
                    dirs += 1;
                } else {
                    files.push(e.clone());
                }
            }
        }
    }
    (files, dirs)
}

unsafe extern "system" fn fm_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            if GetWindowLongPtrW(hwnd, GWLP_USERDATA) != 0 {
                let cx = (lp.0 & 0xffff) as u32 as i32;
                let cy = ((lp.0 >> 16) & 0xffff) as u32 as i32;
                layout(hwnd, cx, cy);
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if p != 0 {
                let dpi = (*(p as *mut FmState)).dpi as f32 / 96.0;
                let mmi = &mut *(lp.0 as *mut MINMAXINFO);
                mmi.ptMinTrackSize.x = (900.0 * dpi) as i32;
                mmi.ptMinTrackSize.y = (560.0 * dpi) as i32;
            }
            LRESULT(0)
        }
        WM_TIMER => {
            let g = state_of(hwnd);
            while let Ok(u) = g.rx.try_recv() {
                if let Some((is_remote, path, entries)) = u.listing {
                    let i = if is_remote { 0 } else { 1 };
                    let p = &mut g.panes[i];
                    p.path = path.clone();
                    p.entries = vec![Entr::dotdot()];
                    if !path.is_empty() {
                        p.entries.extend(entries);
                    } else {
                        p.entries = Vec::new(); // 驱动器列表不显示 ".."
                        p.entries.extend(entries);
                    }
                    set_wtext(p.ed, &path_label(is_remote, &p.path));
                    fill_list(p);
                }
                if let Some(s) = u.status {
                    set_status(hwnd, &s);
                }
                if u.transfers_done > 0 {
                    g.pending = g.pending.saturating_sub(u.transfers_done);
                    if g.pending == 0 {
                        let _ = EnableWindow(g.btn_dl, true);
                        let _ = EnableWindow(g.btn_ul, true);
                    }
                }
                if let Some(e) = u.error {
                    crate::dlg::alert(&e);
                }
            }
            LRESULT(0)
        }
        WM_NOTIFY => {
            let nm = &*(lp.0 as *const NMHDR);
            let pane = if nm.idFrom == ID_LV_R as usize {
                0
            } else if nm.idFrom == ID_LV_L as usize {
                1
            } else {
                return DefWindowProcW(hwnd, msg, wp, lp);
            };
            if nm.code == NM_DBLCLK {
                let ia = &*(lp.0 as *const NMITEMACTIVATE);
                open_entry(hwnd, pane, ia.iItem);
            } else if nm.code == NM_RETURN {
                let g = state_of(hwnd);
                let sel = lv_selected(g.panes[pane].lv);
                if let Some(&i) = sel.first() {
                    open_entry(hwnd, pane, i);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wp.0 & 0xffff) as i32;
            let g = state_of(hwnd);
            match id {
                ID_UP_R | ID_UP_L => {
                    let i = if id == ID_UP_R { 0 } else { 1 };
                    if let Some(par) = parent_path(&g.panes[i].path) {
                        navigate(hwnd, i == 0, &par);
                    }
                }
                ID_GO_R | ID_GO_L => {
                    let i = if id == ID_GO_R { 0 } else { 1 };
                    go_from_edit(hwnd, g, i);
                }
                ID_REF_R | ID_REF_L => {
                    let i = if id == ID_REF_R { 0 } else { 1 };
                    navigate(hwnd, i == 0, &g.panes[i].path.clone());
                }
                ID_MK_R | ID_MK_L => {
                    let i = if id == ID_MK_R { 0 } else { 1 };
                    let suggest = {
                        let (files, _) = selected_files(g, i);
                        let sel_name = lv_selected(g.panes[i].lv)
                            .first()
                            .and_then(|&idx| g.panes[i].entries.get(idx as usize))
                            .map(|e| e.name.clone())
                            .unwrap_or_default();
                        let _ = files;
                        join_path(&g.panes[i].path, &sel_name)
                    };
                    if let Some(t) = input_box(hwnd, "新建文件夹 (完整路径)", &suggest) {
                        let _ = g.tx.send(Task::Mkdir { remote: i == 0, path: t });
                    }
                }
                ID_RM_R | ID_RM_L => {
                    let i = if id == ID_RM_R { 0 } else { 1 };
                    let sel: Vec<String> = lv_selected(g.panes[i].lv)
                        .iter()
                        .filter_map(|&idx| g.panes[i].entries.get(idx as usize))
                        .filter(|e| e.name != "..")
                        .map(|e| join_path(&g.panes[i].path, &e.name))
                        .collect();
                    if sel.is_empty() {
                        set_status(hwnd, "请先选中要删除的条目");
                        return LRESULT(0);
                    }
                    let msg = if sel.len() == 1 {
                        format!("确定删除 {} 吗? (文件夹将整棵删除)", sel[0])
                    } else {
                        format!("确定删除选中的 {} 项吗? (文件夹将整棵删除)", sel.len())
                    };
                    if confirm(hwnd, &msg) {
                        for p in sel {
                            let _ = g.tx.send(Task::Remove { remote: i == 0, path: p });
                        }
                    }
                }
                ID_RN_R | ID_RN_L => {
                    let i = if id == ID_RN_R { 0 } else { 1 };
                    let sel_name = lv_selected(g.panes[i].lv)
                        .first()
                        .and_then(|&idx| g.panes[i].entries.get(idx as usize))
                        .filter(|e| e.name != "..")
                        .map(|e| join_path(&g.panes[i].path, &e.name));
                    let Some(old) = sel_name else { set_status(hwnd, "请先选中一个要重命名的条目"); return LRESULT(0) };
                    if let Some(new) = input_box(hwnd, "重命名为 (完整路径)", &old) {
                        if new != old {
                            let _ = g.tx.send(Task::Rename { remote: i == 0, old, new });
                        }
                    }
                }
                ID_DL => on_download(hwnd, g),
                ID_UL => on_upload(hwnd, g),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            KillTimer(hwnd, 1);
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut FmState;
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

unsafe fn on_download(hwnd: HWND, g: &mut FmState) {
    if g.pending > 0 {
        set_status(hwnd, "有传输正在进行, 请稍候");
        return;
    }
    let (files, dirs) = selected_files(g, 0);
    if files.is_empty() {
        set_status(hwnd, if dirs > 0 { "暂不支持传输文件夹, 请先压缩" } else { "请先在远程栏选中文件" });
        return;
    }
    if g.panes[1].path.is_empty() {
        set_status(hwnd, "请先在本地栏进入一个目录 (下载目标)");
        return;
    }
    let overwrite: Vec<&Entr> = files
        .iter()
        .filter(|e| Path::new(&join_path(&g.panes[1].path, &e.name)).exists())
        .collect();
    if !overwrite.is_empty() {
        let msg = if overwrite.len() == 1 {
            format!("本地已存在同名文件 {}, 覆盖?", overwrite[0].name)
        } else {
            format!("本地已存在 {} 个同名文件, 全部覆盖?", overwrite.len())
        };
        if !confirm(hwnd, &msg) {
            return;
        }
    }
    g.pending = files.len() as u32;
    EnableWindow(g.btn_dl, false);
    EnableWindow(g.btn_ul, false);
    set_status(hwnd, &format!("开始下载 {} 个文件...", files.len()));
    for e in &files {
        let _ = g.tx.send(Task::Download {
            remote: join_path(&g.panes[0].path, &e.name),
            local: join_path(&g.panes[1].path, &e.name),
        });
    }
}

unsafe fn on_upload(hwnd: HWND, g: &mut FmState) {
    if g.pending > 0 {
        set_status(hwnd, "有传输正在进行, 请稍候");
        return;
    }
    let (files, dirs) = selected_files(g, 1);
    if files.is_empty() {
        set_status(hwnd, if dirs > 0 { "暂不支持传输文件夹, 请先压缩" } else { "请先在本地栏选中文件" });
        return;
    }
    if g.panes[0].path.is_empty() {
        set_status(hwnd, "请先在远程栏进入目录 (双击盘符), 上传需要目标目录");
        return;
    }
    let overwrite: Vec<&Entr> = files
        .iter()
        .filter(|e| g.panes[0].entries.iter().any(|r| r.is_dir == false && r.name.eq_ignore_ascii_case(&e.name)))
        .collect();
    if !overwrite.is_empty() {
        let msg = if overwrite.len() == 1 {
            format!("远程已存在同名文件 {}, 覆盖?", overwrite[0].name)
        } else {
            format!("远程已存在 {} 个同名文件, 全部覆盖?", overwrite.len())
        };
        if !confirm(hwnd, &msg) {
            return;
        }
    }
    g.pending = files.len() as u32;
    EnableWindow(g.btn_dl, false);
    EnableWindow(g.btn_ul, false);
    set_status(hwnd, &format!("开始上传 {} 个文件...", files.len()));
    for e in &files {
        let _ = g.tx.send(Task::Upload {
            local: join_path(&g.panes[1].path, &e.name),
            remote: join_path(&g.panes[0].path, &e.name),
        });
    }
}

// ---------------- 简易输入框 (新建/重命名) ----------------

unsafe fn input_box(parent: HWND, title: &str, default: &str) -> Option<String> {
    struct IbState {
        ed: HWND,
        result: Option<String>,
    }
    unsafe extern "system" fn ib_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_CREATE => {
                let cs = &*(lp.0 as *const CREATESTRUCTW);
                let st = Box::from_raw(cs.lpCreateParams as *mut IbState);
                let hinst = GetModuleHandleW(None).unwrap_or_default();
                let mut cls = to_wide("EDIT");
                let ed = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    PCWSTR::from_raw(cls.as_mut_ptr()),
                    PCWSTR::from_raw([0u16].as_ptr()),
                    WINDOW_STYLE(STYLE_EDIT_PLAIN),
                    10,
                    12,
                    470,
                    26,
                    hwnd,
                    HMENU(100 as isize as *mut _),
                    HINSTANCE(hinst.0),
                    None,
                )
                .unwrap();
                if let Some(f) = font_hfont() {
                    SendMessageW(ed, WM_SETFONT, WPARAM(f.0 as usize), LPARAM(1));
                }
                let mut ok = to_wide("确定");
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    PCWSTR::from_raw(to_wide("BUTTON").as_mut_ptr()),
                    PCWSTR::from_raw(ok.as_mut_ptr()),
                    WINDOW_STYLE(STYLE_BTN),
                    490,
                    10,
                    80,
                    30,
                    hwnd,
                    HMENU(1 as isize as *mut _),
                    HINSTANCE(hinst.0),
                    None,
                );
                let mut cancel = to_wide("取消");
                let cancel_h = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    PCWSTR::from_raw(to_wide("BUTTON").as_mut_ptr()),
                    PCWSTR::from_raw(cancel.as_mut_ptr()),
                    WINDOW_STYLE(STYLE_BTN),
                    490,
                    44,
                    80,
                    30,
                    hwnd,
                    HMENU(2 as isize as *mut _),
                    HINSTANCE(hinst.0),
                    None,
                )
                .unwrap();
                let _ = cancel_h;
                if let Some(f) = font_hfont() {
                    // 按钮字体: 找到后再设 (上面创建时未保存句柄, 逐一 GetDlgItem)
                    for bid in [1, 2] {
                        let b = dlg_item(hwnd, bid);
                        if !b.is_invalid() {
                            SendMessageW(b, WM_SETFONT, WPARAM(f.0 as usize), LPARAM(1));
                        }
                    }
                }
                let mut st = st;
                st.ed = ed;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = (wp.0 & 0xffff) as i32;
                if id == 1 {
                    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut IbState;
                    (*p).result = Some(get_wtext((*p).ed));
                }
                if id == 1 || id == 2 {
                    DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut IbState;
                if !p.is_null() {
                    let st = Box::from_raw(p);
                    ret_slot(st.result);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
    static IB_RESULT: Mutex<Option<String>> = Mutex::new(None);
    fn ret_slot(v: Option<String>) {
        *IB_RESULT.lock().unwrap() = v;
    }

    let hinst = GetModuleHandleW(None).expect("hinst");
    let cn = to_wide("RControlInputBox");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(ib_proc),
        hInstance: hinst.into(),
        lpszClassName: PCWSTR::from_raw(cn.as_ptr()),
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
        hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH((windows::Win32::Graphics::Gdi::COLOR_WINDOW.0 + 1) as isize as *mut _),
        ..Default::default()
    };
    RegisterClassW(&wc);
    let st = Box::into_raw(Box::new(IbState { ed: HWND::default(), result: None }));
    let tw = to_wide(title);
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR::from_raw(cn.as_ptr()),
        PCWSTR::from_raw(tw.as_ptr()),
        WS_OVERLAPPED | WS_SYSMENU | WS_CAPTION,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        600,
        120,
        parent,
        None,
        HINSTANCE(hinst.0),
        Some(st as *const _),
    )
    .unwrap(); // 所有权已转移给 WM_CREATE 的 Box::from_raw
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut IbState;
    if !p.is_null() {
        set_wtext((*p).ed, default);
    }
    ShowWindow(hwnd, SW_SHOW);
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        // 回车 = 确定
        if msg.message == WM_KEYDOWN && msg.wParam.0 == VK_RETURN.0 as usize {
            PostMessageW(hwnd, WM_COMMAND, WPARAM(1), LPARAM(0));
            continue;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    IB_RESULT.lock().unwrap().take()
}

// ---------------- worker 线程 ----------------

fn status_send(tx: &Sender<UiUpdate>, s: String) {
    let _ = tx.send(UiUpdate { status: Some(s), ..Default::default() });
}

fn error_send(tx: &Sender<UiUpdate>, s: String) {
    let _ = tx.send(UiUpdate { error: Some(s), ..Default::default() });
}

fn worker(client: Arc<Client>, rx: Receiver<Task>, tx: Sender<UiUpdate>) {
    list_remote(&client, "", &tx);
    list_local(&default_local_dir(), &tx);
    let mut last_remote = String::new();
    let mut last_local = default_local_dir();
    while let Ok(task) = rx.recv() {
        if client.dead_reason().is_some() {
            error_send(&tx, "连接已断开, 文件管理器不可用".into());
            continue;
        }
        match task {
            Task::ListRemote(p) => {
                list_remote(&client, &p, &tx);
                last_remote = p;
            }
            Task::ListLocal(p) => {
                list_local(&p, &tx);
                last_local = p;
            }
            Task::Download { remote, local } => {
                do_download(&client, &remote, &local, &tx);
                let _ = tx.send(UiUpdate { transfers_done: 1, ..Default::default() });
                list_remote(&client, &last_remote, &tx);
                list_local(&last_local, &tx);
            }
            Task::Upload { local, remote } => {
                do_upload(&client, &local, &remote, &tx);
                let _ = tx.send(UiUpdate { transfers_done: 1, ..Default::default() });
                list_remote(&client, &last_remote, &tx);
                list_local(&last_local, &tx);
            }
            Task::Mkdir { remote, path } => {
                if remote {
                    let id = client.next_file_id();
                    let _ = client.send_file_req(id, file_request::Op::Mkdir(FileMkdirReq { path: path.clone() }));
                    match client.wait_file_resp(id, std::time::Duration::from_secs(30)) {
                        Some(r) => {
                            if let Some(file_response::Op::Error(e)) = r.op {
                                error_send(&tx, format!("新建文件夹失败: {}", e.msg));
                            } else {
                                status_send(&tx, "已新建文件夹".into());
                            }
                        }
                        None => error_send(&tx, "新建文件夹失败 (超时/断开)".into()),
                    }
                    list_remote(&client, &last_remote, &tx);
                } else {
                    let ok = fs::create_dir_all(&path).is_ok();
                    status_send(&tx, if ok { "已新建文件夹".into() } else { "新建文件夹失败".into() });
                    list_local(&last_local, &tx);
                }
            }
            Task::Remove { remote, path } => {
                if remote {
                    let id = client.next_file_id();
                    let _ = client.send_file_req(id, file_request::Op::Remove(FileRemoveReq { path }));
                    match client.wait_file_resp(id, std::time::Duration::from_secs(60)) {
                        Some(r) => match r.op {
                            Some(file_response::Op::Error(e)) => error_send(&tx, format!("远程删除失败: {}", e.msg)),
                            _ => status_send(&tx, "已删除".into()),
                        },
                        None => error_send(&tx, "远程删除失败 (超时/断开)".into()),
                    }
                    list_remote(&client, &last_remote, &tx);
                } else {
                    let res = if Path::new(&path).is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
                    let msg = if res.is_ok() { "已删除".to_string() } else { res.err().map(|e| e.to_string()).unwrap_or_default() };
                    status_send(&tx, msg);
                    list_local(&last_local, &tx);
                }
            }
            Task::Rename { remote, old, new } => {
                if remote {
                    let id = client.next_file_id();
                    let _ = client.send_file_req(id, file_request::Op::Rename(FileRenameReq { old, new }));
                    match client.wait_file_resp(id, std::time::Duration::from_secs(30)) {
                        Some(r) => {
                            if let Some(file_response::Op::Error(e)) = r.op {
                                error_send(&tx, format!("远程重命名失败: {}", e.msg));
                            }
                        }
                        None => error_send(&tx, "远程重命名失败 (超时/断开)".into()),
                    }
                    list_remote(&client, &last_remote, &tx);
                } else {
                    let msg = if fs::rename(&old, &new).is_ok() { "已重命名".into() } else { "重命名失败".into() };
                    status_send(&tx, msg);
                    list_local(&last_local, &tx);
                }
            }
        }
    }
}

fn list_remote(client: &Arc<Client>, path: &str, tx: &Sender<UiUpdate>) {
    let id = client.next_file_id();
    if !client.send_file_req(id, file_request::Op::List(FileListReq { path: path.to_string() })) {
        return;
    }
    match client.wait_file_resp(id, std::time::Duration::from_secs(20)) {
        Some(r) => match r.op {
            Some(file_response::Op::Entries(en)) => {
                let mut v: Vec<Entr> = en.entries.iter().map(Entr::from_file).collect();
                sort_entries(&mut v);
                let _ = tx.send(UiUpdate {
                    status: Some(format!("远程 {} 项", v.len())),
                    listing: Some((true, path.to_string(), v)),
                    ..Default::default()
                });
            }
            Some(file_response::Op::Error(e)) => {
                error_send(tx, format!("无法列出远程目录: {}", e.msg));
            }
            _ => {}
        },
        None => error_send(tx, "列远程目录超时".into()),
    }
}

fn list_drives() -> Vec<Entr> {
    let mut v = Vec::new();
    unsafe {
        let drives = windows::Win32::Storage::FileSystem::GetLogicalDrives();
        for i in 0..26u32 {
            if drives & (1 << i) != 0 {
                v.push(Entr { name: format!("{}:\\", (b'A' + i as u8) as char), is_dir: true, size: 0, mtime: 0, readonly: false });
            }
        }
    }
    v
}

fn list_local(path: &str, tx: &Sender<UiUpdate>) {
    if path.is_empty() {
        let _ = tx.send(UiUpdate {
            status: Some("本地 驱动器列表".into()),
            listing: Some((false, String::new(), list_drives())),
            ..Default::default()
        });
        return;
    }
    match fs::read_dir(path) {
        Ok(rd) => {
            let mut v: Vec<Entr> = Vec::new();
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.is_empty() {
                    continue;
                }
                if let Ok(md) = e.metadata() {
                    v.push(Entr {
                        is_dir: md.is_dir(),
                        size: if md.is_dir() { 0 } else { md.len() },
                        mtime: md
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0),
                        readonly: md.permissions().readonly(),
                        name,
                    });
                }
            }
            sort_entries(&mut v);
            let n = v.len();
            let _ = tx.send(UiUpdate {
                status: Some(format!("本地 {} 项", n)),
                listing: Some((false, path.to_string(), v)),
                ..Default::default()
            });
        }
        Err(e) => {
            error_send(tx, format!("无法列出本地目录 {}: {}", path, e));
        }
    }
}

fn speed_str(bytes: u64, t0: Instant) -> String {
    let el = t0.elapsed().as_secs_f64().max(0.001);
    format!("{:.1} MB/s", bytes as f64 / 1024.0 / 1024.0 / el)
}

fn do_download(client: &Arc<Client>, remote: &str, local: &str, tx: &Sender<UiUpdate>) {
    let name = remote.rsplit('\\').next().unwrap_or(remote);
    let id = client.next_file_id();
    if !client.send_file_req(id, file_request::Op::Download(FileDownloadReq { path: remote.to_string() })) {
        status_send(tx, format!("下载 {} 失败: 发送请求失败", name));
        return;
    }
    let meta = loop {
        match client.wait_file_resp(id, std::time::Duration::from_secs(30)) {
            Some(r) => match r.op {
                Some(file_response::Op::DownloadMeta(m)) => break m,
                Some(file_response::Op::Error(e)) => {
                    status_send(tx, format!("下载 {} 失败: {}", name, e.msg));
                    return;
                }
                _ => continue,
            },
            None => {
                status_send(tx, format!("下载 {} 超时/断开", name));
                return;
            }
        }
    };
    let mut f = match fs::File::create(local) {
        Ok(f) => f,
        Err(e) => {
            status_send(tx, format!("无法创建本地文件 {}: {}", local, e));
            return;
        }
    };
    let t0 = Instant::now();
    let mut received: u64 = 0;
    loop {
        match client.wait_file_resp(id, std::time::Duration::from_secs(120)) {
            Some(r) => match r.op {
                Some(file_response::Op::DownloadData(d)) => {
                    if !d.chunk.is_empty() {
                        if let Err(e) = f.write_all(&d.chunk) {
                            status_send(tx, format!("写入失败: {}", e));
                            return;
                        }
                        received += d.chunk.len() as u64;
                        status_send(tx, format!("下载 {} {}/{} ({})", name, fmt_size(received), fmt_size(meta.size), speed_str(received, t0)));
                        let _ = client.send_file_req(id, file_request::Op::DownloadAck(FileDownloadAck { received }));
                    }
                    if d.eof {
                        status_send(tx, format!("下载完成 {} ({})", local, fmt_size(received)));
                        return;
                    }
                }
                Some(file_response::Op::Error(e)) => {
                    status_send(tx, format!("下载中断: {}", e.msg));
                    return;
                }
                _ => {}
            },
            None => {
                status_send(tx, format!("下载中断 (连接断开或超时), 已收 {}", fmt_size(received)));
                return;
            }
        }
    }
}

/// 流式上传 (不整读进内存)
fn do_upload(client: &Arc<Client>, local: &str, remote: &str, tx: &Sender<UiUpdate>) {
    let name = local.rsplit('\\').next().unwrap_or(local);
    let mut f = match fs::File::open(local) {
        Ok(f) => f,
        Err(e) => {
            status_send(tx, format!("无法读取本地文件 {}: {}", local, e));
            return;
        }
    };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let id = client.next_file_id();
    if !client.send_file_req(id, file_request::Op::Upload(FileUploadReq { path: remote.to_string(), size })) {
        status_send(tx, format!("上传 {} 失败: 发送请求失败", name));
        return;
    }
    loop {
        match client.wait_file_resp(id, std::time::Duration::from_secs(30)) {
            Some(r) => match r.op {
                Some(file_response::Op::UploadAck(_)) => break,
                Some(file_response::Op::Error(e)) => {
                    status_send(tx, format!("上传 {} 失败: {}", name, e.msg));
                    return;
                }
                _ => continue,
            },
            None => {
                status_send(tx, format!("上传 {} 超时/断开", name));
                return;
            }
        }
    }
    let chunk = rcontrol_common::codec::file_chunk_size();
    let mut buf = vec![0u8; chunk];
    let t0 = Instant::now();
    let mut sent: u64 = 0;
    let mut since_report = 0u64;
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) => 0,
            Ok(n) => n,
            Err(e) => {
                status_send(tx, format!("读取本地文件失败: {}", e));
                return;
            }
        };
        let eof = n == 0 || sent + n as u64 >= size;
        if !client.send_file_req(
            id,
            file_request::Op::UploadData(FileUploadData { chunk: buf[..n].to_vec(), eof }),
        ) {
            status_send(tx, format!("上传中断 (发送失败), 已发 {}", fmt_size(sent)));
            return;
        }
        sent += n as u64;
        since_report += n as u64;
        if since_report >= chunk as u64 * 8 || eof {
            status_send(tx, format!("上传 {} {}/{} ({})", name, fmt_size(sent), fmt_size(size), speed_str(sent, t0)));
            since_report = 0;
        }
        if eof {
            break;
        }
    }
    // 等 FileDone
    match client.wait_file_resp(id, std::time::Duration::from_secs(60)) {
        Some(r) => match r.op {
            Some(file_response::Op::Error(e)) => status_send(tx, format!("上传中断: {}", e.msg)),
            _ => status_send(tx, format!("上传完成 {} ({})", remote, fmt_size(sent))),
        },
        None => status_send(tx, "上传完成 (未收到确认)".into()),
    }
}
