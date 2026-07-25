//! E7-1 连通性自检
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct PingResult {
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn ping(url: String) -> Result<PingResult, String> {
    use std::time::Instant;

    // 只做 HEAD 请求，超时 8s
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;

    let t0 = Instant::now();
    match client.head(&url).send().await {
        Ok(resp) => {
            let ms = t0.elapsed().as_millis() as u64;
            if resp.status().is_success() || resp.status().as_u16() < 500 {
                Ok(PingResult { reachable: true, latency_ms: Some(ms), error: None })
            } else {
                Ok(PingResult {
                    reachable: false,
                    latency_ms: Some(ms),
                    error: Some(format!("HTTP {}", resp.status())),
                })
            }
        }
        Err(e) => Ok(PingResult { reachable: false, latency_ms: None, error: Some(e.to_string()) }),
    }
}

#[tauri::command]
pub async fn verify_targets(base_url: String, api_key: String) -> Vec<serde_json::Value> {
    use crate::targets::all_targets;
    use std::time::Instant;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    let mut results = Vec::new();
    let models_url = format!("{}/models", base_url.trim_end_matches('/'));

    for t in all_targets() {
        if !t.is_installed() { continue; }
        let t0 = Instant::now();
        let res = client
            .get(&models_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await;
        let ms = t0.elapsed().as_millis() as u64;
        match res {
            Ok(r) if r.status().as_u16() < 500 => {
                results.push(serde_json::json!({
                    "id": t.id(),
                    "ok": true,
                    "latency_ms": ms
                }));
            }
            Ok(r) => {
                results.push(serde_json::json!({
                    "id": t.id(),
                    "ok": false,
                    "error": format!("HTTP {}", r.status())
                }));
            }
            Err(e) => {
                results.push(serde_json::json!({
                    "id": t.id(),
                    "ok": false,
                    "error": e.to_string()
                }));
            }
        }
    }
    results
}
