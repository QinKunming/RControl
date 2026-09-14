//! RControl 主控端
//! 正常启动: 连接对话框 -> 远程画面窗口 (Esc 断开, F9 文件传输)
//! --selftest [host:port] [password]: 自动化自测 (stdout + %TEMP%\rcontrol_selftest.txt)

#![windows_subsystem = "windows"]

mod clip;
mod conn;
mod dlg;
mod files_ui;
mod keys;
mod selftest;
mod view;

/// 本地时间字符串 (不引 chrono)
fn chrono_like_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (h, mi, s) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    format!("{:02}:{:02}:{:02}", h, mi, s)
}

fn main() {
    // windows 子系统看不到 stderr, panic 写日志便于排查
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("[{}] panic: {}\n", chrono_like_now(), info);
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::temp_dir().join("rcontrol_client_panic.log"))
            .and_then(|mut f| std::io::Write::write_all(&mut f, msg.as_bytes()));
    }));

    // DPI 感知: 远程画面按 1:1 物理像素显示, 不做系统级缩放 (否则高 DPI 主控机上会模糊/错位)
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetProcessDPIAware();
    }
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--selftest") {
        let i = args.iter().position(|a| a == "--selftest").unwrap();
        let addr = args.get(i + 1).cloned().unwrap_or_else(|| "127.0.0.1:3333".into());
        let pwd = args
            .get(i + 2)
            .cloned()
            .filter(|p| !p.is_empty())
            .or_else(rcontrol_common::config::local_server_password)
            .unwrap_or_default();
        let code = selftest::run(&addr, &pwd);
        std::process::exit(code);
    }

    // --view host:port password: 跳过对话框直连
    if let Some(i) = args.iter().position(|a| a == "--view") {
        let addr = args.get(i + 1).cloned().unwrap_or_else(|| "127.0.0.1:3333".into());
        let pwd = args
            .get(i + 2)
            .cloned()
            .filter(|p| !p.is_empty())
            .or_else(rcontrol_common::config::local_server_password)
            .unwrap_or_default();
        match conn::connect(&addr, &pwd, std::time::Duration::from_secs(8)) {
            Ok(client) => view::run_viewer(client, false),
            Err(e) => dlg::alert(&format!("连接失败: {}", e)),
        }
        return;
    }

    loop {
        let info = match dlg::run() {
            Some(i) => i,
            None => break,
        };
        match conn::connect(&info.addr, &info.password, std::time::Duration::from_secs(8)) {
            Ok(client) => {
                view::run_viewer(client, info.view_only);
            }
            Err(e) => {
                unsafe { dlg::alert(&format!("连接失败: {}", e)) };
            }
        }
    }
}
