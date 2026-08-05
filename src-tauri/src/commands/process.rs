//! E8-2: 目标应用运行状态探测

use crate::commands::safe_error::SafeCommandError;
use serde::Serialize;
use sysinfo::System;

/// 进程名关键词映射（部分匹配）
fn process_keywords(target_id: &str) -> &'static [&'static str] {
    match target_id {
        "codex" => &["codex"],
        "claude-desktop" => &["Claude", "claude"],
        "claude-code" => &["claude_code", "claude-code", "claude"],
        _ => &[],
    }
}

#[derive(Debug, Serialize)]
pub struct ProcessStatus {
    pub target_id: String,
    pub running: bool,
    pub pid: Option<u32>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct RestartOutcome {
    pub status: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CloseOutcome {
    pub status: &'static str,
    pub message: &'static str,
}

fn restart_after_close<P, L>(
    prepare: P,
    launch: L,
    was_running: bool,
) -> Result<RestartOutcome, SafeCommandError>
where
    P: FnOnce() -> Result<(), SafeCommandError>,
    L: FnOnce() -> Result<(), ()>,
{
    prepare()?;
    if launch().is_err() {
        return Ok(RestartOutcome {
            status: "applied_needs_manual_open",
            message: "设置已保存，请手动打开应用。",
        });
    }
    Ok(RestartOutcome {
        status: "applied",
        message: if was_running {
            "已重启。"
        } else {
            "已启动。"
        },
    })
}

fn wait_for_app_exit<F>(mut is_running: F, attempts: usize, delay: std::time::Duration) -> bool
where
    F: FnMut() -> bool,
{
    for _ in 0..attempts {
        std::thread::sleep(delay);
        if !is_running() {
            return true;
        }
    }
    false
}

#[tauri::command]
pub async fn check_process(target_id: String) -> ProcessStatus {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let keywords = process_keywords(&target_id);
    for (pid, proc) in sys.processes() {
        let name = proc.name().to_string_lossy().to_lowercase();
        if keywords.iter().any(|k| name.contains(&k.to_lowercase())) {
            return ProcessStatus {
                target_id,
                running: true,
                pid: Some(pid.as_u32()),
            };
        }
    }
    ProcessStatus {
        target_id,
        running: false,
        pid: None,
    }
}

/// 关闭并重新启动目标应用。配置只在应用启动时读取一次，所以改完配置必须重启才生效。
///
/// 先请求应用正常退出（保留它自己的会话/草稿处理），轮询确认进程消失后再启动；
/// 超时仍在运行则直接放弃启动并报错，避免出现两个实例。
#[tauri::command]
pub async fn restart_target(target_id: String) -> Result<RestartOutcome, SafeCommandError> {
    let path = crate::targets::app_launch_path(&target_id)
        .ok_or_else(SafeCommandError::invalid_request)?;

    let was_running = app_process_running(&path);
    if was_running {
        quit_app(&target_id, &path).map_err(|_| SafeCommandError::busy())?;
        if !wait_for_app_exit(
            || app_process_running(&path),
            20,
            std::time::Duration::from_millis(250),
        ) {
            return Err(SafeCommandError::busy());
        }
    }

    restart_after_close(
        || {
            if target_id == "codex" {
                crate::commands::codex_sessions::prepare_codex_session_restart().map(|_| ())
            } else {
                Ok(())
            }
        },
        || launch_app(&path).map_err(|_| ()),
        was_running,
    )
}

/// 请求目标应用正常退出，不改动应用配置或会话内容。
#[tauri::command]
pub async fn close_target(target_id: String) -> Result<CloseOutcome, SafeCommandError> {
    let path = crate::targets::app_launch_path(&target_id)
        .ok_or_else(SafeCommandError::invalid_request)?;

    if !app_process_running(&path) {
        return Ok(CloseOutcome {
            status: "not_running",
            message: "应用未运行，可以重新检查。",
        });
    }

    quit_app(&target_id, &path).map_err(|_| SafeCommandError::busy())?;
    if !wait_for_app_exit(
        || app_process_running(&path),
        20,
        std::time::Duration::from_millis(250),
    ) {
        return Err(SafeCommandError::busy());
    }

    Ok(CloseOutcome {
        status: "closed",
        message: "应用已关闭，可以重新检查。",
    })
}

/// 某个安装路径下是否有进程在跑。按可执行文件路径判断，不能用进程名关键词：
/// Claude 桌面端与 claude CLI 的进程名都含 "claude"，名字匹配会把 CLI 误判成桌面端还没退出。
fn app_process_running(path: &std::path::Path) -> bool {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes()
        .values()
        .any(|proc| proc.exe().is_some_and(|exe| is_app_executable(exe, path)))
}

fn is_app_executable(exe: &std::path::Path, path: &std::path::Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        // Electron 的 crashpad 等 helper 退出主应用后仍可能驻留，不能阻塞重新启动。
        exe.parent() == Some(path.join("Contents/MacOS").as_path())
    }
    #[cfg(not(target_os = "macos"))]
    {
        exe == path
    }
}

fn quit_app(target_id: &str, path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = target_id;
        // 用 App 显示名请求正常退出；osascript 比 kill 温和，能让应用保存自己的状态
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("无法解析应用名")?;
        let status = std::process::Command::new("osascript")
            .args(["-e", &format!("quit app \"{name}\"")])
            .status()
            .map_err(|e| format!("请求退出失败：{e}"))?;
        if !status.success() {
            return Err("请求应用退出失败，请手动关闭后再点启动".to_owned());
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let _ = target_id;
        let exe = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("无法解析可执行文件名")?;
        std::process::Command::new("taskkill")
            .args(["/IM", exe])
            .status()
            .map_err(|e| format!("请求退出失败：{e}"))?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (target_id, path);
        Err("当前平台不支持一键重启".to_owned())
    }
}

fn launch_app(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-a")
            .arg(path)
            .status()
            .map_err(|e| format!("启动失败：{e}"))?
            .success()
            .then_some(())
            .ok_or_else(|| "启动失败，请手动打开应用".to_owned())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("启动失败：{e}"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        Err("当前平台不支持一键启动".to_owned())
    }
}

/// 批量检测所有目标的运行状态
#[tauri::command]
pub async fn check_all_processes() -> Vec<ProcessStatus> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let targets = [
        ("codex", process_keywords("codex")),
        ("claude-desktop", process_keywords("claude-desktop")),
        ("claude-code", process_keywords("claude-code")),
    ];

    targets
        .iter()
        .map(|(id, keywords)| {
        let mut found_pid: Option<u32> = None;
        for (pid, proc) in sys.processes() {
            let name = proc.name().to_string_lossy().to_lowercase();
            if keywords.iter().any(|k| name.contains(&k.to_lowercase())) {
                found_pid = Some(pid.as_u32());
                break;
            }
        }
        ProcessStatus {
            target_id: id.to_string(),
            running: found_pid.is_some(),
            pid: found_pid,
        }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_failure_reports_committed_manual_open_state() {
        let outcome = restart_after_close(|| Ok(()), || Err(()), true).unwrap();
        assert_eq!(outcome.status, "applied_needs_manual_open");
    }

    #[test]
    fn prepare_failure_never_launches() {
        let launched = std::cell::Cell::new(false);
        let result = restart_after_close(
            || Err(SafeCommandError::busy()),
            || {
                launched.set(true);
                Ok(())
            },
            true,
        );
        assert!(result.is_err());
        assert!(!launched.get());
    }

    #[test]
    fn exit_timeout_stops_before_prepare_and_launch() {
        assert!(!wait_for_app_exit(|| true, 2, std::time::Duration::ZERO));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn app_executable_ignores_macos_bundle_helpers() {
        let app = std::path::Path::new("/Applications/ChatGPT.app");

        assert!(is_app_executable(
            std::path::Path::new("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"),
            app,
        ));
        assert!(!is_app_executable(
            std::path::Path::new(
                "/Applications/ChatGPT.app/Contents/Frameworks/Codex Framework.framework/Helpers/browser_crashpad_handler",
            ),
            app,
        ));
        assert!(!is_app_executable(
            std::path::Path::new(
                "/Applications/ChatGPT.app/Contents/Frameworks/Codex Helper.app/Contents/MacOS/Codex Helper",
            ),
            app,
        ));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn app_executable_matches_exact_launch_path() {
        let app = std::path::Path::new(r"C:\Users\me\AppData\Local\ChatGPT\ChatGPT.exe");

        assert!(is_app_executable(app, app));
        assert!(!is_app_executable(
            std::path::Path::new(r"C:\Users\me\AppData\Local\ChatGPT\helper.exe"),
            app,
        ));
    }
}
