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
                Ok(PingResult {
                    reachable: true,
                    latency_ms: Some(ms),
                    error: None,
                })
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
        if !t.is_installed() {
            continue;
        }
        let t0 = Instant::now();
        let res = client
            .get(&models_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await;
        let ms = t0.elapsed().as_millis() as u64;
        match res {
            Ok(r) if r.status().is_success() => {
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
                crate::logx::append("verify_targets", &format!("{} failed: {}", t.id(), e));
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
    pub error_kind: Option<String>, // "network" | "auth" | "server" | "unknown"
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
            Ok(classify_reachability_status(code, ms))
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

fn classify_reachability_status(code: u16, latency_ms: u64) -> DiagPingResult {
    if code == 401 || code == 403 {
        DiagPingResult {
            reachable: true,
            latency_ms: Some(latency_ms),
            error_kind: Some("requires_auth".to_owned()),
            error_detail: Some("服务可达，但此地址需要身份验证".to_owned()),
            suggestion: Some("此检查仅验证网络可达性；模型密钥请在首页「检查」中验证。".to_owned()),
        }
    } else if code >= 500 {
        DiagPingResult {
            reachable: false,
            latency_ms: Some(latency_ms),
            error_kind: Some("server".to_owned()),
            error_detail: Some("模型服务暂时不可用".to_owned()),
            suggestion: Some("请稍后重试。".to_owned()),
        }
    } else {
        DiagPingResult {
            reachable: true,
            latency_ms: Some(latency_ms),
            error_kind: None,
            error_detail: None,
            suggestion: None,
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
        401 | 403 => (
            "auth".to_owned(),
            "连接密钥无效或已过期，请重新接入后再试。".to_owned(),
        ),
        404 => (
            "model".to_owned(),
            "当前模型不可用，请重新选择模型后再试。".to_owned(),
        ),
        429 => (
            "rate_limit".to_owned(),
            "服务暂时繁忙，请稍后重试。".to_owned(),
        ),
        500..=599 => (
            "server".to_owned(),
            "模型服务暂时不可用，请稍后重试。".to_owned(),
        ),
        _ => (
            "unknown".to_owned(),
            "检查没有完成，请重新接入后再试。".to_owned(),
        ),
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
    /// TTFT 中位数（仅统计无错误的内容样本；成功数未超过一半时为 None）
    pub median_ttft_ms: Option<u64>,
}

/// 纯函数：中位数（偶数个取中间两值较大者，避免低估延迟）
fn median(values: &[u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2])
}

fn benchmark_median(samples: &[BenchmarkSample]) -> Option<u64> {
    let values: Vec<u64> = samples
        .iter()
        .filter(|sample| sample.error.is_none())
        .filter_map(|sample| sample.ttft_ms)
        .collect();
    (values.len() * 2 > samples.len())
        .then(|| median(&values))
        .flatten()
}

const MAX_BENCH_EVENT_BYTES: usize = 64 * 1024;

/// 仅由有效生成内容触发首字计时；网络分块、SSE 心跳和 role 帧不算输出。
#[derive(Default)]
struct BenchmarkStream {
    pending: Vec<u8>,
    data: String,
    error_event: bool,
    ended: bool,
    last_was_cr: bool,
}

impl BenchmarkStream {
    fn push(&mut self, bytes: &[u8]) -> Result<bool, String> {
        let mut content = false;
        for &byte in bytes {
            if self.ended {
                break;
            }
            if std::mem::take(&mut self.last_was_cr) && byte == b'\n' {
                continue;
            }
            if byte == b'\n' || byte == b'\r' {
                self.last_was_cr = byte == b'\r';
                let line = std::mem::take(&mut self.pending);
                let line = std::str::from_utf8(&line)
                    .map_err(|_| "流式响应编码无效，无法完成测速。".to_owned())?;
                content |= self.line(line)?;
            } else {
                self.pending.push(byte);
            }
            if self.pending.len() + self.data.len() > MAX_BENCH_EVENT_BYTES {
                return Err("流式响应过大，无法完成测速。".to_owned());
            }
        }
        Ok(content)
    }

    fn line(&mut self, line: &str) -> Result<bool, String> {
        if line.is_empty() {
            return self.dispatch();
        }
        if let Some(value) = line.strip_prefix("data:") {
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
        } else if let Some(value) = line.strip_prefix("event:") {
            self.error_event = matches!(value.trim(), "error" | "response.failed");
        }
        Ok(false)
    }

    fn dispatch(&mut self) -> Result<bool, String> {
        let data = std::mem::take(&mut self.data);
        let error_event = std::mem::take(&mut self.error_event);
        if error_event {
            return Err("模型服务返回流式错误，请稍后重试。".to_owned());
        }
        let data = data.trim();
        if data.is_empty() {
            return Ok(false);
        }
        if data == "[DONE]" {
            self.ended = true;
            return Ok(false);
        }
        let value: serde_json::Value = serde_json::from_str(data)
            .map_err(|_| "模型服务返回了无效流式数据，请稍后重试。".to_owned())?;
        let event_type = value.get("type").and_then(serde_json::Value::as_str);
        let incomplete = event_type == Some("response.incomplete")
            && value
                .pointer("/response/incomplete_details/reason")
                .and_then(serde_json::Value::as_str)
                != Some("max_output_tokens");
        if value.get("error").is_some_and(|error| !error.is_null())
            || value
                .pointer("/response/error")
                .is_some_and(|error| !error.is_null())
            || matches!(event_type, Some("error" | "response.failed"))
            || incomplete
        {
            return Err("模型服务返回流式错误，请稍后重试。".to_owned());
        }
        Ok(has_generated_content(&value))
    }

    fn finish(&mut self) -> Result<bool, String> {
        if self.ended {
            return Ok(false);
        }
        let mut content = self.push(b"\n")?;
        content |= self.dispatch()?;
        Ok(content)
    }
}

fn has_generated_content(value: &serde_json::Value) -> bool {
    fn text(value: Option<&serde_json::Value>) -> bool {
        value
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| !text.is_empty())
    }
    if let Some(choices) = value.get("choices").and_then(serde_json::Value::as_array) {
        for choice in choices {
            if text(choice.get("text")) {
                return true;
            }
            if let Some(delta) = choice.get("delta") {
                if [
                    "content",
                    "reasoning_content",
                    "reasoning",
                    "reasoning_text",
                ]
                .iter()
                .any(|key| text(delta.get(*key)))
                {
                    return true;
                }
                if text(delta.pointer("/function_call/arguments"))
                    || delta
                        .get("tool_calls")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|calls| {
                            calls
                                .iter()
                                .any(|call| text(call.pointer("/function/arguments")))
                        })
                {
                    return true;
                }
            }
        }
    }
    match value.get("type").and_then(serde_json::Value::as_str) {
        Some("content_block_delta") => ["text", "thinking", "partial_json"]
            .iter()
            .any(|key| text(value.get("delta").and_then(|delta| delta.get(*key)))),
        Some(
            "response.output_text.delta"
            | "response.reasoning_text.delta"
            | "response.function_call_arguments.delta",
        ) => text(value.get("delta")),
        _ => false,
    }
}

const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;

#[derive(Default)]
struct ErrorBodyPreview(Vec<u8>);

impl ErrorBodyPreview {
    fn push(&mut self, bytes: &[u8]) -> bool {
        let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(self.0.len());
        self.0
            .extend_from_slice(&bytes[..bytes.len().min(remaining)]);
        self.0.len() < MAX_ERROR_BODY_BYTES
    }

    fn text(&self) -> String {
        let prefix: String = String::from_utf8_lossy(&self.0)
            .trim()
            .chars()
            .take(120)
            .collect();
        crate::logx::redact_line(&prefix)
    }
}

async fn bounded_error_preview(mut response: reqwest::Response) -> String {
    let mut preview = ErrorBodyPreview::default();
    while let Ok(Some(bytes)) = response.chunk().await {
        if !preview.push(&bytes) {
            break;
        }
    }
    preview.text()
}

/// 用当前模型向指定分组发真实流式补全请求，测首字延迟（TTFT）。
/// 每次采样都是独立请求：max_tokens=1 控制成本，解析有效内容后计时。
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
    // 全局共享连接池：与 agent 长连接形态一致，跨分组/跨测速轮次复用 TLS，
    // 否则每次 invoke 新建 Client 都要重付握手，测出来的是握手而不是模型速度
    static BENCH_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = BENCH_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .build()
            .expect("failed to build benchmark http client")
    });

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
                let mut stream = BenchmarkStream::default();
                loop {
                    match resp.chunk().await {
                        Ok(Some(bytes)) => match stream.push(&bytes) {
                            Ok(true) if ttft.is_none() => {
                                ttft = Some(t0.elapsed().as_millis() as u64)
                            }
                            Ok(_) => {}
                            Err(error) => {
                                stream_error = Some(error);
                                break;
                            }
                        },
                        Ok(None) => {
                            match stream.finish() {
                                Ok(true) if ttft.is_none() => {
                                    ttft = Some(t0.elapsed().as_millis() as u64)
                                }
                                Ok(_) => {}
                                Err(error) => stream_error = Some(error),
                            }
                            break;
                        }
                        Err(e) => {
                            stream_error = Some(safe_reqwest_detail(&e));
                            break;
                        }
                    }
                }
                // 继续读完短响应以复用连接；任何流错误都令本次样本失败。
                if ttft.is_none() && stream_error.is_none() {
                    stream_error = Some("服务未返回有效流式内容，无法测量首字延迟。".to_owned());
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
            Ok(resp) => {
                let status = resp.status().as_u16();
                // 读错误 body：new-api 的 503 通常带「无可用渠道」等真实原因
                let body_head = bounded_error_preview(resp).await;
                // 对 5xx 补一次非流式探测：能通说明渠道活着，只是不支持流式探测
                if (500..=599).contains(&status) {
                    let fallback_body = serde_json::json!({
                        "model": model,
                        "max_tokens": 16,
                        "stream": false,
                        "messages": [{"role": "user", "content": "hi"}],
                    });
                    let fallback = client
                        .post(&url)
                        .bearer_auth(&api_key)
                        .json(&fallback_body)
                        .send()
                        .await;
                    if let Ok(fr) = fallback {
                        if fr.status().is_success() {
                            BenchmarkSample {
                                ttft_ms: None,
                                total_ms: Some(t0.elapsed().as_millis() as u64),
                                error: Some(
                                    "渠道可用但不支持流式探测（非流式请求成功，无法测首字延迟）"
                                        .to_owned(),
                                ),
                            }
                        } else {
                            BenchmarkSample {
                                ttft_ms: None,
                                total_ms: Some(t0.elapsed().as_millis() as u64),
                                error: Some(format!(
                                    "渠道不可用（HTTP {status}，非流式也失败 {}）：{}",
                                    fr.status().as_u16(),
                                    body_head
                                )),
                            }
                        }
                    } else {
                        BenchmarkSample {
                            ttft_ms: None,
                            total_ms: Some(t0.elapsed().as_millis() as u64),
                            error: Some(format!("渠道不可用（HTTP {status}）：{}", body_head)),
                        }
                    }
                } else {
                    BenchmarkSample {
                        ttft_ms: None,
                        total_ms: Some(t0.elapsed().as_millis() as u64),
                        error: Some(format!(
                            "{}（HTTP {status}：{body_head}）",
                            safe_status_detail(status)
                        )),
                    }
                }
            }
            Err(e) => BenchmarkSample {
                ttft_ms: None,
                total_ms: None,
                error: Some(safe_reqwest_detail(&e)),
            },
        };
        out.push(sample);
    }

    let median_ttft = benchmark_median(&out);
    crate::logx::append(
        "benchmark_group",
        &format!("model={model} samples={rounds} median_ttft={median_ttft:?}"),
    );
    Ok(BenchmarkResult {
        model,
        samples: out,
        median_ttft_ms: median_ttft,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_benchmark_waits_for_generated_content_not_roles_or_heartbeats() {
        let mut stream = BenchmarkStream::default();
        assert!(!stream.push(b": keepalive\n\ndata: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n").unwrap());
        assert!(!stream
            .push(b"data: {\"choices\":[],\"usage\":{\"completion_tokens\":1}}\n\n")
            .unwrap());
        let frame = "data: {\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\r\n\r\n".as_bytes();
        for chunk in frame[..frame.len() - 2].chunks(2) {
            assert!(!stream.push(chunk).unwrap());
        }
        assert!(stream.push(&frame[frame.len() - 2..]).unwrap());
        assert!(!stream.push(b"data: [DONE]\n\n").unwrap());
        assert!(!stream.finish().unwrap());
    }

    #[test]
    fn isolated_benchmark_recognizes_reasoning_tool_and_multiline_frames() {
        for frame in [
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\"arguments\":\"{\"}}]}}]}\n\n",
            "data: {\"choices\":\ndata: [{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"ok\"}}\n\n",
        ] {
            assert!(BenchmarkStream::default().push(frame.as_bytes()).unwrap());
        }
        let mut incomplete = BenchmarkStream::default();
        assert!(!incomplete
            .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"last\"}}]}")
            .unwrap());
        assert!(incomplete.finish().unwrap());
    }

    #[test]
    fn isolated_benchmark_rejects_errors_invalid_data_and_unbounded_events() {
        for frame in [
            "event: error\ndata: {\"message\":\"private fixture\"}\n\n",
            "data: {\"error\":{\"message\":\"private fixture\"}}\n\n",
            "data: not-json-private-fixture\n\n",
        ] {
            let error = BenchmarkStream::default()
                .push(frame.as_bytes())
                .unwrap_err();
            assert!(!error.contains("private"));
        }
        assert!(BenchmarkStream::default()
            .push(&vec![b'x'; MAX_BENCH_EVENT_BYTES + 1])
            .is_err());
        let mut non_stream = BenchmarkStream::default();
        assert!(!non_stream
            .push(b"{\"choices\":[{\"message\":{\"content\":\"not streaming\"}}]}\n")
            .unwrap());
        assert!(!non_stream.finish().unwrap());
        let mut late_error = BenchmarkStream::default();
        assert!(late_error
            .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n")
            .unwrap());
        assert!(late_error
            .push(b"data: {\"error\":\"failure\"}\n\n")
            .is_err());
    }

    #[test]
    fn isolated_benchmark_handles_cr_only_and_split_crlf_events() {
        let mut cr = BenchmarkStream::default();
        assert!(!cr
            .push(b": heartbeat\r\rdata: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\r\r")
            .unwrap());
        assert!(cr
            .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\r\rdata: [DONE]\r\r")
            .unwrap());
        assert!(!cr.finish().unwrap());
        let mut split = BenchmarkStream::default();
        assert!(!split
            .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\r")
            .unwrap());
        assert!(split.push(b"\n\r").unwrap());
        assert!(!split.push(b"\ndata: [DONE]\r\n\r\n").unwrap());
    }

    #[test]
    fn isolated_benchmark_invalidates_content_followed_by_failed_response() {
        for failure in [
            "event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_error\"}}}\n\n",
            "data: {\"type\":\"response.incomplete\",\"response\":{\"incomplete_details\":{\"reason\":\"content_filter\"}}}\n\n",
        ] {
            let mut stream = BenchmarkStream::default();
            assert!(stream.push(b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n").unwrap());
            assert!(stream.push(failure.as_bytes()).is_err());
        }
        // max_tokens=1 is intentional for the benchmark, so this limit is not a service failure.
        assert!(!BenchmarkStream::default().push(b"data: {\"type\":\"response.incomplete\",\"response\":{\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n\n").unwrap());
    }

    #[test]
    fn isolated_error_body_preview_is_bounded_and_redacts_keys() {
        let mut preview = ErrorBodyPreview::default();
        assert!(preview.push(b"failure sk-fixture-secret-1234567890 "));
        assert!(!preview.push(&vec![b'x'; MAX_ERROR_BODY_BYTES * 2]));
        assert_eq!(preview.0.len(), MAX_ERROR_BODY_BYTES);
        assert!(!preview.push(b"ignored"));
        assert_eq!(preview.0.len(), MAX_ERROR_BODY_BYTES);
        assert!(!preview.text().contains("sk-fixture-secret-1234567890"));
        assert!(preview.text().chars().count() <= 120);
    }

    #[test]
    fn isolated_benchmark_requires_a_majority_of_successful_samples() {
        let ok = |value| BenchmarkSample {
            ttft_ms: Some(value),
            total_ms: Some(value),
            error: None,
        };
        let failed = || BenchmarkSample {
            ttft_ms: None,
            total_ms: None,
            error: Some("fixture".to_owned()),
        };
        assert_eq!(benchmark_median(&[]), None);
        assert_eq!(benchmark_median(&[ok(10), failed(), failed()]), None);
        assert_eq!(benchmark_median(&[ok(10), failed()]), None);
        assert_eq!(benchmark_median(&[ok(10), failed(), ok(30)]), Some(30));
        assert_eq!(benchmark_median(&[ok(10)]), Some(10));
        assert_eq!(
            benchmark_median(&[BenchmarkSample {
                ttft_ms: Some(10),
                total_ms: None,
                error: Some("late failure".to_owned())
            }]),
            None
        );
    }

    #[test]
    fn isolated_reachability_does_not_claim_an_unsent_key_is_invalid() {
        for code in [401, 403] {
            let result = classify_reachability_status(code, 12);
            assert!(result.reachable);
            assert_eq!(result.error_kind.as_deref(), Some("requires_auth"));
            assert!(!result.error_detail.unwrap().contains("密钥无效"));
        }
        assert!(classify_reachability_status(200, 12).reachable);
        assert!(classify_reachability_status(405, 12).reachable);
        assert!(!classify_reachability_status(503, 12).reachable);
    }

    #[test]
    fn median_prefers_conservative_middle() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[120]), Some(120));
        assert_eq!(median(&[300, 100, 200]), Some(200));
        assert_eq!(median(&[400, 100, 200, 300]), Some(300));
    }
}
