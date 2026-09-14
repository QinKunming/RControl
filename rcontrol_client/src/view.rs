//! minifb 远程画面窗口: 解码/显示 + 鼠标键盘捕获
//!
//! 显示策略: 远程画面按 1:1 物理像素显示 (不做比例缩放, 避免分辨率不一致时映射出错)。
//! 窗口放不下时显示自绘滚动条, 滚轮/拖拽平移; 鼠标坐标映射 = 窗口像素 + 滚动偏移, 纯整数加法。

use crate::conn::{Client, Frame};
use rcontrol_common::proto::*;
use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};
use rcontrol_common::proto;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// 滚动条宽度 (物理像素)
const SBW: i32 = 14;
/// 滚轮每格滚动像素
const WHEEL_STEP: i32 = 64;
/// 滚动条滑块最小长度
const THUMB_MIN: i32 = 24;
const COLOR_BG: u32 = 0xFF10_1010;
const COLOR_TRACK: u32 = 0xFF3A_3A3A;
const COLOR_THUMB: u32 = 0xFF8A_8A8A;
const COLOR_THUMB_ACTIVE: u32 = 0xFFC0_C0C0;

/// 滚动条滑块拖拽状态: 记录按下点相对滑块顶端的偏移
#[derive(Clone, Copy)]
enum Grab {
    V(i32),
    H(i32),
}

/// 运行画面窗口, 直到窗口关闭或连接断开
pub fn run_viewer(client: std::sync::Arc<Client>, view_only: bool) {
    // 等第一帧确定尺寸
    let first = match client.wait_frame(client.frame_seq(), Duration::from_secs(8)) {
        Some(f) => f,
        None => {
            let msg = client.dead_reason().unwrap_or_else(|| "8 秒内未收到画面".into());
            crate::dlg::alert(&format!("无法获取远程画面: {}", msg));
            return;
        }
    };
    let (mut fw, mut fh, mut img) = match jpeg_to_u32(&first) {
        Some(v) => v,
        None => {
            crate::dlg::alert("首帧解码失败");
            return;
        }
    };

    // 初始窗口 (客户区尺寸): 画面放得下工作区 -> 精确画面原始尺寸并居中;
    // 放不下 -> 最大化占满主控屏, 靠滚动条平移。
    // 创建尺寸必须保证"含边框的整个窗口"都在工作区内 (最小化/还原后也回到这个尺寸,
    // 否则窗口底边会被任务栏压住, 底部水平滚动条看不见)。
    let (area_w, area_h) = work_area();
    let (dec_w, dec_h) = decoration_size();
    let avail_w = (area_w - dec_w).max(320);
    let avail_h = (area_h - dec_h).max(240);
    let win_w = (fw as i32).min(avail_w).max(320) as usize;
    let win_h = (fh as i32).min(avail_h).max(240) as usize;

    let title = format!("RControl - {}", client.hostname);
    let mut opts = WindowOptions::default();
    opts.resize = true;
    let mut window = match Window::new(&title, win_w, win_h, opts) {
        Ok(w) => w,
        Err(e) => {
            crate::dlg::alert(&format!("创建窗口失败: {}", e));
            return;
        }
    };
    window.set_target_fps(60);
    place_initial_window(&title, fw as i32, fh as i32);
    window.set_title(&format!("{}  [{}x{} 1:1]", title, fw, fh));

    let mut screen: Vec<u32> = Vec::new(); // 窗口客户区缓冲 (含滚动条)
    let mut last_frame = first;
    let mut last_drawn_seq = 0u64;
    let mut last_title = Instant::now() - Duration::from_secs(2);
    let mut last_clip_poll = Instant::now() - Duration::from_secs(2);
    let mut clip_last_sent = String::new();
    let mut clip_last_set = String::new();

    // 输入状态
    let keys = crate::keys::table();
    let mut pressed: HashSet<Key> = HashSet::new();
    let mut mouse_btn = [false; 3]; // 左/中/右
    let mut btn_sent = [false; 3]; // 按下是否已转发 (滚动条上的按下不转发, 对应的抬起也不转发)
    let mut last_mouse = (-1.0f32, -1.0f32); // 最近一次转发给远端的帧坐标
    let mut scroll = (0i32, 0i32); // 滚动偏移 (帧坐标)
    let mut grab: Option<Grab> = None;

    // Ctrl+Shift+Del 发 SAS: 记录吞掉的键
    let mut swallowed: HashSet<Key> = HashSet::new();

    // 告知服务端初始配置
    let _ = client.send(&Message {
        msg: Some(message::Msg::SetConfig(SetConfig { fps: 0, quality: 0, max_width: 0, view_only })),
    });

    while window.is_open() {
        // ---- 画面 ----
        let seq = client.frame_seq();
        if seq != last_drawn_seq {
            if let Some(f) = client.wait_frame(seq, Duration::from_millis(1)) {
                last_frame = f;
                last_drawn_seq = seq;
                if let Some((w, h, decoded)) = jpeg_to_u32(&last_frame) {
                    if (w, h) != (fw, fh) {
                        scroll = (0, 0); // 远端分辨率变化, 滚动复位
                    }
                    fw = w;
                    fh = h;
                    img = decoded;
                }
            }
        }
        let (wsz, hsz) = window.get_size();
        let (cw, ch) = (wsz as i32, hsz as i32);
        let lay = view_layout(cw, ch, fw as i32, fh as i32);
        scroll.0 = clamp_scroll(scroll.0, fw as i32, lay.vp_w);
        scroll.1 = clamp_scroll(scroll.1, fh as i32, lay.vp_h);
        if cw > 0 && ch > 0 && !img.is_empty() {
            compose(&mut screen, cw, ch, &img, fw as i32, fh as i32, scroll.0, scroll.1, &lay, grab.is_some());
            let _ = window.update_with_buffer(&screen, wsz, hsz);
        } else {
            window.update();
        }

        // 连接死亡检查
        if let Some(reason) = client.dead_reason() {
            crate::dlg::alert(&format!("连接已断开\n\n{}", reason));
            return;
        }

        // ---- 键盘 ----
        // 失焦时释放所有按住的键 (minifb 收不到另一窗口里的 WM_KEYUP, 否则远端按键会卡住)
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
            if GetFocus().is_invalid() && !pressed.is_empty() {
                for (key, scan) in keys {
                    if pressed.remove(key) {
                        let _ = client.send_key(*scan, false);
                    }
                }
                swallowed.clear();
            }
        }
        let ctrl = window.is_key_down(Key::LeftCtrl) || window.is_key_down(Key::RightCtrl);
        let shift = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
        let alt = window.is_key_down(Key::LeftAlt) || window.is_key_down(Key::RightAlt);

        // 本地热键
        if window.is_key_pressed(Key::Escape, minifb::KeyRepeat::No) {
            break;
        }
        if window.is_key_pressed(Key::F9, minifb::KeyRepeat::No) {
            crate::files_ui::open(client.clone());
        }
        // Ctrl+Alt+Insert -> SAS (备用)
        if alt && ctrl && window.is_key_pressed(Key::Insert, minifb::KeyRepeat::No) && client.system_priv {
            let _ = client.send_sas();
        }

        if !view_only {
            for (key, scan) in keys {
                let down = window.is_key_down(*key);
                let was = pressed.contains(key);
                if down == was {
                    continue;
                }
                if down {
                    pressed.insert(*key);
                    // Ctrl+Shift+Del -> 远程 SAS
                    if *key == Key::Delete && ctrl && shift {
                        if client.system_priv {
                            let _ = client.send_sas();
                            swallowed.insert(*key);
                            continue;
                        }
                    }
                    // F9 是本地热键 (打开文件传输窗口), 不转发远端
                    if *key == Key::F9 {
                        swallowed.insert(*key);
                        continue;
                    }
                    let _ = client.send_key(*scan, true);
                } else {
                    pressed.remove(key);
                    if swallowed.remove(key) {
                        continue;
                    }
                    let _ = client.send_key(*scan, false);
                }
            }
        }

        // ---- 鼠标 ----
        if let Some((mx, my)) = window.get_unscaled_mouse_pos(MouseMode::Pass) {
            if cw > 0 && ch > 0 && !img.is_empty() {
                let (mxi, myi) = (mx as i32, my as i32);
                let (vtp, vtl) = thumb(lay.vp_h, fh as i32, lay.vp_h, scroll.1);
                let (htp, htl) = thumb(lay.vp_w, fw as i32, lay.vp_w, scroll.0);

                // 命中区域: 垂直条 / 水平条 / 右下角死区 / 画面
                let in_v = lay.has_v && mxi >= cw - SBW && myi < lay.vp_h;
                let in_h = lay.has_h && myi >= ch - SBW && mxi < lay.vp_w;
                let in_corner = lay.has_v && lay.has_h && mxi >= cw - SBW && myi >= ch - SBW;
                let over_view = !in_v && !in_h && !in_corner;
                // 帧坐标 = 窗口像素 - 显示偏移 + 滚动偏移 (1:1, 无比例换算)
                let fx = mx - lay.dx as f32 + scroll.0 as f32;
                let fy = my - lay.dy as f32 + scroll.1 as f32;
                let inside = fx >= 0.0 && fx < fw as f32 && fy >= 0.0 && fy < fh as f32;

                // 1) 拖拽滑块 (view_only 也允许平移查看)
                if let Some(g) = grab {
                    if !window.get_mouse_down(MouseButton::Left) {
                        grab = None;
                    } else {
                        match g {
                            Grab::V(off) => {
                                scroll.1 = clamp_scroll(drag_scroll(myi - off, vtl, lay.vp_h, fh as i32), fh as i32, lay.vp_h);
                            }
                            Grab::H(off) => {
                                scroll.0 = clamp_scroll(drag_scroll(mxi - off, htl, lay.vp_w, fw as i32), fw as i32, lay.vp_w);
                            }
                        }
                    }
                }

                // 2) 按钮状态机: 滚动条/画面外的按下不转发; 对应的抬起也不转发
                if grab.is_none() {
                    let btns = [(MouseButton::Left, 0usize), (MouseButton::Middle, 1), (MouseButton::Right, 2)];
                    for (b, i) in btns {
                        let down = window.get_mouse_down(b);
                        if down == mouse_btn[i] {
                            continue;
                        }
                        mouse_btn[i] = down;
                        if down {
                            // 按下在滚动条上: 启动拖拽/翻页
                            if i == 0 && in_v {
                                if myi >= vtp && myi < vtp + vtl {
                                    grab = Some(Grab::V(myi - vtp));
                                } else {
                                    scroll.1 = clamp_scroll(scroll.1 + if myi < vtp { -lay.vp_h } else { lay.vp_h }, fh as i32, lay.vp_h);
                                }
                                btn_sent[0] = false;
                            } else if i == 0 && in_h {
                                if mxi >= htp && mxi < htp + htl {
                                    grab = Some(Grab::H(mxi - htp));
                                } else {
                                    scroll.0 = clamp_scroll(scroll.0 + if mxi < htp { -lay.vp_w } else { lay.vp_w }, fw as i32, lay.vp_w);
                                }
                                btn_sent[0] = false;
                            } else if !view_only && over_view && inside {
                                let _ = client.send_mouse(fx as f64, fy as f64, btn_mask(i, true), 0);
                                btn_sent[i] = true;
                            } else {
                                btn_sent[i] = false;
                            }
                        } else if btn_sent[i] {
                            // 抬起永远配对转发 (位置用最后一次转发的坐标), 避免远端按键卡住
                            let _ = client.send_mouse(last_mouse.0 as f64, last_mouse.1 as f64, btn_mask(i, false), 0);
                            btn_sent[i] = false;
                        }
                    }
                }

                // 3) 移动: 仅画面内且未在拖拽滚动条时转发
                if grab.is_none() && !view_only && over_view && inside {
                    if (fx - last_mouse.0).abs() >= 0.5 || (fy - last_mouse.1).abs() >= 0.5 {
                        last_mouse = (fx, fy);
                        let _ = client.send_mouse(fx as f64, fy as f64, proto::MOUSE_MOVE, 0);
                    }
                }

                // 4) 滚轮: 有滚动条时平移画面, 完整放下画面时转发远端
                if let Some((wx, wy)) = window.get_scroll_wheel() {
                    let needs_scroll = lay.has_v || lay.has_h;
                    if !needs_scroll && !view_only {
                        // minifb 滚轮单位: 12.0/格 (WHEEL_DELTA 120 * 0.1); 换算成格数
                        let v = (wy / 12.0).round() as i32;
                        let h = (wx / 12.0).round() as i32;
                        if v != 0 {
                            let _ = client.send_mouse(fx as f64, fy as f64, proto::MOUSE_WHEEL_V, v);
                        }
                        if h != 0 {
                            let _ = client.send_mouse(fx as f64, fy as f64, proto::MOUSE_WHEEL_H, h);
                        }
                    } else {
                        let v = (wy / 12.0) as i32;
                        let h = (wx / 12.0) as i32;
                        if lay.has_h && (shift || !lay.has_v) {
                            scroll.0 = clamp_scroll(scroll.0 + v * WHEEL_STEP, fw as i32, lay.vp_w);
                        } else if lay.has_v {
                            scroll.1 = clamp_scroll(scroll.1 + v * WHEEL_STEP, fh as i32, lay.vp_h);
                        }
                        if h != 0 && lay.has_h {
                            scroll.0 = clamp_scroll(scroll.0 + h * WHEEL_STEP, fw as i32, lay.vp_w);
                        }
                    }
                }
            }
        }

        // ---- 剪贴板 ----
        if last_clip_poll.elapsed() >= Duration::from_millis(800) {
            last_clip_poll = Instant::now();
            // 接收: 服务端 -> 本地
            let incoming = client.shared.data.lock().unwrap().clipboard_in.take();
            if let Some(text) = incoming {
                if text != clip_last_set && !text.is_empty() {
                    if crate::clip::set_clipboard_text(&text) {
                        clip_last_set = text;
                    }
                }
            }
            // 发送: 本地 -> 服务端
            if let Some(cur) = crate::clip::get_clipboard_text() {
                if cur != clip_last_sent && cur != clip_last_set && !cur.is_empty() && cur.len() < 4 * 1024 * 1024 {
                    clip_last_sent = cur.clone();
                    let _ = client.send(&Message {
                        msg: Some(message::Msg::Clipboard(ClipboardEvent { text: cur })),
                    });
                }
            }
        }

        // ---- 标题栏状态 ----
        if last_title.elapsed() >= Duration::from_millis(1000) {
            last_title = Instant::now();
            let _ = client.send_ping();
            let lat = client.shared.data.lock().unwrap().latency_ms;
            let mut t = format!("RControl - {}", client.hostname);
            if let Some(ms) = lat {
                t.push_str(&format!("  ({} ms)", ms));
            }
            if view_only {
                t.push_str("  [仅查看]");
            }
            if !client.system_priv {
                t.push_str("  [被控端未装服务]");
            }
            t.push_str(&format!("  [{}x{} 1:1]", fw, fh));
            window.set_title(&t);
        }
    }
}

fn btn_mask(i: usize, down: bool) -> u32 {
    match (i, down) {
        (0, true) => proto::MOUSE_LEFT_DOWN,
        (0, false) => proto::MOUSE_LEFT_UP,
        (1, true) => proto::MOUSE_MIDDLE_DOWN,
        (1, false) => proto::MOUSE_MIDDLE_UP,
        (2, true) => proto::MOUSE_RIGHT_DOWN,
        _ => proto::MOUSE_RIGHT_UP,
    }
}

/// 本机工作区大小 (物理像素), 用于初始窗口尺寸
fn work_area() -> (i32, i32) {
    unsafe {
        let mut rc = windows::Win32::Foundation::RECT::default();
        let ok = windows::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
            windows::Win32::UI::WindowsAndMessaging::SPI_GETWORKAREA,
            0,
            Some((&mut rc as *mut _) as *mut core::ffi::c_void),
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok();
        if ok && rc.right > rc.left && rc.bottom > rc.top {
            (rc.right - rc.left, rc.bottom - rc.top)
        } else {
            use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
            (GetSystemMetrics(SM_CXSCREEN).max(640), GetSystemMetrics(SM_CYSCREEN).max(400))
        }
    }
}

/// 窗口边框+标题栏占的尺寸 (WS_OVERLAPPEDWINDOW 可调整边框样式, 与 minifb 创建时一致)
fn decoration_size() -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{AdjustWindowRect, WS_OVERLAPPEDWINDOW};
    unsafe {
        let mut rc = windows::Win32::Foundation::RECT::default();
        if AdjustWindowRect(&mut rc, WS_OVERLAPPEDWINDOW, windows::Win32::Foundation::BOOL(0)).is_ok() {
            (rc.right - rc.left, rc.bottom - rc.top)
        } else {
            (16, 40) // 拿不到就按常见值保守估计
        }
    }
}

struct FindHwnd {
    hwnd: windows::Win32::Foundation::HWND,
}

unsafe extern "system" fn enum_cb(hwnd: windows::Win32::Foundation::HWND, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::BOOL {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    };
    use windows::Win32::System::Threading::GetCurrentProcessId;
    let ctx = &mut *(lparam.0 as *mut FindHwnd);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid != GetCurrentProcessId() || !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    // 标题以 "RControl - " 开头 (后缀随状态变化, 不能精确匹配)
    let prefix: Vec<u16> = "RControl - ".encode_utf16().collect();
    let n = GetWindowTextLengthW(hwnd);
    if (n as usize) < prefix.len() {
        return BOOL(1);
    }
    let mut buf = vec![0u16; prefix.len() + 1];
    let _ = GetWindowTextW(hwnd, &mut buf);
    if buf[..prefix.len()] == prefix[..] {
        ctx.hwnd = hwnd;
        return BOOL(0); // 找到, 停止枚举
    }
    BOOL(1)
}

/// 初始摆位: 画面比主控屏工作区大 -> 最大化 (客户区占满工作区); 放得下 -> 窗口居中。
/// minifb 不暴露窗口句柄, 枚举本进程标题前缀为 "RControl - " 的顶层窗口。
fn place_initial_window(_title: &str, fw: i32, fh: i32) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, SetWindowPos, ShowWindow, SET_WINDOW_POS_FLAGS, SWP_NOSIZE,
        SWP_NOZORDER, SW_MAXIMIZE,
    };
    unsafe {
        let mut ctx = FindHwnd { hwnd: HWND::default() };
        let _ = EnumWindows(Some(enum_cb), windows::Win32::Foundation::LPARAM(&mut ctx as *mut FindHwnd as isize));
        if ctx.hwnd.0.is_null() {
            return; // 找不到就保持创建时的尺寸 (该尺寸保证完整可见), 不影响功能
        }
        let hwnd = ctx.hwnd;
        let (area_w, area_h) = work_area();
        if fw > area_w || fh > area_h {
            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
        } else {
            let mut rc = windows::Win32::Foundation::RECT::default();
            if GetWindowRect(hwnd, &mut rc).is_ok() {
                let ww = rc.right - rc.left;
                let wh = rc.bottom - rc.top;
                if ww > 0 && wh > 0 && ww <= area_w && wh <= area_h {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        (area_w - ww) / 2,
                        (area_h - wh) / 2,
                        0,
                        0,
                        SET_WINDOW_POS_FLAGS(SWP_NOSIZE.0 | SWP_NOZORDER.0),
                    );
                }
            }
        }
    }
}

/// 视口布局: 是否有滚动条, 视口大小, 画面显示偏移 (画面放得下时居中, 否则锚定左上)
struct Layout {
    has_v: bool,
    has_h: bool,
    vp_w: i32,
    vp_h: i32,
    dx: i32,
    dy: i32,
}

fn view_layout(cw: i32, ch: i32, fw: i32, fh: i32) -> Layout {
    let mut has_v = fh > ch;
    let mut has_h = fw > cw;
    // 一条滚动条占掉另一轴的空间后, 重新判断
    if has_h {
        has_v = fh > (ch - SBW).max(1);
    }
    if has_v {
        has_h = fw > (cw - SBW).max(1);
    }
    // 垂直条占右边缘宽度, 水平条占底边缘高度
    let vp_w = if has_v { (cw - SBW).max(1) } else { cw };
    let vp_h = if has_h { (ch - SBW).max(1) } else { ch };
    let fits = !has_v && !has_h;
    Layout {
        has_v,
        has_h,
        vp_w,
        vp_h,
        dx: if fits { (cw - fw) / 2 } else { 0 },
        dy: if fits { (ch - fh) / 2 } else { 0 },
    }
}

fn clamp_scroll(s: i32, content: i32, vp: i32) -> i32 {
    if content > vp {
        s.clamp(0, content - vp)
    } else {
        0
    }
}

/// 滑块位置与长度: (pos, len)
fn thumb(track_len: i32, content: i32, vp: i32, scroll: i32) -> (i32, i32) {
    if content <= vp || track_len <= 0 {
        return (0, track_len);
    }
    let len = ((vp as i64 * track_len as i64) / content as i64).clamp(THUMB_MIN as i64, track_len as i64) as i32;
    let max_scroll = content - vp;
    let pos = ((scroll as i64 * (track_len as i64 - len as i64)) / max_scroll as i64) as i32;
    (pos, len)
}

/// 拖拽滑块: 由滑块顶端位置反算滚动偏移
fn drag_scroll(thumb_pos: i32, thumb_len: i32, track_len: i32, content: i32) -> i32 {
    if content <= 0 || track_len <= thumb_len {
        return 0;
    }
    let max_scroll = content - track_len; // 视口即轨道长度
    ((thumb_pos as i64 * max_scroll as i64) / (track_len as i64 - thumb_len as i64).max(1)) as i32
}

/// 把画面按滚动偏移合成到窗口缓冲 (1:1 拷贝) 并画滚动条
fn compose(dst: &mut Vec<u32>, cw: i32, ch: i32, img: &[u32], fw: i32, fh: i32, sx: i32, sy: i32, lay: &Layout, dragging: bool) {
    let len = (cw as usize) * (ch as usize);
    if dst.len() != len {
        dst.resize(len, COLOR_BG);
    }
    dst.fill(COLOR_BG);
    // 画面
    if lay.has_v || lay.has_h {
        let rows = lay.vp_h.min(fh - sy).max(0);
        let cols = lay.vp_w.min(fw - sx).max(0);
        for y in 0..rows {
            let src = (sy + y) as usize * fw as usize + sx as usize;
            let d = y as usize * cw as usize;
            dst[d..d + cols as usize].copy_from_slice(&img[src..src + cols as usize]);
        }
    } else if lay.dx >= 0 && lay.dy >= 0 {
        let d0 = lay.dy as usize * cw as usize + lay.dx as usize;
        for y in 0..fh {
            let src = y as usize * fw as usize;
            let d = d0 + y as usize * cw as usize;
            dst[d..d + fw as usize].copy_from_slice(&img[src..src + fw as usize]);
        }
    }
    // 滚动条
    let tcol = if dragging { COLOR_THUMB_ACTIVE } else { COLOR_THUMB };
    if lay.has_v {
        let x0 = (cw - SBW) as usize;
        for y in 0..lay.vp_h as usize {
            for x in 0..SBW as usize {
                dst[y * cw as usize + x0 + x] = COLOR_TRACK;
            }
        }
        let (tp, tl) = thumb(lay.vp_h, fh, lay.vp_h, sy);
        for y in tp..(tp + tl).min(lay.vp_h) {
            for x in 0..SBW as usize {
                dst[y as usize * cw as usize + x0 + x] = tcol;
            }
        }
    }
    if lay.has_h {
        let y0 = (ch - SBW) as usize;
        for x in 0..lay.vp_w as usize {
            for y in 0..SBW as usize {
                dst[(y0 + y) * cw as usize + x] = COLOR_TRACK;
            }
        }
        let (tp, tl) = thumb(lay.vp_w, fw, lay.vp_w, sx);
        for x in tp..(tp + tl).min(lay.vp_w) {
            for y in 0..SBW as usize {
                dst[(y0 + y) * cw as usize + x as usize] = tcol;
            }
        }
    }
    if lay.has_v && lay.has_h {
        // 右下角
        let x0 = (cw - SBW) as usize;
        let y0 = (ch - SBW) as usize;
        for y in 0..SBW as usize {
            for x in 0..SBW as usize {
                dst[(y0 + y) * cw as usize + x0 + x] = COLOR_TRACK;
            }
        }
    }
}

/// JPEG 解码为 u32 像素 (0xAABBGGRR), 返回 (宽, 高, 数据)
fn jpeg_to_u32(f: &Frame) -> Option<(u32, u32, Vec<u32>)> {
    use zune_jpeg::JpegDecoder;
    let mut dec = JpegDecoder::new(&f.jpeg[..]);
    let pixels = dec.decode().ok()?;
    let (w, h) = match dec.info() {
        Some(i) => (i.width as u32, i.height as u32),
        None => (f.w, f.h),
    };
    if w == 0 || h == 0 {
        return None;
    }
    let len = (w as usize) * (h as usize);
    if pixels.len() < len * 3 {
        return None;
    }
    let mut out = vec![0u32; len];
    for i in 0..len {
        let p = i * 3; // zune-jpeg 默认输出 RGB (3 字节)
        let r = pixels[p] as u32;
        let g = pixels[p + 1] as u32;
        let b = pixels[p + 2] as u32;
        out[i] = 0xff00_0000 | (r << 16) | (g << 8) | b;
    }
    Some((w, h, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_centered_no_bars() {
        // 1920x1080 画面放进 1920x1200 窗口: 无滚动条, 上下各 60 居中
        let l = view_layout(1920, 1200, 1920, 1080);
        assert!(!l.has_v && !l.has_h);
        assert_eq!((l.dx, l.dy), (0, 60));
        assert_eq!((l.vp_w, l.vp_h), (1920, 1200));
    }

    #[test]
    fn oversize_both_bars() {
        // 3840x2160 画面放进 1920x1080 窗口: 双滚动条, 视口 (1920-14, 1080-14)
        let l = view_layout(1920, 1080, 3840, 2160);
        assert!(l.has_v && l.has_h);
        assert_eq!((l.dx, l.dy), (0, 0));
        assert_eq!((l.vp_w, l.vp_h), (1920 - SBW, 1080 - SBW));
        assert_eq!(clamp_scroll(99999, 3840, l.vp_w), 3840 - l.vp_w);
        assert_eq!(clamp_scroll(-5, 3840, l.vp_w), 0);
    }

    #[test]
    fn only_vertical_bar() {
        // 画面比窗口高但宽度放得下 (含垂直条占的 14px): 只出垂直条
        let l = view_layout(2000, 800, 1920, 1080);
        assert!(l.has_v && !l.has_h);
        assert_eq!(l.vp_w, 2000 - SBW); // 垂直条占宽度
        assert_eq!(l.vp_h, 800);        // 无水平条, 高度不占
        assert_eq!(clamp_scroll(99999, 1080, l.vp_h), 1080 - l.vp_h);
    }

    #[test]
    fn thumb_math() {
        // 滑块长度与位置: 内容 1080 视口 786, 滑块长 = 786*786/1080 = 572
        let (p0, l0) = thumb(786, 1080, 786, 0);
        assert_eq!(l0, 786 * 786 / 1080);
        assert_eq!(p0, 0);
        let (p1, _) = thumb(786, 1080, 786, 1080 - 786);
        assert_eq!(p1, 786 - l0); // 滚到底 = 滑块贴底
        // 拖拽反算: 把滑块拖到底应滚到底
        assert_eq!(drag_scroll(786 - l0, l0, 786, 1080), 1080 - 786);
        assert_eq!(drag_scroll(0, l0, 786, 1080), 0);
    }

    #[test]
    fn compose_maps_1_to_1() {
        // 8x4 画面放进 8x8 窗口 (居中 dy=2): 画面像素应原样出现在 (0..8, 2..6)
        let img: Vec<u32> = (0..32).map(|i| 0xFF00_0000 | i).collect();
        let lay = view_layout(8, 8, 8, 4);
        let mut dst = Vec::new();
        compose(&mut dst, 8, 8, &img, 8, 4, 0, 0, &lay, false);
        assert_eq!(dst[0], COLOR_BG); // 上黑边
        assert_eq!(dst[2 * 8 + 3], img[3]); // 第一行画面
        assert_eq!(dst[5 * 8 + 7], img[8 * 3 + 7]); // 最后一行画面
        assert_eq!(dst[6 * 8], COLOR_BG); // 下黑边
    }

    #[test]
    fn compose_scrolled() {
        // 80x120 画面放进 100x60 窗口: 只出垂直条, 视口 86x60
        let img: Vec<u32> = (0..80 * 120).map(|i| 0xFF00_0000 | i as u32).collect();
        let lay = view_layout(100, 60, 80, 120);
        assert!(lay.has_v && !lay.has_h);
        assert_eq!((lay.vp_w, lay.vp_h), (100 - SBW, 60));
        let mut dst = Vec::new();
        let sy = 120 - lay.vp_h; // 滚到底
        compose(&mut dst, 100, 60, &img, 80, 120, 0, sy, &lay, false);
        // 视口顶行 = 画面第 sy 行
        assert_eq!(dst[0], img[sy as usize * 80]);
        // 视口右下角: 画面宽 80, 最后可见列是 79 (视口 86 比画面宽)
        assert_eq!(dst[(lay.vp_h - 1) as usize * 100 + 79],
                   img[(sy + lay.vp_h - 1) as usize * 80 + 79]);
        // 画面右边缘(80)与滚动条(86)之间的空隙是背景
        assert_eq!(dst[0 * 100 + 83], COLOR_BG);
        // 滚动条轨道
        assert_eq!(dst[5 * 100 + 90], COLOR_TRACK);
    }

    #[test]
    fn compose_draws_both_bars() {
        // 3840x2160 画面放进 1920x1071 窗口 (宽放不下/高放不下):
        // 底部水平条与右侧垂直条都必须真实画出, 不能透出画面内容
        let (fw, fh) = (3840i32, 2160i32);
        let (cw, ch) = (1920i32, 1071i32);
        let green = 0xFF00FF00u32;
        let img = vec![green; (fw * fh) as usize];
        let lay = view_layout(cw, ch, fw, fh);
        assert!(lay.has_v && lay.has_h);
        let mut dst = Vec::new();
        compose(&mut dst, cw, ch, &img, fw, fh, 100, 100, &lay, false);
        let px = |x: i32, y: i32| dst[y as usize * cw as usize + x as usize];
        // 内容区是画面本身
        assert_eq!(px(900, 500), green);
        // 右下角交叉块 = 轨道色
        assert_eq!(px(cw - 7, ch - 7), COLOR_TRACK);
        // 水平条整行: 滑块段=滑块色, 其余=轨道色 (都绝不能是画面绿色)
        let (tp, tl) = thumb(lay.vp_w, fw, lay.vp_w, 100);
        for x in [0, tp + tl / 2, lay.vp_w - 5] {
            assert_ne!(px(x, ch - 7), green, "x={} 透出内容", x);
        }
        assert_eq!(px(tp + tl / 2, ch - 7), COLOR_THUMB);
        assert_eq!(px(0, ch - 7), COLOR_TRACK);
        // 垂直条同理
        let (tpv, tlv) = thumb(lay.vp_h, fh, lay.vp_h, 100);
        for y in [0, tpv + tlv / 2, lay.vp_h - 5] {
            assert_ne!(px(cw - 7, y), green, "y={} 透出内容", y);
        }
        assert_eq!(px(cw - 7, tpv + tlv / 2), COLOR_THUMB);
        assert_eq!(px(cw - 7, 0), COLOR_TRACK);
    }
}
