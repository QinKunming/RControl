//! Windows 服务模式: SCM 接入 + 看门狗
//! 服务本身(SESSION 0/SYSTEM)不做捕获, 只负责在活动控制台会话中维持一个工作进程;
//! 用户登录/注销/锁屏都会引起会话或输入桌面变化, 由工作进程自行适配, 注销导致进程
//! 被杀时由本看门狗重新拉起

use std::sync::mpsc;
use std::time::Duration;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

const SERVICE_NAME: &str = "rcontrol";

define_windows_service!(ffi_service_main, service_main);

pub fn run() {
    crate::logger::init("service");
    crate::logger::log_line("INFO", "service dispatching");
    if let Err(e) = service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        crate::logger::log_line("ERROR", &format!("service_dispatcher start failed: {}", e));
    }
}

fn service_main(_args: Vec<std::ffi::OsString>) {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let event_handler = move |control_event: ServiceControl| match control_event {
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        ServiceControl::Stop | ServiceControl::Preshutdown | ServiceControl::Shutdown => {
            let _ = stop_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        _ => ServiceControlHandlerResult::NotImplemented,
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler);

    let next_status = |state: ServiceState, exit: ServiceExitCode| {
        if let Ok(h) = &status_handle {
            let _ = h.set_service_status(ServiceStatus {
                service_type: windows_service::service::ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted: if state == ServiceState::Running {
                    ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
                } else {
                    ServiceControlAccept::empty()
                },
                exit_code: exit,
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            });
        }
    };

    next_status(ServiceState::StartPending, ServiceExitCode::NO_ERROR);
    crate::logger::log_line("INFO", &format!("service started, pid {}", std::process::id()));
    next_status(ServiceState::Running, ServiceExitCode::NO_ERROR);

    // 看门狗循环
    let mut worker: Option<windows::Win32::Foundation::HANDLE> = None;
    let mut watched_session = u32::MAX;
    let mut spawn_fail = 0u32;
    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        let console = crate::session::console_session_id();
        let session_changed = console != watched_session && console != 0xFFFFFFFF;

        if session_changed {
            if let Some(h) = worker.take() {
                crate::logger::log_line("INFO", "session changed, killing old worker");
                crate::session::kill_process(h);
            }
            watched_session = console;
        }

        let need_spawn = match worker {
            None => true,
            Some(h) => !crate::session::process_alive(h),
        };

        if need_spawn {
            // 工作进程主动退出(配置变化)后立即重启会造成日志噪音, 稍作退避
            if spawn_fail > 0 {
                std::thread::sleep(Duration::from_millis((500 * spawn_fail.min(10)) as u64));
            }
            match crate::session::spawn_worker_in_session(watched_session) {
                Ok(h) => {
                    worker = Some(h);
                    spawn_fail = 0;
                }
                Err(e) => {
                    spawn_fail += 1;
                    if spawn_fail == 1 || spawn_fail % 20 == 0 {
                        crate::logger::log_line("ERROR", &format!("spawn worker failed: {}", e));
                    }
                }
            }
        }

        std::thread::sleep(Duration::from_millis(1500));
    }

    if let Some(h) = worker.take() {
        crate::session::kill_process(h);
    }
    next_status(ServiceState::Stopped, ServiceExitCode::NO_ERROR);
    crate::logger::log_line("INFO", "service stopped");
}
