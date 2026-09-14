//! 通信协议消息定义 (手写 prost derive, 无需 protoc)
#![allow(clippy::all)]

/// 服务端握手包: 连接建立后服务端先发送 (明文)
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ServerHello {
    /// 服务端随机数 (16 字节)
    #[prost(bytes = "vec", tag = "1")]
    pub nonce: Vec<u8>,
    #[prost(string, tag = "2")]
    pub hostname: String,
    #[prost(uint32, tag = "3")]
    pub version: u32,
}

/// 客户端认证请求 (明文)
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AuthRequest {
    /// 客户端随机数 (16 字节)
    #[prost(bytes = "vec", tag = "1")]
    pub nonce: Vec<u8>,
    /// HMAC(auth_key, PROOF_LABEL)
    #[prost(bytes = "vec", tag = "2")]
    pub proof: Vec<u8>,
}

/// 认证结果 (明文, 内含服务端证明)
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AuthResponse {
    #[prost(bool, tag = "1")]
    pub ok: bool,
    #[prost(string, tag = "2")]
    pub reason: String,
    /// HMAC(auth_key, SERVER_PROOF_LABEL)
    #[prost(bytes = "vec", tag = "3")]
    pub proof: Vec<u8>,
    /// 服务端屏幕(虚拟屏幕)尺寸
    #[prost(uint32, tag = "4")]
    pub screen_w: u32,
    #[prost(uint32, tag = "5")]
    pub screen_h: u32,
    /// 是否以 SYSTEM 权限运行 (支持锁屏/登录界面/SAS)
    #[prost(bool, tag = "6")]
    pub system_priv: bool,
    /// 当前是否处于安全桌面 (锁屏/登录/UAC)
    #[prost(bool, tag = "7")]
    pub on_secure: bool,
}

/// 视频帧 (JPEG)。jpeg 为空表示仅更新鼠标位置。
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct VideoFrame {
    #[prost(bytes = "vec", tag = "1")]
    pub jpeg: Vec<u8>,
    #[prost(uint32, tag = "2")]
    pub w: u32,
    #[prost(uint32, tag = "3")]
    pub h: u32,
    #[prost(int32, tag = "4")]
    pub cursor_x: i32,
    #[prost(int32, tag = "5")]
    pub cursor_y: i32,
    #[prost(bool, tag = "6")]
    pub cursor_visible: bool,
    /// 是否处于安全桌面
    #[prost(bool, tag = "7")]
    pub on_secure: bool,
}

/// 鼠标事件。mask 见 MOUSE_* 常量, wheel 为滚轮步数(正=向上/右)
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MouseEvent {
    #[prost(double, tag = "1")]
    pub x: f64,
    #[prost(double, tag = "2")]
    pub y: f64,
    #[prost(uint32, tag = "3")]
    pub mask: u32,
    #[prost(int32, tag = "4")]
    pub wheel: i32,
}

pub const MOUSE_MOVE: u32 = 0;
pub const MOUSE_LEFT_DOWN: u32 = 1;
pub const MOUSE_LEFT_UP: u32 = 2;
pub const MOUSE_RIGHT_DOWN: u32 = 3;
pub const MOUSE_RIGHT_UP: u32 = 4;
pub const MOUSE_MIDDLE_DOWN: u32 = 5;
pub const MOUSE_MIDDLE_UP: u32 = 6;
pub const MOUSE_WHEEL_V: u32 = 7;
pub const MOUSE_WHEEL_H: u32 = 8;

/// 键盘事件: scancode 为 PS/2 set-1 扫描码, bit9(0x100)=E0 扩展键, bit10(0x200)=E1
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct KeyboardEvent {
    #[prost(uint32, tag = "1")]
    pub scancode: u32,
    #[prost(bool, tag = "2")]
    pub down: bool,
}

/// 请求服务端发送 SAS (Ctrl+Alt+Del), 仅 SYSTEM 模式有效
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SasEvent {}

/// 剪贴板文本同步
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ClipboardEvent {
    #[prost(string, tag = "1")]
    pub text: String,
}

/// 客户端运行时配置 (0 表示不变)
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SetConfig {
    #[prost(uint32, tag = "1")]
    pub fps: u32,
    #[prost(uint32, tag = "2")]
    pub quality: u32,
    #[prost(uint32, tag = "3")]
    pub max_width: u32,
    #[prost(bool, tag = "4")]
    pub view_only: bool,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Ping {
    #[prost(uint64, tag = "1")]
    pub time: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Pong {
    #[prost(uint64, tag = "1")]
    pub time: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Disconnected {
    #[prost(string, tag = "1")]
    pub reason: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileEntry {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(bool, tag = "2")]
    pub is_dir: bool,
    #[prost(uint64, tag = "3")]
    pub size: u64,
    /// unix 秒
    #[prost(uint64, tag = "4")]
    pub mtime: u64,
    #[prost(bool, tag = "5")]
    pub is_readonly: bool,
}

/// 文件操作请求 (客户端 -> 服务端)
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileRequest {
    #[prost(uint32, tag = "1")]
    pub id: u32,
    #[prost(oneof = "file_request::Op", tags = "11, 12, 13, 14, 15, 16, 17, 18, 19")]
    pub op: Option<file_request::Op>,
}

pub mod file_request {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Op {
        /// 列目录 (path 为空 = 列出磁盘)
        #[prost(message, tag = "11")]
        List(super::FileListReq),
        #[prost(message, tag = "12")]
        Mkdir(super::FileMkdirReq),
        #[prost(message, tag = "13")]
        Remove(super::FileRemoveReq),
        #[prost(message, tag = "14")]
        Rename(super::FileRenameReq),
        /// 请求下载
        #[prost(message, tag = "15")]
        Download(super::FileDownloadReq),
        /// 下载数据确认 (流控)
        #[prost(message, tag = "16")]
        DownloadAck(super::FileDownloadAck),
        /// 请求上传
        #[prost(message, tag = "17")]
        Upload(super::FileUploadReq),
        /// 上传数据
        #[prost(message, tag = "18")]
        UploadData(super::FileUploadData),
        /// 取消当前传输
        #[prost(message, tag = "19")]
        Cancel(super::FileCancel),
    }
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileListReq {
    #[prost(string, tag = "1")]
    pub path: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileMkdirReq {
    #[prost(string, tag = "1")]
    pub path: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileRemoveReq {
    #[prost(string, tag = "1")]
    pub path: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileRenameReq {
    #[prost(string, tag = "1")]
    pub old: String,
    #[prost(string, tag = "2")]
    pub new: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileDownloadReq {
    #[prost(string, tag = "1")]
    pub path: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileDownloadAck {
    #[prost(uint64, tag = "1")]
    pub received: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileUploadReq {
    #[prost(string, tag = "1")]
    pub path: String,
    #[prost(uint64, tag = "2")]
    pub size: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileUploadData {
    #[prost(bytes = "vec", tag = "1")]
    pub chunk: Vec<u8>,
    #[prost(bool, tag = "2")]
    pub eof: bool,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileCancel {}

/// 文件操作响应 (服务端 -> 客户端)
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileResponse {
    #[prost(uint32, tag = "1")]
    pub id: u32,
    #[prost(oneof = "file_response::Op", tags = "11, 12, 13, 14, 15, 16, 17")]
    pub op: Option<file_response::Op>,
}

pub mod file_response {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Op {
        #[prost(message, tag = "11")]
        Error(super::FileError),
        #[prost(message, tag = "12")]
        Entries(super::FileEntries),
        /// 下载文件元信息 (即将开始传数据)
        #[prost(message, tag = "13")]
        DownloadMeta(super::FileDownloadMeta),
        #[prost(message, tag = "14")]
        DownloadData(super::FileDownloadData),
        /// 上传就绪确认
        #[prost(message, tag = "15")]
        UploadAck(super::FileUploadAck),
        #[prost(message, tag = "16")]
        Done(super::FileDone),
        /// 通用确认 (删除/重命名/建目录成功)
        #[prost(message, tag = "17")]
        Ack(super::FileAck),
    }
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileError {
    #[prost(string, tag = "1")]
    pub msg: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileEntries {
    #[prost(string, tag = "1")]
    pub path: String,
    #[prost(message, repeated, tag = "2")]
    pub entries: Vec<FileEntry>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileDownloadMeta {
    #[prost(string, tag = "1")]
    pub path: String,
    #[prost(uint64, tag = "2")]
    pub size: u64,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileDownloadData {
    #[prost(bytes = "vec", tag = "1")]
    pub chunk: Vec<u8>,
    #[prost(uint64, tag = "2")]
    pub offset: u64,
    #[prost(bool, tag = "3")]
    pub eof: bool,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileUploadAck {}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileDone {
    #[prost(string, tag = "1")]
    pub msg: String,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileAck {}

/// 顶层消息信封
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Message {
    #[prost(
        oneof = "message::Msg",
        tags = "1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15"
    )]
    pub msg: Option<message::Msg>,
}

pub mod message {
    #[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Msg {
        #[prost(message, tag = "1")]
        VideoFrame(super::VideoFrame),
        #[prost(message, tag = "2")]
        Mouse(super::MouseEvent),
        #[prost(message, tag = "3")]
        Keyboard(super::KeyboardEvent),
        #[prost(message, tag = "4")]
        Sas(super::SasEvent),
        #[prost(message, tag = "5")]
        Clipboard(super::ClipboardEvent),
        #[prost(message, tag = "6")]
        SetConfig(super::SetConfig),
        #[prost(message, tag = "7")]
        Ping(super::Ping),
        #[prost(message, tag = "8")]
        Pong(super::Pong),
        #[prost(message, tag = "9")]
        Disconnected(super::Disconnected),
        #[prost(message, tag = "10")]
        FileRequest(super::FileRequest),
        #[prost(message, tag = "11")]
        FileResponse(super::FileResponse),
        #[prost(message, tag = "12")]
        ServerHello(super::ServerHello),
        #[prost(message, tag = "13")]
        AuthRequest(super::AuthRequest),
        #[prost(message, tag = "14")]
        AuthResponse(super::AuthResponse),
        #[prost(message, tag = "15")]
        None(super::Empty),
    }
}

/// 空占位
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Empty {}
