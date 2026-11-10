use crate::active_groups::{
    clear_record_at, detect_active_group_at, detect_effective_selection_at, record_for, record_path,
    write_record_at,
    ActiveGroupStatus,
};
use crate::codex_sessions::{
    atomic_replace_file as atomic_replace_codex_file, sync_parent as sync_codex_parent,
};
use crate::commands::codex_sessions::recover_codex_session_storage_since;
use crate::commands::safe_error::SafeCommandError;
use crate::targets::{
    all_targets, preflight_target_apply, product_targets, transaction_paths_for_home, ApplyPlan,
    ApplySummary, Target,
};
use serde::{de::Deserializer, Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static PROVIDER_TRANSACTION_LOCK: Mutex<()> = Mutex::new(());
const LEGACY_PROVIDER_TRANSACTION_VERSION: u8 = 1;
const PROVIDER_TRANSACTION_VERSION: u8 = 2;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProviderTransactionPhase {
    Prepared,
    RecordsApplied,
    ClaudeApplied,
    CodexStarted,
    Committed,
    RollingBack,
    RolledBack,
}

#[derive(Debug, Serialize)]
pub struct ApplyResult {
    pub changed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProviderTransactionManifest {
    version: u8,
    phase: ProviderTransactionPhase,
    existed: Vec<bool>,
    known_codex_transactions: Vec<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_target_ids",
        skip_serializing_if = "Option::is_none"
    )]
    target_ids: Option<Vec<String>>,
}

fn deserialize_target_ids<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer).map(Some)
}

fn provider_transaction_root() -> PathBuf {
    crate::targets::app_data_dir()
        .join(".niko")
        .join("provider-transaction")
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
    drop(file);
    durable_replace(&temporary, &path)
}

#[cfg(test)]
fn begin_provider_transaction_at(
    root: &Path,
    paths: &[PathBuf],
    known_codex_transactions: Vec<String>,
) -> Result<ProviderTransactionManifest, SafeCommandError> {
    begin_provider_transaction_with_version_at(
        root, paths, known_codex_transactions, None, LEGACY_PROVIDER_TRANSACTION_VERSION,
    )
}

fn begin_provider_transaction_at_with_targets(
    root: &Path,
    paths: &[PathBuf],
    known_codex_transactions: Vec<String>,
    target_ids: Option<Vec<String>>,
) -> Result<ProviderTransactionManifest, SafeCommandError> {
    begin_provider_transaction_with_version_at(
        root, paths, known_codex_transactions, target_ids, PROVIDER_TRANSACTION_VERSION,
    )
}

fn begin_provider_transaction_with_version_at(
    root: &Path,
    paths: &[PathBuf],
    known_codex_transactions: Vec<String>,
    target_ids: Option<Vec<String>>,
    version: u8,
) -> Result<ProviderTransactionManifest, SafeCommandError> {
    let mut manifest = ProviderTransactionManifest {
        version,
        phase: ProviderTransactionPhase::Prepared,
        existed: vec![false; paths.len()],
        known_codex_transactions,
        target_ids,
    };
    validate_provider_transaction_manifest(&manifest, paths)?;
    fs::create_dir_all(root.parent().ok_or_else(|| SafeCommandError::change_failed(false))?)
        .map_err(|_| SafeCommandError::change_failed(false))?;
    let mut directory = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(root).map_err(|_| SafeCommandError::busy())?;
    let result = (|| {
        sync_parent(root)?;
        let mut existed = Vec::with_capacity(paths.len());
        for (index, path) in paths.iter().enumerate() {
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.file_type().is_file() => {
                    let backup = root.join(format!("{index}.backup"));
                    let mut options = OpenOptions::new();
                    options.read(true).write(true).create_new(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        options.mode(0o600);
                    }
                    let mut backup_file = options.open(&backup)
                        .map_err(|_| SafeCommandError::change_failed(false))?;
                    let mut source = File::open(path)
                        .map_err(|_| SafeCommandError::change_failed(false))?;
                    io::copy(&mut source, &mut backup_file)
                        .map_err(|_| SafeCommandError::change_failed(false))?;
                    apply_backup_permissions(path, &backup, metadata.permissions())?;
                    // Retain the writable handle: Windows FlushFileBuffers
                    // cannot sync a newly reopened read-only backup handle.
                    backup_file.sync_all()
                        .map_err(|_| SafeCommandError::change_failed(false))?;
                    existed.push(true);
                }
                Ok(_) => return Err(SafeCommandError::change_failed(false)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => existed.push(false),
                Err(_) => return Err(SafeCommandError::change_failed(false)),
            }
        }
        manifest.existed = existed;
        persist_provider_manifest(root, &manifest)?;
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
    validate_provider_transaction_manifest(manifest, paths)?;
    // Reject incomplete or substituted backups before replacing any target.
    for (index, existed) in manifest.existed.iter().enumerate() {
        if *existed {
            let metadata = fs::symlink_metadata(root.join(format!("{index}.backup")))
                .map_err(|_| SafeCommandError::change_failed(false))?;
            if !metadata.file_type().is_file() {
                return Err(SafeCommandError::change_failed(false));
            }
        }
    }
    for (index, (path, existed)) in paths.iter().zip(&manifest.existed).enumerate() {
        if *existed {
            let backup = root.join(format!("{index}.backup"));
            replace_from_backup(&backup, path)?;
        } else {
            match fs::symlink_metadata(path) {
                Ok(metadata)
                    if metadata.file_type().is_file() || metadata.file_type().is_symlink() =>
                {
                    fs::remove_file(path).map_err(|_| SafeCommandError::change_failed(false))?;
                    sync_parent(path)?;
                }
                Ok(_) => return Err(SafeCommandError::change_failed(false)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(SafeCommandError::change_failed(false)),
            }
        }
    }
    if manifest.version == PROVIDER_TRANSACTION_VERSION {
        let mut restored = manifest.clone();
        restored.phase = ProviderTransactionPhase::RolledBack;
        persist_provider_manifest(root, &restored)?;
    }
    finish_provider_transaction_at(root)
}

fn provider_transaction_paths_for_targets(
    target_ids: &[String],
) -> Result<Vec<PathBuf>, SafeCommandError> {
    provider_transaction_paths_for_layout(
        &crate::targets::user_home_dir(), target_ids, PROVIDER_TRANSACTION_VERSION,
    )
}

fn provider_transaction_paths_for_layout(
    home: &Path,
    target_ids: &[String],
    version: u8,
) -> Result<Vec<PathBuf>, SafeCommandError> {
    let mut paths = Vec::new();
    for target_id in target_ids {
        // v1 numbered backups deliberately omitted Codex auth/config. Keep that
        // exact index layout when reading old journals; only v2 includes them.
        if version == LEGACY_PROVIDER_TRANSACTION_VERSION && target_id == "codex" {
            continue;
        }
        for path in transaction_paths_for_home(home, target_id)
            .map_err(|_| SafeCommandError::invalid_request())?
        {
            // Claude desktop/CLI share settings.json. v2 takes one backup at
            // its first canonical position; v1 must retain its original layout.
            if version == LEGACY_PROVIDER_TRANSACTION_VERSION || !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    for target_id in target_ids {
        paths.push(record_path(home, target_id).map_err(|_| SafeCommandError::invalid_request())?);
    }
    Ok(paths)
}

fn validate_provider_transaction_shape(
    manifest: &ProviderTransactionManifest,
) -> Result<(), SafeCommandError> {
    if !matches!(manifest.version, LEGACY_PROVIDER_TRANSACTION_VERSION | PROVIDER_TRANSACTION_VERSION) {
        return Err(SafeCommandError::change_failed(false));
    }
    if manifest.version == LEGACY_PROVIDER_TRANSACTION_VERSION
        && matches!(manifest.phase, ProviderTransactionPhase::RollingBack | ProviderTransactionPhase::RolledBack)
    {
        return Err(SafeCommandError::change_failed(false));
    }
    if manifest.version == PROVIDER_TRANSACTION_VERSION
        && (manifest.target_ids.is_none() || !manifest.known_codex_transactions.is_empty())
    {
        return Err(SafeCommandError::change_failed(false));
    }

    let Some(target_ids) = manifest.target_ids.as_deref() else {
        // A v1 journal without target_ids is the pre-Issue #58 format. It can
        // still carry Codex inner-transaction ids because that format used one
        // Claude path list for both target flows.
        return match manifest.phase {
            ProviderTransactionPhase::Prepared
            | ProviderTransactionPhase::ClaudeApplied
            | ProviderTransactionPhase::CodexStarted
            | ProviderTransactionPhase::Committed => Ok(()),
            ProviderTransactionPhase::RecordsApplied
            | ProviderTransactionPhase::RollingBack
            | ProviderTransactionPhase::RolledBack => Err(SafeCommandError::change_failed(false)),
        };
    };
    if target_ids.is_empty() {
        return Err(SafeCommandError::change_failed(false));
    }

    // 目标必须唯一且按规范顺序出现；恢复逻辑依赖这个顺序重建路径清单
    const CANONICAL_ORDER: &[&str] = &[
        "codex",
        "claude-desktop",
        "claude-cli",
        "grok",
        "antigravity",
        "dsh",
        "zcode",
    ];
    let valid_target_order = !target_ids.is_empty()
        && target_ids.iter().all(|id| CANONICAL_ORDER.contains(&id.as_str()))
        && {
            let ranks: Vec<usize> = target_ids
                .iter()
                .map(|id| CANONICAL_ORDER.iter().position(|c| c == id).expect("checked"))
                .collect();
            ranks.windows(2).all(|pair| pair[0] < pair[1])
        };
    if !valid_target_order
        || (!target_ids.iter().any(|id| id == "codex")
            && !manifest.known_codex_transactions.is_empty())
    {
        return Err(SafeCommandError::change_failed(false));
    }
    Ok(())
}

fn validate_provider_transaction_path_structure(
    manifest: &ProviderTransactionManifest,
    paths: &[PathBuf],
) -> Result<(), SafeCommandError> {
    if paths.iter().any(|path| {
        path.as_os_str().is_empty()
            || path
                .components()
                .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    }) {
        return Err(SafeCommandError::change_failed(false));
    }

    let Some(target_ids) = manifest.target_ids.as_deref() else {
        // Old journals have no target ordering metadata. Their Claude-only path
        // list remains compatible with the already-established recovery rule.
        return Ok(());
    };

    let record_count = target_ids.len();
    let record_start = paths
        .len()
        .checked_sub(record_count)
        .ok_or_else(|| SafeCommandError::change_failed(false))?;
    for (index, target_id) in target_ids.iter().enumerate() {
        let expected = format!("{target_id}.json");
        if paths[record_start + index]
            .file_name()
            .and_then(|name| name.to_str())
            != Some(expected.as_str())
        {
            return Err(SafeCommandError::change_failed(false));
        }
    }

    Ok(())
}

fn validate_provider_transaction_manifest(
    manifest: &ProviderTransactionManifest,
    paths: &[PathBuf],
) -> Result<(), SafeCommandError> {
    validate_provider_transaction_shape(manifest)?;
    if paths.is_empty()
        || paths.len() != manifest.existed.len()
        || paths.iter().collect::<HashSet<_>>().len() != paths.len()
    {
        return Err(SafeCommandError::change_failed(false));
    }
    validate_provider_transaction_path_structure(manifest, paths)?;

    // Legacy no-target journals use the old Claude-only recovery rule. For
    // target-aware journals validate against their own version's index layout.
    // This root is used only to derive names; validation never consults HOME.
    if let Some(target_ids) = manifest.target_ids.as_deref() {
        let expected = provider_transaction_paths_for_layout(
            Path::new("__niko_layout__"), target_ids, manifest.version,
        )?;
        if paths.len() != expected.len()
            || paths.iter().zip(&expected)
                .any(|(actual, expected)| actual.file_name() != expected.file_name())
        {
            return Err(SafeCommandError::change_failed(false));
        }
    }
    Ok(())
}

fn manifest_transaction_paths(
    manifest: &ProviderTransactionManifest,
) -> Result<Vec<PathBuf>, SafeCommandError> {
    manifest_transaction_paths_at(&crate::targets::user_home_dir(), manifest)
}

fn manifest_transaction_paths_at(
    home: &Path,
    manifest: &ProviderTransactionManifest,
) -> Result<Vec<PathBuf>, SafeCommandError> {
    validate_provider_transaction_shape(manifest)?;
    match manifest.target_ids.as_deref() {
        None => transaction_paths_for_home(home, "claude-desktop")
            .map_err(|_| SafeCommandError::change_failed(false)),
        Some(target_ids) => provider_transaction_paths_for_layout(home, target_ids, manifest.version),
    }
}

fn finish_provider_transaction_at(root: &Path) -> Result<(), SafeCommandError> {
    if !root.exists() {
        return Ok(());
    }
    fs::remove_dir_all(root).map_err(|_| SafeCommandError::change_failed(false))?;
    sync_parent(root)
}

fn recover_provider_transaction_at(
    root: &Path,
    paths: &[PathBuf],
    recover_codex: impl FnOnce(&[String]) -> Result<Option<bool>, SafeCommandError>,
) -> Result<(), SafeCommandError> {
    if !root.exists() {
        return Ok(());
    }
    let bytes =
        fs::read(root.join("manifest.json")).map_err(|_| SafeCommandError::change_failed(false))?;
    let manifest: ProviderTransactionManifest =
        serde_json::from_slice(&bytes).map_err(|_| SafeCommandError::change_failed(false))?;
    validate_provider_transaction_manifest(&manifest, paths)?;
    match manifest.phase {
        ProviderTransactionPhase::Committed | ProviderTransactionPhase::RolledBack => {
            finish_provider_transaction_at(root)
        }
        ProviderTransactionPhase::CodexStarted if manifest.known_codex_transactions.is_empty() => {
            // 配置专用事务不再创建 Codex 会话迁移子事务；恢复旧记录时，
            // 只回滚外层状态，不能重新触发全量会话恢复。
            restore_provider_transaction_at(root, paths, &manifest)
        }
        ProviderTransactionPhase::CodexStarted => {
            if recover_codex(&manifest.known_codex_transactions)? == Some(true) {
                finish_provider_transaction_at(root)
            } else {
                restore_provider_transaction_at(root, paths, &manifest)
            }
        }
        ProviderTransactionPhase::Prepared
        | ProviderTransactionPhase::RecordsApplied
        | ProviderTransactionPhase::ClaudeApplied
        | ProviderTransactionPhase::RollingBack => {
            restore_provider_transaction_at(root, paths, &manifest)
        }
    }
}

fn recover_provider_transaction_without_sessions() -> Result<(), SafeCommandError> {
    let root = provider_transaction_root();
    if !root.exists() {
        return Ok(());
    }
    let bytes =
        fs::read(root.join("manifest.json")).map_err(|_| SafeCommandError::change_failed(false))?;
    let manifest: ProviderTransactionManifest =
        serde_json::from_slice(&bytes).map_err(|_| SafeCommandError::change_failed(false))?;
    validate_provider_transaction_shape(&manifest)?;
    let paths = manifest_transaction_paths(&manifest)?;
    validate_provider_transaction_manifest(&manifest, &paths)?;
    recover_provider_transaction_at(&root, &paths, |_known| Err(SafeCommandError::busy()))
}

fn recover_provider_transaction_with_sessions() -> Result<(), SafeCommandError> {
    let root = provider_transaction_root();
    if !root.exists() {
        return Ok(());
    }
    let bytes =
        fs::read(root.join("manifest.json")).map_err(|_| SafeCommandError::change_failed(false))?;
    let manifest: ProviderTransactionManifest =
        serde_json::from_slice(&bytes).map_err(|_| SafeCommandError::change_failed(false))?;
    validate_provider_transaction_shape(&manifest)?;
    let paths = manifest_transaction_paths(&manifest)?;
    validate_provider_transaction_manifest(&manifest, &paths)?;
    recover_provider_transaction_at(&root, &paths, recover_codex_session_storage_since)
}

pub(crate) fn lock_and_recover_provider_transaction(
) -> Result<MutexGuard<'static, ()>, SafeCommandError> {
    let guard = PROVIDER_TRANSACTION_LOCK
        .try_lock()
        .map_err(|_| SafeCommandError::busy())?;
    recover_provider_transaction_without_sessions()?;
    Ok(guard)
}

pub(crate) fn lock_and_recover_provider_transaction_for_sessions(
) -> Result<MutexGuard<'static, ()>, SafeCommandError> {
    let guard = PROVIDER_TRANSACTION_LOCK
        .try_lock()
        .map_err(|_| SafeCommandError::busy())?;
    recover_provider_transaction_with_sessions()?;
    Ok(guard)
}

/// 只读命令共享的事务锁：用阻塞式加锁而不是 try_lock。
/// 首页启动会并发发起 list_targets / detect_* 等多个命令，try_lock 会让
/// 抢不到锁的那个直接失败（界面显示「未能读取本机应用状态」）。
/// 临界区很短，阻塞等待远好于让用户看到假失败。
pub(crate) fn lock_provider_transaction_readonly(
) -> Result<MutexGuard<'static, ()>, SafeCommandError> {
    PROVIDER_TRANSACTION_LOCK
        .lock()
        .map_err(|_| SafeCommandError::busy())
}

#[derive(Debug, Serialize)]
pub struct TargetInfo {
    pub id: String,
    pub name: String,
    pub installed: bool,
    /// 本机已安装时提取到的真实应用图标（PNG data URI）
    pub icon: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EffectiveSelectionStatus {
    pub version: u16,
    pub target_id: String,
    pub status: crate::active_groups::ActiveGroupState,
    pub group: Option<String>,
    pub model: Option<String>,
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

fn records_for_plan(
    target_ids: &[String],
    plan: &ApplyPlan,
) -> Result<Vec<crate::active_groups::ActiveGroupRecord>, SafeCommandError> {
    let group = plan
        .model_group
        .as_deref()
        .filter(|group| !group.is_empty())
        .ok_or_else(SafeCommandError::invalid_request)?;
    target_ids
        .iter()
        .map(|target_id| {
            let digest = crate::targets::expected_config_digest(target_id, plan)
                .map_err(|_| SafeCommandError::invalid_request())?;
            record_for(target_id, group, digest).map_err(|_| SafeCommandError::change_failed(false))
        })
        .collect()
}

fn write_active_records(
    records: &[crate::active_groups::ActiveGroupRecord],
) -> Result<(), SafeCommandError> {
    let home = crate::targets::user_home_dir();
    for record in records {
        write_record_at(&home, record).map_err(|_| SafeCommandError::change_failed(false))?;
    }
    Ok(())
}

fn clear_active_records(target_ids: &[String]) -> Result<(), SafeCommandError> {
    let home = crate::targets::user_home_dir();
    for target_id in target_ids {
        clear_record_at(&home, target_id).map_err(|_| SafeCommandError::change_failed(false))?;
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ProviderTransactionStep {
    BeforeManifest(ProviderTransactionPhase),
    AfterManifest(ProviderTransactionPhase),
    BeforeTarget(String),
    AfterTarget(String),
    BeforeTargetSync,
}

fn rollback_provider_transaction_at(
    root: &Path,
    paths: &[PathBuf],
    manifest: &ProviderTransactionManifest,
    error: SafeCommandError,
) -> SafeCommandError {
    let mut rolling_back = manifest.clone();
    rolling_back.phase = ProviderTransactionPhase::RollingBack;
    // A failed commit may already have renamed a Committed manifest before a
    // directory-sync error. Persist rollback intent before changing any target;
    // otherwise a crash during rollback could be mistaken for committed work.
    if persist_provider_manifest(root, &rolling_back).is_err() {
        return SafeCommandError::change_failed(false);
    }
    if restore_provider_transaction_at(root, paths, &rolling_back).is_ok() {
        error
    } else {
        SafeCommandError::change_failed(false)
    }
}

fn sync_provider_targets(paths: &[PathBuf]) -> Result<(), SafeCommandError> {
    for path in paths {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                #[cfg(unix)]
                if path.file_name().and_then(|name| name.to_str()) == Some("auth.json") {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                        .map_err(|_| SafeCommandError::change_failed(false))?;
                }
                #[cfg(windows)]
                let file = OpenOptions::new().read(true).write(true).open(path);
                #[cfg(not(windows))]
                let file = File::open(path);
                file.and_then(|file| file.sync_all())
                    .map_err(|_| SafeCommandError::change_failed(false))?;
                sync_parent(path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Missing targets are valid when restoring defaults. Sync an
                // existing parent to persist any deletion performed by apply.
                if path.parent().is_some_and(|parent| parent.is_dir()) {
                    sync_parent(path)?;
                }
            }
            _ => return Err(SafeCommandError::change_failed(false)),
        }
    }
    Ok(())
}

fn advance_provider_phase(
    root: &Path,
    manifest: &mut ProviderTransactionManifest,
    phase: ProviderTransactionPhase,
    checkpoint: &mut impl FnMut(ProviderTransactionStep) -> Result<(), SafeCommandError>,
) -> Result<(), SafeCommandError> {
    manifest.phase = phase;
    checkpoint(ProviderTransactionStep::BeforeManifest(phase))?;
    persist_provider_manifest(root, manifest)?;
    checkpoint(ProviderTransactionStep::AfterManifest(phase))
}

/// All configuration mutations share this coordinator. Tests pass explicit
/// fixture paths/targets and failures; no installation probing or IPC occurs.
fn run_provider_transaction_at<T>(
    root: &Path,
    paths: &[PathBuf],
    target_ids: &[String],
    mutate_records: impl FnOnce() -> Result<(), SafeCommandError>,
    mut mutate_target: impl FnMut(&str) -> Result<T, SafeCommandError>,
    mut checkpoint: impl FnMut(ProviderTransactionStep) -> Result<(), SafeCommandError>,
) -> Result<Vec<T>, SafeCommandError> {
    checkpoint(ProviderTransactionStep::BeforeManifest(ProviderTransactionPhase::Prepared))?;
    let mut manifest = begin_provider_transaction_at_with_targets(
        root, paths, Vec::new(), Some(target_ids.to_vec()),
    )?;
    let result = (|| {
        checkpoint(ProviderTransactionStep::AfterManifest(ProviderTransactionPhase::Prepared))?;
        mutate_records()?;
        advance_provider_phase(root, &mut manifest, ProviderTransactionPhase::RecordsApplied, &mut checkpoint)?;
        let mut results = Vec::new();
        // Preserve the existing batch order: other targets first, Codex last.
        for target_id in target_ids.iter().filter(|id| id.as_str() != "codex")
            .chain(target_ids.iter().filter(|id| id.as_str() == "codex"))
        {
            if target_id == "codex" {
                advance_provider_phase(root, &mut manifest, ProviderTransactionPhase::CodexStarted, &mut checkpoint)?;
            }
            checkpoint(ProviderTransactionStep::BeforeTarget(target_id.clone()))?;
            results.push(mutate_target(target_id)?);
            checkpoint(ProviderTransactionStep::AfterTarget(target_id.clone()))?;
            if target_id != "codex" {
                advance_provider_phase(root, &mut manifest, ProviderTransactionPhase::ClaudeApplied, &mut checkpoint)?;
            }
        }
        checkpoint(ProviderTransactionStep::BeforeTargetSync)?;
        sync_provider_targets(paths)?;
        Ok(results)
    })();
    let results = match result {
        Ok(results) => results,
        Err(error) => return Err(rollback_provider_transaction_at(root, paths, &manifest, error)),
    };
    manifest.phase = ProviderTransactionPhase::Committed;
    if let Err(error) = checkpoint(ProviderTransactionStep::BeforeManifest(ProviderTransactionPhase::Committed))
        .and_then(|_| persist_provider_manifest(root, &manifest))
    {
        return Err(rollback_provider_transaction_at(root, paths, &manifest, error));
    }
    // The commit is durable. Cleanup failure must not report a failed apply or
    // roll back committed data; the next recovery can finish garbage cleanup.
    if checkpoint(ProviderTransactionStep::AfterManifest(ProviderTransactionPhase::Committed)).is_ok() {
        let _ = finish_provider_transaction_at(root);
    }
    Ok(results)
}

fn apply_explicit_targets(
    targets: &[&dyn Target],
    plan: &ApplyPlan,
    checkpoint: impl FnMut(ProviderTransactionStep) -> Result<(), SafeCommandError>,
) -> Result<Vec<ApplySummary>, SafeCommandError> {
    let target_ids: Vec<String> = targets.iter().map(|target| target.id().to_owned()).collect();
    for target_id in &target_ids {
        preflight_target_apply(target_id).map_err(|_| SafeCommandError::change_failed(false))?;
    }
    let records = records_for_plan(&target_ids, plan)?;
    let paths = provider_transaction_paths_for_targets(&target_ids)?;
    run_provider_transaction_at(
        &provider_transaction_root(), &paths, &target_ids,
        || write_active_records(&records),
        |target_id| {
            targets.iter().find(|target| target.id() == target_id)
                .ok_or_else(SafeCommandError::invalid_request)?
                .apply(plan).map_err(|_| SafeCommandError::change_failed(true))
        },
        checkpoint,
    )
}

fn restore_explicit_target(
    target_id: &str,
    checkpoint: impl FnMut(ProviderTransactionStep) -> Result<(), SafeCommandError>,
) -> Result<ApplySummary, SafeCommandError> {
    preflight_target_apply(target_id).map_err(|_| SafeCommandError::change_failed(false))?;
    let target_ids = vec![target_id.to_owned()];
    let paths = provider_transaction_paths_for_targets(&target_ids)?;
    let mut summaries = run_provider_transaction_at(
        &provider_transaction_root(), &paths, &target_ids,
        || clear_active_records(&target_ids),
        |id| crate::targets::restore_defaults(id).map_err(|_| SafeCommandError::change_failed(true)),
        checkpoint,
    )?;
    summaries.pop().ok_or_else(SafeCommandError::invalid_request)
}

#[tauri::command]
pub async fn list_targets() -> Result<Vec<TargetInfo>, SafeCommandError> {
    let _guard = lock_provider_transaction_readonly()?;
    let infos = product_targets()
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
pub async fn detect_active_groups(
    available_groups: Option<Vec<String>>,
) -> Result<Vec<ActiveGroupStatus>, SafeCommandError> {
    let _guard = lock_provider_transaction_readonly()?;
    let home = crate::targets::user_home_dir();
    let root = provider_transaction_root();
    if fs::symlink_metadata(&root).is_ok() {
        return Ok(all_targets()
            .iter()
            .map(|target| ActiveGroupStatus {
                version: crate::active_groups::ACTIVE_GROUP_STATUS_VERSION,
                target_id: target.id().to_owned(),
                status: crate::active_groups::ActiveGroupState::Unknown,
                group: None,
            })
            .collect());
    }
    let available = available_groups.as_deref();
    Ok(all_targets()
        .iter()
        .map(|target| detect_active_group_at(&home, target.id(), available))
        .collect())
}

#[tauri::command]
pub async fn detect_effective_selections(
    available_groups: Option<Vec<String>>,
) -> Result<Vec<EffectiveSelectionStatus>, SafeCommandError> {
    let _guard = lock_provider_transaction_readonly()?;
    let home = crate::targets::user_home_dir();
    let root = provider_transaction_root();
    if fs::symlink_metadata(&root).is_ok() {
        return Ok(all_targets()
            .iter()
            .map(|target| EffectiveSelectionStatus {
                version: crate::active_groups::ACTIVE_GROUP_STATUS_VERSION,
                target_id: target.id().to_owned(),
                status: crate::active_groups::ActiveGroupState::Unknown,
                group: None,
                model: None,
            })
            .collect());
    }
    let available = available_groups.as_deref();
    Ok(all_targets()
        .iter()
        .map(|target| {
            let selection = detect_effective_selection_at(&home, target.id(), available);
            let confirmed = selection.status == crate::active_groups::ActiveGroupState::Active
                && selection.group.is_some()
                && selection.model.is_some();
            EffectiveSelectionStatus {
                version: crate::active_groups::ACTIVE_GROUP_STATUS_VERSION,
                target_id: target.id().to_owned(),
                status: if confirmed {
                    selection.status
                } else if selection.status == crate::active_groups::ActiveGroupState::Active {
                    crate::active_groups::ActiveGroupState::Unknown
                } else {
                    selection.status
                },
                group: confirmed.then_some(selection.group).flatten(),
                model: confirmed.then_some(selection.model).flatten(),
            }
        })
        .collect())
}

#[tauri::command]
pub async fn apply_target(req: ApplyRequest) -> Result<ApplyResult, SafeCommandError> {
    let _guard = lock_and_recover_provider_transaction()?;
    let plan = ApplyPlan {
        base_url: req.base_url.clone(),
        api_key: req.api_key.clone(),
        model_group: req.model_group.clone(),
        model: req.model.clone(),
        codex_mixed: req.codex_mixed,
    };
    let targets = all_targets();
    let target = targets
        .iter()
        .find(|t| t.id() == req.target_id)
        .ok_or_else(SafeCommandError::invalid_request)?;
    let mut summaries = apply_explicit_targets(&[target.as_ref()], &plan, |_| Ok(()))?;
    let summary = summaries.pop().ok_or_else(SafeCommandError::invalid_request)?;
    let changed = ApplyResult {
        changed: summary.changed_keys,
        warning: (req.target_id == "codex").then(|| {
            "模型配置已生效；历史会话同步请在会话管理页面单独执行。".to_owned()
        }),
    };
    crate::logx::append(
        "apply_target",
        &format!("{} changed={:?}", req.target_id, changed.changed),
    );
    Ok(changed)
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
    let targets = product_targets();
    let installed: Vec<_> = targets
        .iter()
        .filter(|target| target.is_installed())
        .collect();
    if installed.is_empty() {
        return Ok(Vec::new());
    }
    let explicit: Vec<&dyn Target> = installed.iter().map(|target| target.as_ref()).collect();
    let mut summaries = apply_explicit_targets(&explicit, &plan, |_| Ok(()))?;
    // Keep the public result order aligned with product_targets, even though
    // transactional execution deliberately writes Codex last.
    summaries.sort_by_key(|summary| installed.iter()
        .position(|target| target.id() == summary.target_id).unwrap_or(usize::MAX));
    Ok(summaries.into_iter().map(|summary| {
        let warning = (summary.target_id == "codex").then(|| {
            "模型配置已生效；历史会话同步请在会话管理页面单独执行。".to_owned()
        });
        serde_json::json!({
            "id": summary.target_id,
            "ok": true,
            "changed": summary.changed_keys,
            "warning": warning,
        })
    }).collect())
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
    pub model: Option<String>,
    pub latency_ms: Option<u64>,
    pub detail: String,
}

fn safe_effective_config_error(error: &str) -> String {
    if error.contains("未找到") {
        return "没有找到目标应用的设置，请先安装并打开应用，再接入到应用。".to_owned();
    }
    if error.contains("尚未接入")
        || error.contains("缺少")
        || error.contains("没有")
        || error.contains("没有默认模型")
    {
        return "应用设置还没有生效，请先接入到应用后再检查。".to_owned();
    }
    "无法读取应用设置，请重新接入后再检查。".to_owned()
}

/// 用目标应用磁盘上真实生效的配置发一条最小请求，确认配置真的能用。
#[tauri::command]
pub async fn test_connectivity(target_id: String) -> Result<ConnectivityResult, String> {
    let cfg = crate::targets::effective_config(&target_id)
        .map_err(|error| safe_effective_config_error(&error))?;
    let model = cfg
        .model
        .clone()
        .ok_or("应用设置里还没有选好模型，请先接入到应用后再检查。")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|_| "模型服务暂时不可用，请稍后重试。".to_owned())?;

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
        "openai-chat" => client
            .post(&cfg.endpoint)
            .header("Authorization", format!("Bearer {}", cfg.api_key))
            .json(&serde_json::json!({
                "model": model,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1
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
                format!("可以正常使用，{model} 可用（{:.1}s）", ms as f64 / 1000.0)
            } else {
                let _ = r.text().await;
                let hint = match code {
                    401 | 403 => "连接密钥无效或已过期，请重新接入后再试",
                    404 => "服务地址或模型不可用，请重新接入后再试",
                    429 => "服务暂时繁忙，请稍后重试",
                    c if c >= 500 => "模型服务暂时不可用，请稍后重试",
                    _ => "检查没有完成，请重新接入后再试",
                };
                hint.to_owned()
            };
            crate::logx::append(
                "test_connectivity",
                &format!("{target_id} HTTP {code} {ms}ms"),
            );
            Ok(ConnectivityResult {
                target_id,
                ok,
                model: Some(model),
                latency_ms: Some(ms),
                detail,
            })
        }
        Err(e) => {
            crate::logx::append("test_connectivity", &format!("{target_id} request_failed"));
            let detail = if e.is_timeout() {
                "网络连接超时，请检查网络后重试。".to_owned()
            } else if e.is_connect() {
                "网络连接失败，请检查网络后重试。".to_owned()
            } else {
                "检查没有完成，请重新接入后再试。".to_owned()
            };
            Ok(ConnectivityResult {
                target_id,
                ok: false,
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
    let targets = all_targets();
    let _target = targets
        .iter()
        .find(|target| target.id() == target_id)
        .ok_or_else(SafeCommandError::invalid_request)?;
    let summary = restore_explicit_target(&target_id, |_| Ok(()))?;
    crate::logx::append(
        "restore_target_defaults",
        &format!("{target_id} changed={:?}", summary.changed_keys),
    );
    Ok(summary.changed_keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_os = "windows"))]
    fn run_ready<F: std::future::Future>(future: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut context = std::task::Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        match std::future::Future::poll(future.as_mut(), &mut context) {
            std::task::Poll::Ready(output) => output,
            std::task::Poll::Pending => panic!("startup probe unexpectedly suspended"),
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn directory_entries(path: &Path) -> Vec<String> {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    #[cfg(not(target_os = "windows"))]
    fn assert_startup_probes_preserve_fixture(
        config: &Path,
        record: &Path,
        transaction_root: &Path,
        manifest: Option<&Path>,
        backup: Option<&Path>,
    ) {
        let config_before = fs::read(config).unwrap();
        let record_before = fs::read(record).unwrap();
        let transaction_entries_before = transaction_root
            .exists()
            .then(|| directory_entries(transaction_root));
        let manifest_before = manifest.map(|path| fs::read(path).unwrap());
        let backup_before = backup.map(|path| fs::read(path).unwrap());

        // list_targets probes real installed applications/icons; it is not a
        // fixture-only command and must never be invoked by these tests.
        assert!(run_ready(detect_active_groups(Some(vec!["CC Switch".to_owned()]))).is_ok());
        assert!(run_ready(detect_effective_selections(Some(vec!["CC Switch".to_owned()]))).is_ok());

        assert_eq!(fs::read(config).unwrap(), config_before);
        assert_eq!(fs::read(record).unwrap(), record_before);
        assert_eq!(
            transaction_root.exists().then(|| directory_entries(transaction_root)),
            transaction_entries_before,
        );
        if let Some(path) = manifest {
            assert_eq!(fs::read(path).unwrap(), manifest_before.unwrap());
        }
        if let Some(path) = backup {
            assert_eq!(fs::read(path).unwrap(), backup_before.unwrap());
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn startup_probes_are_read_only_with_or_without_pending_provider_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(temp.path());
        let codex = temp.path().join(".codex");
        let config = codex.join("config.toml");
        let record = record_path(temp.path(), "codex").unwrap();
        let transaction_root = provider_transaction_root();
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(
            &config,
            b"model_provider = \"cc-switch\"\nmodel = \"gpt-5\"\n\n[model_providers.cc-switch]\nname = \"CC Switch\"\nbase_url = \"https://cc-switch.example/v1\"\n",
        )
        .unwrap();
        fs::create_dir_all(record.parent().unwrap()).unwrap();
        fs::write(&record, b"active-record-before").unwrap();

        assert_startup_probes_preserve_fixture(&config, &record, &transaction_root, None, None);

        let paths = vec![record.clone()];
        let mut manifest = begin_provider_transaction_with_version_at(
            &transaction_root,
            &paths,
            Vec::new(),
            Some(vec!["codex".to_owned()]),
            LEGACY_PROVIDER_TRANSACTION_VERSION,
        )
        .unwrap();
        fs::write(&record, b"active-record-during-interrupted-write").unwrap();
        manifest.phase = ProviderTransactionPhase::RecordsApplied;
        persist_provider_manifest(&transaction_root, &manifest).unwrap();
        let manifest_path = transaction_root.join("manifest.json");
        let backup_path = transaction_root.join("0.backup");

        assert_startup_probes_preserve_fixture(
            &config,
            &record,
            &transaction_root,
            Some(&manifest_path),
            Some(&backup_path),
        );
    }

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
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
        assert!(!stored.as_object().unwrap().contains_key("target_ids"));
        fs::write(&target, b"interrupted").unwrap();

        recover_provider_transaction_at(&root, std::slice::from_ref(&target), |_| Ok(None))
            .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"old");
        replace_from_backup(&snapshot, &target).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"snapshot");
    }

    #[test]
    fn records_applied_transaction_restores_record_and_target() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("transaction");
        let record = temp.path().join("codex.json");
        let paths = vec![record.clone()];
        fs::write(&record, b"record-old").unwrap();
        let mut manifest = begin_provider_transaction_with_version_at(
            &root,
            &paths,
            Vec::new(),
            Some(vec!["codex".to_owned()]),
            LEGACY_PROVIDER_TRANSACTION_VERSION,
        )
        .unwrap();

        fs::write(&record, b"record-new").unwrap();
        manifest.phase = ProviderTransactionPhase::RecordsApplied;
        persist_provider_manifest(&root, &manifest).unwrap();

        recover_provider_transaction_at(&root, &paths, |_| Ok(None)).unwrap();
        assert_eq!(fs::read(&record).unwrap(), b"record-old");
        assert!(!root.exists());
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

    #[test]
    fn invalid_terminal_journals_are_preserved_before_dispatch() {
        let cases = [
            (
                "unknown-version",
                serde_json::json!({
                    "version": 99,
                    "phase": "committed",
                    "existed": [false],
                    "known_codex_transactions": [],
                    "target_ids": ["codex"]
                }),
                vec!["codex.json"],
            ),
            (
                "unknown-field",
                serde_json::json!({
                    "version": 1,
                    "phase": "committed",
                    "existed": [false],
                    "known_codex_transactions": [],
                    "target_ids": ["codex"],
                    "unexpected": true
                }),
                vec!["codex.json"],
            ),
            (
                "invalid-schema",
                serde_json::json!({
                    "version": 1,
                    "phase": "committed",
                    "existed": "not-an-array",
                    "known_codex_transactions": [],
                    "target_ids": ["codex"]
                }),
                vec!["codex.json"],
            ),
            (
                "duplicate-target",
                serde_json::json!({
                    "version": 1,
                    "phase": "committed",
                    "existed": [false, false],
                    "known_codex_transactions": [],
                    "target_ids": ["codex", "codex"]
                }),
                vec!["codex.json", "codex-copy.json"],
            ),
            (
                "wrong-target-order",
                serde_json::json!({
                    "version": 1,
                    "phase": "committed",
                    "existed": [false, false],
                    "known_codex_transactions": [],
                    "target_ids": ["claude-desktop", "codex"]
                }),
                vec!["settings.json", "codex.json"],
            ),
            (
                "path-count",
                serde_json::json!({
                    "version": 1,
                    "phase": "committed",
                    "existed": [false, false],
                    "known_codex_transactions": [],
                    "target_ids": ["codex"]
                }),
                vec!["codex.json"],
            ),
            (
                "path-traversal",
                serde_json::json!({
                    "version": 1,
                    "phase": "committed",
                    "existed": [false],
                    "known_codex_transactions": [],
                    "target_ids": ["codex"]
                }),
                vec!["../outside.json"],
            ),
            (
                "explicit-empty-targets",
                serde_json::json!({
                    "version": 1,
                    "phase": "committed",
                    "existed": [false],
                    "known_codex_transactions": [],
                    "target_ids": []
                }),
                vec!["codex.json"],
            ),
            (
                "null-targets",
                serde_json::json!({
                    "version": 1,
                    "phase": "committed",
                    "existed": [false],
                    "known_codex_transactions": [],
                    "target_ids": null
                }),
                vec!["codex.json"],
            ),
        ];

        for phase in [
            "prepared",
            "records_applied",
            "claude_applied",
            "committed",
            "codex_started",
        ] {
            for (name, mut manifest, path_names) in cases.iter().cloned() {
                manifest["phase"] = serde_json::Value::String(phase.to_owned());
                let temp = tempfile::tempdir().unwrap();
                let root = temp.path().join("transaction");
                fs::create_dir(&root).unwrap();
                let manifest_path = root.join("manifest.json");
                fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
                let paths = path_names
                    .into_iter()
                    .map(|name| temp.path().join(name))
                    .collect::<Vec<_>>();
                let mut recover_called = false;

                let result = recover_provider_transaction_at(&root, &paths, |_| {
                    recover_called = true;
                    Ok(Some(true))
                });
                let error = match result {
                    Err(error) => error,
                    Ok(()) => panic!("{phase}/{name} unexpectedly accepted"),
                };

                assert_eq!(
                    error,
                    SafeCommandError::change_failed(false),
                    "{phase}/{name}"
                );
                assert!(root.is_dir(), "{phase}/{name}");
                assert!(manifest_path.is_file(), "{phase}/{name}");
                assert!(!recover_called, "{phase}/{name}");
                let error_json = serde_json::to_string(&error).unwrap();
                assert!(!error_json.contains(&root.to_string_lossy().to_string()));
                assert!(!error_json.contains("outside"));
            }
        }
    }

    #[test]
    fn legacy_manifest_without_target_ids_is_compatible_in_recovery_and_commit() {
        for phase in [
            ProviderTransactionPhase::Prepared,
            ProviderTransactionPhase::ClaudeApplied,
            ProviderTransactionPhase::CodexStarted,
            ProviderTransactionPhase::Committed,
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("transaction");
            let target = temp.path().join("settings.json");
            fs::write(&target, b"old").unwrap();
            let mut manifest =
                begin_provider_transaction_at(&root, std::slice::from_ref(&target), Vec::new())
                    .unwrap();
            let stored: serde_json::Value =
                serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
            assert!(!stored.as_object().unwrap().contains_key("target_ids"));

            fs::write(&target, b"new").unwrap();
            manifest.phase = phase;
            persist_provider_manifest(&root, &manifest).unwrap();
            recover_provider_transaction_at(&root, std::slice::from_ref(&target), |_| Ok(None))
                .unwrap();

            let expected = if matches!(phase, ProviderTransactionPhase::Committed) {
                b"new"
            } else {
                b"old"
            };
            assert_eq!(fs::read(&target).unwrap(), expected);
            assert!(!root.exists());
        }
    }

    #[test]
    fn legacy_records_applied_manifest_is_preserved_before_recovery_dispatch() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("transaction");
        let target = temp.path().join("settings.json");
        fs::write(&target, b"old").unwrap();
        let mut manifest =
            begin_provider_transaction_at(&root, std::slice::from_ref(&target), Vec::new())
                .unwrap();
        fs::write(&target, b"new").unwrap();
        manifest.phase = ProviderTransactionPhase::RecordsApplied;
        persist_provider_manifest(&root, &manifest).unwrap();

        let mut recover_called = false;
        let error = recover_provider_transaction_at(&root, std::slice::from_ref(&target), |_| {
            recover_called = true;
            Ok(Some(true))
        })
        .unwrap_err();

        assert_eq!(error, SafeCommandError::change_failed(false));
        assert!(!recover_called);
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(root.is_dir());
        assert!(root.join("manifest.json").is_file());
        let error_json = serde_json::to_string(&error).unwrap();
        assert!(!error_json.contains(&root.to_string_lossy().to_string()));
    }

    #[test]
    fn legacy_unknown_phase_is_preserved_before_recovery_dispatch() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("transaction");
        fs::create_dir(&root).unwrap();
        let manifest_path = root.join("manifest.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "phase": "future_phase",
                "existed": [false],
                "known_codex_transactions": []
            }))
            .unwrap(),
        )
        .unwrap();
        let target = temp.path().join("settings.json");
        let mut recover_called = false;

        let error = recover_provider_transaction_at(&root, &[target], |_| {
            recover_called = true;
            Ok(Some(true))
        })
        .unwrap_err();

        assert_eq!(error, SafeCommandError::change_failed(false));
        assert!(!recover_called);
        assert!(root.is_dir());
        assert!(manifest_path.is_file());
        let error_json = serde_json::to_string(&error).unwrap();
        assert!(!error_json.contains(&root.to_string_lossy().to_string()));
    }

    #[test]
    fn missing_transaction_manifest_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("transaction");
        fs::create_dir(&root).unwrap();
        let target = temp.path().join("codex.json");
        let mut recover_called = false;

        let error = recover_provider_transaction_at(&root, &[target], |_| {
            recover_called = true;
            Ok(Some(true))
        })
        .unwrap_err();

        assert_eq!(error, SafeCommandError::change_failed(false));
        assert!(root.is_dir());
        assert!(!root.join("manifest.json").exists());
        assert!(!recover_called);
    }

    #[test]
    fn new_target_manifest_cleans_only_after_validating_terminal_paths() {
        let fixture = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(fixture.path());
        let claude_names = transaction_paths_for_home(fixture.path(), "claude-desktop")
            .unwrap()
            .into_iter()
            .map(|path| path.file_name().unwrap().to_owned())
            .collect::<Vec<_>>();
        for phase in [
            ProviderTransactionPhase::Committed,
            ProviderTransactionPhase::CodexStarted,
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("transaction");
            fs::create_dir(&root).unwrap();
            let mut paths = claude_names
                .iter()
                .map(|name| temp.path().join(name))
                .collect::<Vec<_>>();
            paths.push(temp.path().join("codex.json"));
            paths.push(temp.path().join("claude-desktop.json"));
            let manifest = ProviderTransactionManifest {
                version: LEGACY_PROVIDER_TRANSACTION_VERSION,
                phase,
                existed: vec![false; paths.len()],
                known_codex_transactions: vec!["fixture-transaction".to_owned()],
                target_ids: Some(vec!["codex".to_owned(), "claude-desktop".to_owned()]),
            };
            persist_provider_manifest(&root, &manifest).unwrap();
            let mut recover_called = false;
            recover_provider_transaction_at(&root, &paths, |known| {
                recover_called = true;
                assert_eq!(known, ["fixture-transaction".to_owned()]);
                Ok(Some(true))
            })
            .unwrap();
            assert_eq!(
                recover_called,
                matches!(phase, ProviderTransactionPhase::CodexStarted)
            );
            assert!(!root.exists());
        }
    }

    #[cfg(windows)]
    #[test]
    fn persist_provider_manifest_replaces_manifest_after_closing_temporary_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("transaction");
        fs::create_dir(&root).unwrap();
        let mut manifest = ProviderTransactionManifest {
            version: PROVIDER_TRANSACTION_VERSION,
            phase: ProviderTransactionPhase::Prepared,
            existed: vec![false; 3],
            known_codex_transactions: Vec::new(),
            target_ids: Some(vec!["codex".to_owned()]),
        };

        persist_provider_manifest(&root, &manifest).unwrap();
        manifest.phase = ProviderTransactionPhase::Committed;
        persist_provider_manifest(&root, &manifest).unwrap();

        let stored: ProviderTransactionManifest =
            serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
        assert!(matches!(stored.phase, ProviderTransactionPhase::Committed));
        assert!(!root.join("manifest.tmp").exists());
    }

    fn isolated_plan(mixed: bool) -> ApplyPlan {
        ApplyPlan {
            base_url: "https://fixture.invalid/v1".to_owned(),
            api_key: "fixture-new-key".to_owned(),
            model_group: Some("fixture-group".to_owned()),
            model: Some("fixture-model".to_owned()),
            codex_mixed: mixed,
        }
    }

    fn isolated_write(path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn isolated_seed(paths: &[PathBuf]) {
        for path in paths {
            let contents = match path.file_name().and_then(|name| name.to_str()) {
                Some("auth.json") => r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":"fixture-old-key","tokens":{"id_token":"fixture-token"}}"#,
                Some("config.toml") => "model_provider = \"openai\"\nmodel = \"fixture-old-model\"\n",
                _ => "{\"fixture_before\":true}",
            };
            isolated_write(path, contents.as_bytes());
        }
    }

    fn isolated_contents(paths: &[PathBuf]) -> Vec<Option<Vec<u8>>> {
        paths.iter().map(|path| match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => panic!("fixture read failed: {error}"),
        }).collect()
    }

    struct IsolatedTarget {
        id: &'static str,
        paths: Vec<PathBuf>,
    }

    impl Target for IsolatedTarget {
        fn id(&self) -> &'static str { self.id }
        fn display_name(&self) -> &'static str { "fixture target" }
        fn is_installed(&self) -> bool { panic!("installation probes are forbidden in fixtures") }
        fn apply(&self, _plan: &ApplyPlan) -> Result<ApplySummary, String> {
            for path in &self.paths {
                fs::write(path, b"{\"fixture_after\":true}").map_err(|error| error.to_string())?;
            }
            Ok(ApplySummary { target_id: self.id.to_owned(), changed_keys: vec!["fixture".to_owned()] })
        }
    }

    #[test]
    fn isolated_codex_second_file_failure_restores_auth_config_and_record() {
        for mixed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let _home = crate::targets::set_test_home(temp.path());
            let ids = vec!["codex".to_owned()];
            let paths = provider_transaction_paths_for_targets(&ids).unwrap();
            assert_eq!(paths.len(), 3);
            assert!(paths.iter().all(|path| path.starts_with(temp.path())));
            isolated_seed(&paths);
            let before = isolated_contents(&paths);
            // The first auth write succeeds; the second file's temporary-file
            // creation fails deterministically on both Unix and Windows.
            fs::create_dir(paths[1].with_extension("tmp")).unwrap();
            let target = crate::targets::CodexTarget;
            assert!(apply_explicit_targets(&[&target], &isolated_plan(mixed), |_| Ok(())).is_err());
            assert_eq!(isolated_contents(&paths), before);
            assert!(!provider_transaction_root().exists());
        }
    }

    #[test]
    fn isolated_codex_modes_commit_and_restore_preserving_chatgpt_tokens() {
        for mixed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let _home = crate::targets::set_test_home(temp.path());
            let paths = provider_transaction_paths_for_targets(&["codex".to_owned()]).unwrap();
            isolated_seed(&paths);
            let codex = crate::targets::CodexTarget;
            apply_explicit_targets(&[&codex], &isolated_plan(mixed), |_| Ok(())).unwrap();
            let auth: serde_json::Value = serde_json::from_slice(&fs::read(&paths[0]).unwrap()).unwrap();
            let config: toml::Table = fs::read_to_string(&paths[1]).unwrap().parse().unwrap();
            assert_eq!(auth["tokens"]["id_token"], "fixture-token");
            assert_eq!(config["model_provider"].as_str(), Some("custom"));
            if mixed {
                assert!(auth.get("OPENAI_API_KEY").is_none());
                assert_eq!(config["model_providers"]["custom"]["experimental_bearer_token"].as_str(), Some("fixture-new-key"));
            } else {
                assert_eq!(auth["OPENAI_API_KEY"], "fixture-new-key");
                assert_eq!(auth["auth_mode"], "apikey");
            }
            assert!(!provider_transaction_root().exists());
            restore_explicit_target("codex", |_| Ok(())).unwrap();
            let auth: serde_json::Value = serde_json::from_slice(&fs::read(&paths[0]).unwrap()).unwrap();
            assert_eq!(auth["tokens"]["id_token"], "fixture-token");
            assert!(auth.get("OPENAI_API_KEY").is_none());
            let config: toml::Table = fs::read_to_string(&paths[1]).unwrap().parse().unwrap();
            assert!(config.get("model_provider").is_none());
            assert!(!paths[2].exists());
            assert!(!provider_transaction_root().exists());
        }
    }

    #[test]
    fn isolated_codex_failure_restores_original_file_absence() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(temp.path());
        let paths = provider_transaction_paths_for_targets(&["codex".to_owned()]).unwrap();
        fs::create_dir_all(paths[1].with_extension("tmp")).unwrap();
        let target = crate::targets::CodexTarget;
        assert!(apply_explicit_targets(&[&target], &isolated_plan(false), |_| Ok(())).is_err());
        assert_eq!(isolated_contents(&paths), vec![None, None, None]);
        assert!(!provider_transaction_root().exists());
    }

    #[test]
    fn isolated_codex_preflight_rejects_bad_input_before_any_transaction_write() {
        for index in [0usize, 1] {
            let temp = tempfile::tempdir().unwrap();
            let _home = crate::targets::set_test_home(temp.path());
            let paths = provider_transaction_paths_for_targets(&["codex".to_owned()]).unwrap();
            isolated_seed(&paths);
            fs::write(&paths[index], b"this is not valid configuration [").unwrap();
            let before = isolated_contents(&paths);
            let target = crate::targets::CodexTarget;
            assert!(apply_explicit_targets(&[&target], &isolated_plan(false), |_| Ok(())).is_err());
            assert_eq!(isolated_contents(&paths), before);
            assert!(!provider_transaction_root().exists());
            assert!(!temp.path().join(".niko/backups").exists());
        }
    }

    #[test]
    fn isolated_batch_failure_rolls_back_previously_applied_target_and_codex() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(temp.path());
        let ids = vec!["codex".to_owned(), "claude-desktop".to_owned()];
        let paths = provider_transaction_paths_for_targets(&ids).unwrap();
        isolated_seed(&paths);
        let before = isolated_contents(&paths);
        fs::create_dir(paths[1].with_extension("tmp")).unwrap();
        let codex = crate::targets::CodexTarget;
        let other = IsolatedTarget {
            id: "claude-desktop",
            paths: transaction_paths_for_home(temp.path(), "claude-desktop").unwrap(),
        };
        let mut applied = Vec::new();
        let result = apply_explicit_targets(&[&codex, &other], &isolated_plan(false), |step| {
            if let ProviderTransactionStep::AfterTarget(id) = step { applied.push(id); }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(applied, vec!["claude-desktop"]);
        assert_eq!(isolated_contents(&paths), before);
        assert!(!provider_transaction_root().exists());
    }

    #[test]
    fn isolated_restore_second_file_failure_restores_entire_previous_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(temp.path());
        let paths = provider_transaction_paths_for_targets(&["codex".to_owned()]).unwrap();
        isolated_seed(&paths);
        let codex = crate::targets::CodexTarget;
        apply_explicit_targets(&[&codex], &isolated_plan(false), |_| Ok(())).unwrap();
        let before = isolated_contents(&paths);
        // restore writes config first, then auth. Block only the auth temp file.
        fs::create_dir(paths[0].with_extension("tmp")).unwrap();
        assert!(restore_explicit_target("codex", |_| Ok(())).is_err());
        assert_eq!(isolated_contents(&paths), before);
        assert!(!provider_transaction_root().exists());
    }

    #[test]
    fn isolated_v2_shared_paths_are_backed_up_once_in_canonical_order() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(temp.path());
        let ids = vec!["codex".to_owned(), "claude-desktop".to_owned(), "claude-cli".to_owned()];
        let paths = provider_transaction_paths_for_targets(&ids).unwrap();
        let shared = temp.path().join(".claude/settings.json");
        assert_eq!(paths.iter().filter(|path| *path == &shared).count(), 1);
        assert_eq!(paths[0], temp.path().join(".codex/auth.json"));
        assert_eq!(paths[1], temp.path().join(".codex/config.toml"));
        isolated_seed(&paths);
        let before = isolated_contents(&paths);
        let codex = crate::targets::CodexTarget;
        let desktop = IsolatedTarget { id: "claude-desktop", paths: transaction_paths_for_home(temp.path(), "claude-desktop").unwrap() };
        let cli = IsolatedTarget { id: "claude-cli", paths: vec![shared] };
        let result = apply_explicit_targets(&[&codex, &desktop, &cli], &isolated_plan(false), |step| {
            if step == ProviderTransactionStep::BeforeManifest(ProviderTransactionPhase::Committed) {
                Err(SafeCommandError::change_failed(true))
            } else { Ok(()) }
        });
        assert!(result.is_err());
        assert_eq!(isolated_contents(&paths), before);
    }

    fn isolated_checkpoints() -> Vec<ProviderTransactionStep> {
        use ProviderTransactionPhase::*;
        use ProviderTransactionStep::*;
        vec![
            BeforeManifest(Prepared), AfterManifest(Prepared),
            BeforeManifest(RecordsApplied), AfterManifest(RecordsApplied),
            BeforeTarget("claude-desktop".to_owned()), AfterTarget("claude-desktop".to_owned()),
            BeforeManifest(ClaudeApplied), AfterManifest(ClaudeApplied),
            BeforeManifest(CodexStarted), AfterManifest(CodexStarted),
            BeforeTarget("codex".to_owned()), AfterTarget("codex".to_owned()),
            BeforeTargetSync, BeforeManifest(Committed),
        ]
    }

    #[test]
    fn isolated_each_precommit_failure_rolls_back_complete_batch() {
        for failed_step in isolated_checkpoints() {
            let temp = tempfile::tempdir().unwrap();
            let _home = crate::targets::set_test_home(temp.path());
            let ids = vec!["codex".to_owned(), "claude-desktop".to_owned()];
            let paths = provider_transaction_paths_for_targets(&ids).unwrap();
            isolated_seed(&paths);
            let before = isolated_contents(&paths);
            let codex = crate::targets::CodexTarget;
            let other = IsolatedTarget { id: "claude-desktop", paths: transaction_paths_for_home(temp.path(), "claude-desktop").unwrap() };
            let result = apply_explicit_targets(&[&codex, &other], &isolated_plan(false), |step| {
                if step == failed_step { Err(SafeCommandError::change_failed(true)) } else { Ok(()) }
            });
            assert!(result.is_err(), "{failed_step:?}");
            assert_eq!(isolated_contents(&paths), before, "{failed_step:?}");
            assert!(!provider_transaction_root().exists(), "{failed_step:?}");
        }
    }

    #[test]
    fn isolated_crash_recovery_is_idempotent_at_every_durable_phase() {
        let mut checkpoints = isolated_checkpoints();
        checkpoints.push(ProviderTransactionStep::AfterManifest(ProviderTransactionPhase::Committed));
        for crash_at in checkpoints {
            let temp = tempfile::tempdir().unwrap();
            let _home = crate::targets::set_test_home(temp.path());
            let ids = vec!["codex".to_owned(), "claude-desktop".to_owned()];
            let paths = provider_transaction_paths_for_targets(&ids).unwrap();
            isolated_seed(&paths);
            let before = isolated_contents(&paths);
            let codex = crate::targets::CodexTarget;
            let other = IsolatedTarget { id: "claude-desktop", paths: transaction_paths_for_home(temp.path(), "claude-desktop").unwrap() };
            let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = apply_explicit_targets(&[&codex, &other], &isolated_plan(false), |step| {
                    if step == crash_at { panic!("fixture crash at {step:?}"); }
                    Ok(())
                });
            }));
            assert!(interrupted.is_err());
            let after_crash = isolated_contents(&paths);
            recover_provider_transaction_without_sessions().unwrap();
            recover_provider_transaction_without_sessions().unwrap();
            let expected = if crash_at == ProviderTransactionStep::AfterManifest(ProviderTransactionPhase::Committed) {
                after_crash
            } else { before };
            assert_eq!(isolated_contents(&paths), expected, "{crash_at:?}");
            assert!(!provider_transaction_root().exists());
        }
    }

    #[test]
    fn isolated_partial_record_write_failure_restores_records_before_targets_run() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(temp.path());
        let ids = vec!["codex".to_owned(), "claude-cli".to_owned()];
        let paths = provider_transaction_paths_for_targets(&ids).unwrap();
        isolated_seed(&paths);
        let before = isolated_contents(&paths);
        let result = run_provider_transaction_at::<()>(
            &provider_transaction_root(), &paths, &ids,
            || {
                fs::write(record_path(temp.path(), "codex").unwrap(), b"partially-written-record").unwrap();
                Err(SafeCommandError::change_failed(true))
            },
            |_| panic!("targets must not run after a record write fails"),
            |_| Ok(()),
        );
        assert!(result.is_err());
        assert_eq!(isolated_contents(&paths), before);
        assert!(!provider_transaction_root().exists());
    }

    #[test]
    fn isolated_commit_error_after_manifest_replacement_still_rolls_back() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(temp.path());
        let paths = provider_transaction_paths_for_targets(&["codex".to_owned()]).unwrap();
        isolated_seed(&paths);
        let before = isolated_contents(&paths);
        let codex = crate::targets::CodexTarget;
        let result = apply_explicit_targets(&[&codex], &isolated_plan(false), |step| {
            if step == ProviderTransactionStep::BeforeManifest(ProviderTransactionPhase::Committed) {
                // Emulate rename succeeding followed by a reported sync failure.
                let root = provider_transaction_root();
                let mut manifest: ProviderTransactionManifest = serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
                manifest.phase = ProviderTransactionPhase::Committed;
                persist_provider_manifest(&root, &manifest).unwrap();
                return Err(SafeCommandError::change_failed(false));
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(isolated_contents(&paths), before);
        assert!(!provider_transaction_root().exists());
    }

    #[test]
    fn isolated_durable_commit_survives_cleanup_interruption() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(temp.path());
        let paths = provider_transaction_paths_for_targets(&["codex".to_owned()]).unwrap();
        isolated_seed(&paths);
        let codex = crate::targets::CodexTarget;
        let result = apply_explicit_targets(&[&codex], &isolated_plan(false), |step| {
            if step == ProviderTransactionStep::AfterManifest(ProviderTransactionPhase::Committed) {
                Err(SafeCommandError::change_failed(false))
            } else { Ok(()) }
        });
        assert!(result.is_ok());
        let committed = isolated_contents(&paths);
        assert!(provider_transaction_root().join("manifest.json").is_file());
        recover_provider_transaction_without_sessions().unwrap();
        assert_eq!(isolated_contents(&paths), committed);
        assert!(!provider_transaction_root().exists());
    }

    #[test]
    fn isolated_interrupted_rollback_restores_remaining_files_and_absence() {
        for absent_auth in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let _home = crate::targets::set_test_home(temp.path());
            let ids = vec!["codex".to_owned()];
            let paths = provider_transaction_paths_for_targets(&ids).unwrap();
            if absent_auth { isolated_seed(&paths[1..]); } else { isolated_seed(&paths); }
            let before = isolated_contents(&paths);
            let root = provider_transaction_root();
            let mut manifest = begin_provider_transaction_at_with_targets(&root, &paths, Vec::new(), Some(ids)).unwrap();
            for path in &paths { isolated_write(path, b"fixture-after"); }
            manifest.phase = ProviderTransactionPhase::RollingBack;
            persist_provider_manifest(&root, &manifest).unwrap();
            // Simulate a crash after one backup was already restored.
            replace_from_backup(&root.join("1.backup"), &paths[1]).unwrap();
            recover_provider_transaction_without_sessions().unwrap();
            recover_provider_transaction_without_sessions().unwrap();
            assert_eq!(isolated_contents(&paths), before);
        }
    }

    #[test]
    fn isolated_legacy_both_layouts_keep_backup_indices_and_unrecorded_codex_files() {
        for with_targets in [false, true] {
            for committed in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let _home = crate::targets::set_test_home(temp.path());
                let root = provider_transaction_root();
                let ids = with_targets.then(|| vec!["codex".to_owned(), "claude-desktop".to_owned()]);
                let mut manifest = ProviderTransactionManifest {
                    version: LEGACY_PROVIDER_TRANSACTION_VERSION,
                    phase: ProviderTransactionPhase::Prepared,
                    existed: Vec::new(),
                    known_codex_transactions: Vec::new(),
                    target_ids: ids.clone(),
                };
                let paths = manifest_transaction_paths_at(temp.path(), &manifest).unwrap();
                assert!(!paths.contains(&temp.path().join(".codex/auth.json")));
                assert!(!paths.contains(&temp.path().join(".codex/config.toml")));
                isolated_seed(&paths);
                let codex_paths = transaction_paths_for_home(temp.path(), "codex").unwrap();
                isolated_seed(&codex_paths);
                let codex_before = isolated_contents(&codex_paths);
                let before = isolated_contents(&paths);
                manifest = begin_provider_transaction_with_version_at(&root, &paths, Vec::new(), ids, LEGACY_PROVIDER_TRANSACTION_VERSION).unwrap();
                for path in &paths { fs::write(path, b"legacy-after").unwrap(); }
                let after = isolated_contents(&paths);
                manifest.phase = if committed { ProviderTransactionPhase::Committed } else { ProviderTransactionPhase::CodexStarted };
                persist_provider_manifest(&root, &manifest).unwrap();
                recover_provider_transaction_without_sessions().unwrap();
                assert_eq!(isolated_contents(&paths), if committed { after } else { before });
                assert_eq!(isolated_contents(&codex_paths), codex_before);
                assert!(!root.exists());
            }
        }
    }

    #[test]
    fn isolated_legacy_inner_transaction_requires_explicit_session_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(temp.path());
        let ids = vec!["codex".to_owned()];
        let paths = provider_transaction_paths_for_layout(temp.path(), &ids, LEGACY_PROVIDER_TRANSACTION_VERSION).unwrap();
        isolated_seed(&paths);
        let root = provider_transaction_root();
        let mut manifest = begin_provider_transaction_with_version_at(&root, &paths, vec!["legacy-inner".to_owned()], Some(ids), LEGACY_PROVIDER_TRANSACTION_VERSION).unwrap();
        manifest.phase = ProviderTransactionPhase::CodexStarted;
        persist_provider_manifest(&root, &manifest).unwrap();
        let journal = fs::read(root.join("manifest.json")).unwrap();
        assert_eq!(recover_provider_transaction_without_sessions().unwrap_err(), SafeCommandError::busy());
        assert_eq!(fs::read(root.join("manifest.json")).unwrap(), journal);
        // The explicit legacy recovery's callback is a fixture, never migration IPC.
        recover_provider_transaction_at(&root, &paths, |_| Ok(Some(false))).unwrap();
    }

    #[test]
    fn isolated_unknown_and_invalid_v2_journals_are_preserved() {
        let cases = [
            serde_json::json!({"version":99,"phase":"committed","existed":[false,false,false],"known_codex_transactions":[],"target_ids":["codex"]}),
            serde_json::json!({"version":2,"phase":"committed","existed":[false,false,false],"known_codex_transactions":[]}),
            serde_json::json!({"version":2,"phase":"codex_started","existed":[false,false,false],"known_codex_transactions":["not-allowed"],"target_ids":["codex"]}),
            serde_json::json!({"version":2,"phase":"committed","existed":[false],"known_codex_transactions":[],"target_ids":["codex"]}),
            serde_json::json!({"version":2,"phase":"future_phase","existed":[false,false,false],"known_codex_transactions":[],"target_ids":["codex"]}),
        ];
        for value in cases {
            let temp = tempfile::tempdir().unwrap();
            let _home = crate::targets::set_test_home(temp.path());
            let paths = provider_transaction_paths_for_targets(&["codex".to_owned()]).unwrap();
            isolated_seed(&paths);
            let before = isolated_contents(&paths);
            let root = provider_transaction_root();
            let bytes = serde_json::to_vec(&value).unwrap();
            isolated_write(&root.join("manifest.json"), &bytes);
            assert!(recover_provider_transaction_without_sessions().is_err());
            assert_eq!(fs::read(root.join("manifest.json")).unwrap(), bytes);
            assert_eq!(isolated_contents(&paths), before);
        }
    }

    #[test]
    fn isolated_missing_backup_rejects_before_restoring_any_file() {
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(temp.path());
        let ids = vec!["codex".to_owned()];
        let paths = provider_transaction_paths_for_targets(&ids).unwrap();
        isolated_seed(&paths);
        let before = isolated_contents(&paths);
        let root = provider_transaction_root();
        isolated_write(&root.join("0.backup"), b"auth-backup");
        isolated_write(&root.join("1.backup"), b"config-backup");
        let manifest = ProviderTransactionManifest { version: PROVIDER_TRANSACTION_VERSION, phase: ProviderTransactionPhase::CodexStarted, existed: vec![true; 3], known_codex_transactions: Vec::new(), target_ids: Some(ids) };
        persist_provider_manifest(&root, &manifest).unwrap();
        assert!(recover_provider_transaction_without_sessions().is_err());
        assert_eq!(isolated_contents(&paths), before);
        assert!(root.join("manifest.json").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn isolated_auth_backups_and_restored_auth_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::targets::set_test_home(temp.path());
        let paths = provider_transaction_paths_for_targets(&["codex".to_owned()]).unwrap();
        isolated_seed(&paths);
        fs::set_permissions(&paths[0], fs::Permissions::from_mode(0o644)).unwrap();
        let target = crate::targets::CodexTarget;
        let result = apply_explicit_targets(&[&target], &isolated_plan(false), |step| {
            if step == ProviderTransactionStep::AfterManifest(ProviderTransactionPhase::Prepared) {
                assert_eq!(fs::metadata(provider_transaction_root().join("0.backup")).unwrap().permissions().mode() & 0o777, 0o600);
                return Err(SafeCommandError::change_failed(true));
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(fs::metadata(&paths[0]).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[cfg(windows)]
    #[test]
    fn windows_coordinator_uses_platform_atomic_replace() {
        let _replace: fn(&Path, &Path) -> Result<(), SafeCommandError> = durable_replace;
        let _sync: fn(&Path) -> Result<(), SafeCommandError> = sync_parent;
    }
}
