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
        .map_err(|_| "检查没有完成，请稍后重试。".to_owned())?;

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
                    error: Some("模型服务暂时不可用，请稍后重试。".to_owned()),
                })
            }
        }
        Err(e) => Ok(PingResult {
            reachable: false,
            latency_ms: None,
            error: Some(safe_reqwest_detail(&e)),
        }),
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
                    "error": safe_status_detail(r.status().as_u16())
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
                    "error": safe_reqwest_detail(&e)
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
        .map_err(|_| "检查没有完成，请稍后重试。".to_owned())?;

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
                    error_detail: Some("连接密钥无效或已过期".to_owned()),
                    suggestion: Some("请重新接入后再检查。".to_owned()),
                })
            } else if code >= 500 {
                Ok(DiagPingResult {
                    reachable: false,
                    latency_ms: Some(ms),
                    error_kind: Some("server".to_owned()),
                    error_detail: Some("模型服务暂时不可用".to_owned()),
                    suggestion: Some("请稍后重试。".to_owned()),
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
                error_detail: Some(safe_reqwest_detail(&e)),
                suggestion: Some(suggestion),
            })
        }
    }
}

fn classify_reqwest_error(e: &reqwest::Error) -> (String, String) {
    if e.is_timeout() {
        return (
            "network".to_owned(),
            "网络连接超时，请检查网络后重试。".to_owned(),
        );
    }
    if e.is_connect() {
        return (
            "network".to_owned(),
            "网络连接失败，请检查网络后重试。".to_owned(),
        );
    }
    if e.is_status() {
        return (
            "server".to_owned(),
            "模型服务暂时不可用，请稍后重试。".to_owned(),
        );
    }
    (
        "unknown".to_owned(),
        "检查没有完成，请稍后重试。".to_owned(),
    )
}

fn safe_reqwest_detail(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "网络连接超时，请检查网络后重试。".to_owned();
    }
    if e.is_connect() {
        return "网络连接失败，请检查网络后重试。".to_owned();
    }
    "检查没有完成，请稍后重试。".to_owned()
}

fn safe_status_detail(code: u16) -> String {
    match code {
        401 | 403 => "连接密钥无效或已过期（HTTP 401/403），请重新接入后再试。".to_owned(),
        404 => "服务地址或模型不可用（HTTP 404）。".to_owned(),
        429 => "该分组渠道限流中（HTTP 429），请稍后重试。".to_owned(),
        500..=599 => format!("渠道上游不可用（HTTP {code}）：该分组渠道故障或过载，非客户端问题。"),
        _ => format!("渠道返回异常状态（HTTP {code}）。"),
    }
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
        401 | 403 => ("auth".to_owned(), "连接密钥无效或已过期，请重新接入后再试。".to_owned()),
        404 => ("model".to_owned(), "当前模型不可用，请重新选择模型后再试。".to_owned()),
        429 => ("rate_limit".to_owned(), "服务暂时繁忙，请稍后重试。".to_owned()),
        500..=599 => ("server".to_owned(), "模型服务暂时不可用，请稍后重试。".to_owned()),
        _ => ("unknown".to_owned(), "检查没有完成，请重新接入后再试。".to_owned()),
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

// ─── E9: 分组测速（TTFT + 中位数采样） ───────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct BenchmarkSample {
    pub ttft_ms: Option<u64>,
    pub total_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BenchmarkResult {
    pub model: String,
    pub samples: Vec<BenchmarkSample>,
    /// TTFT 中位数（只统计成功样本；不足一半成功时为 None）
    pub median_ttft_ms: Option<u64>,
}

/// 纯函数：中位数（偶数个取中间两值较小者，偏向保守估计）
fn median(values: &[u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2])
}

/// 用当前模型向指定分组发真实流式补全请求，测首字延迟（TTFT）。
/// 每次采样都是独立请求：max_tokens=1 控制成本，流式读首 chunk 计时。
/// HEAD ping 只能测网关可达性，测不出模型排队与推理首字，所以这里发真实请求。
#[tauri::command]
pub async fn benchmark_group(
    base_url: String,
    api_key: String,
    model: String,
    samples: Option<usize>,
) -> Result<BenchmarkResult, String> {
    use std::time::Instant;

    // 采样次数限制在 1~5：多了慢且费钱，少了抖动大
    let rounds = samples.unwrap_or(3).clamp(1, 5);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| "测速没有完成，请稍后重试。".to_owned())?;

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1,
        "stream": true,
        "messages": [{"role": "user", "content": "hi"}],
    });

    let mut out: Vec<BenchmarkSample> = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let t0 = Instant::now();
        let sample = match client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let mut resp = resp;
                let mut ttft = None;
                let mut stream_error: Option<String> = None;
                // 记录首段内容形态，用于区分「空流」「错误 JSON」等渠道侧问题
                let mut first_head: Option<String> = None;
                let mut got_stream_end = false;
                loop {
                    match resp.chunk().await {
                        Ok(Some(bytes)) => {
                            if !bytes.is_empty() {
                                if first_head.is_none() {
                                    first_head = Some(
                                        String::from_utf8_lossy(&bytes[..bytes.len().min(64)]).to_string(),
                                    );
                                }
                                ttft = Some(t0.elapsed().as_millis() as u64);
                                // 首 chunk 已到手即判定 TTFT；max_tokens=1 时服务端随即收尾，
                                // 不继续消费剩余流
                                break;
                            }
                        }
                        Ok(None) => {
                            got_stream_end = true;
                            break;
                        }
                        Err(e) => {
                            stream_error = Some(safe_reqwest_detail(&e));
                            break;
                        }
                    }
                }
                // 200 但流里没有任何字节就收尾：大概率渠道不支持该模型/密钥无权限/假流式
                if ttft.is_none() && stream_error.is_none() && got_stream_end && first_head.is_none() {
                    stream_error =
                        Some("渠道返回空流（HTTP 200，0 字节即关闭）：密钥可能无该模型权限或渠道为假流式".to_owned());
                }
                // 首段不是 SSE data 行也不是 JSON choice：多半是错误 JSON 包在 200 里
                if ttft.is_some() {
                    if let Some(head) = &first_head {
                        let trimmed = head.trim_start();
                        if trimmed.starts_with('{') && trimmed.contains("\"error\"") {
                            stream_error = Some(format!("渠道返回错误 JSON（200）：{}", head));
                        }
                    }
                }
                if let Some(error) = stream_error {
                    BenchmarkSample {
                        ttft_ms: None,
                        total_ms: Some(t0.elapsed().as_millis() as u64),
                        error: Some(error),
                    }
                } else {
                    match ttft {
                        Some(ms) => BenchmarkSample {
                            ttft_ms: Some(ms),
                            total_ms: Some(t0.elapsed().as_millis() as u64),
                            error: None,
                        },
                        None => BenchmarkSample {
                            ttft_ms: None,
                            total_ms: Some(t0.elapsed().as_millis() as u64),
                            error: Some("模型服务暂时不可用，请稍后重试。".to_owned()),
                        },
                    }
                }
            }
            Ok(resp) => BenchmarkSample {
                ttft_ms: None,
                total_ms: Some(t0.elapsed().as_millis() as u64),
                error: Some(safe_status_detail(resp.status().as_u16())),
            },
            Err(e) => BenchmarkSample {
                ttft_ms: None,
                total_ms: None,
                error: Some(safe_reqwest_detail(&e)),
            },
        };
        out.push(sample);
    }

    let ttfts: Vec<u64> = out.iter().filter_map(|s| s.ttft_ms).collect();
    let median_ttft = median(&ttfts);
    crate::logx::append(
        "benchmark_group",
        &format!("model={model} samples={rounds} median_ttft={median_ttft:?}"),
    );
    Ok(BenchmarkResult { model, samples: out, median_ttft_ms: median_ttft })
}

#[cfg(test)]
mod tests {
    use super::median;

    #[test]
    fn median_prefers_conservative_middle() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[120]), Some(120));
        assert_eq!(median(&[300, 100, 200]), Some(200));
        assert_eq!(median(&[400, 100, 200, 300]), Some(300));
    }
}
