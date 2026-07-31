use crate::codex_sessions::CodexMigrationInput;
use crate::commands::codex_sessions::{
    normalize_codex_session_storage_inner, normalize_codex_session_storage_with_input,
    preflight_codex_session_storage, recover_codex_session_storage_since,
};
use crate::commands::safe_error::SafeCommandError;
use crate::targets::{all_targets, preflight_target_apply, transaction_paths, ApplyPlan};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static PROVIDER_TRANSACTION_LOCK: Mutex<()> = Mutex::new(());
const PROVIDER_TRANSACTION_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProviderTransactionPhase {
    Prepared,
    ClaudeApplied,
    CodexStarted,
    Committed,
}

#[derive(Debug, Deserialize, Serialize)]
struct ProviderTransactionManifest {
    version: u8,
    phase: ProviderTransactionPhase,
    existed: Vec<bool>,
    known_codex_transactions: Vec<String>,
}

fn provider_transaction_root() -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Users\default\AppData\Roaming"));
    #[cfg(not(target_os = "windows"))]
    let base = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    base.join(".niko").join("provider-transaction")
}

fn sync_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn persist_provider_manifest(
    root: &Path,
    manifest: &ProviderTransactionManifest,
) -> Result<(), SafeCommandError> {
    let path = root.join("manifest.json");
    let temporary = root.join("manifest.tmp");
    let mut bytes =
        serde_json::to_vec(manifest).map_err(|_| SafeCommandError::change_failed(false))?;
    bytes.push(b'\n');
    let mut file = File::create(&temporary).map_err(|_| SafeCommandError::change_failed(false))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| SafeCommandError::change_failed(false))?;
    fs::rename(&temporary, &path)
        .and_then(|_| sync_parent(&path))
        .map_err(|_| SafeCommandError::change_failed(false))
}

fn begin_provider_transaction_at(
    root: &Path,
    paths: &[PathBuf],
    known_codex_transactions: Vec<String>,
) -> Result<ProviderTransactionManifest, SafeCommandError> {
    fs::create_dir_all(root.parent().expect("transaction root has parent"))
        .map_err(|_| SafeCommandError::change_failed(false))?;
    fs::create_dir(root).map_err(|_| SafeCommandError::busy())?;
    let result = (|| {
        let mut existed = Vec::with_capacity(paths.len());
        for (index, path) in paths.iter().enumerate() {
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_file() => {
                    let backup = root.join(format!("{index}.backup"));
                    fs::copy(path, &backup)
                        .and_then(|_| File::open(&backup)?.sync_all())
                        .map_err(|_| SafeCommandError::change_failed(false))?;
                    existed.push(true);
                }
                Ok(_) => return Err(SafeCommandError::change_failed(false)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => existed.push(false),
                Err(_) => return Err(SafeCommandError::change_failed(false)),
            }
        }
        let manifest = ProviderTransactionManifest {
            version: PROVIDER_TRANSACTION_VERSION,
            phase: ProviderTransactionPhase::Prepared,
            existed,
            known_codex_transactions,
        };
        persist_provider_manifest(&root, &manifest)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(root);
    }
    result
}

fn restore_provider_transaction_at(
    root: &Path,
    paths: &[PathBuf],
    manifest: &ProviderTransactionManifest,
) -> Result<(), SafeCommandError> {
    if manifest.version != PROVIDER_TRANSACTION_VERSION || paths.len() != manifest.existed.len() {
        return Err(SafeCommandError::change_failed(false));
    }
    for (index, (path, existed)) in paths.iter().zip(&manifest.existed).enumerate() {
        if *existed {
            let backup = root.join(format!("{index}.backup"));
            let parent = path
                .parent()
                .ok_or_else(|| SafeCommandError::change_failed(false))?;
            fs::create_dir_all(parent).map_err(|_| SafeCommandError::change_failed(false))?;
            let temporary = path.with_extension("niko-restore");
            fs::copy(&backup, &temporary)
                .and_then(|_| File::open(&temporary)?.sync_all())
                .and_then(|_| fs::rename(&temporary, path))
                .and_then(|_| sync_parent(path))
                .map_err(|_| SafeCommandError::change_failed(false))?;
        } else if path.exists() {
            fs::remove_file(path)
                .and_then(|_| sync_parent(path))
                .map_err(|_| SafeCommandError::change_failed(false))?;
        }
    }
    fs::remove_dir_all(&root)
        .and_then(|_| sync_parent(&root))
        .map_err(|_| SafeCommandError::change_failed(false))
}

fn restore_provider_transaction(
    manifest: &ProviderTransactionManifest,
) -> Result<(), SafeCommandError> {
    let paths =
        transaction_paths("claude-desktop").map_err(|_| SafeCommandError::change_failed(false))?;
    restore_provider_transaction_at(&provider_transaction_root(), &paths, manifest)
}

fn finish_provider_transaction() -> Result<(), SafeCommandError> {
    let root = provider_transaction_root();
    if !root.exists() {
        return Ok(());
    }
    fs::remove_dir_all(&root)
        .and_then(|_| sync_parent(&root))
        .map_err(|_| SafeCommandError::change_failed(false))
}

fn recover_provider_transaction() -> Result<(), SafeCommandError> {
    let root = provider_transaction_root();
    if !root.exists() {
        return Ok(());
    }
    let bytes = match fs::read(root.join("manifest.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return finish_provider_transaction();
        }
        Err(_) => return Err(SafeCommandError::change_failed(false)),
    };
    let manifest: ProviderTransactionManifest =
        serde_json::from_slice(&bytes).map_err(|_| SafeCommandError::change_failed(false))?;
    match manifest.phase {
        ProviderTransactionPhase::Committed => finish_provider_transaction(),
        ProviderTransactionPhase::CodexStarted => {
            if recover_codex_session_storage_since(&manifest.known_codex_transactions)?
                == Some(true)
            {
                finish_provider_transaction()
            } else {
                restore_provider_transaction(&manifest)
            }
        }
        ProviderTransactionPhase::Prepared | ProviderTransactionPhase::ClaudeApplied => {
            restore_provider_transaction(&manifest)
        }
    }
}

pub(crate) fn lock_and_recover_provider_transaction(
) -> Result<MutexGuard<'static, ()>, SafeCommandError> {
    let guard = PROVIDER_TRANSACTION_LOCK
        .try_lock()
        .map_err(|_| SafeCommandError::busy())?;
    recover_provider_transaction()?;
    Ok(guard)
}

#[derive(Debug, Serialize)]
pub struct TargetInfo {
    pub id: String,
    pub name: String,
    pub installed: bool,
    /// 本机已安装时提取到的真实应用图标（PNG data URI）
    pub icon: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApplyRequest {
    pub target_id: String,
    pub base_url: String,
    pub api_key: String,
    pub model_group: Option<String>,
    pub model: Option<String>,
    /// Codex 混用模式（保留 ChatGPT 登录态），仅对 codex 目标有意义
    #[serde(default)]
    pub codex_mixed: bool,
}

#[tauri::command]
pub async fn list_targets() -> Result<Vec<TargetInfo>, SafeCommandError> {
    let _guard = lock_and_recover_provider_transaction()?;
    let infos = all_targets()
        .iter()
        .map(|t| TargetInfo {
            id: t.id().to_owned(),
            name: t.display_name().to_owned(),
            installed: t.is_installed(),
            icon: t.icon_data_uri(),
        })
        .collect();
    Ok(infos)
}

#[tauri::command]
pub async fn apply_target(req: ApplyRequest) -> Result<Vec<String>, SafeCommandError> {
    let _guard = lock_and_recover_provider_transaction()?;
    let plan = ApplyPlan {
        base_url: req.base_url,
        api_key: req.api_key,
        model_group: req.model_group,
        model: req.model,
        codex_mixed: req.codex_mixed,
    };
    if req.target_id == "codex" {
        let result = normalize_codex_session_storage_with_input(
            "custom".to_owned(),
            Some(CodexMigrationInput {
                base_url: Some(plan.base_url),
                api_key: Some(plan.api_key),
                model: plan.model,
                mixed: plan.codex_mixed,
            }),
        )?;
        return Ok(if result.status == "unchanged" {
            Vec::new()
        } else {
            vec!["codex".to_owned()]
        });
    }

    let targets = all_targets();
    let target = targets
        .iter()
        .find(|t| t.id() == req.target_id)
        .ok_or_else(SafeCommandError::invalid_request)?;

    let summary = target
        .apply(&plan)
        .map_err(|_| SafeCommandError::change_failed(true))?;
    crate::logx::append(
        "apply_target",
        &format!("{} changed={:?}", req.target_id, summary.changed_keys),
    );
    Ok(summary.changed_keys)
}

/// 对所有已安装目标同时应用 plan
#[tauri::command]
pub async fn apply_all_targets(
    base_url: String,
    api_key: String,
    model_group: Option<String>,
    model: Option<String>,
    codex_mixed: Option<bool>,
) -> Result<Vec<serde_json::Value>, SafeCommandError> {
    let _guard = lock_and_recover_provider_transaction()?;
    let plan = ApplyPlan {
        base_url,
        api_key,
        model_group,
        model,
        codex_mixed: codex_mixed.unwrap_or(false),
    };
    let targets = all_targets();
    let codex = targets
        .iter()
        .find(|target| target.id() == "codex" && target.is_installed());
    let claude = targets
        .iter()
        .find(|target| target.id() == "claude-desktop" && target.is_installed());
    if codex.is_none() && claude.is_none() {
        return Ok(Vec::new());
    }

    let codex_input = CodexMigrationInput {
        base_url: Some(plan.base_url.clone()),
        api_key: Some(plan.api_key.clone()),
        model: plan.model.clone(),
        mixed: plan.codex_mixed,
    };
    if claude.is_none() {
        let outcome =
            normalize_codex_session_storage_with_input("custom".to_owned(), Some(codex_input))?;
        return Ok(vec![serde_json::json!({
            "id": "codex",
            "ok": true,
            "changed": if outcome.status == "unchanged" { Vec::<String>::new() } else { vec!["codex".to_owned()] }
        })]);
    }
    if codex.is_none() {
        let summary = claude
            .expect("installed Claude target")
            .apply(&plan)
            .map_err(|_| SafeCommandError::change_failed(true))?;
        return Ok(vec![serde_json::json!({
            "id": summary.target_id,
            "ok": true,
            "changed": summary.changed_keys
        })]);
    }

    let known_codex_transactions = preflight_codex_session_storage(codex_input.clone())?;
    preflight_target_apply("claude-desktop").map_err(|_| SafeCommandError::change_failed(false))?;
    let paths =
        transaction_paths("claude-desktop").map_err(|_| SafeCommandError::change_failed(false))?;
    let mut manifest = begin_provider_transaction_at(
        &provider_transaction_root(),
        &paths,
        known_codex_transactions,
    )?;

    let claude_summary = match claude.expect("installed Claude target").apply(&plan) {
        Ok(summary) => summary,
        Err(_) => {
            restore_provider_transaction(&manifest)?;
            return Err(SafeCommandError::change_failed(true));
        }
    };
    manifest.phase = ProviderTransactionPhase::ClaudeApplied;
    persist_provider_manifest(&provider_transaction_root(), &manifest)?;
    manifest.phase = ProviderTransactionPhase::CodexStarted;
    persist_provider_manifest(&provider_transaction_root(), &manifest)?;

    let codex_outcome =
        match normalize_codex_session_storage_with_input("custom".to_owned(), Some(codex_input)) {
            Ok(outcome) => outcome,
            Err(error) => {
                restore_provider_transaction(&manifest)?;
                return Err(error);
            }
        };
    manifest.phase = ProviderTransactionPhase::Committed;
    persist_provider_manifest(&provider_transaction_root(), &manifest)?;
    finish_provider_transaction()?;

    Ok(vec![
        serde_json::json!({
            "id": "codex",
            "ok": true,
            "changed": if codex_outcome.status == "unchanged" { Vec::<String>::new() } else { vec!["codex".to_owned()] }
        }),
        serde_json::json!({
            "id": claude_summary.target_id,
            "ok": true,
            "changed": claude_summary.changed_keys
        }),
    ])
}

use crate::targets::{check_drift, DriftReport};

#[tauri::command]
pub async fn check_drift_cmd(
    target_id: String,
    base_url: String,
    api_key: String,
    model_group: Option<String>,
    codex_mixed: Option<bool>,
) -> Result<DriftReport, String> {
    let plan = ApplyPlan {
        base_url,
        api_key,
        model_group,
        model: None,
        codex_mixed: codex_mixed.unwrap_or(false),
    };
    check_drift(&target_id, &plan)
}

#[tauri::command]
pub async fn check_all_drift(
    base_url: String,
    api_key: String,
    model_group: Option<String>,
    codex_mixed: Option<bool>,
) -> Result<Vec<DriftReport>, String> {
    let plan = ApplyPlan {
        base_url,
        api_key,
        model_group,
        model: None,
        codex_mixed: codex_mixed.unwrap_or(false),
    };
    let targets = all_targets();
    let mut reports = Vec::new();
    for t in &targets {
        if t.is_installed() {
            match check_drift(t.id(), &plan) {
                Ok(r) => reports.push(r),
                Err(e) => reports.push(DriftReport {
                    target_id: t.id().to_owned(),
                    drifted: true,
                    mismatched_keys: vec![format!("error: {e}")],
                }),
            }
        }
    }
    Ok(reports)
}

// ─── 连通性测试 ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ConnectivityResult {
    pub target_id: String,
    pub ok: bool,
    pub endpoint: String,
    pub model: Option<String>,
    pub latency_ms: Option<u64>,
    pub detail: String,
}

/// 用目标应用磁盘上真实生效的配置发一条最小请求，确认配置真的能用。
#[tauri::command]
pub async fn test_connectivity(target_id: String) -> Result<ConnectivityResult, String> {
    let cfg = crate::targets::effective_config(&target_id)?;
    let model = cfg
        .model
        .clone()
        .ok_or("配置里没有默认模型，请先点击启用")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    // 两家协议的请求体和鉴权头都不同，必须按各自格式发，否则测不出真实可用性
    let req = match cfg.auth_style.as_str() {
        "anthropic" => client
            .post(&cfg.endpoint)
            .header("x-api-key", &cfg.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "ping"}]
            })),
        _ => client
            .post(&cfg.endpoint)
            .header("Authorization", format!("Bearer {}", cfg.api_key))
            .json(&serde_json::json!({
                "model": model,
                "input": "ping",
                "max_output_tokens": 16
            })),
    };

    let t0 = std::time::Instant::now();
    let res = req.send().await;
    let ms = t0.elapsed().as_millis() as u64;

    match res {
        Ok(r) => {
            let code = r.status().as_u16();
            let ok = code < 300;
            let detail = if ok {
                format!("连通正常，{model} 可用（{:.1}s）", ms as f64 / 1000.0)
            } else {
                let body = r.text().await.unwrap_or_default();
                let hint = match code {
                    401 | 403 => "Key 无效或该分组无权限",
                    404 => "地址或模型不存在，可尝试重新启用",
                    429 => "请求过于频繁，稍后再试",
                    c if c >= 500 => "上游或服务端错误",
                    _ => "请求被拒绝",
                };
                format!(
                    "HTTP {code}：{hint}。{}",
                    body.chars().take(160).collect::<String>()
                )
            };
            crate::logx::append(
                "test_connectivity",
                &format!("{target_id} {} HTTP {code} {ms}ms", cfg.endpoint),
            );
            Ok(ConnectivityResult {
                target_id,
                ok,
                endpoint: cfg.endpoint,
                model: Some(model),
                latency_ms: Some(ms),
                detail,
            })
        }
        Err(e) => {
            crate::logx::append("test_connectivity", &format!("{target_id} error: {e}"));
            let detail = if e.is_timeout() {
                "请求超时，检查网络或代理设置".to_owned()
            } else if e.is_connect() {
                "无法连接服务器，检查网络或代理设置".to_owned()
            } else {
                format!("请求失败：{e}")
            };
            Ok(ConnectivityResult {
                target_id,
                ok: false,
                endpoint: cfg.endpoint,
                model: Some(model),
                latency_ms: None,
                detail,
            })
        }
    }
}

// ─── 恢复官方默认配置 ───────────────────────────────────────────────────────

/// 移除 Niko 写入的中转配置，让应用回到用官方账号登录的状态
#[tauri::command]
pub async fn restore_target_defaults(target_id: String) -> Result<Vec<String>, SafeCommandError> {
    let _guard = lock_and_recover_provider_transaction()?;
    if target_id == "codex" {
        let result = normalize_codex_session_storage_inner("openai".to_owned())?;
        return Ok(if result.status == "unchanged" {
            Vec::new()
        } else {
            vec!["codex".to_owned()]
        });
    }
    let summary = crate::targets::restore_defaults(&target_id)
        .map_err(|_| SafeCommandError::change_failed(true))?;
    crate::logx::append(
        "restore_target_defaults",
        &format!("{target_id} changed={:?}", summary.changed_keys),
    );
    Ok(summary.changed_keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_backups_restore_both_target_failure_orders() {
        for first in [0usize, 1usize] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("transaction");
            let paths = vec![
                temp.path().join("codex.json"),
                temp.path().join("claude.json"),
            ];
            fs::write(&paths[0], b"codex-old").unwrap();
            fs::write(&paths[1], b"claude-old").unwrap();
            let manifest = begin_provider_transaction_at(&root, &paths, Vec::new()).unwrap();

            fs::write(&paths[first], b"new-before-second-target-failed").unwrap();
            restore_provider_transaction_at(&root, &paths, &manifest).unwrap();

            assert_eq!(fs::read(&paths[0]).unwrap(), b"codex-old");
            assert_eq!(fs::read(&paths[1]).unwrap(), b"claude-old");
        }
    }

    #[test]
    fn durable_backup_restores_absence_for_single_installed_target() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("transaction");
        let path = temp.path().join("new-settings.json");
        let paths = vec![path.clone()];
        let manifest = begin_provider_transaction_at(&root, &paths, Vec::new()).unwrap();
        fs::write(&path, b"new").unwrap();
        restore_provider_transaction_at(&root, &paths, &manifest).unwrap();
        assert!(!path.exists());
    }
}
