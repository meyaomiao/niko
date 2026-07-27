//! E5-5: 快照列表与恢复
//!
//! 每次 apply 前由 targets/mod.rs 写一份 `.bak` 到
//! `~/.niko/backups/{target_id}/{timestamp}_{filename}`
//! 本模块提供列出和恢复的 Tauri 命令。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

fn backup_dir_for(target_id: &str) -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("C:\\Users\\default\\AppData\\Roaming"));
    #[cfg(not(target_os = "windows"))]
    let base = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));

    base.join(".niko").join("backups").join(target_id)
}

/// 把 `src` 文件备份到对应目标的备份目录，文件名加上时间戳前缀。
pub fn save_backup(target_id: &str, src: &Path) -> std::io::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    let dir = backup_dir_for(target_id);
    fs::create_dir_all(&dir)?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let fname = src
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let dest = dir.join(format!("{ts}_{fname}"));
    fs::copy(src, &dest)?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct SnapshotEntry {
    pub target_id: String,
    pub filename: String,
    /// Unix timestamp（秒）
    pub timestamp: u64,
    /// 原始文件名（去掉时间戳前缀）
    pub original_name: String,
}

/// 列出某目标的所有备份（按时间倒序）
#[tauri::command]
pub async fn list_snapshots(target_id: String) -> Vec<SnapshotEntry> {
    let dir = backup_dir_for(&target_id);
    if !dir.exists() {
        return vec![];
    }
    let mut entries: Vec<SnapshotEntry> = fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let fname = e.file_name().to_string_lossy().to_string();
            // 格式 {ts}_{original}
            let (ts_str, orig) = fname.split_once('_')?;
            let ts: u64 = ts_str.parse().ok()?;
            Some(SnapshotEntry {
                target_id: target_id.clone(),
                filename: fname.clone(),
                timestamp: ts,
                original_name: orig.to_owned(),
            })
        })
        .collect();

    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    entries
}

/// 从备份文件恢复到目标应用的实际配置路径
#[tauri::command]
pub async fn restore_snapshot(target_id: String, filename: String) -> Result<(), String> {
    let dir = backup_dir_for(&target_id);
    let src = dir.join(&filename);
    if !src.exists() {
        return Err(format!("备份文件不存在: {filename}"));
    }

    // 解析原始文件名，推导目标路径
    let original_name = filename
        .split_once('_')
        .map(|(_, o)| o)
        .unwrap_or(&filename);

    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));

    let dest: PathBuf = match target_id.as_str() {
        "codex" => {
            if original_name == "auth.json" {
                home.join(".codex").join("auth.json")
            } else {
                home.join(".codex").join("config.toml")
            }
        }
        // Claude Desktop 的接入点是它内置 Claude Code 面板读的 ~/.claude/settings.json
        "claude-desktop" => home.join(".claude").join("settings.json"),
        "claude-code" => home.join(".claude").join("settings.json"),
        other => return Err(format!("未知目标: {other}")),
    };

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(&src, &dest).map_err(|e| e.to_string())?;
    Ok(())
}
