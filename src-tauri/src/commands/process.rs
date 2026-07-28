//! E8-2: 目标应用运行状态探测

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
    ProcessStatus { target_id, running: false, pid: None }
}

/// 关闭并重新启动目标应用。配置只在应用启动时读取一次，所以改完配置必须重启才生效。
///
/// 先请求应用正常退出（保留它自己的会话/草稿处理），轮询确认进程消失后再启动；
/// 超时仍在运行则直接放弃启动并报错，避免出现两个实例。
#[tauri::command]
pub async fn restart_target(target_id: String) -> Result<String, String> {
    let path = crate::targets::app_launch_path(&target_id)
        .ok_or_else(|| "未找到已安装的应用，请先安装后再试".to_owned())?;

    let was_running = check_process(target_id.clone()).await.running;
    if was_running {
        quit_app(&target_id, &path)?;
        let mut quit = false;
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if !check_process(target_id.clone()).await.running {
                quit = true;
                break;
            }
        }
        if !quit {
            return Err("应用未能退出，请手动关闭后再点启动".to_owned());
        }
    }

    launch_app(&path)?;
    Ok(if was_running { "已重启".to_owned() } else { "已启动".to_owned() })
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

    targets.iter().map(|(id, keywords)| {
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
    }).collect()
}
