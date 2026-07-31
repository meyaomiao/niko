use crate::codex_sessions::{
    migrate_codex_sessions_transactional, scan_codex_sessions, CodexProcessPolicy,
    MigrationErrorKind, MigrationOptions, MigrationProviderTarget, MigrationRequest,
    NormalizationStatus, ProviderLayout, ScanReport, ScanRequest, MIGRATION_ROOT_MARKER,
    MIGRATION_ROOT_MARKER_CONTENT,
};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Serialize)]
pub struct CodexSessionDiagnostic {
    pub level: String,
    pub code: String,
    pub message: String,
    pub path: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CodexSessionThread {
    pub thread_id: String,
    pub providers: Vec<String>,
    pub workspaces: Vec<String>,
    pub archived: Option<bool>,
    pub rollout_count: usize,
}

#[derive(Debug, Serialize)]
pub struct CodexSessionInventory {
    pub codex_home: String,
    pub active_provider: Option<String>,
    pub defined_providers: Vec<String>,
    pub provider_layout: String,
    pub layout_hint: String,
    pub normalization_status: String,
    pub normalization_target_provider: String,
    pub session_index_entries: Option<usize>,
    pub thread_count: usize,
    pub archived_thread_count: usize,
    pub diagnostics: Vec<CodexSessionDiagnostic>,
    pub threads: Vec<CodexSessionThread>,
}

#[derive(Debug, Serialize)]
pub struct CodexSessionMutationOutcome {
    pub ok: bool,
    pub target_provider: String,
    pub changed_artifacts: usize,
    pub restart_allowed: bool,
    pub retryable: bool,
    pub message: String,
}

fn home_dir() -> PathBuf {
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

fn codex_home() -> PathBuf {
    home_dir().join(".codex")
}

fn approve_scan_request() -> ScanRequest {
    ScanRequest::new(codex_home())
}

fn provider_layout_label(layout: ProviderLayout) -> &'static str {
    match layout {
        ProviderLayout::Empty => "empty",
        ProviderLayout::Official => "official",
        ProviderLayout::CcSwitchCustom => "custom",
        ProviderLayout::NikoMomotoken => "legacy",
        ProviderLayout::CodexPlusPlusCompatible => "compatible",
        ProviderLayout::Mixed => "mixed",
    }
}

fn normalization_status_label(status: NormalizationStatus) -> &'static str {
    match status {
        NormalizationStatus::NoChanges => "no_changes",
        NormalizationStatus::WouldNormalize => "needs_check",
        NormalizationStatus::Blocked => "blocked",
    }
}

fn friendly_layout_hint(layout: ProviderLayout) -> &'static str {
    match layout {
        ProviderLayout::Empty => "还没有本地会话",
        ProviderLayout::Official => "当前本地会话已回到官方状态",
        ProviderLayout::CcSwitchCustom => "当前本地会话已接入 Niko",
        ProviderLayout::NikoMomotoken => "发现旧版 Niko 本地会话",
        ProviderLayout::CodexPlusPlusCompatible => "发现兼容的本地会话",
        ProviderLayout::Mixed => "本地会话状态有点混杂，建议重新检查",
    }
}

fn normalize_target(target: &str) -> Result<MigrationProviderTarget, String> {
    match target {
        "custom" => Ok(MigrationProviderTarget::Custom),
        "openai" | "official" => Ok(MigrationProviderTarget::OpenAi),
        other => Err(format!("未知目标: {other}")),
    }
}

fn friendly_mutation_message(ok: bool, target: &str, changed_artifacts: usize) -> String {
    if ok {
        match target {
            "custom" => {
                if changed_artifacts == 0 {
                    "本地会话已经是当前状态".to_owned()
                } else {
                    "已整理本地会话，可以继续使用".to_owned()
                }
            }
            _ => {
                if changed_artifacts == 0 {
                    "本地会话已经是官方状态".to_owned()
                } else {
                    "已恢复到官方，本地会话可以继续查看".to_owned()
                }
            }
        }
    } else {
        String::new()
    }
}

fn is_uuid_like(thread_id: &str) -> bool {
    thread_id.len() == 36
        && thread_id
            .chars()
            .enumerate()
            .all(|(index, ch)| match index {
                8 | 13 | 18 | 23 => ch == '-',
                _ => ch.is_ascii_hexdigit(),
            })
}

fn friendly_error_message(kind: MigrationErrorKind) -> (String, bool) {
    match kind {
        MigrationErrorKind::InvalidRequest => {
            ("本地会话整理请求无效，请重新检查后再试。".to_owned(), false)
        }
        MigrationErrorKind::RootNotAuthorized => {
            ("本地会话目前只读，暂时不能整理。".to_owned(), false)
        }
        MigrationErrorKind::ScanBlocked | MigrationErrorKind::RecoveryRequired => (
            "本地会话状态还没准备好，请重新检查后再试。".to_owned(),
            true,
        ),
        MigrationErrorKind::UnknownSchema | MigrationErrorKind::CorruptStorage => (
            "本地会话文件暂时无法整理，请先查看或稍后再试。".to_owned(),
            false,
        ),
        MigrationErrorKind::NikoLocked
        | MigrationErrorKind::NikoLockUnverifiable
        | MigrationErrorKind::ProviderSyncLocked => {
            ("另一个整理任务正在进行，请稍后再试。".to_owned(), true)
        }
        MigrationErrorKind::CodexRunning
        | MigrationErrorKind::FileOccupied
        | MigrationErrorKind::SqliteBusy => (
            "Codex 正在占用本地会话文件，请先关闭 Codex 后重试，原配置和数据保持可用。".to_owned(),
            true,
        ),
        MigrationErrorKind::PermissionDenied => (
            "没有权限变更本地会话，请检查本机文件权限。".to_owned(),
            false,
        ),
        MigrationErrorKind::InsufficientSpace => {
            ("磁盘空间不足，请先腾出一些空间再试。".to_owned(), true)
        }
        MigrationErrorKind::SourceChanged
        | MigrationErrorKind::BackupHashMismatch
        | MigrationErrorKind::ValidationFailed
        | MigrationErrorKind::JournalCorrupt
        | MigrationErrorKind::InjectedCrash => (
            "本地会话在整理期间发生了变化，请重新检查后再试。".to_owned(),
            true,
        ),
        MigrationErrorKind::Io => ("本地会话暂时无法整理，请稍后重试。".to_owned(), true),
    }
}

fn mutation_request() -> MigrationRequest {
    let request = approve_scan_request();
    MigrationRequest {
        scan: request,
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

fn report_is_current(report: &ScanReport, target: MigrationProviderTarget) -> bool {
    let target = match target {
        MigrationProviderTarget::Custom => "custom",
        MigrationProviderTarget::OpenAi => "openai",
    };
    report.config.active_provider.as_deref() == Some(target)
        && report
            .rollouts
            .iter()
            .all(|rollout| rollout.provider == target)
        && report
            .sqlite_databases
            .iter()
            .flat_map(|database| database.state_rows.iter())
            .all(|row| row.provider == target)
}

fn authorize_codex_root(codex_home: &PathBuf) -> Result<(), MigrationErrorKind> {
    let marker = codex_home.join(MIGRATION_ROOT_MARKER);
    match fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let contents =
                fs::read_to_string(marker).map_err(|_| MigrationErrorKind::RootNotAuthorized)?;
            if contents == MIGRATION_ROOT_MARKER_CONTENT {
                Ok(())
            } else {
                Err(MigrationErrorKind::RootNotAuthorized)
            }
        }
        Ok(_) => Err(MigrationErrorKind::RootNotAuthorized),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(marker)
                .map_err(|_| MigrationErrorKind::RootNotAuthorized)?;
            file.write_all(MIGRATION_ROOT_MARKER_CONTENT.as_bytes())
                .map_err(|_| MigrationErrorKind::RootNotAuthorized)?;
            file.sync_all()
                .map_err(|_| MigrationErrorKind::RootNotAuthorized)
        }
        Err(_) => Err(MigrationErrorKind::RootNotAuthorized),
    }
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .status()
            .map_err(|error| format!("打开失败：{error}"))?
            .success()
            .then_some(())
            .ok_or_else(|| "打开失败，请确认 ChatGPT 桌面端已安装".to_owned())
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()
            .map_err(|error| format!("打开失败：{error}"))?
            .success()
            .then_some(())
            .ok_or_else(|| "打开失败，请确认 ChatGPT 桌面端已安装".to_owned())
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        Command::new("xdg-open")
            .arg(url)
            .status()
            .map_err(|error| format!("打开失败：{error}"))?
            .success()
            .then_some(())
            .ok_or_else(|| "当前平台不支持打开本地 thread".to_owned())
    }
}

fn empty_inventory(codex_home: PathBuf) -> CodexSessionInventory {
    CodexSessionInventory {
        codex_home: codex_home.display().to_string(),
        active_provider: None,
        defined_providers: Vec::new(),
        provider_layout: provider_layout_label(ProviderLayout::Empty).to_owned(),
        layout_hint: friendly_layout_hint(ProviderLayout::Empty).to_owned(),
        normalization_status: normalization_status_label(NormalizationStatus::NoChanges).to_owned(),
        normalization_target_provider: "custom".to_owned(),
        session_index_entries: None,
        thread_count: 0,
        archived_thread_count: 0,
        diagnostics: Vec::new(),
        threads: Vec::new(),
    }
}

fn to_inventory(report: crate::codex_sessions::ScanReport) -> CodexSessionInventory {
    let archived_thread_count = report
        .threads
        .iter()
        .filter(|thread| thread.archived == Some(true))
        .count();
    CodexSessionInventory {
        codex_home: report.codex_home.display().to_string(),
        active_provider: report.config.active_provider.clone(),
        defined_providers: report.config.defined_providers.clone(),
        provider_layout: provider_layout_label(report.provider_layout).to_owned(),
        layout_hint: friendly_layout_hint(report.provider_layout).to_owned(),
        normalization_status: normalization_status_label(report.normalization.status).to_owned(),
        normalization_target_provider: report.normalization.target_provider,
        session_index_entries: report.session_index.as_ref().map(|index| index.entry_count),
        thread_count: report.threads.len(),
        archived_thread_count,
        diagnostics: report
            .diagnostics
            .into_iter()
            .map(|diagnostic| CodexSessionDiagnostic {
                level: match diagnostic.level {
                    crate::codex_sessions::DiagnosticLevel::Warning => "warning".to_owned(),
                    crate::codex_sessions::DiagnosticLevel::Blocker => "blocker".to_owned(),
                },
                code: diagnostic.code.to_owned(),
                message: diagnostic.message,
                path: diagnostic.path.map(|path| path.display().to_string()),
                thread_id: diagnostic.thread_id,
            })
            .collect(),
        threads: report
            .threads
            .into_iter()
            .map(|thread| CodexSessionThread {
                thread_id: thread.thread_id,
                providers: thread.providers,
                workspaces: thread
                    .workspaces
                    .into_iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                archived: thread.archived,
                rollout_count: thread.rollout_paths.len(),
            })
            .collect(),
    }
}

#[tauri::command]
pub async fn scan_codex_session_inventory() -> Result<CodexSessionInventory, String> {
    let codex_home = codex_home();
    if !codex_home.exists() {
        return Ok(empty_inventory(codex_home));
    }
    if !codex_home.is_dir() {
        return Err("本地会话目录暂时不可用，请稍后再试。".to_owned());
    }
    let report = scan_codex_sessions(&ScanRequest::new(&codex_home))
        .map_err(|_| "本地会话暂时无法读取，请稍后再试。".to_owned())?;
    Ok(to_inventory(report))
}

pub(crate) fn normalize_codex_session_storage_inner(
    target_provider: String,
) -> CodexSessionMutationOutcome {
    let target = match normalize_target(&target_provider) {
        Ok(target) => target,
        Err(message) => {
            return CodexSessionMutationOutcome {
                ok: false,
                target_provider,
                changed_artifacts: 0,
                restart_allowed: false,
                retryable: false,
                message,
            }
        }
    };

    let codex_home = codex_home();
    if !codex_home.exists() {
        let message = friendly_mutation_message(true, &target_provider, 0);
        return CodexSessionMutationOutcome {
            ok: true,
            target_provider,
            changed_artifacts: 0,
            restart_allowed: true,
            retryable: false,
            message,
        };
    }

    let scan = match scan_codex_sessions(&ScanRequest::new(&codex_home)) {
        Ok(report) => report,
        Err(_) => {
            return CodexSessionMutationOutcome {
                ok: false,
                target_provider,
                changed_artifacts: 0,
                restart_allowed: true,
                retryable: true,
                message: "本地会话暂时无法读取，请稍后重试。".to_owned(),
            }
        }
    };
    if !scan.is_blocked() && report_is_current(&scan, target) {
        let message = friendly_mutation_message(true, &target_provider, 0);
        return CodexSessionMutationOutcome {
            ok: true,
            target_provider,
            changed_artifacts: 0,
            restart_allowed: true,
            retryable: false,
            message,
        };
    }
    if scan.is_blocked() {
        let (message, retryable) = friendly_error_message(MigrationErrorKind::ScanBlocked);
        return CodexSessionMutationOutcome {
            ok: false,
            target_provider,
            changed_artifacts: 0,
            restart_allowed: true,
            retryable,
            message,
        };
    }
    if let Err(kind) = authorize_codex_root(&codex_home) {
        let (message, retryable) = friendly_error_message(kind);
        return CodexSessionMutationOutcome {
            ok: false,
            target_provider,
            changed_artifacts: 0,
            restart_allowed: true,
            retryable,
            message,
        };
    }

    match migrate_codex_sessions_transactional(&mutation_request(), target) {
        Ok(report) => {
            let message =
                friendly_mutation_message(true, &target_provider, report.changed_artifacts);
            CodexSessionMutationOutcome {
                ok: true,
                target_provider,
                changed_artifacts: report.changed_artifacts,
                restart_allowed: report.restart_allowed,
                retryable: false,
                message,
            }
        }
        Err(error) => {
            let (message, retryable) = friendly_error_message(error.kind);
            CodexSessionMutationOutcome {
                ok: false,
                target_provider,
                changed_artifacts: 0,
                restart_allowed: error.restart_allowed,
                retryable: retryable || error.retryable,
                message,
            }
        }
    }
}

pub(crate) fn prepare_codex_session_restart() -> Result<(), String> {
    let codex_home = codex_home();
    if !codex_home.exists() {
        return Ok(());
    }
    let report = scan_codex_sessions(&ScanRequest::new(codex_home))
        .map_err(|_| "本地会话暂时无法读取，请稍后重试。".to_owned())?;
    if report.is_blocked() {
        return Err(friendly_error_message(MigrationErrorKind::ScanBlocked).0);
    }
    if report.config.active_provider.as_deref() == Some("openai") {
        return Ok(());
    }
    let outcome = normalize_codex_session_storage_inner("custom".to_owned());
    outcome.ok.then_some(()).ok_or(outcome.message)
}

#[tauri::command]
pub async fn normalize_codex_session_storage(
    target_provider: String,
) -> CodexSessionMutationOutcome {
    normalize_codex_session_storage_inner(target_provider)
}

#[tauri::command]
pub async fn open_codex_thread(thread_id: String) -> Result<(), String> {
    let thread_id = thread_id.trim();
    if !is_uuid_like(thread_id) {
        return Err("线程编号无效".to_owned());
    }
    open_url(&format!("codex://threads/{thread_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_sessions::{
        ConfigInventory, Diagnostic, DiagnosticLevel, NormalizationPlan, ProviderLayout, ScanReport,
    };

    #[test]
    fn inventory_conversion_keeps_thread_and_layout_summary() {
        let report = ScanReport {
            codex_home: PathBuf::from("/tmp/.codex"),
            config: ConfigInventory {
                path: PathBuf::from("/tmp/.codex/config.toml"),
                present: true,
                active_provider: Some("openai".to_owned()),
                defined_providers: vec!["openai".to_owned()],
                effective_sqlite_home: PathBuf::from("/tmp/.codex"),
                sqlite_home_source: crate::codex_sessions::SqliteHomeSource::CodexHome,
            },
            rollouts: Vec::new(),
            session_index: None,
            sqlite_databases: Vec::new(),
            threads: vec![crate::codex_sessions::ThreadInventory {
                thread_id: "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11".to_owned(),
                rollout_paths: vec![PathBuf::from("/tmp/.codex/sessions/a.jsonl")],
                state_databases: Vec::new(),
                history_databases: Vec::new(),
                providers: vec!["openai".to_owned()],
                workspaces: vec![PathBuf::from("/workspace")],
                archived: Some(false),
                storage_versions: vec!["rollout:jsonl".to_owned()],
            }],
            provider_layout: ProviderLayout::Official,
            diagnostics: vec![Diagnostic {
                level: DiagnosticLevel::Warning,
                code: "warning",
                message: "ok".to_owned(),
                path: None,
                thread_id: None,
            }],
            normalization: NormalizationPlan {
                status: NormalizationStatus::NoChanges,
                target_provider: "custom".to_owned(),
                actions: Vec::new(),
            },
        };

        let view = to_inventory(report);
        assert_eq!(view.provider_layout, "official");
        assert_eq!(view.normalization_status, "no_changes");
        assert_eq!(view.thread_count, 1);
        assert_eq!(
            view.threads[0].thread_id,
            "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11"
        );
        assert_eq!(view.threads[0].providers, vec!["openai"]);
        assert_eq!(view.threads[0].workspaces, vec!["/workspace"]);
    }

    #[test]
    fn friendly_error_map_keeps_user_language_plain() {
        let (message, retryable) = friendly_error_message(MigrationErrorKind::FileOccupied);
        assert!(retryable);
        assert!(message.contains("关闭 Codex"));
    }

    #[test]
    fn uuid_like_validation_accepts_thread_ids() {
        assert!(is_uuid_like("019fb1b4-f24c-7ec3-a736-c68cf9a0ae11"));
        assert!(!is_uuid_like("not-a-thread"));
    }
}
