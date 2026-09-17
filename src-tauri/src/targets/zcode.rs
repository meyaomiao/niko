//! ZCode 桌面端：只新增自定义槽 `custom:momotoken`，不改智谱 OAuth。

use super::{
    claude_base_url, home_dir, ApplyPlan, ApplySummary, EffectiveConfig, Target,
    TargetConfigObservation,
};
use crate::commands::snapshots::save_backup;
use crate::fsx;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const ZCODE_TARGET_ID: &str = "zcode";
const ZCODE_PROVIDER_ID: &str = "custom:momotoken";
const ZCODE_PROVIDER_NAME: &str = "Niko / momotoken";
#[cfg(target_os = "macos")]
const ZCODE_APP_NAMES: &[&str] = &["ZCode.app"];

pub struct ZcodeTarget;

fn zcode_config_path(home: &Path) -> PathBuf {
    home.join(".zcode").join("v2").join("config.json")
}

pub(crate) fn zcode_config_file() -> PathBuf {
    zcode_config_path(&home_dir())
}

fn zcode_base_url(base_url: &str) -> String {
    claude_base_url(base_url)
}

#[cfg(target_os = "windows")]
fn zcode_windows_exe() -> Option<PathBuf> {
    let local = PathBuf::from(std::env::var("LOCALAPPDATA").ok()?);
    [
        local.join("Programs").join("ZCode").join("ZCode.exe"),
        local.join("ZCode").join("ZCode.exe"),
        local.join("Programs").join("zcode").join("ZCode.exe"),
    ]
    .into_iter()
    .find(|path| path.exists())
}

fn model_entry() -> Value {
    json!({
        "reasoning": {
            "enabled": true,
            "variants": ["low", "high", "max"],
            "defaultVariant": "high"
        },
        "limit": {
            "context": 200000,
            "output": 128000
        },
        "modalities": {
            "input": ["text"],
            "output": ["text"]
        },
        "zcode": {
            "modified": false,
            "priority": 1
        }
    })
}

fn desired_provider(plan: &ApplyPlan) -> Value {
    let model = plan.model.clone().unwrap_or_default();
    let mut models = Map::new();
    if !model.is_empty() {
        models.insert(model, model_entry());
    }
    json!({
        "name": ZCODE_PROVIDER_NAME,
        "kind": "anthropic",
        "options": {
            "apiKey": plan.api_key,
            "baseURL": zcode_base_url(&plan.base_url)
        },
        "enabled": true,
        "source": "custom",
        "models": Value::Object(models)
    })
}

fn read_config(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({ "provider": {} }));
    }
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    if raw.trim().is_empty() {
        return Ok(json!({ "provider": {} }));
    }
    serde_json::from_str(&raw).map_err(|error| format!("~/.zcode/v2/config.json 解析失败，未做任何修改：{error}"))
}

fn write_config(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let content = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    let snap = fsx::write_with_snapshot(path, content.as_bytes()).map_err(|error| error.to_string())?;
    snap.commit();
    Ok(())
}

pub(crate) fn zcode_apply(plan: &ApplyPlan) -> Result<Vec<String>, String> {
    let path = zcode_config_file();
    let _ = save_backup(ZCODE_TARGET_ID, &path);
    let mut root = read_config(&path)?;
    let object = root
        .as_object_mut()
        .ok_or("~/.zcode/v2/config.json 根节点不是 object")?;
    let providers = object
        .entry("provider")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("config.json 的 provider 不是 object")?;
    let desired = desired_provider(plan);
    let mut changed = Vec::new();
    match providers.get(ZCODE_PROVIDER_ID) {
        Some(existing) if existing == &desired => {}
        _ => {
            providers.insert(ZCODE_PROVIDER_ID.to_owned(), desired);
            changed.push(format!("config.json:provider.{ZCODE_PROVIDER_ID}"));
        }
    }
    if !changed.is_empty() {
        write_config(&path, &root)?;
    }
    Ok(changed)
}

pub(crate) fn zcode_restore() -> Result<Vec<String>, String> {
    let path = zcode_config_file();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let _ = save_backup(ZCODE_TARGET_ID, &path);
    let mut root = read_config(&path)?;
    let object = root
        .as_object_mut()
        .ok_or("~/.zcode/v2/config.json 根节点不是 object")?;
    let Some(providers) = object.get_mut("provider").and_then(Value::as_object_mut) else {
        return Ok(Vec::new());
    };
    if providers.remove(ZCODE_PROVIDER_ID).is_none() {
        return Ok(Vec::new());
    }
    write_config(&path, &root)?;
    Ok(vec![format!("-config.json:provider.{ZCODE_PROVIDER_ID}")])
}

pub(crate) fn zcode_effective() -> Result<EffectiveConfig, String> {
    zcode_effective_at(&home_dir())
}

fn zcode_effective_at(home: &Path) -> Result<EffectiveConfig, String> {
    let path = zcode_config_path(home);
    let root = read_config(&path)?;
    let provider = root
        .get("provider")
        .and_then(Value::as_object)
        .and_then(|providers| providers.get(ZCODE_PROVIDER_ID))
        .ok_or("ZCode 配置里缺少 Niko 接入信息，请先点击接入")?;
    let options = provider
        .get("options")
        .and_then(Value::as_object)
        .ok_or("custom:momotoken 缺少 options")?;
    let base_url = options
        .get("baseURL")
        .and_then(Value::as_str)
        .ok_or("custom:momotoken 缺少 baseURL")?;
    let api_key = options
        .get("apiKey")
        .and_then(Value::as_str)
        .ok_or("custom:momotoken 缺少 apiKey")?;
    let model = provider
        .get("models")
        .and_then(Value::as_object)
        .and_then(|models| models.keys().next())
        .cloned();
    Ok(EffectiveConfig {
        target_id: ZCODE_TARGET_ID.to_owned(),
        endpoint: format!("{}/v1/messages", base_url.trim_end_matches('/')),
        api_key: api_key.to_owned(),
        model,
        auth_style: "anthropic".to_owned(),
    })
}

pub(crate) fn zcode_drift(home: &Path, plan: &ApplyPlan) -> Vec<String> {
    let mut mismatched = Vec::new();
    let path = zcode_config_path(home);
    if !path.exists() {
        mismatched.push("config.json:missing".to_owned());
        return mismatched;
    }
    let Ok(root) = read_config(&path) else {
        mismatched.push("config.json:parse_error".to_owned());
        return mismatched;
    };
    let Some(provider) = root
        .get("provider")
        .and_then(Value::as_object)
        .and_then(|providers| providers.get(ZCODE_PROVIDER_ID))
    else {
        mismatched.push(format!("config.json:provider.{ZCODE_PROVIDER_ID}:missing"));
        return mismatched;
    };
    if provider != &desired_provider(plan) {
        mismatched.push(format!("config.json:provider.{ZCODE_PROVIDER_ID}"));
    }
    mismatched
}

pub(crate) fn observe_zcode_config(home: &Path) -> TargetConfigObservation {
    let path = zcode_config_path(home);
    if !path.exists() {
        return TargetConfigObservation::Other;
    }
    let Ok(root) = read_config(&path) else {
        return TargetConfigObservation::Unreadable;
    };
    let Some(provider) = root
        .get("provider")
        .and_then(Value::as_object)
        .and_then(|providers| providers.get(ZCODE_PROVIDER_ID))
    else {
        return TargetConfigObservation::Other;
    };
    let Some(options) = provider.get("options").and_then(Value::as_object) else {
        return TargetConfigObservation::Ambiguous;
    };
    let Some(base_url) = options.get("baseURL").and_then(Value::as_str) else {
        return TargetConfigObservation::Ambiguous;
    };
    let Some(api_key) = options.get("apiKey").and_then(Value::as_str) else {
        return TargetConfigObservation::Unreadable;
    };
    let model = provider
        .get("models")
        .and_then(Value::as_object)
        .and_then(|models| models.keys().next())
        .map(String::as_str);
    super::matchable_target_config(
        ZCODE_TARGET_ID,
        &format!("{}/v1/messages", base_url.trim_end_matches('/')),
        "anthropic",
        model,
        api_key,
    )
}

impl Target for ZcodeTarget {
    fn id(&self) -> &'static str {
        ZCODE_TARGET_ID
    }
    fn display_name(&self) -> &'static str {
        "ZCode"
    }

    fn is_installed(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            if super::macos_app_exists(ZCODE_APP_NAMES) {
                return true;
            }
        }
        #[cfg(target_os = "windows")]
        {
            if zcode_windows_exe().is_some() {
                return true;
            }
        }
        false
    }

    fn icon_data_uri(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            return super::macos_app_icon_data_uri(ZCODE_APP_NAMES);
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    fn apply(&self, plan: &ApplyPlan) -> Result<ApplySummary, String> {
        let changed = zcode_apply(plan)?;
        Ok(ApplySummary {
            target_id: self.id().to_owned(),
            changed_keys: changed,
        })
    }
}

pub fn zcode_app_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return super::macos_app_path(ZCODE_APP_NAMES);
    }
    #[cfg(target_os = "windows")]
    {
        return zcode_windows_exe();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::targets::set_test_home;

    fn tmp_dir(name: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "zcode_target_{}_{}_{}",
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
            model: Some("claude-sonnet-4-5".to_owned()),
            codex_mixed: false,
        }
    }

    #[test]
    fn apply_adds_custom_slot_without_touching_builtin() {
        let home = tmp_dir("apply");
        let config_dir = home.join(".zcode").join("v2");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.json"),
            r#"{"provider":{"builtin:bigmodel":{"name":"Bigmodel","enabled":true}}}"#,
        )
        .unwrap();
        let _guard = set_test_home(&home);
        let summary = ZcodeTarget.apply(&plan()).unwrap();
        assert_eq!(summary.target_id, "zcode");
        let raw = fs::read_to_string(config_dir.join("config.json")).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert!(value.pointer("/provider/builtin:bigmodel").is_some());
        assert_eq!(
            value
                .pointer("/provider/custom:momotoken/options/baseURL")
                .and_then(Value::as_str),
            Some("https://momotoken.win")
        );
        assert_eq!(
            value
                .pointer("/provider/custom:momotoken/kind")
                .and_then(Value::as_str),
            Some("anthropic")
        );
        let again = ZcodeTarget.apply(&plan()).unwrap();
        assert!(again.changed_keys.is_empty());
    }

    #[test]
    fn restore_removes_only_custom_slot() {
        let home = tmp_dir("restore");
        let config_dir = home.join(".zcode").join("v2");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.json"),
            r#"{"provider":{"builtin:bigmodel":{"name":"Bigmodel"}}}"#,
        )
        .unwrap();
        let _guard = set_test_home(&home);
        ZcodeTarget.apply(&plan()).unwrap();
        let changed = zcode_restore().unwrap();
        assert_eq!(changed, vec!["-config.json:provider.custom:momotoken".to_owned()]);
        let raw = fs::read_to_string(config_dir.join("config.json")).unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert!(value.pointer("/provider/custom:momotoken").is_none());
        assert!(value.pointer("/provider/builtin:bigmodel").is_some());
    }
}
