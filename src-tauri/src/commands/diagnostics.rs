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
    crate::logx::append("verify_targets", &format!("GET {models_url}"));

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
                crate::logx::append(
                    "verify_targets",
                    &format!("{} failed: {}", t.id(), e),
                );
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

// ─── E7-3: 错误分类 ─────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct DiagPingResult {
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub error_kind: Option<String>,   // "network" | "auth" | "server" | "unknown"
    pub error_detail: Option<String>,
    pub suggestion: Option<String>,
}

#[tauri::command]
pub async fn ping_diag(url: String) -> Result<DiagPingResult, String> {
    use std::time::Instant;
    crate::logx::append("ping_diag", &format!("checking {url}"));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;

    let t0 = Instant::now();
    match client.head(&url).send().await {
        Ok(resp) => {
            let ms = t0.elapsed().as_millis() as u64;
            let code = resp.status().as_u16();
            crate::logx::append("ping_diag", &format!("HTTP {code} latency={ms}ms"));
            if code == 401 || code == 403 {
                Ok(DiagPingResult {
                    reachable: false,
                    latency_ms: Some(ms),
                    error_kind: Some("auth".to_owned()),
                    error_detail: Some(format!("HTTP {code} 未授权")),
                    suggestion: Some("请检查 API Key 是否填写正确，或 Key 已过期".to_owned()),
                })
            } else if code >= 500 {
                Ok(DiagPingResult {
                    reachable: false,
                    latency_ms: Some(ms),
                    error_kind: Some("server".to_owned()),
                    error_detail: Some(format!("HTTP {code} 服务端错误")),
                    suggestion: Some("服务端暂时不可用，稍后重试或联系管理员".to_owned()),
                })
            } else {
                Ok(DiagPingResult {
                    reachable: true,
                    latency_ms: Some(ms),
                    error_kind: None,
                    error_detail: None,
                    suggestion: None,
                })
            }
        }
        Err(e) => {
            let detail = e.to_string();
            crate::logx::append("ping_diag", &format!("error: {detail}"));
            let (kind, suggestion) = classify_reqwest_error(&e);
            Ok(DiagPingResult {
                reachable: false,
                latency_ms: None,
                error_kind: Some(kind),
                error_detail: Some(detail),
                suggestion: Some(suggestion),
            })
        }
    }
}

fn classify_reqwest_error(e: &reqwest::Error) -> (String, String) {
    if e.is_timeout() {
        return (
            "network".to_owned(),
            "连接超时，请检查网络或使用代理".to_owned(),
        );
    }
    if e.is_connect() {
        return (
            "network".to_owned(),
            "无法连接到服务器，请检查网络或代理设置".to_owned(),
        );
    }
    if e.is_status() {
        return (
            "server".to_owned(),
            "服务端返回了异常状态码，稍后重试".to_owned(),
        );
    }
    (
        "unknown".to_owned(),
        "未知错误，请导出日志后联系支持".to_owned(),
    )
}

// ─── E7-3: 日志导出 ─────────────────────────────────────────────────────────

/// 把当前会话日志写到用户指定路径，并再做一次脱敏。返回最终路径。
#[tauri::command]
pub async fn export_log(dest_path: String) -> Result<String, String> {
    use std::fs;
    use std::path::PathBuf;

    let content = crate::logx::read_tail(256 * 1024);
    // 导出时对每行再脱敏一次
    let cleaned: String = content
        .lines()
        .map(|l| crate::logx::redact_line(l))
        .collect::<Vec<_>>()
        .join("\n");

    let dest = PathBuf::from(&dest_path);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&dest, cleaned.as_bytes()).map_err(|e| e.to_string())?;
    crate::logx::append("export_log", &format!("exported to {dest_path}"));
    Ok(dest_path)
}

// ─── E7-2: 兼容等级实测 ─────────────────────────────────────────────────────

/// 单个「目标应用 + 模型」组合的实测结果。
/// level 与 docs/niko-design.md §9 的四级矩阵一致：
/// native / good / limited / unsupported。
#[derive(Debug, serde::Serialize)]
pub struct CompatProbe {
    pub target_id: String,
    pub model: String,
    pub ok: bool,
    pub level: String,
    pub latency_ms: Option<u64>,
    pub error_kind: Option<String>,
    pub detail: Option<String>,
    pub checked_at: i64,
}

/// 对每个已安装目标，用当前选中模型发一条最小 chat 请求做实测。
/// 基线等级由调用方（前端）提供，实测只做「确认」或「降级」，不做升级。
#[tauri::command]
pub async fn probe_compat(
    base_url: String,
    api_key: String,
    model: String,
    baselines: std::collections::HashMap<String, String>,
) -> Vec<CompatProbe> {
    use crate::targets::all_targets;
    use std::time::Instant;

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            crate::logx::append("probe_compat", &format!("client build failed: {e}"));
            return Vec::new();
        }
    };

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let now = chrono_now();
    let mut out = Vec::new();

    for t in all_targets() {
        if !t.is_installed() {
            continue;
        }
        let baseline = baselines
            .get(t.id())
            .cloned()
            .unwrap_or_else(|| "unsupported".to_owned());

        if baseline == "unsupported" {
            out.push(CompatProbe {
                target_id: t.id().to_owned(),
                model: model.clone(),
                ok: false,
                level: "unsupported".to_owned(),
                latency_ms: None,
                error_kind: None,
                detail: Some("该组合不建议使用，已跳过实测".to_owned()),
                checked_at: now,
            });
            continue;
        }

        let body = serde_json::json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "ping"}]
        });

        let t0 = Instant::now();
        let res = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send()
            .await;
        let ms = t0.elapsed().as_millis() as u64;

        match res {
            Ok(r) => {
                let code = r.status().as_u16();
                crate::logx::append(
                    "probe_compat",
                    &format!("{} model={} HTTP {} {}ms", t.id(), model, code, ms),
                );
                if code < 300 {
                    out.push(CompatProbe {
                        target_id: t.id().to_owned(),
                        model: model.clone(),
                        ok: true,
                        level: baseline,
                        latency_ms: Some(ms),
                        error_kind: None,
                        detail: None,
                        checked_at: now,
                    });
                } else {
                    let (kind, detail) = classify_status(code);
                    out.push(CompatProbe {
                        target_id: t.id().to_owned(),
                        model: model.clone(),
                        ok: false,
                        level: downgrade(&baseline, &kind),
                        latency_ms: Some(ms),
                        error_kind: Some(kind),
                        detail: Some(detail),
                        checked_at: now,
                    });
                }
            }
            Err(e) => {
                let (kind, suggestion) = classify_reqwest_error(&e);
                crate::logx::append(
                    "probe_compat",
                    &format!("{} model={} error: {}", t.id(), model, e),
                );
                out.push(CompatProbe {
                    target_id: t.id().to_owned(),
                    model: model.clone(),
                    ok: false,
                    level: downgrade(&baseline, &kind),
                    latency_ms: None,
                    error_kind: Some(kind),
                    detail: Some(suggestion),
                    checked_at: now,
                });
            }
        }
    }

    out
}

fn classify_status(code: u16) -> (String, String) {
    match code {
        401 | 403 => ("auth".to_owned(), format!("HTTP {code}：Key 无效或无该分组权限")),
        404 => ("model".to_owned(), format!("HTTP {code}：当前分组不存在该模型")),
        429 => ("rate_limit".to_owned(), format!("HTTP {code}：请求过于频繁，稍后重试")),
        c if c >= 500 => ("server".to_owned(), format!("HTTP {c}：上游或服务端错误")),
        c => ("unknown".to_owned(), format!("HTTP {c}：请求被拒绝")),
    }
}

/// 实测失败时的降级规则：只降不升。
/// 鉴权/限流是账号侧问题，不代表组合不兼容，等级保持基线但标记未通过；
/// 模型不存在或服务端错误则降为 limited/unsupported。
fn downgrade(baseline: &str, kind: &str) -> String {
    match kind {
        "auth" | "rate_limit" => baseline.to_owned(),
        "model" => "unsupported".to_owned(),
        _ => match baseline {
            "native" | "good" => "limited".to_owned(),
            other => other.to_owned(),
        },
    }
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
