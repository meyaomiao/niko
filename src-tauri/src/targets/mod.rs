//! E5-1/E5-2/E5-3/E6-1/E6-2/E6-3
//! Target trait + TOML/JSON 保守合并 + 三个接入目标实现

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use crate::fsx;
use crate::commands::snapshots::save_backup;

// ─── 公共结构 ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyPlan {
    pub base_url: String,
    pub api_key: String,
    pub model_group: Option<String>,
    /// 用户在登录器里选中的模型，写入目标配置的默认模型字段
    pub model: Option<String>,
    /// Codex 混用模式：保留 auth.json 里的 ChatGPT 登录态，密钥改写进 provider 段。
    /// 仅对 Codex 生效，Claude 桌面端只有 env 一条路，没有这个维度。
    #[serde(default)]
    pub codex_mixed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplySummary {
    pub target_id: String,
    pub changed_keys: Vec<String>,
}

pub trait Target: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn is_installed(&self) -> bool;
    fn apply(&self, plan: &ApplyPlan) -> Result<ApplySummary, String>;

    /// 本机已安装时的真实应用图标（PNG data URI）。未安装或平台不支持时返回 None，
    /// 由前端回落到随应用提供的目标应用图标。
    fn icon_data_uri(&self) -> Option<String> {
        None
    }
}

// ─── E5-2: TOML 保守合并 ────────────────────────────────────────────────────
//
// Codex 只从 [model_providers.<name>] 读取自定义端点，顶层 model_provider 指向它。
// 会话统一后日常只更新 custom 路由，不再因 Provider 切换改写历史。

/// Codex config.toml 里我们独占的 provider 段名
const CODEX_PROVIDER: &str = "custom";
const LEGACY_CODEX_PROVIDER: &str = "momotoken";

/// 纯 API 模式固定使用 Codex 当前支持的最高推理档。
const CODEX_MAX_REASONING_EFFORT: &str = "ultra";

/// 其他 Codex 切换工具写在 config.toml 顶层、会覆盖本次配置效果的键。
/// - `model_context_window` / `model_auto_compact_token_limit`：压低桌面端上下文与自动压缩阈值
/// - `service_tier`：OpenAI 官方专属参数，透传给第三方上游会报错
/// - `model_catalog_json`：指向别家模型元数据表，会改写模型上下文
const CODEX_CONFLICTING_ROOT_KEYS: &[&str] = &[
    "model_context_window",
    "model_auto_compact_token_limit",
    "service_tier",
    "model_catalog_json",
];

fn merge_toml_codex_provider(
    path: &Path,
    base_url: &str,
    model: Option<&str>,
    mixed_api_key: Option<&str>,
) -> Result<Vec<String>, String> {
    let raw = if path.exists() {
        fs::read_to_string(path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };

    // 解析失败时必须报错退出：继续走下去会用空表覆写，等于清空用户整份 Codex 配置。
    let mut doc: toml::Table = if raw.trim().is_empty() {
        toml::Table::new()
    } else {
        raw.parse::<toml::Table>()
            .map_err(|e| format!("~/.codex/config.toml 解析失败，未做任何修改：{e}"))?
    };
    let mut changed = Vec::new();

    let providers = doc
        .entry("model_providers")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("model_providers 字段不是 table")?;

    // wire_api 必须是 responses：新版 Codex 已拒绝 "chat"
    let mut provider = toml::Table::new();
    provider.insert("name".to_owned(), toml::Value::String(CODEX_PROVIDER.to_owned()));
    provider.insert("base_url".to_owned(), toml::Value::String(base_url.to_owned()));
    provider.insert("wire_api".to_owned(), toml::Value::String("responses".to_owned()));
    provider.insert("requires_openai_auth".to_owned(), toml::Value::Boolean(true));
    match mixed_api_key {
        // 混用模式：密钥留在 provider 段，auth.json 保持 ChatGPT 登录态
        Some(key) => {
            provider.insert(
                "experimental_bearer_token".to_owned(),
                toml::Value::String(key.to_owned()),
            );
        }
        // 纯 API 模式：requires_openai_auth 让 Codex 从 auth.json 读取密钥。
        // 不能设置 env_key；它只读取 Codex 进程环境，桌面启动不会从 auth.json 注入环境变量。
        None => {}
    }
    let provider = toml::Value::Table(provider);

    if providers.get(CODEX_PROVIDER) != Some(&provider) {
        providers.insert(CODEX_PROVIDER.to_owned(), provider);
        changed.push(format!("model_providers.{CODEX_PROVIDER}"));
    }

    let selected = toml::Value::String(CODEX_PROVIDER.to_owned());
    if doc.get("model_provider") != Some(&selected) {
        doc.insert("model_provider".to_owned(), selected);
        changed.push("model_provider".to_owned());
    }

    if let Some(model) = model {
        let new_model = toml::Value::String(model.to_owned());
        if doc.get("model") != Some(&new_model) {
            doc.insert("model".to_owned(), new_model);
            changed.push("model".to_owned());
        }
    }

    // 纯 API 模式不依赖 ChatGPT 账号权益，直接为所选 API 模型开启最高推理档。
    // 混用模式不强制改写，保留用户现有偏好。
    if mixed_api_key.is_none() {
        let max_reasoning = toml::Value::String(CODEX_MAX_REASONING_EFFORT.to_owned());
        if doc.get("model_reasoning_effort") != Some(&max_reasoning) {
            doc.insert("model_reasoning_effort".to_owned(), max_reasoning);
            changed.push("model_reasoning_effort".to_owned());
        }
    }

    // 其他切换工具（CC Switch / CodexPlusPlus）留下的顶层键会盖掉本次配置，必须清掉
    for key in CODEX_CONFLICTING_ROOT_KEYS {
        if doc.remove(*key).is_some() {
            changed.push(format!("-{key}"));
        }
    }

    if !changed.is_empty() {
        let content = toml::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let snap = fsx::write_with_snapshot(path, content.as_bytes()).map_err(|e| e.to_string())?;
        snap.commit();
    }

    Ok(changed)
}

// ─── E5-3: JSON 保守合并 ────────────────────────────────────────────────────
//
// 策略：只写/更新指定顶层 key，不碰其他 key。

fn merge_json_keys(path: &Path, updates: &[(&str, Value)]) -> Result<Vec<String>, String> {
    let mut root: Value = if path.exists() {
        let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).unwrap_or(Value::Object(Default::default()))
    } else {
        Value::Object(Default::default())
    };

    let obj = root.as_object_mut().ok_or("JSON root 不是 object")?;
    let mut changed = Vec::new();

    for (k, v) in updates {
        if obj.get(*k) != Some(v) {
            obj.insert(k.to_string(), v.clone());
            changed.push(k.to_string());
        }
    }

    if !changed.is_empty() {
        let content = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let snap = fsx::write_with_snapshot(path, content.as_bytes()).map_err(|e| e.to_string())?;
        snap.commit();
    }

    Ok(changed)
}

/// 删除 JSON 里的指定顶层 key，其余内容原封不动
fn remove_json_keys(path: &Path, keys: &[&str]) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut root: Value = serde_json::from_str(&raw).unwrap_or(Value::Object(Default::default()));
    let obj = root.as_object_mut().ok_or("JSON root 不是 object")?;

    let mut changed = Vec::new();
    for k in keys {
        if obj.remove(*k).is_some() {
            changed.push(format!("-{k}"));
        }
    }

    if !changed.is_empty() {
        let content = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        let snap = fsx::write_with_snapshot(path, content.as_bytes()).map_err(|e| e.to_string())?;
        snap.commit();
    }

    Ok(changed)
}

/// 移除 Codex 的 API Key 登录态，同时保留已有 ChatGPT tokens。
fn remove_codex_api_auth(path: &Path) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut root: Value = serde_json::from_str(&raw).unwrap_or(Value::Object(Default::default()));
    let obj = root.as_object_mut().ok_or("JSON root 不是 object")?;
    let mut changed = Vec::new();

    if obj.remove("OPENAI_API_KEY").is_some() {
        changed.push("-OPENAI_API_KEY".to_owned());
    }
    if obj.get("auth_mode").and_then(Value::as_str) == Some("apikey") {
        obj.remove("auth_mode");
        changed.push("-auth_mode".to_owned());
    }

    if !changed.is_empty() {
        let content = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        let snap = fsx::write_with_snapshot(path, content.as_bytes()).map_err(|e| e.to_string())?;
        snap.commit();
    }

    Ok(changed)
}

/// 删除 JSON 里 env 块的指定环境变量，其余内容原封不动
fn remove_json_env(path: &Path, keys: &[&str]) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut root: Value = serde_json::from_str(&raw).unwrap_or(Value::Object(Default::default()));
    let obj = root.as_object_mut().ok_or("JSON root 不是 object")?;
    let Some(env) = obj.get_mut("env").and_then(Value::as_object_mut) else {
        return Ok(Vec::new());
    };

    let mut changed = Vec::new();
    for k in keys {
        if env.remove(*k).is_some() {
            changed.push(format!("-env.{k}"));
        }
    }

    if !changed.is_empty() {
        let content = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        let snap = fsx::write_with_snapshot(path, content.as_bytes()).map_err(|e| e.to_string())?;
        snap.commit();
    }

    Ok(changed)
}

/// 只更新 JSON 里 env 块的指定环境变量，保留用户已有的其他变量
fn merge_json_env(path: &Path, vars: &[(&str, String)]) -> Result<Vec<String>, String> {
    let mut root: Value = if path.exists() {
        let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).unwrap_or(Value::Object(Default::default()))
    } else {
        Value::Object(Default::default())
    };

    let obj = root.as_object_mut().ok_or("JSON root 不是 object")?;
    let env = obj
        .entry("env")
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .ok_or("env 字段不是 object")?;

    let mut changed = Vec::new();
    for (k, v) in vars {
        let val = Value::String(v.clone());
        if env.get(*k) != Some(&val) {
            env.insert((*k).to_owned(), val);
            changed.push(format!("env.{k}"));
        }
    }

    if !changed.is_empty() {
        let content = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let snap = fsx::write_with_snapshot(path, content.as_bytes()).map_err(|e| e.to_string())?;
        snap.commit();
    }

    Ok(changed)
}

fn home_dir() -> PathBuf {
    user_home_dir()
}

pub(crate) fn user_home_dir() -> PathBuf {
    dirs_home()
}

/// 跨平台 home 目录（不依赖额外 crate，用环境变量）
fn dirs_home() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                let drive = std::env::var("HOMEDRIVE").ok()?;
                let path = std::env::var("HOMEPATH").ok()?;
                (!drive.is_empty() && !path.is_empty()).then(|| format!("{drive}{path}"))
            })
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Users\\default"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
    }
}

// ─── 桌面应用探测 ───────────────────────────────────────────────────────────
//
// 只面向桌面端：用户可能装了 App 但从未跑过 CLI，所以不能只看 ~/.codex 之类的目录。

/// Codex 桌面端已并入 ChatGPT 桌面应用（bundle id 仍是 com.openai.codex），
/// 老版本可能还叫 Codex.app，所以新名优先、旧名兜底。
#[cfg(target_os = "macos")]
const CODEX_APP_NAMES: &[&str] = &["ChatGPT.app", "Codex.app", "OpenAI Codex.app"];

#[cfg(target_os = "macos")]
fn macos_app_exists(names: &[&str]) -> bool {
    let user_apps = home_dir().join("Applications");
    names.iter().any(|name| {
        Path::new("/Applications").join(name).exists() || user_apps.join(name).exists()
    })
}

/// Windows 下按 (LOCALAPPDATA 子目录, 可执行文件名) 逐个探测
#[cfg(target_os = "windows")]
fn windows_app_exists(candidates: &[(&str, &str)]) -> bool {
    let Ok(local) = std::env::var("LOCALAPPDATA") else {
        return false;
    };
    let root = PathBuf::from(local);
    candidates
        .iter()
        .any(|(dir, exe)| root.join(dir).join(exe).exists())
}

/// 已安装 App 的 bundle 路径（用于提取真实应用图标）
#[cfg(target_os = "macos")]
fn macos_app_path(names: &[&str]) -> Option<PathBuf> {
    let user_apps = home_dir().join("Applications");
    names.iter().find_map(|name| {
        let system = Path::new("/Applications").join(name);
        if system.exists() {
            return Some(system);
        }
        let user = user_apps.join(name);
        user.exists().then_some(user)
    })
}

/// Windows 下已安装可执行文件的完整路径
#[cfg(target_os = "windows")]
fn windows_app_path(candidates: &[(&str, &str)]) -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var("LOCALAPPDATA").ok()?);
    candidates.iter().find_map(|(dir, exe)| {
        let p = root.join(dir).join(exe);
        p.exists().then_some(p)
    })
}

/// 某个 target 对应的、本机可直接启动的应用路径。未安装或平台不支持时返回 None。
pub fn app_launch_path(target_id: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        match target_id {
            "codex" => macos_app_path(CODEX_APP_NAMES),
            "claude-desktop" => macos_app_path(&["Claude.app"]),
            _ => None,
        }
    }
    #[cfg(target_os = "windows")]
    {
        match target_id {
            "codex" => windows_app_path(&[
                ("Programs\\ChatGPT", "ChatGPT.exe"),
                ("ChatGPT", "ChatGPT.exe"),
                ("Programs\\Codex", "Codex.exe"),
                ("Codex", "Codex.exe"),
            ]),
            "claude-desktop" => windows_app_path(&[("AnthropicClaude", "claude.exe")]),
            _ => None,
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = target_id;
        None
    }
}

/// 从已安装的 App 里取真实图标，转成 PNG 后以 data URI 返回。
/// 已安装时优先使用本机应用包里的图标，确保与用户实际安装的版本一致。
#[cfg(target_os = "macos")]
fn macos_app_icon_data_uri(names: &[&str]) -> Option<String> {
    static ICON_TEMP_SEQUENCE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);

    let app = macos_app_path(names)?;
    let resources = app.join("Contents").join("Resources");

    // Info.plist 里的 CFBundleIconFile 可能不带 .icns 后缀
    let declared = fs::read_to_string(app.join("Contents").join("Info.plist"))
        .ok()
        .and_then(|raw| {
            let key = raw.find("<key>CFBundleIconFile</key>")?;
            let rest = &raw[key..];
            let start = rest.find("<string>")? + "<string>".len();
            let end = rest[start..].find("</string>")?;
            Some(rest[start..start + end].trim().to_owned())
        });

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(name) = declared {
        candidates.push(resources.join(&name));
        if !name.ends_with(".icns") {
            candidates.push(resources.join(format!("{name}.icns")));
        }
    }
    // 兜底：AppIcon.icns 是常见默认名，Electron 应用多为 electron.icns
    candidates.push(resources.join("AppIcon.icns"));
    candidates.push(resources.join("electron.icns"));

    let icns = candidates.into_iter().find(|p| p.exists())?;

    // sips 是 macOS 自带工具，无需额外依赖
    let sequence = ICON_TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let out = std::env::temp_dir().join(format!(
        "niko-icon-{}-{sequence}.png",
        std::process::id()
    ));
    let ok = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "png", "-Z", "128"])
        .arg(&icns)
        .arg("--out")
        .arg(&out)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        return None;
    }

    let png = fs::read(&out).ok()?;
    let _ = fs::remove_file(&out);
    Some(format!("data:image/png;base64,{}", base64_encode(&png)))
}

/// 极简 base64，避免为一张图标引入新依赖
#[cfg(target_os = "macos")]
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

// ─── E6-1: Codex ────────────────────────────────────────────────────────────

pub struct CodexTarget;

impl Target for CodexTarget {
    fn id(&self) -> &'static str { "codex" }
    fn display_name(&self) -> &'static str { "ChatGPT 桌面端" }

    fn is_installed(&self) -> bool {
        // 桌面端与 CLI 共用 ~/.codex；但只装了桌面端、没跑过 CLI 时该目录可能还不存在，
        // 所以先按已安装的 App 判断。
        #[cfg(target_os = "macos")]
        {
            if macos_app_exists(CODEX_APP_NAMES) {
                return true;
            }
        }
        #[cfg(target_os = "windows")]
        {
            if windows_app_exists(&[
                ("Programs\\ChatGPT", "ChatGPT.exe"),
                ("ChatGPT", "ChatGPT.exe"),
                ("Programs\\Codex", "Codex.exe"),
                ("Codex", "Codex.exe"),
            ]) {
                return true;
            }
        }
        home_dir().join(".codex").exists()
    }

    fn icon_data_uri(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            return macos_app_icon_data_uri(CODEX_APP_NAMES);
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplySummary, String> {
        let codex_dir = home_dir().join(".codex");
        let mut changed = Vec::new();

        let auth_path = codex_dir.join("auth.json");
        // E5-5: 写入前留存备份，供设置页恢复
        let _ = save_backup(self.id(), &auth_path);
        if plan.codex_mixed {
            // 混用模式：auth.json 里同时存在 OPENAI_API_KEY 和 ChatGPT tokens 时，Codex 优先吃
            // api key，官方登录态就等于失效。必须把这个键移除，密钥改由 provider 段承载。
            changed.append(&mut remove_codex_api_auth(&auth_path)?);
        } else {
            // 纯 API 模式必须同时选中 apikey 鉴权；否则已有 auth_mode=chatgpt 时会忽略新 Key。
            // 只合并这两个键，桌面端已有的 ChatGPT tokens 继续保留，切回混用模式时还能恢复。
            changed.append(&mut merge_json_keys(
                &auth_path,
                &[
                    ("OPENAI_API_KEY", Value::String(plan.api_key.clone())),
                    ("auth_mode", Value::String("apikey".to_owned())),
                ],
            )?);
        }

        // 保守合并 config.toml
        let config_path = codex_dir.join("config.toml");
        let _ = save_backup(self.id(), &config_path);
        let mut toml_changed = merge_toml_codex_provider(
            &config_path,
            &plan.base_url,
            plan.model.as_deref(),
            plan.codex_mixed.then_some(plan.api_key.as_str()),
        )?;
        changed.append(&mut toml_changed);

        Ok(ApplySummary { target_id: self.id().to_owned(), changed_keys: changed })
    }
}

// ─── E6-2: Claude Desktop ───────────────────────────────────────────────────

pub struct ClaudeDesktopTarget;

/// Claude Code 会自己在 `ANTHROPIC_BASE_URL` 后拼 `/v1/messages`，所以这里必须去掉
/// 末尾的 `/v1`，否则请求路径变成 `/v1/v1/messages`，上游直接 404。
fn claude_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_owned()
}

// Claude 桌面端在 3p（第三方网关）部署模式下，Claude Code 面板不读 ~/.claude/settings.json 的
// env，而是读自己的托管配置 `Claude-3p/configLibrary/`：启动子进程时按托管配置生成临时凭证并
// 注入 ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN 环境变量，优先级高于 settings.json。
// 因此只写 settings.json 会被托管配置完全压制，必须同时写这里。

/// Niko 在 configLibrary 里独占的条目 id（固定值，避免每次启用都新建一条）
const CLAUDE_3P_ENTRY_ID: &str = "6e696b6f-0000-4000-8000-000000000001";
/// 条目在桌面端配置列表里显示的名字
const CLAUDE_3P_ENTRY_NAME: &str = "Niko";

/// Claude 桌面端 3p 托管配置目录。未安装 / 不支持的平台返回 None。
fn claude_3p_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(
            home_dir()
                .join("Library")
                .join("Application Support")
                .join("Claude-3p")
                .join("configLibrary"),
        )
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .ok()
            .map(|appdata| PathBuf::from(appdata).join("Claude-3p").join("configLibrary"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// 写入 Niko 的托管网关条目并设为生效项。其他工具（CC Switch 等）的条目保留不动。
fn claude_managed_apply(dir: &Path, base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let entry_path = dir.join(format!("{CLAUDE_3P_ENTRY_ID}.json"));
    let _ = save_backup("claude-desktop", &entry_path);
    let mut changed: Vec<String> = merge_json_keys(
        &entry_path,
        &[
            ("inferenceProvider", Value::String("gateway".to_owned())),
            ("inferenceGatewayBaseUrl", Value::String(base_url.to_owned())),
            ("inferenceGatewayApiKey", Value::String(api_key.to_owned())),
            ("inferenceGatewayAuthScheme", Value::String("bearer".to_owned())),
        ],
    )?
    .into_iter()
    .map(|k| format!("configLibrary/{CLAUDE_3P_ENTRY_ID}.json:{k}"))
    .collect();

    let meta_path = dir.join("_meta.json");
    let _ = save_backup("claude-desktop", &meta_path);
    let mut meta: Value = if meta_path.exists() {
        let raw = fs::read_to_string(&meta_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).unwrap_or_else(|_| Value::Object(Default::default()))
    } else {
        Value::Object(Default::default())
    };
    let obj = meta.as_object_mut().ok_or("_meta.json root 不是 object")?;

    let mut meta_changed = false;
    let entries = obj
        .entry("entries")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("_meta.json entries 不是数组")?;
    if !entries
        .iter()
        .any(|e| e.get("id").and_then(Value::as_str) == Some(CLAUDE_3P_ENTRY_ID))
    {
        entries.push(serde_json::json!({
            "id": CLAUDE_3P_ENTRY_ID,
            "name": CLAUDE_3P_ENTRY_NAME,
        }));
        meta_changed = true;
        changed.push("configLibrary/_meta.json:entries".to_owned());
    }
    if obj.get("appliedId").and_then(Value::as_str) != Some(CLAUDE_3P_ENTRY_ID) {
        obj.insert("appliedId".to_owned(), Value::String(CLAUDE_3P_ENTRY_ID.to_owned()));
        meta_changed = true;
        changed.push("configLibrary/_meta.json:appliedId".to_owned());
    }
    if meta_changed {
        let content = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let snap =
            fsx::write_with_snapshot(&meta_path, content.as_bytes()).map_err(|e| e.to_string())?;
        snap.commit();
    }

    Ok(changed)
}

/// 读回托管配置里当前真正生效的网关地址与密钥
fn claude_managed_effective(dir: &Path) -> Option<(String, String)> {
    let meta: Value = serde_json::from_str(&fs::read_to_string(dir.join("_meta.json")).ok()?).ok()?;
    let applied = meta.get("appliedId")?.as_str()?;
    let entry: Value =
        serde_json::from_str(&fs::read_to_string(dir.join(format!("{applied}.json"))).ok()?).ok()?;
    if entry.get("inferenceProvider").and_then(Value::as_str) != Some("gateway") {
        return None;
    }
    let base_url = entry.get("inferenceGatewayBaseUrl")?.as_str()?.to_owned();
    let api_key = entry.get("inferenceGatewayApiKey")?.as_str()?.to_owned();
    Some((base_url, api_key))
}

/// 摘掉 Niko 的托管条目，让桌面端回到原来的选择（其他条目与配置保留）
fn claude_managed_restore(dir: &Path) -> Result<Vec<String>, String> {
    let mut changed = Vec::new();
    let meta_path = dir.join("_meta.json");
    if meta_path.exists() {
        let _ = save_backup("claude-desktop", &meta_path);
        let raw = fs::read_to_string(&meta_path).map_err(|e| e.to_string())?;
        let mut meta: Value =
            serde_json::from_str(&raw).unwrap_or_else(|_| Value::Object(Default::default()));
        if let Some(obj) = meta.as_object_mut() {
            let mut meta_changed = false;
            if obj.get("appliedId").and_then(Value::as_str) == Some(CLAUDE_3P_ENTRY_ID) {
                obj.remove("appliedId");
                meta_changed = true;
                changed.push("-configLibrary/_meta.json:appliedId".to_owned());
            }
            if let Some(entries) = obj.get_mut("entries").and_then(Value::as_array_mut) {
                let before = entries.len();
                entries
                    .retain(|e| e.get("id").and_then(Value::as_str) != Some(CLAUDE_3P_ENTRY_ID));
                if entries.len() != before {
                    meta_changed = true;
                    changed.push("-configLibrary/_meta.json:entries".to_owned());
                }
            }
            if meta_changed {
                let content = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
                let snap = fsx::write_with_snapshot(&meta_path, content.as_bytes())
                    .map_err(|e| e.to_string())?;
                snap.commit();
            }
        }
    }

    let entry_path = dir.join(format!("{CLAUDE_3P_ENTRY_ID}.json"));
    if entry_path.exists() {
        let _ = save_backup("claude-desktop", &entry_path);
        fs::remove_file(&entry_path).map_err(|e| e.to_string())?;
        changed.push(format!("-configLibrary/{CLAUDE_3P_ENTRY_ID}.json"));
    }

    Ok(changed)
}

/// 会覆盖 settings.json 里 `model` 字段的模型相关环境变量。写入模型时一并清除。
const CLAUDE_MODEL_ENV_CONFLICTS: &[&str] = &[
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
    "ANTHROPIC_SMALL_FAST_MODEL",
];

impl Target for ClaudeDesktopTarget {
    fn id(&self) -> &'static str { "claude-desktop" }
    fn display_name(&self) -> &'static str { "Claude 桌面端" }

    fn is_installed(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            macos_app_exists(&["Claude.app"])
        }
        #[cfg(target_os = "windows")]
        {
            windows_app_exists(&[("AnthropicClaude", "claude.exe")])
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            false
        }
    }

    fn icon_data_uri(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            return macos_app_icon_data_uri(&["Claude.app"]);
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplySummary, String> {
        // Claude Desktop 把 ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN 列为「由 Claude Desktop
        // 托管、不可覆盖」，桌面端聊天本身没有自定义 API 端点入口。能接入的是它内置的
        // Claude Code 面板，读的是 ~/.claude/settings.json，和 CLI 共用同一份配置。
        let settings_path = home_dir().join(".claude").join("settings.json");
        let _ = save_backup(self.id(), &settings_path);

        let mut changed = merge_json_env(
            &settings_path,
            &[
                ("ANTHROPIC_AUTH_TOKEN", plan.api_key.clone()),
                ("ANTHROPIC_BASE_URL", claude_base_url(&plan.base_url)),
            ],
        )?;
        if let Some(model) = &plan.model {
            changed.append(&mut merge_json_keys(
                &settings_path,
                &[("model", Value::String(model.clone()))],
            )?);
            // 这些环境变量优先级高于 settings 的 model 字段（ANTHROPIC_MODEL），或会把
            // opus/sonnet/haiku/fable 别名重定向到别的模型。其他切换工具留下的残留会
            // 直接盖掉我们刚写入的模型，必须清掉，否则用户看到的仍是旧模型。
            changed.append(&mut remove_json_env(&settings_path, CLAUDE_MODEL_ENV_CONFLICTS)?);
        }

        // 3p 模式下托管配置注入的环境变量优先级高于 settings.json，必须一并写入才会真正生效
        if let Some(dir) = claude_3p_config_dir() {
            changed.append(&mut claude_managed_apply(
                &dir,
                &claude_base_url(&plan.base_url),
                &plan.api_key,
            )?);
        }

        Ok(ApplySummary { target_id: self.id().to_owned(), changed_keys: changed })
    }
}

// ─── 目标注册表 ─────────────────────────────────────────────────────────────

pub fn all_targets() -> Vec<Box<dyn Target>> {
    vec![Box::new(CodexTarget), Box::new(ClaudeDesktopTarget)]
}

pub fn transaction_paths(target_id: &str) -> Result<Vec<PathBuf>, String> {
    match target_id {
        "claude-desktop" => {
            let mut paths = vec![home_dir().join(".claude").join("settings.json")];
            if let Some(dir) = claude_3p_config_dir() {
                paths.push(dir.join(format!("{CLAUDE_3P_ENTRY_ID}.json")));
                paths.push(dir.join("_meta.json"));
            }
            Ok(paths)
        }
        other => Err(format!("unknown transaction target: {other}")),
    }
}

pub fn preflight_target_apply(target_id: &str) -> Result<(), String> {
    for path in transaction_paths(target_id)? {
        if !path.exists() {
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|_| "target settings unreadable".to_owned())?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|_| "target settings malformed".to_owned())?;
        let object = value
            .as_object()
            .ok_or_else(|| "target settings have an unsupported shape".to_owned())?;
        if path.file_name().and_then(|name| name.to_str()) == Some("settings.json")
            && object.get("env").is_some_and(|env| !env.is_object())
        {
            return Err("target settings have an unsupported shape".to_owned());
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("_meta.json") {
            if object.get("entries").is_some_and(|entries| !entries.is_array())
                || object.get("appliedId").is_some_and(|id| !id.is_string())
            {
                return Err("target settings have an unsupported shape".to_owned());
            }
        }
    }
    Ok(())
}

// ─── 连通性测试：回读磁盘上真实生效的配置 ────────────────────────────────────

/// 从目标应用配置文件里读回真正生效的接入参数。
/// 测试必须用磁盘上的值，而不是前端内存里的值，否则测的是「本该写成什么」而非「实际写成了什么」。
#[derive(Debug, Serialize)]
pub struct EffectiveConfig {
    pub target_id: String,
    /// 该应用实际会请求的完整地址
    pub endpoint: String,
    pub api_key: String,
    pub model: Option<String>,
    /// anthropic 走 x-api-key + anthropic-version，openai 走 Bearer
    pub auth_style: String,
}

pub fn effective_config(target_id: &str) -> Result<EffectiveConfig, String> {
    let h = home_dir();
    match target_id {
        "codex" => {
            let config_path = h.join(".codex").join("config.toml");
            let raw = fs::read_to_string(&config_path)
                .map_err(|_| "未找到 ChatGPT 本地配置，请先点击启用".to_owned())?;
            let doc: toml::Table = raw
                .parse()
                .map_err(|e| format!("~/.codex/config.toml 解析失败：{e}"))?;
            if doc.get("model_provider").and_then(|v| v.as_str()) != Some(CODEX_PROVIDER) {
                return Err("当前 ChatGPT 配置尚未接入 Niko，请先点击启用".to_owned());
            }
            let provider = doc
                .get("model_providers")
                .and_then(|v| v.as_table())
                .and_then(|t| t.get(CODEX_PROVIDER))
                .and_then(|v| v.as_table())
                .ok_or("ChatGPT 配置里缺少 Niko 接入信息，请先点击启用")?;
            let base_url = provider
                .get("base_url")
                .and_then(|v| v.as_str())
                .ok_or("provider 缺少 base_url")?;
            // 混用模式密钥在 provider 段，纯 API 模式在 auth.json
            let api_key = match provider.get("experimental_bearer_token").and_then(|v| v.as_str()) {
                Some(key) => key.to_owned(),
                None => {
                    let auth_raw = fs::read_to_string(h.join(".codex").join("auth.json"))
                        .map_err(|_| "未找到 ~/.codex/auth.json，请先点击启用".to_owned())?;
                    let auth: Value = serde_json::from_str(&auth_raw)
                        .map_err(|e| format!("auth.json 解析失败：{e}"))?;
                    auth.get("OPENAI_API_KEY")
                        .and_then(Value::as_str)
                        .ok_or("auth.json 里没有 OPENAI_API_KEY，请先点击启用")?
                        .to_owned()
                }
            };
            Ok(EffectiveConfig {
                target_id: target_id.to_owned(),
                endpoint: format!("{}/responses", base_url.trim_end_matches('/')),
                api_key,
                model: doc.get("model").and_then(|v| v.as_str()).map(str::to_owned),
                auth_style: "openai".to_owned(),
            })
        }
        "claude-desktop" => {
            // 托管配置存在时它才是真正生效的来源，settings.json 只是 CLI 的兜底
            if let Some((base_url, api_key)) =
                claude_3p_config_dir().as_deref().and_then(claude_managed_effective)
            {
                let model = fs::read_to_string(h.join(".claude").join("settings.json"))
                    .ok()
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                    .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_owned));
                return Ok(EffectiveConfig {
                    target_id: target_id.to_owned(),
                    endpoint: format!("{}/v1/messages", base_url.trim_end_matches('/')),
                    api_key,
                    model,
                    auth_style: "anthropic".to_owned(),
                });
            }
            let settings_path = h.join(".claude").join("settings.json");
            let raw = fs::read_to_string(&settings_path)
                .map_err(|_| "未找到 ~/.claude/settings.json，请先点击启用".to_owned())?;
            let v: Value = serde_json::from_str(&raw)
                .map_err(|e| format!("settings.json 解析失败：{e}"))?;
            let env = v.get("env").ok_or("settings.json 里没有 env 配置，请先点击启用")?;
            let base_url = env
                .get("ANTHROPIC_BASE_URL")
                .and_then(Value::as_str)
                .ok_or("settings.json 里没有 ANTHROPIC_BASE_URL，请先点击启用")?;
            let api_key = env
                .get("ANTHROPIC_AUTH_TOKEN")
                .and_then(Value::as_str)
                .ok_or("settings.json 里没有 ANTHROPIC_AUTH_TOKEN，请先点击启用")?;
            Ok(EffectiveConfig {
                target_id: target_id.to_owned(),
                endpoint: format!("{}/v1/messages", base_url.trim_end_matches('/')),
                api_key: api_key.to_owned(),
                model: v.get("model").and_then(Value::as_str).map(str::to_owned),
                auth_style: "anthropic".to_owned(),
            })
        }
        other => Err(format!("未知目标: {other}")),
    }
}

// ─── 恢复官方默认配置 ───────────────────────────────────────────────────────
//
// 只移除 Niko 自己写入的键，让应用回到「未接第三方中转」的状态。
// 用户其他配置（projects / mcp_servers / sandbox / ChatGPT 登录态）一律保留。

/// 移除 TOML 里 Niko 写入的 provider 段与顶层键
fn remove_toml_codex_provider(path: &Path) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut doc: toml::Table = raw
        .parse::<toml::Table>()
        .map_err(|e| format!("~/.codex/config.toml 解析失败，未做任何修改：{e}"))?;

    let mut changed = Vec::new();
    let active_provider = doc
        .get("model_provider")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    let owns_config = matches!(
        active_provider.as_deref(),
        Some(CODEX_PROVIDER) | Some(LEGACY_CODEX_PROVIDER)
    ) || doc
            .get("model_providers")
            .and_then(|v| v.as_table())
            .is_some_and(|providers| {
                providers.contains_key(CODEX_PROVIDER)
                    || providers.contains_key(LEGACY_CODEX_PROVIDER)
            });
    if let Some(providers) = doc.get_mut("model_providers").and_then(|v| v.as_table_mut()) {
        if providers.remove(CODEX_PROVIDER).is_some() {
            changed.push(format!("-model_providers.{CODEX_PROVIDER}"));
        }
        if providers.remove(LEGACY_CODEX_PROVIDER).is_some() {
            changed.push(format!("-model_providers.{LEGACY_CODEX_PROVIDER}"));
        }
    }
    // 只有仍指向 Niko 当前或旧版入口时才移除，避免抹掉用户手动选择的其他来源。
    if matches!(
        active_provider.as_deref(),
        Some(CODEX_PROVIDER) | Some(LEGACY_CODEX_PROVIDER)
    ) {
        doc.remove("model_provider");
        changed.push("-model_provider".to_owned());
    }
    if owns_config {
        if doc.remove("model").is_some() {
            changed.push("-model".to_owned());
        }
        if doc.get("model_reasoning_effort").and_then(|v| v.as_str())
            == Some(CODEX_MAX_REASONING_EFFORT)
        {
            doc.remove("model_reasoning_effort");
            changed.push("-model_reasoning_effort".to_owned());
        }
    }

    if !changed.is_empty() {
        let content = toml::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        let snap = fsx::write_with_snapshot(path, content.as_bytes()).map_err(|e| e.to_string())?;
        snap.commit();
    }
    Ok(changed)
}

/// 把目标应用恢复为官方默认（走官方账号登录）的配置状态
pub fn restore_defaults(target_id: &str) -> Result<ApplySummary, String> {
    let h = home_dir();
    let mut changed = Vec::new();

    match target_id {
        "codex" => {
            let config_path = h.join(".codex").join("config.toml");
            let auth_path = h.join(".codex").join("auth.json");
            let _ = save_backup(target_id, &config_path);
            let _ = save_backup(target_id, &auth_path);
            changed.append(&mut remove_toml_codex_provider(&config_path)?);
            // 移除中转密钥与对应登录模式，ChatGPT tokens 保持不动
            changed.append(&mut remove_codex_api_auth(&auth_path)?);
        }
        "claude-desktop" => {
            let settings_path = h.join(".claude").join("settings.json");
            let _ = save_backup(target_id, &settings_path);
            changed.append(&mut remove_json_env(
                &settings_path,
                &["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL"],
            )?);
            changed.append(&mut remove_json_keys(&settings_path, &["model"])?);
            if let Some(dir) = claude_3p_config_dir() {
                if dir.exists() {
                    changed.append(&mut claude_managed_restore(&dir)?);
                }
            }
        }
        other => return Err(format!("未知目标: {other}")),
    }

    Ok(ApplySummary { target_id: target_id.to_owned(), changed_keys: changed })
}

// ─── E5-4: 配置漂移检测 ──────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DriftReport {
    pub target_id: String,
    pub drifted: bool,
    pub mismatched_keys: Vec<String>,
}

/// 检测某个 target 当前配置是否与期望 plan 一致
pub fn check_drift(target_id: &str, plan: &ApplyPlan) -> Result<DriftReport, String> {
    let h = home_dir();
    let mut mismatched = Vec::new();

    match target_id {
        "codex" => {
            let auth_path = h.join(".codex").join("auth.json");
            let auth: Value = if auth_path.exists() {
                let raw = fs::read_to_string(&auth_path).unwrap_or_default();
                serde_json::from_str(&raw).unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            let auth_key = auth.get("OPENAI_API_KEY").and_then(Value::as_str);
            if plan.codex_mixed {
                // 混用模式下 auth.json 不该残留 api key，否则会盖掉 ChatGPT 登录态
                if auth_key.is_some() {
                    mismatched.push("auth.json:OPENAI_API_KEY".to_owned());
                }
            } else if !auth_path.exists() {
                mismatched.push("auth.json:missing".to_owned());
            } else if auth_key != Some(&plan.api_key) {
                mismatched.push("auth.json:OPENAI_API_KEY".to_owned());
            }
            let auth_mode = auth.get("auth_mode").and_then(Value::as_str);
            if plan.codex_mixed {
                if auth_mode == Some("apikey") {
                    mismatched.push("auth.json:auth_mode".to_owned());
                }
            } else if auth_mode != Some("apikey") {
                mismatched.push("auth.json:auth_mode".to_owned());
            }
            let config_path = h.join(".codex").join("config.toml");
            if config_path.exists() {
                let raw = fs::read_to_string(&config_path).unwrap_or_default();
                let Ok(doc) = raw.parse::<toml::Table>() else {
                    return Ok(DriftReport {
                        target_id: target_id.to_owned(),
                        drifted: true,
                        mismatched_keys: vec!["config.toml:parse_error".to_owned()],
                    });
                };
                let provider = doc
                    .get("model_providers")
                    .and_then(|v| v.as_table())
                    .and_then(|t| t.get(CODEX_PROVIDER))
                    .and_then(|v| v.as_table());
                if provider.and_then(|t| t.get("base_url")).and_then(|v| v.as_str())
                    != Some(&plan.base_url)
                {
                    mismatched.push(format!("config.toml:model_providers.{CODEX_PROVIDER}.base_url"));
                }
                if doc.get("model_provider").and_then(|v| v.as_str()) != Some(CODEX_PROVIDER) {
                    mismatched.push("config.toml:model_provider".to_owned());
                }
                let bearer = provider
                    .and_then(|t| t.get("experimental_bearer_token"))
                    .and_then(|v| v.as_str());
                let expected_bearer = plan.codex_mixed.then_some(plan.api_key.as_str());
                if bearer != expected_bearer {
                    mismatched
                        .push(format!("config.toml:model_providers.{CODEX_PROVIDER}.experimental_bearer_token"));
                }
                // auth.json 鉴权与 env_key 是两条独立路径；这里不应依赖进程环境。
                let env_key = provider.and_then(|t| t.get("env_key")).and_then(|v| v.as_str());
                if env_key.is_some() {
                    mismatched
                        .push(format!("config.toml:model_providers.{CODEX_PROVIDER}.env_key"));
                }
                if let Some(model) = &plan.model {
                    if doc.get("model").and_then(|v| v.as_str()) != Some(model.as_str()) {
                        mismatched.push("config.toml:model".to_owned());
                    }
                }
                if !plan.codex_mixed
                    && doc.get("model_reasoning_effort").and_then(|v| v.as_str())
                        != Some(CODEX_MAX_REASONING_EFFORT)
                {
                    mismatched.push("config.toml:model_reasoning_effort".to_owned());
                }
                // 别家工具留下的顶层键仍在，说明配置被它们的设置压制
                for key in CODEX_CONFLICTING_ROOT_KEYS {
                    if doc.get(*key).is_some() {
                        mismatched.push(format!("config.toml:{key}"));
                    }
                }
            } else {
                mismatched.push("config.toml:missing".to_owned());
            }
        }
        // Claude Desktop 内置 Claude Code 面板读 ~/.claude/settings.json
        "claude-desktop" => {
            let settings = h.join(".claude").join("settings.json");
            if settings.exists() {
                let raw = fs::read_to_string(&settings).unwrap_or_default();
                let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                let env = v.get("env");
                if env.and_then(|e| e.get("ANTHROPIC_AUTH_TOKEN")).and_then(Value::as_str)
                    != Some(&plan.api_key)
                {
                    mismatched.push("settings.json:env.ANTHROPIC_AUTH_TOKEN".to_owned());
                }
                if env.and_then(|e| e.get("ANTHROPIC_BASE_URL")).and_then(Value::as_str)
                    != Some(claude_base_url(&plan.base_url).as_str())
                {
                    mismatched.push("settings.json:env.ANTHROPIC_BASE_URL".to_owned());
                }
                if let Some(model) = &plan.model {
                    if v.get("model").and_then(Value::as_str) != Some(model.as_str()) {
                        mismatched.push("settings.json:model".to_owned());
                    }
                    // 残留的模型环境变量会覆盖 model 字段，等同于配置未生效
                    for k in CLAUDE_MODEL_ENV_CONFLICTS {
                        if env.and_then(|e| e.get(*k)).is_some() {
                            mismatched.push(format!("settings.json:env.{k}"));
                        }
                    }
                }
            } else {
                mismatched.push("settings.json:missing".to_owned());
            }
            // 托管配置若仍指向别家网关，桌面端注入的环境变量会盖掉 settings.json
            if let Some(dir) = claude_3p_config_dir() {
                if dir.join("_meta.json").exists() {
                    match claude_managed_effective(&dir) {
                        Some((base_url, api_key)) => {
                            if base_url != claude_base_url(&plan.base_url) {
                                mismatched
                                    .push("configLibrary:inferenceGatewayBaseUrl".to_owned());
                            }
                            if api_key != plan.api_key {
                                mismatched.push("configLibrary:inferenceGatewayApiKey".to_owned());
                            }
                        }
                        None => mismatched.push("configLibrary:missing".to_owned()),
                    }
                }
            }
        }
        other => return Err(format!("未知目标: {other}")),
    }

    Ok(DriftReport {
        target_id: target_id.to_owned(),
        drifted: !mismatched.is_empty(),
        mismatched_keys: mismatched,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        // 并行测试下时间戳精度不足会撞到同一目录，用递增序号保证互不干扰
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "targets_test_{}_{}_{}",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    /// Codex 只读 [model_providers.*]，且必须保留用户已有的其他 provider 与顶层配置
    #[test]
    fn codex_toml_writes_provider_and_keeps_user_keys() {
        let p = tmp_path("config.toml");
        fs::write(
            &p,
            "approval_policy = \"on-request\"\n\n[model_providers.custom]\nname = \"custom\"\nbase_url = \"https://example.com/v1\"\n",
        )
        .unwrap();

        let changed =
            merge_toml_codex_provider(&p, "https://momotoken.win/v1", Some("claude-sonnet-4-6"), None)
                .unwrap();
        assert!(changed.contains(&"model_providers.custom".to_owned()));
        assert!(changed.contains(&"model_provider".to_owned()));
        assert!(changed.contains(&"model".to_owned()));

        let doc: toml::Table = fs::read_to_string(&p).unwrap().parse().unwrap();
        assert_eq!(
            doc.get("approval_policy").and_then(|v| v.as_str()),
            Some("on-request")
        );
        assert_eq!(doc.get("model_provider").and_then(|v| v.as_str()), Some("custom"));
        assert_eq!(doc.get("model").and_then(|v| v.as_str()), Some("claude-sonnet-4-6"));
        assert_eq!(
            doc.get("model_reasoning_effort").and_then(|v| v.as_str()),
            Some(CODEX_MAX_REASONING_EFFORT)
        );

        let providers = doc.get("model_providers").unwrap().as_table().unwrap();
        assert!(providers.contains_key("custom"), "用户已有 provider 必须保留");
        let ours = providers.get("custom").unwrap().as_table().unwrap();
        assert_eq!(
            ours.get("base_url").and_then(|v| v.as_str()),
            Some("https://momotoken.win/v1")
        );
        // 新版 Codex 已拒绝 wire_api = "chat"
        assert_eq!(ours.get("wire_api").and_then(|v| v.as_str()), Some("responses"));
        assert!(ours.get("env_key").is_none());
        assert_eq!(ours.get("requires_openai_auth").and_then(|v| v.as_bool()), Some(true));

        // 幂等：同样的 plan 不应再产生改动
        assert!(
            merge_toml_codex_provider(&p, "https://momotoken.win/v1", Some("claude-sonnet-4-6"), None)
                .unwrap()
                .is_empty()
        );
    }

    /// 纯 API 模式清掉其他切换器的冲突键，并把旧推理档纠正为最高档
    #[test]
    fn codex_pure_api_clears_conflicts_and_uses_max_reasoning() {
        let p = tmp_path("config.toml");
        fs::write(
            &p,
            "model_provider = \"custom\"\nmodel_context_window = 128000\nmodel_auto_compact_token_limit = 120000\nservice_tier = \"priority\"\nmodel_catalog_json = \"model-catalogs/x.json\"\nmodel_reasoning_effort = \"high\"\n",
        )
        .unwrap();

        merge_toml_codex_provider(&p, "https://momotoken.win/v1", Some("gpt-5.6"), None).unwrap();

        let doc: toml::Table = fs::read_to_string(&p).unwrap().parse().unwrap();
        for key in CODEX_CONFLICTING_ROOT_KEYS {
            assert!(doc.get(*key).is_none(), "{key} 必须被清除");
        }
        assert_eq!(
            doc.get("model_reasoning_effort").and_then(|v| v.as_str()),
            Some(CODEX_MAX_REASONING_EFFORT)
        );
    }

    /// config.toml 语法错误时必须报错，绝不能用空表覆写用户整份配置
    #[test]
    fn codex_toml_refuses_to_overwrite_unparsable_config() {
        let p = tmp_path("config.toml");
        let broken = "model_provider = \"custom\nthis is not toml";
        fs::write(&p, broken).unwrap();

        let err = merge_toml_codex_provider(&p, "https://momotoken.win/v1", Some("gpt-5.6"), None)
            .unwrap_err();
        assert!(err.contains("解析失败"), "错误信息应说明解析失败: {err}");
        assert_eq!(fs::read_to_string(&p).unwrap(), broken, "原文件必须保持不变");
    }

    /// 纯 API 模式选中 apikey 鉴权，同时保留已有 ChatGPT tokens 供切回混用模式
    #[test]
    fn codex_auth_json_selects_api_key_and_keeps_chatgpt_tokens() {
        let p = tmp_path("auth.json");
        fs::write(
            &p,
            r#"{"auth_mode":"chatgpt","tokens":{"id_token":"keep"}}"#,
        )
        .unwrap();

        let changed = merge_json_keys(
            &p,
            &[
                ("OPENAI_API_KEY", Value::String("sk-abc".to_owned())),
                ("auth_mode", Value::String("apikey".to_owned())),
            ],
        )
        .unwrap();
        assert_eq!(changed, vec!["OPENAI_API_KEY".to_owned(), "auth_mode".to_owned()]);

        let v: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v.get("OPENAI_API_KEY").and_then(Value::as_str), Some("sk-abc"));
        assert_eq!(v.get("auth_mode").and_then(Value::as_str), Some("apikey"));
        assert_eq!(
            v.pointer("/tokens/id_token").and_then(Value::as_str),
            Some("keep")
        );
    }

    /// 混用模式：密钥走 provider 段的 experimental_bearer_token，auth.json 不留 api key，
    /// ChatGPT 登录态必须完整保留，否则 Codex 会优先吃 api key 让官方登录失效。
    #[test]
    fn codex_mixed_mode_keeps_chatgpt_auth_and_moves_key_to_provider() {
        let auth = tmp_path("auth.json");
        fs::write(
            &auth,
            r#"{"OPENAI_API_KEY":"sk-old","auth_mode":"apikey","tokens":{"id_token":"keep"}}"#,
        )
        .unwrap();
        let removed = remove_codex_api_auth(&auth).unwrap();
        assert_eq!(removed, vec!["-OPENAI_API_KEY".to_owned(), "-auth_mode".to_owned()]);

        let v: Value = serde_json::from_str(&fs::read_to_string(&auth).unwrap()).unwrap();
        assert!(v.get("OPENAI_API_KEY").is_none());
        assert!(v.get("auth_mode").is_none());
        assert_eq!(v.pointer("/tokens/id_token").and_then(Value::as_str), Some("keep"));

        fs::write(&auth, r#"{"auth_mode":"chatgpt","tokens":{"id_token":"keep"}}"#).unwrap();
        assert!(remove_codex_api_auth(&auth).unwrap().is_empty());
        let v: Value = serde_json::from_str(&fs::read_to_string(&auth).unwrap()).unwrap();
        assert_eq!(v.get("auth_mode").and_then(Value::as_str), Some("chatgpt"));

        let cfg = tmp_path("config.toml");
        fs::write(&cfg, "model_reasoning_effort = \"high\"\n").unwrap();
        merge_toml_codex_provider(&cfg, "https://momotoken.win/v1", None, Some("sk-new")).unwrap();
        let doc: toml::Table = fs::read_to_string(&cfg).unwrap().parse().unwrap();
        let ours = doc
            .get("model_providers")
            .unwrap()
            .as_table()
            .unwrap()
            .get("custom")
            .unwrap()
            .as_table()
            .unwrap();
        assert_eq!(
            ours.get("experimental_bearer_token").and_then(|v| v.as_str()),
            Some("sk-new")
        );
        assert!(ours.get("env_key").is_none(), "混用模式不该依赖进程环境");
        assert_eq!(
            doc.get("model_reasoning_effort").and_then(|v| v.as_str()),
            Some("high"),
            "混用模式不应强制改写用户的推理偏好"
        );
    }

    /// 从混用或旧版 env_key 配置切回纯 API 时，只保留 auth.json 鉴权路径
    #[test]
    fn codex_pure_api_mode_uses_auth_json_without_process_env() {
        let cfg = tmp_path("config.toml");
        merge_toml_codex_provider(&cfg, "https://momotoken.win/v1", None, Some("sk-mixed")).unwrap();
        let mut doc: toml::Table = fs::read_to_string(&cfg).unwrap().parse().unwrap();
        doc.get_mut("model_providers")
            .and_then(|v| v.as_table_mut())
            .and_then(|t| t.get_mut("custom"))
            .and_then(|v| v.as_table_mut())
            .unwrap()
            .insert("env_key".to_owned(), toml::Value::String("OPENAI_API_KEY".to_owned()));
        doc.insert("model_reasoning_effort".to_owned(), toml::Value::String("high".to_owned()));
        fs::write(&cfg, toml::to_string_pretty(&doc).unwrap()).unwrap();

        merge_toml_codex_provider(&cfg, "https://momotoken.win/v1", None, None).unwrap();

        let doc: toml::Table = fs::read_to_string(&cfg).unwrap().parse().unwrap();
        let ours = doc
            .get("model_providers")
            .unwrap()
            .as_table()
            .unwrap()
            .get("custom")
            .unwrap()
            .as_table()
            .unwrap();
        assert!(ours.get("experimental_bearer_token").is_none());
        assert!(ours.get("env_key").is_none());
        assert_eq!(ours.get("requires_openai_auth").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            doc.get("model_reasoning_effort").and_then(|v| v.as_str()),
            Some(CODEX_MAX_REASONING_EFFORT)
        );
    }

    /// 恢复官方配置时清掉 Niko 在纯 API 模式写入的最高推理档。
    #[test]
    fn codex_restore_removes_niko_max_reasoning() {
        let cfg = tmp_path("config.toml");
        fs::write(&cfg, "approval_policy = \"on-request\"\n").unwrap();
        merge_toml_codex_provider(&cfg, "https://momotoken.win/v1", Some("gpt-5.6-sol"), None)
            .unwrap();

        let changed = remove_toml_codex_provider(&cfg).unwrap();
        assert!(changed.contains(&"-model_reasoning_effort".to_owned()));

        let doc: toml::Table = fs::read_to_string(&cfg).unwrap().parse().unwrap();
        assert_eq!(
            doc.get("approval_policy").and_then(|v| v.as_str()),
            Some("on-request")
        );
        assert!(doc.get("model_reasoning_effort").is_none());
        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model").is_none());
    }

    /// Claude Desktop 只读 settings.json 的 env 块，顶层 apiKey 不被识别
    #[test]
    fn claude_settings_writes_env_block_and_keeps_user_vars() {
        let p = tmp_path("settings.json");
        fs::write(
            &p,
            r#"{"includeCoAuthoredBy":false,"env":{"MY_VAR":"keep"}}"#,
        )
        .unwrap();

        let changed = merge_json_env(
            &p,
            &[
                ("ANTHROPIC_AUTH_TOKEN", "sk-abc".to_owned()),
                ("ANTHROPIC_BASE_URL", "https://momotoken.win/v1".to_owned()),
            ],
        )
        .unwrap();
        assert_eq!(changed.len(), 2);

        let v: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v.get("includeCoAuthoredBy").and_then(Value::as_bool), Some(false));
        let env = v.get("env").unwrap();
        assert_eq!(env.get("MY_VAR").and_then(Value::as_str), Some("keep"));
        assert_eq!(env.get("ANTHROPIC_AUTH_TOKEN").and_then(Value::as_str), Some("sk-abc"));
        assert_eq!(
            env.get("ANTHROPIC_BASE_URL").and_then(Value::as_str),
            Some("https://momotoken.win/v1")
        );

        assert!(merge_json_env(
            &p,
            &[
                ("ANTHROPIC_AUTH_TOKEN", "sk-abc".to_owned()),
                ("ANTHROPIC_BASE_URL", "https://momotoken.win/v1".to_owned()),
            ]
        )
        .unwrap()
        .is_empty());
    }

    /// 其他切换工具写在 env 里的模型别名变量优先级高于 settings 的 model 字段，
    /// 不清掉就会让登录器选的模型不生效；同时不能碰用户自己的其他变量。
    #[test]
    fn claude_settings_clears_conflicting_model_env_vars() {
        let p = tmp_path("settings.json");
        fs::write(
            &p,
            r#"{"model":"old-model","env":{"MY_VAR":"keep","ANTHROPIC_MODEL":"old-model","ANTHROPIC_DEFAULT_OPUS_MODEL":"claude-opus-4-8","ANTHROPIC_DEFAULT_SONNET_MODEL_NAME":"claude-opus-4-8","ANTHROPIC_AUTH_TOKEN":"sk-abc"}}"#,
        )
        .unwrap();

        let changed = remove_json_env(&p, CLAUDE_MODEL_ENV_CONFLICTS).unwrap();
        assert_eq!(changed.len(), 3);

        let v: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        let env = v.get("env").unwrap();
        assert_eq!(env.get("MY_VAR").and_then(Value::as_str), Some("keep"));
        assert_eq!(env.get("ANTHROPIC_AUTH_TOKEN").and_then(Value::as_str), Some("sk-abc"));
        assert!(env.get("ANTHROPIC_MODEL").is_none());
        assert!(env.get("ANTHROPIC_DEFAULT_OPUS_MODEL").is_none());
        assert!(env.get("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME").is_none());

        assert!(remove_json_env(&p, CLAUDE_MODEL_ENV_CONFLICTS).unwrap().is_empty());
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let p = tmp_path(name);
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// 3p 模式下桌面端按托管配置注入环境变量，优先级高于 settings.json，
    /// 所以启用必须改写 appliedId，且不能删掉别家工具留下的条目。
    #[test]
    fn claude_managed_apply_takes_over_applied_entry_and_keeps_others() {
        let dir = tmp_dir("claude_3p_apply");
        fs::write(
            dir.join("_meta.json"),
            r#"{"appliedId":"cc-switch-id","entries":[{"id":"cc-switch-id","name":"CC Switch"}]}"#,
        )
        .unwrap();
        fs::write(
            dir.join("cc-switch-id.json"),
            r#"{"inferenceProvider":"gateway","inferenceGatewayBaseUrl":"https://other.example","inferenceGatewayApiKey":"sk-other"}"#,
        )
        .unwrap();

        let changed =
            claude_managed_apply(&dir, "https://momotoken.win", "sk-niko").unwrap();
        assert!(!changed.is_empty());

        let meta: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("_meta.json")).unwrap()).unwrap();
        assert_eq!(meta.get("appliedId").and_then(Value::as_str), Some(CLAUDE_3P_ENTRY_ID));
        let entries = meta.get("entries").unwrap().as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .any(|e| e.get("id").and_then(Value::as_str) == Some("cc-switch-id")));

        let effective = claude_managed_effective(&dir).unwrap();
        assert_eq!(effective, ("https://momotoken.win".to_owned(), "sk-niko".to_owned()));

        // 幂等：再次启用不产生变更
        assert!(claude_managed_apply(&dir, "https://momotoken.win", "sk-niko")
            .unwrap()
            .is_empty());
    }

    /// 恢复官方默认要摘掉我们的条目，别家条目与其配置文件必须留着
    #[test]
    fn claude_managed_restore_removes_only_our_entry() {
        let dir = tmp_dir("claude_3p_restore");
        fs::write(
            dir.join("_meta.json"),
            r#"{"appliedId":"cc-switch-id","entries":[{"id":"cc-switch-id","name":"CC Switch"}]}"#,
        )
        .unwrap();
        fs::write(dir.join("cc-switch-id.json"), r#"{"inferenceProvider":"gateway"}"#).unwrap();
        claude_managed_apply(&dir, "https://momotoken.win", "sk-niko").unwrap();

        let changed = claude_managed_restore(&dir).unwrap();
        assert!(!changed.is_empty());

        let meta: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("_meta.json")).unwrap()).unwrap();
        assert!(meta.get("appliedId").is_none());
        let entries = meta.get("entries").unwrap().as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].get("id").and_then(Value::as_str), Some("cc-switch-id"));
        assert!(!dir.join(format!("{CLAUDE_3P_ENTRY_ID}.json")).exists());
        assert!(dir.join("cc-switch-id.json").exists());
        assert!(claude_managed_restore(&dir).unwrap().is_empty());
    }

    /// 生效值只认 appliedId 指向的条目：别家工具切回去后必须能被检测出来
    #[test]
    fn claude_managed_effective_follows_applied_id() {
        let dir = tmp_dir("claude_3p_effective");
        claude_managed_apply(&dir, "https://momotoken.win", "sk-niko").unwrap();
        fs::write(
            dir.join("cc-switch-id.json"),
            r#"{"inferenceProvider":"gateway","inferenceGatewayBaseUrl":"https://deepkey.top","inferenceGatewayApiKey":"sk-other"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("_meta.json"),
            r#"{"appliedId":"cc-switch-id","entries":[{"id":"cc-switch-id","name":"CC Switch"}]}"#,
        )
        .unwrap();

        let effective = claude_managed_effective(&dir).unwrap();
        assert_eq!(effective, ("https://deepkey.top".to_owned(), "sk-other".to_owned()));
    }
}
