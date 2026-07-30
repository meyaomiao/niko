use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use cookie::{Cookie, SameSite};
use reqwest::{
    header::{HeaderMap, HeaderValue, CONTENT_TYPE, COOKIE, ORIGIN, RETRY_AFTER, SET_COOKIE},
    Client, StatusCode,
};
use serde::{Deserialize, Serialize};
use tauri::{Manager, WebviewUrl, WindowEvent};
use zeroize::Zeroize;

const NIKO_ORIGIN: &str = "https://niko-ai.cc";
const CONFIG_URL: &str = "https://niko-ai.cc/api/niko/v1/config";
const REGISTER_URL: &str = "https://niko-ai.cc/api/niko/v1/auth/register";
const VERIFY_URL: &str = "https://niko-ai.cc/desktop/verify/";
const VERIFY_WINDOW_LABEL: &str = "register-verification";
const CSRF_COOKIE_NAME: &str = "__Host-niko_csrf";
const CHALLENGE_TTL: Duration = Duration::from_secs(5 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: usize = 32 * 1024;

struct SensitiveString(String);

impl SensitiveString {
    fn new(value: String) -> Self {
        Self(value)
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl Drop for SensitiveString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Deserialize)]
#[serde(transparent)]
struct SensitiveInput(String);

impl SensitiveInput {
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl Drop for SensitiveInput {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug, Serialize)]
pub struct RegistrationError {
    code: &'static str,
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_seconds: Option<u64>,
}

impl RegistrationError {
    fn new(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            retry_after_seconds: None,
        }
    }

    fn rate_limited(retry_after_seconds: Option<u64>) -> Self {
        Self {
            code: "RATE_LIMITED",
            message: "操作过于频繁，请稍后重试。",
            retry_after_seconds,
        }
    }
}

#[derive(Deserialize)]
struct ConfigEnvelope {
    data: ConfigData,
}

#[derive(Deserialize)]
struct ConfigData {
    csrf_token: String,
    turnstile_site_key: String,
    turnstile_required: bool,
}

#[derive(Deserialize)]
struct RegistrationSuccessEnvelope {
    success: bool,
    data: RegistrationSuccessData,
}

#[derive(Deserialize)]
struct RegistrationSuccessData {
    login_required: bool,
    account: RegistrationSuccessAccount,
}

#[derive(Deserialize)]
struct RegistrationSuccessAccount {
    username: String,
}

struct RegistrationSession {
    nonce: String,
    client: Client,
    csrf_token: SensitiveString,
    turnstile_token: Option<SensitiveString>,
    expires_at: Instant,
}

struct RegistrationExchange {
    client: Client,
    csrf_token: SensitiveString,
    turnstile_token: SensitiveString,
}

#[derive(Clone, Default)]
pub struct RegistrationState {
    inner: Arc<Mutex<Option<RegistrationSession>>>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ChallengeState {
    Pending,
    Verified,
    Expired,
    Missing,
}

#[derive(Serialize)]
pub struct ChallengeStart {
    nonce: String,
    expires_in_seconds: u64,
}

#[derive(Serialize)]
pub struct ChallengeStatus {
    status: ChallengeState,
}

#[derive(Deserialize)]
pub struct RegisterAccountRequest {
    nonce: String,
    username: String,
    password: SensitiveInput,
}

#[derive(Serialize)]
pub struct RegisterAccountResponse {
    username: String,
}

impl RegistrationState {
    fn replace(&self, session: RegistrationSession) -> Result<(), RegistrationError> {
        *self.lock()? = Some(session);
        Ok(())
    }

    fn status(&self, nonce: &str) -> Result<ChallengeState, RegistrationError> {
        let mut guard = self.lock()?;
        let Some(session) = guard.as_ref() else {
            return Ok(ChallengeState::Missing);
        };
        if session.nonce != nonce {
            return Ok(ChallengeState::Missing);
        }
        if session.expires_at <= Instant::now() {
            *guard = None;
            return Ok(ChallengeState::Expired);
        }
        Ok(if session.turnstile_token.is_some() {
            ChallengeState::Verified
        } else {
            ChallengeState::Pending
        })
    }

    fn complete(&self, nonce: &str, token: String) -> Result<(), RegistrationError> {
        if token.len() < 20
            || token.len() > 4096
            || token.chars().any(|character| character.is_control())
        {
            return Err(RegistrationError::new(
                "TURNSTILE_FAILED",
                "安全验证返回了无效结果。",
            ));
        }
        let mut guard = self.lock()?;
        let Some(session) = guard.as_mut() else {
            return Err(verification_required());
        };
        if session.nonce != nonce {
            return Err(verification_required());
        }
        if session.expires_at <= Instant::now() {
            *guard = None;
            return Err(challenge_expired());
        }
        if session.turnstile_token.is_some() {
            return Err(verification_required());
        }
        session.turnstile_token = Some(SensitiveString::new(token));
        Ok(())
    }

    fn take_verified(&self, nonce: &str) -> Result<RegistrationExchange, RegistrationError> {
        let mut guard = self.lock()?;
        let Some(session) = guard.as_ref() else {
            return Err(verification_required());
        };
        if session.nonce != nonce {
            return Err(verification_required());
        }
        if session.expires_at <= Instant::now() {
            *guard = None;
            return Err(challenge_expired());
        }
        if session.turnstile_token.is_none() {
            return Err(verification_required());
        }

        let mut session = guard.take().expect("registration session checked above");
        Ok(RegistrationExchange {
            client: session.client,
            csrf_token: session.csrf_token,
            turnstile_token: session
                .turnstile_token
                .take()
                .expect("Turnstile token checked above"),
        })
    }

    fn clear_if_nonce(&self, nonce: &str) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        if guard.as_ref().is_some_and(|session| session.nonce == nonce) {
            *guard = None;
            true
        } else {
            false
        }
    }

    fn clear_if_pending(&self, nonce: &str) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        if guard
            .as_ref()
            .is_some_and(|session| session.nonce == nonce && session.turnstile_token.is_none())
        {
            *guard = None;
        }
    }

    fn clear(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = None;
        }
    }

    fn lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<RegistrationSession>>, RegistrationError> {
        self.inner
            .lock()
            .map_err(|_| RegistrationError::new("REGISTRATION_STATE_ERROR", "注册状态暂时不可用。"))
    }
}

fn verification_required() -> RegistrationError {
    RegistrationError::new("VERIFICATION_REQUIRED", "请先完成安全验证。")
}

fn challenge_expired() -> RegistrationError {
    RegistrationError::new("CHALLENGE_EXPIRED", "安全验证已过期，请重新验证。")
}

fn build_http_client() -> Result<Client, RegistrationError> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!("Niko-Desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| RegistrationError::new("VERIFICATION_UNAVAILABLE", "安全验证暂时不可用。"))
}

fn random_nonce() -> Result<String, RegistrationError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| RegistrationError::new("VERIFICATION_UNAVAILABLE", "安全验证暂时不可用。"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

async fn response_bytes(mut response: reqwest::Response) -> Result<Vec<u8>, RegistrationError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(RegistrationError::new(
                "INVALID_RESPONSE",
                "账户服务返回了无效结果。",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn network_error(error: reqwest::Error) -> RegistrationError {
    if error.is_timeout() {
        RegistrationError::new("TIMEOUT", "注册请求超时，请稍后重试。")
    } else {
        RegistrationError::new("NETWORK_ERROR", "网络连接失败，请检查连接后重试。")
    }
}

fn valid_csrf(value: &str) -> bool {
    (32..=256).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_turnstile_site_key(value: &str) -> bool {
    (20..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn extract_csrf_cookie(
    headers: &HeaderMap,
    expected_token: &str,
) -> Result<SensitiveString, RegistrationError> {
    if !valid_csrf(expected_token) {
        return Err(RegistrationError::new(
            "INVALID_CONFIG",
            "安全验证配置无效。",
        ));
    }

    for value in headers.get_all(SET_COOKIE) {
        let Ok(raw) = value.to_str() else {
            continue;
        };
        let Ok(cookie) = Cookie::parse(raw.to_owned()) else {
            continue;
        };
        if cookie.name() != CSRF_COOKIE_NAME {
            continue;
        }
        if cookie.value() != expected_token
            || cookie.secure() != Some(true)
            || cookie.path() != Some("/")
            || cookie.domain().is_some()
            || cookie.same_site() != Some(SameSite::Lax)
        {
            break;
        }
        return Ok(SensitiveString::new(expected_token.to_owned()));
    }

    Err(RegistrationError::new(
        "INVALID_CONFIG",
        "安全验证配置无效。",
    ))
}

async fn initialize_session(
    nonce: String,
) -> Result<(RegistrationSession, String), RegistrationError> {
    let client = build_http_client()?;
    let response = client
        .get(CONFIG_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(network_error)?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response_bytes(response).await?;
    if !status.is_success() {
        return Err(RegistrationError::new(
            "VERIFICATION_UNAVAILABLE",
            "安全验证暂时不可用。",
        ));
    }
    let config: ConfigEnvelope = serde_json::from_slice(&body)
        .map_err(|_| RegistrationError::new("INVALID_CONFIG", "安全验证配置无效。"))?;
    if !config.data.turnstile_required || !valid_turnstile_site_key(&config.data.turnstile_site_key)
    {
        return Err(RegistrationError::new(
            "INVALID_CONFIG",
            "生产安全验证未正确启用。",
        ));
    }
    let csrf_token = extract_csrf_cookie(&headers, &config.data.csrf_token)?;
    let site_key = config.data.turnstile_site_key;

    Ok((
        RegistrationSession {
            nonce,
            client,
            csrf_token,
            turnstile_token: None,
            expires_at: Instant::now() + CHALLENGE_TTL,
        },
        site_key,
    ))
}

enum NavigationAction {
    Allow,
    Complete(String),
    Deny,
}

fn is_https_origin(url: &tauri::Url, expected_host: &str) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str() == Some(expected_host)
        && url.port_or_known_default() == Some(443)
}

fn verification_page_query_matches(
    url: &tauri::Url,
    expected_nonce: &str,
    expected_site_key: &str,
) -> bool {
    let mut nonce = None;
    let mut site_key = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "nonce" if nonce.is_none() => nonce = Some(value.to_string()),
            "site_key" if site_key.is_none() => site_key = Some(value.to_string()),
            _ => return false,
        }
    }
    nonce.as_deref() == Some(expected_nonce) && site_key.as_deref() == Some(expected_site_key)
}

fn navigation_action(
    url: &tauri::Url,
    expected_nonce: &str,
    expected_site_key: &str,
) -> NavigationAction {
    if is_https_origin(url, "niko-ai.cc")
        && url.path() == "/desktop/verify/"
        && url.fragment().is_none()
        && verification_page_query_matches(url, expected_nonce, expected_site_key)
    {
        return NavigationAction::Allow;
    }
    if is_https_origin(url, "challenges.cloudflare.com") {
        return NavigationAction::Allow;
    }
    if url.as_str() == "about:blank" {
        return NavigationAction::Allow;
    }
    if url.scheme() != "niko-register"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str() != Some("verified")
        || url.port().is_some()
        || !matches!(url.path(), "" | "/")
        || url.fragment().is_some()
    {
        return NavigationAction::Deny;
    }

    let mut nonce = None;
    let mut site_key = None;
    let mut token = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "nonce" if nonce.is_none() => nonce = Some(value.to_string()),
            "site_key" if site_key.is_none() => site_key = Some(value.to_string()),
            "token" if token.is_none() => token = Some(value.to_string()),
            _ => return NavigationAction::Deny,
        }
    }
    if nonce.as_deref() != Some(expected_nonce) || site_key.as_deref() != Some(expected_site_key) {
        return NavigationAction::Deny;
    }
    match token {
        Some(token) => NavigationAction::Complete(token),
        None => NavigationAction::Deny,
    }
}

fn close_verification_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(VERIFY_WINDOW_LABEL) {
        let _ = window.close();
    }
}

fn open_verification_window(
    app: &tauri::AppHandle,
    state: RegistrationState,
    nonce: &str,
    site_key: &str,
) -> Result<(), RegistrationError> {
    let mut url: tauri::Url = VERIFY_URL
        .parse()
        .map_err(|_| RegistrationError::new("VERIFICATION_UNAVAILABLE", "安全验证暂时不可用。"))?;
    url.query_pairs_mut()
        .append_pair("nonce", nonce)
        .append_pair("site_key", site_key);
    let navigation_state = state.clone();
    let navigation_nonce = nonce.to_owned();
    let navigation_site_key = site_key.to_owned();
    let navigation_app = app.clone();
    let window =
        tauri::WebviewWindowBuilder::new(app, VERIFY_WINDOW_LABEL, WebviewUrl::External(url))
            .title("Niko 安全验证")
            .inner_size(420.0, 360.0)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .center()
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
            .on_navigation(move |target| {
                match navigation_action(target, &navigation_nonce, &navigation_site_key) {
                    NavigationAction::Allow => true,
                    NavigationAction::Deny => false,
                    NavigationAction::Complete(token) => {
                        if navigation_state.complete(&navigation_nonce, token).is_ok() {
                            let app = navigation_app.clone();
                            tauri::async_runtime::spawn(async move {
                                close_verification_window(&app);
                            });
                        }
                        false
                    }
                }
            })
            .build()
            .map_err(|_| {
                RegistrationError::new("VERIFICATION_UNAVAILABLE", "无法打开应用内安全验证。")
            })?;

    let window_state = state;
    let window_nonce = nonce.to_owned();
    window.on_window_event(move |event| {
        if matches!(
            event,
            WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
        ) {
            window_state.clear_if_pending(&window_nonce);
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn start_registration_challenge(
    app: tauri::AppHandle,
    state: tauri::State<'_, RegistrationState>,
) -> Result<ChallengeStart, RegistrationError> {
    close_verification_window(&app);
    state.clear();

    let nonce = random_nonce()?;
    let (session, site_key) = initialize_session(nonce.clone()).await?;
    state.replace(session)?;
    if let Err(error) = open_verification_window(&app, state.inner().clone(), &nonce, &site_key) {
        state.clear_if_nonce(&nonce);
        return Err(error);
    }

    let expiry_state = state.inner().clone();
    let expiry_nonce = nonce.clone();
    let expiry_app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(CHALLENGE_TTL).await;
        if expiry_state.clear_if_nonce(&expiry_nonce) {
            close_verification_window(&expiry_app);
        }
    });

    Ok(ChallengeStart {
        nonce,
        expires_in_seconds: CHALLENGE_TTL.as_secs(),
    })
}

#[tauri::command]
pub fn registration_challenge_status(
    state: tauri::State<'_, RegistrationState>,
    nonce: String,
) -> Result<ChallengeStatus, RegistrationError> {
    Ok(ChallengeStatus {
        status: state.status(&nonce)?,
    })
}

#[tauri::command]
pub fn cancel_registration_challenge(
    app: tauri::AppHandle,
    state: tauri::State<'_, RegistrationState>,
    nonce: String,
) {
    if state.clear_if_nonce(&nonce) {
        close_verification_window(&app);
    }
}

#[derive(Serialize)]
struct RegistrationPayload<'a> {
    username: &'a str,
    password: &'a str,
    turnstile_token: &'a str,
}

fn build_registration_request(
    client: &Client,
    endpoint: &str,
    csrf_token: &str,
    username: &str,
    password: &str,
    turnstile_token: &str,
) -> Result<reqwest::Request, RegistrationError> {
    let cookie = HeaderValue::from_str(&format!("{CSRF_COOKIE_NAME}={csrf_token}"))
        .map_err(|_| RegistrationError::new("INVALID_CONFIG", "安全验证配置无效。"))?;
    client
        .post(endpoint)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(ORIGIN, NIKO_ORIGIN)
        .header("X-CSRF-Token", csrf_token)
        .header(COOKIE, cookie)
        .json(&RegistrationPayload {
            username,
            password,
            turnstile_token,
        })
        .build()
        .map_err(|_| RegistrationError::new("REGISTRATION_REQUEST_ERROR", "无法创建注册请求。"))
}

fn remote_code(payload: &serde_json::Value) -> &str {
    payload
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| payload.get("code").and_then(serde_json::Value::as_str))
        .unwrap_or("")
}

fn sanitized_remote_error(
    status: StatusCode,
    payload: &[u8],
    retry_after: Option<u64>,
) -> RegistrationError {
    let parsed = serde_json::from_slice::<serde_json::Value>(payload).unwrap_or_default();
    match remote_code(&parsed) {
        "USERNAME_TAKEN" | "ACCOUNT_CONFLICT" => {
            RegistrationError::new("USERNAME_TAKEN", "这个用户名已被使用，请换一个再试。")
        }
        "TURNSTILE_FAILED" | "TURNSTILE_REQUIRED" => {
            RegistrationError::new("TURNSTILE_FAILED", "安全验证未通过，请重新验证。")
        }
        "CSRF_REJECTED" | "ORIGIN_REJECTED" => {
            RegistrationError::new("CSRF_REJECTED", "注册安全会话已过期，请重新验证。")
        }
        "RATE_LIMITED" => RegistrationError::rate_limited(retry_after),
        "INVALID_REQUEST" => {
            RegistrationError::new("INVALID_REQUEST", "注册信息格式无效，请检查后重试。")
        }
        _ if status == StatusCode::TOO_MANY_REQUESTS => {
            RegistrationError::rate_limited(retry_after)
        }
        _ => RegistrationError::new("ACCOUNT_SERVICE_ERROR", "账户服务暂时不可用，请稍后重试。"),
    }
}

fn validate_registration_input(username: &str, password: &str) -> Result<(), RegistrationError> {
    if !(2..=32).contains(&username.chars().count())
        || username.chars().any(|character| character.is_control())
    {
        return Err(RegistrationError::new(
            "INVALID_REQUEST",
            "用户名需为 2-32 个字符。",
        ));
    }
    if !(8..=128).contains(&password.chars().count())
        || password.chars().any(|character| character.is_control())
    {
        return Err(RegistrationError::new(
            "INVALID_REQUEST",
            "密码需为 8-128 个字符。",
        ));
    }
    Ok(())
}

fn invalid_registration_response() -> RegistrationError {
    RegistrationError::new("INVALID_RESPONSE", "账户服务返回了无效结果。")
}

fn validate_registration_success(
    content_type: Option<&str>,
    body: &[u8],
    expected_username: &str,
) -> Result<(), RegistrationError> {
    let valid_content_type = content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("application/json"));
    if !valid_content_type {
        return Err(invalid_registration_response());
    }

    let payload: RegistrationSuccessEnvelope =
        serde_json::from_slice(body).map_err(|_| invalid_registration_response())?;
    if !payload.success
        || !payload.data.login_required
        || payload.data.account.username != expected_username
    {
        return Err(invalid_registration_response());
    }
    Ok(())
}

#[tauri::command]
pub async fn register_niko_account(
    state: tauri::State<'_, RegistrationState>,
    request: RegisterAccountRequest,
) -> Result<RegisterAccountResponse, RegistrationError> {
    let username = request.username.trim().to_owned();
    validate_registration_input(&username, request.password.as_str())?;
    let exchange = state.take_verified(&request.nonce)?;
    let http_request = build_registration_request(
        &exchange.client,
        REGISTER_URL,
        exchange.csrf_token.as_str(),
        &username,
        request.password.as_str(),
        exchange.turnstile_token.as_str(),
    )?;

    let response = exchange
        .client
        .execute(http_request)
        .await
        .map_err(network_error)?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| seconds.min(24 * 60 * 60));
    let body = response_bytes(response).await?;
    if !status.is_success() {
        return Err(sanitized_remote_error(status, &body, retry_after));
    }
    validate_registration_success(content_type.as_deref(), &body, &username)?;

    Ok(RegisterAccountResponse { username })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session(nonce: &str, expires_at: Instant) -> RegistrationSession {
        RegistrationSession {
            nonce: nonce.to_owned(),
            client: build_http_client().unwrap(),
            csrf_token: SensitiveString::new("c".repeat(64)),
            turnstile_token: None,
            expires_at,
        }
    }

    #[test]
    fn csrf_cookie_must_be_a_matching_secure_host_cookie() {
        let token = "c".repeat(64);
        let mut headers = HeaderMap::new();
        headers.append(
            SET_COOKIE,
            HeaderValue::from_str(&format!(
                "{CSRF_COOKIE_NAME}={token}; Path=/; Secure; SameSite=Lax"
            ))
            .unwrap(),
        );
        assert_eq!(
            extract_csrf_cookie(&headers, &token).unwrap().as_str(),
            token
        );

        let mut insecure = HeaderMap::new();
        insecure.append(
            SET_COOKIE,
            HeaderValue::from_str(&format!("{CSRF_COOKIE_NAME}={token}; Path=/; SameSite=Lax"))
                .unwrap(),
        );
        assert_eq!(
            extract_csrf_cookie(&insecure, &token).err().unwrap().code,
            "INVALID_CONFIG"
        );
    }

    #[test]
    fn temporary_state_is_consumed_once_and_expired_state_is_cleared() {
        let state = RegistrationState::default();
        state
            .replace(test_session(
                "nonce",
                Instant::now() + Duration::from_secs(60),
            ))
            .unwrap();
        state.complete("nonce", "t".repeat(40)).unwrap();
        assert_eq!(state.status("nonce").unwrap(), ChallengeState::Verified);
        assert!(!state.clear_if_nonce("stale-nonce"));
        assert_eq!(state.status("nonce").unwrap(), ChallengeState::Verified);
        let exchange = state.take_verified("nonce").unwrap();
        assert_eq!(exchange.turnstile_token.as_str(), "t".repeat(40));
        assert_eq!(state.status("nonce").unwrap(), ChallengeState::Missing);
        assert_eq!(
            state.take_verified("nonce").err().unwrap().code,
            "VERIFICATION_REQUIRED"
        );

        state
            .replace(test_session(
                "expired",
                Instant::now() - Duration::from_secs(1),
            ))
            .unwrap();
        assert_eq!(state.status("expired").unwrap(), ChallengeState::Expired);
        assert_eq!(state.status("expired").unwrap(), ChallengeState::Missing);
    }

    #[test]
    fn registration_request_matches_the_public_bff_contract() {
        let client = build_http_client().unwrap();
        let csrf = "c".repeat(64);
        let request = build_registration_request(
            &client,
            "https://niko-ai.cc/api/niko/v1/auth/register",
            &csrf,
            "alice",
            "Password123",
            &"t".repeat(40),
        )
        .unwrap();

        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(request.url().as_str(), REGISTER_URL);
        assert_eq!(request.headers().get(ORIGIN).unwrap(), NIKO_ORIGIN);
        assert_eq!(
            request
                .headers()
                .get("X-CSRF-Token")
                .unwrap()
                .to_str()
                .unwrap(),
            csrf
        );
        assert_eq!(
            request.headers().get(COOKIE).unwrap().to_str().unwrap(),
            format!("{CSRF_COOKIE_NAME}={csrf}")
        );
        assert_eq!(
            request.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let payload: serde_json::Value =
            serde_json::from_slice(request.body().and_then(reqwest::Body::as_bytes).unwrap())
                .unwrap();
        assert_eq!(
            payload,
            serde_json::json!({
                "username": "alice",
                "password": "Password123",
                "turnstile_token": "t".repeat(40),
            })
        );
        assert!(
            !String::from_utf8_lossy(request.body().unwrap().as_bytes().unwrap())
                .contains("NIKO_BFF_SECRET")
        );
    }

    #[test]
    fn remote_errors_are_mapped_without_echoing_sensitive_details() {
        let body = br#"{"error":{"code":"UNKNOWN","message":"NIKO_BFF_SECRET=server-only"}}"#;
        let error = sanitized_remote_error(StatusCode::BAD_GATEWAY, body, None);
        let serialized = serde_json::to_string(&error).unwrap();
        assert_eq!(error.code, "ACCOUNT_SERVICE_ERROR");
        assert!(!serialized.contains("server-only"));
        assert!(!serialized.contains("NIKO_BFF_SECRET"));

        let conflict = sanitized_remote_error(
            StatusCode::CONFLICT,
            br#"{"error":{"code":"USERNAME_TAKEN","message":"internal id 42"}}"#,
            None,
        );
        assert_eq!(conflict.code, "USERNAME_TAKEN");
        assert!(!serde_json::to_string(&conflict)
            .unwrap()
            .contains("internal id"));
    }

    #[test]
    fn registration_success_response_matches_the_public_contract() {
        let body = br#"{
            "success": true,
            "data": {
                "login_required": true,
                "account": { "username": "alice" }
            }
        }"#;
        validate_registration_success(Some("application/json; charset=utf-8"), body, "alice")
            .unwrap();
    }

    #[test]
    fn registration_success_rejects_html_and_malformed_json() {
        assert_eq!(
            validate_registration_success(None, b"{}", "alice")
                .unwrap_err()
                .code,
            "INVALID_RESPONSE"
        );
        assert_eq!(
            validate_registration_success(Some("text/html"), b"<html></html>", "alice")
                .unwrap_err()
                .code,
            "INVALID_RESPONSE"
        );
        assert_eq!(
            validate_registration_success(
                Some("application/json"),
                br#"{"success": true,"#,
                "alice",
            )
            .unwrap_err()
            .code,
            "INVALID_RESPONSE"
        );

        let login_not_required = br#"{
            "success": true,
            "data": {
                "login_required": false,
                "account": { "username": "alice" }
            }
        }"#;
        assert_eq!(
            validate_registration_success(Some("application/json"), login_not_required, "alice",)
                .unwrap_err()
                .code,
            "INVALID_RESPONSE"
        );

        let missing_account = br#"{
            "success": true,
            "data": { "login_required": true }
        }"#;
        assert_eq!(
            validate_registration_success(Some("application/json"), missing_account, "alice")
                .unwrap_err()
                .code,
            "INVALID_RESPONSE"
        );
    }

    #[test]
    fn registration_success_rejects_false_or_invalid_contract_fields() {
        let success_false = br#"{
            "success": false,
            "data": {
                "login_required": true,
                "account": { "username": "alice" }
            }
        }"#;
        assert_eq!(
            validate_registration_success(Some("application/json"), success_false, "alice")
                .unwrap_err()
                .code,
            "INVALID_RESPONSE"
        );

        let invalid_login_required = br#"{
            "success": true,
            "data": {
                "login_required": "true",
                "account": { "username": "alice" }
            }
        }"#;
        assert_eq!(
            validate_registration_success(
                Some("application/json"),
                invalid_login_required,
                "alice",
            )
            .unwrap_err()
            .code,
            "INVALID_RESPONSE"
        );
    }

    #[test]
    fn registration_success_rejects_a_different_username() {
        let body = br#"{
            "success": true,
            "data": {
                "login_required": true,
                "account": { "username": "bob" }
            }
        }"#;
        assert_eq!(
            validate_registration_success(Some("application/json"), body, "alice")
                .unwrap_err()
                .code,
            "INVALID_RESPONSE"
        );
    }

    #[test]
    fn verification_navigation_is_restricted_and_nonce_bound() {
        let nonce = "a".repeat(32);
        let site_key = "0x4AAAAAAD_7tPGZn65hZ-Ov";
        let page: tauri::Url = format!("{VERIFY_URL}?nonce={nonce}&site_key={site_key}")
            .parse()
            .unwrap();
        assert!(matches!(
            navigation_action(&page, &nonce, site_key),
            NavigationAction::Allow
        ));
        let wrong_page_site_key: tauri::Url = format!(
            "{VERIFY_URL}?nonce={nonce}&site_key={}",
            "1x00000000000000000000AA"
        )
        .parse()
        .unwrap();
        assert!(matches!(
            navigation_action(&wrong_page_site_key, &nonce, site_key),
            NavigationAction::Deny
        ));

        let cloudflare: tauri::Url = "https://challenges.cloudflare.com/turnstile/v0/"
            .parse()
            .unwrap();
        assert!(matches!(
            navigation_action(&cloudflare, &nonce, site_key),
            NavigationAction::Allow
        ));
        assert!(matches!(
            navigation_action(&"about:blank".parse().unwrap(), &nonce, site_key),
            NavigationAction::Allow
        ));

        let callback: tauri::Url = format!(
            "niko-register://verified?nonce={nonce}&site_key={site_key}&token={}",
            "t".repeat(40)
        )
        .parse()
        .unwrap();
        assert!(matches!(
            navigation_action(&callback, &nonce, site_key),
            NavigationAction::Complete(token) if token == "t".repeat(40)
        ));

        let wrong_nonce: tauri::Url = format!(
            "niko-register://verified?nonce={}&site_key={site_key}&token={}",
            "b".repeat(32),
            "t".repeat(40)
        )
        .parse()
        .unwrap();
        assert!(matches!(
            navigation_action(&wrong_nonce, &nonce, site_key),
            NavigationAction::Deny
        ));

        let wrong_site_key: tauri::Url = format!(
            "niko-register://verified?nonce={nonce}&site_key={}&token={}",
            "1x00000000000000000000AA",
            "t".repeat(40)
        )
        .parse()
        .unwrap();
        assert!(matches!(
            navigation_action(&wrong_site_key, &nonce, site_key),
            NavigationAction::Deny
        ));

        let alternate_niko_port: tauri::Url =
            format!("https://niko-ai.cc:444/desktop/verify/?nonce={nonce}&site_key={site_key}")
                .parse()
                .unwrap();
        assert!(matches!(
            navigation_action(&alternate_niko_port, &nonce, site_key),
            NavigationAction::Deny
        ));
        assert!(matches!(
            navigation_action(
                &"https://challenges.cloudflare.com:444/turnstile/v0/"
                    .parse()
                    .unwrap(),
                &nonce,
                site_key,
            ),
            NavigationAction::Deny
        ));
        assert!(matches!(
            navigation_action(&"about:blank?x=1".parse().unwrap(), &nonce, site_key),
            NavigationAction::Deny
        ));
        assert!(matches!(
            navigation_action(&"about:blank#x".parse().unwrap(), &nonce, site_key),
            NavigationAction::Deny
        ));
        assert!(matches!(
            navigation_action(&"https://example.com/".parse().unwrap(), &nonce, site_key,),
            NavigationAction::Deny
        ));
    }
}
