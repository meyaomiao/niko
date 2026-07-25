use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Deserialize)]
pub struct ApplyRequest {
    pub target_id: String,
    pub model_group: String,
}

#[tauri::command]
pub async fn list_targets() -> Result<Vec<Target>, String> {
    // E5-1 / E6-x 实现
    Ok(vec![])
}

#[tauri::command]
pub async fn apply_target(_req: ApplyRequest) -> Result<(), String> {
    // E5-1 实现
    Err("not implemented".into())
}
