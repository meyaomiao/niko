use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct TargetInfo {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub active: bool,
}

#[derive(Debug, Deserialize)]
pub struct ApplyRequest {
    pub target_id: String,
    pub base_url: String,
    pub api_key: String,
    pub model_group: Option<String>,
}

#[tauri::command]
pub async fn list_targets() -> Result<Vec<TargetInfo>, String> {
    // E5-1 / E6-x 实现
    Ok(vec![])
}

#[tauri::command]
pub async fn apply_target(_req: ApplyRequest) -> Result<(), String> {
    // E5-1 实现
    Err("not implemented".into())
}
