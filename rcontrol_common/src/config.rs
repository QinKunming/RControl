//! 配置文件读写

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ServerConfig {
    /// 监听端口 (可自定义)
    pub port: u16,
    /// 访问口令, 为空时自动生成
    pub password: String,
    /// 最大帧率
    pub max_fps: u32,
    /// JPEG 质量 1-100
    pub quality: u32,
    /// 编码最大宽度 (超过则缩放, 0=原始)
    pub max_width: u32,
    pub allow_file: bool,
    pub allow_clipboard: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            port: 3333,
            password: String::new(),
            max_fps: 30,
            quality: 70,
            max_width: 0,
            allow_file: true,
            allow_clipboard: true,
        }
    }
}

/// %ProgramData%\rcontrol
pub fn server_data_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("rcontrol")
}

pub fn server_config_path() -> PathBuf {
    server_data_dir().join("config.toml")
}

pub fn load_server_config() -> ServerConfig {
    let path = server_config_path();
    if let Ok(text) = fs::read_to_string(&path) {
        if let Ok(mut cfg) = toml::from_str::<ServerConfig>(&text) {
            if cfg.password.is_empty() {
                cfg.password = crate::crypto::generate_password(10);
                let _ = save_server_config(&cfg);
            }
            return cfg;
        }
    }
    let mut cfg = ServerConfig::default();
    cfg.password = crate::crypto::generate_password(10);
    let _ = save_server_config(&cfg);
    cfg
}

pub fn save_server_config(cfg: &ServerConfig) -> std::io::Result<()> {
    let dir = server_data_dir();
    fs::create_dir_all(&dir)?;
    let text = toml::to_string_pretty(cfg).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    // 原子写: 先写临时文件再替换
    let tmp = dir.join("config.toml.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, server_config_path())
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct ClientConfig {
    /// 上次连接的 "host:port"
    pub last_addr: String,
    /// 画质档 0=流畅 1=均衡 2=高清
    pub quality_preset: u32,
}

/// 读取本机被控端口令 (同机自测用)
pub fn local_server_password() -> Option<String> {
    let text = fs::read_to_string(server_config_path()).ok()?;
    toml::from_str::<ServerConfig>(&text).ok().map(|c| c.password)
}

/// %APPDATA%\rcontrol
pub fn client_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("rcontrol")
}

pub fn load_client_config() -> ClientConfig {
    let path = client_data_dir().join("client.toml");
    if let Ok(text) = fs::read_to_string(&path) {
        if let Ok(cfg) = toml::from_str::<ClientConfig>(&text) {
            return cfg;
        }
    }
    ClientConfig::default()
}

pub fn save_client_config(cfg: &ClientConfig) {
    let dir = client_data_dir();
    if fs::create_dir_all(&dir).is_ok() {
        let _ = fs::write(dir.join("client.toml"), toml::to_string_pretty(cfg).unwrap_or_default());
    }
}
