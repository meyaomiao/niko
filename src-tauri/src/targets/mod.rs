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
// 策略：只改 [openai] 下的 base_url / api_key，其余 key 原封不动。

fn merge_toml_openai(
    path: &Path,
    base_url: &str,
    api_key: &str,
    model: Option<&str>,
) -> Result<Vec<String>, String> {
    let raw = if path.exists() {
        fs::read_to_string(path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };

    let mut doc: toml::Table = raw.parse::<toml::Table>().unwrap_or_default();

    let openai = doc
        .entry("openai")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));

    let tbl = openai
        .as_table_mut()
        .ok_or("openai 字段不是 table")?;

    let mut changed = Vec::new();

    let new_url = toml::Value::String(base_url.to_owned());
    if tbl.get("base_url") != Some(&new_url) {
        tbl.insert("base_url".to_owned(), new_url);
        changed.push("openai.base_url".to_owned());
    }

    let new_key = toml::Value::String(api_key.to_owned());
    if tbl.get("api_key") != Some(&new_key) {
        tbl.insert("api_key".to_owned(), new_key);
        changed.push("openai.api_key".to_owned());
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

/// 写整个 JSON 文件（用于 auth.json 这种完全由我们控制的文件）
fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let content = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let snap = fsx::write_with_snapshot(path, content.as_bytes()).map_err(|e| e.to_string())?;
    snap.commit();
    Ok(())
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

// ─── E6-1: Codex ────────────────────────────────────────────────────────────

pub struct CodexTarget;

impl Target for CodexTarget {
    fn id(&self) -> &'static str { "codex" }
    fn display_name(&self) -> &'static str { "Codex (OpenAI CLI)" }

    fn is_installed(&self) -> bool {
        home_dir().join(".codex").exists()
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplySummary, String> {
        let codex_dir = home_dir().join(".codex");
        let mut changed = Vec::new();

        // 写 auth.json
        let auth_path = codex_dir.join("auth.json");
        // E5-5: 写入前留存备份，供设置页恢复
        let _ = save_backup(self.id(), &auth_path);
        let auth_val = serde_json::json!({
            "token": plan.api_key,
            "baseUrl": plan.base_url
        });
        write_json(&auth_path, &auth_val)?;
        changed.push("auth.json".to_owned());

        // 保守合并 config.toml
        let config_path = codex_dir.join("config.toml");
        let _ = save_backup(self.id(), &config_path);
        let mut toml_changed = merge_toml_openai(&config_path, &plan.base_url, &plan.api_key, plan.model.as_deref())?;
        changed.append(&mut toml_changed);

        Ok(ApplySummary { target_id: self.id().to_owned(), changed_keys: changed })
    }
}

// ─── E6-2: Claude Desktop ───────────────────────────────────────────────────

pub struct ClaudeDesktopTarget;

impl ClaudeDesktopTarget {
    fn config_path() -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            home_dir()
                .join("Library")
                .join("Application Support")
                .join("Claude")
                .join("claude_desktop_config.json")
        }
        #[cfg(target_os = "windows")]
        {
            std::env::var("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|_| home_dir().join("AppData").join("Roaming"))
                .join("Claude")
                .join("claude_desktop_config.json")
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            home_dir().join(".config").join("Claude").join("claude_desktop_config.json")
        }
    }
}

impl Target for ClaudeDesktopTarget {
    fn id(&self) -> &'static str { "claude-desktop" }
    fn display_name(&self) -> &'static str { "Claude Desktop" }

    fn is_installed(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            Path::new("/Applications/Claude.app").exists()
                || home_dir().join("Applications").join("Claude.app").exists()
        }
        #[cfg(target_os = "windows")]
        {
            std::env::var("LOCALAPPDATA")
                .map(|p| PathBuf::from(p).join("AnthropicClaude").join("claude.exe").exists())
                .unwrap_or(false)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            false
        }
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplySummary, String> {
        let path = Self::config_path();
        // E5-5: 写入前留存备份
        let _ = save_backup(self.id(), &path);

        // 读取现有文件，只更新 mcpServers.momotoken，不动其他字段
        let mut root: Value = if path.exists() {
            let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            serde_json::from_str(&raw).unwrap_or(Value::Object(Default::default()))
        } else {
            Value::Object(Default::default())
        };

        let obj = root.as_object_mut().ok_or("config 根节点不是 object")?;

        let mcp_servers = obj
            .entry("mcpServers")
            .or_insert_with(|| Value::Object(Default::default()));

        let servers = mcp_servers.as_object_mut().ok_or("mcpServers 不是 object")?;

        servers.insert(
            "momotoken".to_owned(),
            serde_json::json!({
                "type": "openai-compatible",
                "baseUrl": plan.base_url,
                "apiKey": plan.api_key,
                "model": plan.model
            }),
        );

        let content = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let snap = fsx::write_with_snapshot(&path, content.as_bytes()).map_err(|e| e.to_string())?;
        snap.commit();

        Ok(ApplySummary {
            target_id: self.id().to_owned(),
            changed_keys: vec!["mcpServers.momotoken".to_owned()],
        })
    }
}

// ─── E6-3: Claude Code CLI ──────────────────────────────────────────────────

pub struct ClaudeCodeTarget;

impl Target for ClaudeCodeTarget {
    fn id(&self) -> &'static str { "claude-code" }
    fn display_name(&self) -> &'static str { "Claude Code (CLI)" }

    fn is_installed(&self) -> bool {
        // claude code 安装后会有 ~/.claude 目录
        home_dir().join(".claude").exists()
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplySummary, String> {
        let settings_path = home_dir().join(".claude").join("settings.json");
        // E5-5: 写入前留存备份
        let _ = save_backup(self.id(), &settings_path);

        let mut updates = vec![
            ("apiKey", Value::String(plan.api_key.clone())),
            ("baseUrl", Value::String(plan.base_url.clone())),
        ];
        if let Some(model) = &plan.model {
            updates.push(("model", Value::String(model.clone())));
        }
        let changed = merge_json_keys(&settings_path, &updates)?;

        Ok(ApplySummary { target_id: self.id().to_owned(), changed_keys: changed })
    }
}

// ─── 目标注册表 ─────────────────────────────────────────────────────────────

pub fn all_targets() -> Vec<Box<dyn Target>> {
    vec![
        Box::new(CodexTarget),
        Box::new(ClaudeDesktopTarget),
        Box::new(ClaudeCodeTarget),
    ]
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
                if v.get("token").and_then(Value::as_str) != Some(&plan.api_key) {
                    mismatched.push("auth.json:token".to_owned());
                }
                if v.get("baseUrl").and_then(Value::as_str) != Some(&plan.base_url) {
                    mismatched.push("auth.json:baseUrl".to_owned());
                }
            } else {
                mismatched.push("auth.json:missing".to_owned());
            }
            let config_path = h.join(".codex").join("config.toml");
            if config_path.exists() {
                let raw = fs::read_to_string(&config_path).unwrap_or_default();
                let doc: toml::Table = raw.parse().unwrap_or_default();
                let openai = doc.get("openai").and_then(|v| v.as_table());
                if openai.and_then(|t| t.get("api_key")).and_then(|v| v.as_str())
                    != Some(&plan.api_key)
                {
                    mismatched.push("config.toml:[openai].api_key".to_owned());
                }
                if openai.and_then(|t| t.get("base_url")).and_then(|v| v.as_str())
                    != Some(&plan.base_url)
                {
                    mismatched.push("config.toml:[openai].base_url".to_owned());
                }
            }
        }
        "claude-desktop" => {
            let path = ClaudeDesktopTarget::config_path();
            if path.exists() {
                let raw = fs::read_to_string(&path).unwrap_or_default();
                let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                let entry = v
                    .get("mcpServers")
                    .and_then(|s| s.get("momotoken"));
                if entry.and_then(|e| e.get("apiKey")).and_then(Value::as_str)
                    != Some(&plan.api_key)
                {
                    mismatched.push("mcpServers.momotoken.apiKey".to_owned());
                }
                if entry.and_then(|e| e.get("baseUrl")).and_then(Value::as_str)
                    != Some(&plan.base_url)
                {
                    mismatched.push("mcpServers.momotoken.baseUrl".to_owned());
                }
            } else {
                mismatched.push("claude_desktop_config.json:missing".to_owned());
            }
        }
        "claude-code" => {
            let settings = h.join(".claude").join("settings.json");
            if settings.exists() {
                let raw = fs::read_to_string(&settings).unwrap_or_default();
                let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                if v.get("apiKey").and_then(Value::as_str) != Some(&plan.api_key) {
                    mismatched.push("settings.json:apiKey".to_owned());
                }
                if v.get("baseUrl").and_then(Value::as_str) != Some(&plan.base_url) {
                    mismatched.push("settings.json:baseUrl".to_owned());
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

// ─── E6-4: 角色映射与兜底 ───────────────────────────────────────────────────

/// 将用户选择的 "角色" 映射为实际模型名（momotoken 分组可用时优先）
pub fn resolve_model(role: &str, group: Option<&str>) -> String {
    let group = group.unwrap_or("default");
    // 标准角色 → 推荐模型（按优先级排列，第一个是兜底）
    let candidates: &[&str] = match role {
        "fast" | "haiku" => &["claude-haiku-3-5", "claude-haiku-3", "gpt-4o-mini"],
        "balanced" | "sonnet" => &["claude-sonnet-4-5", "claude-sonnet-4", "claude-sonnet-3-5", "gpt-4o"],
        "best" | "opus" => &["claude-opus-4-5", "claude-opus-4", "claude-opus-3", "gpt-4o"],
        "code" => &["claude-sonnet-4-5", "gpt-4o", "claude-sonnet-3-5"],
        other => return other.to_owned(),
    };

    // 分组名透传作为 model 前缀提示（简单策略：直接返回第一个候选）
    // 后续 E7-1 连通性检测后可优化为选第一个可用的
    let _ = group;
    candidates.first().copied().unwrap_or(role).to_owned()
}
