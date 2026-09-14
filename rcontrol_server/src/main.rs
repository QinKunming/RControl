//! RControl 被控端
//! 默认启动设置 GUI; --service 为 Windows 服务模式; --worker 为工作进程 (由服务或 GUI 拉起)

#![windows_subsystem = "windows"]

mod capture;
mod clip;
mod desktop;
mod encode;
mod files;
mod gui;
mod input;
mod logger;
mod service;
mod service_manager;
mod session;
mod worker;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);

    if has("--service") {
        service::run();
    } else if has("--worker") {
        worker::run(has("--user"));
    } else {
        gui::run();
    }
}
