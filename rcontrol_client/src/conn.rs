//! 连接: 握手/认证 + 读写线程 + 共享状态

use rcontrol_common::codec;
use rcontrol_common::crypto::{self, Channel};
use rcontrol_common::proto::*;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 一帧远端画面
#[derive(Clone)]
pub struct Frame {
    pub jpeg: Vec<u8>,
    pub w: u32,
    pub h: u32,
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub cursor_visible: bool,
    pub on_secure: bool,
}

#[derive(Default)]
pub struct SharedData {
    /// 画面序号 (每收到带 JPEG 的帧 +1, 用于等待新帧)
    pub frame_seq: u64,
    pub frame: Option<Frame>,
    /// 文件响应队列
    pub files: Vec<FileResponse>,
    /// 服务端推送的剪贴板文本
    pub clipboard_in: Option<String>,
    pub latency_ms: Option<u64>,
    pub dead: Option<String>,
}

#[derive(Default)]
pub struct Shared {
    pub data: Mutex<SharedData>,
    pub cond: Condvar,
}

pub struct Client {
    stream: Mutex<TcpStream>,
    ch_wr: Mutex<Channel>,
    pub shared: Arc<Shared>,
    pub hostname: String,
    pub screen_w: u32,
    pub screen_h: u32,
    pub system_priv: bool,
    next_file_id: AtomicU32,
    /// 发送失败后置 true (避免重复打印)
    pub broken: std::sync::atomic::AtomicBool,
}

/// 连接并完成握手认证。失败返回原因字符串。
pub fn connect(addr: &str, password: &str, timeout: Duration) -> Result<Arc<Client>, String> {
    let mut stream = TcpStream::connect(addr).map_err(|e| format!("无法连接 {}: {}", addr, e))?;
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(timeout)).ok();

    // 1. ServerHello
    let hello = codec::read_msg_plain(&mut stream).map_err(|e| format!("握手失败: {}", e))?;
    let nonce_s = match hello.msg {
        Some(message::Msg::ServerHello(h)) => {
            if h.version != rcontrol_common::VERSION {
                return Err(format!("版本不兼容 (服务端 v{}, 本客户端 v{})", h.version, rcontrol_common::VERSION));
            }
            h.nonce
        }
        _ => return Err("握手失败: 未收到 ServerHello".into()),
    };

    // 2. 派生密钥并发送 AuthRequest
    let nonce_c = crypto::random_nonce();
    let (auth_key, session_key) = crypto::derive_keys(password, &nonce_c, &nonce_s);
    let req = Message {
        msg: Some(message::Msg::AuthRequest(AuthRequest { nonce: nonce_c, proof: crypto::client_proof(&auth_key) })),
    };
    codec::write_msg_plain(&mut stream, &req).map_err(|e| format!("发送失败: {}", e))?;

    // 3. AuthResponse
    let resp = codec::read_msg_plain(&mut stream).map_err(|e| format!("认证读取失败: {}", e))?;
    let auth = match resp.msg {
        Some(message::Msg::AuthResponse(a)) => a,
        _ => return Err("握手失败: 未收到 AuthResponse".into()),
    };
    if !auth.ok {
        return Err(if auth.reason.is_empty() { "口令错误".into() } else { auth.reason });
    }
    if auth.proof != crypto::server_proof(&auth_key) {
        return Err("服务端身份校验失败".into());
    }

    // 4. 读线程接管读端
    let rd_stream = stream.try_clone().map_err(|e| format!("clone socket: {}", e))?;
    let shared = Arc::new(Shared::default());
    {
        let shared = shared.clone();
        std::thread::spawn(move || reader_loop(rd_stream, Channel::new(&session_key), shared));
    }

    Ok(Arc::new(Client {
        stream: Mutex::new(stream),
        ch_wr: Mutex::new(Channel::new(&session_key)),
        shared,
        hostname: hostname_of(addr),
        screen_w: auth.screen_w,
        screen_h: auth.screen_h,
        system_priv: auth.system_priv,
        next_file_id: AtomicU32::new(1),
        broken: std::sync::atomic::AtomicBool::new(false),
    }))
}

fn hostname_of(addr: &str) -> String {
    addr.rsplit_once(':').map(|(h, _)| h.to_string()).unwrap_or_else(|| addr.to_string())
}

fn reader_loop(mut rd: TcpStream, mut ch: Channel, shared: Arc<Shared>) {
    loop {
        let r = codec::read_msg_enc(&mut rd, &mut ch);
        let msg = match r {
            Ok(m) => m,
            Err(e) => {
                let mut g = shared.data.lock().unwrap();
                if g.dead.is_none() {
                    g.dead = Some(format!("连接断开: {}", e));
                }
                drop(g);
                shared.cond.notify_all();
                return;
            }
        };
        let mut g = shared.data.lock().unwrap();
        match msg.msg {
            Some(message::Msg::VideoFrame(f)) => {
                if f.jpeg.is_empty() {
                    // 仅光标更新
                    if let Some(fr) = g.frame.as_mut() {
                        fr.cursor_x = f.cursor_x;
                        fr.cursor_y = f.cursor_y;
                        fr.cursor_visible = f.cursor_visible;
                        fr.on_secure = f.on_secure;
                    }
                } else {
                    g.frame_seq += 1;
                    g.frame = Some(Frame {
                        jpeg: f.jpeg,
                        w: f.w,
                        h: f.h,
                        cursor_x: f.cursor_x,
                        cursor_y: f.cursor_y,
                        cursor_visible: f.cursor_visible,
                        on_secure: f.on_secure,
                    });
                }
            }
            Some(message::Msg::FileResponse(f)) => g.files.push(f),
            Some(message::Msg::Clipboard(c)) => g.clipboard_in = Some(c.text),
            Some(message::Msg::Pong(p)) => {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                g.latency_ms = Some(now.saturating_sub(p.time));
            }
            Some(message::Msg::Disconnected(d)) => {
                if g.dead.is_none() {
                    g.dead = Some(format!("被控端断开: {}", d.reason));
                }
            }
            _ => {}
        }
        drop(g);
        shared.cond.notify_all();
    }
}

impl Client {
    /// 发送一条加密消息
    pub fn send(&self, msg: &Message) -> bool {
        let mut g = match self.stream.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let mut ch = match self.ch_wr.lock() {
            Ok(c) => c,
            Err(_) => return false,
        };
        match codec::write_msg_enc(&mut *g, &mut ch, msg) {
            Ok(()) => true,
            Err(e) => {
                self.broken.store(true, Ordering::Relaxed);
                let mut s = self.shared.data.lock().unwrap();
                if s.dead.is_none() {
                    s.dead = Some(format!("发送失败: {}", e));
                }
                drop(s);
                self.shared.cond.notify_all();
                false
            }
        }
    }

    pub fn next_file_id(&self) -> u32 {
        self.next_file_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn send_ping(&self) -> bool {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        self.send(&Message { msg: Some(message::Msg::Ping(Ping { time: now })) })
    }

    pub fn send_mouse(&self, x: f64, y: f64, mask: u32, wheel: i32) -> bool {
        self.send(&Message { msg: Some(message::Msg::Mouse(MouseEvent { x, y, mask, wheel })) })
    }

    pub fn send_key(&self, scancode: u32, down: bool) -> bool {
        self.send(&Message { msg: Some(message::Msg::Keyboard(KeyboardEvent { scancode, down })) })
    }

    pub fn send_sas(&self) -> bool {
        self.send(&Message { msg: Some(message::Msg::Sas(SasEvent {})) })
    }

    pub fn send_file_req(&self, id: u32, op: file_request::Op) -> bool {
        self.send(&Message { msg: Some(message::Msg::FileRequest(FileRequest { id, op: Some(op) })) })
    }

    /// 阻塞等待文件响应 (匹配 id), 也会顺带发现连接死亡
    pub fn wait_file_resp(&self, id: u32, timeout: Duration) -> Option<FileResponse> {
        let deadline = std::time::Instant::now() + timeout;
        let mut g = self.shared.data.lock().unwrap();
        loop {
            if let Some(pos) = g.files.iter().position(|f| f.id == id) {
                return Some(g.files.remove(pos));
            }
            if g.dead.is_some() {
                return None;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return None;
            }
            let (ng, _t) = self.shared.cond.wait_timeout(g, deadline - now).unwrap();
            g = ng;
        }
    }

    /// 等待下一帧 (frame_seq 超过 base)
    pub fn wait_frame(&self, base: u64, timeout: Duration) -> Option<Frame> {
        let deadline = std::time::Instant::now() + timeout;
        let mut g = self.shared.data.lock().unwrap();
        loop {
            if g.frame_seq > base {
                return g.frame.clone();
            }
            if g.dead.is_some() {
                return None;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return g.frame.clone();
            }
            let (ng, _t) = self.shared.cond.wait_timeout(g, deadline - now).unwrap();
            g = ng;
        }
    }

    pub fn frame_seq(&self) -> u64 {
        self.shared.data.lock().unwrap().frame_seq
    }

    pub fn dead_reason(&self) -> Option<String> {
        self.shared.data.lock().unwrap().dead.clone()
    }
}
