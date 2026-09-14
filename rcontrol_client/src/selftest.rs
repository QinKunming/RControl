//! 自动化自测: 连接本机被控端, 验证 认证/视频/Ping/文件上传下载/剪贴板/鼠标移动
//! 用法: rcontrol_client.exe --selftest [host:port] [password]
//! 退出码 0 = 全部通过

use crate::conn;
use rcontrol_common::proto::*;
use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

fn out(line: &str) {
    println!("{}", line);
    if let Ok(mut g) = LOG.lock() {
        if g.is_none() {
            let p = std::env::temp_dir().join("rcontrol_selftest.txt");
            *g = std::fs::File::create(&p).ok();
        }
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "{}", line);
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

struct Report {
    pass: u32,
    fail: u32,
}

impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: &str) {
        if ok {
            self.pass += 1;
            out(&format!("[通过] {} {}", name, detail));
        } else {
            self.fail += 1;
            out(&format!("[失败] {} {}", name, detail));
        }
    }
}

pub fn run(addr: &str, password: &str) -> i32 {
    out(&format!("== RControl 客户端自测 -> {} ==", addr));
    let mut r = Report { pass: 0, fail: 0 };

    // 1. 错误口令应被拒绝
    match conn::connect(addr, "wrong-password-xyz", Duration::from_secs(8)) {
        Ok(_) => r.check("错误口令拒绝", false, "竟然连接成功!"),
        Err(e) => r.check("错误口令拒绝", e.contains("口令"), &e),
    }

    // 2. 正确口令连接
    let client = match conn::connect(addr, password, Duration::from_secs(8)) {
        Ok(c) => {
            r.check("连接与认证", true, &format!("hostname={} 屏幕={}x{} SYSTEM={}", c.hostname, c.screen_w, c.screen_h, c.system_priv));
            c
        }
        Err(e) => {
            r.check("连接与认证", false, &e);
            out(&format!("== 结果: {} 通过 / {} 失败 ==", r.pass, r.fail));
            return 1;
        }
    };

    // 3. 收到视频帧并解码
    let base = client.frame_seq();
    match client.wait_frame(base, Duration::from_secs(8)) {
        Some(f) => {
            use zune_jpeg::JpegDecoder;
            let mut dec = JpegDecoder::new(&f.jpeg[..]);
            match dec.decode() {
                Ok(px) => r.check("视频帧解码", px.len() >= (f.w as usize) * (f.h as usize) * 3, &format!("{}x{} jpeg={}B", f.w, f.h, f.jpeg.len())),
                Err(e) => r.check("视频帧解码", false, &format!("{:?}", e)),
            }
        }
        None => r.check("视频帧解码", false, "8 秒未收到帧"),
    }

    // 4. Ping/Pong
    client.send_ping();
    let t0 = Instant::now();
    let mut pong = false;
    while t0.elapsed() < Duration::from_secs(5) {
        if client.shared.data.lock().unwrap().latency_ms.is_some() {
            pong = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let lat = client.shared.data.lock().unwrap().latency_ms;
    r.check("Ping/Pong", pong, &format!("延迟 {:?}", lat.map(|l| format!("{}ms", l)).unwrap_or_else(|| "?".into())));

    // 5. 鼠标移动注入 (帧坐标往返: 发送帧坐标, 远端注入后回报的光标帧坐标应一致;
    //    帧与屏幕分辨率不同时能验证 帧坐标<->屏幕坐标 的双向换算)
    let (fw, fh) = {
        let d = client.shared.data.lock().unwrap();
        (d.frame.as_ref().map(|f| f.w).unwrap_or(client.screen_w),
         d.frame.as_ref().map(|f| f.h).unwrap_or(client.screen_h))
    };
    let mut moved = false;
    let mut probe_detail = String::new();
    for (fx, fy) in [(fw as f64 * 0.25, fh as f64 * 0.25), (fw as f64 * 0.75, fh as f64 * 0.25)] {
        let _ = client.send_mouse(fx, fy, MOUSE_MOVE, 0);
        std::thread::sleep(Duration::from_millis(900));
        let base = client.frame_seq();
        if let Some(f) = client.wait_frame(base, Duration::from_secs(3)) {
            let dx = (f.cursor_x as f64 - fx).abs();
            let dy = (f.cursor_y as f64 - fy).abs();
            let ok = f.cursor_visible && dx <= 8.0 && dy <= 8.0;
            probe_detail.push_str(&format!("发({:.0},{:.0})回({},{}) ", fx, fy, f.cursor_x, f.cursor_y));
            if !ok {
                break;
            }
            moved = true;
        } else {
            break;
        }
    }
    r.check("鼠标移动注入", moved, &format!("帧 {}x{} 屏幕 {}x{} {}", fw, fh, client.screen_w, client.screen_h, probe_detail));

    // 6. 文件: 列根目录 / 建目录 / 上传 / 下载 / 校验 / 删除
    let id = client.next_file_id();
    client.send_file_req(id, file_request::Op::List(FileListReq { path: String::new() }));
    match client.wait_file_resp(id, Duration::from_secs(10)) {
        Some(resp) => match resp.op {
            Some(file_response::Op::Entries(en)) => {
                r.check("远程列目录", !en.entries.is_empty(), &format!("{} 项 (含驱动器)", en.entries.len()));
            }
            _ => r.check("远程列目录", false, "响应类型错误"),
        },
        None => r.check("远程列目录", false, "超时"),
    }

    // 在临时目录做上传下载
    let local_tmp = std::env::temp_dir();
    let test_dir = "rcontrol_selftest";
    let remote_dir = format!("C:\\{}", test_dir);
    let id = client.next_file_id();
    client.send_file_req(id, file_request::Op::Mkdir(FileMkdirReq { path: remote_dir.clone() }));
    let mkdir_ok = matches!(
        client.wait_file_resp(id, Duration::from_secs(10)).and_then(|r| r.op),
        Some(file_response::Op::Ack(_))
    );
    r.check("远程建目录", mkdir_ok, &remote_dir);

    // 准备测试数据 2.5MB 伪随机
    let mut data = Vec::with_capacity(2560 * 1024);
    let mut seed: u64 = 0x1234_5678_9abc_def0;
    for _ in 0..2560 * 128 {
        // xorshift
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        data.extend_from_slice(&seed.to_le_bytes());
    }
    data.truncate(2560 * 1024);
    let up_local = local_tmp.join("rcontrol_selftest_up.bin");
    std::fs::write(&up_local, &data).unwrap();
    let remote_file = format!("{}\\self.bin", remote_dir);

    // 上传
    let id = client.next_file_id();
    client.send_file_req(id, file_request::Op::Upload(FileUploadReq { path: remote_file.clone(), size: data.len() as u64 }));
    let ack = matches!(
        client.wait_file_resp(id, Duration::from_secs(15)).and_then(|r| r.op),
        Some(file_response::Op::UploadAck(_))
    );
    let mut up_ok = false;
    if ack {
        let chunk = rcontrol_common::codec::file_chunk_size();
        let total = data.len();
        let mut sent = 0usize;
        while sent < total {
            let end = (sent + chunk).min(total);
            let eof = end >= total;
            client.send_file_req(id, file_request::Op::UploadData(FileUploadData { chunk: data[sent..end].to_vec(), eof }));
            sent = end;
        }
        if total == 0 {
            client.send_file_req(id, file_request::Op::UploadData(FileUploadData { chunk: Vec::new(), eof: true }));
        }
        up_ok = client.wait_file_resp(id, Duration::from_secs(60)).is_some();
    }
    r.check("文件上传", up_ok, &format!("{} 字节 -> {}", data.len(), remote_file));

    // 下载并比较
    let down_local = local_tmp.join("rcontrol_selftest_down.bin");
    let _ = std::fs::remove_file(&down_local);
    let id = client.next_file_id();
    client.send_file_req(id, file_request::Op::Download(FileDownloadReq { path: remote_file.clone() }));
    let mut down_ok = false;
    let mut recv: Vec<u8> = Vec::new();
    if let Some(meta) = client.wait_file_resp(id, Duration::from_secs(15)) {
        if let Some(file_response::Op::DownloadMeta(m)) = meta.op {
            let deadline = Instant::now() + Duration::from_secs(120);
            while Instant::now() < deadline {
                match client.wait_file_resp(id, Duration::from_secs(60)) {
                    Some(resp) => match resp.op {
                        Some(file_response::Op::DownloadData(d)) => {
                            recv.extend_from_slice(&d.chunk);
                            client.send_file_req(id, file_request::Op::DownloadAck(FileDownloadAck { received: recv.len() as u64 }));
                            if d.eof {
                                down_ok = true;
                                break;
                            }
                        }
                        Some(file_response::Op::Error(e)) => {
                            out(&format!("  下载错误: {}", e.msg));
                            break;
                        }
                        _ => {}
                    },
                    None => break,
                }
            }
            let _ = m;
        }
    }
    let content_ok = down_ok && recv == data;
    let _ = std::fs::write(&down_local, &recv);
    r.check("文件下载与校验", content_ok, &format!("{} 字节往返一致", recv.len()));

    // 清理远程
    let id = client.next_file_id();
    client.send_file_req(id, file_request::Op::Remove(FileRemoveReq { path: remote_dir.clone() }));
    let rm_ok = matches!(
        client.wait_file_resp(id, Duration::from_secs(15)).and_then(|r| r.op),
        Some(file_response::Op::Ack(_))
    );
    r.check("远程删除", rm_ok, &remote_dir);
    let _ = std::fs::remove_file(&up_local);
    let _ = std::fs::remove_file(&down_local);

    // 7. 剪贴板 (本机回环: 发送 -> 服务端写入本机剪贴板)
    let marker = format!("rcontrol-selftest-{}", now_ms());
    let _ = client.send(&Message { msg: Some(message::Msg::Clipboard(ClipboardEvent { text: marker.clone() })) });
    let mut clip_ok = false;
    let t0 = Instant::now();
    let mut dbg_log = String::new();
    while t0.elapsed() < Duration::from_secs(5) {
        let got = crate::clip::get_clipboard_text();
        dbg_log.push_str(&format!("t+{:?}: {:?}\n", t0.elapsed(), got));
        if let Some(cur) = got {
            if cur == marker {
                clip_ok = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = std::fs::write(std::env::temp_dir().join("rcontrol_clip_debug.txt"), format!("marker={}\n{}", marker, dbg_log));
    r.check("剪贴板同步", clip_ok, "发送文本应出现在本地剪贴板");

    out(&format!("== 结果: {} 通过 / {} 失败 ==", r.pass, r.fail));
    if r.fail == 0 { 0 } else { 1 }
}
