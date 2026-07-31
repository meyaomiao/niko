use crate::codex_sessions::{
    codex_migration_ids, migrate_codex_sessions_transactional, preflight_codex_session_migration,
    recover_codex_migration_since, recover_codex_session_migrations, scan_codex_sessions,
    CodexMigrationInput, CodexProcessPolicy, DiagnosticLevel, MigrationErrorKind, MigrationOptions,
    MigrationOutcome, MigrationProviderTarget, MigrationRequest, NormalizationStatus, ScanReport,
    ScanRequest, ThreadInventory, MIGRATION_ROOT_MARKER, MIGRATION_ROOT_MARKER_CONTENT,
};
use crate::commands::safe_error::SafeCommandError;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const QUERY_MAX: usize = 80;
const PAGE_MAX: usize = 20;
const CURSOR_MASK: usize = 0x5a17_3c2d;

#[derive(Debug, Serialize, PartialEq)]
pub struct CodexSessionThread {
    pub thread_id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub updated_at: Option<String>,
    pub archived: bool,
    pub can_continue: bool,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CodexSessionPage {
    pub status: &'static str,
    pub items: Vec<CodexSessionThread>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CodexSessionMutationOutcome {
    pub status: &'static str,
    pub message: &'static str,
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

fn thread_route_is_healthy(
    active_provider: Option<&str>,
    archived: Option<bool>,
    providers: &[String],
) -> bool {
    archived == Some(false)
        && providers.len() == 1
        && active_provider == providers.first().map(String::as_str)
}

fn thread_is_healthy(report: &ScanReport, thread: &ThreadInventory) -> bool {
    is_uuid_like(&thread.thread_id)
        && thread.rollout_paths.len() == 1
        && thread.state_databases.len() == 1
        && thread.history_databases.len() <= 1
        && thread_route_is_healthy(
            report.config.active_provider.as_deref(),
            thread.archived,
            &thread.providers,
        )
        && !report.diagnostics.iter().any(|diagnostic| {
            diagnostic.level == DiagnosticLevel::Blocker
                && (diagnostic.thread_id.is_none()
                    || diagnostic.thread_id.as_deref() == Some(&thread.thread_id))
        })
}

fn encode_cursor(offset: usize) -> String {
    format!("p1_{:08x}", offset ^ CURSOR_MASK)
}

fn decode_cursor(cursor: Option<&str>) -> Result<usize, SafeCommandError> {
    let Some(cursor) = cursor else { return Ok(0) };
    if cursor.len() != 11 || !cursor.starts_with("p1_") {
        return Err(SafeCommandError::invalid_request());
    }
    usize::from_str_radix(&cursor[3..], 16)
        .map(|value| value ^ CURSOR_MASK)
        .map_err(|_| SafeCommandError::invalid_request())
}

fn page_from_report(
    report: &ScanReport,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<CodexSessionPage, SafeCommandError> {
    let query = query.trim().to_lowercase();
    let mut threads = report
        .threads
        .iter()
        .filter(|thread| query.is_empty() || thread.thread_id.to_lowercase().contains(&query))
        .collect::<Vec<_>>();
    threads.sort_by(|left, right| right.thread_id.cmp(&left.thread_id));
    if offset > threads.len() {
        return Err(SafeCommandError::invalid_request());
    }
    let end = offset.saturating_add(limit).min(threads.len());
    let items = threads[offset..end]
        .iter()
        .map(|thread| CodexSessionThread {
            thread_id: thread.thread_id.clone(),
            title: None,
            summary: None,
            updated_at: None,
            archived: thread.archived.unwrap_or(false),
            can_continue: thread_is_healthy(report, thread),
        })
        .collect();
    Ok(CodexSessionPage {
        status: report_status(report),
        items,
        next_cursor: (end < threads.len()).then(|| encode_cursor(end)),
    })
}

#[tauri::command]
pub async fn scan_codex_session_inventory(
    query: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
) -> Result<CodexSessionPage, SafeCommandError> {
    let query = query.unwrap_or_default();
    if query.chars().count() > QUERY_MAX {
        return Err(SafeCommandError::invalid_request());
    }
    let offset = decode_cursor(cursor.as_deref())?;
    let limit = limit.unwrap_or(PAGE_MAX).clamp(1, PAGE_MAX);
    let root = codex_home();
    if !root.exists() {
        return Ok(CodexSessionPage {
            status: "healthy",
            items: Vec::new(),
            next_cursor: None,
        });
    }
    if !root.is_dir() {
        return Err(SafeCommandError::read_failed());
    }
    let report = scan_codex_sessions(&ScanRequest::new(root))
        .map_err(|_| SafeCommandError::read_failed())?;
    page_from_report(&report, &query, offset, limit)
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
            return Ok(CodexSessionMutationOutcome {
                status: "unchanged",
                message: "当前状态无需调整。",
            });
        }
        fs::create_dir_all(&root).map_err(|_| SafeCommandError::change_failed(false))?;
    }
    let scan = scan_codex_sessions(&ScanRequest::new(&root))
        .map_err(|_| SafeCommandError::read_failed())?;
    if scan.is_blocked() {
        return Err(SafeCommandError::change_failed(false));
    }
    authorize_codex_root(&root)?;
    match migrate_codex_sessions_transactional(&mutation_request(codex), target) {
        Ok(report) if report.changed_artifacts == 0 => Ok(CodexSessionMutationOutcome {
            status: "unchanged",
            message: "当前状态无需调整。",
        }),
        Ok(_) if target == MigrationProviderTarget::OpenAi => Ok(CodexSessionMutationOutcome {
            status: "applied",
            message: "已恢复到官方，可以继续使用。",
        }),
        Ok(_) => Ok(CodexSessionMutationOutcome {
            status: "applied",
            message: "已完成检查，可以继续使用。",
        }),
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
        return Ok(CodexSessionMutationOutcome {
            status: "unchanged",
            message: "当前状态无需调整。",
        });
    }
    let report = scan_codex_sessions(&ScanRequest::new(root))
        .map_err(|_| SafeCommandError::read_failed())?;
    if report.is_blocked() {
        return Err(SafeCommandError::change_failed(false));
    }
    if !restart_needs_normalization(report.config.active_provider.as_deref()) {
        return Ok(CodexSessionMutationOutcome {
            status: "unchanged",
            message: "当前状态无需调整。",
        });
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
    use super::*;
    #[test]
    fn cursor_is_bounded_and_roundtrips() {
        for offset in [0, 1, 20, 4000] {
            let value = encode_cursor(offset);
            assert!(value.len() <= 16);
            assert_eq!(decode_cursor(Some(&value)).unwrap(), offset);
        }
        assert!(decode_cursor(Some("20")).is_err());
    }
    #[test]
    fn uuid_validation_is_strict() {
        assert!(is_uuid_like("019fb1b4-f24c-7ec3-a736-c68cf9a0ae11"));
        assert!(!is_uuid_like("../../auth.json"));
    }

    #[test]
    fn restart_keeps_official_route_unchanged() {
        assert!(!restart_needs_normalization(Some("openai")));
        assert!(restart_needs_normalization(Some("custom")));
        assert!(restart_needs_normalization(Some("momotoken")));
    }

    #[test]
    fn continuation_requires_active_non_archived_provider_route() {
        let custom = vec!["custom".to_owned()];
        let official = vec!["openai".to_owned()];
        assert!(thread_route_is_healthy(
            Some("custom"),
            Some(false),
            &custom,
        ));
        assert!(!thread_route_is_healthy(
            Some("custom"),
            Some(true),
            &custom,
        ));
        assert!(!thread_route_is_healthy(
            Some("custom"),
            Some(false),
            &official,
        ));
    }
}
