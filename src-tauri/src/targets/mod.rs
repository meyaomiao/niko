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
    // env_key + requires_openai_auth 让 Codex 从 auth.json 的 OPENAI_API_KEY 取密钥
    let mut provider = toml::Table::new();
    provider.insert("name".to_owned(), toml::Value::String(CODEX_PROVIDER.to_owned()));
    provider.insert("base_url".to_owned(), toml::Value::String(base_url.to_owned()));
    provider.insert("wire_api".to_owned(), toml::Value::String("responses".to_owned()));
    provider.insert("env_key".to_owned(), toml::Value::String("OPENAI_API_KEY".to_owned()));
    provider.insert("requires_openai_auth".to_owned(), toml::Value::Boolean(true));
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

/// Codex 桌面端在不同渠道下的 bundle 名
#[cfg(target_os = "macos")]
const CODEX_APP_NAMES: &[&str] = &["Codex.app", "OpenAI Codex.app"];

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

// ─── E6-1: Codex ────────────────────────────────────────────────────────────

pub struct CodexTarget;

impl Target for CodexTarget {
    fn id(&self) -> &'static str { "codex" }
    fn display_name(&self) -> &'static str { "Codex 桌面端" }

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
            if windows_app_exists(&[("Programs\\Codex", "Codex.exe"), ("Codex", "Codex.exe")]) {
                return true;
            }
        }
        home_dir().join(".codex").exists()
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplySummary, String> {
        let codex_dir = home_dir().join(".codex");
        let mut changed = Vec::new();

        // 合并 auth.json
        let auth_path = codex_dir.join("auth.json");
        // E5-5: 写入前留存备份，供设置页恢复
        let _ = save_backup(self.id(), &auth_path);
        // Codex 只认 OPENAI_API_KEY 这个键；整体覆写会清掉桌面端已有的 ChatGPT 登录态，
        // 所以只合并这一个键。
        changed.append(&mut merge_json_keys(
            &auth_path,
            &[("OPENAI_API_KEY", Value::String(plan.api_key.clone()))],
        )?);

        // 保守合并 config.toml
        let config_path = codex_dir.join("config.toml");
        let _ = save_backup(self.id(), &config_path);
        let mut toml_changed =
            merge_toml_codex_provider(&config_path, &plan.base_url, plan.model.as_deref())?;
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
            if auth_path.exists() {
                let raw = fs::read_to_string(&auth_path).unwrap_or_default();
                let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                if v.get("OPENAI_API_KEY").and_then(Value::as_str) != Some(&plan.api_key) {
                    mismatched.push("auth.json:OPENAI_API_KEY".to_owned());
                }
            } else {
                mismatched.push("auth.json:missing".to_owned());
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
            merge_toml_codex_provider(&p, "https://momotoken.win/v1", Some("claude-sonnet-4-6"))
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
            merge_toml_codex_provider(&p, "https://momotoken.win/v1", Some("claude-sonnet-4-6"))
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
