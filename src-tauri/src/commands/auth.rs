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

/// 「记住我」凭证：账号明文存本地配置不安全，统一放系统钥匙串。
const REMEMBER_ACCOUNT: &str = "remembered-login";

#[derive(Debug, Serialize, Deserialize)]
pub struct RememberedLogin {
    pub username: String,
    pub password: String,
}

#[tauri::command]
pub fn save_remembered_login(login: RememberedLogin) -> Result<(), String> {
    let payload = serde_json::to_string(&login).map_err(|e| e.to_string())?;
    crate::credentials::CredentialStore::set(REMEMBER_ACCOUNT, &payload)
}

#[tauri::command]
pub fn load_remembered_login() -> Option<RememberedLogin> {
    let raw = crate::credentials::CredentialStore::get(REMEMBER_ACCOUNT).ok()?;
    serde_json::from_str(&raw).ok()
}

#[tauri::command]
pub fn clear_remembered_login() -> Result<(), String> {
    crate::credentials::CredentialStore::delete(REMEMBER_ACCOUNT)
}
