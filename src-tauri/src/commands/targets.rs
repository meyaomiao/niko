use serde::{Deserialize, Serialize};
use crate::targets::{all_targets, ApplyPlan};

#[derive(Debug, Serialize)]
pub struct TargetInfo {
    pub id: String,
    pub name: String,
    pub installed: bool,
    /// 本机已安装时提取到的真实应用图标（PNG data URI）
    pub icon: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApplyRequest {
    pub target_id: String,
    pub base_url: String,
    pub api_key: String,
    pub model_group: Option<String>,
    pub model: Option<String>,
    /// Codex 混用模式（保留 ChatGPT 登录态），仅对 codex 目标有意义
    #[serde(default)]
    pub codex_mixed: bool,
}

#[tauri::command]
pub async fn list_targets() -> Result<Vec<TargetInfo>, String> {
    let infos = all_targets()
        .iter()
        .map(|t| TargetInfo {
            id: t.id().to_owned(),
            name: t.display_name().to_owned(),
            installed: t.is_installed(),
            icon: t.icon_data_uri(),
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
        codex_mixed: req.codex_mixed,
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
    codex_mixed: Option<bool>,
) -> Result<Vec<serde_json::Value>, String> {
    let plan = ApplyPlan {
        base_url,
        api_key,
        model_group,
        model,
        codex_mixed: codex_mixed.unwrap_or(false),
    };
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
    codex_mixed: Option<bool>,
) -> Result<DriftReport, String> {
    let plan = ApplyPlan {
        base_url,
        api_key,
        model_group,
        model: None,
        codex_mixed: codex_mixed.unwrap_or(false),
    };
    check_drift(&target_id, &plan)
}

#[tauri::command]
pub async fn check_all_drift(
    base_url: String,
    api_key: String,
    model_group: Option<String>,
    codex_mixed: Option<bool>,
) -> Result<Vec<DriftReport>, String> {
    let plan = ApplyPlan {
        base_url,
        api_key,
        model_group,
        model: None,
        codex_mixed: codex_mixed.unwrap_or(false),
    };
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

// ─── 连通性测试 ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ConnectivityResult {
    pub target_id: String,
    pub ok: bool,
    pub endpoint: String,
    pub model: Option<String>,
    pub latency_ms: Option<u64>,
    pub detail: String,
}

/// 用目标应用磁盘上真实生效的配置发一条最小请求，确认配置真的能用。
#[tauri::command]
pub async fn test_connectivity(target_id: String) -> Result<ConnectivityResult, String> {
    let cfg = crate::targets::effective_config(&target_id)?;
    let model = cfg.model.clone().ok_or("配置里没有默认模型，请先点击启用")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    // 两家协议的请求体和鉴权头都不同，必须按各自格式发，否则测不出真实可用性
    let req = match cfg.auth_style.as_str() {
        "anthropic" => client
            .post(&cfg.endpoint)
            .header("x-api-key", &cfg.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "ping"}]
            })),
        _ => client
            .post(&cfg.endpoint)
            .header("Authorization", format!("Bearer {}", cfg.api_key))
            .json(&serde_json::json!({
                "model": model,
                "input": "ping",
                "max_output_tokens": 16
            })),
    };

    let t0 = std::time::Instant::now();
    let res = req.send().await;
    let ms = t0.elapsed().as_millis() as u64;

    match res {
        Ok(r) => {
            let code = r.status().as_u16();
            let ok = code < 300;
            let detail = if ok {
                format!("连通正常，{model} 可用（{ms}ms）")
            } else {
                let body = r.text().await.unwrap_or_default();
                let hint = match code {
                    401 | 403 => "Key 无效或该分组无权限",
                    404 => "地址或模型不存在，可尝试重新启用",
                    429 => "请求过于频繁，稍后再试",
                    c if c >= 500 => "上游或服务端错误",
                    _ => "请求被拒绝",
                };
                format!("HTTP {code}：{hint}。{}", body.chars().take(160).collect::<String>())
            };
            crate::logx::append(
                "test_connectivity",
                &format!("{target_id} {} HTTP {code} {ms}ms", cfg.endpoint),
            );
            Ok(ConnectivityResult {
                target_id,
                ok,
                endpoint: cfg.endpoint,
                model: Some(model),
                latency_ms: Some(ms),
                detail,
            })
        }
        Err(e) => {
            crate::logx::append("test_connectivity", &format!("{target_id} error: {e}"));
            let detail = if e.is_timeout() {
                "请求超时，检查网络或代理设置".to_owned()
            } else if e.is_connect() {
                "无法连接服务器，检查网络或代理设置".to_owned()
            } else {
                format!("请求失败：{e}")
            };
            Ok(ConnectivityResult {
                target_id,
                ok: false,
                endpoint: cfg.endpoint,
                model: Some(model),
                latency_ms: None,
                detail,
            })
        }
    }
}

// ─── 恢复官方默认配置 ───────────────────────────────────────────────────────

/// 移除 Niko 写入的中转配置，让应用回到用官方账号登录的状态
#[tauri::command]
pub async fn restore_target_defaults(target_id: String) -> Result<Vec<String>, String> {
    let summary = crate::targets::restore_defaults(&target_id)?;
    crate::logx::append(
        "restore_target_defaults",
        &format!("{target_id} changed={:?}", summary.changed_keys),
    );
    Ok(summary.changed_keys)
}
