//! GDI 屏幕捕获 (整块虚拟屏幕, 支持锁屏/登录界面安全桌面)
//! 锁屏时 DXGI 复制不可用, GDI 是唯一能在 winlogon 桌面工作的捕获方式 (rustdesk 同款方案)

use crate::desktop::DesktopState;
use std::time::Duration;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, HDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS,
    SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorInfo, GetSystemMetrics, CURSOR_SHOWING, CURSORINFO, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    /// 缩放后的 RGB 数据 (已按 max_width 缩放)
    pub rgb: Vec<u8>,
    /// 像素是否相对上帧有变化
    pub changed: bool,
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub cursor_visible: bool,
    pub on_secure: bool,
}

pub struct Capturer {
    desktop: DesktopState,
    // GDI 对象 (按尺寸缓存, 尺寸变化时重建)
    dc_mem: Option<HDC>,
    bmp: Option<windows::Win32::Graphics::Gdi::HBITMAP>,
    width: i32,
    height: i32,
    vx: i32,
    vy: i32,
    bmi: BITMAPINFO,
    buf: Vec<u8>,   // BGRA top-down
    last_rgb: Vec<u8>,
    max_width: u32,
    pub last_error: String,
}

impl Default for Capturer {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Capturer {
    pub fn new(max_width: u32) -> Self {
        Capturer {
            desktop: DesktopState::new(),
            dc_mem: None,
            bmp: None,
            width: 0,
            height: 0,
            vx: 0,
            vy: 0,
            bmi: unsafe { std::mem::zeroed() },
            buf: Vec::new(),
            last_rgb: Vec::new(),
            max_width,
            last_error: String::new(),
        }
    }

    pub fn is_secure(&self) -> bool {
        self.desktop.is_secure()
    }

    /// 运行时调整最大编码宽度
    pub fn set_max_width(&mut self, w: u32) {
        if w != self.max_width {
            self.max_width = w;
            self.last_rgb.clear(); // 尺寸变化, 强制全帧发送
        }
    }

    /// 查询虚拟屏幕尺寸 (可在任意线程调用)
    pub fn screen_size() -> (i32, i32) {
        unsafe {
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            (w.max(1), h.max(1))
        }
    }

    fn rebuild_gdi_objects(&mut self) -> bool {
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if w <= 0 || h <= 0 {
                self.last_error = "bad screen metrics".into();
                return false;
            }
            if w == self.width && h == self.height && self.dc_mem.is_some() {
                return true;
            }
            self.release_gdi_objects();
            let screen_dc = GetDC(None);
            if screen_dc.is_invalid() {
                self.last_error = "GetDC failed".into();
                return false;
            }
            let mem = CreateCompatibleDC(screen_dc);
            let bmp = CreateCompatibleBitmap(screen_dc, w, h);
            ReleaseDC(None, screen_dc);
            if mem.is_invalid() || bmp.is_invalid() {
                self.last_error = "create dc/bmp failed".into();
                return false;
            }
            SelectObject(mem, windows::Win32::Graphics::Gdi::HGDIOBJ(bmp.0));
            self.dc_mem = Some(mem);
            self.bmp = Some(bmp);
            self.width = w;
            self.height = h;
            self.vx = vx;
            self.vy = vy;
            self.bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            self.buf = vec![0u8; (w * h * 4) as usize];
            crate::logger::log_line("INFO", &format!("capture buffer {}x{} at ({},{})", w, h, vx, vy));
            true
        }
    }

    fn release_gdi_objects(&mut self) {
        unsafe {
            if let Some(b) = self.bmp.take() {
                let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(b.0));
            }
            if let Some(d) = self.dc_mem.take() {
                let _ = DeleteDC(d);
            }
            // 尺寸清零, 触发下次重建
            self.width = 0;
            self.height = 0;
        }
    }

    /// 捕获一帧。失败返回 None (调用方应 sleep 后重试, 不要断开连接)。
    pub fn capture(&mut self) -> Option<CapturedFrame> {
        // 1. 附加到当前输入桌面 (300ms 检查一次, 失败时立即重试)
        if !self.desktop.ensure(Duration::from_millis(300)) {
            self.last_error = "open input desktop failed".into();
            self.release_gdi_objects(); // 换桌面后 GDI 对象需要重建
            return None;
        }

        // 2. 重建/检查 GDI 对象
        if !self.rebuild_gdi_objects() {
            return None;
        }

        unsafe {
            // 3. BitBlt 整个虚拟屏幕
            let screen_dc = GetDC(None);
            if screen_dc.is_invalid() {
                self.last_error = "GetDC failed".into();
                return None;
            }
            let mem = self.dc_mem.unwrap();
            let blt_ok = BitBlt(
                mem,
                0,
                0,
                self.width,
                self.height,
                screen_dc,
                self.vx,
                self.vy,
                SRCCOPY | CAPTUREBLT,
            )
            .is_ok();
            ReleaseDC(None, screen_dc);
            if !blt_ok {
                self.last_error = format!("BitBlt failed: {}", std::io::Error::last_os_error());
                self.release_gdi_objects();
                return None;
            }

            // 4. GetDIBits -> BGRA
            let mut bmi = self.bmi;
            let rows = GetDIBits(
                mem,
                self.bmp.unwrap(),
                0,
                self.height as u32,
                Some(self.buf.as_mut_ptr().cast()),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            if rows == 0 {
                self.last_error = format!("GetDIBits failed: {}", std::io::Error::last_os_error());
                self.release_gdi_objects();
                return None;
            }

            // 5. 光标信息 (换算到帧坐标: 相对采集原点 + 按缩放比例)
            let mut ci = CURSORINFO {
                cbSize: std::mem::size_of::<CURSORINFO>() as u32,
                ..Default::default()
            };
            let ci_ok = GetCursorInfo(&mut ci).is_ok();

            // 6. BGRA -> RGB (可选缩放)
            let (w, h) = (self.width as u32, self.height as u32);
            let scale = if self.max_width > 0 && w > self.max_width {
                self.max_width as f64 / w as f64
            } else {
                1.0
            };
            let (tw, th) = if scale < 1.0 {
                ((w as f64 * scale).round() as u32, (h as f64 * scale).round() as u32)
            } else {
                (w, h)
            };
            let (cursor_x, cursor_y, cursor_visible) = if ci_ok && ci.flags == CURSOR_SHOWING {
                (
                    ((ci.ptScreenPos.x - self.vx) as f64 * tw as f64 / w as f64) as i32,
                    ((ci.ptScreenPos.y - self.vy) as f64 * th as f64 / h as f64) as i32,
                    true,
                )
            } else {
                (0, 0, false)
            };
            let rgb = if scale < 1.0 {
                crate::encode::bgra_scale_to_rgb(&self.buf, w, h, tw, th)
            } else {
                crate::encode::bgra_to_rgb(&self.buf)
            };

            // 7. 与上帧比较, 静止画面跳过编码
            let changed = rgb != self.last_rgb;
            if changed {
                self.last_rgb = rgb.clone();
            }
            self.last_error.clear();
            Some(CapturedFrame {
                width: tw,
                height: th,
                rgb,
                changed,
                cursor_x,
                cursor_y,
                cursor_visible,
                on_secure: self.desktop.is_secure(),
            })
        }
    }
}

impl Drop for Capturer {
    fn drop(&mut self) {
        self.release_gdi_objects();
    }
}
