//! new-api 中转站：系统访问令牌 → 账号/分组/模型目录 → 按分组签发推理密钥。
//! 请求全部走 Rust，避免 WebView CORS，也不把令牌交给前端网络层。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const TOKEN_PAGE_SIZE: u32 = 100;
const TOKEN_PAGE_CAP: u32 = 3;
const DEVICE_NAME_MAX: usize = 40;
const TOKEN_NAME_MAX: usize = 50;

#[derive(Debug, Clone, Serialize)]
pub struct NewApiError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for NewApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NewApiError {}

impl NewApiError {
    fn invalid() -> Self {
        Self {
            code: "invalid_request",
            message: "站点地址或访问令牌无效，请检查后再试。".to_string(),
        }
    }

    fn network() -> Self {
        Self {
            code: "network",
            message: "无法连接该中转站，请检查地址和网络后重试。".to_string(),
        }
    }

    fn auth() -> Self {
        Self {
            code: "auth",
            message: "系统访问令牌无效或已过期，请到中转站控制台重新生成。".to_string(),
        }
    }

    fn need_user() -> Self {
        Self {
            code: "need_user_id",
            message: "该中转站还需要填写数字用户 ID。可在控制台个人中心查看。".to_string(),
        }
    }

    fn not_newapi() -> Self {
        Self {
            code: "not_newapi",
            message: "这个地址不像 new-api 中转站，请确认站点地址后重试。".to_string(),
        }
    }

    fn failed() -> Self {
        Self {
            code: "failed",
            message: "中转站暂时无法完成操作，请稍后重试。".to_string(),
        }
    }

    fn failed_with(message: &str) -> Self {
        Self {
            code: "failed",
            message: sanitize_station_message(message),
        }
    }
}

fn sanitize_station_message(message: &str) -> String {
    let redacted = crate::logx::redact_line(message);
    let trimmed = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() {
        return NewApiError::failed().message;
    }
    let clipped: String = trimmed.chars().take(80).collect();
    let has_cjk = clipped.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    if has_cjk {
        return clipped;
    }
    if looks_name_conflict(&clipped) {
        return "这个分组已有密钥，请再点一次接入。".to_string();
    }
    NewApiError::failed().message
}

#[derive(Debug, Deserialize)]
pub struct ConnectRequest {
    pub origin: String,
    pub access_token: String,
    pub user_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ConnectResult {
    pub origin: String,
    pub user_id: i64,
    pub username: String,
    pub quota: f64,
    pub group: String,
    pub quota_per_unit: Option<Value>,
    pub system_name: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthenticatedRequest {
    pub origin: String,
    pub access_token: String,
    pub user_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct ProvisionRequest {
    pub origin: String,
    pub access_token: String,
    pub user_id: i64,
    pub group: String,
    pub device_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProvisionResult {
    pub api_key: String,
    pub token_id: i64,
    pub group: String,
}

#[derive(Debug, Deserialize)]
pub struct UsageRequest {
    pub origin: String,
    pub access_token: String,
    pub user_id: i64,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub start_timestamp: Option<i64>,
    pub end_timestamp: Option<i64>,
    pub group: Option<String>,
    pub model_name: Option<String>,
}

fn http_client() -> Result<reqwest::Client, NewApiError> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|_| NewApiError::failed())
}

fn blocked_host(host: &str) -> bool {
    matches!(
        host,
        "0.0.0.0" | "169.254.169.254" | "metadata.google.internal" | "[::]" | "[::0]"
    )
}

fn blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_unspecified()
                || v4 == Ipv4Addr::new(169, 254, 169, 254)
                || v4.octets()[0] == 224
        }
        IpAddr::V6(v6) => v6.is_unspecified() || is_ipv4_mapped_metadata(v6),
    }
}

fn is_ipv4_mapped_metadata(ip: Ipv6Addr) -> bool {
    ip.to_ipv4_mapped() == Some(Ipv4Addr::new(169, 254, 169, 254))
}

pub fn normalize_origin(input: &str) -> Result<String, NewApiError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(NewApiError::invalid());
    }
    let raw = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = reqwest::Url::parse(&raw).map_err(|_| NewApiError::invalid())?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return Err(NewApiError::invalid());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(NewApiError::invalid());
    }
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if host.is_empty() || blocked_host(&host) {
        return Err(NewApiError::invalid());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if blocked_ip(ip) {
            return Err(NewApiError::invalid());
        }
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    if path == "/v1" || path == "/api" {
        path.clear();
    }
    let host_port = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    Ok(format!("{}://{}{}", url.scheme(), host_port, path))
}

fn api_url(origin: &str, path: &str) -> String {
    format!("{origin}/api{path}")
}

fn envelope_success(value: &Value) -> Result<&Value, NewApiError> {
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        let message = value.get("message").and_then(Value::as_str).unwrap_or("");
        if needs_user_id(message) {
            return Err(NewApiError::need_user());
        }
        if looks_name_conflict(message) {
            return Err(NewApiError::failed_with(message));
        }
        if looks_unauthorized(message) {
            return Err(NewApiError::auth());
        }
        return Err(NewApiError::failed_with(message));
    }
    Ok(value.get("data").unwrap_or(value))
}

fn needs_user_id(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    text.contains("new-api-user")
        || text.contains("my-api-user")
        || text.contains("未提供") && text.contains("user")
        || text.contains("与登录用户不匹配")
}

fn looks_name_conflict(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    text.contains("名称已存在")
        || text.contains("already exist")
        || text.contains("duplicate")
        || (text.contains("已存在")
            && (text.contains("令牌") || text.contains("token") || text.contains("name")))
}

fn looks_unauthorized(message: &str) -> bool {
    if looks_name_conflict(message) {
        return false;
    }
    let text = message.to_ascii_lowercase();
    if text.contains("无权") && !(text.contains("未登录") || text.contains("token") || text.contains("令牌"))
    {
        return false;
    }
    text.contains("未登录")
        || text.contains("unauthorized")
        || text.contains("invalid token")
        || text.contains("invalid access")
        || text.contains("access token")
        || (text.contains("过期")
            && (text.contains("token") || text.contains("登录") || text.contains("令牌")))
}

fn looks_like_status(value: &Value) -> bool {
    value.get("quota_per_unit").is_some()
        || value.get("system_name").is_some()
        || value.get("version").is_some()
        || value.get("start_time").is_some()
}

async fn read_json(response: reqwest::Response) -> Result<Value, NewApiError> {
    let status = response.status();
    let bytes = response.bytes().await.map_err(|_| NewApiError::network())?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(NewApiError::failed());
    }
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if status.as_u16() == 401 || status.as_u16() == 403 {
        let message = value.get("message").and_then(Value::as_str).unwrap_or("");
        if needs_user_id(message) {
            return Err(NewApiError::need_user());
        }
        if looks_name_conflict(message) {
            return Err(NewApiError::failed_with(message));
        }
        return Err(NewApiError::auth());
    }
    if !status.is_success() {
        let message = value.get("message").and_then(Value::as_str).unwrap_or("");
        if needs_user_id(message) {
            return Err(NewApiError::need_user());
        }
        return Err(NewApiError::failed_with(message));
    }
    envelope_success(&value).cloned()
}

async fn request_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    token: &str,
    user_id: Option<i64>,
    body: Option<Value>,
) -> Result<Value, NewApiError> {
    let mut builder = client
        .request(method, url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json");
    if let Some(id) = user_id {
        let header = id.to_string();
        builder = builder
            .header("New-Api-User", &header)
            .header("My-Api-User", &header);
    }
    if let Some(payload) = body {
        builder = builder.json(&payload);
    }
    let response = builder.send().await.map_err(|error| {
        if error.is_timeout() || error.is_connect() || error.is_request() {
            NewApiError::network()
        } else {
            NewApiError::failed()
        }
    })?;
    read_json(response).await
}

async fn get_json(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    user_id: Option<i64>,
) -> Result<Value, NewApiError> {
    request_json(client, reqwest::Method::GET, url, token, user_id, None).await
}

fn user_id_from_value(value: &Value) -> Option<i64> {
    value
        .get("id")
        .and_then(|item| item.as_i64().or_else(|| item.as_u64().map(|n| n as i64)))
        .filter(|id| *id > 0)
}

fn user_name_from_value(value: &Value) -> String {
    value
        .get("display_name")
        .and_then(Value::as_str)
        .or_else(|| value.get("username").and_then(Value::as_str))
        .unwrap_or("已连接")
        .to_string()
}

async fn fetch_user(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: Option<i64>,
) -> Result<Value, NewApiError> {
    get_json(client, &api_url(origin, "/user/self"), token, user_id).await
}

async fn resolve_user(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    hinted: Option<i64>,
) -> Result<(i64, Value), NewApiError> {
    match fetch_user(client, origin, token, hinted.filter(|id| *id > 0)).await {
        Ok(user) => {
            let id = user_id_from_value(&user).or(hinted.filter(|id| *id > 0));
            let id = id.ok_or(NewApiError::need_user())?;
            if hinted.filter(|hint| *hint > 0 && *hint != id).is_some() {
                return Err(NewApiError::need_user());
            }
            Ok((id, user))
        }
        Err(error) if error.code == "need_user_id" && hinted.filter(|id| *id > 0).is_none() => {
            Err(error)
        }
        Err(error) => Err(error),
    }
}

fn ascii_slug(input: &str, fallback: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if ch == '-' || ch == '_' {
            if !out.ends_with('-') {
                out.push('-');
            }
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn token_name(device_name: Option<&str>, group: &str) -> String {
    let device = ascii_slug(device_name.unwrap_or("macOS"), "macOS");
    let truncated_device: String = device.chars().take(DEVICE_NAME_MAX).collect();
    let group_slug = ascii_slug(group, "default");
    let prefix = format!("Niko-{truncated_device}-");
    let prefix_len = prefix.chars().count();
    if prefix_len >= TOKEN_NAME_MAX {
        return prefix.chars().take(TOKEN_NAME_MAX).collect();
    }
    let remain = TOKEN_NAME_MAX - prefix_len;
    let truncated_group: String = group_slug.chars().take(remain).collect();
    format!("{prefix}{truncated_group}")
}

fn with_sk_prefix(key: &str) -> String {
    if key.starts_with("sk-") {
        key.to_string()
    } else {
        format!("sk-{key}")
    }
}

fn extract_key(value: &Value) -> Option<String> {
    let candidates = [
        value.get("key").and_then(Value::as_str),
        value.get("api_key").and_then(Value::as_str),
        value.get("token").and_then(Value::as_str),
        value.pointer("/data/key").and_then(Value::as_str),
        value.pointer("/data/api_key").and_then(Value::as_str),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|key| !key.is_empty())
        .map(with_sk_prefix)
}

fn token_items(value: &Value) -> Vec<Value> {
    if let Some(items) = value.get("items").and_then(Value::as_array) {
        return items.clone();
    }
    if let Some(items) = value.get("records").and_then(Value::as_array) {
        return items.clone();
    }
    if let Some(data) = value.get("data") {
        if let Some(items) = data.as_array() {
            return items.clone();
        }
        if let Some(items) = data.get("items").and_then(Value::as_array) {
            return items.clone();
        }
        if let Some(items) = data.get("records").and_then(Value::as_array) {
            return items.clone();
        }
    }
    if let Some(items) = value.as_array() {
        return items.clone();
    }
    Vec::new()
}

fn percent_encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn token_id_from_value(value: &Value) -> Option<i64> {
    value
        .get("id")
        .and_then(|item| item.as_i64().or_else(|| item.as_u64().map(|n| n as i64)))
        .filter(|id| *id > 0)
}

fn match_token_item(item: &Value, group: &str, name: &str) -> Option<(i64, Option<String>)> {
    let item_name = item.get("name").and_then(Value::as_str).unwrap_or("");
    let item_group = item.get("group").and_then(Value::as_str).unwrap_or("");
    let status = item.get("status").and_then(Value::as_i64).unwrap_or(1);
    if status != 1 {
        return None;
    }
    let group_ok = item_group.eq_ignore_ascii_case(group);
    let name_ok = item_name == name
        || item_name.starts_with("Niko-")
        || item_name.starts_with("Niko ·")
        || item_name.contains("Niko");
    if group_ok && name_ok {
        let id = token_id_from_value(item)?;
        return Some((id, extract_key(item)));
    }
    None
}

fn match_any_group_token(item: &Value, group: &str) -> Option<(i64, Option<String>)> {
    let item_group = item.get("group").and_then(Value::as_str).unwrap_or("");
    let status = item.get("status").and_then(Value::as_i64).unwrap_or(1);
    if status != 1 || !item_group.eq_ignore_ascii_case(group) {
        return None;
    }
    let id = token_id_from_value(item)?;
    Some((id, extract_key(item)))
}

async fn search_tokens_by_keyword(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: i64,
    keyword: &str,
) -> Result<Vec<Value>, NewApiError> {
    let encoded = percent_encode(keyword);
    let urls = [
        api_url(origin, &format!("/token/?keyword={encoded}")),
        api_url(origin, &format!("/token/search?keyword={encoded}")),
        api_url(origin, &format!("/token/?search={encoded}")),
    ];
    for url in urls {
        if let Ok(value) = get_json(client, &url, token, Some(user_id)).await {
            let items = token_items(&value);
            if !items.is_empty() {
                return Ok(items);
            }
        }
    }
    Ok(Vec::new())
}

async fn list_tokens_page(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: i64,
    page: u32,
) -> Result<Value, NewApiError> {
    let with_slash = format!(
        "{}?p={page}&size={TOKEN_PAGE_SIZE}",
        api_url(origin, "/token/")
    );
    match get_json(client, &with_slash, token, Some(user_id)).await {
        Ok(value) => Ok(value),
        Err(error) if error.code == "failed" && page <= 1 => {
            let without_slash = format!(
                "{}?p={page}&size={TOKEN_PAGE_SIZE}",
                api_url(origin, "/token")
            );
            get_json(client, &without_slash, token, Some(user_id)).await
        }
        Err(error) => Err(error),
    }
}

async fn collect_token_pages(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: i64,
    name: &str,
) -> Result<Vec<Value>, NewApiError> {
    let mut items = search_tokens_by_keyword(client, origin, token, user_id, name)
        .await
        .unwrap_or_default();
    if items.is_empty() {
        items = search_tokens_by_keyword(client, origin, token, user_id, "Niko")
            .await
            .unwrap_or_default();
    }
    for page in 0..=TOKEN_PAGE_CAP {
        let data = match list_tokens_page(client, origin, token, user_id, page).await {
            Ok(value) => value,
            Err(error) if error.code == "failed" && page <= 1 => break,
            Err(error) if error.code == "auth" => return Err(error),
            Err(_) => break,
        };
        let page_items = token_items(&data);
        if page_items.is_empty() {
            if page == 0 {
                continue;
            }
            break;
        }
        items.extend(page_items.iter().cloned());
        if page_items.len() < TOKEN_PAGE_SIZE as usize {
            break;
        }
    }
    Ok(items)
}

async fn find_existing_token(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: i64,
    group: &str,
    name: &str,
) -> Result<Option<(i64, Option<String>)>, NewApiError> {
    let items = collect_token_pages(client, origin, token, user_id, name).await?;
    for item in &items {
        if let Some(found) = match_token_item(item, group, name) {
            return Ok(Some(found));
        }
    }
    for item in &items {
        if let Some(found) = match_any_group_token(item, group) {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

async fn resolve_token_key(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: i64,
    id: i64,
    key: Option<String>,
) -> Option<String> {
    if let Some(api_key) = key {
        return Some(api_key);
    }
    match fetch_token_key(client, origin, token, user_id, id).await {
        Ok(api_key) => Some(api_key),
        Err(error) => {
            crate::logx::append(
                "newapi_provision",
                &format!("token {id} key missing: {}", error.message),
            );
            None
        }
    }
}

async fn fetch_token_key(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: i64,
    token_id: i64,
) -> Result<String, NewApiError> {
    let urls = [
        api_url(origin, &format!("/token/{token_id}/key")),
        api_url(origin, &format!("/token/{token_id}")),
    ];
    for url in urls {
        if let Ok(value) = get_json(client, &url, token, Some(user_id)).await {
            if let Some(key) = extract_key(&value).or_else(|| {
                token_items(&value)
                    .into_iter()
                    .find_map(|item| extract_key(&item))
            }) {
                return Ok(key);
            }
        }
    }
    let search = api_url(origin, &format!("/token/search?keyword={token_id}"));
    let value = get_json(client, &search, token, Some(user_id)).await?;
    extract_key(&value)
        .or_else(|| {
            token_items(&value)
                .into_iter()
                .find_map(|item| extract_key(&item))
        })
        .ok_or(NewApiError::failed())
}

pub async fn connect(req: ConnectRequest) -> Result<ConnectResult, NewApiError> {
    let origin = normalize_origin(&req.origin)?;
    let token = req.access_token.trim();
    if token.is_empty() {
        return Err(NewApiError::invalid());
    }
    let client = http_client()?;
    let status = client
        .get(api_url(&origin, "/status"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| NewApiError::network())?;
    let status_json = read_json(status).await.unwrap_or(Value::Null);
    if !looks_like_status(&status_json) && status_json != Value::Null {
        // 某些部署把 status 包在 data 里已经解开；空对象仍允许继续，靠 /user/self 判定。
        if status_json.as_object().map(|o| o.is_empty()).unwrap_or(false) {
            return Err(NewApiError::not_newapi());
        }
    }

    let (user_id, user) = resolve_user(&client, &origin, token, req.user_id).await?;
    Ok(ConnectResult {
        origin,
        user_id,
        username: user_name_from_value(&user),
        quota: user.get("quota").and_then(Value::as_f64).unwrap_or(0.0),
        group: user
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        quota_per_unit: status_json.get("quota_per_unit").cloned(),
        system_name: status_json
            .get("system_name")
            .and_then(Value::as_str)
            .unwrap_or("new-api")
            .to_string(),
    })
}

pub async fn bootstrap(req: AuthenticatedRequest) -> Result<Value, NewApiError> {
    let origin = normalize_origin(&req.origin)?;
    if req.user_id <= 0 || req.access_token.trim().is_empty() {
        return Err(NewApiError::invalid());
    }
    let client = http_client()?;
    let token = req.access_token.trim();
    let user_id = Some(req.user_id);
    let status_url = api_url(&origin, "/status");
    let user_url = api_url(&origin, "/user/self");
    let groups_url = api_url(&origin, "/user/self/groups");
    let models_url = api_url(&origin, "/user/models");
    let self_models_url = api_url(&origin, "/user/self/models");
    let pricing_url = api_url(&origin, "/pricing");
    let (status, user, groups, models, pricing) = tokio::join!(
        async {
            match client
                .get(&status_url)
                .header("Accept", "application/json")
                .send()
                .await
            {
                Ok(response) => read_json(response).await.unwrap_or(Value::Null),
                Err(_) => Value::Null,
            }
        },
        get_json(&client, &user_url, token, user_id),
        get_json(&client, &groups_url, token, user_id),
        async {
            match get_json(&client, &models_url, token, user_id).await {
                Ok(value) => value,
                Err(_) => get_json(&client, &self_models_url, token, user_id)
                    .await
                    .unwrap_or(Value::Null),
            }
        },
        async {
            match client
                .get(&pricing_url)
                .header("Accept", "application/json")
                .send()
                .await
            {
                Ok(response) => {
                    let bytes = response.bytes().await.unwrap_or_default();
                    if bytes.len() > MAX_BODY_BYTES {
                        Value::Null
                    } else {
                        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
                    }
                }
                Err(_) => Value::Null,
            }
        },
    );
    let user = user?;
    let groups = groups.unwrap_or(Value::Null);
    Ok(json!({
        "origin": origin,
        "status": status,
        "user": user,
        "groups": groups,
        "models": models,
        "pricing": pricing,
    }))
}

pub async fn pricing(origin: String) -> Result<Value, NewApiError> {
    let origin = normalize_origin(&origin)?;
    let client = http_client()?;
    let response = client
        .get(api_url(&origin, "/pricing"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| NewApiError::network())?;
    let bytes = response.bytes().await.map_err(|_| NewApiError::network())?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(NewApiError::failed());
    }
    serde_json::from_slice(&bytes).map_err(|_| NewApiError::failed())
}

pub async fn status(origin: String) -> Result<Value, NewApiError> {
    let origin = normalize_origin(&origin)?;
    let client = http_client()?;
    let response = client
        .get(api_url(&origin, "/status"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| NewApiError::network())?;
    read_json(response).await
}

fn provision_ok(api_key: String, token_id: i64, group: &str) -> ProvisionResult {
    ProvisionResult {
        api_key,
        token_id,
        group: group.to_string(),
    }
}

async fn try_reuse_existing(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: i64,
    group: &str,
    name: &str,
) -> Result<Option<ProvisionResult>, NewApiError> {
    let Some((id, key)) = find_existing_token(client, origin, token, user_id, group, name).await?
    else {
        return Ok(None);
    };
    if let Some(api_key) = resolve_token_key(client, origin, token, user_id, id, key).await {
        crate::logx::append(
            "newapi_provision",
            &format!("reused token {id} group={group}"),
        );
        return Ok(Some(provision_ok(api_key, id, group)));
    }
    Ok(None)
}

fn create_bodies(name: &str, group: &str) -> Vec<Value> {
    // 只创建一把无限额度密钥。站点若拒无限额度，再退回有额度的请求体。
    vec![
        json!({
            "name": name,
            "remain_quota": 0,
            "expired_time": -1,
            "unlimited_quota": true,
            "group": group,
        }),
        json!({
            "name": name,
            "remain_quota": 500_000,
            "expired_time": -1,
            "unlimited_quota": false,
            "group": group,
        }),
    ]
}

async fn create_token(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: i64,
    body: Value,
) -> Result<Value, NewApiError> {
    let urls = [
        api_url(origin, "/token/"),
        api_url(origin, "/token"),
        api_url(origin, "/user/token"),
    ];
    let mut last = NewApiError::failed();
    for url in urls {
        match request_json(
            client,
            reqwest::Method::POST,
            &url,
            token,
            Some(user_id),
            Some(body.clone()),
        )
        .await
        {
            Ok(value) => return Ok(value),
            Err(error) if error.code == "auth" => return Err(error),
            Err(error) => last = error,
        }
    }
    Err(last)
}

pub async fn provision(req: ProvisionRequest) -> Result<ProvisionResult, NewApiError> {
    let origin = normalize_origin(&req.origin)?;
    let group = req.group.trim();
    if req.user_id <= 0 || req.access_token.trim().is_empty() || group.is_empty() {
        return Err(NewApiError::invalid());
    }
    let client = http_client()?;
    let token = req.access_token.trim();
    let name = token_name(req.device_name.as_deref(), group);
    crate::logx::append(
        "newapi_provision",
        &format!("start group={group} name={name} user={}", req.user_id),
    );

    if let Some(result) =
        try_reuse_existing(&client, &origin, token, req.user_id, group, &name).await?
    {
        return Ok(result);
    }

    let mut last_error = NewApiError::failed();
    for body in create_bodies(&name, group) {
        match create_token(&client, &origin, token, req.user_id, body).await {
            Ok(created) => {
                let token_id = token_id_from_value(&created)
                    .or_else(|| created.get("data").and_then(token_id_from_value))
                    .unwrap_or(0);
                crate::logx::append(
                    "newapi_provision",
                    &format!("created token {token_id} group={group}"),
                );
                if let Some(api_key) = resolve_token_key(
                    &client,
                    &origin,
                    token,
                    req.user_id,
                    token_id,
                    extract_key(&created),
                )
                .await
                {
                    return Ok(provision_ok(api_key, token_id, group));
                }
                // 站点已经建成功，只是响应里没带回密钥：立刻复用，禁止再创建。
                if let Some(result) =
                    try_reuse_existing(&client, &origin, token, req.user_id, group, &name).await?
                {
                    return Ok(result);
                }
                last_error = NewApiError::failed_with("中转站创建了密钥但没有返回可用内容");
                break;
            }
            Err(error) => {
                crate::logx::append(
                    "newapi_provision",
                    &format!("create failed: {}", error.message),
                );
                last_error = error;
                if last_error.code == "auth" {
                    return Err(last_error);
                }
            }
        }
    }

    if let Some(result) =
        try_reuse_existing(&client, &origin, token, req.user_id, group, &name).await?
    {
        return Ok(result);
    }
    Err(last_error)
}

pub async fn usage(req: UsageRequest) -> Result<Value, NewApiError> {
    let origin = normalize_origin(&req.origin)?;
    if req.user_id <= 0 || req.access_token.trim().is_empty() {
        return Err(NewApiError::invalid());
    }
    let client = http_client()?;
    let mut query = vec![
        ("p".to_string(), req.page.unwrap_or(1).to_string()),
        (
            "page_size".to_string(),
            req.page_size.unwrap_or(100).to_string(),
        ),
        ("type".to_string(), "2".to_string()),
    ];
    if let Some(start) = req.start_timestamp {
        query.push(("start_timestamp".to_string(), start.to_string()));
    }
    if let Some(end) = req.end_timestamp {
        query.push(("end_timestamp".to_string(), end.to_string()));
    }
    if let Some(group) = req.group.as_deref().filter(|value| !value.is_empty()) {
        query.push(("group".to_string(), group.to_string()));
    }
    if let Some(model) = req.model_name.as_deref().filter(|value| !value.is_empty()) {
        query.push(("model_name".to_string(), model.to_string()));
    }
    let encoded = query
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    get_json(
        &client,
        &format!("{}?{encoded}", api_url(&origin, "/log/self")),
        req.access_token.trim(),
        Some(req.user_id),
    )
    .await
}

#[tauri::command]
pub async fn newapi_connect(req: ConnectRequest) -> Result<ConnectResult, NewApiError> {
    connect(req).await
}

#[tauri::command]
pub async fn newapi_bootstrap(req: AuthenticatedRequest) -> Result<Value, NewApiError> {
    bootstrap(req).await
}

#[tauri::command]
pub async fn newapi_provision(req: ProvisionRequest) -> Result<ProvisionResult, NewApiError> {
    provision(req).await
}

#[tauri::command]
pub async fn newapi_pricing(origin: String) -> Result<Value, NewApiError> {
    pricing(origin).await
}

#[tauri::command]
pub async fn newapi_status(origin: String) -> Result<Value, NewApiError> {
    status(origin).await
}

#[tauri::command]
pub async fn newapi_usage(req: UsageRequest) -> Result<Value, NewApiError> {
    usage(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_normalization_strips_api_suffix() {
        assert_eq!(
            normalize_origin("https://relay.example.com/v1/").unwrap(),
            "https://relay.example.com"
        );
        assert_eq!(
            normalize_origin("relay.example.com/api").unwrap(),
            "https://relay.example.com"
        );
        assert_eq!(
            normalize_origin("http://192.168.1.8:3000").unwrap(),
            "http://192.168.1.8:3000"
        );
    }

    #[test]
    fn origin_normalization_rejects_credentials_and_metadata() {
        assert!(normalize_origin("javascript:alert(1)").is_err());
        assert!(normalize_origin("https://user:pass@example.com").is_err());
        assert!(normalize_origin("https://169.254.169.254").is_err());
        assert!(normalize_origin("").is_err());
    }

    #[test]
    fn token_names_include_device_and_group() {
        assert_eq!(token_name(Some("macOS"), "claude"), "Niko-macOS-claude");
        assert_eq!(token_name(Some("  "), "default"), "Niko-macOS-default");
        let long = token_name(Some("macOS"), &"g".repeat(80));
        assert!(long.chars().count() <= TOKEN_NAME_MAX);
        assert!(long.starts_with("Niko-macOS-"));
        assert_eq!(ascii_slug("Claude 分组", "default"), "Claude");
    }

    #[test]
    fn create_bodies_prefer_unlimited_and_do_not_spam() {
        let bodies = create_bodies("Niko-macOS-gpt", "gpt");
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0]["unlimited_quota"], json!(true));
        assert_eq!(bodies[0]["remain_quota"], json!(0));
        assert_eq!(bodies[1]["unlimited_quota"], json!(false));
    }

    #[test]
    fn keys_gain_sk_prefix_when_missing() {
        assert_eq!(
            extract_key(&json!({"key": "abc"})).as_deref(),
            Some("sk-abc")
        );
        assert_eq!(
            extract_key(&json!({"key": "sk-abc"})).as_deref(),
            Some("sk-abc")
        );
        assert_eq!(
            extract_key(&json!({"data": {"api_key": "xyz"}})).as_deref(),
            Some("sk-xyz")
        );
    }

    #[test]
    fn token_items_read_common_envelopes() {
        assert_eq!(
            token_items(&json!({"items": [{"id": 1}]})).len(),
            1
        );
        assert_eq!(
            token_items(&json!({"data": {"items": [{"id": 2}, {"id": 3}]}})).len(),
            2
        );
        assert_eq!(token_items(&json!([{"id": 4}])).len(), 1);
        assert_eq!(
            token_items(&json!({"data": {"records": [{"id": 5}]}})).len(),
            1
        );
    }

    #[test]
    fn name_conflicts_are_not_auth_errors() {
        assert!(looks_name_conflict("令牌名称已存在"));
        assert!(!looks_unauthorized("令牌名称已存在"));
        assert!(looks_unauthorized("系统访问令牌无效或已过期"));
        assert!(!looks_unauthorized("余额不足"));
        assert!(!looks_unauthorized("无权进行此操作"));
        assert!(looks_unauthorized("无权进行此操作，未登录"));
    }

    #[test]
    fn user_header_errors_are_classified() {
        assert!(needs_user_id("无权进行此操作，未提供 My-Api-User"));
        assert!(needs_user_id("New-Api-User 与登录用户不匹配"));
        assert!(!needs_user_id("余额不足"));
    }
}
