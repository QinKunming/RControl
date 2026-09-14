//! 服务安装/卸载/查询 + 防火墙规则

use std::ffi::OsString;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use windows_service::service::{
    ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceState, ServiceStartType, ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

pub const SERVICE_NAME: &str = "rcontrol";
const FW_RULE_NAME: &str = "RControl";

pub fn install_service() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)
        .map_err(|e| format!("打开服务管理器失败: {}", e))?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("RControl 远程控制服务"),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec![OsString::from("--service")],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };
    manager
        .create_service(&info, ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::DELETE)
        .map_err(|e| format!("创建服务失败: {}", e))?;
    Ok(())
}

pub fn start_service() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| e.to_string())?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::START)
        .map_err(|e| format!("打开服务失败: {}", e))?;
    service.start(&[] as &[&str]).map_err(|e| format!("启动服务失败: {}", e))?;
    Ok(())
}

pub fn stop_service() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| e.to_string())?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP)
        .map_err(|e| format!("打开服务失败: {}", e))?;
    service.stop().map_err(|e| format!("停止服务失败: {}", e))?;
    Ok(())
}

pub fn uninstall_service() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| e.to_string())?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE)
        .map_err(|e| format!("服务未安装? {}", e))?;
    let _ = service.stop();
    service.delete().map_err(|e| format!("删除服务失败: {}", e))?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
pub enum ServiceStateInfo {
    NotInstalled,
    Stopped,
    StartPending,
    Running,
    Other,
}

pub fn query_service() -> ServiceStateInfo {
    let manager = match ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT) {
        Ok(m) => m,
        Err(_) => return ServiceStateInfo::NotInstalled,
    };
    let service = match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
        Ok(s) => s,
        Err(_) => return ServiceStateInfo::NotInstalled,
    };
    match service.query_status() {
        Ok(st) => match st.current_state {
            ServiceState::Stopped => ServiceStateInfo::Stopped,
            ServiceState::StartPending => ServiceStateInfo::StartPending,
            ServiceState::Running => ServiceStateInfo::Running,
            _ => ServiceStateInfo::Other,
        },
        Err(_) => ServiceStateInfo::NotInstalled,
    }
}

/// 添加防火墙入站规则 (TCP 端口)
pub fn add_firewall_rule(port: u16) -> Result<(), String> {
    let rule = format!(
        "advfirewall firewall add rule name=\"{}\" dir=in action=allow protocol=TCP localport={}",
        FW_RULE_NAME, port
    );
    run_netsh(&rule)
}

pub fn remove_firewall_rule() -> Result<(), String> {
    run_netsh(&format!("advfirewall firewall delete rule name=\"{}\"", FW_RULE_NAME))
}

fn run_netsh(args: &str) -> Result<(), String> {
    let argv: Vec<&str> = args.split_whitespace().collect();
    let out = Command::new("netsh.exe")
        .args(&argv)
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("执行 netsh 失败: {}", e))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    Ok(())
}

/// 供 GUI 启动临时工作进程
pub fn spawn_user_worker() -> Result<u32, String> {
    let exe: PathBuf = std::env::current_exe().map_err(|e| e.to_string())?;
    Command::new(exe)
        .arg("--worker")
        .arg("--user")
        .creation_flags(0x0800_0000)
        .spawn()
        .map(|c| c.id())
        .map_err(|e| format!("启动失败: {}", e))
}
