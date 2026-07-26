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
