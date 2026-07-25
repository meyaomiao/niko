use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct BootstrapInfo {
    pub min_version: String,
    pub server_url: String,
}

#[tauri::command]
pub async fn get_bootstrap() -> Result<BootstrapInfo, String> {
    // E2-4 / E3-1 实现
    Err("not implemented".into())
}
