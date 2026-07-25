use serde::{Deserialize, Serialize};
use crate::targets::{all_targets, ApplyPlan};

#[derive(Debug, Serialize)]
pub struct TargetInfo {
    pub id: String,
    pub name: String,
    pub installed: bool,
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
    let infos = all_targets()
        .iter()
        .map(|t| TargetInfo {
            id: t.id().to_owned(),
            name: t.display_name().to_owned(),
            installed: t.is_installed(),
        })
        .collect();
    Ok(infos)
}

#[tauri::command]
pub async fn apply_target(req: ApplyRequest) -> Result<Vec<String>, String> {
    let plan = ApplyPlan {
        base_url: req.base_url,
        api_key: req.api_key,
        model_group: req.model_group,
    };

    let targets = all_targets();
    let target = targets
        .iter()
        .find(|t| t.id() == req.target_id)
        .ok_or_else(|| format!("未知目标: {}", req.target_id))?;

    let summary = target.apply(&plan)?;
    Ok(summary.changed_keys)
}

/// 对所有已安装目标同时应用 plan
#[tauri::command]
pub async fn apply_all_targets(
    base_url: String,
    api_key: String,
    model_group: Option<String>,
) -> Result<Vec<serde_json::Value>, String> {
    let plan = ApplyPlan { base_url, api_key, model_group };
    let targets = all_targets();
    let mut results = Vec::new();
    for t in &targets {
        if t.is_installed() {
            match t.apply(&plan) {
                Ok(s) => results.push(serde_json::json!({
                    "id": s.target_id,
                    "ok": true,
                    "changed": s.changed_keys
                })),
                Err(e) => results.push(serde_json::json!({
                    "id": t.id(),
                    "ok": false,
                    "error": e
                })),
            }
        }
    }
    Ok(results)
}
