use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub turnstile: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub username: String,
}

#[tauri::command]
pub async fn login(_req: LoginRequest) -> Result<LoginResponse, String> {
    // E3-2 实现
    Err("not implemented".into())
}

#[tauri::command]
pub async fn logout() -> Result<(), String> {
    // E3-2 实现
    Ok(())
}
