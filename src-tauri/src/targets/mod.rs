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
    /// 由前端回落到内置占位图形。
    fn icon_data_uri(&self) -> Option<String> {
        None
    }
}

// ─── E5-2: TOML 保守合并 ────────────────────────────────────────────────────
//
// Codex 只从 [model_providers.<name>] 读取自定义端点，顶层 model_provider 指向它。
// 策略：只改 momotoken 这一个 provider 段和顶层 model / model_provider，其余 key 原封不动。

/// Codex config.toml 里我们独占的 provider 段名
const CODEX_PROVIDER: &str = "momotoken";

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

    let mut doc: toml::Table = raw.parse::<toml::Table>().unwrap_or_default();
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
        // 纯 API 模式：从 auth.json 的 OPENAI_API_KEY 取密钥
        None => {
            provider.insert("env_key".to_owned(), toml::Value::String("OPENAI_API_KEY".to_owned()));
        }
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
    dirs_home()
}

/// 跨平台 home 目录（不依赖额外 crate，用环境变量）
fn dirs_home() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .or_else(|_| {
                let drive = std::env::var("HOMEDRIVE").unwrap_or_default();
                let path = std::env::var("HOMEPATH").unwrap_or_default();
                Ok::<String, std::env::VarError>(format!("{drive}{path}"))
            })
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("C:\\Users\\default"))
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

/// 从已安装的 App 里取真实图标，转成 PNG 后以 data URI 返回。
/// 图标属于各自厂商的商标资源，不入库、不随包分发，只在运行时按需读取本机已安装的副本。
#[cfg(target_os = "macos")]
fn macos_app_icon_data_uri(names: &[&str]) -> Option<String> {
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
    let out = std::env::temp_dir().join(format!("piko-icon-{}.png", std::process::id()));
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
            changed.append(&mut remove_json_keys(&auth_path, &["OPENAI_API_KEY"])?);
        } else {
            // 纯 API 模式：Codex 只认 OPENAI_API_KEY 这个键；整体覆写会清掉桌面端已有的
            // ChatGPT 登录态，所以只合并这一个键。
            changed.append(&mut merge_json_keys(
                &auth_path,
                &[("OPENAI_API_KEY", Value::String(plan.api_key.clone()))],
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
                ("ANTHROPIC_BASE_URL", plan.base_url.clone()),
            ],
        )?;
        if let Some(model) = &plan.model {
            changed.append(&mut merge_json_keys(
                &settings_path,
                &[("model", Value::String(model.clone()))],
            )?);
        }

        Ok(ApplySummary { target_id: self.id().to_owned(), changed_keys: changed })
    }
}

// ─── 目标注册表 ─────────────────────────────────────────────────────────────

pub fn all_targets() -> Vec<Box<dyn Target>> {
    vec![Box::new(CodexTarget), Box::new(ClaudeDesktopTarget)]
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
            let config_path = h.join(".codex").join("config.toml");
            if config_path.exists() {
                let raw = fs::read_to_string(&config_path).unwrap_or_default();
                let doc: toml::Table = raw.parse().unwrap_or_default();
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
                    != Some(&plan.base_url)
                {
                    mismatched.push("settings.json:env.ANTHROPIC_BASE_URL".to_owned());
                }
            } else {
                mismatched.push("settings.json:missing".to_owned());
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
        let dir = std::env::temp_dir().join(format!(
            "targets_test_{}_{}",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
        assert!(changed.contains(&"model_providers.momotoken".to_owned()));
        assert!(changed.contains(&"model_provider".to_owned()));
        assert!(changed.contains(&"model".to_owned()));

        let doc: toml::Table = fs::read_to_string(&p).unwrap().parse().unwrap();
        assert_eq!(
            doc.get("approval_policy").and_then(|v| v.as_str()),
            Some("on-request")
        );
        assert_eq!(doc.get("model_provider").and_then(|v| v.as_str()), Some("momotoken"));
        assert_eq!(doc.get("model").and_then(|v| v.as_str()), Some("claude-sonnet-4-6"));

        let providers = doc.get("model_providers").unwrap().as_table().unwrap();
        assert!(providers.contains_key("custom"), "用户已有 provider 必须保留");
        let ours = providers.get("momotoken").unwrap().as_table().unwrap();
        assert_eq!(
            ours.get("base_url").and_then(|v| v.as_str()),
            Some("https://momotoken.win/v1")
        );
        // 新版 Codex 已拒绝 wire_api = "chat"
        assert_eq!(ours.get("wire_api").and_then(|v| v.as_str()), Some("responses"));
        assert_eq!(ours.get("env_key").and_then(|v| v.as_str()), Some("OPENAI_API_KEY"));

        // 幂等：同样的 plan 不应再产生改动
        assert!(
            merge_toml_codex_provider(&p, "https://momotoken.win/v1", Some("claude-sonnet-4-6"), None)
                .unwrap()
                .is_empty()
        );
    }

    /// Codex 桌面端可能已有 ChatGPT 登录态，写 API Key 不能清掉它
    #[test]
    fn codex_auth_json_merges_api_key_and_keeps_chatgpt_login() {
        let p = tmp_path("auth.json");
        fs::write(
            &p,
            r#"{"preferred_auth_method":"chatgpt","tokens":{"id_token":"keep"}}"#,
        )
        .unwrap();

        let changed =
            merge_json_keys(&p, &[("OPENAI_API_KEY", Value::String("sk-abc".to_owned()))]).unwrap();
        assert_eq!(changed, vec!["OPENAI_API_KEY".to_owned()]);

        let v: Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v.get("OPENAI_API_KEY").and_then(Value::as_str), Some("sk-abc"));
        assert_eq!(
            v.get("preferred_auth_method").and_then(Value::as_str),
            Some("chatgpt")
        );
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
            r#"{"OPENAI_API_KEY":"sk-old","preferred_auth_method":"chatgpt","tokens":{"id_token":"keep"}}"#,
        )
        .unwrap();
        let removed = remove_json_keys(&auth, &["OPENAI_API_KEY"]).unwrap();
        assert_eq!(removed, vec!["-OPENAI_API_KEY".to_owned()]);

        let v: Value = serde_json::from_str(&fs::read_to_string(&auth).unwrap()).unwrap();
        assert!(v.get("OPENAI_API_KEY").is_none());
        assert_eq!(
            v.get("preferred_auth_method").and_then(Value::as_str),
            Some("chatgpt")
        );
        assert_eq!(v.pointer("/tokens/id_token").and_then(Value::as_str), Some("keep"));

        let cfg = tmp_path("config.toml");
        merge_toml_codex_provider(&cfg, "https://momotoken.win/v1", None, Some("sk-new")).unwrap();
        let doc: toml::Table = fs::read_to_string(&cfg).unwrap().parse().unwrap();
        let ours = doc
            .get("model_providers")
            .unwrap()
            .as_table()
            .unwrap()
            .get("momotoken")
            .unwrap()
            .as_table()
            .unwrap();
        assert_eq!(
            ours.get("experimental_bearer_token").and_then(|v| v.as_str()),
            Some("sk-new")
        );
        assert!(ours.get("env_key").is_none(), "混用模式不该再从 auth.json 取密钥");
    }

    /// 从混用切回纯 API 时，provider 段里的 bearer token 必须被清掉，否则残留旧密钥
    #[test]
    fn codex_pure_api_mode_clears_mixed_bearer_token() {
        let cfg = tmp_path("config.toml");
        merge_toml_codex_provider(&cfg, "https://momotoken.win/v1", None, Some("sk-mixed")).unwrap();
        merge_toml_codex_provider(&cfg, "https://momotoken.win/v1", None, None).unwrap();

        let doc: toml::Table = fs::read_to_string(&cfg).unwrap().parse().unwrap();
        let ours = doc
            .get("model_providers")
            .unwrap()
            .as_table()
            .unwrap()
            .get("momotoken")
            .unwrap()
            .as_table()
            .unwrap();
        assert!(ours.get("experimental_bearer_token").is_none());
        assert_eq!(ours.get("env_key").and_then(|v| v.as_str()), Some("OPENAI_API_KEY"));
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
}
