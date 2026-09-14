//! 文件传输服务端

use rcontrol_common::codec;
use rcontrol_common::proto::*;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

/// 传输控制指令 (从主网络循环路由到传输线程)
pub enum FileCtl {
    Ack(u64),
    Data(Vec<u8>, bool),
    Cancel,
}

pub type Transfers = Arc<Mutex<HashMap<u32, Sender<FileCtl>>>>;

/// 发消息回调 (阻塞发送, 线程安全, 可克隆)
pub type Respond = Arc<dyn Fn(Message) + Send + Sync>;

fn respond_file(respond: &Respond, id: u32, op: file_response::Op) {
    respond(Message { msg: Some(message::Msg::FileResponse(FileResponse { id, op: Some(op) })) });
}

/// 处理一个文件请求 (在新线程中运行)
pub fn handle_request(req: FileRequest, respond: Respond, transfers: Transfers) {
    let id = req.id;
    let op = match req.op {
        Some(o) => o,
        None => return,
    };
    match op {
        file_request::Op::List(r) => {
            std::thread::spawn(move || list_dir(id, &r.path, &respond));
        }
        file_request::Op::Mkdir(r) => {
            std::thread::spawn(move || {
                let msg = match fs::create_dir_all(&r.path) {
                    Ok(()) => file_response::Op::Ack(FileAck {}),
                    Err(e) => file_response::Op::Error(FileError { msg: e.to_string() }),
                };
                respond_file(&respond, id, msg);
            });
        }
        file_request::Op::Remove(r) => {
            std::thread::spawn(move || {
                let res = if Path::new(&r.path).is_dir() {
                    fs::remove_dir_all(&r.path)
                } else {
                    fs::remove_file(&r.path)
                };
                let msg = match res {
                    Ok(()) => file_response::Op::Ack(FileAck {}),
                    Err(e) => file_response::Op::Error(FileError { msg: e.to_string() }),
                };
                respond_file(&respond, id, msg);
            });
        }
        file_request::Op::Rename(r) => {
            std::thread::spawn(move || {
                let msg = match fs::rename(&r.old, &r.new) {
                    Ok(()) => file_response::Op::Ack(FileAck {}),
                    Err(e) => file_response::Op::Error(FileError { msg: e.to_string() }),
                };
                respond_file(&respond, id, msg);
            });
        }
        file_request::Op::Download(r) => {
            std::thread::spawn(move || download(id, &r.path, respond, transfers));
        }
        file_request::Op::Upload(r) => {
            std::thread::spawn(move || upload(id, &r.path, r.size, respond, transfers));
        }
        // Ack/Data/Cancel 属于已存在传输, 不在此处理
        _ => {}
    }
}

/// 将控制指令路由给对应传输线程
pub fn route_ctl(transfers: &Transfers, id: u32, ctl: FileCtl) -> bool {
    if let Ok(map) = transfers.lock() {
        if let Some(tx) = map.get(&id) {
            return tx.send(ctl).is_ok();
        }
    }
    false
}

fn mtime_unix(md: &fs::Metadata) -> u64 {
    md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn list_dir(id: u32, path: &str, respond: &Respond) {
    let entries = if path.is_empty() {
        // 列出磁盘
        unsafe {
            let drives = windows::Win32::Storage::FileSystem::GetLogicalDrives();
            let mut list = Vec::new();
            for i in 0..26u32 {
                if drives & (1 << i) != 0 {
                    let name = format!("{}:\\", (b'A' + i as u8) as char);
                    list.push(FileEntry { name, is_dir: true, size: 0, mtime: 0, is_readonly: false });
                }
            }
            list
        }
    } else {
        match fs::read_dir(path) {
            Ok(rd) => {
                let mut list: Vec<FileEntry> = Vec::new();
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.is_empty() {
                        continue;
                    }
                    match e.metadata() {
                        Ok(md) => list.push(FileEntry {
                            name,
                            is_dir: md.is_dir(),
                            size: if md.is_dir() { 0 } else { md.len() },
                            mtime: mtime_unix(&md),
                            is_readonly: md.permissions().readonly(),
                        }),
                        Err(_) => continue,
                    }
                }
                list.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                list
            }
            Err(e) => {
                respond_file(respond, id, file_response::Op::Error(FileError { msg: format!("无法列出 {}: {}", path, e) }));
                return;
            }
        }
    };
    respond_file(respond, id, file_response::Op::Entries(FileEntries { path: path.to_string(), entries }));
}

struct TransferGuard<'a> {
    transfers: &'a Transfers,
    id: u32,
}

impl<'a> Drop for TransferGuard<'a> {
    fn drop(&mut self) {
        if let Ok(mut map) = self.transfers.lock() {
            map.remove(&self.id);
        }
    }
}

/// 已发送未确认的字节数 (流控窗口)
const WINDOW: u64 = 4 * 1024 * 1024;

fn download(id: u32, path: &str, respond: Respond, transfers: Transfers) {
    let _guard = TransferGuard { transfers: &transfers, id };
    let (tx, rx) = std::sync::mpsc::channel::<FileCtl>();
    if let Ok(mut map) = transfers.lock() {
        map.insert(id, tx);
    } else {
        return;
    }
    let mut f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            respond_file(&respond, id, file_response::Op::Error(FileError { msg: format!("无法打开 {}: {}", path, e) }));
            return;
        }
    };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    respond_file(&respond, id, file_response::Op::DownloadMeta(FileDownloadMeta { path: path.to_string(), size }));

    let chunk_size = codec::file_chunk_size();
    let mut buf = vec![0u8; chunk_size];
    let mut outstanding: u64 = 0;
    let mut offset: u64 = 0;
    loop {
        // 流控: 等待客户端确认
        while outstanding >= WINDOW {
            match rx.recv_timeout(std::time::Duration::from_secs(60)) {
                Ok(FileCtl::Ack(received)) => {
                    outstanding = outstanding.saturating_sub(received);
                }
                Ok(FileCtl::Cancel) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    respond_file(&respond, id, file_response::Op::Error(FileError { msg: "已取消".into() }));
                    return;
                }
                Ok(FileCtl::Data(_, _)) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    respond_file(&respond, id, file_response::Op::Error(FileError { msg: "传输超时".into() }));
                    return;
                }
            }
        }
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                respond_file(&respond, id, file_response::Op::DownloadData(FileDownloadData {
                    chunk: buf[..n].to_vec(),
                    offset,
                    eof: false,
                }));
                offset += n as u64;
                outstanding += n as u64;
            }
            Err(e) => {
                respond_file(&respond, id, file_response::Op::Error(FileError { msg: format!("读取失败: {}", e) }));
                return;
            }
        }
    }
    // 排空剩余确认
    while outstanding > 0 {
        match rx.recv_timeout(std::time::Duration::from_secs(60)) {
            Ok(FileCtl::Ack(received)) => outstanding = outstanding.saturating_sub(received),
            _ => break,
        }
    }
    respond_file(&respond, id, file_response::Op::DownloadData(FileDownloadData { chunk: Vec::new(), offset, eof: true }));
}

fn upload(id: u32, path: &str, size: u64, respond: Respond, transfers: Transfers) {
    let _guard = TransferGuard { transfers: &transfers, id };
    let (tx, rx) = std::sync::mpsc::channel::<FileCtl>();
    if let Ok(mut map) = transfers.lock() {
        map.insert(id, tx);
    } else {
        return;
    }
    let mut f = match fs::File::create(path) {
        Ok(f) => f,
        Err(e) => {
            respond_file(&respond, id, file_response::Op::Error(FileError { msg: format!("无法创建 {}: {}", path, e) }));
            return;
        }
    };
    respond_file(&respond, id, file_response::Op::UploadAck(FileUploadAck {}));
    let mut received: u64 = 0;
    loop {
        match rx.recv_timeout(std::time::Duration::from_secs(120)) {
            Ok(FileCtl::Data(chunk, eof)) => {
                if !chunk.is_empty() {
                    if let Err(e) = f.write_all(&chunk) {
                        respond_file(&respond, id, file_response::Op::Error(FileError { msg: format!("写入失败: {}", e) }));
                        return;
                    }
                    received += chunk.len() as u64;
                }
                if eof {
                    break;
                }
            }
            Ok(FileCtl::Cancel) | Ok(FileCtl::Ack(_)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = fs::remove_file(path);
                respond_file(&respond, id, file_response::Op::Error(FileError { msg: "已取消".into() }));
                return;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                respond_file(&respond, id, file_response::Op::Error(FileError { msg: "传输超时".into() }));
                return;
            }
        }
    }
    respond_file(
        &respond,
        id,
        file_response::Op::Done(FileDone { msg: format!("上传完成 {} / {} 字节", received, size) }),
    );
}
