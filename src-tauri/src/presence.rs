//! 匿名使用心跳：只上报本机安装标识、平台和版本。
//! 失败一律静默，不得影响登录或接入。

use std::fs;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const HEARTBEAT_URL: &str = "https://niko-ai.cc/api/presence/heartbeat";
const STATS_URL: &str = "https://niko-ai.cc/api/presence/stats";
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct UsageStats {
    pub online: UsageOnline,
    pub downloads: UsageDownloads,
    pub generated_at: String,
    pub window_seconds: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct UsageOnline {
    pub total: u64,
    pub macos: u64,
    pub windows: u64,
    pub linux: u64,
    #[serde(default)]
    pub by_version: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct UsageDownloads {
    pub installers: u64,
    pub macos: u64,
    pub windows: u64,
    #[serde(default)]
    pub by_release: Vec<UsageReleaseDownloads>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct UsageReleaseDownloads {
    pub tag: String,
    pub installers: u64,
    pub macos: u64,
    pub windows: u64,
}

fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

fn install_id_path() -> std::path::PathBuf {
    crate::logx::base_dir().join("install_id")
}

fn looks_like_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if *byte != b'-' {
                    return false;
                }
            }
            _ => {
                if !byte.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

fn random_uuid_v4() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    ))
}

pub fn load_or_create_install_id() -> Result<String, String> {
    let path = install_id_path();
    if let Ok(existing) = fs::read_to_string(&path) {
        let trimmed = existing.trim().to_ascii_lowercase();
        if looks_like_uuid(&trimmed) {
            return Ok(trimmed);
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let id = random_uuid_v4()?;
    fs::write(&path, &id).map_err(|error| error.to_string())?;
    Ok(id)
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!("Niko-Desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())
}

async fn limited_json(response: reqwest::Response) -> Result<serde_json::Value, String> {
    let mut body = Vec::new();
    let mut stream = response;
    while let Some(chunk) = stream.chunk().await.map_err(|error| error.to_string())? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err("使用情况返回了过大的结果。".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| "使用情况返回了无效结果。".into())
}

pub async fn send_heartbeat() -> Result<(), String> {
    let install_id = load_or_create_install_id()?;
    let client = http_client()?;
    let response = client
        .post(HEARTBEAT_URL)
        .json(&serde_json::json!({
            "install_id": install_id,
            "platform": platform(),
            "app_version": env!("CARGO_PKG_VERSION"),
        }))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("heartbeat {}", response.status().as_u16()))
    }
}

pub fn spawn_heartbeat() {
    tauri::async_runtime::spawn(async {
        loop {
            if let Err(error) = send_heartbeat().await {
                crate::logx::append("presence", &format!("heartbeat skipped: {error}"));
            }
            tokio::time::sleep(HEARTBEAT_INTERVAL).await;
        }
    });
}

#[derive(Deserialize)]
struct StatsEnvelope {
    data: UsageStats,
}

#[tauri::command]
pub async fn fetch_usage_stats(secret: String) -> Result<UsageStats, String> {
    let secret = secret.trim();
    if secret.len() < 16 {
        return Err("请输入有效的访问口令。".into());
    }
    let client = http_client()?;
    let response = client
        .get(STATS_URL)
        .header("Authorization", format!("Bearer {secret}"))
        .send()
        .await
        .map_err(|_| "无法读取使用情况，请稍后重试。".to_string())?;
    let status = response.status();
    let payload = limited_json(response).await?;
    if !status.is_success() {
        let message = payload
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(|value| value.as_str())
            .unwrap_or("无法读取使用情况。");
        return Err(message.to_string());
    }
    if let Ok(envelope) = serde_json::from_value::<StatsEnvelope>(payload.clone()) {
        return Ok(envelope.data);
    }
    serde_json::from_value(payload).map_err(|_| "使用情况返回了无效结果。".into())
}

/// 仅用于测试：确认安装标识不会出现在心跳日志里。
pub fn hashed_install_preview(install_id: &str) -> String {
    let digest = Sha256::digest(format!("niko-presence-v1:{install_id}").as_bytes());
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .chars()
        .take(32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_shape_is_strict() {
        assert!(looks_like_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!looks_like_uuid("not-a-uuid"));
        assert!(!looks_like_uuid("550e8400e29b41d4a716446655440000"));
    }

    #[test]
    fn hashed_preview_does_not_contain_raw_id() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let hashed = hashed_install_preview(id);
        assert_eq!(hashed.len(), 32);
        assert!(!hashed.contains("550e8400"));
    }
}
