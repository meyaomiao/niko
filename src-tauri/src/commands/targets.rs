use crate::codex_sessions::{
    atomic_replace_file as atomic_replace_codex_file, sync_parent as sync_codex_parent,
    CodexMigrationInput,
};
use crate::commands::codex_sessions::{
    normalize_codex_session_storage_inner, normalize_codex_session_storage_with_input,
    preflight_codex_session_storage, recover_codex_session_storage_since,
};
use crate::commands::safe_error::SafeCommandError;
use crate::targets::{all_targets, preflight_target_apply, transaction_paths, ApplyPlan};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
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

fn sync_parent(path: &Path) -> Result<(), SafeCommandError> {
    sync_codex_parent(path).map_err(|_| SafeCommandError::change_failed(false))
}

fn durable_replace(temporary: &Path, target: &Path) -> Result<(), SafeCommandError> {
    atomic_replace_codex_file(temporary, target, None)
        .map_err(|_| SafeCommandError::change_failed(false))?;
    sync_parent(target)
}

pub(crate) fn replace_from_backup(source: &Path, target: &Path) -> Result<(), SafeCommandError> {
    replace_from_backup_with_hook(source, target, |_| Ok(()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackupReplaceStep {
    Copy,
    Permissions,
    Sync,
    Replace,
}

struct TemporaryReplacement {
    path: PathBuf,
    file: Option<File>,
}

impl TemporaryReplacement {
    fn create_in(parent: &Path) -> io::Result<Self> {
        for _ in 0..32 {
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).map_err(|error| io::Error::other(error.to_string()))?;
            let suffix = random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let path = parent.join(format!(".niko-restore-{suffix}"));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "temporary replacement names are occupied",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn file(&self) -> &File {
        self.file.as_ref().expect("temporary file is open")
    }

    fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("temporary file is open")
    }

    fn close(&mut self) {
        self.file.take();
    }
}

impl Drop for TemporaryReplacement {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

fn replace_from_backup_with_hook(
    source: &Path,
    target: &Path,
    mut before: impl FnMut(BackupReplaceStep) -> Result<(), SafeCommandError>,
) -> Result<(), SafeCommandError> {
    let source_metadata =
        fs::symlink_metadata(source).map_err(|_| SafeCommandError::change_failed(false))?;
    if !source_metadata.file_type().is_file() {
        return Err(SafeCommandError::change_failed(false));
    }
    let parent = target
        .parent()
        .ok_or_else(|| SafeCommandError::change_failed(false))?;
    fs::create_dir_all(parent).map_err(|_| SafeCommandError::change_failed(false))?;
    let mut source_file = File::open(source).map_err(|_| SafeCommandError::change_failed(false))?;
    let mut temporary = TemporaryReplacement::create_in(parent)
        .map_err(|_| SafeCommandError::change_failed(false))?;

    before(BackupReplaceStep::Copy)?;
    io::copy(&mut source_file, temporary.file_mut())
        .map_err(|_| SafeCommandError::change_failed(false))?;
    before(BackupReplaceStep::Permissions)?;
    apply_backup_permissions(target, temporary.path(), source_metadata.permissions())?;
    before(BackupReplaceStep::Sync)?;
    temporary
        .file()
        .sync_all()
        .map_err(|_| SafeCommandError::change_failed(false))?;
    sync_parent(temporary.path())?;

    temporary.close();
    before(BackupReplaceStep::Replace)?;
    durable_replace(temporary.path(), target)
}

#[cfg(unix)]
fn apply_backup_permissions(
    target: &Path,
    temporary: &Path,
    source_permissions: fs::Permissions,
) -> Result<(), SafeCommandError> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = if target.file_name().and_then(|name| name.to_str()) == Some("auth.json") {
        fs::Permissions::from_mode(0o600)
    } else {
        source_permissions
    };
    fs::set_permissions(temporary, permissions).map_err(|_| SafeCommandError::change_failed(false))
}

#[cfg(not(unix))]
fn apply_backup_permissions(
    _target: &Path,
    temporary: &Path,
    source_permissions: fs::Permissions,
) -> Result<(), SafeCommandError> {
    fs::set_permissions(temporary, source_permissions)
        .map_err(|_| SafeCommandError::change_failed(false))
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
    durable_replace(&temporary, &path)
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
            replace_from_backup(&backup, path)?;
        } else if path.exists() {
            fs::remove_file(path).map_err(|_| SafeCommandError::change_failed(false))?;
            sync_parent(path)?;
        }
    }
    fs::remove_dir_all(root).map_err(|_| SafeCommandError::change_failed(false))?;
    sync_parent(root)
}

fn restore_provider_transaction(
    manifest: &ProviderTransactionManifest,
) -> Result<(), SafeCommandError> {
    let paths =
        transaction_paths("claude-desktop").map_err(|_| SafeCommandError::change_failed(false))?;
    restore_provider_transaction_at(&provider_transaction_root(), &paths, manifest)
}

fn finish_provider_transaction_at(root: &Path) -> Result<(), SafeCommandError> {
    if !root.exists() {
        return Ok(());
    }
    fs::remove_dir_all(root).map_err(|_| SafeCommandError::change_failed(false))?;
    sync_parent(root)
}

fn finish_provider_transaction() -> Result<(), SafeCommandError> {
    finish_provider_transaction_at(&provider_transaction_root())
}

fn recover_provider_transaction_at(
    root: &Path,
    paths: &[PathBuf],
    recover_codex: impl FnOnce(&[String]) -> Result<Option<bool>, SafeCommandError>,
) -> Result<(), SafeCommandError> {
    if !root.exists() {
        return Ok(());
    }
    let bytes = match fs::read(root.join("manifest.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return finish_provider_transaction_at(root);
        }
        Err(_) => return Err(SafeCommandError::change_failed(false)),
    };
    let manifest: ProviderTransactionManifest =
        serde_json::from_slice(&bytes).map_err(|_| SafeCommandError::change_failed(false))?;
    match manifest.phase {
        ProviderTransactionPhase::Committed => finish_provider_transaction_at(root),
        ProviderTransactionPhase::CodexStarted => {
            if recover_codex(&manifest.known_codex_transactions)? == Some(true) {
                finish_provider_transaction_at(root)
            } else {
                restore_provider_transaction_at(root, paths, &manifest)
            }
        }
        ProviderTransactionPhase::Prepared | ProviderTransactionPhase::ClaudeApplied => {
            restore_provider_transaction_at(root, paths, &manifest)
        }
    }
}

fn recover_provider_transaction() -> Result<(), SafeCommandError> {
    let paths =
        transaction_paths("claude-desktop").map_err(|_| SafeCommandError::change_failed(false))?;
    recover_provider_transaction_at(
        &provider_transaction_root(),
        &paths,
        recover_codex_session_storage_since,
    )
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
    if let Err(error) = persist_provider_manifest(&provider_transaction_root(), &manifest) {
        restore_provider_transaction(&manifest)?;
        return Err(error);
    }
    manifest.phase = ProviderTransactionPhase::CodexStarted;
    if let Err(error) = persist_provider_manifest(&provider_transaction_root(), &manifest) {
        restore_provider_transaction(&manifest)?;
        return Err(error);
    }

    let codex_outcome =
        match normalize_codex_session_storage_with_input("custom".to_owned(), Some(codex_input)) {
            Ok(outcome) => outcome,
            Err(error) => {
                match recover_codex_session_storage_since(&manifest.known_codex_transactions) {
                    Ok(Some(true)) => {
                        crate::commands::codex_sessions::CodexSessionMutationOutcome {
                            status: "applied",
                            message: "已完成检查，可以继续使用。",
                        }
                    }
                    Ok(_) => {
                        restore_provider_transaction(&manifest)?;
                        return Err(error);
                    }
                    Err(recovery_error) => return Err(recovery_error),
                }
            }
        };
    manifest.phase = ProviderTransactionPhase::Committed;
    let _ = persist_provider_manifest(&provider_transaction_root(), &manifest);
    let _ = finish_provider_transaction();

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

    fn temporary_replace_artifacts(parent: &Path) -> Vec<PathBuf> {
        fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".niko-restore-")
            })
            .map(|entry| entry.path())
            .collect()
    }

    #[test]
    fn coordinator_rejects_concurrent_writer() {
        let _held = PROVIDER_TRANSACTION_LOCK.try_lock().unwrap();
        let error = lock_and_recover_provider_transaction().unwrap_err();
        assert_eq!(error, SafeCommandError::busy());
    }

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

    #[test]
    fn prepared_transaction_recovers_before_following_write() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("transaction");
        let target = temp.path().join("settings.json");
        let snapshot = temp.path().join("snapshot.json");
        fs::write(&target, b"old").unwrap();
        fs::write(&snapshot, b"snapshot").unwrap();
        begin_provider_transaction_at(&root, std::slice::from_ref(&target), Vec::new()).unwrap();
        fs::write(&target, b"interrupted").unwrap();

        recover_provider_transaction_at(&root, std::slice::from_ref(&target), |_| Ok(None))
            .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"old");
        replace_from_backup(&snapshot, &target).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"snapshot");
    }

    #[test]
    fn backup_replace_handles_existing_and_absent_targets() {
        for target_exists in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source.json");
            let target = temp.path().join("settings.json");
            fs::write(&source, b"new").unwrap();
            if target_exists {
                fs::write(&target, b"old").unwrap();
            }

            replace_from_backup(&source, &target).unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"new");
            assert!(temporary_replace_artifacts(temp.path()).is_empty());
        }
    }

    #[test]
    fn fixed_temporary_file_preoccupation_is_not_reused() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.json");
        let target = temp.path().join("settings.json");
        let occupied = target.with_extension("niko-restore");
        fs::write(&source, b"new").unwrap();
        fs::write(&occupied, b"occupied").unwrap();

        replace_from_backup(&source, &target).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"new");
        assert_eq!(fs::read(occupied).unwrap(), b"occupied");
        assert!(temporary_replace_artifacts(temp.path()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_preoccupation_and_symlink_source_never_touch_sentinel() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.json");
        let target = temp.path().join("auth.json");
        let sentinel = temp.path().join("outside-sentinel");
        let occupied = target.with_extension("niko-restore");
        fs::write(&source, b"new").unwrap();
        fs::write(&sentinel, b"sentinel").unwrap();
        symlink(&sentinel, &occupied).unwrap();

        replace_from_backup(&source, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(fs::read(&sentinel).unwrap(), b"sentinel");

        let linked_source = temp.path().join("linked-source.json");
        symlink(&sentinel, &linked_source).unwrap();
        let rejected_target = temp.path().join("config.toml");
        assert!(replace_from_backup(&linked_source, &rejected_target).is_err());
        assert!(!rejected_target.exists());
        assert_eq!(fs::read(&sentinel).unwrap(), b"sentinel");
        assert!(temporary_replace_artifacts(temp.path()).is_empty());
    }

    #[test]
    fn every_backup_replace_failure_cleans_random_temporary_file() {
        for failed_step in [
            BackupReplaceStep::Copy,
            BackupReplaceStep::Permissions,
            BackupReplaceStep::Sync,
            BackupReplaceStep::Replace,
        ] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source.json");
            let target = temp.path().join("settings.json");
            fs::write(&source, b"new").unwrap();
            fs::write(&target, b"old").unwrap();

            let result = replace_from_backup_with_hook(&source, &target, |step| {
                if step == failed_step {
                    Err(SafeCommandError::change_failed(false))
                } else {
                    Ok(())
                }
            });
            assert!(result.is_err(), "{failed_step:?}");
            assert_eq!(fs::read(&target).unwrap(), b"old", "{failed_step:?}");
            assert!(
                temporary_replace_artifacts(temp.path()).is_empty(),
                "{failed_step:?}"
            );
        }
    }

    #[test]
    fn codex_started_uses_durable_inner_outcome_for_outer_recovery() {
        for committed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("transaction");
            let target = temp.path().join("settings.json");
            fs::write(&target, b"old").unwrap();
            let mut manifest = begin_provider_transaction_at(
                &root,
                std::slice::from_ref(&target),
                vec!["known".to_owned()],
            )
            .unwrap();
            fs::write(&target, b"new").unwrap();
            manifest.phase = ProviderTransactionPhase::CodexStarted;
            persist_provider_manifest(&root, &manifest).unwrap();

            recover_provider_transaction_at(&root, std::slice::from_ref(&target), |known| {
                assert_eq!(known, ["known".to_owned()]);
                Ok(Some(committed))
            })
            .unwrap();
            assert_eq!(
                fs::read(&target).unwrap(),
                if committed { b"new" } else { b"old" },
            );
            assert!(!root.exists());
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_coordinator_uses_platform_atomic_replace() {
        let _replace: fn(&Path, &Path) -> Result<(), SafeCommandError> = durable_replace;
        let _sync: fn(&Path) -> Result<(), SafeCommandError> = sync_parent;
    }
}
