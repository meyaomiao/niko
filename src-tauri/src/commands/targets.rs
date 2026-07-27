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
    pub model: Option<String>,
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
        model: req.model,
    };

    let targets = all_targets();
    let target = targets
        .iter()
        .find(|t| t.id() == req.target_id)
        .ok_or_else(|| format!("未知目标: {}", req.target_id))?;

    let summary = target.apply(&plan)?;
    crate::logx::append(
        "apply_target",
        &format!("{} changed={:?}", req.target_id, summary.changed_keys),
    );
    Ok(summary.changed_keys)
}

/// 对所有已安装目标同时应用 plan
#[tauri::command]
pub async fn apply_all_targets(
    base_url: String,
    api_key: String,
    model_group: Option<String>,
    model: Option<String>,
) -> Result<Vec<serde_json::Value>, String> {
    let plan = ApplyPlan { base_url, api_key, model_group, model };
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

use crate::targets::{check_drift, DriftReport};

#[tauri::command]
pub async fn check_drift_cmd(
    target_id: String,
    base_url: String,
    api_key: String,
    model_group: Option<String>,
) -> Result<DriftReport, String> {
    let plan = ApplyPlan { base_url, api_key, model_group, model: None };
    check_drift(&target_id, &plan)
}

#[tauri::command]
pub async fn check_all_drift(
    base_url: String,
    api_key: String,
    model_group: Option<String>,
) -> Result<Vec<DriftReport>, String> {
    let plan = ApplyPlan { base_url, api_key, model_group, model: None };
    let targets = all_targets();
    let mut reports = Vec::new();
    for t in &targets {
        if t.is_installed() {
            match check_drift(t.id(), &plan) {
                Ok(r) => reports.push(r),
                Err(e) => reports.push(DriftReport {
                    target_id: t.id().to_owned(),
                    drifted: true,
                    mismatched_keys: vec![format!("error: {e}")],
                }),
            }
        }
    }
    Ok(reports)
}
