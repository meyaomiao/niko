#[tauri::command]
pub async fn ping() -> Result<String, String> {
    Ok("pong".into())
}
