//! 工作进程: 真正的被控端服务 (TCP 监听/认证/视频/输入/文件/剪贴板)
//! 由服务看门狗以 SYSTEM 身份在控制台会话内拉起, 或由 GUI 以当前用户临时启动

use crate::capture::Capturer;
use crate::files::{self, FileCtl, Transfers};
use crate::input::{input_loop, InputEvent};
use rcontrol_common::codec;
use rcontrol_common::config::{load_server_config, save_server_config, server_config_path, ServerConfig};
use rcontrol_common::crypto::{self, Channel};
use rcontrol_common::proto::*;
use std::collections::HashMap;
use std::io;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const AUTH_FAIL_LIMIT: u32 = 5;
const AUTH_BLOCK_SECS: u64 = 60;

#[derive(Clone)]
pub struct RuntimeCfg {
    pub max_fps: u32,
    pub quality: u32,
    pub max_width: u32,
    pub allow_file: bool,
    pub allow_clipboard: bool,
    pub view_only: bool,
}

pub struct State {
    pub port: u16,
    pub password: String,
    pub cfg: Mutex<RuntimeCfg>,
    pub connected: AtomicBool,
    pub on_secure: AtomicBool,
    pub system_priv: bool,
    pub user_mode: bool,
    pub hostname: String,
    pub last_error: Mutex<String>,
    pub transfers: Transfers,
    pub auth_fail: Mutex<HashMap<IpAddr, (u32, Instant)>>,
    pub stop: AtomicBool,
    /// 当前编码帧尺寸 (供输入线程做 帧坐标->屏幕坐标 换算, 画质压缩降采样时两者不同)
    pub frame_w: std::sync::atomic::AtomicU32,
    pub frame_h: std::sync::atomic::AtomicU32,
}

impl State {
    fn set_err(&self, msg: impl Into<String>) {
        *self.last_error.lock().unwrap() = msg.into();
    }
}

pub fn run(user_mode: bool) {
    crate::logger::init(if user_mode { "worker_user" } else { "worker" });
    let mut cfg = load_server_config();
    // --port N: 本次运行覆盖监听端口 (不改配置文件, 多实例/排障用)
    if let Some(i) = std::env::args().position(|a| a == "--port") {
        if let Some(v) = std::env::args().nth(i + 1).and_then(|s| s.parse::<u16>().ok()) {
            if v > 0 {
                cfg.port = v;
            }
        }
    }
    if cfg.password.is_empty() {
        cfg.password = crypto::generate_password(10);
        let _ = save_server_config(&cfg);
    }
    crate::desktop::set_process_dpi_aware();

    let system_priv = is_running_as_system();
    let hostname = get_hostname();
    crate::logger::log_line(
        "INFO",
        &format!(
            "worker start: pid={} session={} system={} user_mode={} port={} hostname={}",
            std::process::id(),
            current_session_id(),
            system_priv,
            user_mode,
            cfg.port,
            hostname
        ),
    );

    let state = Arc::new(State {
        port: cfg.port,
        password: cfg.password.clone(),
        cfg: Mutex::new(RuntimeCfg {
            max_fps: cfg.max_fps,
            quality: cfg.quality,
            max_width: cfg.max_width,
            allow_file: cfg.allow_file,
            allow_clipboard: cfg.allow_clipboard,
            view_only: false,
        }),
        connected: AtomicBool::new(false),
        on_secure: AtomicBool::new(false),
        system_priv,
        user_mode,
        hostname,
        last_error: Mutex::new(String::new()),
        transfers: Arc::new(Mutex::new(HashMap::new())),
        auth_fail: Mutex::new(HashMap::new()),
        stop: AtomicBool::new(false),
        frame_w: std::sync::atomic::AtomicU32::new(0),
        frame_h: std::sync::atomic::AtomicU32::new(0),
    });

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime");
    rt.block_on(async_main(state));
    crate::logger::log_line("INFO", "worker exit");
    std::process::exit(0);
}

async fn async_main(state: Arc<State>) {
    // 状态文件任务
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(1000));
            loop {
                tick.tick().await;
                write_status(&state);
                if state.stop.load(Ordering::Relaxed) {
                    return;
                }
            }
        });
    }
    // 配置监视: 端口/口令变化 -> 退出自身, 由看门狗(服务/GUI)重启
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut last = config_sig();
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            loop {
                tick.tick().await;
                let cur = config_sig();
                if cur != last && last.is_some() {
                    crate::logger::log_line("INFO", "config changed, worker restarting");
                    state.stop.store(true, Ordering::Relaxed);
                    std::process::exit(0);
                }
                last = cur;
            }
        });
    }

    let port = state.port;
    loop {
        match TcpListener::bind(("0.0.0.0", port)).await {
            Ok(listener) => {
                crate::logger::log_line("INFO", &format!("listening on 0.0.0.0:{}", port));
                loop {
                    match listener.accept().await {
                        Ok((stream, peer)) => {
                            let st = state.clone();
                            let stopped = Arc::new(AtomicBool::new(false));
                            tokio::spawn(async move {
                                handle_conn(stream, peer.to_string(), st, stopped).await;
                            });
                        }
                        Err(e) => {
                            crate::logger::log_line("ERROR", &format!("accept error: {}", e));
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                    if state.stop.load(Ordering::Relaxed) {
                        return;
                    }
                }
            }
            Err(e) => {
                crate::logger::log_line("ERROR", &format!("bind 0.0.0.0:{} failed: {}", port, e));
                state.set_err(format!("端口 {} 绑定失败: {}", port, e));
                tokio::time::sleep(Duration::from_secs(3)).await;
                if state.stop.load(Ordering::Relaxed) {
                    return;
                }
            }
        }
    }
}

fn config_sig() -> Option<(u16, String)> {
    let text = std::fs::read_to_string(server_config_path()).ok()?;
    let cfg: ServerConfig = toml::from_str(&text).ok()?;
    Some((cfg.port, cfg.password))
}

fn write_status(state: &State) {
    #[derive(serde::Serialize)]
    struct Status {
        ts: u64,
        pid: u32,
        session: u32,
        system_priv: bool,
        user_mode: bool,
        port: u16,
        connected: bool,
        on_secure: bool,
        hostname: String,
        last_error: String,
    }
    let s = Status {
        ts: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        pid: std::process::id(),
        session: current_session_id(),
        system_priv: state.system_priv,
        user_mode: state.user_mode,
        port: state.port,
        connected: state.connected.load(Ordering::Relaxed),
        on_secure: state.on_secure.load(Ordering::Relaxed),
        hostname: state.hostname.clone(),
        last_error: state.last_error.lock().unwrap().clone(),
    };
    if let Ok(json) = serde_json::to_string(&s) {
        let dir = rcontrol_common::config::server_data_dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("status.json"), json);
    }
}

// ---------------- 连接处理 ----------------

async fn read_frame_async(r: &mut (impl AsyncReadExt + Unpin)) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > codec::MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad frame len"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_frame_async(w: &mut (impl AsyncWriteExt + Unpin), data: &[u8]) -> io::Result<()> {
    w.write_all(&(data.len() as u32).to_be_bytes()).await?;
    w.write_all(data).await?;
    Ok(())
}

async fn handle_conn(stream: TcpStream, peer: String, state: Arc<State>, stopped: Arc<AtomicBool>) {
    let _ = stream.set_nodelay(true);
    let ip: Option<IpAddr> = peer.rsplit(':').next().and_then(|s| s.parse().ok());

    // 已有连接: 拒绝新连接
    if state.connected.load(Ordering::Relaxed) {
        crate::logger::log_line("INFO", &format!("reject {}: already connected", peer));
        let (_, mut w) = stream.into_split();
        let hello = ServerHello {
            nonce: crypto::random_nonce(),
            hostname: state.hostname.clone(),
            version: rcontrol_common::VERSION,
        };
        let msg = Message { msg: Some(message::Msg::ServerHello(hello)) };
        let _ = write_frame_async(&mut w, &codec::encode_msg(&msg)).await;
        let resp = Message { msg: Some(message::Msg::AuthResponse(AuthResponse {
            ok: false,
            reason: "已有其他控制连接".into(),
            proof: Vec::new(),
            screen_w: 0,
            screen_h: 0,
            system_priv: false,
            on_secure: false,
        })) };
        let _ = write_frame_async(&mut w, &codec::encode_msg(&resp)).await;
        return;
    }

    crate::logger::log_line("INFO", &format!("connection from {}", peer));
    state.set_err("");
    state.connected.store(true, Ordering::Relaxed);

    let result = run_session(stream, &state, &stopped, ip).await;
    state.connected.store(false, Ordering::Relaxed);
    stopped.store(true, Ordering::Relaxed);
    crate::logger::log_line("INFO", &format!("connection {} closed: {:?}", peer, result.err()));
}

fn scaled_screen_size(state: &State) -> (u32, u32) {
    let cfg = state.cfg.lock().unwrap();
    let (w, h) = Capturer::screen_size();
    let scale = if cfg.max_width > 0 && w > cfg.max_width as i32 { cfg.max_width as f64 / w as f64 } else { 1.0 };
    if scale < 1.0 {
        ((w as f64 * scale).round() as u32, (h as f64 * scale).round() as u32)
    } else {
        (w as u32, h as u32)
    }
}

async fn run_session(
    stream: TcpStream,
    state: &Arc<State>,
    stopped: &Arc<AtomicBool>,
    ip: Option<IpAddr>,
) -> io::Result<()> {
    let (rd, mut wr) = stream.into_split();
    let mut rd = rd;

    // ---- 握手 ----
    let nonce_s = crypto::random_nonce();
    let hello = Message { msg: Some(message::Msg::ServerHello(ServerHello {
        nonce: nonce_s.clone(),
        hostname: state.hostname.clone(),
        version: rcontrol_common::VERSION,
    })) };
    write_frame_async(&mut wr, &codec::encode_msg(&hello)).await?;

    let auth_buf = tokio::time::timeout(Duration::from_secs(10), read_frame_async(&mut rd)).await??;
    let auth_msg = codec::decode_msg(&auth_buf)?;
    let (nonce_c, proof) = match auth_msg.msg {
        Some(message::Msg::AuthRequest(a)) => (a.nonce, a.proof),
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "expect AuthRequest")),
    };

    // 失败限速
    if let Some(ip) = ip {
        let mut fails = state.auth_fail.lock().unwrap();
        if let Some((count, first)) = fails.get_mut(&ip) {
            if first.elapsed().as_secs() >= AUTH_BLOCK_SECS {
                *count = 0;
            }
            if *count >= AUTH_FAIL_LIMIT {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "认证失败次数过多, 暂时拒绝"));
            }
        }
    }

    let (auth_key, session_key) = crypto::derive_keys(&state.password, &nonce_c, &nonce_s);
    let ok = crypto::verify_client_proof(&auth_key, &proof);
    let (sw, sh) = scaled_screen_size(state);

    let resp = Message { msg: Some(message::Msg::AuthResponse(AuthResponse {
        ok,
        reason: if ok { String::new() } else { "口令错误".into() },
        proof: if ok { crypto::server_proof(&auth_key) } else { Vec::new() },
        screen_w: sw,
        screen_h: sh,
        system_priv: state.system_priv,
        on_secure: state.on_secure.load(Ordering::Relaxed),
    })) };
    write_frame_async(&mut wr, &codec::encode_msg(&resp)).await?;
    if !ok {
        crate::logger::log_line("WARN", &format!("auth failed from {:?}", ip));
        if let Some(ip) = ip {
            let mut fails = state.auth_fail.lock().unwrap();
            let e = fails.entry(ip).or_insert((0, Instant::now()));
            e.0 += 1;
        }
        return Ok(());
    }
    crate::logger::log_line("INFO", "auth ok");

    // ---- 加密通道 ----
    let mut ch_send = Channel::new(&session_key);
    let mut ch_recv = Channel::new(&session_key);

    // 双通道写出: 优先通道(输入/控制/文件) + 视频通道(可丢帧)
    let (prio_tx, mut prio_rx) = tokio::sync::mpsc::channel::<Message>(256);
    let (video_tx, mut video_rx) = tokio::sync::mpsc::channel::<Message>(2);

    // 写出任务
    let writer = tokio::spawn(async move {
        let mut wr = wr;
        let mut video_open = true;
        let mut err: Option<io::Error> = None;
        loop {
            let msg = tokio::select! {
                biased;
                m = prio_rx.recv() => match m { Some(m) => m, None => break },
                m = video_rx.recv(), if video_open => match m {
                    Some(m) => m,
                    None => { video_open = false; continue }
                },
            };
            let bytes = codec::encode_msg(&msg);
            let ct = ch_send.seal(&bytes);
            if let Err(e) = write_frame_async(&mut wr, &ct).await {
                err = Some(e);
                break;
            }
        }
        let _ = wr.shutdown().await;
        err
    });

    // ---- 捕获线程 ----
    {
        let video_tx = video_tx.clone();
        let state = state.clone();
        let stopped = stopped.clone();
        std::thread::spawn(move || {
            capture_loop(video_tx, state, || stopped.load(Ordering::Relaxed));
        });
    }

    // ---- 输入线程 ----
    let (input_tx, input_rx) = mpsc::channel::<InputEvent>();
    {
        let stopped = stopped.clone();
        let st = state.clone();
        std::thread::spawn(move || {
            let frame_size = move || {
                (
                    st.frame_w.load(std::sync::atomic::Ordering::Relaxed),
                    st.frame_h.load(std::sync::atomic::Ordering::Relaxed),
                )
            };
            input_loop(input_rx, &|| stopped.load(Ordering::Relaxed), &frame_size);
        });
    }

    // ---- 剪贴板线程 ----
    if state.cfg.lock().unwrap().allow_clipboard {
        let prio_tx_c = prio_tx.clone();
        let state = state.clone();
        let stopped = stopped.clone();
        std::thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            let sec_state = state.clone();
            std::thread::spawn(move || {
                crate::clip::clipboard_poll_loop(tx, &|| false, move || sec_state.on_secure.load(Ordering::Relaxed))
            });
            loop {
                if stopped.load(Ordering::Relaxed) {
                    return;
                }
                match rx.recv_timeout(Duration::from_millis(300)) {
                    Ok(text) => {
                        let _ = prio_tx_c.blocking_send(Message { msg: Some(message::Msg::Clipboard(ClipboardEvent { text})) });
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        });
    }

    // ---- 文件响应回调 ----
    let transfers = state.transfers.clone();
    let respond: files::Respond = {
        let prio_tx = prio_tx.clone();
        Arc::new(move |m: Message| {
            let _ = prio_tx.blocking_send(m);
        })
    };

    // ---- 读循环 ----
    let result: io::Result<()> = loop {
        if stopped.load(Ordering::Relaxed) {
            break Ok(());
        }
        let ct = tokio::time::timeout(Duration::from_secs(60), read_frame_async(&mut rd)).await;
        let ct = match ct {
            Ok(r) => r?,
            Err(_) => break Err(io::Error::new(io::ErrorKind::TimedOut, "read timeout")),
        };
        let pt = match ch_recv.open(&ct) {
            Ok(p) => p,
            Err(_) => break Err(io::Error::new(io::ErrorKind::InvalidData, "decrypt failed")),
        };
        let msg = match codec::decode_msg(&pt) {
            Ok(m) => m,
            Err(e) => break Err(e),
        };
        match msg.msg {
            Some(message::Msg::Mouse(m)) => {
                if !state.cfg.lock().unwrap().view_only {
                    let _ = input_tx.send(InputEvent::Mouse(m));
                }
            }
            Some(message::Msg::Keyboard(k)) => {
                if !state.cfg.lock().unwrap().view_only {
                    let _ = input_tx.send(InputEvent::Key(k));
                }
            }
            Some(message::Msg::Sas(_)) => {
                if state.system_priv {
                    let _ = input_tx.send(InputEvent::Sas);
                } else {
                    let _ = prio_tx.try_send(Message { msg: Some(message::Msg::Disconnected(Disconnected {
                        reason: "被控端非服务(SYSTEM)模式, 无法发送 Ctrl+Alt+Del, 请安装服务".into(),
                    })) });
                }
            }
            Some(message::Msg::Clipboard(c)) => {
                if state.cfg.lock().unwrap().allow_clipboard {
                    crate::clip::set_clipboard_text(&c.text);
                }
            }
            Some(message::Msg::SetConfig(sc)) => {
                let mut cfg = state.cfg.lock().unwrap();
                if sc.fps > 0 {
                    cfg.max_fps = sc.fps.clamp(1, 60);
                }
                if sc.quality > 0 {
                    cfg.quality = sc.quality.clamp(10, 95);
                }
                if sc.max_width > 0 {
                    cfg.max_width = sc.max_width.clamp(640, 3840);
                }
                cfg.view_only = sc.view_only;
            }
            Some(message::Msg::Ping(p)) => {
                let _ = prio_tx.try_send(Message { msg: Some(message::Msg::Pong(Pong { time: p.time })) });
            }
            Some(message::Msg::FileRequest(fr)) => {
                let allow = state.cfg.lock().unwrap().allow_file;
                match fr.op.as_ref() {
                    Some(file_request::Op::DownloadAck(a)) => {
                        files::route_ctl(&transfers, fr.id, FileCtl::Ack(a.received));
                    }
                    Some(file_request::Op::UploadData(d)) => {
                        files::route_ctl(&transfers, fr.id, FileCtl::Data(d.chunk.clone(), d.eof));
                    }
                    Some(file_request::Op::Cancel(_)) => {
                        files::route_ctl(&transfers, fr.id, FileCtl::Cancel);
                    }
                    _ if !allow => {
                        let _ = prio_tx.try_send(Message { msg: Some(message::Msg::FileResponse(FileResponse {
                            id: fr.id,
                            op: Some(file_response::Op::Error(FileError { msg: "被控端已禁用文件传输".into() })),
                        })) });
                    }
                    _ => {
                        files::handle_request(fr, respond.clone(), transfers.clone());
                    }
                }
            }
            _ => {}
        }
    };

    stopped.store(true, Ordering::Relaxed);
    drop(prio_tx);
    drop(video_tx);
    // 传输线程可能还持有通道克隆, 写出任务限时等待
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
    result
}

// ---------------- 视频捕获循环 ----------------

fn capture_loop(video_tx: tokio::sync::mpsc::Sender<Message>, state: Arc<State>, stopped: impl Fn() -> bool) {
    let mut cap = {
        let cfg = state.cfg.lock().unwrap();
        Capturer::new(cfg.max_width)
    };
    let mut last_cursor = (i32::MIN, i32::MIN, false);
    let mut fail_count = 0u32;
    loop {
        if stopped() {
            return;
        }
        let t0 = Instant::now();
        let (fps, quality, max_width) = {
            let cfg = state.cfg.lock().unwrap();
            (cfg.max_fps, cfg.quality, cfg.max_width)
        };
        cap.set_max_width(max_width);
        match cap.capture() {
            Some(frame) => {
                state.on_secure.store(frame.on_secure, Ordering::Relaxed);
                state.frame_w.store(frame.width, Ordering::Relaxed);
                state.frame_h.store(frame.height, Ordering::Relaxed);
                fail_count = 0;
                let cursor = (frame.cursor_x, frame.cursor_y, frame.cursor_visible);
                let cursor_moved = cursor != last_cursor;
                last_cursor = cursor;
                if frame.changed {
                    if let Some(jpeg) = crate::encode::encode_jpeg(&frame.rgb, frame.width, frame.height, quality) {
                        let msg = Message { msg: Some(message::Msg::VideoFrame(VideoFrame {
                            jpeg,
                            w: frame.width,
                            h: frame.height,
                            cursor_x: frame.cursor_x,
                            cursor_y: frame.cursor_y,
                            cursor_visible: frame.cursor_visible,
                            on_secure: frame.on_secure,
                        })) };
                        // 拥塞时丢帧保延迟
                        let _ = video_tx.try_send(msg);
                    }
                } else if cursor_moved {
                    // 仅光标变化: 轻量帧
                    let msg = Message { msg: Some(message::Msg::VideoFrame(VideoFrame {
                        jpeg: Vec::new(),
                        w: frame.width,
                        h: frame.height,
                        cursor_x: frame.cursor_x,
                        cursor_y: frame.cursor_y,
                        cursor_visible: frame.cursor_visible,
                        on_secure: frame.on_secure,
                    })) };
                    let _ = video_tx.try_send(msg);
                }
            }
            None => {
                // 锁屏瞬间/桌面切换失败: 等待重试, 不算致命
                fail_count += 1;
                if fail_count == 1 || fail_count % 30 == 0 {
                    crate::logger::log_line("INFO", &format!("capture unavailable: {}", cap.last_error));
                }
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
        }
        // 帧率控制 (计入捕获+编码耗时)
        let frame_budget = Duration::from_millis((1000 / fps.max(1)) as u64);
        let elapsed = t0.elapsed();
        if elapsed < frame_budget {
            std::thread::sleep(frame_budget - elapsed);
        }
    }
}

// ---------------- 系统信息 ----------------

pub fn current_session_id() -> u32 {
    unsafe {
        let mut sid = 0u32;
        let _ = windows::Win32::System::RemoteDesktop::ProcessIdToSessionId(
            windows::Win32::System::Threading::GetCurrentProcessId(),
            &mut sid,
        );
        sid
    }
}

pub fn is_running_as_system() -> bool {
    // SYSTEM 用户 SID = S-1-5-18
    unsafe {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Security::Authorization::ConvertSidToStringSidA;
        use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY};
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        use windows::core::PSTR;

        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        if len == 0 {
            return false;
        }
        let mut buf = vec![0u8; len as usize];
        if GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len).is_err() {
            return false;
        }
        let tu = &*(buf.as_ptr() as *const windows::Win32::Security::TOKEN_USER);
        let mut sid_str = PSTR::null();
        if ConvertSidToStringSidA(tu.User.Sid, &mut sid_str).is_err() {
            return false;
        }
        let s = std::ffi::CStr::from_ptr(sid_str.0 as *const i8).to_string_lossy().to_string();
        let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(sid_str.0 as _));
        s == "S-1-5-18"
    }
}

pub fn get_hostname() -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let mut len = buf.len() as u32;
        if windows::Win32::System::WindowsProgramming::GetComputerNameW(
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok()
        {
            return String::from_utf16_lossy(&buf[..len as usize]);
        }
        "unknown".into()
    }
}
