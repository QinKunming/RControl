//! 会话内进程生成: 服务(SESSION 0)在控制台会话中以 SYSTEM 身份拉起工作进程
//! 方法: 找到目标会话中的 winlogon.exe, 复用其令牌 CreateProcessAsUser (rustdesk/UltraVNC 同款)

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, STILL_ACTIVE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::RemoteDesktop::{ProcessIdToSessionId, WTSGetActiveConsoleSessionId};
use windows::Win32::Security::TOKEN_ALL_ACCESS;
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, GetExitCodeProcess, OpenProcess, OpenProcessToken, TerminateProcess,
    PROCESS_INFORMATION, PROCESS_ALL_ACCESS, STARTUPINFOW,
};

pub fn console_session_id() -> u32 {
    unsafe { WTSGetActiveConsoleSessionId() }
}

/// 在指定会话中查找进程 pid (如 winlogon.exe), 找不到返回 0
fn find_process_in_session(name_lower: &str, session: u32) -> u32 {
    unsafe {
        let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return 0,
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = 0u32;
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let exe = String::from_utf16_lossy(&entry.szExeFile);
                let exe = exe.trim_end_matches('\0').to_lowercase();
                if exe == name_lower {
                    // PROCESSENTRY32W 无会话字段, 逐个查询
                    let mut sid = 0u32;
                    let ok = ProcessIdToSessionId(entry.th32ProcessID, &mut sid);
                    if ok.is_ok() && sid == session {
                        found = entry.th32ProcessID;
                        break;
                    }
                }
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        found
    }
}

/// 以目标会话 winlogon 令牌 (SYSTEM) 在该会话内启动工作进程, 返回进程句柄
pub fn spawn_worker_in_session(session: u32) -> Result<HANDLE, String> {
    unsafe {
        let mut pid = find_process_in_session("winlogon.exe", session);
        if pid == 0 {
            // 回退: csrss.exe (SYSTEM 进程, 始终存在于活动会话)
            pid = find_process_in_session("csrss.exe", session);
        }
        if pid == 0 {
            return Err(format!("session {} 中找不到 winlogon.exe/csrss.exe", session));
        }

        let hproc = OpenProcess(PROCESS_ALL_ACCESS, false, pid).map_err(|e| format!("OpenProcess({}): {}", pid, e))?;
        let mut token = HANDLE::default();
        OpenProcessToken(hproc, TOKEN_ALL_ACCESS, &mut token).map_err(|e| format!("OpenProcessToken: {}", e))?;
        let _ = CloseHandle(hproc);

        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut cmd: Vec<u16> = format!("\"{}\" --worker", exe.display())
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut si = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        // 不设 lpDesktop (继承 winlogon 环境), DETACHED_PROCESS, 参考 rustdesk LaunchProcessWin
        let mut pi = PROCESS_INFORMATION::default();
        let created = CreateProcessAsUserW(
            token,
            PCWSTR::null(),
            PWSTR(cmd.as_mut_ptr()),
            None,
            None,
            false,
            windows::Win32::System::Threading::DETACHED_PROCESS,
            None,
            PCWSTR::null(),
            &si,
            &mut pi,
        );
        let _ = CloseHandle(token);
        if let Err(e) = created {
            return Err(format!("CreateProcessAsUserW: {}", e));
        }
        let _ = CloseHandle(pi.hThread);
        crate::logger::log_line("INFO", &format!("spawned worker pid {} in session {}", pi.dwProcessId, session));
        Ok(pi.hProcess)
    }
}

pub fn process_alive(h: HANDLE) -> bool {
    unsafe {
        let mut code = 0u32;
        if GetExitCodeProcess(h, &mut code).is_err() {
            return false;
        }
        code == STILL_ACTIVE.0 as u32
    }
}

pub fn kill_process(h: HANDLE) {
    unsafe {
        let _ = TerminateProcess(h, 0);
        let _ = CloseHandle(h);
    }
}
