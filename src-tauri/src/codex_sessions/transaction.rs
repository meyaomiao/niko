//! Crash-safe provider migration for explicitly approved Codex storage roots.
//!
//! This module builds on the E10-1 inventory and E10-2 round-trip proof. It is
//! intentionally a Rust API only: no production Tauri command invokes it yet.

use super::{
    build_fixture_thread_proofs, digest_hex, read_rollout_logical, rewrite_config_provider,
    rewrite_rollout_header_provider, scan_codex_sessions, shift_fixture_proof_offsets,
    FixtureMutationError, FixtureProviderTarget, FixtureThreadProof, RolloutArtifact, ScanReport,
    ScanRequest, CUSTOM_PROVIDER, OFFICIAL_PROVIDER,
};
use rusqlite::backup::{Backup, StepResult};
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sysinfo::{Disks, Pid, ProcessesToUpdate, System};

pub const MIGRATION_ROOT_MARKER: &str = ".niko-e10-3-migration-root";
pub const MIGRATION_ROOT_MARKER_CONTENT: &str = "niko-e10-3-approved-transaction-root\n";

const JOURNAL_FORMAT_VERSION: u32 = 1;
const TRANSACTION_DIRECTORY: &str = ".niko-session-migrations";
const JOURNAL_FILE: &str = "journal.json";
const OWNER_FILE: &str = "owner.json";
const NIKO_LOCK_FILE: &str = "niko-session-migration.lock";
const PROVIDER_SYNC_LOCK_DIRECTORY: &str = "provider-sync.lock";
const DEFAULT_SPACE_RESERVE_BYTES: u64 = 1024 * 1024;

static MIGRATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct MigrationRequest {
    pub scan: ScanRequest,
    pub options: MigrationOptions,
}

impl MigrationRequest {
    pub fn new(scan: ScanRequest) -> Self {
        Self {
            scan,
            options: MigrationOptions::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct MigrationOptions {
    pub retained_transactions: usize,
    pub busy_retries: u32,
    pub busy_retry_delay: Duration,
    pub process_wait_attempts: u32,
    pub process_wait_delay: Duration,
    pub codex_process_policy: CodexProcessPolicy,
    pub space_reserve_bytes: u64,
}

impl Default for MigrationOptions {
    fn default() -> Self {
        Self {
            retained_transactions: 3,
            busy_retries: 4,
            busy_retry_delay: Duration::from_millis(50),
            process_wait_attempts: 20,
            process_wait_delay: Duration::from_millis(250),
            codex_process_policy: CodexProcessPolicy::RequestNormalExit,
            space_reserve_bytes: DEFAULT_SPACE_RESERVE_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexProcessPolicy {
    RequestNormalExit,
    RequireStopped,
    IsolatedFixture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationProviderTarget {
    Custom,
    OpenAi,
}

impl MigrationProviderTarget {
    fn fixture_target(self) -> FixtureProviderTarget {
        match self {
            Self::Custom => FixtureProviderTarget::Custom,
            Self::OpenAi => FixtureProviderTarget::OpenAi,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationState {
    Planned,
    Snapshotted,
    Staged,
    Committing,
    Validating,
    Committed,
    RollingBack,
    RolledBack,
}

impl MigrationState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Committed | Self::RolledBack)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationOutcome {
    AlreadyCurrent,
    Committed,
    RolledBack,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationReport {
    pub migration_id: Option<String>,
    pub outcome: MigrationOutcome,
    pub state: MigrationState,
    pub changed_artifacts: usize,
    pub restart_allowed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryReport {
    pub migrations: Vec<MigrationReport>,
    pub restart_allowed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationErrorKind {
    InvalidRequest,
    RootNotAuthorized,
    ScanBlocked,
    UnknownSchema,
    CorruptStorage,
    NikoLocked,
    NikoLockUnverifiable,
    ProviderSyncLocked,
    CodexRunning,
    SqliteBusy,
    FileOccupied,
    PermissionDenied,
    InsufficientSpace,
    SourceChanged,
    BackupHashMismatch,
    ValidationFailed,
    JournalCorrupt,
    RecoveryRequired,
    InjectedCrash,
    Io,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationError {
    pub kind: MigrationErrorKind,
    pub code: &'static str,
    pub message: &'static str,
    pub artifact_id: Option<String>,
    pub retryable: bool,
    pub restart_allowed: bool,
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for MigrationError {}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FaultPoint {
    PreflightProviderLock,
    PreflightProcess,
    PreflightPermission,
    PreflightSpace,
    PreflightSqliteBusy,
    PlannedPersisted,
    SnapshotArtifact,
    SnapshottedPersisted,
    StageArtifact,
    StagedPersisted,
    CommittingPersisted,
    CommitArtifact,
    ValidatingPersisted,
    Validation,
    CommittedPersisted,
    RollingBackPersisted,
    RollbackArtifact,
    RolledBackPersisted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InjectedFaultKind {
    Crash,
    ProviderSyncLocked,
    CodexRunning,
    PermissionDenied,
    InsufficientSpace,
    SqliteBusy,
    FileOccupied,
    HashMismatch,
    ValidationFailed,
}

pub trait MigrationFaultInjector {
    fn inject(&self, point: FaultPoint, artifact_id: Option<&str>) -> Option<InjectedFaultKind>;
}

#[derive(Default)]
pub struct NoMigrationFaults;

impl MigrationFaultInjector for NoMigrationFaults {
    fn inject(&self, _point: FaultPoint, _artifact_id: Option<&str>) -> Option<InjectedFaultKind> {
        None
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum RootSlot {
    Codex,
    Sqlite,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactKind {
    Config,
    Rollout,
    SessionIndex,
    Sqlite,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ArtifactLocator {
    root: RootSlot,
    relative_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct JournalEntry {
    artifact_id: String,
    locator: ArtifactLocator,
    kind: ArtifactKind,
    mutable: bool,
    existed: bool,
    byte_size: u64,
    source_hash: Option<String>,
    backup_hash: Option<String>,
    staged_hash: Option<String>,
    applied: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct RootBinding {
    slot: RootSlot,
    fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct LockOwner {
    journal_id: String,
    nonce: String,
    pid: u32,
    process_start_time: u64,
    root_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ProviderLockOwner {
    owner_kind: String,
    migration: LockOwner,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ThreadValidationProof {
    thread_id_hash: String,
    visible_event_count: usize,
    visible_history_digest: String,
    provider_neutral_digest: String,
    workspace_hash: Option<String>,
    archived: bool,
    state_database_count: usize,
    history_signature: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct MigrationJournal {
    format_version: u32,
    migration_id: String,
    state: MigrationState,
    target_provider: String,
    source_provider: Option<String>,
    created_at_millis: u128,
    updated_at_millis: u128,
    roots: Vec<RootBinding>,
    owner: LockOwner,
    entries: Vec<JournalEntry>,
    old_proofs: Vec<ThreadValidationProof>,
    new_proofs: Vec<ThreadValidationProof>,
    restart_allowed: bool,
}

#[derive(Clone)]
struct ApprovedRoots {
    codex: PathBuf,
    sqlite: Option<PathBuf>,
}

impl ApprovedRoots {
    fn path(&self, slot: RootSlot) -> Result<&Path, MigrationError> {
        match slot {
            RootSlot::Codex => Ok(&self.codex),
            RootSlot::Sqlite => self.sqlite.as_deref().ok_or_else(|| {
                migration_error(
                    MigrationErrorKind::JournalCorrupt,
                    "journal_root_missing",
                    "the journal references an unapproved storage root",
                    None,
                )
            }),
        }
    }

    fn bindings(&self) -> Vec<RootBinding> {
        let mut bindings = vec![RootBinding {
            slot: RootSlot::Codex,
            fingerprint: path_fingerprint(&self.codex),
        }];
        if let Some(sqlite) = &self.sqlite {
            bindings.push(RootBinding {
                slot: RootSlot::Sqlite,
                fingerprint: path_fingerprint(sqlite),
            });
        }
        bindings
    }

    fn distinct(&self) -> Vec<(RootSlot, &Path)> {
        let mut roots = vec![(RootSlot::Codex, self.codex.as_path())];
        if let Some(sqlite) = &self.sqlite {
            roots.push((RootSlot::Sqlite, sqlite.as_path()));
        }
        roots
    }
}

enum StagePayload {
    Bytes(Vec<u8>),
    Sqlite(SqliteMutation),
    None,
}

struct PlannedEntry {
    journal: JournalEntry,
    absolute_path: PathBuf,
    payload: StagePayload,
}

#[derive(Default)]
struct SqliteMutation {
    state_updates: Vec<StateUpdate>,
    history_updates: Vec<HistoryUpdate>,
}

struct StateUpdate {
    thread_id: String,
    source_provider: String,
    target_provider: String,
}

struct HistoryUpdate {
    thread_id: String,
    source_offset: i64,
    delta: i64,
}

struct MigrationPlan {
    roots: ApprovedRoots,
    entries: Vec<PlannedEntry>,
    old_proofs: Vec<ThreadValidationProof>,
    new_proofs: Vec<ThreadValidationProof>,
    source_provider: Option<String>,
    target_provider: String,
}

pub fn migrate_codex_sessions_transactional(
    request: &MigrationRequest,
    target: MigrationProviderTarget,
) -> Result<MigrationReport, MigrationError> {
    migrate_codex_sessions_transactional_with_faults(request, target, &NoMigrationFaults)
}

pub fn migrate_codex_sessions_transactional_with_faults(
    request: &MigrationRequest,
    target: MigrationProviderTarget,
    faults: &dyn MigrationFaultInjector,
) -> Result<MigrationReport, MigrationError> {
    let roots = approve_roots(&request.scan)?;
    let has_pending = read_journals(&roots)?
        .iter()
        .any(|journal| !journal.state.is_terminal());
    if has_pending {
        let recovered = recover_codex_session_migrations_with_faults(request, faults)?;
        if !recovered.restart_allowed {
            return Err(migration_error(
                MigrationErrorKind::RecoveryRequired,
                "migration_recovery_incomplete",
                "an earlier migration must be recovered before a new migration can start",
                None,
            ));
        }
    }

    let plan = build_migration_plan(request, target)?;
    let changed_artifacts = plan
        .entries
        .iter()
        .filter(|entry| entry.journal.mutable)
        .count();
    if changed_artifacts == 0 {
        return Ok(MigrationReport {
            migration_id: None,
            outcome: MigrationOutcome::AlreadyCurrent,
            state: MigrationState::Committed,
            changed_artifacts: 0,
            restart_allowed: true,
        });
    }

    let journals = read_journals(&plan.roots)?;
    clear_verified_stale_lock(&plan.roots, &journals)?;

    preflight(request, &plan, faults)?;
    execute_plan(request, plan, changed_artifacts, faults)
}

fn build_migration_plan(
    request: &MigrationRequest,
    target: MigrationProviderTarget,
) -> Result<MigrationPlan, MigrationError> {
    let roots = approve_roots(&request.scan)?;
    let report = scan_codex_sessions(&request.scan).map_err(|_| {
        migration_error(
            MigrationErrorKind::InvalidRequest,
            "migration_scan_request_invalid",
            "the explicit Codex storage roots are invalid",
            None,
        )
    })?;
    reject_blocked_scan(&report)?;

    let target_provider = target_provider(target).to_owned();
    let before_proofs = build_fixture_thread_proofs(&report).map_err(redact_fixture_error)?;
    let mut expected_proofs = before_proofs.clone();
    let mut rollout_deltas = BTreeMap::<String, i64>::new();
    let mut entries = Vec::new();

    for rollout in &report.rollouts {
        let mutable = rollout.provider != target_provider;
        let payload = if mutable {
            let logical = read_rollout_logical(rollout).map_err(redact_fixture_error)?;
            let (rewritten, delta) = rewrite_rollout_header_provider(
                &logical,
                &rollout.thread_id,
                &rollout.provider,
                &target_provider,
                &rollout.path,
            )
            .map_err(redact_fixture_error)?;
            rollout_deltas.insert(rollout.thread_id.clone(), delta);
            StagePayload::Bytes(encode_rollout(rollout, &rewritten)?)
        } else {
            StagePayload::None
        };
        entries.push(planned_entry(
            &roots,
            &rollout.path,
            ArtifactKind::Rollout,
            mutable,
            payload,
        )?);
    }

    for proof in &mut expected_proofs {
        if let Some(delta) = rollout_deltas.get(&proof.thread_id).copied() {
            shift_fixture_proof_offsets(proof, delta).map_err(redact_fixture_error)?;
        }
    }

    for database in &report.sqlite_databases {
        let mut mutation = SqliteMutation::default();
        for row in &database.state_rows {
            if row.provider != target_provider {
                mutation.state_updates.push(StateUpdate {
                    thread_id: row.thread_id.clone(),
                    source_provider: row.provider.clone(),
                    target_provider: target_provider.clone(),
                });
            }
        }
        for row in &database.history_rows {
            if let Some(delta) = rollout_deltas
                .get(&row.thread_id)
                .copied()
                .filter(|delta| *delta != 0)
            {
                let source_offset = row.next_rollout_byte_offset.ok_or_else(|| {
                    migration_error(
                        MigrationErrorKind::ValidationFailed,
                        "migration_history_cursor_missing",
                        "a verified history database lacks a durable byte cursor",
                        None,
                    )
                })?;
                mutation.history_updates.push(HistoryUpdate {
                    thread_id: row.thread_id.clone(),
                    source_offset,
                    delta,
                });
            }
        }
        let mutable = !mutation.state_updates.is_empty() || !mutation.history_updates.is_empty();
        let payload = if mutable {
            StagePayload::Sqlite(mutation)
        } else {
            StagePayload::None
        };
        entries.push(planned_entry(
            &roots,
            &database.path,
            ArtifactKind::Sqlite,
            mutable,
            payload,
        )?);
    }

    if let Some(index) = &report.session_index {
        entries.push(planned_entry(
            &roots,
            &index.path,
            ArtifactKind::SessionIndex,
            false,
            StagePayload::None,
        )?);
    }

    let config_mutable = report.config.active_provider.as_deref() != Some(&target_provider);
    let config_payload = if config_mutable {
        StagePayload::Bytes(
            rewrite_config_provider(&report, target.fixture_target())
                .map_err(redact_fixture_error)?,
        )
    } else {
        StagePayload::None
    };
    entries.push(planned_entry(
        &roots,
        &report.config.path,
        ArtifactKind::Config,
        config_mutable,
        config_payload,
    )?);

    entries.sort_by(|left, right| {
        commit_rank(left.journal.kind)
            .cmp(&commit_rank(right.journal.kind))
            .then_with(|| left.journal.artifact_id.cmp(&right.journal.artifact_id))
    });

    Ok(MigrationPlan {
        roots,
        entries,
        old_proofs: validation_proofs(&before_proofs),
        new_proofs: validation_proofs(&expected_proofs),
        source_provider: report.config.active_provider.clone(),
        target_provider,
    })
}

fn approve_roots(request: &ScanRequest) -> Result<ApprovedRoots, MigrationError> {
    if !request.codex_home.is_absolute() || !request.codex_home.is_dir() {
        return Err(migration_error(
            MigrationErrorKind::InvalidRequest,
            "migration_codex_root_invalid",
            "the Codex root must be an explicit absolute directory",
            None,
        ));
    }
    let codex = fs::canonicalize(&request.codex_home).map_err(|error| {
        classify_io(
            error,
            "migration_codex_root_unreadable",
            "the approved Codex root could not be resolved",
            None,
        )
    })?;
    validate_root_marker(&codex)?;

    let sqlite = match &request.explicit_sqlite_home {
        Some(path) => {
            if !path.is_absolute() || !path.is_dir() {
                return Err(migration_error(
                    MigrationErrorKind::InvalidRequest,
                    "migration_sqlite_root_invalid",
                    "the SQLite root must be an explicit absolute directory",
                    None,
                ));
            }
            let canonical = fs::canonicalize(path).map_err(|error| {
                classify_io(
                    error,
                    "migration_sqlite_root_unreadable",
                    "the approved SQLite root could not be resolved",
                    None,
                )
            })?;
            validate_root_marker(&canonical)?;
            (canonical != codex).then_some(canonical)
        }
        None => None,
    };
    let roots = ApprovedRoots { codex, sqlite };
    validate_runtime_layout(&roots)?;
    Ok(roots)
}

fn validate_runtime_layout(roots: &ApprovedRoots) -> Result<(), MigrationError> {
    for (_, root) in roots.distinct() {
        validate_runtime_directory(root, &root.join(TRANSACTION_DIRECTORY))?;
    }
    validate_runtime_directory(&roots.codex, &roots.codex.join("tmp"))
}

fn validate_runtime_directory(root: &Path, path: &Path) -> Result<(), MigrationError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(classify_io(
                error,
                "migration_runtime_metadata_failed",
                "transaction runtime metadata could not be inspected",
                None,
            ))
        }
    };
    if !metadata.file_type().is_dir() {
        return Err(migration_error(
            MigrationErrorKind::InvalidRequest,
            "migration_runtime_path_unsafe",
            "a transaction runtime path is not a regular directory",
            None,
        ));
    }
    let canonical = fs::canonicalize(path).map_err(|error| {
        classify_io(
            error,
            "migration_runtime_path_unreadable",
            "a transaction runtime directory could not be resolved",
            None,
        )
    })?;
    if !canonical.starts_with(root) {
        return Err(migration_error(
            MigrationErrorKind::InvalidRequest,
            "migration_runtime_path_outside_root",
            "a transaction runtime directory resolves outside its approved root",
            None,
        ));
    }
    Ok(())
}

fn validate_root_marker(root: &Path) -> Result<(), MigrationError> {
    let marker = root.join(MIGRATION_ROOT_MARKER);
    let metadata = fs::symlink_metadata(&marker).map_err(|_| {
        migration_error(
            MigrationErrorKind::RootNotAuthorized,
            "migration_root_marker_missing",
            "every writable root requires an explicit migration authorization marker",
            None,
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(migration_error(
            MigrationErrorKind::RootNotAuthorized,
            "migration_root_marker_invalid",
            "the migration authorization marker must be a regular file",
            None,
        ));
    }
    let contents = fs::read_to_string(marker).map_err(|_| {
        migration_error(
            MigrationErrorKind::RootNotAuthorized,
            "migration_root_marker_unreadable",
            "the migration authorization marker could not be read",
            None,
        )
    })?;
    if contents != MIGRATION_ROOT_MARKER_CONTENT {
        return Err(migration_error(
            MigrationErrorKind::RootNotAuthorized,
            "migration_root_marker_invalid",
            "the migration authorization marker has unexpected contents",
            None,
        ));
    }
    Ok(())
}

fn reject_blocked_scan(report: &ScanReport) -> Result<(), MigrationError> {
    if !report.is_blocked() {
        return Ok(());
    }
    diagnose_scan_access_error(report)?;
    let codes = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.level == super::DiagnosticLevel::Blocker)
        .map(|diagnostic| diagnostic.code)
        .collect::<BTreeSet<_>>();
    let (kind, code, message) = if codes.contains("sqlite_schema_unknown") {
        (
            MigrationErrorKind::UnknownSchema,
            "migration_schema_unknown",
            "an unrecognized SQLite schema blocks the migration",
        )
    } else if codes.contains("sqlite_integrity_failed")
        || codes.contains("sqlite_unreadable")
        || codes.iter().any(|code| code.contains("invalid"))
    {
        (
            MigrationErrorKind::CorruptStorage,
            "migration_storage_corrupt",
            "corrupt or unreadable session storage blocks the migration",
        )
    } else {
        (
            MigrationErrorKind::ScanBlocked,
            "migration_inventory_blocked",
            "the session inventory contains migration blockers",
        )
    };
    Err(migration_error(kind, code, message, None))
}

fn diagnose_scan_access_error(report: &ScanReport) -> Result<(), MigrationError> {
    for diagnostic in &report.diagnostics {
        let Some(path) = diagnostic.path.as_deref() else {
            continue;
        };
        if diagnostic.code.starts_with("sqlite_") && path.is_file() {
            let artifact_id = Some(digest_text(&path_fingerprint(path))[..20].to_owned());
            let connection = match Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            ) {
                Ok(connection) => connection,
                Err(error) => return Err(classify_sqlite(error, artifact_id)),
            };
            connection
                .busy_timeout(Duration::ZERO)
                .map_err(|error| classify_sqlite(error, artifact_id.clone()))?;
            if let Err(error) =
                connection.query_row("PRAGMA quick_check(1)", [], |row| row.get::<_, String>(0))
            {
                let classified = classify_sqlite(error, artifact_id);
                if matches!(
                    classified.kind,
                    MigrationErrorKind::SqliteBusy | MigrationErrorKind::PermissionDenied
                ) {
                    return Err(classified);
                }
            }
        } else if matches!(
            diagnostic.code,
            "config_unreadable"
                | "rollout_unreadable"
                | "rollout_header_unreadable"
                | "session_index_unreadable"
        ) {
            if let Err(error) = File::open(path) {
                return Err(classify_io(
                    error,
                    "migration_storage_access_failed",
                    "a storage artifact is occupied or unreadable",
                    None,
                ));
            }
        }
    }
    Ok(())
}

fn planned_entry(
    roots: &ApprovedRoots,
    path: &Path,
    kind: ArtifactKind,
    mutable: bool,
    payload: StagePayload,
) -> Result<PlannedEntry, MigrationError> {
    let existed = path.exists();
    let resolved = resolve_artifact_path(roots, path, existed)?;
    let locator = locate_artifact(roots, &resolved)?;
    let artifact_id = artifact_id(&locator);
    let metadata = if existed {
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            classify_io(
                error,
                "migration_artifact_metadata_failed",
                "storage artifact metadata could not be read",
                Some(artifact_id.clone()),
            )
        })?;
        if !metadata.file_type().is_file() {
            return Err(migration_error(
                MigrationErrorKind::InvalidRequest,
                "migration_artifact_not_regular",
                "a storage artifact is not a regular file",
                Some(artifact_id),
            ));
        }
        Some(metadata)
    } else {
        None
    };
    let source_hash = if existed {
        Some(if kind == ArtifactKind::Sqlite {
            sqlite_source_hash(&resolved)?
        } else {
            sha256_file(&resolved, Some(&artifact_id))?
        })
    } else {
        None
    };
    Ok(PlannedEntry {
        journal: JournalEntry {
            artifact_id,
            locator,
            kind,
            mutable,
            existed,
            byte_size: metadata.map_or(0, |metadata| metadata.len()),
            source_hash,
            backup_hash: None,
            staged_hash: None,
            applied: false,
        },
        absolute_path: resolved,
        payload,
    })
}

fn resolve_artifact_path(
    roots: &ApprovedRoots,
    path: &Path,
    existed: bool,
) -> Result<PathBuf, MigrationError> {
    let resolved = if existed {
        fs::canonicalize(path).map_err(|error| {
            classify_io(
                error,
                "migration_artifact_unreadable",
                "a storage artifact could not be resolved",
                None,
            )
        })?
    } else {
        let parent = path.parent().ok_or_else(|| {
            migration_error(
                MigrationErrorKind::InvalidRequest,
                "migration_artifact_parent_missing",
                "a storage artifact has no approved parent directory",
                None,
            )
        })?;
        fs::canonicalize(parent)
            .map_err(|error| {
                classify_io(
                    error,
                    "migration_artifact_parent_unreadable",
                    "a storage artifact parent could not be resolved",
                    None,
                )
            })?
            .join(path.file_name().ok_or_else(|| {
                migration_error(
                    MigrationErrorKind::InvalidRequest,
                    "migration_artifact_name_missing",
                    "a storage artifact has no file name",
                    None,
                )
            })?)
    };
    if !resolved.starts_with(&roots.codex)
        && !roots
            .sqlite
            .as_ref()
            .is_some_and(|root| resolved.starts_with(root))
    {
        return Err(migration_error(
            MigrationErrorKind::InvalidRequest,
            "migration_artifact_outside_roots",
            "a storage artifact resolves outside the approved roots",
            None,
        ));
    }
    Ok(resolved)
}

fn locate_artifact(roots: &ApprovedRoots, path: &Path) -> Result<ArtifactLocator, MigrationError> {
    let (root, relative_path) = if let Some(sqlite) = roots
        .sqlite
        .as_ref()
        .filter(|sqlite| path.starts_with(sqlite))
    {
        (RootSlot::Sqlite, path.strip_prefix(sqlite))
    } else {
        (RootSlot::Codex, path.strip_prefix(&roots.codex))
    };
    let relative_path = relative_path.map_err(|_| {
        migration_error(
            MigrationErrorKind::InvalidRequest,
            "migration_artifact_outside_roots",
            "a storage artifact resolves outside the approved roots",
            None,
        )
    })?;
    if relative_path.as_os_str().is_empty()
        || relative_path.to_str().is_none()
        || relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(migration_error(
            MigrationErrorKind::InvalidRequest,
            "migration_artifact_path_invalid",
            "a storage artifact has an unsupported relative path",
            None,
        ));
    }
    Ok(ArtifactLocator {
        root,
        relative_path: relative_path.to_path_buf(),
    })
}

fn encode_rollout(rollout: &RolloutArtifact, logical: &[u8]) -> Result<Vec<u8>, MigrationError> {
    match rollout.encoding {
        super::RolloutEncoding::Jsonl => Ok(logical.to_vec()),
        super::RolloutEncoding::Zstd => zstd::stream::encode_all(logical, 1).map_err(|_| {
            migration_error(
                MigrationErrorKind::ValidationFailed,
                "migration_rollout_encode_failed",
                "a compressed rollout could not be staged",
                None,
            )
        }),
    }
}

fn target_provider(target: MigrationProviderTarget) -> &'static str {
    match target {
        MigrationProviderTarget::Custom => CUSTOM_PROVIDER,
        MigrationProviderTarget::OpenAi => OFFICIAL_PROVIDER,
    }
}

fn commit_rank(kind: ArtifactKind) -> u8 {
    match kind {
        ArtifactKind::Rollout => 0,
        ArtifactKind::Sqlite => 1,
        ArtifactKind::SessionIndex => 2,
        ArtifactKind::Config => 3,
    }
}

fn validation_proofs(proofs: &[FixtureThreadProof]) -> Vec<ThreadValidationProof> {
    let mut results = proofs
        .iter()
        .map(|proof| {
            let mut history = Sha256::new();
            for item in &proof.history {
                history.update(path_fingerprint(&item.database));
                history.update(item.row.turn_count.to_le_bytes());
                history.update(item.row.item_count.to_le_bytes());
                update_optional_i64(&mut history, item.row.first_ordinal);
                update_optional_i64(&mut history, item.row.last_ordinal);
                update_optional_i64(&mut history, item.row.next_rollout_byte_offset);
                update_optional_i64(&mut history, item.row.next_rollout_ordinal);
                for turn in &item.row.turns {
                    history.update(turn.turn_id.as_bytes());
                    history.update(turn.rollout_ordinal.to_le_bytes());
                    update_optional_i64(&mut history, turn.rollout_byte_offset);
                    update_optional_i64(&mut history, turn.rollout_end_ordinal);
                    update_optional_i64(&mut history, turn.rollout_end_byte_offset);
                    history.update(turn.status.as_bytes());
                }
            }
            ThreadValidationProof {
                thread_id_hash: digest_text(&proof.thread_id),
                visible_event_count: proof.visible_event_count,
                visible_history_digest: proof.visible_history_digest.clone(),
                provider_neutral_digest: proof.provider_neutral_digest.clone(),
                workspace_hash: proof
                    .workspace
                    .as_ref()
                    .map(|workspace| path_fingerprint(workspace)),
                archived: proof.archived,
                state_database_count: proof.state_databases.len(),
                history_signature: digest_hex(history.finalize().as_slice()),
            }
        })
        .collect::<Vec<_>>();
    results.sort_by(|left, right| left.thread_id_hash.cmp(&right.thread_id_hash));
    results
}

fn update_optional_i64(hasher: &mut Sha256, value: Option<i64>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_le_bytes());
        }
        None => hasher.update([0]),
    }
}

fn preflight(
    request: &MigrationRequest,
    plan: &MigrationPlan,
    faults: &dyn MigrationFaultInjector,
) -> Result<(), MigrationError> {
    inject(faults, FaultPoint::PreflightProviderLock, None)?;
    if path_entry_exists(&provider_sync_lock_path(&plan.roots)) {
        return Err(migration_error(
            MigrationErrorKind::ProviderSyncLocked,
            "provider_sync_locked",
            "Codex++ provider synchronization is in progress; retry after it finishes",
            None,
        ));
    }

    inject(faults, FaultPoint::PreflightProcess, None)?;
    ensure_codex_stopped(&request.options, &plan.roots)?;

    inject(faults, FaultPoint::PreflightPermission, None)?;
    validate_write_permissions(plan)?;

    inject(faults, FaultPoint::PreflightSpace, None)?;
    validate_available_space(request, plan)?;

    for entry in plan
        .entries
        .iter()
        .filter(|entry| entry.journal.mutable && entry.journal.kind == ArtifactKind::Sqlite)
    {
        inject(
            faults,
            FaultPoint::PreflightSqliteBusy,
            Some(&entry.journal.artifact_id),
        )?;
        probe_sqlite_writable(
            &entry.absolute_path,
            &request.options,
            &entry.journal.artifact_id,
        )?;
    }
    verify_source_hashes(&plan.entries)?;
    Ok(())
}

fn validate_write_permissions(plan: &MigrationPlan) -> Result<(), MigrationError> {
    for (_, root) in plan.roots.distinct() {
        permission_metadata_check(root, None)?;
        let runtime_parent = root.join(TRANSACTION_DIRECTORY);
        if runtime_parent.exists() {
            permission_metadata_check(&runtime_parent, None)?;
        }
    }
    let lock_parent = plan.roots.codex.join("tmp");
    if lock_parent.exists() {
        permission_metadata_check(&lock_parent, None)?;
    }

    for entry in plan.entries.iter().filter(|entry| entry.journal.mutable) {
        if entry.journal.existed {
            permission_metadata_check(&entry.absolute_path, Some(&entry.journal.artifact_id))?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&entry.absolute_path)
                .map_err(|error| {
                    classify_io(
                        error,
                        "migration_artifact_not_writable",
                        "a mutable storage artifact is not writable",
                        Some(entry.journal.artifact_id.clone()),
                    )
                })?;
        }
        let parent = entry.absolute_path.parent().ok_or_else(|| {
            migration_error(
                MigrationErrorKind::InvalidRequest,
                "migration_artifact_parent_missing",
                "a mutable storage artifact has no parent directory",
                Some(entry.journal.artifact_id.clone()),
            )
        })?;
        permission_metadata_check(parent, Some(&entry.journal.artifact_id))?;
    }
    Ok(())
}

fn permission_metadata_check(path: &Path, artifact_id: Option<&str>) -> Result<(), MigrationError> {
    let metadata = fs::metadata(path).map_err(|error| {
        classify_io(
            error,
            "migration_permission_probe_failed",
            "storage permissions could not be verified",
            artifact_id.map(str::to_owned),
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o222 == 0 {
            return Err(migration_error(
                MigrationErrorKind::PermissionDenied,
                "migration_permission_denied",
                "a required storage location is read-only",
                artifact_id.map(str::to_owned),
            ));
        }
    }
    #[cfg(windows)]
    if metadata.permissions().readonly() {
        return Err(migration_error(
            MigrationErrorKind::PermissionDenied,
            "migration_permission_denied",
            "a required storage location is read-only",
            artifact_id.map(str::to_owned),
        ));
    }
    Ok(())
}

fn validate_available_space(
    request: &MigrationRequest,
    plan: &MigrationPlan,
) -> Result<(), MigrationError> {
    let mut required = BTreeMap::<RootSlot, u64>::new();
    for entry in &plan.entries {
        let mut bytes = entry.journal.byte_size.max(4096);
        if entry.journal.kind == ArtifactKind::Sqlite {
            bytes = bytes.saturating_add(sqlite_sidecar_size(&entry.absolute_path, "-wal"));
        }
        *required.entry(entry.journal.locator.root).or_default() = required
            .get(&entry.journal.locator.root)
            .copied()
            .unwrap_or_default()
            .saturating_add(bytes);
        if entry.journal.mutable {
            let stage_bytes = match &entry.payload {
                StagePayload::Bytes(bytes) => u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                StagePayload::Sqlite(_) => bytes,
                StagePayload::None => 0,
            };
            *required.entry(entry.journal.locator.root).or_default() = required
                .get(&entry.journal.locator.root)
                .copied()
                .unwrap_or_default()
                .saturating_add(stage_bytes);
        }
    }
    for (slot, bytes) in required {
        let root = plan.roots.path(slot)?;
        let available = available_space(root)?;
        if available < bytes.saturating_add(request.options.space_reserve_bytes) {
            return Err(migration_error(
                MigrationErrorKind::InsufficientSpace,
                "migration_space_insufficient",
                "the approved storage root lacks space for a verified backup and staging copy",
                None,
            ));
        }
    }
    Ok(())
}

fn available_space(path: &Path) -> Result<u64, MigrationError> {
    let disks = Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|disk| path.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count())
        .map(|disk| disk.available_space())
        .ok_or_else(|| {
            migration_error(
                MigrationErrorKind::Io,
                "migration_space_probe_failed",
                "available storage space could not be determined",
                None,
            )
        })
}

fn probe_sqlite_writable(
    path: &Path,
    options: &MigrationOptions,
    artifact_id: &str,
) -> Result<(), MigrationError> {
    for attempt in 0..=options.busy_retries {
        let result = (|| -> rusqlite::Result<()> {
            let mut connection = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            connection.busy_timeout(Duration::ZERO)?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.rollback()
        })();
        match result {
            Ok(()) => return Ok(()),
            Err(error) if sqlite_is_busy(&error) && attempt < options.busy_retries => {
                thread::sleep(options.busy_retry_delay);
            }
            Err(error) if sqlite_is_busy(&error) => {
                return Err(migration_error(
                    MigrationErrorKind::SqliteBusy,
                    "migration_sqlite_busy",
                    "a writable SQLite database remains busy; close Codex and retry",
                    Some(artifact_id.to_owned()),
                ));
            }
            Err(error) => return Err(classify_sqlite(error, Some(artifact_id.to_owned()))),
        }
    }
    unreachable!("busy retry loop always returns")
}

fn verify_source_hashes(entries: &[PlannedEntry]) -> Result<(), MigrationError> {
    for entry in entries {
        let current = if entry.journal.existed {
            Some(if entry.journal.kind == ArtifactKind::Sqlite {
                sqlite_source_hash(&entry.absolute_path)?
            } else {
                sha256_file(&entry.absolute_path, Some(&entry.journal.artifact_id))?
            })
        } else if entry.absolute_path.exists() {
            Some(String::new())
        } else {
            None
        };
        if current != entry.journal.source_hash {
            return Err(migration_error(
                MigrationErrorKind::SourceChanged,
                "migration_source_changed",
                "session storage changed after planning; retry from a fresh inventory",
                Some(entry.journal.artifact_id.clone()),
            ));
        }
    }
    Ok(())
}

fn ensure_codex_stopped(
    options: &MigrationOptions,
    roots: &ApprovedRoots,
) -> Result<(), MigrationError> {
    if options.codex_process_policy == CodexProcessPolicy::IsolatedFixture {
        validate_fixture_process_bypass(roots)?;
        return Ok(());
    }
    if !codex_process_running() {
        return Ok(());
    }
    if options.codex_process_policy == CodexProcessPolicy::RequireStopped {
        return Err(migration_error(
            MigrationErrorKind::CodexRunning,
            "codex_still_running",
            "Codex is still running; close Codex and retry",
            None,
        ));
    }
    request_codex_normal_exit()?;
    for _ in 0..options.process_wait_attempts {
        if !codex_process_running() {
            return Ok(());
        }
        thread::sleep(options.process_wait_delay);
    }
    Err(migration_error(
        MigrationErrorKind::CodexRunning,
        "codex_still_running",
        "Codex is still running; close Codex and retry",
        None,
    ))
}

fn validate_fixture_process_bypass(roots: &ApprovedRoots) -> Result<(), MigrationError> {
    for (_, root) in roots.distinct() {
        let marker = root.join(super::FIXTURE_ROOT_MARKER);
        let metadata = fs::symlink_metadata(&marker).map_err(|_| {
            migration_error(
                MigrationErrorKind::RootNotAuthorized,
                "fixture_process_bypass_marker_missing",
                "isolated process bypass requires the E10-2 fixture marker in every root",
                None,
            )
        })?;
        let contents = fs::read_to_string(marker).ok();
        if !metadata.file_type().is_file()
            || contents.as_deref() != Some(super::FIXTURE_ROOT_MARKER_CONTENT)
        {
            return Err(migration_error(
                MigrationErrorKind::RootNotAuthorized,
                "fixture_process_bypass_marker_invalid",
                "isolated process bypass requires a valid E10-2 fixture marker",
                None,
            ));
        }
    }
    Ok(())
}

fn codex_process_running() -> bool {
    let current_pid = std::process::id();
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system.processes().iter().any(|(pid, process)| {
        if pid.as_u32() == current_pid {
            return false;
        }
        let name = process.name().to_string_lossy().to_ascii_lowercase();
        matches!(name.as_str(), "codex" | "codex.exe")
    })
}

fn request_codex_normal_exit() -> Result<(), MigrationError> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("osascript")
            .args(["-e", "tell application \"Codex\" to quit"])
            .status()
            .map_err(|error| {
                classify_io(
                    error,
                    "codex_exit_request_failed",
                    "Codex could not be asked to exit normally",
                    None,
                )
            })?;
        if !status.success() {
            return Err(migration_error(
                MigrationErrorKind::CodexRunning,
                "codex_exit_request_failed",
                "Codex could not be asked to exit normally",
                None,
            ));
        }
    }
    #[cfg(target_os = "windows")]
    {
        let status = Command::new("taskkill")
            .args(["/IM", "Codex.exe"])
            .status()
            .map_err(|error| {
                classify_io(
                    error,
                    "codex_exit_request_failed",
                    "Codex could not be asked to exit normally",
                    None,
                )
            })?;
        if !status.success() {
            return Err(migration_error(
                MigrationErrorKind::CodexRunning,
                "codex_exit_request_failed",
                "Codex could not be asked to exit normally",
                None,
            ));
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err(migration_error(
        MigrationErrorKind::CodexRunning,
        "codex_exit_unsupported",
        "Codex must be closed before migration on this platform",
        None,
    ));
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    Ok(())
}

fn execute_plan(
    request: &MigrationRequest,
    mut plan: MigrationPlan,
    changed_artifacts: usize,
    faults: &dyn MigrationFaultInjector,
) -> Result<MigrationReport, MigrationError> {
    let migration_id = new_migration_id();
    create_operation_directories(&plan.roots, &migration_id)?;
    let now = unix_millis();
    let owner = new_lock_owner(&migration_id, &plan.roots)?;
    let mut journal = MigrationJournal {
        format_version: JOURNAL_FORMAT_VERSION,
        migration_id: migration_id.clone(),
        state: MigrationState::Planned,
        target_provider: plan.target_provider.clone(),
        source_provider: plan.source_provider.clone(),
        created_at_millis: now,
        updated_at_millis: now,
        roots: plan.roots.bindings(),
        owner,
        entries: plan
            .entries
            .iter()
            .map(|entry| entry.journal.clone())
            .collect(),
        old_proofs: plan.old_proofs.clone(),
        new_proofs: plan.new_proofs.clone(),
        restart_allowed: false,
    };
    persist_journal(&plan.roots, &journal)?;
    persist_owner_file(&plan.roots, &journal)?;
    let mut lock = match acquire_niko_lock(&plan.roots, &journal) {
        Ok(lock) => lock,
        Err(error) => {
            cleanup_unstarted_operation(&plan.roots, &migration_id);
            return Err(error);
        }
    };
    let mut provider_lock = match acquire_provider_sync_lock(&plan.roots, &journal) {
        Ok(lock) => lock,
        Err(mut error) => {
            let _ = rollback_transaction(request, &plan.roots, &mut journal, &NoMigrationFaults);
            lock.release()?;
            error.restart_allowed = true;
            return Err(error);
        }
    };

    let result: Result<(), MigrationError> = (|| {
        inject(faults, FaultPoint::PlannedPersisted, None)?;
        provider_lock.verify()?;
        commit_barrier(request, &plan)?;
        snapshot_all(request, &mut plan, &mut journal, faults)?;
        stage_all(&mut plan, &mut journal, faults)?;
        provider_lock.verify()?;
        commit_barrier(request, &plan)?;
        transition_journal(&plan.roots, &mut journal, MigrationState::Committing)?;
        inject(faults, FaultPoint::CommittingPersisted, None)?;
        commit_all(&plan.roots, &plan.entries, &mut journal, faults)?;
        transition_journal(&plan.roots, &mut journal, MigrationState::Validating)?;
        inject(faults, FaultPoint::ValidatingPersisted, None)?;
        inject(faults, FaultPoint::Validation, None)?;
        validate_new_state(request, &plan.roots, &journal)?;
        journal.restart_allowed = true;
        transition_journal(&plan.roots, &mut journal, MigrationState::Committed)?;
        inject(faults, FaultPoint::CommittedPersisted, None)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            prune_completed_transactions(
                &plan.roots,
                request.options.retained_transactions,
                Some(&migration_id),
            )?;
            provider_lock.release()?;
            lock.release()?;
            Ok(MigrationReport {
                migration_id: Some(migration_id),
                outcome: MigrationOutcome::Committed,
                state: MigrationState::Committed,
                changed_artifacts,
                restart_allowed: true,
            })
        }
        Err(mut error) if error.kind == MigrationErrorKind::InjectedCrash => {
            simulate_process_exit(&plan.roots, &mut journal);
            provider_lock.simulate_process_exit(&journal.owner);
            provider_lock.abandon();
            lock.abandon();
            error.restart_allowed = false;
            Err(error)
        }
        Err(mut error) => match rollback_transaction(request, &plan.roots, &mut journal, faults) {
            Ok(()) => {
                provider_lock.release()?;
                lock.release()?;
                error.restart_allowed = true;
                Err(error)
            }
            Err(mut rollback_error) => {
                if rollback_error.kind == MigrationErrorKind::InjectedCrash {
                    simulate_process_exit(&plan.roots, &mut journal);
                    provider_lock.simulate_process_exit(&journal.owner);
                    provider_lock.abandon();
                    lock.abandon();
                }
                rollback_error.restart_allowed = false;
                Err(rollback_error)
            }
        },
    }
}

fn commit_barrier(request: &MigrationRequest, plan: &MigrationPlan) -> Result<(), MigrationError> {
    ensure_codex_stopped(&request.options, &plan.roots)?;
    for entry in plan
        .entries
        .iter()
        .filter(|entry| entry.journal.mutable && entry.journal.kind == ArtifactKind::Sqlite)
    {
        probe_sqlite_writable(
            &entry.absolute_path,
            &request.options,
            &entry.journal.artifact_id,
        )?;
    }
    verify_source_hashes(&plan.entries)?;
    Ok(())
}

fn snapshot_all(
    request: &MigrationRequest,
    plan: &mut MigrationPlan,
    journal: &mut MigrationJournal,
    faults: &dyn MigrationFaultInjector,
) -> Result<(), MigrationError> {
    for (index, entry) in plan.entries.iter_mut().enumerate() {
        let backup = backup_path(&plan.roots, &journal.migration_id, &entry.journal)?;
        let backup_hash = if entry.journal.existed {
            if entry.journal.kind == ArtifactKind::Sqlite {
                sqlite_consistent_backup(
                    &entry.absolute_path,
                    &backup,
                    &request.options,
                    &entry.journal.artifact_id,
                )?;
                copy_permissions_if_present(&entry.absolute_path, &backup)?;
                verify_sqlite_file(&backup, &entry.journal.artifact_id)?;
            } else {
                copy_file_synced(
                    &entry.absolute_path,
                    &backup,
                    Some(&entry.journal.artifact_id),
                )?;
            }
            Some(sha256_file(&backup, Some(&entry.journal.artifact_id))?)
        } else {
            None
        };
        inject(
            faults,
            FaultPoint::SnapshotArtifact,
            Some(&entry.journal.artifact_id),
        )?;
        entry.journal.backup_hash = backup_hash.clone();
        journal.entries[index].backup_hash = backup_hash;
        persist_journal(&plan.roots, journal)?;
    }
    transition_journal(&plan.roots, journal, MigrationState::Snapshotted)?;
    inject(faults, FaultPoint::SnapshottedPersisted, None)
}

fn stage_all(
    plan: &mut MigrationPlan,
    journal: &mut MigrationJournal,
    faults: &dyn MigrationFaultInjector,
) -> Result<(), MigrationError> {
    for (index, entry) in plan.entries.iter_mut().enumerate() {
        if !entry.journal.mutable {
            continue;
        }
        let stage = staged_path(&plan.roots, &journal.migration_id, &entry.journal)?;
        match &entry.payload {
            StagePayload::Bytes(bytes) => {
                write_file_synced(&stage, bytes, Some(&entry.journal.artifact_id))?;
                copy_permissions_if_present(&entry.absolute_path, &stage)?;
            }
            StagePayload::Sqlite(mutation) => {
                let backup = backup_path(&plan.roots, &journal.migration_id, &entry.journal)?;
                copy_file_synced(&backup, &stage, Some(&entry.journal.artifact_id))?;
                apply_sqlite_mutation(&stage, mutation, &entry.journal.artifact_id)?;
                verify_sqlite_file(&stage, &entry.journal.artifact_id)?;
                copy_permissions_if_present(&entry.absolute_path, &stage)?;
                File::open(&stage)
                    .and_then(|file| file.sync_all())
                    .map_err(|error| {
                        classify_io(
                            error,
                            "migration_stage_sync_failed",
                            "a staged SQLite database could not be flushed",
                            Some(entry.journal.artifact_id.clone()),
                        )
                    })?;
            }
            StagePayload::None => {
                return Err(migration_error(
                    MigrationErrorKind::JournalCorrupt,
                    "migration_stage_plan_missing",
                    "a mutable artifact lacks a staging plan",
                    Some(entry.journal.artifact_id.clone()),
                ));
            }
        }
        let staged_hash = sha256_file(&stage, Some(&entry.journal.artifact_id))?;
        inject(
            faults,
            FaultPoint::StageArtifact,
            Some(&entry.journal.artifact_id),
        )?;
        entry.journal.staged_hash = Some(staged_hash.clone());
        journal.entries[index].staged_hash = Some(staged_hash);
        persist_journal(&plan.roots, journal)?;
    }
    transition_journal(&plan.roots, journal, MigrationState::Staged)?;
    inject(faults, FaultPoint::StagedPersisted, None)
}

fn commit_all(
    roots: &ApprovedRoots,
    entries: &[PlannedEntry],
    journal: &mut MigrationJournal,
    faults: &dyn MigrationFaultInjector,
) -> Result<(), MigrationError> {
    for (index, entry) in entries.iter().enumerate() {
        if !entry.journal.mutable {
            continue;
        }
        commit_journal_entry(roots, journal, index)?;
        inject(
            faults,
            FaultPoint::CommitArtifact,
            Some(&entry.journal.artifact_id),
        )?;
        journal.entries[index].applied = true;
        persist_journal(roots, journal)?;
    }
    Ok(())
}

fn commit_journal_entry(
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
    index: usize,
) -> Result<(), MigrationError> {
    let entry = journal.entries.get(index).ok_or_else(|| {
        migration_error(
            MigrationErrorKind::JournalCorrupt,
            "migration_manifest_entry_missing",
            "the migration manifest is incomplete",
            None,
        )
    })?;
    let target = resolve_locator(roots, &entry.locator)?;
    let stage = staged_path(roots, &journal.migration_id, entry)?;
    verify_expected_hash(&stage, entry.staged_hash.as_deref(), entry)?;
    if entry.kind == ArtifactKind::Sqlite {
        normalize_sqlite_target(&target, &entry.artifact_id)?;
    }
    atomic_install(&stage, &target, &entry.artifact_id)?;
    if entry.kind == ArtifactKind::Sqlite {
        remove_sqlite_sidecars(&target, &entry.artifact_id)?;
    }
    verify_expected_hash(&target, entry.staged_hash.as_deref(), entry)
}

fn transition_journal(
    roots: &ApprovedRoots,
    journal: &mut MigrationJournal,
    next: MigrationState,
) -> Result<(), MigrationError> {
    if journal.state == next {
        return Ok(());
    }
    let valid = matches!(
        (journal.state, next),
        (MigrationState::Planned, MigrationState::Snapshotted)
            | (MigrationState::Snapshotted, MigrationState::Staged)
            | (MigrationState::Staged, MigrationState::Committing)
            | (MigrationState::Committing, MigrationState::Validating)
            | (MigrationState::Validating, MigrationState::Committed)
            | (_, MigrationState::RollingBack)
            | (MigrationState::RollingBack, MigrationState::RolledBack)
    );
    if !valid || journal.state.is_terminal() {
        return Err(migration_error(
            MigrationErrorKind::JournalCorrupt,
            "migration_state_transition_invalid",
            "the migration journal contains an invalid state transition",
            None,
        ));
    }
    journal.state = next;
    journal.updated_at_millis = unix_millis();
    persist_journal(roots, journal)
}

struct NikoLockGuard {
    lock_path: PathBuf,
    owner_bytes: Vec<u8>,
    release_on_drop: bool,
}

impl NikoLockGuard {
    fn release(&mut self) -> Result<(), MigrationError> {
        if !self.release_on_drop {
            return Ok(());
        }
        let current = fs::read(&self.lock_path).map_err(|error| {
            classify_io(
                error,
                "niko_lock_read_failed",
                "the Niko migration lock could not be verified",
                None,
            )
        })?;
        if current != self.owner_bytes {
            return Err(migration_error(
                MigrationErrorKind::NikoLockUnverifiable,
                "niko_lock_owner_changed",
                "the Niko migration lock owner changed unexpectedly",
                None,
            ));
        }
        fs::remove_file(&self.lock_path).map_err(|error| {
            classify_io(
                error,
                "niko_lock_release_failed",
                "the Niko migration lock could not be released",
                None,
            )
        })?;
        sync_parent(&self.lock_path)?;
        self.release_on_drop = false;
        Ok(())
    }

    fn abandon(&mut self) {
        self.release_on_drop = false;
    }
}

impl Drop for NikoLockGuard {
    fn drop(&mut self) {
        if self.release_on_drop {
            let current = fs::read(&self.lock_path).ok();
            if current.as_deref() == Some(self.owner_bytes.as_slice()) {
                let _ = fs::remove_file(&self.lock_path);
                let _ = sync_parent(&self.lock_path);
            }
        }
    }
}

fn acquire_niko_lock(
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
) -> Result<NikoLockGuard, MigrationError> {
    let owner_path = owner_path(roots, &journal.migration_id);
    let lock_path = niko_lock_path(roots);
    fs::create_dir_all(lock_path.parent().expect("lock path has a parent")).map_err(|error| {
        classify_io(
            error,
            "niko_lock_parent_failed",
            "the Niko migration lock directory could not be prepared",
            None,
        )
    })?;
    let owner_bytes = fs::read(&owner_path).map_err(|error| {
        classify_io(
            error,
            "niko_lock_owner_read_failed",
            "the Niko migration lock owner record could not be read",
            None,
        )
    })?;
    match fs::hard_link(&owner_path, &lock_path) {
        Ok(()) => {
            sync_parent(&lock_path)?;
            Ok(NikoLockGuard {
                lock_path,
                owner_bytes,
                release_on_drop: true,
            })
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(migration_error(
            MigrationErrorKind::NikoLocked,
            "niko_migration_locked",
            "another Niko migration owns the storage transaction lock",
            None,
        )),
        Err(error) => Err(classify_io(
            error,
            "niko_lock_acquire_failed",
            "the Niko migration lock could not be acquired",
            None,
        )),
    }
}

struct ProviderSyncLockGuard {
    lock_path: PathBuf,
    owner_bytes: Vec<u8>,
    release_on_drop: bool,
}

impl ProviderSyncLockGuard {
    fn verify(&self) -> Result<(), MigrationError> {
        let metadata = fs::symlink_metadata(&self.lock_path).map_err(|_| {
            migration_error(
                MigrationErrorKind::ProviderSyncLocked,
                "provider_sync_lock_lost",
                "the provider synchronization lock disappeared during migration",
                None,
            )
        })?;
        if !metadata.file_type().is_dir() {
            return Err(migration_error(
                MigrationErrorKind::ProviderSyncLocked,
                "provider_sync_lock_replaced",
                "the provider synchronization lock changed during migration",
                None,
            ));
        }
        let current = fs::read(self.lock_path.join(OWNER_FILE)).map_err(|_| {
            migration_error(
                MigrationErrorKind::ProviderSyncLocked,
                "provider_sync_owner_lost",
                "the provider synchronization lock owner disappeared",
                None,
            )
        })?;
        if current != self.owner_bytes {
            return Err(migration_error(
                MigrationErrorKind::ProviderSyncLocked,
                "provider_sync_owner_changed",
                "the provider synchronization lock owner changed during migration",
                None,
            ));
        }
        Ok(())
    }

    fn release(&mut self) -> Result<(), MigrationError> {
        if !self.release_on_drop {
            return Ok(());
        }
        self.verify()?;
        fs::remove_file(self.lock_path.join(OWNER_FILE)).map_err(|error| {
            classify_io(
                error,
                "provider_sync_owner_remove_failed",
                "the provider synchronization owner record could not be removed",
                None,
            )
        })?;
        fs::remove_dir(&self.lock_path).map_err(|error| {
            classify_io(
                error,
                "provider_sync_lock_release_failed",
                "the provider synchronization lock could not be released",
                None,
            )
        })?;
        sync_parent(&self.lock_path)?;
        self.release_on_drop = false;
        Ok(())
    }

    fn simulate_process_exit(&mut self, owner: &LockOwner) {
        let provider_owner = ProviderLockOwner {
            owner_kind: "niko".to_owned(),
            migration: owner.clone(),
        };
        if let Ok(mut bytes) = serde_json::to_vec(&provider_owner) {
            bytes.push(b'\n');
            if write_file_synced(&self.lock_path.join(OWNER_FILE), &bytes, None).is_ok() {
                self.owner_bytes = bytes;
            }
        }
    }

    fn abandon(&mut self) {
        self.release_on_drop = false;
    }
}

impl Drop for ProviderSyncLockGuard {
    fn drop(&mut self) {
        if self.release_on_drop && self.verify().is_ok() {
            let _ = fs::remove_file(self.lock_path.join(OWNER_FILE));
            let _ = fs::remove_dir(&self.lock_path);
            let _ = sync_parent(&self.lock_path);
        }
    }
}

fn acquire_provider_sync_lock(
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
) -> Result<ProviderSyncLockGuard, MigrationError> {
    let lock_path = provider_sync_lock_path(roots);
    let parent = lock_path.parent().expect("provider lock has a parent");
    fs::create_dir_all(parent).map_err(|error| {
        classify_io(
            error,
            "provider_sync_parent_failed",
            "the provider synchronization lock directory could not be prepared",
            None,
        )
    })?;
    match fs::create_dir(&lock_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(migration_error(
                MigrationErrorKind::ProviderSyncLocked,
                "provider_sync_locked",
                "Codex++ provider synchronization is in progress; retry later",
                None,
            ))
        }
        Err(error) => {
            return Err(classify_io(
                error,
                "provider_sync_lock_acquire_failed",
                "the provider synchronization lock could not be acquired",
                None,
            ))
        }
    }
    let provider_owner = ProviderLockOwner {
        owner_kind: "niko".to_owned(),
        migration: journal.owner.clone(),
    };
    let mut owner_bytes = serde_json::to_vec(&provider_owner).map_err(|_| {
        migration_error(
            MigrationErrorKind::JournalCorrupt,
            "provider_sync_owner_serialize_failed",
            "the provider synchronization owner could not be serialized",
            None,
        )
    })?;
    owner_bytes.push(b'\n');
    if let Err(error) = write_file_synced(&lock_path.join(OWNER_FILE), &owner_bytes, None) {
        let _ = fs::remove_file(lock_path.join(OWNER_FILE));
        let _ = fs::remove_dir(&lock_path);
        return Err(error);
    }
    sync_directory(&lock_path)?;
    sync_parent(&lock_path)?;
    Ok(ProviderSyncLockGuard {
        lock_path,
        owner_bytes,
        release_on_drop: true,
    })
}

fn persist_owner_file(
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
) -> Result<(), MigrationError> {
    let mut bytes = serde_json::to_vec(&journal.owner).map_err(|_| {
        migration_error(
            MigrationErrorKind::JournalCorrupt,
            "niko_lock_owner_serialize_failed",
            "the Niko migration lock owner could not be serialized",
            None,
        )
    })?;
    bytes.push(b'\n');
    write_file_synced(&owner_path(roots, &journal.migration_id), &bytes, None)
}

fn persist_journal(
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
) -> Result<(), MigrationError> {
    let mut bytes = serde_json::to_vec_pretty(journal).map_err(|_| {
        migration_error(
            MigrationErrorKind::JournalCorrupt,
            "migration_journal_serialize_failed",
            "the migration journal could not be serialized",
            None,
        )
    })?;
    bytes.push(b'\n');
    let path = journal_path(roots, &journal.migration_id);
    let temporary = path.with_extension("json.tmp");
    write_file_synced(&temporary, &bytes, None)?;
    atomic_replace_file(&temporary, &path, None)?;
    sync_parent(&path)
}

pub fn recover_codex_session_migrations(
    request: &MigrationRequest,
) -> Result<RecoveryReport, MigrationError> {
    recover_codex_session_migrations_with_faults(request, &NoMigrationFaults)
}

pub fn recover_codex_session_migrations_with_faults(
    request: &MigrationRequest,
    faults: &dyn MigrationFaultInjector,
) -> Result<RecoveryReport, MigrationError> {
    let roots = approve_roots(&request.scan)?;
    let mut journals = read_journals(&roots)?;
    clear_verified_stale_lock(&roots, &journals)?;
    clear_verified_stale_provider_lock(&roots, &journals)?;
    let mut pending = journals
        .drain(..)
        .filter(|journal| !journal.state.is_terminal())
        .collect::<Vec<_>>();
    pending.sort_by_key(|journal| (journal.created_at_millis, journal.migration_id.clone()));
    if pending.is_empty() {
        return Ok(RecoveryReport {
            migrations: Vec::new(),
            restart_allowed: true,
        });
    }

    inject(faults, FaultPoint::PreflightProviderLock, None)?;
    if path_entry_exists(&provider_sync_lock_path(&roots)) {
        return Err(migration_error(
            MigrationErrorKind::ProviderSyncLocked,
            "provider_sync_locked",
            "Codex++ provider synchronization is in progress; retry recovery later",
            None,
        ));
    }
    inject(faults, FaultPoint::PreflightProcess, None)?;
    ensure_codex_stopped(&request.options, &roots)?;

    let mut reports = Vec::new();
    for mut journal in pending {
        validate_journal_roots(&roots, &journal)?;
        journal.owner = new_lock_owner(&journal.migration_id, &roots)?;
        persist_journal(&roots, &journal)?;
        persist_owner_file(&roots, &journal)?;
        let mut lock = acquire_niko_lock(&roots, &journal)?;
        let mut provider_lock = acquire_provider_sync_lock(&roots, &journal)?;
        provider_lock.verify()?;
        let result = recover_one(request, &roots, &mut journal, faults);
        match result {
            Ok(report) => {
                prune_completed_transactions(
                    &roots,
                    request.options.retained_transactions,
                    report.migration_id.as_deref(),
                )?;
                provider_lock.release()?;
                lock.release()?;
                reports.push(report);
            }
            Err(mut error) if error.kind == MigrationErrorKind::InjectedCrash => {
                simulate_process_exit(&roots, &mut journal);
                provider_lock.simulate_process_exit(&journal.owner);
                provider_lock.abandon();
                lock.abandon();
                error.restart_allowed = false;
                return Err(error);
            }
            Err(mut error) => {
                error.restart_allowed = false;
                return Err(error);
            }
        }
    }
    Ok(RecoveryReport {
        migrations: reports,
        restart_allowed: true,
    })
}

fn recover_one(
    request: &MigrationRequest,
    roots: &ApprovedRoots,
    journal: &mut MigrationJournal,
    faults: &dyn MigrationFaultInjector,
) -> Result<MigrationReport, MigrationError> {
    let changed_artifacts = journal.entries.iter().filter(|entry| entry.mutable).count();
    let finish_new = matches!(
        journal.state,
        MigrationState::Committing | MigrationState::Validating
    ) && all_non_config_targets_are_new(roots, journal)?;

    if finish_new {
        recovery_commit_barrier(request, roots, journal)?;
        for index in 0..journal.entries.len() {
            if !journal.entries[index].mutable {
                continue;
            }
            if target_matches_staged(roots, journal, index)? {
                journal.entries[index].applied = true;
                persist_journal(roots, journal)?;
                continue;
            }
            commit_journal_entry(roots, journal, index)?;
            inject(
                faults,
                FaultPoint::CommitArtifact,
                Some(&journal.entries[index].artifact_id),
            )?;
            journal.entries[index].applied = true;
            persist_journal(roots, journal)?;
        }
        if journal.state == MigrationState::Committing {
            transition_journal(roots, journal, MigrationState::Validating)?;
            inject(faults, FaultPoint::ValidatingPersisted, None)?;
        }
        inject(faults, FaultPoint::Validation, None)?;
        validate_new_state(request, roots, journal)?;
        journal.restart_allowed = true;
        transition_journal(roots, journal, MigrationState::Committed)?;
        inject(faults, FaultPoint::CommittedPersisted, None)?;
        return Ok(MigrationReport {
            migration_id: Some(journal.migration_id.clone()),
            outcome: MigrationOutcome::Committed,
            state: MigrationState::Committed,
            changed_artifacts,
            restart_allowed: true,
        });
    }

    rollback_transaction(request, roots, journal, faults)?;
    Ok(MigrationReport {
        migration_id: Some(journal.migration_id.clone()),
        outcome: MigrationOutcome::RolledBack,
        state: MigrationState::RolledBack,
        changed_artifacts,
        restart_allowed: true,
    })
}

fn recovery_commit_barrier(
    request: &MigrationRequest,
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
) -> Result<(), MigrationError> {
    ensure_codex_stopped(&request.options, roots)?;
    for entry in journal
        .entries
        .iter()
        .filter(|entry| entry.mutable && entry.kind == ArtifactKind::Sqlite)
    {
        let target = resolve_locator(roots, &entry.locator)?;
        probe_sqlite_writable(&target, &request.options, &entry.artifact_id)?;
    }
    verify_observed_entries(roots, journal)
}

fn rollback_transaction(
    request: &MigrationRequest,
    roots: &ApprovedRoots,
    journal: &mut MigrationJournal,
    faults: &dyn MigrationFaultInjector,
) -> Result<(), MigrationError> {
    if journal.state == MigrationState::RolledBack {
        return Ok(());
    }
    if journal.state == MigrationState::Committed {
        return Err(migration_error(
            MigrationErrorKind::JournalCorrupt,
            "migration_rollback_committed",
            "a committed migration cannot enter automatic rollback",
            None,
        ));
    }
    let targets_may_have_changed = matches!(
        journal.state,
        MigrationState::Committing | MigrationState::Validating | MigrationState::RollingBack
    );
    if journal.state != MigrationState::RollingBack {
        transition_journal(roots, journal, MigrationState::RollingBack)?;
        inject(faults, FaultPoint::RollingBackPersisted, None)?;
    }

    if targets_may_have_changed {
        verify_all_backups(roots, journal)?;
        let mut indexes = journal
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.mutable)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        indexes.sort_by_key(|index| commit_rank(journal.entries[*index].kind));
        for index in indexes {
            restore_journal_entry(roots, journal, index)?;
            inject(
                faults,
                FaultPoint::RollbackArtifact,
                Some(&journal.entries[index].artifact_id),
            )?;
            journal.entries[index].applied = false;
            persist_journal(roots, journal)?;
        }
        validate_old_state(request, roots, journal)?;
    }
    journal.restart_allowed = true;
    transition_journal(roots, journal, MigrationState::RolledBack)?;
    inject(faults, FaultPoint::RolledBackPersisted, None)
}

fn restore_journal_entry(
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
    index: usize,
) -> Result<(), MigrationError> {
    let entry = &journal.entries[index];
    let target = resolve_locator(roots, &entry.locator)?;
    if entry.existed {
        let backup = backup_path(roots, &journal.migration_id, entry)?;
        verify_expected_hash(&backup, entry.backup_hash.as_deref(), entry)?;
        if entry.kind == ArtifactKind::Sqlite && target.exists() {
            normalize_sqlite_target(&target, &entry.artifact_id)?;
        }
        atomic_install(&backup, &target, &entry.artifact_id)?;
        if entry.kind == ArtifactKind::Sqlite {
            remove_sqlite_sidecars(&target, &entry.artifact_id)?;
        }
        verify_expected_hash(&target, entry.backup_hash.as_deref(), entry)
    } else if target.exists() {
        let staged = entry.staged_hash.as_deref().ok_or_else(|| {
            migration_error(
                MigrationErrorKind::JournalCorrupt,
                "migration_staged_hash_missing",
                "a created artifact lacks its staged hash",
                Some(entry.artifact_id.clone()),
            )
        })?;
        if sha256_file(&target, Some(&entry.artifact_id))? != staged {
            return Err(migration_error(
                MigrationErrorKind::SourceChanged,
                "migration_rollback_target_changed",
                "a created artifact changed before rollback could remove it",
                Some(entry.artifact_id.clone()),
            ));
        }
        fs::remove_file(&target).map_err(|error| {
            classify_io(
                error,
                "migration_rollback_remove_failed",
                "a newly created artifact could not be removed during rollback",
                Some(entry.artifact_id.clone()),
            )
        })?;
        sync_parent(&target)
    } else {
        Ok(())
    }
}

fn verify_all_backups(
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
) -> Result<(), MigrationError> {
    for entry in &journal.entries {
        if entry.existed {
            let backup = backup_path(roots, &journal.migration_id, entry)?;
            verify_expected_hash(&backup, entry.backup_hash.as_deref(), entry)?;
            if entry.kind == ArtifactKind::Sqlite {
                verify_sqlite_file(&backup, &entry.artifact_id)?;
            }
        }
    }
    Ok(())
}

fn all_non_config_targets_are_new(
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
) -> Result<bool, MigrationError> {
    let candidates = journal
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.mutable && entry.kind != ArtifactKind::Config)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(true);
    }
    for (index, _) in candidates {
        if !target_matches_staged(roots, journal, index)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn target_matches_staged(
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
    index: usize,
) -> Result<bool, MigrationError> {
    let entry = &journal.entries[index];
    let Some(expected) = entry.staged_hash.as_deref() else {
        return Ok(false);
    };
    let target = resolve_locator(roots, &entry.locator)?;
    if !target.is_file() {
        return Ok(false);
    }
    Ok(sha256_file(&target, Some(&entry.artifact_id))? == expected)
}

fn validate_new_state(
    request: &MigrationRequest,
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
) -> Result<(), MigrationError> {
    verify_observed_entries(roots, journal)?;
    for entry in journal.entries.iter().filter(|entry| entry.mutable) {
        let target = resolve_locator(roots, &entry.locator)?;
        verify_expected_hash(&target, entry.staged_hash.as_deref(), entry)?;
        if entry.kind == ArtifactKind::Sqlite {
            verify_sqlite_file(&target, &entry.artifact_id)?;
            ensure_sidecars_absent(&target, &entry.artifact_id)?;
        }
    }
    let report = scan_codex_sessions(&request.scan).map_err(|_| {
        migration_error(
            MigrationErrorKind::ValidationFailed,
            "migration_validation_scan_failed",
            "the committed storage could not be inventoried",
            None,
        )
    })?;
    reject_blocked_scan(&report).map_err(|_| {
        migration_error(
            MigrationErrorKind::ValidationFailed,
            "migration_validation_blocked",
            "the committed storage failed structural validation",
            None,
        )
    })?;
    if report.config.active_provider.as_deref() != Some(&journal.target_provider)
        || report
            .rollouts
            .iter()
            .any(|rollout| rollout.provider != journal.target_provider)
        || report
            .sqlite_databases
            .iter()
            .flat_map(|database| database.state_rows.iter())
            .any(|row| row.provider != journal.target_provider)
    {
        return Err(migration_error(
            MigrationErrorKind::ValidationFailed,
            "migration_target_validation_failed",
            "the committed storage did not converge on the target provider",
            None,
        ));
    }
    let proofs = build_fixture_thread_proofs(&report).map_err(redact_fixture_error)?;
    if validation_proofs(&proofs) != journal.new_proofs {
        return Err(migration_error(
            MigrationErrorKind::ValidationFailed,
            "migration_roundtrip_validation_failed",
            "thread history or pagination invariants changed during migration",
            None,
        ));
    }
    Ok(())
}

fn validate_old_state(
    request: &MigrationRequest,
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
) -> Result<(), MigrationError> {
    verify_observed_entries(roots, journal)?;
    for entry in journal.entries.iter().filter(|entry| entry.mutable) {
        let target = resolve_locator(roots, &entry.locator)?;
        if entry.existed {
            verify_expected_hash(&target, entry.backup_hash.as_deref(), entry)?;
            if entry.kind == ArtifactKind::Sqlite {
                verify_sqlite_file(&target, &entry.artifact_id)?;
                ensure_sidecars_absent(&target, &entry.artifact_id)?;
            }
        } else if target.exists() {
            return Err(migration_error(
                MigrationErrorKind::ValidationFailed,
                "migration_rollback_created_artifact_present",
                "rollback left an artifact that did not exist before migration",
                Some(entry.artifact_id.clone()),
            ));
        }
    }
    let report = scan_codex_sessions(&request.scan).map_err(|_| {
        migration_error(
            MigrationErrorKind::ValidationFailed,
            "migration_rollback_scan_failed",
            "the restored storage could not be inventoried",
            None,
        )
    })?;
    reject_blocked_scan(&report).map_err(|_| {
        migration_error(
            MigrationErrorKind::ValidationFailed,
            "migration_rollback_validation_blocked",
            "the restored storage failed structural validation",
            None,
        )
    })?;
    if report.config.active_provider != journal.source_provider {
        return Err(migration_error(
            MigrationErrorKind::ValidationFailed,
            "migration_rollback_provider_changed",
            "rollback did not restore the original provider configuration",
            None,
        ));
    }
    let proofs = build_fixture_thread_proofs(&report).map_err(redact_fixture_error)?;
    if validation_proofs(&proofs) != journal.old_proofs {
        return Err(migration_error(
            MigrationErrorKind::ValidationFailed,
            "migration_rollback_roundtrip_failed",
            "rollback did not restore the original thread history invariants",
            None,
        ));
    }
    Ok(())
}

fn verify_observed_entries(
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
) -> Result<(), MigrationError> {
    for entry in journal.entries.iter().filter(|entry| !entry.mutable) {
        let target = resolve_locator(roots, &entry.locator)?;
        let current = if entry.existed {
            Some(if entry.kind == ArtifactKind::Sqlite {
                sqlite_source_hash(&target)?
            } else {
                sha256_file(&target, Some(&entry.artifact_id))?
            })
        } else if target.exists() {
            Some(String::new())
        } else {
            None
        };
        if current != entry.source_hash {
            return Err(migration_error(
                MigrationErrorKind::SourceChanged,
                "migration_observed_artifact_changed",
                "an observed storage artifact changed during the transaction",
                Some(entry.artifact_id.clone()),
            ));
        }
    }
    Ok(())
}

fn read_journals(roots: &ApprovedRoots) -> Result<Vec<MigrationJournal>, MigrationError> {
    let directory = roots.codex.join(TRANSACTION_DIRECTORY);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(classify_io(
                error,
                "migration_journal_directory_unreadable",
                "the migration journal directory could not be read",
                None,
            ))
        }
    };
    let mut journals = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            classify_io(
                error,
                "migration_journal_entry_unreadable",
                "a migration journal entry could not be read",
                None,
            )
        })?;
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let Some(id) = name.to_str().filter(|name| is_migration_id(name)) else {
            continue;
        };
        let bytes = match fs::read(entry.path().join(JOURNAL_FILE)) {
            Ok(bytes) => bytes,
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && operation_is_unstarted(roots, id)? =>
            {
                cleanup_unstarted_operation(roots, id);
                continue;
            }
            Err(_) => {
                return Err(migration_error(
                    MigrationErrorKind::JournalCorrupt,
                    "migration_journal_missing",
                    "a transaction directory lacks a readable migration journal",
                    None,
                ))
            }
        };
        let journal: MigrationJournal = serde_json::from_slice(&bytes).map_err(|_| {
            migration_error(
                MigrationErrorKind::JournalCorrupt,
                "migration_journal_invalid",
                "a migration journal is not valid JSON",
                None,
            )
        })?;
        if journal.format_version != JOURNAL_FORMAT_VERSION
            || journal.migration_id != id
            || journal.entries.iter().any(|entry| {
                entry.artifact_id != artifact_id(&entry.locator)
                    || entry
                        .locator
                        .relative_path
                        .components()
                        .any(|component| !matches!(component, Component::Normal(_)))
            })
        {
            return Err(migration_error(
                MigrationErrorKind::JournalCorrupt,
                "migration_journal_inconsistent",
                "a migration journal failed identity or manifest validation",
                None,
            ));
        }
        validate_journal_roots(roots, &journal)?;
        journals.push(journal);
    }
    journals.sort_by_key(|journal| (journal.created_at_millis, journal.migration_id.clone()));
    Ok(journals)
}

fn validate_journal_roots(
    roots: &ApprovedRoots,
    journal: &MigrationJournal,
) -> Result<(), MigrationError> {
    if journal.roots != roots.bindings()
        || journal.owner.root_fingerprint != path_fingerprint(&roots.codex)
        || journal.owner.journal_id != journal.migration_id
    {
        return Err(migration_error(
            MigrationErrorKind::JournalCorrupt,
            "migration_journal_roots_changed",
            "the explicit recovery roots do not match the migration journal",
            None,
        ));
    }
    for entry in &journal.entries {
        let _ = resolve_locator(roots, &entry.locator)?;
    }
    Ok(())
}

fn clear_verified_stale_lock(
    roots: &ApprovedRoots,
    journals: &[MigrationJournal],
) -> Result<(), MigrationError> {
    let lock = niko_lock_path(roots);
    let metadata = match fs::symlink_metadata(&lock) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(classify_io(
                error,
                "niko_lock_unreadable",
                "the Niko migration lock could not be read",
                None,
            ))
        }
    };
    if !metadata.file_type().is_file() {
        return Err(migration_error(
            MigrationErrorKind::NikoLockUnverifiable,
            "niko_lock_type_invalid",
            "the Niko migration lock is not a regular owner record",
            None,
        ));
    }
    let bytes = fs::read(&lock).map_err(|_| {
        migration_error(
            MigrationErrorKind::NikoLockUnverifiable,
            "niko_lock_unreadable",
            "the Niko migration lock owner record could not be read",
            None,
        )
    })?;
    let owner: LockOwner = serde_json::from_slice(&bytes).map_err(|_| {
        migration_error(
            MigrationErrorKind::NikoLockUnverifiable,
            "niko_lock_owner_invalid",
            "the Niko migration lock owner record is invalid",
            None,
        )
    })?;
    let journal = journals
        .iter()
        .find(|journal| journal.migration_id == owner.journal_id)
        .ok_or_else(|| {
            migration_error(
                MigrationErrorKind::NikoLockUnverifiable,
                "niko_lock_journal_missing",
                "the Niko lock cannot be tied to a migration journal",
                None,
            )
        })?;
    let owner_copy = fs::read(owner_path(roots, &owner.journal_id)).map_err(|_| {
        migration_error(
            MigrationErrorKind::NikoLockUnverifiable,
            "niko_lock_owner_copy_missing",
            "the Niko lock lacks its durable owner identity",
            None,
        )
    })?;
    if owner != journal.owner
        || owner_copy != bytes
        || owner.root_fingerprint != path_fingerprint(&roots.codex)
    {
        return Err(migration_error(
            MigrationErrorKind::NikoLockUnverifiable,
            "niko_lock_owner_mismatch",
            "the Niko lock owner and journal are inconsistent",
            None,
        ));
    }
    if process_identity_is_live(owner.pid, owner.process_start_time) {
        return Err(migration_error(
            MigrationErrorKind::NikoLocked,
            "niko_migration_locked",
            "another live Niko migration owns the storage transaction lock",
            None,
        ));
    }
    fs::remove_file(&lock).map_err(|error| {
        classify_io(
            error,
            "niko_stale_lock_remove_failed",
            "a verified stale Niko migration lock could not be cleared",
            None,
        )
    })?;
    sync_parent(&lock)
}

fn operation_is_unstarted(
    roots: &ApprovedRoots,
    migration_id: &str,
) -> Result<bool, MigrationError> {
    for (_, root) in roots.distinct() {
        let operation = operation_root(root, migration_id);
        let metadata = match fs::symlink_metadata(&operation) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(classify_io(
                    error,
                    "migration_unstarted_metadata_failed",
                    "an unstarted transaction directory could not be inspected",
                    None,
                ))
            }
        };
        if !metadata.file_type().is_dir() {
            return Ok(false);
        }

        let entries = fs::read_dir(&operation).map_err(|error| {
            classify_io(
                error,
                "migration_unstarted_directory_unreadable",
                "an unstarted transaction directory could not be read",
                None,
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                classify_io(
                    error,
                    "migration_unstarted_entry_unreadable",
                    "an unstarted transaction directory entry could not be read",
                    None,
                )
            })?;
            let file_type = entry.file_type().map_err(|error| {
                classify_io(
                    error,
                    "migration_unstarted_entry_type_failed",
                    "an unstarted transaction directory entry could not be inspected",
                    None,
                )
            })?;
            match entry.file_name().to_str() {
                Some("backup" | "staged") if file_type.is_dir() => {
                    if !directory_is_empty(&entry.path())? {
                        return Ok(false);
                    }
                }
                Some("journal.json.tmp") if file_type.is_file() => {}
                _ => return Ok(false),
            }
        }
    }
    Ok(true)
}

fn directory_is_empty(path: &Path) -> Result<bool, MigrationError> {
    let mut entries = fs::read_dir(path).map_err(|error| {
        classify_io(
            error,
            "migration_unstarted_child_unreadable",
            "an unstarted transaction child directory could not be read",
            None,
        )
    })?;
    Ok(entries.next().is_none())
}

fn clear_verified_stale_provider_lock(
    roots: &ApprovedRoots,
    journals: &[MigrationJournal],
) -> Result<(), MigrationError> {
    let lock = provider_sync_lock_path(roots);
    let metadata = match fs::symlink_metadata(&lock) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(classify_io(
                error,
                "provider_sync_lock_unreadable",
                "the provider synchronization lock could not be inspected",
                None,
            ))
        }
    };
    if !metadata.file_type().is_dir() {
        return Ok(());
    }
    let bytes = match fs::read(lock.join(OWNER_FILE)) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(()),
    };
    let provider_owner: ProviderLockOwner =
        match serde_json::from_slice::<ProviderLockOwner>(&bytes) {
            Ok(owner) if owner.owner_kind == "niko" => owner,
            _ => return Ok(()),
        };
    let journal = journals
        .iter()
        .find(|journal| journal.migration_id == provider_owner.migration.journal_id);
    let Some(journal) = journal else {
        return Ok(());
    };
    if provider_owner.migration != journal.owner
        || provider_owner.migration.root_fingerprint != path_fingerprint(&roots.codex)
    {
        return Ok(());
    }
    if process_identity_is_live(
        provider_owner.migration.pid,
        provider_owner.migration.process_start_time,
    ) {
        return Ok(());
    }
    fs::remove_file(lock.join(OWNER_FILE)).map_err(|error| {
        classify_io(
            error,
            "provider_sync_stale_owner_remove_failed",
            "a verified stale provider lock owner could not be removed",
            None,
        )
    })?;
    fs::remove_dir(&lock).map_err(|error| {
        classify_io(
            error,
            "provider_sync_stale_lock_remove_failed",
            "a verified stale provider synchronization lock could not be cleared",
            None,
        )
    })?;
    sync_parent(&lock)
}

fn process_identity_is_live(pid: u32, process_start_time: u64) -> bool {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system
        .process(Pid::from_u32(pid))
        .is_some_and(|process| process.start_time() == process_start_time)
}

fn new_lock_owner(migration_id: &str, roots: &ApprovedRoots) -> Result<LockOwner, MigrationError> {
    let pid = std::process::id();
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let process_start_time = system
        .process(Pid::from_u32(pid))
        .map(|process| process.start_time())
        .ok_or_else(|| {
            migration_error(
                MigrationErrorKind::Io,
                "niko_owner_identity_unavailable",
                "the current Niko process identity could not be verified",
                None,
            )
        })?;
    Ok(LockOwner {
        journal_id: migration_id.to_owned(),
        nonce: digest_text(&format!(
            "{migration_id}:{pid}:{process_start_time}:{}",
            MIGRATION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        )),
        pid,
        process_start_time,
        root_fingerprint: path_fingerprint(&roots.codex),
    })
}

fn simulate_process_exit(roots: &ApprovedRoots, journal: &mut MigrationJournal) {
    journal.owner.pid = u32::MAX;
    journal.owner.process_start_time = 0;
    journal.updated_at_millis = unix_millis();
    let _ = persist_journal(roots, journal);
    let _ = persist_owner_file(roots, journal);
}

fn create_operation_directories(
    roots: &ApprovedRoots,
    migration_id: &str,
) -> Result<(), MigrationError> {
    for (_, root) in roots.distinct() {
        let operation = operation_root(root, migration_id);
        for child in ["backup", "staged"] {
            fs::create_dir_all(operation.join(child)).map_err(|error| {
                classify_io(
                    error,
                    "migration_operation_directory_failed",
                    "a transaction directory could not be created",
                    None,
                )
            })?;
        }
        sync_directory(&operation)?;
        sync_parent(&operation)?;
    }
    Ok(())
}

fn cleanup_unstarted_operation(roots: &ApprovedRoots, migration_id: &str) {
    if !is_migration_id(migration_id) {
        return;
    }
    for (_, root) in roots.distinct() {
        let operation = operation_root(root, migration_id);
        if operation.is_dir() {
            let _ = fs::remove_dir_all(operation);
        }
    }
}

fn prune_completed_transactions(
    roots: &ApprovedRoots,
    retained: usize,
    current: Option<&str>,
) -> Result<(), MigrationError> {
    let mut completed = read_journals(roots)?
        .into_iter()
        .filter(|journal| journal.state.is_terminal())
        .collect::<Vec<_>>();
    completed.sort_by(|left, right| {
        right
            .updated_at_millis
            .cmp(&left.updated_at_millis)
            .then_with(|| right.migration_id.cmp(&left.migration_id))
    });
    let keep = retained.max(1);
    let mut retained_ids = completed
        .iter()
        .take(keep)
        .map(|journal| journal.migration_id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(current) = current {
        retained_ids.insert(current.to_owned());
    }
    for journal in completed {
        if retained_ids.contains(&journal.migration_id) {
            continue;
        }
        if !is_migration_id(&journal.migration_id) {
            return Err(migration_error(
                MigrationErrorKind::JournalCorrupt,
                "migration_retention_id_invalid",
                "a retained transaction has an invalid identity",
                None,
            ));
        }
        for (_, root) in roots.distinct() {
            let operation = operation_root(root, &journal.migration_id);
            if operation.is_dir() {
                fs::remove_dir_all(&operation).map_err(|error| {
                    classify_io(
                        error,
                        "migration_retention_remove_failed",
                        "an expired transaction backup could not be removed",
                        None,
                    )
                })?;
                sync_parent(&operation)?;
            }
        }
    }
    Ok(())
}

fn operation_root(root: &Path, migration_id: &str) -> PathBuf {
    root.join(TRANSACTION_DIRECTORY).join(migration_id)
}

fn journal_path(roots: &ApprovedRoots, migration_id: &str) -> PathBuf {
    operation_root(&roots.codex, migration_id).join(JOURNAL_FILE)
}

fn owner_path(roots: &ApprovedRoots, migration_id: &str) -> PathBuf {
    operation_root(&roots.codex, migration_id).join(OWNER_FILE)
}

fn backup_path(
    roots: &ApprovedRoots,
    migration_id: &str,
    entry: &JournalEntry,
) -> Result<PathBuf, MigrationError> {
    Ok(
        operation_root(roots.path(entry.locator.root)?, migration_id)
            .join("backup")
            .join(format!("{}.backup", entry.artifact_id)),
    )
}

fn staged_path(
    roots: &ApprovedRoots,
    migration_id: &str,
    entry: &JournalEntry,
) -> Result<PathBuf, MigrationError> {
    Ok(
        operation_root(roots.path(entry.locator.root)?, migration_id)
            .join("staged")
            .join(format!("{}.stage", entry.artifact_id)),
    )
}

fn niko_lock_path(roots: &ApprovedRoots) -> PathBuf {
    roots.codex.join("tmp").join(NIKO_LOCK_FILE)
}

fn provider_sync_lock_path(roots: &ApprovedRoots) -> PathBuf {
    roots.codex.join("tmp").join(PROVIDER_SYNC_LOCK_DIRECTORY)
}

fn path_entry_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn resolve_locator(
    roots: &ApprovedRoots,
    locator: &ArtifactLocator,
) -> Result<PathBuf, MigrationError> {
    if locator.relative_path.as_os_str().is_empty()
        || locator
            .relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(migration_error(
            MigrationErrorKind::JournalCorrupt,
            "migration_manifest_path_invalid",
            "the migration manifest contains an unsafe artifact path",
            None,
        ));
    }
    Ok(roots.path(locator.root)?.join(&locator.relative_path))
}

fn is_migration_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sqlite_consistent_backup(
    source_path: &Path,
    destination_path: &Path,
    options: &MigrationOptions,
    artifact_id: &str,
) -> Result<(), MigrationError> {
    if destination_path.exists() {
        fs::remove_file(destination_path).map_err(|error| {
            classify_io(
                error,
                "migration_backup_replace_failed",
                "an incomplete SQLite backup could not be replaced",
                Some(artifact_id.to_owned()),
            )
        })?;
    }
    let source = Connection::open_with_flags(
        source_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    source
        .busy_timeout(Duration::ZERO)
        .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    let mut destination = Connection::open(destination_path)
        .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    {
        let backup = Backup::new(&source, &mut destination)
            .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
        let mut busy_attempts = 0;
        loop {
            match backup
                .step(128)
                .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?
            {
                StepResult::Done => break,
                StepResult::More => busy_attempts = 0,
                StepResult::Busy | StepResult::Locked => {
                    if busy_attempts >= options.busy_retries {
                        return Err(migration_error(
                            MigrationErrorKind::SqliteBusy,
                            "migration_sqlite_backup_busy",
                            "a SQLite database remained busy during consistent backup",
                            Some(artifact_id.to_owned()),
                        ));
                    }
                    busy_attempts += 1;
                    thread::sleep(options.busy_retry_delay);
                }
                _ => {
                    return Err(migration_error(
                        MigrationErrorKind::Io,
                        "migration_sqlite_backup_unknown",
                        "SQLite returned an unsupported backup state",
                        Some(artifact_id.to_owned()),
                    ))
                }
            }
        }
    }
    destination
        .pragma_update(None, "journal_mode", "DELETE")
        .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    drop(destination);
    File::open(destination_path)
        .and_then(|file| file.sync_all())
        .map_err(|error| {
            classify_io(
                error,
                "migration_sqlite_backup_sync_failed",
                "a consistent SQLite backup could not be flushed",
                Some(artifact_id.to_owned()),
            )
        })?;
    sync_parent(destination_path)
}

fn apply_sqlite_mutation(
    path: &Path,
    mutation: &SqliteMutation,
    artifact_id: &str,
) -> Result<(), MigrationError> {
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    for update in &mutation.state_updates {
        let changed = transaction
            .execute(
                "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND model_provider = ?3",
                params![
                    update.target_provider,
                    update.thread_id,
                    update.source_provider
                ],
            )
            .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
        if changed != 1 {
            return Err(migration_error(
                MigrationErrorKind::SourceChanged,
                "migration_sqlite_state_raced",
                "a SQLite provider row changed after inventory",
                Some(artifact_id.to_owned()),
            ));
        }
    }
    for update in &mutation.history_updates {
        transaction
            .execute(
                "UPDATE thread_turns
                 SET rollout_byte_offset = rollout_byte_offset + ?1,
                     rollout_end_byte_offset = CASE
                         WHEN rollout_end_byte_offset IS NULL THEN NULL
                         ELSE rollout_end_byte_offset + ?1 END
                 WHERE thread_id = ?2",
                params![update.delta, update.thread_id],
            )
            .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
        let changed = transaction
            .execute(
                "UPDATE thread_history_projection_state
                 SET next_rollout_byte_offset = next_rollout_byte_offset + ?1
                 WHERE thread_id = ?2 AND next_rollout_byte_offset = ?3",
                params![update.delta, update.thread_id, update.source_offset],
            )
            .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
        if changed != 1 {
            return Err(migration_error(
                MigrationErrorKind::SourceChanged,
                "migration_sqlite_history_raced",
                "a SQLite history cursor changed after inventory",
                Some(artifact_id.to_owned()),
            ));
        }
    }
    transaction
        .commit()
        .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    connection
        .pragma_update(None, "journal_mode", "DELETE")
        .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    drop(connection);
    remove_sqlite_sidecars(path, artifact_id)
}

fn normalize_sqlite_target(path: &Path, artifact_id: &str) -> Result<(), MigrationError> {
    if !path.exists() {
        return Ok(());
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    connection
        .busy_timeout(Duration::ZERO)
        .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    let mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    if mode.eq_ignore_ascii_case("wal") {
        let (busy, _, _): (i64, i64, i64) = connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
        if busy != 0 {
            return Err(migration_error(
                MigrationErrorKind::SqliteBusy,
                "migration_sqlite_checkpoint_busy",
                "a SQLite WAL remained busy during commit",
                Some(artifact_id.to_owned()),
            ));
        }
        connection
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    }
    drop(connection);
    remove_sqlite_sidecars(path, artifact_id)
}

fn verify_sqlite_file(path: &Path, artifact_id: &str) -> Result<(), MigrationError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    let result: String = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(|error| classify_sqlite(error, Some(artifact_id.to_owned())))?;
    if result != "ok" {
        return Err(migration_error(
            MigrationErrorKind::CorruptStorage,
            "migration_sqlite_integrity_failed",
            "a SQLite backup or staged database failed integrity validation",
            Some(artifact_id.to_owned()),
        ));
    }
    Ok(())
}

fn remove_sqlite_sidecars(path: &Path, artifact_id: &str) -> Result<(), MigrationError> {
    for suffix in ["-wal", "-shm"] {
        let sidecar = sqlite_sidecar_path(path, suffix);
        match fs::remove_file(&sidecar) {
            Ok(()) => sync_parent(&sidecar)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(classify_io(
                    error,
                    "migration_sqlite_sidecar_occupied",
                    "a SQLite WAL/SHM sidecar remains occupied",
                    Some(artifact_id.to_owned()),
                ))
            }
        }
    }
    Ok(())
}

fn ensure_sidecars_absent(path: &Path, artifact_id: &str) -> Result<(), MigrationError> {
    if ["-wal", "-shm"]
        .into_iter()
        .any(|suffix| sqlite_sidecar_path(path, suffix).exists())
    {
        return Err(migration_error(
            MigrationErrorKind::ValidationFailed,
            "migration_sqlite_sidecar_present",
            "a committed SQLite database retained an unexpected WAL/SHM sidecar",
            Some(artifact_id.to_owned()),
        ));
    }
    Ok(())
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    value.into()
}

fn sqlite_sidecar_size(path: &Path, suffix: &str) -> u64 {
    fs::metadata(sqlite_sidecar_path(path, suffix))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn sqlite_source_hash(path: &Path) -> Result<String, MigrationError> {
    let artifact = digest_text(&path_fingerprint(path));
    let mut hasher = Sha256::new();
    hash_file_into(&mut hasher, path, b"main\0", Some(&artifact))?;
    let wal = sqlite_sidecar_path(path, "-wal");
    if wal.is_file() {
        hash_file_into(&mut hasher, &wal, b"wal\0", Some(&artifact))?;
    } else {
        hasher.update(b"no-wal\0");
    }
    Ok(digest_hex(hasher.finalize().as_slice()))
}

fn sqlite_is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(
                inner.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

fn atomic_install(source: &Path, target: &Path, artifact_id: &str) -> Result<(), MigrationError> {
    let parent = target.parent().ok_or_else(|| {
        migration_error(
            MigrationErrorKind::InvalidRequest,
            "migration_target_parent_missing",
            "a migration target has no parent directory",
            Some(artifact_id.to_owned()),
        )
    })?;
    let swap = parent.join(format!(".niko-{artifact_id}.swap"));
    if swap.exists() {
        let metadata = fs::symlink_metadata(&swap).map_err(|error| {
            classify_io(
                error,
                "migration_swap_metadata_failed",
                "an interrupted atomic replacement could not be inspected",
                Some(artifact_id.to_owned()),
            )
        })?;
        if !metadata.file_type().is_file() {
            return Err(migration_error(
                MigrationErrorKind::FileOccupied,
                "migration_swap_occupied",
                "an atomic replacement staging name is occupied",
                Some(artifact_id.to_owned()),
            ));
        }
        fs::remove_file(&swap).map_err(|error| {
            classify_io(
                error,
                "migration_swap_remove_failed",
                "an interrupted atomic replacement could not be cleared",
                Some(artifact_id.to_owned()),
            )
        })?;
    }
    copy_file_synced(source, &swap, Some(artifact_id))?;
    atomic_replace_file(&swap, target, Some(artifact_id))?;
    sync_parent(target)
}

#[cfg(unix)]
fn atomic_replace_file(
    replacement: &Path,
    target: &Path,
    artifact_id: Option<&str>,
) -> Result<(), MigrationError> {
    fs::rename(replacement, target).map_err(|error| {
        classify_io(
            error,
            "migration_atomic_replace_failed",
            "an artifact could not be atomically replaced",
            artifact_id.map(str::to_owned),
        )
    })
}

#[cfg(windows)]
fn atomic_replace_file(
    replacement: &Path,
    target: &Path,
    artifact_id: Option<&str>,
) -> Result<(), MigrationError> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *mut core::ffi::c_void,
            reserved: *mut core::ffi::c_void,
        ) -> i32;
        fn MoveFileExW(existing: *const u16, new_name: *const u16, flags: u32) -> i32;
    }

    const REPLACEFILE_WRITE_THROUGH: u32 = 0x0000_0001;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let replacement_wide = wide(replacement);
    let target_wide = wide(target);
    let result = unsafe {
        if target.exists() {
            ReplaceFileW(
                target_wide.as_ptr(),
                replacement_wide.as_ptr(),
                ptr::null(),
                REPLACEFILE_WRITE_THROUGH,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } else {
            MoveFileExW(
                replacement_wide.as_ptr(),
                target_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    if result == 0 {
        return Err(classify_io(
            io::Error::last_os_error(),
            "migration_atomic_replace_failed",
            "an artifact could not be atomically replaced",
            artifact_id.map(str::to_owned),
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn atomic_replace_file(
    replacement: &Path,
    target: &Path,
    artifact_id: Option<&str>,
) -> Result<(), MigrationError> {
    if target.exists() {
        fs::remove_file(target).map_err(|error| {
            classify_io(
                error,
                "migration_atomic_replace_failed",
                "an artifact could not be replaced",
                artifact_id.map(str::to_owned),
            )
        })?;
    }
    fs::rename(replacement, target).map_err(|error| {
        classify_io(
            error,
            "migration_atomic_replace_failed",
            "an artifact could not be replaced",
            artifact_id.map(str::to_owned),
        )
    })
}

fn write_file_synced(
    path: &Path,
    bytes: &[u8],
    artifact_id: Option<&str>,
) -> Result<(), MigrationError> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            classify_io(
                error,
                "migration_file_create_failed",
                "a transaction file could not be created",
                artifact_id.map(str::to_owned),
            )
        })?;
    file.write_all(bytes).map_err(|error| {
        classify_io(
            error,
            "migration_file_write_failed",
            "a transaction file could not be written",
            artifact_id.map(str::to_owned),
        )
    })?;
    file.sync_all().map_err(|error| {
        classify_io(
            error,
            "migration_file_sync_failed",
            "a transaction file could not be flushed",
            artifact_id.map(str::to_owned),
        )
    })?;
    sync_parent(path)
}

fn copy_file_synced(
    source: &Path,
    destination: &Path,
    artifact_id: Option<&str>,
) -> Result<(), MigrationError> {
    fs::copy(source, destination).map_err(|error| {
        classify_io(
            error,
            "migration_file_copy_failed",
            "a storage artifact could not be copied into the transaction",
            artifact_id.map(str::to_owned),
        )
    })?;
    File::open(destination)
        .and_then(|file| file.sync_all())
        .map_err(|error| {
            classify_io(
                error,
                "migration_file_copy_sync_failed",
                "a copied storage artifact could not be flushed",
                artifact_id.map(str::to_owned),
            )
        })?;
    sync_parent(destination)
}

fn copy_permissions_if_present(source: &Path, destination: &Path) -> Result<(), MigrationError> {
    if !source.exists() {
        return Ok(());
    }
    let permissions = fs::metadata(source)
        .map_err(|error| {
            classify_io(
                error,
                "migration_permissions_read_failed",
                "artifact permissions could not be read",
                None,
            )
        })?
        .permissions();
    fs::set_permissions(destination, permissions).map_err(|error| {
        classify_io(
            error,
            "migration_permissions_apply_failed",
            "artifact permissions could not be preserved",
            None,
        )
    })?;
    File::open(destination)
        .and_then(|file| file.sync_all())
        .map_err(|error| {
            classify_io(
                error,
                "migration_permissions_sync_failed",
                "artifact permissions could not be flushed",
                None,
            )
        })?;
    sync_parent(destination)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), MigrationError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            classify_io(
                error,
                "migration_directory_sync_failed",
                "transaction directory metadata could not be flushed",
                None,
            )
        })
}

#[cfg(windows)]
fn sync_directory(_path: &Path) -> Result<(), MigrationError> {
    // ReplaceFileW/MoveFileExW use write-through. Opening directories for
    // FlushFileBuffers requires platform-specific privileges and is not
    // consistently available to desktop applications.
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_path: &Path) -> Result<(), MigrationError> {
    Ok(())
}

fn sync_parent(path: &Path) -> Result<(), MigrationError> {
    match path.parent() {
        Some(parent) => sync_directory(parent),
        None => Ok(()),
    }
}

fn sha256_file(path: &Path, artifact_id: Option<&str>) -> Result<String, MigrationError> {
    let mut hasher = Sha256::new();
    hash_file_into(&mut hasher, path, b"", artifact_id)?;
    Ok(digest_hex(hasher.finalize().as_slice()))
}

fn hash_file_into(
    hasher: &mut Sha256,
    path: &Path,
    label: &[u8],
    artifact_id: Option<&str>,
) -> Result<(), MigrationError> {
    let mut file = File::open(path).map_err(|error| {
        classify_io(
            error,
            "migration_hash_open_failed",
            "a storage artifact could not be opened for hashing",
            artifact_id.map(str::to_owned),
        )
    })?;
    let length = file
        .metadata()
        .map(|metadata| metadata.len())
        .map_err(|error| {
            classify_io(
                error,
                "migration_hash_metadata_failed",
                "storage artifact metadata could not be hashed",
                artifact_id.map(str::to_owned),
            )
        })?;
    hasher.update(label);
    hasher.update(length.to_le_bytes());
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            classify_io(
                error,
                "migration_hash_read_failed",
                "a storage artifact could not be hashed",
                artifact_id.map(str::to_owned),
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(())
}

fn verify_expected_hash(
    path: &Path,
    expected: Option<&str>,
    entry: &JournalEntry,
) -> Result<(), MigrationError> {
    let expected = expected.ok_or_else(|| {
        migration_error(
            MigrationErrorKind::JournalCorrupt,
            "migration_manifest_hash_missing",
            "the migration manifest lacks a required hash",
            Some(entry.artifact_id.clone()),
        )
    })?;
    if sha256_file(path, Some(&entry.artifact_id))? != expected {
        return Err(migration_error(
            MigrationErrorKind::BackupHashMismatch,
            "migration_hash_mismatch",
            "a backup, staged artifact, or committed artifact failed hash validation",
            Some(entry.artifact_id.clone()),
        ));
    }
    Ok(())
}

fn artifact_id(locator: &ArtifactLocator) -> String {
    let root = match locator.root {
        RootSlot::Codex => "codex",
        RootSlot::Sqlite => "sqlite",
    };
    let value = format!("{root}:{}", locator.relative_path.to_string_lossy());
    digest_text(&value)[..20].to_owned()
}

fn path_fingerprint(path: &Path) -> String {
    digest_text(&path.to_string_lossy())
}

fn digest_text(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    digest_hex(hasher.finalize().as_slice())
}

fn new_migration_id() -> String {
    let sequence = MIGRATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let value = format!("{}:{}:{}", unix_nanos(), std::process::id(), sequence);
    digest_text(&value)[..32].to_owned()
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn inject(
    faults: &dyn MigrationFaultInjector,
    point: FaultPoint,
    artifact_id: Option<&str>,
) -> Result<(), MigrationError> {
    let Some(kind) = faults.inject(point, artifact_id) else {
        return Ok(());
    };
    let artifact_id = artifact_id.map(str::to_owned);
    let (error_kind, code, message) = match kind {
        InjectedFaultKind::Crash => (
            MigrationErrorKind::InjectedCrash,
            "migration_injected_crash",
            "a deterministic crash was injected",
        ),
        InjectedFaultKind::ProviderSyncLocked => (
            MigrationErrorKind::ProviderSyncLocked,
            "provider_sync_locked",
            "Codex++ provider synchronization is in progress; retry later",
        ),
        InjectedFaultKind::CodexRunning => (
            MigrationErrorKind::CodexRunning,
            "codex_still_running",
            "Codex is still running; close Codex and retry",
        ),
        InjectedFaultKind::PermissionDenied => (
            MigrationErrorKind::PermissionDenied,
            "migration_permission_denied",
            "a required storage location is not writable",
        ),
        InjectedFaultKind::InsufficientSpace => (
            MigrationErrorKind::InsufficientSpace,
            "migration_space_insufficient",
            "the approved storage root lacks sufficient space",
        ),
        InjectedFaultKind::SqliteBusy => (
            MigrationErrorKind::SqliteBusy,
            "migration_sqlite_busy",
            "a writable SQLite database remains busy",
        ),
        InjectedFaultKind::FileOccupied => (
            MigrationErrorKind::FileOccupied,
            "migration_file_occupied",
            "a mutable storage file remains occupied",
        ),
        InjectedFaultKind::HashMismatch => (
            MigrationErrorKind::BackupHashMismatch,
            "migration_hash_mismatch",
            "a deterministic hash failure was injected",
        ),
        InjectedFaultKind::ValidationFailed => (
            MigrationErrorKind::ValidationFailed,
            "migration_validation_failed",
            "a deterministic validation failure was injected",
        ),
    };
    Err(migration_error(error_kind, code, message, artifact_id))
}

fn redact_fixture_error(_error: FixtureMutationError) -> MigrationError {
    migration_error(
        MigrationErrorKind::ValidationFailed,
        "migration_roundtrip_plan_failed",
        "the provider-neutral round-trip proof could not be constructed",
        None,
    )
}

fn classify_sqlite(error: rusqlite::Error, artifact_id: Option<String>) -> MigrationError {
    if sqlite_is_busy(&error) {
        return migration_error(
            MigrationErrorKind::SqliteBusy,
            "migration_sqlite_busy",
            "a writable SQLite database remains busy; close Codex and retry",
            artifact_id,
        );
    }
    if matches!(
        error,
        rusqlite::Error::SqliteFailure(ref inner, _)
            if matches!(
                inner.code,
                rusqlite::ErrorCode::PermissionDenied
                    | rusqlite::ErrorCode::ReadOnly
                    | rusqlite::ErrorCode::CannotOpen
            )
    ) {
        return migration_error(
            MigrationErrorKind::PermissionDenied,
            "migration_sqlite_permission_denied",
            "a SQLite database is not writable",
            artifact_id,
        );
    }
    migration_error(
        MigrationErrorKind::Io,
        "migration_sqlite_operation_failed",
        "a SQLite backup or transaction operation failed",
        artifact_id,
    )
}

fn classify_io(
    error: io::Error,
    code: &'static str,
    message: &'static str,
    artifact_id: Option<String>,
) -> MigrationError {
    let raw = error.raw_os_error();
    let kind = if error.kind() == io::ErrorKind::PermissionDenied {
        MigrationErrorKind::PermissionDenied
    } else if error.kind() == io::ErrorKind::WouldBlock || matches!(raw, Some(16 | 26 | 32 | 33)) {
        MigrationErrorKind::FileOccupied
    } else {
        MigrationErrorKind::Io
    };
    migration_error(kind, code, message, artifact_id)
}

fn migration_error(
    kind: MigrationErrorKind,
    code: &'static str,
    message: &'static str,
    artifact_id: Option<String>,
) -> MigrationError {
    let retryable = matches!(
        kind,
        MigrationErrorKind::NikoLocked
            | MigrationErrorKind::ProviderSyncLocked
            | MigrationErrorKind::CodexRunning
            | MigrationErrorKind::SqliteBusy
            | MigrationErrorKind::FileOccupied
            | MigrationErrorKind::SourceChanged
            | MigrationErrorKind::InsufficientSpace
    );
    MigrationError {
        kind,
        code,
        message,
        artifact_id,
        retryable,
        restart_allowed: false,
    }
}
