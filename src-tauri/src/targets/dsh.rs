//! DeepSeek Harness（`dsh web`）接入：独占 `llm-pi-ai.providers.momotoken`。

use super::yamlx::{
    mapping_get, mapping_str, quote_yaml_scalar, remove_child_mapping, remove_child_scalar,
    upsert_child_mapping, upsert_child_scalar, upsert_top_mapping, yaml_mapping_or_err,
};
use super::{
    find_cli_executable, home_dir, ApplyPlan, ApplySummary, EffectiveConfig, Target,
    TargetConfigObservation,
};
use crate::commands::snapshots::save_backup;
use crate::fsx;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const DSH_TARGET_ID: &str = "dsh";
pub const DSH_PROVIDER: &str = "momotoken";
pub const DSH_API_KEY_ENV: &str = "MOMOTOKEN_API_KEY";
const DSH_DISPLAY_NAME: &str = "Niko / momotoken";
const DSH_WEB_PORT: u16 = 3080;
const DSH_WEB_HOST: &str = "127.0.0.1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PreviousDefault {
    provider: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
}

pub struct DshTarget;

fn dsh_home(home: &Path) -> PathBuf {
    home.join(".dsh")
}

fn settings_path(home: &Path) -> PathBuf {
    dsh_home(home).join("settings.yaml")
}

fn credentials_path(home: &Path) -> PathBuf {
    dsh_home(home).join(".credentials.yaml")
}

fn previous_default_path(home: &Path) -> PathBuf {
    home.join(".niko").join("dsh-previous-default.json")
}

fn niko_dir(home: &Path) -> PathBuf {
    home.join(".niko")
}

pub(crate) fn dsh_settings_path() -> PathBuf {
    settings_path(&home_dir())
}

pub(crate) fn dsh_credentials_path() -> PathBuf {
    credentials_path(&home_dir())
}

pub(crate) fn dsh_openai_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/v1")
    }
}

fn find_dsh_executable() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let extras = {
        let mut list = vec![home_dir().join(".local").join("bin").join("dsh.exe")];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            list.push(PathBuf::from(local).join("dsh").join("dsh.exe"));
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let npm = PathBuf::from(appdata).join("npm");
            list.push(npm.join("dsh.cmd"));
            list.push(npm.join("dsh.exe"));
        }
        list
    };
    #[cfg(not(target_os = "windows"))]
    let extras = vec![
        home_dir().join(".local").join("bin").join("dsh"),
        PathBuf::from("/usr/local/bin/dsh"),
        PathBuf::from("/opt/homebrew/bin/dsh"),
    ];
    find_cli_executable("dsh", &extras)
}

fn provider_body(plan: &ApplyPlan) -> String {
    let model = quote_yaml_scalar(&plan.model.clone().unwrap_or_default());
    let base = quote_yaml_scalar(&dsh_openai_base_url(&plan.base_url));
    format!(
        "displayName: {}\n\
apiKeyEnv: {DSH_API_KEY_ENV}\n\
api: openai-completions\n\
baseURL: {base}\n\
models:\n\
  - id: {model}\n\
    name: {model}\n",
        quote_yaml_scalar(DSH_DISPLAY_NAME)
    )
}

fn default_model_body(plan: &ApplyPlan) -> String {
    let model = quote_yaml_scalar(&plan.model.clone().unwrap_or_default());
    format!("provider: {DSH_PROVIDER}\nmodel: {model}\n")
}

fn chmod_owner_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(path) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o600);
            let _ = fs::set_permissions(path, permissions);
        }
    }
    let _ = path;
}

fn write_yaml(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let snap = fsx::write_with_snapshot(path, content.as_bytes()).map_err(|error| error.to_string())?;
    snap.commit();
    chmod_owner_only(path);
    Ok(())
}

fn save_previous_default(home: &Path, settings: &serde_yaml::Value) -> Result<(), String> {
    let current = mapping_get(settings, "agent-default-model");
    let provider = current.and_then(|value| mapping_str(value, "provider"));
    if provider.as_deref() == Some(DSH_PROVIDER) {
        return Ok(());
    }
    let previous = PreviousDefault {
        provider,
        model: current.and_then(|value| mapping_str(value, "model")),
        reasoning_effort: current.and_then(|value| mapping_str(value, "reasoningEffort")),
    };
    let path = previous_default_path(home);
    fs::create_dir_all(niko_dir(home)).map_err(|error| error.to_string())?;
    let content = serde_json::to_string_pretty(&previous).map_err(|error| error.to_string())?;
    fs::write(&path, content).map_err(|error| error.to_string())
}

fn apply_settings(home: &Path, plan: &ApplyPlan) -> Result<Vec<String>, String> {
    let path = settings_path(home);
    let _ = save_backup(DSH_TARGET_ID, &path);
    let raw = if path.exists() {
        fs::read_to_string(&path).map_err(|error| error.to_string())?
    } else {
        String::new()
    };
    let parsed = yaml_mapping_or_err(&raw, "~/.dsh/settings.yaml")?;
    save_previous_default(home, &parsed)?;

    let mut changed = Vec::new();
    let (with_provider, provider_changed) = upsert_child_mapping(
        &raw,
        &["llm-pi-ai", "providers"],
        DSH_PROVIDER,
        &provider_body(plan),
    )?;
    if provider_changed {
        changed.push(format!("settings.yaml:llm-pi-ai.providers.{DSH_PROVIDER}"));
    }
    let (next, default_changed) =
        upsert_top_mapping(&with_provider, "agent-default-model", &default_model_body(plan))?;
    if default_changed {
        changed.push("settings.yaml:agent-default-model".to_owned());
    }
    if !changed.is_empty() {
        write_yaml(&path, &next)?;
    }
    Ok(changed)
}

fn apply_credentials(home: &Path, plan: &ApplyPlan) -> Result<Vec<String>, String> {
    let path = credentials_path(home);
    let _ = save_backup(DSH_TARGET_ID, &path);
    let raw = if path.exists() {
        fs::read_to_string(&path).map_err(|error| error.to_string())?
    } else {
        "version: 1\n".to_owned()
    };
    let _ = yaml_mapping_or_err(&raw, "~/.dsh/.credentials.yaml")?;
    let (with_version, _) = if raw.contains("version:") {
        (raw, false)
    } else {
        upsert_child_scalar(&raw, &[], "version", "1")?
    };
    let (next, changed) = upsert_child_scalar(&with_version, &["refs"], DSH_API_KEY_ENV, &plan.api_key)?;
    if changed {
        write_yaml(&path, &next)?;
        return Ok(vec![format!(".credentials.yaml:refs.{DSH_API_KEY_ENV}")]);
    }
    Ok(Vec::new())
}

pub(crate) fn dsh_apply(plan: &ApplyPlan) -> Result<Vec<String>, String> {
    let home = home_dir();
    let mut changed = apply_settings(&home, plan)?;
    changed.append(&mut apply_credentials(&home, plan)?);
    Ok(changed)
}

pub(crate) fn dsh_restore() -> Result<Vec<String>, String> {
    let home = home_dir();
    let mut changed = Vec::new();
    let settings = settings_path(&home);
    if settings.exists() {
        let _ = save_backup(DSH_TARGET_ID, &settings);
        let raw = fs::read_to_string(&settings).map_err(|error| error.to_string())?;
        let _ = yaml_mapping_or_err(&raw, "~/.dsh/settings.yaml")?;
        let (without_provider, provider_changed) =
            remove_child_mapping(&raw, &["llm-pi-ai", "providers"], DSH_PROVIDER)?;
        if provider_changed {
            changed.push(format!("-settings.yaml:llm-pi-ai.providers.{DSH_PROVIDER}"));
        }
        let next = restore_previous_default(&home, &without_provider, &mut changed)?;
        if next != raw {
            write_yaml(&settings, &next)?;
        }
    }

    let credentials = credentials_path(&home);
    if credentials.exists() {
        let _ = save_backup(DSH_TARGET_ID, &credentials);
        let raw = fs::read_to_string(&credentials).map_err(|error| error.to_string())?;
        let _ = yaml_mapping_or_err(&raw, "~/.dsh/.credentials.yaml")?;
        let (next, removed) = remove_child_scalar(&raw, &["refs"], DSH_API_KEY_ENV)?;
        if removed {
            write_yaml(&credentials, &next)?;
            changed.push(format!("-.credentials.yaml:refs.{DSH_API_KEY_ENV}"));
        }
    }
    Ok(changed)
}

fn restore_previous_default(
    home: &Path,
    settings: &str,
    changed: &mut Vec<String>,
) -> Result<String, String> {
    let parsed = yaml_mapping_or_err(settings, "~/.dsh/settings.yaml")?;
    let current_provider = mapping_get(&parsed, "agent-default-model")
        .and_then(|value| mapping_str(value, "provider"));
    if current_provider.as_deref() != Some(DSH_PROVIDER) {
        return Ok(settings.to_owned());
    }
    let sidecar = previous_default_path(home);
    if let Ok(raw) = fs::read_to_string(&sidecar) {
        if let Ok(previous) = serde_json::from_str::<PreviousDefault>(&raw) {
            if let (Some(provider), Some(model)) = (previous.provider, previous.model) {
                let mut body = format!("provider: {provider}\nmodel: {model}\n");
                if let Some(effort) = previous.reasoning_effort {
                    body.push_str(&format!("reasoningEffort: {effort}\n"));
                }
                let (next, restored) = upsert_top_mapping(settings, "agent-default-model", &body)?;
                if restored {
                    changed.push("settings.yaml:agent-default-model".to_owned());
                }
                let _ = fs::remove_file(sidecar);
                return Ok(next);
            }
        }
    }
    let (next, removed) = remove_child_mapping(settings, &[], "agent-default-model")?;
    if removed {
        changed.push("-settings.yaml:agent-default-model".to_owned());
    }
    let _ = fs::remove_file(sidecar);
    Ok(next)
}

pub(crate) fn dsh_effective() -> Result<EffectiveConfig, String> {
    dsh_effective_at(&home_dir())
}

fn dsh_effective_at(home: &Path) -> Result<EffectiveConfig, String> {
    let settings = fs::read_to_string(settings_path(home))
        .map_err(|_| "未找到 DSH 配置，请先点击接入".to_owned())?;
    let doc = yaml_mapping_or_err(&settings, "~/.dsh/settings.yaml")?;
    let provider = mapping_get(&doc, "llm-pi-ai")
        .and_then(|value| mapping_get(value, "providers"))
        .and_then(|value| mapping_get(value, DSH_PROVIDER))
        .ok_or("DSH 配置里缺少 Niko 接入信息，请先点击接入")?;
    let base_url = mapping_str(provider, "baseURL").ok_or("momotoken 缺少 baseURL")?;
    let credentials = fs::read_to_string(credentials_path(home))
        .map_err(|_| "未找到 DSH 密钥，请先点击接入".to_owned())?;
    let creds = yaml_mapping_or_err(&credentials, "~/.dsh/.credentials.yaml")?;
    let api_key = mapping_get(&creds, "refs")
        .and_then(|value| mapping_str(value, DSH_API_KEY_ENV))
        .ok_or("DSH 密钥里缺少 MOMOTOKEN_API_KEY，请先点击接入")?;
    let model = mapping_get(provider, "models")
        .and_then(serde_yaml::Value::as_sequence)
        .and_then(|models| models.first())
        .and_then(|model| mapping_str(model, "id"));
    Ok(EffectiveConfig {
        target_id: DSH_TARGET_ID.to_owned(),
        endpoint: format!("{}/chat/completions", base_url.trim_end_matches('/')),
        api_key,
        model,
        auth_style: "openai-chat".to_owned(),
    })
}

pub(crate) fn dsh_drift(home: &Path, plan: &ApplyPlan) -> Vec<String> {
    let mut mismatched = Vec::new();
    let path = settings_path(home);
    if !path.exists() {
        mismatched.push("settings.yaml:missing".to_owned());
        return mismatched;
    }
    let Ok(raw) = fs::read_to_string(&path) else {
        mismatched.push("settings.yaml:unreadable".to_owned());
        return mismatched;
    };
    let Ok(doc) = yaml_mapping_or_err(&raw, "~/.dsh/settings.yaml") else {
        mismatched.push("settings.yaml:parse_error".to_owned());
        return mismatched;
    };
    let provider = mapping_get(&doc, "llm-pi-ai")
        .and_then(|value| mapping_get(value, "providers"))
        .and_then(|value| mapping_get(value, DSH_PROVIDER));
    let Some(provider) = provider else {
        mismatched.push(format!("settings.yaml:llm-pi-ai.providers.{DSH_PROVIDER}:missing"));
        return mismatched;
    };
    if mapping_str(provider, "baseURL").as_deref() != Some(dsh_openai_base_url(&plan.base_url).as_str())
    {
        mismatched.push(format!("settings.yaml:llm-pi-ai.providers.{DSH_PROVIDER}.baseURL"));
    }
    let expected_model = plan.model.clone().unwrap_or_default();
    let actual_model = mapping_get(provider, "models")
        .and_then(serde_yaml::Value::as_sequence)
        .and_then(|models| models.first())
        .and_then(|model| mapping_str(model, "id"));
    if actual_model.as_deref() != Some(expected_model.as_str()) {
        mismatched.push(format!("settings.yaml:llm-pi-ai.providers.{DSH_PROVIDER}.models"));
    }
    let default_provider = mapping_get(&doc, "agent-default-model")
        .and_then(|value| mapping_str(value, "provider"));
    if default_provider.as_deref() != Some(DSH_PROVIDER) {
        mismatched.push("settings.yaml:agent-default-model.provider".to_owned());
    }
    let credentials = credentials_path(home);
    if !credentials.exists() {
        mismatched.push(".credentials.yaml:missing".to_owned());
        return mismatched;
    }
    let Ok(cred_raw) = fs::read_to_string(&credentials) else {
        mismatched.push(".credentials.yaml:unreadable".to_owned());
        return mismatched;
    };
    let Ok(creds) = yaml_mapping_or_err(&cred_raw, "~/.dsh/.credentials.yaml") else {
        mismatched.push(".credentials.yaml:parse_error".to_owned());
        return mismatched;
    };
    if mapping_get(&creds, "refs")
        .and_then(|value| mapping_str(value, DSH_API_KEY_ENV))
        .as_deref()
        != Some(plan.api_key.as_str())
    {
        mismatched.push(format!(".credentials.yaml:refs.{DSH_API_KEY_ENV}"));
    }
    mismatched
}

pub(crate) fn observe_dsh_config(home: &Path) -> TargetConfigObservation {
    let settings = settings_path(home);
    if !settings.exists() {
        return TargetConfigObservation::Other;
    }
    let Ok(raw) = fs::read_to_string(&settings) else {
        return TargetConfigObservation::Unreadable;
    };
    let Ok(doc) = yaml_mapping_or_err(&raw, "~/.dsh/settings.yaml") else {
        return TargetConfigObservation::Unreadable;
    };
    let Some(provider) = mapping_get(&doc, "llm-pi-ai")
        .and_then(|value| mapping_get(value, "providers"))
        .and_then(|value| mapping_get(value, DSH_PROVIDER))
    else {
        return TargetConfigObservation::Other;
    };
    let Some(base_url) = mapping_str(provider, "baseURL") else {
        return TargetConfigObservation::Ambiguous;
    };
    let Ok(effective) = dsh_effective_at(home) else {
        return TargetConfigObservation::Unreadable;
    };
    super::matchable_target_config(
        DSH_TARGET_ID,
        &format!("{}/chat/completions", base_url.trim_end_matches('/')),
        "openai-chat",
        effective.model.as_deref(),
        &effective.api_key,
    )
}

pub fn dsh_web_url() -> String {
    format!("http://{DSH_WEB_HOST}:{DSH_WEB_PORT}")
}

pub fn dsh_port_open() -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("{DSH_WEB_HOST}:{DSH_WEB_PORT}")
            .parse()
            .unwrap_or_else(|_| std::net::SocketAddr::from(([127, 0, 0, 1], DSH_WEB_PORT))),
        std::time::Duration::from_millis(200),
    )
    .is_ok()
}

pub fn open_dsh_in_browser() -> Result<(), String> {
    let url = dsh_web_url();
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .status()
            .map_err(|error| format!("打开浏览器失败：{error}"))?
            .success()
            .then_some(())
            .ok_or_else(|| "打开浏览器失败，请手动访问 DSH 页面".to_owned())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .status()
            .map_err(|error| format!("打开浏览器失败：{error}"))?
            .success()
            .then_some(())
            .ok_or_else(|| "打开浏览器失败，请手动访问 DSH 页面".to_owned())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = url;
        Err("当前平台不支持一键打开 DSH".to_owned())
    }
}

pub fn spawn_dsh_web() -> Result<(), String> {
    let exe = find_dsh_executable().ok_or("还没有找到 DSH，请先安装后再打开")?;
    let mut command = std::process::Command::new(&exe);
    command
        .arg("web")
        .arg("--host")
        .arg(DSH_WEB_HOST)
        .arg("--port")
        .arg(DSH_WEB_PORT.to_string())
        .arg("--no-open");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("启动 DSH 失败：{error}"))
}

pub fn wait_for_dsh_port(attempts: usize, delay: std::time::Duration) -> bool {
    for _ in 0..attempts {
        if dsh_port_open() {
            return true;
        }
        std::thread::sleep(delay);
    }
    false
}

impl Target for DshTarget {
    fn id(&self) -> &'static str {
        DSH_TARGET_ID
    }
    fn display_name(&self) -> &'static str {
        "DSH"
    }

    fn is_installed(&self) -> bool {
        find_dsh_executable().is_some()
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplySummary, String> {
        let changed = dsh_apply(plan)?;
        Ok(ApplySummary {
            target_id: self.id().to_owned(),
            changed_keys: changed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::targets::set_test_home;

    fn tmp_dir(name: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "dsh_target_{}_{}_{}",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn plan() -> ApplyPlan {
        ApplyPlan {
            base_url: "https://momotoken.win/v1".to_owned(),
            api_key: "sk-test".to_owned(),
            model_group: Some("default".to_owned()),
            model: Some("gpt-5.4".to_owned()),
            codex_mixed: false,
        }
    }

    #[test]
    fn apply_writes_provider_credential_and_default_model() {
        let home = tmp_dir("apply");
        fs::create_dir_all(home.join(".dsh")).unwrap();
        fs::write(
            home.join(".dsh").join("settings.yaml"),
            "\
llm-pi-ai:
  providers:
    grok-cli:
      apiKeyEnv: GROK_CLI_API_KEY
agent-default-model:
  provider: grok
  model: grok-4.6
",
        )
        .unwrap();
        let _guard = set_test_home(&home);
        let summary = DshTarget.apply(&plan()).unwrap();
        assert_eq!(summary.target_id, "dsh");
        assert!(!summary.changed_keys.is_empty());
        let settings = fs::read_to_string(home.join(".dsh").join("settings.yaml")).unwrap();
        assert!(settings.contains("grok-cli:"));
        assert!(settings.contains("momotoken:"));
        assert!(settings.contains("baseURL: https://momotoken.win/v1"));
        assert!(settings.contains("provider: momotoken"));
        let creds = fs::read_to_string(home.join(".dsh").join(".credentials.yaml")).unwrap();
        assert!(creds.contains("MOMOTOKEN_API_KEY: sk-test"));
        let again = DshTarget.apply(&plan()).unwrap();
        assert!(again.changed_keys.is_empty());
    }

    #[test]
    fn restore_removes_only_niko_and_puts_default_back() {
        let home = tmp_dir("restore");
        fs::create_dir_all(home.join(".dsh")).unwrap();
        fs::write(
            home.join(".dsh").join("settings.yaml"),
            "\
llm-pi-ai:
  providers:
    grok-cli:
      apiKeyEnv: GROK_CLI_API_KEY
agent-default-model:
  provider: grok
  model: grok-4.6
",
        )
        .unwrap();
        let _guard = set_test_home(&home);
        DshTarget.apply(&plan()).unwrap();
        let changed = dsh_restore().unwrap();
        assert!(!changed.is_empty());
        let settings = fs::read_to_string(home.join(".dsh").join("settings.yaml")).unwrap();
        assert!(settings.contains("grok-cli:"));
        assert!(!settings.contains("momotoken:"));
        assert!(settings.contains("provider: grok"));
        let creds = fs::read_to_string(home.join(".dsh").join(".credentials.yaml")).unwrap();
        assert!(!creds.contains("MOMOTOKEN_API_KEY"));
    }
}
