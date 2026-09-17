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
const TOKEN_PAGE_CAP: u32 = 20;
const DEVICE_NAME_MAX: usize = 40;

#[derive(Debug, Clone, Serialize)]
pub struct NewApiError {
    pub code: &'static str,
    pub message: &'static str,
}

impl std::fmt::Display for NewApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for NewApiError {}

impl NewApiError {
    fn invalid() -> Self {
        Self {
            code: "invalid_request",
            message: "站点地址或访问令牌无效，请检查后再试。",
        }
    }

    fn network() -> Self {
        Self {
            code: "network",
            message: "无法连接该中转站，请检查地址和网络后重试。",
        }
    }

    fn auth() -> Self {
        Self {
            code: "auth",
            message: "系统访问令牌无效或已过期，请到中转站控制台重新生成。",
        }
    }

    fn need_user() -> Self {
        Self {
            code: "need_user_id",
            message: "该中转站还需要填写数字用户 ID。可在控制台个人中心查看。",
        }
    }

    fn not_newapi() -> Self {
        Self {
            code: "not_newapi",
            message: "这个地址不像 new-api 中转站，请确认站点地址后重试。",
        }
    }

    fn failed() -> Self {
        Self {
            code: "failed",
            message: "中转站暂时无法完成操作，请稍后重试。",
        }
    }
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
        if looks_unauthorized(message) {
            return Err(NewApiError::auth());
        }
        return Err(NewApiError::failed());
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

fn looks_unauthorized(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    text.contains("无权")
        || text.contains("未登录")
        || text.contains("token")
        || text.contains("unauthorized")
        || text.contains("invalid")
        || text.contains("过期")
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
        return Err(NewApiError::auth());
    }
    if !status.is_success() {
        let message = value.get("message").and_then(Value::as_str).unwrap_or("");
        if needs_user_id(message) {
            return Err(NewApiError::need_user());
        }
        return Err(NewApiError::failed());
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

fn token_name(device_name: Option<&str>, group: &str) -> String {
    let device = device_name.unwrap_or("Niko").trim();
    let device = if device.is_empty() { "Niko" } else { device };
    let truncated: String = device.chars().take(DEVICE_NAME_MAX).collect();
    format!("Niko · {truncated} · {group}")
}

fn extract_key(value: &Value) -> Option<String> {
    value
        .get("key")
        .and_then(Value::as_str)
        .or_else(|| value.get("api_key").and_then(Value::as_str))
        .filter(|key| !key.is_empty())
        .map(|key| {
            if key.starts_with("sk-") {
                key.to_string()
            } else {
                format!("sk-{key}")
            }
        })
}

fn token_items(value: &Value) -> Vec<Value> {
    if let Some(items) = value.get("items").and_then(Value::as_array) {
        return items.clone();
    }
    if let Some(items) = value.as_array() {
        return items.clone();
    }
    Vec::new()
}

async fn find_existing_token(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: i64,
    group: &str,
    name: &str,
) -> Result<Option<(i64, Option<String>)>, NewApiError> {
    for page in 1..=TOKEN_PAGE_CAP {
        let list_url = format!(
            "{}?p={page}&size={TOKEN_PAGE_SIZE}",
            api_url(origin, "/token/")
        );
        let data = match get_json(client, &list_url, token, Some(user_id)).await {
            Ok(value) => value,
            Err(error) if error.code == "failed" && page == 1 => return Ok(None),
            Err(error) => return Err(error),
        };
        let items = token_items(&data);
        if items.is_empty() {
            return Ok(None);
        }
        for item in &items {
            let item_name = item.get("name").and_then(Value::as_str).unwrap_or("");
            let item_group = item.get("group").and_then(Value::as_str).unwrap_or("");
            let status = item.get("status").and_then(Value::as_i64).unwrap_or(1);
            if status != 1 {
                continue;
            }
            if item_group == group && (item_name == name || item_name.contains("Niko")) {
                let id = item
                    .get("id")
                    .and_then(Value::as_i64)
                    .ok_or(NewApiError::failed())?;
                return Ok(Some((id, extract_key(item))));
            }
        }
        if items.len() < TOKEN_PAGE_SIZE as usize {
            break;
        }
    }
    Ok(None)
}

async fn fetch_token_key(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    user_id: i64,
    token_id: i64,
) -> Result<String, NewApiError> {
    let url = api_url(origin, &format!("/token/{token_id}/key"));
    match get_json(client, &url, token, Some(user_id)).await {
        Ok(value) => extract_key(&value).ok_or(NewApiError::failed()),
        Err(_) => {
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
    }
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
                    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
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

pub async fn provision(req: ProvisionRequest) -> Result<ProvisionResult, NewApiError> {
    let origin = normalize_origin(&req.origin)?;
    let group = req.group.trim();
    if req.user_id <= 0 || req.access_token.trim().is_empty() || group.is_empty() {
        return Err(NewApiError::invalid());
    }
    let client = http_client()?;
    let token = req.access_token.trim();
    let name = token_name(req.device_name.as_deref(), group);
    if let Some((id, key)) =
        find_existing_token(&client, &origin, token, req.user_id, group, &name).await?
    {
        if let Some(api_key) = key {
            return Ok(ProvisionResult {
                api_key,
                token_id: id,
                group: group.to_string(),
            });
        }
        if let Ok(api_key) = fetch_token_key(&client, &origin, token, req.user_id, id).await {
            return Ok(ProvisionResult {
                api_key,
                token_id: id,
                group: group.to_string(),
            });
        }
    }

    let create_body = json!({
        "name": name,
        "remain_quota": 0,
        "expired_time": -1,
        "unlimited_quota": true,
        "group": group,
    });
    let created = match request_json(
        &client,
        reqwest::Method::POST,
        &api_url(&origin, "/token/"),
        token,
        Some(req.user_id),
        Some(create_body.clone()),
    )
    .await
    {
        Ok(value) => value,
        Err(_) => {
            request_json(
                &client,
                reqwest::Method::POST,
                &api_url(&origin, "/token"),
                token,
                Some(req.user_id),
                Some(create_body),
            )
            .await?
        }
    };
    let token_id = created
        .get("id")
        .and_then(Value::as_i64)
        .or_else(|| {
            created
                .get("data")
                .and_then(|data| data.get("id"))
                .and_then(Value::as_i64)
        })
        .unwrap_or(0);
    if let Some(api_key) = extract_key(&created) {
        return Ok(ProvisionResult {
            api_key,
            token_id,
            group: group.to_string(),
        });
    }
    if token_id > 0 {
        if let Ok(api_key) = fetch_token_key(&client, &origin, token, req.user_id, token_id).await {
            return Ok(ProvisionResult {
                api_key,
                token_id,
                group: group.to_string(),
            });
        }
    }
    if let Some((id, key)) =
        find_existing_token(&client, &origin, token, req.user_id, group, &name).await?
    {
        if let Some(api_key) = key {
            return Ok(ProvisionResult {
                api_key,
                token_id: id,
                group: group.to_string(),
            });
        }
        let api_key = fetch_token_key(&client, &origin, token, req.user_id, id).await?;
        return Ok(ProvisionResult {
            api_key,
            token_id: id,
            group: group.to_string(),
        });
    }
    Err(NewApiError::failed())
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
        assert_eq!(token_name(Some("macOS"), "claude"), "Niko · macOS · claude");
        assert_eq!(token_name(Some("  "), "default"), "Niko · Niko · default");
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
    }

    #[test]
    fn user_header_errors_are_classified() {
        assert!(needs_user_id("无权进行此操作，未提供 My-Api-User"));
        assert!(needs_user_id("New-Api-User 与登录用户不匹配"));
        assert!(!needs_user_id("余额不足"));
    }
}
