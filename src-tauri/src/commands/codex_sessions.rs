use crate::codex_sessions::{
    codex_migration_ids, migrate_codex_sessions_transactional, preflight_codex_session_migration,
    recover_codex_migration_since, recover_codex_session_migrations, scan_codex_sessions,
    CodexMigrationInput, CodexProcessPolicy, DiagnosticLevel, MigrationErrorKind, MigrationOptions,
    MigrationOutcome, MigrationProviderTarget, MigrationRequest, NormalizationStatus, ScanReport,
    ScanRequest, ThreadInventory, MIGRATION_ROOT_MARKER, MIGRATION_ROOT_MARKER_CONTENT,
};
use crate::commands::safe_error::SafeCommandError;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const QUERY_MAX: usize = 80;
const PAGE_MAX: usize = 50;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CodexSessionBlocker {
    pub title: String,
    pub thread_id: String,
    pub reason: &'static str,
    pub next_step: &'static str,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CodexSessionThread {
    pub thread_id: String,
    pub title: Option<String>,
    // Keep the #60 IPC field shape without returning session text.
    pub summary: Option<String>,
    pub updated_at: Option<String>,
    pub archived: bool,
    pub provider: Option<String>,
    pub can_continue: bool,
    pub needs_migration: bool,
    pub blockers: Vec<CodexSessionBlocker>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CodexSessionPage {
    pub status: &'static str,
    pub items: Vec<CodexSessionThread>,
    pub blockers: Vec<CodexSessionBlocker>,
    pub page: usize,
    pub page_size: usize,
    pub total: usize,
    pub total_pages: usize,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CodexSessionMutationOutcome {
    pub status: &'static str,
    pub message: String,
    pub requested: usize,
    pub migrated: usize,
    pub failed: usize,
    pub changed_artifacts: usize,
}

pub(crate) fn mutation_outcome(
    status: &'static str,
    message: impl Into<String>,
    requested: usize,
    migrated: usize,
    changed_artifacts: usize,
) -> CodexSessionMutationOutcome {
    CodexSessionMutationOutcome {
        status,
        message: message.into(),
        requested,
        migrated,
        failed: 0,
        changed_artifacts,
    }
}

fn home_dir() -> PathBuf {
    crate::targets::user_home_dir()
}

fn codex_home() -> PathBuf {
    home_dir().join(".codex")
}

fn is_uuid_like(value: &str) -> bool {
    value.len() == 36
        && value.chars().enumerate().all(|(index, ch)| match index {
            8 | 13 | 18 | 23 => ch == '-',
            _ => ch.is_ascii_hexdigit(),
        })
}

fn report_status(report: &ScanReport) -> &'static str {
    if report.is_blocked() {
        "blocked"
    } else if report.normalization.status == NormalizationStatus::WouldNormalize {
        "needs_check"
    } else {
        "healthy"
    }
}

fn compact_session_text(value: Option<&str>) -> Option<String> {
    let mut compact = String::new();
    let mut spaced = false;
    for character in value?.trim().chars() {
        if character.is_control() {
            spaced = true;
            continue;
        }
        if spaced && !compact.is_empty() {
            compact.push(' ');
        }
        spaced = false;
        compact.push(character);
    }
    let compact = compact.trim();
    (!compact.is_empty()).then(|| compact.to_owned())
}

fn contains_sensitive_session_text(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.contains('/')
        || value.contains('\\')
        || lower.contains("://")
        || lower.contains("auth.json")
        || lower.contains("config.toml")
        || lower.contains("sqlite")
        || lower.contains("journal")
        || lower.contains("wal")
        || lower.contains("api_key")
        || lower.contains("sk-")
        || lower.contains("bearer ")
        || lower.contains("stack trace")
        || lower.contains("traceback")
        || lower.contains("panic")
        || lower.contains("exception")
}

fn safe_session_text(value: Option<&str>, max_chars: usize) -> Option<String> {
    let compact = compact_session_text(value)?;
    if compact.chars().count() > max_chars || contains_sensitive_session_text(&compact) {
        return None;
    }
    Some(compact)
}

fn safe_session_title(value: Option<&str>) -> Option<String> {
    let compact = compact_session_text(value)?;
    if contains_sensitive_session_text(&compact) {
        return None;
    }
    let mut title = compact.chars().take(119).collect::<String>();
    if compact.chars().count() > 119 {
        title.push('…');
    }
    Some(title)
}

fn safe_session_summary(value: Option<&str>) -> Option<String> {
    safe_session_text(value, 96)
}

fn safe_session_id(value: &str) -> String {
    if is_uuid_like(value) {
        value.to_owned()
    } else {
        "无法确认".to_owned()
    }
}

fn session_provider_label(provider: &str) -> &'static str {
    match provider {
        "custom" => "Niko 模型服务",
        "openai" => "ChatGPT 官方模型服务",
        _ => "已记录的模型服务",
    }
}

fn blocker_copy(code: &str, title: Option<&str>, thread_id: Option<&str>) -> CodexSessionBlocker {
    let (reason, next_step) = match code {
        "duplicate_thread_id" => (
            "同一会话的本地记录重复。",
            "关闭 ChatGPT 后重新检查；确认前不要迁移该会话。",
        ),
        "thread_provider_mismatch" => (
            "会话的模型服务记录不一致。",
            "关闭 ChatGPT 后重新检查；确认前不要迁移该会话。",
        ),
        "thread_archive_mismatch" => (
            "会话的归档状态不一致。",
            "关闭 ChatGPT 后重新检查；确认前不要迁移该会话。",
        ),
        "thread_storage_incomplete" => (
            "会话缺少可验证的本地记录。",
            "关闭 ChatGPT 后重新检查；确认前不要迁移该会话。",
        ),
        "thread_rollout_path_mismatch" => (
            "会话记录指向的本地内容不一致。",
            "关闭 ChatGPT 后重新检查；确认前不要迁移该会话。",
        ),
        code if code.starts_with("config_") || code == "active_provider_definition_missing" => (
            "ChatGPT 设置无法确认。",
            "关闭 ChatGPT 后重新检查会话。",
        ),
        code if code.starts_with("sqlite_") => (
            "会话数据库无法确认。",
            "关闭 ChatGPT 后重新检查会话。",
        ),
        code if code.starts_with("rollout_") || code.starts_with("session_index_") => (
            "会话记录无法确认。",
            "关闭 ChatGPT 后重新检查会话。",
        ),
        _ => (
            "会话的本地结构无法确认。",
            "关闭 ChatGPT 后重新检查会话。",
        ),
    };
    CodexSessionBlocker {
        title: safe_session_title(title).unwrap_or_else(|| "未命名会话".to_owned()),
        thread_id: thread_id.map(safe_session_id).unwrap_or_else(|| "无法确认".to_owned()),
        reason,
        next_step,
    }
}

fn display_session_title(thread: &ThreadInventory) -> String {
    safe_session_title(thread.title.as_deref())
        .or_else(|| safe_session_summary(thread.summary.as_deref()))
        .unwrap_or_else(|| "未命名会话".to_owned())
}

fn blockers_for_thread(
    report: &ScanReport,
    thread: &ThreadInventory,
) -> Vec<CodexSessionBlocker> {
    let title = display_session_title(thread);
    report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.level == DiagnosticLevel::Blocker
                && diagnostic.thread_id.as_deref() == Some(thread.thread_id.as_str())
        })
        .map(|diagnostic| blocker_copy(diagnostic.code, Some(&title), Some(&thread.thread_id)))
        .collect()
}

fn thread_route_is_healthy(archived: Option<bool>, providers: &[String]) -> bool {
    // A historical provider can differ from the active route; only the
    // thread-local provider bucket and archive state are relevant here.
    archived == Some(false)
        && providers.len() == 1
        && !providers[0].trim().is_empty()
}

fn thread_storage_is_healthy(report: &ScanReport, thread: &ThreadInventory) -> bool {
    is_uuid_like(&thread.thread_id)
        && thread.rollout_paths.len() == 1
        && thread.state_databases.len() == 1
        && thread.history_databases.len() <= 1
        && thread.archived == Some(false)
        && thread.providers.len() == 1
        && !report.diagnostics.iter().any(|diagnostic| {
            diagnostic.level == DiagnosticLevel::Blocker
                && (diagnostic.thread_id.is_none()
                    || diagnostic.thread_id.as_deref() == Some(&thread.thread_id))
        })
}

fn thread_is_healthy(report: &ScanReport, thread: &ThreadInventory) -> bool {
    thread_storage_is_healthy(report, thread)
        && thread_route_is_healthy(thread.archived, &thread.providers)
}

fn thread_needs_migration_to(
    report: &ScanReport,
    thread: &ThreadInventory,
    target_provider: &str,
) -> bool {
    thread_storage_is_healthy(report, thread)
        && (report.config.active_provider.as_deref() != Some(target_provider)
            || thread.providers.first().map(String::as_str) != Some(target_provider))
}

fn thread_needs_migration(report: &ScanReport, thread: &ThreadInventory) -> bool {
    thread_needs_migration_to(report, thread, "custom")
}

fn migration_requested(report: &ScanReport, target: MigrationProviderTarget) -> usize {
    let target_provider = match target {
        MigrationProviderTarget::Custom => "custom",
        MigrationProviderTarget::OpenAi => "openai",
    };
    report
        .threads
        .iter()
        .filter(|thread| thread_needs_migration_to(report, thread, target_provider))
        .count()
}

fn migration_provider_count(report: &ScanReport, target: MigrationProviderTarget) -> usize {
    let target_provider = match target {
        MigrationProviderTarget::Custom => "custom",
        MigrationProviderTarget::OpenAi => "openai",
    };
    report
        .threads
        .iter()
        .filter(|thread| {
            thread_storage_is_healthy(report, thread)
                && thread.providers.first().map(String::as_str) != Some(target_provider)
        })
        .count()
}

fn page_from_report(
    report: &ScanReport,
    query: &str,
    page: usize,
    page_size: usize,
) -> Result<CodexSessionPage, SafeCommandError> {
    if page == 0 {
        return Err(SafeCommandError::invalid_request());
    }
    let query = query.trim().to_lowercase();
    let mut threads = report
        .threads
        .iter()
        .filter(|thread| {
            let display_title = display_session_title(thread);
            query.is_empty()
                || thread.thread_id.to_lowercase().contains(&query)
                || display_title.to_lowercase().contains(&query)
        })
        .collect::<Vec<_>>();
    // Keep one deterministic order for all projects: newest session first.
    // Missing timestamps stay at the end and the id only breaks ties.
    threads.sort_by(|left, right| {
        right
            .updated_at_ms
            .cmp(&left.updated_at_ms)
            .then_with(|| right.thread_id.cmp(&left.thread_id))
    });
    let total = threads.len();
    let total_pages = if total == 0 {
        0
    } else {
        (total - 1) / page_size + 1
    };
    if page > total_pages && !(total == 0 && page == 1) {
        return Err(SafeCommandError::invalid_request());
    }
    let start = page.saturating_sub(1).saturating_mul(page_size);
    let end = start.saturating_add(page_size).min(total);
    let items = threads[start..end]
        .iter()
        .map(|thread| {
            CodexSessionThread {
                thread_id: safe_session_id(&thread.thread_id),
                title: Some(display_session_title(thread)),
                summary: None,
                updated_at: thread.updated_at_ms.map(|value| value.to_string()),
                archived: thread.archived.unwrap_or(false),
                provider: (thread.providers.len() == 1)
                    .then(|| session_provider_label(&thread.providers[0]).to_owned()),
                can_continue: thread_is_healthy(report, thread),
                needs_migration: thread_needs_migration(report, thread),
                blockers: blockers_for_thread(report, thread),
            }
        })
        .collect();
    let blockers = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.level == DiagnosticLevel::Blocker && diagnostic.thread_id.is_none())
        .map(|diagnostic| blocker_copy(diagnostic.code, Some("ChatGPT 会话检查"), None))
        .chain(threads[start..end].iter().flat_map(|thread| {
            blockers_for_thread(report, thread)
        }))
        .collect();
    Ok(CodexSessionPage {
        status: report_status(report),
        items,
        blockers,
        page,
        page_size,
        total,
        total_pages,
    })
}

#[tauri::command]
pub async fn scan_codex_session_inventory(
    query: Option<String>,
    page: Option<usize>,
    page_size: Option<usize>,
) -> Result<CodexSessionPage, SafeCommandError> {
    let query = query.unwrap_or_default();
    if query.chars().count() > QUERY_MAX {
        return Err(SafeCommandError::invalid_request());
    }
    let page = page.unwrap_or(1);
    if page == 0 {
        return Err(SafeCommandError::invalid_request());
    }
    let page_size = page_size.unwrap_or(PAGE_MAX).clamp(1, PAGE_MAX);
    let root = codex_home();
    if !root.exists() {
        if page > 1 {
            return Err(SafeCommandError::invalid_request());
        }
        return Ok(CodexSessionPage {
            status: "healthy",
            items: Vec::new(),
            blockers: Vec::new(),
            page,
            page_size,
            total: 0,
            total_pages: 0,
        });
    }
    if !root.is_dir() {
        return Err(SafeCommandError::read_failed());
    }
    let report = scan_codex_sessions(&ScanRequest::new(root))
        .map_err(|_| SafeCommandError::read_failed())?;
    page_from_report(&report, &query, page, page_size)
}

fn normalize_target(target: &str) -> Result<MigrationProviderTarget, SafeCommandError> {
    match target {
        "custom" => Ok(MigrationProviderTarget::Custom),
        "openai" | "official" => Ok(MigrationProviderTarget::OpenAi),
        _ => Err(SafeCommandError::invalid_request()),
    }
}

fn restart_needs_normalization(active_provider: Option<&str>) -> bool {
    active_provider != Some("openai")
}

fn mutation_request(codex: Option<CodexMigrationInput>) -> MigrationRequest {
    MigrationRequest {
        scan: ScanRequest::new(codex_home()),
        codex,
        thread_ids: None,
        options: MigrationOptions {
            busy_retries: 4,
            busy_retry_delay: Duration::from_millis(50),
            process_wait_attempts: 20,
            process_wait_delay: Duration::from_millis(250),
            retained_transactions: 3,
            codex_process_policy: CodexProcessPolicy::RequestNormalExit,
            space_reserve_bytes: 1024 * 1024,
        },
    }
}

fn authorize_codex_root(root: &PathBuf) -> Result<(), SafeCommandError> {
    let marker = root.join(MIGRATION_ROOT_MARKER);
    match fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.file_type().is_file() => fs::read_to_string(marker)
            .ok()
            .filter(|value| value == MIGRATION_ROOT_MARKER_CONTENT)
            .map(|_| ())
            .ok_or_else(|| SafeCommandError::change_failed(false)),
        Ok(_) => Err(SafeCommandError::change_failed(false)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(marker)
                .map_err(|_| SafeCommandError::change_failed(false))?;
            file.write_all(MIGRATION_ROOT_MARKER_CONTENT.as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|_| SafeCommandError::change_failed(false))
        }
        Err(_) => Err(SafeCommandError::change_failed(false)),
    }
}

fn map_migration_error(kind: MigrationErrorKind, retryable: bool) -> SafeCommandError {
    match kind {
        MigrationErrorKind::InvalidRequest
        | MigrationErrorKind::UnknownSchema
        | MigrationErrorKind::CorruptStorage
        | MigrationErrorKind::PermissionDenied
        | MigrationErrorKind::RootNotAuthorized => SafeCommandError::change_failed(false),
        MigrationErrorKind::NikoLocked
        | MigrationErrorKind::NikoLockUnverifiable
        | MigrationErrorKind::ProviderSyncLocked
        | MigrationErrorKind::CodexRunning
        | MigrationErrorKind::FileOccupied
        | MigrationErrorKind::SqliteBusy => SafeCommandError::busy(),
        _ => SafeCommandError::change_failed(retryable),
    }
}

pub(crate) fn normalize_codex_session_storage_inner(
    target_provider: String,
) -> Result<CodexSessionMutationOutcome, SafeCommandError> {
    normalize_codex_session_storage_with_input(target_provider, None)
}

pub(crate) fn normalize_codex_session_storage_with_input(
    target_provider: String,
    codex: Option<CodexMigrationInput>,
) -> Result<CodexSessionMutationOutcome, SafeCommandError> {
    let target = normalize_target(&target_provider)?;
    let root = codex_home();
    if !root.exists() {
        if codex.is_none() {
            return Ok(mutation_outcome("unchanged", "当前状态无需调整。", 0, 0, 0));
        }
        fs::create_dir_all(&root).map_err(|_| SafeCommandError::change_failed(false))?;
    }
    let scan = scan_codex_sessions(&ScanRequest::new(&root))
        .map_err(|_| SafeCommandError::read_failed())?;
    if scan.is_blocked() {
        return Err(SafeCommandError::change_failed(false));
    }
    let requested = migration_requested(&scan, target);
    let planned_migrated = migration_provider_count(&scan, target);
    authorize_codex_root(&root)?;
    match migrate_codex_sessions_transactional(&mutation_request(codex), target) {
        Ok(report) => {
            let migrated = (report.changed_artifacts > 0)
                .then_some(planned_migrated)
                .unwrap_or(0);
            let message = if report.changed_artifacts == 0 {
                "当前状态无需调整。".to_owned()
            } else if target == MigrationProviderTarget::OpenAi {
                format!(
                    "已恢复到官方：处理 {requested} 个会话，迁移 {migrated} 个，更新 {} 个文件。",
                    report.changed_artifacts
                )
            } else {
                format!(
                    "已完成检查：处理 {requested} 个会话，迁移 {migrated} 个，更新 {} 个文件。",
                    report.changed_artifacts
                )
            };
            Ok(mutation_outcome(
                if report.changed_artifacts == 0 { "unchanged" } else { "applied" },
                message,
                requested,
                migrated,
                report.changed_artifacts,
            ))
        }
        Err(error) => Err(map_migration_error(error.kind, error.retryable)),
    }
}

fn normalize_selected_thread_ids(
    thread_ids: Vec<String>,
) -> Result<BTreeSet<String>, SafeCommandError> {
    let mut selected = BTreeSet::new();
    for thread_id in thread_ids {
        let thread_id = thread_id.trim();
        if !is_uuid_like(thread_id) {
            return Err(SafeCommandError::invalid_request());
        }
        selected.insert(thread_id.to_owned());
    }
    if selected.is_empty() || selected.len() > PAGE_MAX {
        return Err(SafeCommandError::invalid_request());
    }
    Ok(selected)
}

pub(crate) fn normalize_codex_session_storage_selected_inner(
    target_provider: String,
    thread_ids: Vec<String>,
) -> Result<CodexSessionMutationOutcome, SafeCommandError> {
    let target = normalize_target(&target_provider)?;
    let selected = normalize_selected_thread_ids(thread_ids)?;
    let root = codex_home();
    if !root.exists() {
        return Err(SafeCommandError::read_failed());
    }
    let scan = scan_codex_sessions(&ScanRequest::new(&root))
        .map_err(|_| SafeCommandError::read_failed())?;
    if scan.is_blocked() {
        return Err(SafeCommandError::change_failed(false));
    }
    if target != MigrationProviderTarget::Custom
        || selected.iter().any(|thread_id| {
            scan.threads
                .iter()
                .find(|thread| thread.thread_id == *thread_id)
                .is_none_or(|thread| !thread_needs_migration(&scan, thread))
        })
    {
        return Err(SafeCommandError::change_failed(false));
    }
    let requested = selected.len();
    let planned_migrated = selected
        .iter()
        .filter(|thread_id| {
            scan.threads.iter().any(|thread| {
                thread.thread_id == **thread_id
                    && thread.providers.first().map(String::as_str) != Some("custom")
            })
        })
        .count();
    authorize_codex_root(&root)?;
    let mut request = mutation_request(None);
    request.thread_ids = Some(selected);
    match migrate_codex_sessions_transactional(&request, target) {
        Ok(report) => {
            let migrated = (report.changed_artifacts > 0)
                .then_some(planned_migrated)
                .unwrap_or(0);
            let message = if report.changed_artifacts == 0 {
                "选中的会话无需调整。".to_owned()
            } else {
                format!(
                    "已修复选中的会话：处理 {requested} 个，迁移 {migrated} 个，更新 {} 个文件。",
                    report.changed_artifacts
                )
            };
            Ok(mutation_outcome(
                if report.changed_artifacts == 0 { "unchanged" } else { "applied" },
                message,
                requested,
                migrated,
                report.changed_artifacts,
            ))
        }
        Err(error) => Err(map_migration_error(error.kind, error.retryable)),
    }
}

pub(crate) fn preflight_codex_session_storage(
    codex: CodexMigrationInput,
) -> Result<Vec<String>, SafeCommandError> {
    let root = codex_home();
    if !root.exists() {
        fs::create_dir_all(&root).map_err(|_| SafeCommandError::change_failed(false))?;
    }
    authorize_codex_root(&root)?;
    let request = mutation_request(Some(codex));
    preflight_codex_session_migration(&request, MigrationProviderTarget::Custom)
        .map_err(|error| map_migration_error(error.kind, error.retryable))?;
    codex_migration_ids(&request).map_err(|error| map_migration_error(error.kind, error.retryable))
}

pub(crate) fn recover_codex_session_storage_since(
    known_ids: &[String],
) -> Result<Option<bool>, SafeCommandError> {
    let request = mutation_request(None);
    recover_codex_migration_since(&request, known_ids)
        .map(|outcome| outcome.map(|value| value == MigrationOutcome::Committed))
        .map_err(|error| map_migration_error(error.kind, error.retryable))
}

pub(crate) fn recover_codex_session_storage() -> Result<(), SafeCommandError> {
    if !codex_home().exists() {
        return Ok(());
    }
    recover_codex_session_migrations(&mutation_request(None))
        .map(|_| ())
        .map_err(|error| map_migration_error(error.kind, error.retryable))
}

pub(crate) fn prepare_codex_session_restart(
) -> Result<CodexSessionMutationOutcome, SafeCommandError> {
    let _guard = crate::commands::targets::lock_and_recover_provider_transaction()?;
    let root = codex_home();
    if !root.exists() {
        return Ok(mutation_outcome("unchanged", "当前状态无需调整。", 0, 0, 0));
    }
    let report = scan_codex_sessions(&ScanRequest::new(root))
        .map_err(|_| SafeCommandError::read_failed())?;
    if report.is_blocked() {
        return Err(SafeCommandError::change_failed(false));
    }
    if !restart_needs_normalization(report.config.active_provider.as_deref()) {
        return Ok(mutation_outcome("unchanged", "当前状态无需调整。", 0, 0, 0));
    }
    normalize_codex_session_storage_inner("custom".to_owned())
}

#[tauri::command]
pub async fn normalize_codex_session_storage(
    target_provider: String,
) -> Result<CodexSessionMutationOutcome, SafeCommandError> {
    let _guard = crate::commands::targets::lock_and_recover_provider_transaction()?;
    normalize_codex_session_storage_inner(target_provider)
}

#[tauri::command]
pub async fn normalize_codex_session_storage_selected(
    target_provider: String,
    thread_ids: Vec<String>,
) -> Result<CodexSessionMutationOutcome, SafeCommandError> {
    let _guard = crate::commands::targets::lock_and_recover_provider_transaction()?;
    normalize_codex_session_storage_selected_inner(target_provider, thread_ids)
}

fn open_url(url: &str) -> Result<(), ()> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .status()
            .ok()
            .filter(|status| status.success())
            .map(|_| ())
            .ok_or(())
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()
            .ok()
            .filter(|status| status.success())
            .map(|_| ())
            .ok_or(())
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        Command::new("xdg-open")
            .arg(url)
            .status()
            .ok()
            .filter(|status| status.success())
            .map(|_| ())
            .ok_or(())
    }
}

#[tauri::command]
pub async fn open_codex_thread(thread_id: String) -> Result<(), SafeCommandError> {
    let thread_id = thread_id.trim();
    open_codex_thread_checked(thread_id)
}

fn open_codex_thread_checked(thread_id: &str) -> Result<(), SafeCommandError> {
    if !is_uuid_like(thread_id) {
        return Err(SafeCommandError::invalid_request());
    }
    let report = scan_codex_sessions(&ScanRequest::new(codex_home()))
        .map_err(|_| SafeCommandError::read_failed())?;
    let thread = report
        .threads
        .iter()
        .find(|thread| thread.thread_id == thread_id)
        .ok_or_else(SafeCommandError::invalid_request)?;
    if !thread_is_healthy(&report, thread) {
        return Err(SafeCommandError::change_failed(false));
    }
    open_url(&format!("codex://threads/{thread_id}")).map_err(|_| SafeCommandError::open_failed())
}

#[cfg(test)]
mod tests {
    use crate::codex_sessions::Diagnostic;
    use super::*;

    #[test]
    fn uuid_validation_is_strict() {
        assert!(is_uuid_like("019fb1b4-f24c-7ec3-a736-c68cf9a0ae11"));
        assert!(!is_uuid_like("../../auth.json"));
    }

    #[test]
    fn selected_thread_ids_are_bounded_and_deduplicated() {
        let id = "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11".to_owned();
        assert_eq!(normalize_selected_thread_ids(vec![id.clone(), id]).unwrap().len(), 1);
        assert!(normalize_selected_thread_ids(vec!["not-a-uuid".to_owned()]).is_err());
        assert!(normalize_selected_thread_ids(Vec::new()).is_err());
    }

    #[test]
    fn restart_keeps_official_route_unchanged() {
        assert!(!restart_needs_normalization(Some("openai")));
        assert!(restart_needs_normalization(Some("custom")));
        assert!(restart_needs_normalization(Some("momotoken")));
    }

    #[test]
    fn continuation_ignores_historical_provider_route_mismatch() {
        let custom = vec!["custom".to_owned()];
        let official = vec!["openai".to_owned()];
        assert!(thread_route_is_healthy(Some(false), &custom));
        assert!(thread_route_is_healthy(Some(false), &official));
        assert!(!thread_route_is_healthy(Some(true), &custom));
        assert!(!thread_route_is_healthy(Some(false), &[]));
    }

    #[test]
    fn blocker_mapping_is_specific_and_safe() {
        let blocker = blocker_copy(
            "thread_provider_mismatch",
            Some("项目规划"),
            Some("019fb1b4-f24c-7ec3-a736-c68cf9a0ae11"),
        );
        assert_eq!(blocker.title, "项目规划");
        assert_eq!(blocker.thread_id, "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11");
        assert_eq!(blocker.reason, "会话的模型服务记录不一致。");
        assert!(blocker.next_step.contains("关闭 ChatGPT"));

        let unsafe_blocker = blocker_copy(
            "rollout_header_invalid",
            Some("/Users/example/.codex/config.toml"),
            Some("../../auth.json"),
        );
        assert_eq!(unsafe_blocker.title, "未命名会话");
        assert_eq!(unsafe_blocker.thread_id, "无法确认");
        assert!(!serde_json::to_string(&unsafe_blocker)
            .unwrap()
            .contains("config.toml"));
    }

    #[test]
    fn display_title_prefers_title_then_safe_summary_then_unnamed() {
        let mut thread = ThreadInventory {
            thread_id: "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11".into(),
            rollout_paths: Vec::new(),
            state_databases: Vec::new(),
            history_databases: Vec::new(),
            providers: Vec::new(),
            workspaces: Vec::new(),
            archived: None,
            title: Some("数据库会话标题".into()),
            summary: Some("安全摘要".into()),
            updated_at_ms: None,
            storage_versions: Vec::new(),
        };
        assert_eq!(display_session_title(&thread), "数据库会话标题");

        thread.title = Some("一个很长的会话标题 ".repeat(20));
        let long_title = display_session_title(&thread);
        assert!(long_title.ends_with('…'));
        assert!(long_title.chars().count() <= 120);

        thread.title = Some("修复 error token 预算".into());
        assert_eq!(display_session_title(&thread), "修复 error token 预算");

        thread.title = None;
        assert_eq!(display_session_title(&thread), "安全摘要");

        thread.summary = Some("/Users/example/.codex/config.toml".into());
        assert_eq!(display_session_title(&thread), "未命名会话");
    }

    #[test]
    fn page_maps_thread_blockers_without_returning_raw_diagnostics() {
        let mut report = ScanReport {
            codex_home: PathBuf::from("/tmp/codex"),
            config: crate::codex_sessions::ConfigInventory {
                path: PathBuf::from("/tmp/codex/config.toml"),
                present: true,
                active_provider: Some("custom".into()),
                defined_providers: vec!["custom".into()],
                effective_sqlite_home: PathBuf::from("/tmp/codex"),
                sqlite_home_source: crate::codex_sessions::SqliteHomeSource::CodexHome,
            },
            rollouts: Vec::new(),
            session_index: None,
            sqlite_databases: Vec::new(),
            threads: vec![ThreadInventory {
                thread_id: "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11".into(),
                rollout_paths: vec![PathBuf::from("rollout")],
                state_databases: vec![PathBuf::from("state")],
                history_databases: Vec::new(),
                providers: vec!["custom".into()],
                workspaces: vec![PathBuf::from("workspace")],
                archived: Some(false),
                title: Some("项目规划".into()),
                summary: Some("正文不能返回".into()),
                updated_at_ms: Some(1),
                storage_versions: Vec::new(),
            }],
            provider_layout: crate::codex_sessions::ProviderLayout::NikoMomotoken,
            diagnostics: vec![Diagnostic {
                level: DiagnosticLevel::Blocker,
                code: "thread_provider_mismatch",
                message: "raw path /Users/example/.codex/config.toml token sk-secret".into(),
                path: None,
                thread_id: Some("019fb1b4-f24c-7ec3-a736-c68cf9a0ae11".into()),
            }],
            normalization: crate::codex_sessions::NormalizationPlan {
                status: NormalizationStatus::Blocked,
                target_provider: "custom".into(),
                actions: Vec::new(),
            },
        };
        let mut second = report.threads[0].clone();
        second.thread_id = "019fb1b4-f24c-7ec3-a736-c68cf9a0ae10".into();
        second.title = Some("第二会话".into());
        second.updated_at_ms = Some(0);
        report.threads.push(second);
        report.diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Blocker,
            code: "thread_archive_mismatch",
            message: "second raw diagnostic".into(),
            path: None,
            thread_id: Some("019fb1b4-f24c-7ec3-a736-c68cf9a0ae10".into()),
        });

        let page = page_from_report(&report, "", 1, 1).unwrap();
        assert_eq!(page.items[0].title, Some("项目规划".into()));
        assert_eq!(page.items[0].blockers.len(), 1);
        assert_eq!(page.items[0].blockers[0].title, "项目规划");
        assert_eq!(page.items[0].blockers[0].thread_id, "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11");
        assert_eq!(page.items[0].summary, None);
        assert!(!page.blockers.iter().any(|blocker| {
            blocker.title == "第二会话"
                && blocker.thread_id == "019fb1b4-f24c-7ec3-a736-c68cf9a0ae10"
        }));
        let second_page = page_from_report(&report, "", 2, 1).unwrap();
        assert_eq!(second_page.items[0].title, Some("第二会话".into()));
        assert!(second_page.blockers.iter().any(|blocker| {
            blocker.title == "第二会话"
                && blocker.thread_id == "019fb1b4-f24c-7ec3-a736-c68cf9a0ae10"
        }));
        assert!(!serde_json::to_string(&page).unwrap().contains("/Users"));
        assert!(!serde_json::to_string(&page).unwrap().contains("sk-secret"));
        report.threads[0].title = Some("/Users/example/.codex/config.toml".into());
        report.threads[0].summary = None;
        let unsafe_page = page_from_report(&report, "", 1, 1).unwrap();
        assert_eq!(unsafe_page.items[0].title, Some("未命名会话".into()));
        assert_eq!(unsafe_page.items[0].blockers[0].title, "未命名会话");
    }

    #[test]
    fn paginated_report_has_total_and_rejects_out_of_range_pages() {
        let report = ScanReport {
            codex_home: PathBuf::from("/tmp/codex"),
            config: crate::codex_sessions::ConfigInventory {
                path: PathBuf::from("/tmp/codex/config.toml"),
                present: true,
                active_provider: Some("custom".into()),
                defined_providers: Vec::new(),
                effective_sqlite_home: PathBuf::from("/tmp/codex"),
                sqlite_home_source: crate::codex_sessions::SqliteHomeSource::CodexHome,
            },
            rollouts: Vec::new(),
            session_index: None,
            sqlite_databases: Vec::new(),
            threads: (0..PAGE_MAX + 1)
                .map(|index| ThreadInventory {
                    thread_id: format!("019fb1b4-f24c-7ec3-a736-c68cf9a0ae{index:02x}"),
                    rollout_paths: vec![PathBuf::from("rollout")],
                    state_databases: vec![PathBuf::from("state")],
                    history_databases: Vec::new(),
                    providers: vec!["custom".into()],
                    workspaces: vec![PathBuf::from("workspace")],
                    archived: Some(false),
                    title: Some(format!("title-{index}")),
                    summary: Some(format!("summary-{index}")),
                    updated_at_ms: Some(index as i64),
                    storage_versions: Vec::new(),
                })
                .collect(),
            provider_layout: crate::codex_sessions::ProviderLayout::Mixed,
            diagnostics: Vec::new(),
            normalization: crate::codex_sessions::NormalizationPlan {
                status: NormalizationStatus::NoChanges,
                target_provider: "custom".into(),
                actions: Vec::new(),
            },
        };
        let first = page_from_report(&report, "", 1, PAGE_MAX).unwrap();
        assert_eq!(first.page, 1);
        assert_eq!(first.page_size, PAGE_MAX);
        assert_eq!(first.total, PAGE_MAX + 1);
        assert_eq!(first.total_pages, 2);
        assert_eq!(first.items.len(), PAGE_MAX);
        assert_eq!(first.items.first().and_then(|item| item.updated_at.as_deref()), Some("50"));
        assert!(first.items.iter().all(|item| item.can_continue));
        assert!(page_from_report(&report, "", 3, PAGE_MAX).is_err());

        let mut mismatch_report = report.clone();
        mismatch_report.threads[PAGE_MAX].providers = vec!["custom".into(), "openai".into()];
        let mismatch_page = page_from_report(&mismatch_report, "", 1, PAGE_MAX).unwrap();
        assert!(mismatch_page.items.iter().any(|item| !item.can_continue));

        let mut official_report = mismatch_report.clone();
        official_report.threads[PAGE_MAX].providers = vec!["openai".into()];
        let official_page = page_from_report(&official_report, "", 1, PAGE_MAX).unwrap();
        assert!(official_page.items.iter().any(|item| item.needs_migration));

        let mut empty_report = report;
        empty_report.threads.clear();
        assert!(page_from_report(&empty_report, "", 1, PAGE_MAX).is_ok());
        assert!(page_from_report(&empty_report, "", 2, PAGE_MAX).is_err());
    }
}
