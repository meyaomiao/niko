//! Isolated Codex session storage inventory and fixture-only migration PoC.
//!
//! Portions of the path resolution and provider-bucket migration are adapted
//! from CC Switch commit 606e7bbe75db7f8285f7a3be006fac22b5d22796,
//! Copyright (c) 2025 Jason Young, under the MIT License. See
//! `third_party/licenses/CC-Switch-MIT.txt` and `THIRD_PARTY_NOTICES.md`.
//!
//! This module deliberately has no home-directory or environment fallback.
//! Callers must provide the Codex home, and must separately approve an
//! external SQLite home before configuration can lead the scan outside it.

mod transaction;

pub use transaction::*;

use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const CUSTOM_PROVIDER: &str = "custom";
pub const OFFICIAL_PROVIDER: &str = "openai";
pub const NIKO_PROVIDER: &str = "momotoken";
pub const FIXTURE_ROOT_MARKER: &str = ".niko-e10-2-fixture";
pub const FIXTURE_ROOT_MARKER_CONTENT: &str = "niko-e10-2-custom-roundtrip-poc\n";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanRequest {
    pub codex_home: PathBuf,
    pub explicit_sqlite_home: Option<PathBuf>,
}

impl ScanRequest {
    pub fn new(codex_home: impl Into<PathBuf>) -> Self {
        Self {
            codex_home: codex_home.into(),
            explicit_sqlite_home: None,
        }
    }

    /// Approves one independent SQLite home. This is also the only supported
    /// way for a caller to supply a `CODEX_SQLITE_HOME` equivalent.
    pub fn with_sqlite_home(mut self, sqlite_home: impl Into<PathBuf>) -> Self {
        self.explicit_sqlite_home = Some(sqlite_home.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScanError {
    CodexHomeMustBeAbsolute(PathBuf),
    CodexHomeIsNotDirectory(PathBuf),
    SqliteHomeMustBeAbsolute(PathBuf),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CodexHomeMustBeAbsolute(path) => {
                write!(
                    f,
                    "Codex home must be an explicit absolute path: {}",
                    path.display()
                )
            }
            Self::CodexHomeIsNotDirectory(path) => {
                write!(f, "Codex home is not a directory: {}", path.display())
            }
            Self::SqliteHomeMustBeAbsolute(path) => {
                write!(
                    f,
                    "SQLite home must be an explicit absolute path: {}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ScanError {}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DiagnosticLevel {
    Warning,
    Blocker,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub code: &'static str,
    pub message: String,
    pub path: Option<PathBuf>,
    pub thread_id: Option<String>,
}

impl Diagnostic {
    fn blocker(code: &'static str, message: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            level: DiagnosticLevel::Blocker,
            code,
            message: message.into(),
            path,
            thread_id: None,
        }
    }

    fn for_thread(
        level: DiagnosticLevel,
        code: &'static str,
        message: impl Into<String>,
        thread_id: &str,
    ) -> Self {
        Self {
            level,
            code,
            message: message.into(),
            path: None,
            thread_id: Some(thread_id.to_owned()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SqliteHomeSource {
    CodexHome,
    Config,
    Explicit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigInventory {
    pub path: PathBuf,
    pub present: bool,
    pub active_provider: Option<String>,
    pub defined_providers: Vec<String>,
    pub effective_sqlite_home: PathBuf,
    pub sqlite_home_source: SqliteHomeSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RolloutEncoding {
    Jsonl,
    Zstd,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RolloutArtifact {
    pub path: PathBuf,
    pub logical_path: PathBuf,
    pub thread_id: String,
    pub provider: String,
    pub workspace: Option<PathBuf>,
    pub archived: bool,
    pub encoding: RolloutEncoding,
    pub cli_version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionIndexArtifact {
    pub path: PathBuf,
    pub byte_size: u64,
    pub entry_count: usize,
    pub thread_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SqliteSchemaKind {
    State,
    ThreadHistory,
    StateAndThreadHistory,
    Auxiliary,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SqliteSidecarKind {
    Wal,
    Shm,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteSidecar {
    pub kind: SqliteSidecarKind,
    pub path: PathBuf,
    pub byte_size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteIndex {
    pub name: String,
    pub table: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateThreadRow {
    pub thread_id: String,
    pub rollout_path: PathBuf,
    pub provider: String,
    pub workspace: PathBuf,
    pub archived: bool,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub updated_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistoryThreadRow {
    pub thread_id: String,
    pub turn_count: u64,
    pub item_count: u64,
    pub first_ordinal: Option<i64>,
    pub last_ordinal: Option<i64>,
    pub next_rollout_byte_offset: Option<i64>,
    pub next_rollout_ordinal: Option<i64>,
    pub turns: Vec<HistoryTurnCursor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryTurnCursor {
    pub turn_id: String,
    pub rollout_ordinal: i64,
    pub rollout_byte_offset: Option<i64>,
    pub rollout_end_ordinal: Option<i64>,
    pub rollout_end_byte_offset: Option<i64>,
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteArtifact {
    pub path: PathBuf,
    pub byte_size: u64,
    pub readable: bool,
    pub schema_kind: SqliteSchemaKind,
    pub user_version: Option<i64>,
    pub migration_version: Option<i64>,
    pub tables: Vec<String>,
    pub indexes: Vec<SqliteIndex>,
    pub sidecars: Vec<SqliteSidecar>,
    pub state_rows: Vec<StateThreadRow>,
    pub history_rows: Vec<HistoryThreadRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadInventory {
    pub thread_id: String,
    pub rollout_paths: Vec<PathBuf>,
    pub state_databases: Vec<PathBuf>,
    pub history_databases: Vec<PathBuf>,
    pub providers: Vec<String>,
    pub workspaces: Vec<PathBuf>,
    pub archived: Option<bool>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub updated_at_ms: Option<i64>,
    pub storage_versions: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderLayout {
    Empty,
    Official,
    CcSwitchCustom,
    NikoMomotoken,
    CodexPlusPlusCompatible,
    Mixed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NormalizationStatus {
    NoChanges,
    WouldNormalize,
    Blocked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanAction {
    ConfigureCustomBucket {
        from: String,
        to: String,
    },
    RewriteRolloutHeader {
        path: PathBuf,
        thread_id: String,
        from: String,
        to: String,
    },
    UpdateStateRow {
        database: PathBuf,
        thread_id: String,
        from: String,
        to: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizationPlan {
    pub status: NormalizationStatus,
    pub target_provider: String,
    pub actions: Vec<PlanAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanReport {
    pub codex_home: PathBuf,
    pub config: ConfigInventory,
    pub rollouts: Vec<RolloutArtifact>,
    pub session_index: Option<SessionIndexArtifact>,
    pub sqlite_databases: Vec<SqliteArtifact>,
    pub threads: Vec<ThreadInventory>,
    pub provider_layout: ProviderLayout,
    pub diagnostics: Vec<Diagnostic>,
    pub normalization: NormalizationPlan,
}

impl ScanReport {
    pub fn is_blocked(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|item| item.level == DiagnosticLevel::Blocker)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixtureProviderTarget {
    Custom,
    OpenAi,
}

impl FixtureProviderTarget {
    fn provider(self) -> &'static str {
        match self {
            Self::Custom => CUSTOM_PROVIDER,
            Self::OpenAi => OFFICIAL_PROVIDER,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureHistoryProof {
    pub database: PathBuf,
    pub row: HistoryThreadRow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureThreadProof {
    pub thread_id: String,
    pub rollout_path: PathBuf,
    pub state_databases: Vec<PathBuf>,
    pub workspace: Option<PathBuf>,
    pub archived: bool,
    pub visible_event_count: usize,
    pub visible_history_digest: String,
    pub provider_neutral_digest: String,
    pub history: Vec<FixtureHistoryProof>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureMigrationReport {
    pub target: FixtureProviderTarget,
    pub changed_paths: Vec<PathBuf>,
    pub before: ScanReport,
    pub after: ScanReport,
    pub before_threads: Vec<FixtureThreadProof>,
    pub after_threads: Vec<FixtureThreadProof>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureAppendReport {
    pub thread_id: String,
    pub turn_id: String,
    pub start_ordinal: i64,
    pub end_ordinal: i64,
    pub start_byte_offset: i64,
    pub end_byte_offset: i64,
    pub before: FixtureThreadProof,
    pub after: FixtureThreadProof,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureMutationError {
    pub code: &'static str,
    pub message: String,
    pub path: Option<PathBuf>,
}

impl fmt::Display for FixtureMutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for FixtureMutationError {}

/// Inventories an explicitly supplied Codex home without writing to it.
pub fn scan_codex_sessions(request: &ScanRequest) -> Result<ScanReport, ScanError> {
    validate_request(request)?;

    let mut diagnostics = Vec::new();
    let (config, sqlite_homes) = inspect_config(request, &mut diagnostics);
    let mut rollouts = scan_rollouts(&request.codex_home, &mut diagnostics);
    rollouts.sort_by(|left, right| left.path.cmp(&right.path));
    let session_index = inspect_session_index(&request.codex_home, &mut diagnostics);

    let database_paths =
        discover_sqlite_databases(&request.codex_home, &sqlite_homes, &mut diagnostics);
    let sqlite_databases = database_paths
        .iter()
        .map(|path| inspect_sqlite(path, &mut diagnostics))
        .collect::<Vec<_>>();

    let threads = build_thread_inventory(
        &request.codex_home,
        &rollouts,
        &sqlite_databases,
        &mut diagnostics,
    );
    let provider_layout = detect_provider_layout(&config, &rollouts, &sqlite_databases);
    diagnostics.sort_by(|left, right| {
        left.level
            .cmp(&right.level)
            .then_with(|| left.code.cmp(right.code))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    let normalization = build_normalization_plan(
        &config,
        &rollouts,
        &sqlite_databases,
        diagnostics
            .iter()
            .any(|item| item.level == DiagnosticLevel::Blocker),
    );

    Ok(ScanReport {
        codex_home: request.codex_home.clone(),
        config,
        rollouts,
        session_index,
        sqlite_databases,
        threads,
        provider_layout,
        diagnostics,
        normalization,
    })
}

fn validate_request(request: &ScanRequest) -> Result<(), ScanError> {
    if !request.codex_home.is_absolute() {
        return Err(ScanError::CodexHomeMustBeAbsolute(
            request.codex_home.clone(),
        ));
    }
    if !request.codex_home.is_dir() {
        return Err(ScanError::CodexHomeIsNotDirectory(
            request.codex_home.clone(),
        ));
    }
    if let Some(sqlite_home) = &request.explicit_sqlite_home {
        if !sqlite_home.is_absolute() {
            return Err(ScanError::SqliteHomeMustBeAbsolute(sqlite_home.clone()));
        }
    }
    Ok(())
}

fn inspect_config(
    request: &ScanRequest,
    diagnostics: &mut Vec<Diagnostic>,
) -> (ConfigInventory, Vec<PathBuf>) {
    let config_path = request.codex_home.join("config.toml");
    let mut inventory = ConfigInventory {
        path: config_path.clone(),
        present: false,
        active_provider: Some(OFFICIAL_PROVIDER.to_owned()),
        defined_providers: Vec::new(),
        effective_sqlite_home: request.codex_home.clone(),
        sqlite_home_source: SqliteHomeSource::CodexHome,
    };

    let config_text = match fs::read_to_string(&config_path) {
        Ok(text) => {
            inventory.present = true;
            Some(text)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            diagnostics.push(Diagnostic::blocker(
                "config_unreadable",
                "config.toml could not be read",
                Some(config_path),
            ));
            inventory.active_provider = None;
            None
        }
    };

    let mut configured_sqlite_home = None;
    if let Some(config_text) = config_text {
        match config_text.parse::<toml::Table>() {
            Ok(config) => {
                inventory.active_provider = match config.get("model_provider") {
                    None => Some(OFFICIAL_PROVIDER.to_owned()),
                    Some(value) => match value.as_str().map(str::trim) {
                        Some("") | None => {
                            diagnostics.push(Diagnostic::blocker(
                                "config_provider_invalid",
                                "model_provider must be a non-empty string",
                                Some(inventory.path.clone()),
                            ));
                            None
                        }
                        Some(provider) => Some(provider.to_owned()),
                    },
                };

                if let Some(value) = config.get("model_providers") {
                    if let Some(table) = value.as_table() {
                        inventory.defined_providers = table.keys().cloned().collect();
                        inventory.defined_providers.sort();
                    } else {
                        diagnostics.push(Diagnostic::blocker(
                            "config_provider_table_invalid",
                            "model_providers must be a table",
                            Some(inventory.path.clone()),
                        ));
                    }
                }

                if let Some(value) = config.get("sqlite_home") {
                    match value.as_str().map(str::trim) {
                        Some(raw) if !raw.is_empty() && Path::new(raw).is_absolute() => {
                            configured_sqlite_home = Some(PathBuf::from(raw));
                        }
                        _ => diagnostics.push(Diagnostic::blocker(
                            "config_sqlite_home_invalid",
                            "sqlite_home must be a non-empty absolute path",
                            Some(inventory.path.clone()),
                        )),
                    }
                }
            }
            Err(_) => {
                inventory.active_provider = None;
                diagnostics.push(Diagnostic::blocker(
                    "config_toml_invalid",
                    "config.toml is not valid TOML",
                    Some(inventory.path.clone()),
                ));
            }
        }
    }

    let mut sqlite_homes = vec![request.codex_home.clone()];
    if let Some(configured) = configured_sqlite_home {
        if configured == request.codex_home
            || request.explicit_sqlite_home.as_ref() == Some(&configured)
        {
            inventory.effective_sqlite_home = configured.clone();
            inventory.sqlite_home_source = SqliteHomeSource::Config;
            push_unique(&mut sqlite_homes, configured);
        } else {
            diagnostics.push(Diagnostic::blocker(
                "sqlite_home_not_approved",
                "config.toml points outside the explicit scan roots",
                Some(configured),
            ));
        }
    } else if let Some(explicit) = &request.explicit_sqlite_home {
        inventory.effective_sqlite_home = explicit.clone();
        inventory.sqlite_home_source = SqliteHomeSource::Explicit;
        push_unique(&mut sqlite_homes, explicit.clone());
    }

    if let Some(active) = &inventory.active_provider {
        if active != OFFICIAL_PROVIDER && !inventory.defined_providers.contains(active) {
            diagnostics.push(Diagnostic::blocker(
                "active_provider_definition_missing",
                "the active custom provider has no model_providers definition",
                Some(inventory.path.clone()),
            ));
        }
    }

    (inventory, sqlite_homes)
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

fn scan_rollouts(codex_home: &Path, diagnostics: &mut Vec<Diagnostic>) -> Vec<RolloutArtifact> {
    let mut paths = Vec::new();
    collect_rollout_paths(&codex_home.join("sessions"), false, &mut paths, diagnostics);
    collect_rollout_paths(
        &codex_home.join("archived_sessions"),
        true,
        &mut paths,
        diagnostics,
    );
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    paths
        .into_iter()
        .filter_map(|(path, archived)| inspect_rollout(path, archived, diagnostics))
        .collect()
}

fn collect_rollout_paths(
    root: &Path,
    archived: bool,
    paths: &mut Vec<(PathBuf, bool)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                diagnostics.push(Diagnostic::blocker(
                    "rollout_directory_unreadable",
                    "a rollout directory could not be read",
                    Some(directory),
                ));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    diagnostics.push(Diagnostic::blocker(
                        "rollout_entry_unreadable",
                        "a rollout directory entry could not be read",
                        Some(directory.clone()),
                    ));
                    continue;
                }
            };
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    diagnostics.push(Diagnostic::blocker(
                        "rollout_entry_unreadable",
                        "a rollout entry type could not be read",
                        Some(path),
                    ));
                    continue;
                }
            };
            if file_type.is_dir() {
                directories.push(path);
            } else if is_rollout_path(&path) {
                if file_type.is_file() {
                    paths.push((path, archived));
                } else {
                    diagnostics.push(Diagnostic::blocker(
                        "rollout_entry_unsupported",
                        "a rollout entry is not a regular file",
                        Some(path),
                    ));
                }
            }
        }
    }
}

fn is_rollout_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"))
}

fn inspect_rollout(
    path: PathBuf,
    archived: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<RolloutArtifact> {
    let encoding = if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".zst"))
    {
        RolloutEncoding::Zstd
    } else {
        RolloutEncoding::Jsonl
    };

    let file = match File::open(&path) {
        Ok(file) => file,
        Err(_) => {
            diagnostics.push(Diagnostic::blocker(
                "rollout_unreadable",
                "a rollout file could not be opened",
                Some(path),
            ));
            return None;
        }
    };
    let mut reader: Box<dyn BufRead> = match encoding {
        RolloutEncoding::Jsonl => Box::new(BufReader::new(file)),
        RolloutEncoding::Zstd => match zstd::stream::read::Decoder::new(file) {
            Ok(decoder) => Box::new(BufReader::new(decoder)),
            Err(_) => {
                diagnostics.push(Diagnostic::blocker(
                    "rollout_zstd_invalid",
                    "a compressed rollout header could not be decoded",
                    Some(path),
                ));
                return None;
            }
        },
    };

    let mut first_line = String::new();
    loop {
        first_line.clear();
        match reader.read_line(&mut first_line) {
            Ok(0) => {
                diagnostics.push(Diagnostic::blocker(
                    "rollout_header_missing",
                    "a rollout has no session metadata header",
                    Some(path),
                ));
                return None;
            }
            Ok(_) if first_line.trim().is_empty() => continue,
            Ok(_) => break,
            Err(_) => {
                diagnostics.push(Diagnostic::blocker(
                    "rollout_header_unreadable",
                    "a rollout session metadata header could not be read",
                    Some(path),
                ));
                return None;
            }
        }
    }

    let value = match serde_json::from_str::<JsonValue>(first_line.trim()) {
        Ok(value) => value,
        Err(_) => {
            diagnostics.push(Diagnostic::blocker(
                "rollout_header_invalid",
                "a rollout session metadata header is not valid JSON",
                Some(path),
            ));
            return None;
        }
    };
    if value.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
        diagnostics.push(Diagnostic::blocker(
            "rollout_header_unknown",
            "the first rollout record is not session_meta",
            Some(path),
        ));
        return None;
    }
    let payload = match value.get("payload").and_then(JsonValue::as_object) {
        Some(payload) => payload,
        None => {
            diagnostics.push(Diagnostic::blocker(
                "rollout_header_invalid",
                "a rollout session_meta payload is not an object",
                Some(path),
            ));
            return None;
        }
    };
    let thread_id = payload
        .get("id")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let provider = payload
        .get("model_provider")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (thread_id, provider) = match (thread_id, provider) {
        (Some(thread_id), Some(provider)) => (thread_id.to_owned(), provider.to_owned()),
        _ => {
            diagnostics.push(Diagnostic::blocker(
                "rollout_header_capability_missing",
                "session_meta must contain non-empty id and model_provider fields",
                Some(path),
            ));
            return None;
        }
    };
    let workspace = payload
        .get("cwd")
        .and_then(JsonValue::as_str)
        .map(PathBuf::from);
    let cli_version = payload
        .get("cli_version")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    let logical_path = plain_rollout_path(&path);

    Some(RolloutArtifact {
        path,
        logical_path,
        thread_id,
        provider,
        workspace,
        archived,
        encoding,
        cli_version,
    })
}

fn plain_rollout_path(path: &Path) -> PathBuf {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".jsonl.zst"))
    {
        path.with_extension("")
    } else {
        path.to_path_buf()
    }
}

fn inspect_session_index(
    codex_home: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SessionIndexArtifact> {
    let path = codex_home.join("session_index.jsonl");
    let metadata = match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            diagnostics.push(Diagnostic::blocker(
                "session_index_not_file",
                "session_index.jsonl is not a regular file",
                Some(path),
            ));
            return None;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => {
            diagnostics.push(Diagnostic::blocker(
                "session_index_unreadable",
                "session_index.jsonl metadata could not be read",
                Some(path),
            ));
            return None;
        }
    };
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(_) => {
            diagnostics.push(Diagnostic::blocker(
                "session_index_unreadable",
                "session_index.jsonl could not be opened",
                Some(path),
            ));
            return None;
        }
    };

    let mut thread_ids = Vec::new();
    for (line_number, line) in BufReader::new(file).lines().enumerate() {
        let line = match line {
            Ok(line) => line,
            Err(_) => {
                diagnostics.push(Diagnostic::blocker(
                    "session_index_unreadable",
                    format!("session index line {} could not be read", line_number + 1),
                    Some(path.clone()),
                ));
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let thread_id = serde_json::from_str::<JsonValue>(&line)
            .ok()
            .and_then(|value| {
                value
                    .get("id")
                    .and_then(JsonValue::as_str)
                    .map(str::to_owned)
            })
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        match thread_id {
            Some(thread_id) => thread_ids.push(thread_id),
            None => diagnostics.push(Diagnostic::blocker(
                "session_index_entry_invalid",
                format!("session index line {} lacks a valid id", line_number + 1),
                Some(path.clone()),
            )),
        }
    }
    let entry_count = thread_ids.len();
    thread_ids.sort();
    thread_ids.dedup();
    Some(SessionIndexArtifact {
        path,
        byte_size: metadata.len(),
        entry_count,
        thread_ids,
    })
}

fn discover_sqlite_databases(
    codex_home: &Path,
    sqlite_homes: &[PathBuf],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    collect_database_files(codex_home, false, true, &mut paths, diagnostics);
    collect_database_files(
        &codex_home.join("sqlite"),
        true,
        false,
        &mut paths,
        diagnostics,
    );
    for sqlite_home in sqlite_homes {
        if sqlite_home == codex_home {
            continue;
        }
        collect_database_files(sqlite_home, false, true, &mut paths, diagnostics);
        collect_database_files(
            &sqlite_home.join("sqlite"),
            true,
            false,
            &mut paths,
            diagnostics,
        );
    }
    paths.into_iter().collect()
}

fn collect_database_files(
    root: &Path,
    recursive: bool,
    required: bool,
    paths: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => continue,
            Err(_) => {
                diagnostics.push(Diagnostic::blocker(
                    "sqlite_directory_unreadable",
                    "a configured SQLite directory could not be read",
                    Some(directory),
                ));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    diagnostics.push(Diagnostic::blocker(
                        "sqlite_entry_unreadable",
                        "a SQLite directory entry could not be read",
                        Some(directory.clone()),
                    ));
                    continue;
                }
            };
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    diagnostics.push(Diagnostic::blocker(
                        "sqlite_entry_unreadable",
                        "a SQLite entry type could not be read",
                        Some(path),
                    ));
                    continue;
                }
            };
            if recursive && file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file() && is_database_path(&path) {
                paths.insert(path);
            } else if is_database_path(&path) && !file_type.is_file() {
                diagnostics.push(Diagnostic::blocker(
                    "sqlite_entry_unsupported",
                    "a SQLite candidate is not a regular file",
                    Some(path),
                ));
            }
        }
        if !recursive {
            break;
        }
    }
}

fn is_database_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "sqlite" | "sqlite3" | "db"
            )
        })
}

fn inspect_sqlite(path: &Path, diagnostics: &mut Vec<Diagnostic>) -> SqliteArtifact {
    let byte_size = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let sidecars = inspect_sidecars(path);
    let mut artifact = SqliteArtifact {
        path: path.to_path_buf(),
        byte_size,
        readable: false,
        schema_kind: SqliteSchemaKind::Unknown,
        user_version: None,
        migration_version: None,
        tables: Vec::new(),
        indexes: Vec::new(),
        sidecars,
        state_rows: Vec::new(),
        history_rows: Vec::new(),
    };

    let connection = match Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(connection) => connection,
        Err(_) => {
            diagnostics.push(Diagnostic::blocker(
                "sqlite_unreadable",
                "a SQLite candidate could not be opened read-only",
                Some(path.to_path_buf()),
            ));
            return artifact;
        }
    };
    let _ = connection.busy_timeout(Duration::from_secs(1));
    let _ = connection.pragma_update(None, "query_only", true);

    let quick_check =
        connection.query_row("PRAGMA quick_check(1)", [], |row| row.get::<_, String>(0));
    if quick_check.as_deref() != Ok("ok") {
        diagnostics.push(Diagnostic::blocker(
            "sqlite_integrity_failed",
            "a SQLite candidate failed read-only quick_check",
            Some(path.to_path_buf()),
        ));
        return artifact;
    }
    artifact.readable = true;
    artifact.user_version = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .ok();

    let table_columns = match read_table_columns(&connection) {
        Ok(table_columns) => table_columns,
        Err(_) => {
            diagnostics.push(Diagnostic::blocker(
                "sqlite_schema_unreadable",
                "SQLite table capabilities could not be read",
                Some(path.to_path_buf()),
            ));
            return artifact;
        }
    };
    artifact.tables = table_columns.keys().cloned().collect();
    artifact.indexes = read_indexes(&connection).unwrap_or_default();
    artifact.migration_version = read_migration_version(&connection, &table_columns);

    let state_capability = capability_status(
        &table_columns,
        "threads",
        &["id", "rollout_path", "model_provider", "cwd", "archived"],
    );
    let history_capability = history_capability_status(&table_columns);
    artifact.schema_kind = match (state_capability, history_capability) {
        (Capability::Complete, Capability::Complete) => SqliteSchemaKind::StateAndThreadHistory,
        (Capability::Complete, Capability::Absent) => SqliteSchemaKind::State,
        (Capability::Absent, Capability::Complete) => SqliteSchemaKind::ThreadHistory,
        (Capability::Absent, Capability::Absent) if is_known_auxiliary_schema(&table_columns) => {
            SqliteSchemaKind::Auxiliary
        }
        _ => SqliteSchemaKind::Unknown,
    };

    if artifact.schema_kind == SqliteSchemaKind::Unknown {
        diagnostics.push(Diagnostic::blocker(
            "sqlite_schema_unknown",
            "SQLite schema lacks a verified state, paginated-history, or auxiliary capability",
            Some(path.to_path_buf()),
        ));
        return artifact;
    }

    if matches!(
        artifact.schema_kind,
        SqliteSchemaKind::State | SqliteSchemaKind::StateAndThreadHistory
    ) {
        match read_state_rows(
            &connection,
            table_columns
                .get("threads")
                .expect("verified state schema must include threads"),
        ) {
            Ok(rows) => {
                if rows.iter().any(|row| {
                    row.thread_id.trim().is_empty()
                        || row.rollout_path.as_os_str().is_empty()
                        || row.provider.trim().is_empty()
                        || row.workspace.as_os_str().is_empty()
                }) {
                    diagnostics.push(Diagnostic::blocker(
                        "sqlite_state_row_invalid",
                        "a threads row has an empty required field",
                        Some(path.to_path_buf()),
                    ));
                }
                artifact.state_rows = rows;
            }
            Err(_) => diagnostics.push(Diagnostic::blocker(
                "sqlite_state_rows_unreadable",
                "verified threads rows could not be read",
                Some(path.to_path_buf()),
            )),
        }
    }
    if matches!(
        artifact.schema_kind,
        SqliteSchemaKind::ThreadHistory | SqliteSchemaKind::StateAndThreadHistory
    ) {
        match read_history_rows(&connection) {
            Ok(rows) => {
                if rows.iter().any(|row| row.thread_id.trim().is_empty()) {
                    diagnostics.push(Diagnostic::blocker(
                        "sqlite_history_row_invalid",
                        "a paginated history row has an empty thread id",
                        Some(path.to_path_buf()),
                    ));
                }
                if rows.iter().any(|row| !history_cursor_is_valid(row)) {
                    diagnostics.push(Diagnostic::blocker(
                        "sqlite_history_cursor_invalid",
                        "paginated history cursor, ordinal, lineage, or byte offsets are invalid",
                        Some(path.to_path_buf()),
                    ));
                }
                artifact.history_rows = rows;
            }
            Err(_) => diagnostics.push(Diagnostic::blocker(
                "sqlite_history_rows_unreadable",
                "verified paginated thread-history rows could not be read",
                Some(path.to_path_buf()),
            )),
        }
    }

    artifact
}

fn history_cursor_is_valid(row: &HistoryThreadRow) -> bool {
    let (Some(next_byte_offset), Some(next_ordinal)) =
        (row.next_rollout_byte_offset, row.next_rollout_ordinal)
    else {
        return false;
    };
    if next_byte_offset < 0 || next_ordinal < 0 {
        return false;
    }
    row.turns.iter().all(|turn| {
        let Some(start_byte_offset) = turn.rollout_byte_offset else {
            return false;
        };
        if turn.turn_id.trim().is_empty()
            || turn.rollout_ordinal < 0
            || start_byte_offset < 0
            || turn.rollout_ordinal >= next_ordinal
            || start_byte_offset >= next_byte_offset
        {
            return false;
        }
        match (turn.rollout_end_ordinal, turn.rollout_end_byte_offset) {
            (Some(end_ordinal), Some(end_byte_offset)) => {
                end_ordinal >= turn.rollout_ordinal
                    && end_ordinal < next_ordinal
                    && end_byte_offset > start_byte_offset
                    && end_byte_offset <= next_byte_offset
            }
            (None, None) => turn.status == "inProgress",
            _ => false,
        }
    })
}

fn inspect_sidecars(database: &Path) -> Vec<SqliteSidecar> {
    [
        ("-wal", SqliteSidecarKind::Wal),
        ("-shm", SqliteSidecarKind::Shm),
    ]
    .into_iter()
    .filter_map(|(suffix, kind)| {
        let mut name: OsString = database.as_os_str().to_os_string();
        name.push(suffix);
        let path = PathBuf::from(name);
        fs::metadata(&path)
            .ok()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| SqliteSidecar {
                kind,
                path,
                byte_size: metadata.len(),
            })
    })
    .collect()
}

fn read_table_columns(
    connection: &Connection,
) -> rusqlite::Result<BTreeMap<String, BTreeSet<String>>> {
    let mut statement =
        connection.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut result = BTreeMap::new();
    for table in tables {
        let escaped = table.replace('"', "\"\"");
        let mut statement = connection.prepare(&format!("PRAGMA table_info(\"{escaped}\")"))?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<BTreeSet<_>>>()?;
        result.insert(table, columns);
    }
    Ok(result)
}

fn read_indexes(connection: &Connection) -> rusqlite::Result<Vec<SqliteIndex>> {
    let mut statement = connection.prepare(
        "SELECT name, tbl_name FROM sqlite_master \
         WHERE type = 'index' AND name NOT LIKE 'sqlite_autoindex_%' \
         ORDER BY name, tbl_name",
    )?;
    let indexes = statement
        .query_map([], |row| {
            Ok(SqliteIndex {
                name: row.get(0)?,
                table: row.get(1)?,
            })
        })?
        .collect();
    indexes
}

fn read_migration_version(
    connection: &Connection,
    tables: &BTreeMap<String, BTreeSet<String>>,
) -> Option<i64> {
    if !tables
        .get("_sqlx_migrations")
        .is_some_and(|columns| columns.contains("version"))
    {
        return None;
    }
    connection
        .query_row("SELECT MAX(version) FROM _sqlx_migrations", [], |row| {
            row.get::<_, Option<i64>>(0)
        })
        .ok()
        .flatten()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Capability {
    Absent,
    Complete,
    Incomplete,
}

fn capability_status(
    tables: &BTreeMap<String, BTreeSet<String>>,
    table: &str,
    required_columns: &[&str],
) -> Capability {
    match tables.get(table) {
        None => Capability::Absent,
        Some(columns)
            if required_columns
                .iter()
                .all(|column| columns.contains(*column)) =>
        {
            Capability::Complete
        }
        Some(_) => Capability::Incomplete,
    }
}

fn history_capability_status(tables: &BTreeMap<String, BTreeSet<String>>) -> Capability {
    let capabilities = [
        capability_status(
            tables,
            "thread_turns",
            &[
                "thread_id",
                "turn_id",
                "rollout_ordinal",
                "rollout_byte_offset",
                "rollout_end_ordinal",
                "rollout_end_byte_offset",
            ],
        ),
        capability_status(
            tables,
            "thread_items",
            &[
                "thread_id",
                "turn_id",
                "item_id",
                "rollout_ordinal",
                "updated_at_ordinal",
                "item_type",
            ],
        ),
        capability_status(
            tables,
            "thread_history_projection_state",
            &[
                "thread_id",
                "next_rollout_byte_offset",
                "next_rollout_ordinal",
            ],
        ),
    ];
    if capabilities.iter().all(|item| *item == Capability::Absent) {
        Capability::Absent
    } else if capabilities
        .iter()
        .all(|item| *item == Capability::Complete)
    {
        Capability::Complete
    } else {
        Capability::Incomplete
    }
}

fn is_known_auxiliary_schema(tables: &BTreeMap<String, BTreeSet<String>>) -> bool {
    const KNOWN_TABLES: &[&str] = &[
        "_sqlx_migrations",
        "app_server_history_snapshots",
        "automations",
        "automation_runs",
        "external_agent_config_imports",
        "inbox_items",
        "jobs",
        "logs",
        "local_app_server_feature_enablement",
        "local_thread_catalog",
        "local_thread_catalog_hosts",
        "local_thread_catalog_metadata",
        "local_thread_catalog_sync_state",
        "memories",
        "memory_usage",
        "remote_control_enrollments",
        "stage1_outputs",
        "thread_timeline_ledger",
        "thread_goal_continuation_deferrals",
        "thread_goals",
    ];
    let application_tables = tables
        .keys()
        .filter(|name| !name.starts_with("sqlite_"))
        .collect::<Vec<_>>();
    !application_tables.is_empty()
        && application_tables
            .iter()
            .all(|name| KNOWN_TABLES.contains(&name.as_str()))
        && application_tables
            .iter()
            .any(|name| name.as_str() != "_sqlx_migrations")
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_owned();
        (!value.is_empty()).then_some(value)
    })
}

fn timestamp_to_millis(value: i64) -> i64 {
    match value.unsigned_abs() {
        0..=99_999_999_999 => value.saturating_mul(1_000),
        100_000_000_000..=99_999_999_999_999 => value,
        100_000_000_000_000..=99_999_999_999_999_999 => value / 1_000,
        _ => value / 1_000_000,
    }
}

fn read_state_rows(
    connection: &Connection,
    thread_columns: &BTreeSet<String>,
) -> rusqlite::Result<Vec<StateThreadRow>> {
    let title = thread_columns
        .contains("title")
        .then_some("title")
        .unwrap_or("NULL");
    let preview = thread_columns
        .contains("preview")
        .then_some("preview")
        .unwrap_or("NULL");
    let first_user_message = thread_columns
        .contains("first_user_message")
        .then_some("first_user_message")
        .unwrap_or("NULL");
    let updated_at_ms = thread_columns
        .contains("updated_at_ms")
        .then_some("updated_at_ms")
        .unwrap_or("NULL");
    let updated_at = thread_columns
        .contains("updated_at")
        .then_some("updated_at")
        .unwrap_or("NULL");
    let sql = format!(
        "SELECT id, rollout_path, model_provider, cwd, archived, \
                {title} AS title, {preview} AS preview, {first_user_message} AS first_user_message, \
                CAST({updated_at_ms} AS INTEGER) AS updated_at_ms, \
                CAST({updated_at} AS INTEGER) AS updated_at \
         FROM threads ORDER BY id"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement
        .query_map([], |row| {
            let title = normalize_optional_text(row.get(5)?);
            let preview = normalize_optional_text(row.get(6)?);
            let first_user_message = normalize_optional_text(row.get(7)?);
            let updated_at_ms = row
                .get::<_, Option<i64>>(8)?
                .map(timestamp_to_millis)
                .or_else(|| {
                    row.get::<_, Option<i64>>(9)
                        .ok()
                        .flatten()
                        .map(timestamp_to_millis)
                });
            Ok(StateThreadRow {
                thread_id: row.get(0)?,
                rollout_path: PathBuf::from(row.get::<_, String>(1)?),
                provider: row.get(2)?,
                workspace: PathBuf::from(row.get::<_, String>(3)?),
                archived: row.get::<_, i64>(4)? != 0,
                title,
                summary: preview.or(first_user_message),
                updated_at_ms,
            })
        })?
        .collect();
    rows
}

#[derive(Default)]
struct HistoryBuilder {
    turn_count: u64,
    item_count: u64,
    first_ordinal: Option<i64>,
    last_ordinal: Option<i64>,
    next_rollout_byte_offset: Option<i64>,
    next_rollout_ordinal: Option<i64>,
    turns: Vec<HistoryTurnCursor>,
}

fn read_history_rows(connection: &Connection) -> rusqlite::Result<Vec<HistoryThreadRow>> {
    let mut rows = BTreeMap::<String, HistoryBuilder>::new();
    {
        let mut statement = connection.prepare(
            "SELECT thread_id, turn_id, rollout_ordinal, rollout_byte_offset, \
                    rollout_end_ordinal, rollout_end_byte_offset, status \
             FROM thread_turns ORDER BY thread_id, rollout_ordinal, turn_id",
        )?;
        for row in statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                HistoryTurnCursor {
                    turn_id: row.get(1)?,
                    rollout_ordinal: row.get(2)?,
                    rollout_byte_offset: row.get(3)?,
                    rollout_end_ordinal: row.get(4)?,
                    rollout_end_byte_offset: row.get(5)?,
                    status: row.get(6)?,
                },
            ))
        })? {
            let (thread_id, turn) = row?;
            let entry = rows.entry(thread_id).or_default();
            entry.turn_count += 1;
            merge_ordinal_range(
                entry,
                Some(turn.rollout_ordinal),
                turn.rollout_end_ordinal.or(Some(turn.rollout_ordinal)),
            );
            entry.turns.push(turn);
        }
    }
    {
        let mut statement = connection.prepare(
            "SELECT thread_id, COUNT(*), MIN(rollout_ordinal), MAX(rollout_ordinal) \
             FROM thread_items GROUP BY thread_id ORDER BY thread_id",
        )?;
        for row in statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })? {
            let (thread_id, count, first, last) = row?;
            let entry = rows.entry(thread_id).or_default();
            entry.item_count = count;
            merge_ordinal_range(entry, first, last);
        }
    }
    {
        let mut statement = connection.prepare(
            "SELECT thread_id, next_rollout_byte_offset, next_rollout_ordinal \
             FROM thread_history_projection_state ORDER BY thread_id",
        )?;
        for row in statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })? {
            let (thread_id, next_byte_offset, next_ordinal) = row?;
            let entry = rows.entry(thread_id).or_default();
            entry.next_rollout_byte_offset = Some(next_byte_offset);
            entry.next_rollout_ordinal = Some(next_ordinal);
        }
    }

    Ok(rows
        .into_iter()
        .map(|(thread_id, row)| HistoryThreadRow {
            thread_id,
            turn_count: row.turn_count,
            item_count: row.item_count,
            first_ordinal: row.first_ordinal,
            last_ordinal: row.last_ordinal,
            next_rollout_byte_offset: row.next_rollout_byte_offset,
            next_rollout_ordinal: row.next_rollout_ordinal,
            turns: row.turns,
        })
        .collect())
}

fn merge_ordinal_range(builder: &mut HistoryBuilder, first: Option<i64>, last: Option<i64>) {
    if let Some(first) = first {
        builder.first_ordinal = Some(
            builder
                .first_ordinal
                .map_or(first, |current| current.min(first)),
        );
    }
    if let Some(last) = last {
        builder.last_ordinal = Some(
            builder
                .last_ordinal
                .map_or(last, |current| current.max(last)),
        );
    }
}

#[derive(Default)]
struct ThreadBuilder {
    rollout_paths: Vec<PathBuf>,
    rollout_logical_paths: Vec<PathBuf>,
    state_databases: Vec<PathBuf>,
    state_rollout_paths: Vec<PathBuf>,
    history_databases: Vec<PathBuf>,
    providers: BTreeSet<String>,
    workspaces: BTreeSet<PathBuf>,
    archived: BTreeSet<bool>,
    title: Option<String>,
    summary: Option<String>,
    updated_at_ms: Option<i64>,
    storage_versions: BTreeSet<String>,
}

fn build_thread_inventory(
    codex_home: &Path,
    rollouts: &[RolloutArtifact],
    databases: &[SqliteArtifact],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<ThreadInventory> {
    let mut threads = BTreeMap::<String, ThreadBuilder>::new();
    for rollout in rollouts {
        let thread = threads.entry(rollout.thread_id.clone()).or_default();
        thread.rollout_paths.push(rollout.path.clone());
        thread
            .rollout_logical_paths
            .push(normalize_comparison_path(&rollout.logical_path));
        thread.providers.insert(rollout.provider.clone());
        if let Some(workspace) = &rollout.workspace {
            thread.workspaces.insert(workspace.clone());
        }
        thread.archived.insert(rollout.archived);
        let format = match rollout.encoding {
            RolloutEncoding::Jsonl => "jsonl",
            RolloutEncoding::Zstd => "jsonl.zst",
        };
        thread.storage_versions.insert(match &rollout.cli_version {
            Some(version) => format!("rollout:{format}:cli={version}"),
            None => format!("rollout:{format}"),
        });
    }
    for database in databases {
        for row in &database.state_rows {
            let thread = threads.entry(row.thread_id.clone()).or_default();
            thread.state_databases.push(database.path.clone());
            thread
                .state_rollout_paths
                .push(resolve_state_rollout_path(codex_home, &row.rollout_path));
            thread.providers.insert(row.provider.clone());
            thread.workspaces.insert(row.workspace.clone());
            thread.archived.insert(row.archived);
            if thread.title.is_none() {
                thread.title = row.title.clone();
            }
            if thread.summary.is_none() {
                thread.summary = row.summary.clone();
            }
            if thread.updated_at_ms.is_none() {
                thread.updated_at_ms = row.updated_at_ms;
            }
            thread.storage_versions.insert(format!(
                "state:{}:{}",
                database
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("database"),
                schema_version(database)
            ));
        }
        for row in &database.history_rows {
            let thread = threads.entry(row.thread_id.clone()).or_default();
            thread.history_databases.push(database.path.clone());
            thread.storage_versions.insert(format!(
                "history:{}:{}",
                database
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("database"),
                schema_version(database)
            ));
        }
    }

    for (thread_id, thread) in &threads {
        if thread.rollout_paths.len() > 1
            || thread.state_databases.len() > 1
            || thread.history_databases.len() > 1
        {
            diagnostics.push(Diagnostic::for_thread(
                DiagnosticLevel::Blocker,
                "duplicate_thread_id",
                "a thread id occurs in multiple rollout, state, or history artifacts",
                thread_id,
            ));
        }
        if thread.providers.len() > 1 {
            diagnostics.push(Diagnostic::for_thread(
                DiagnosticLevel::Blocker,
                "thread_provider_mismatch",
                "rollout and SQLite provider buckets disagree for one thread",
                thread_id,
            ));
        }
        if thread.archived.len() > 1 {
            diagnostics.push(Diagnostic::for_thread(
                DiagnosticLevel::Blocker,
                "thread_archive_mismatch",
                "rollout location and SQLite archive state disagree",
                thread_id,
            ));
        }
        if thread.rollout_paths.is_empty() || thread.state_databases.is_empty() {
            diagnostics.push(Diagnostic::for_thread(
                DiagnosticLevel::Blocker,
                "thread_storage_incomplete",
                "thread inventory is missing a rollout or verified state row",
                thread_id,
            ));
        } else if thread.rollout_logical_paths.len() == 1
            && thread.state_rollout_paths.len() == 1
            && thread.rollout_logical_paths[0] != thread.state_rollout_paths[0]
        {
            diagnostics.push(Diagnostic::for_thread(
                DiagnosticLevel::Blocker,
                "thread_rollout_path_mismatch",
                "the state row points at a different rollout path",
                thread_id,
            ));
        }
    }

    threads
        .into_iter()
        .map(|(thread_id, mut thread)| {
            thread.rollout_paths.sort();
            thread.state_databases.sort();
            thread.history_databases.sort();
            ThreadInventory {
                thread_id,
                rollout_paths: thread.rollout_paths,
                state_databases: thread.state_databases,
                history_databases: thread.history_databases,
                providers: thread.providers.into_iter().collect(),
                workspaces: thread.workspaces.into_iter().collect(),
                archived: if thread.archived.len() == 1 {
                    thread.archived.into_iter().next()
                } else {
                    None
                },
                title: thread.title,
                summary: thread.summary,
                updated_at_ms: thread.updated_at_ms,
                storage_versions: thread.storage_versions.into_iter().collect(),
            }
        })
        .collect()
}

fn resolve_state_rollout_path(codex_home: &Path, path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        codex_home.join(path)
    };
    normalize_comparison_path(&plain_rollout_path(&path))
}

fn normalize_comparison_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        path.parent()
            .and_then(|parent| fs::canonicalize(parent).ok())
            .and_then(|parent| path.file_name().map(|name| parent.join(name)))
            .unwrap_or_else(|| path.to_path_buf())
    })
}

fn schema_version(database: &SqliteArtifact) -> String {
    if let Some(version) = database.migration_version {
        format!("migration={version}")
    } else if let Some(version) = database.user_version {
        format!("user={version}")
    } else {
        "unversioned".to_owned()
    }
}

fn detect_provider_layout(
    config: &ConfigInventory,
    rollouts: &[RolloutArtifact],
    databases: &[SqliteArtifact],
) -> ProviderLayout {
    let mut providers = BTreeSet::new();
    if let Some(provider) = &config.active_provider {
        providers.insert(provider.as_str());
    }
    for provider in rollouts.iter().map(|rollout| rollout.provider.as_str()) {
        providers.insert(provider);
    }
    for provider in databases
        .iter()
        .flat_map(|database| database.state_rows.iter())
        .map(|row| row.provider.as_str())
    {
        providers.insert(provider);
    }

    if providers.is_empty() {
        ProviderLayout::Empty
    } else if providers.len() > 1 {
        ProviderLayout::Mixed
    } else {
        match providers.into_iter().next().expect("one provider") {
            OFFICIAL_PROVIDER => ProviderLayout::Official,
            CUSTOM_PROVIDER => ProviderLayout::CcSwitchCustom,
            NIKO_PROVIDER => ProviderLayout::NikoMomotoken,
            _ => ProviderLayout::CodexPlusPlusCompatible,
        }
    }
}

fn build_normalization_plan(
    config: &ConfigInventory,
    rollouts: &[RolloutArtifact],
    databases: &[SqliteArtifact],
    blocked: bool,
) -> NormalizationPlan {
    if blocked {
        return NormalizationPlan {
            status: NormalizationStatus::Blocked,
            target_provider: CUSTOM_PROVIDER.to_owned(),
            actions: Vec::new(),
        };
    }

    let mut actions = Vec::new();
    if let Some(provider) = &config.active_provider {
        if provider != CUSTOM_PROVIDER {
            actions.push(PlanAction::ConfigureCustomBucket {
                from: provider.clone(),
                to: CUSTOM_PROVIDER.to_owned(),
            });
        }
    }
    for rollout in rollouts {
        if rollout.provider != CUSTOM_PROVIDER {
            actions.push(PlanAction::RewriteRolloutHeader {
                path: rollout.path.clone(),
                thread_id: rollout.thread_id.clone(),
                from: rollout.provider.clone(),
                to: CUSTOM_PROVIDER.to_owned(),
            });
        }
    }
    for database in databases {
        for row in &database.state_rows {
            if row.provider != CUSTOM_PROVIDER {
                actions.push(PlanAction::UpdateStateRow {
                    database: database.path.clone(),
                    thread_id: row.thread_id.clone(),
                    from: row.provider.clone(),
                    to: CUSTOM_PROVIDER.to_owned(),
                });
            }
        }
    }

    NormalizationPlan {
        status: if actions.is_empty() {
            NormalizationStatus::NoChanges
        } else {
            NormalizationStatus::WouldNormalize
        },
        target_provider: CUSTOM_PROVIDER.to_owned(),
        actions,
    }
}

/// Rewrites provider bucket references only inside explicitly marked fixture roots.
/// This PoC is deliberately not connected to any production command.
pub fn migrate_fixture_provider(
    request: &ScanRequest,
    target: FixtureProviderTarget,
) -> Result<FixtureMigrationReport, FixtureMutationError> {
    validate_fixture_write_roots(request)?;
    let before = scan_for_fixture_mutation(request)?;
    let before_threads = build_fixture_thread_proofs(&before)?;
    let target_provider = target.provider();
    let mut changed_paths = BTreeSet::new();
    let mut rollout_byte_deltas = BTreeMap::<String, i64>::new();

    if before.config.active_provider.as_deref() != Some(target_provider) {
        let contents = rewrite_config_provider(&before, target)?;
        fs::write(&before.config.path, contents).map_err(|_| {
            fixture_error(
                "fixture_config_write_failed",
                "fixture config.toml could not be written",
                Some(before.config.path.clone()),
            )
        })?;
        changed_paths.insert(before.config.path.clone());
    }

    for rollout in &before.rollouts {
        if rollout.provider == target_provider {
            continue;
        }
        let logical = read_rollout_logical(rollout)?;
        let (rewritten, byte_delta) = rewrite_rollout_header_provider(
            &logical,
            &rollout.thread_id,
            &rollout.provider,
            target_provider,
            &rollout.path,
        )?;
        write_rollout_logical(rollout, &rewritten)?;
        rollout_byte_deltas.insert(rollout.thread_id.clone(), byte_delta);
        changed_paths.insert(rollout.path.clone());
    }

    for database in &before.sqlite_databases {
        let state_updates = database
            .state_rows
            .iter()
            .filter(|row| row.provider != target_provider)
            .collect::<Vec<_>>();
        let history_updates = database
            .history_rows
            .iter()
            .filter_map(|row| {
                rollout_byte_deltas
                    .get(&row.thread_id)
                    .copied()
                    .filter(|delta| *delta != 0)
                    .map(|delta| (row, delta))
            })
            .collect::<Vec<_>>();
        if state_updates.is_empty() && history_updates.is_empty() {
            continue;
        }
        let mut connection = Connection::open_with_flags(
            &database.path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| {
            fixture_error(
                "fixture_state_open_failed",
                "fixture state database could not be opened read-write",
                Some(database.path.clone()),
            )
        })?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| {
                fixture_error(
                    "fixture_state_transaction_failed",
                    "fixture state transaction could not start",
                    Some(database.path.clone()),
                )
            })?;
        for row in state_updates {
            let changed = transaction
                .execute(
                    "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND model_provider = ?3",
                    params![target_provider, row.thread_id, row.provider],
                )
                .map_err(|_| {
                    fixture_error(
                        "fixture_state_update_failed",
                        "fixture state provider row could not be updated",
                        Some(database.path.clone()),
                    )
                })?;
            if changed != 1 {
                return Err(fixture_error(
                    "fixture_state_update_raced",
                    "fixture state provider row changed after validation",
                    Some(database.path.clone()),
                ));
            }
        }
        for (row, delta) in history_updates {
            transaction
                .execute(
                    "UPDATE thread_turns \
                     SET rollout_byte_offset = rollout_byte_offset + ?1, \
                         rollout_end_byte_offset = CASE \
                             WHEN rollout_end_byte_offset IS NULL THEN NULL \
                             ELSE rollout_end_byte_offset + ?1 END \
                     WHERE thread_id = ?2",
                    params![delta, row.thread_id],
                )
                .map_err(|_| {
                    fixture_error(
                        "fixture_history_offset_update_failed",
                        "fixture turn byte offsets could not be shifted with the rollout header",
                        Some(database.path.clone()),
                    )
                })?;
            let changed = transaction
                .execute(
                    "UPDATE thread_history_projection_state \
                     SET next_rollout_byte_offset = next_rollout_byte_offset + ?1 \
                     WHERE thread_id = ?2 AND next_rollout_byte_offset = ?3",
                    params![delta, row.thread_id, row.next_rollout_byte_offset],
                )
                .map_err(|_| {
                    fixture_error(
                        "fixture_projection_offset_update_failed",
                        "fixture projection byte offset could not be shifted with the rollout header",
                        Some(database.path.clone()),
                    )
                })?;
            if changed != 1 {
                return Err(fixture_error(
                    "fixture_projection_offset_raced",
                    "fixture projection byte offset changed after validation",
                    Some(database.path.clone()),
                ));
            }
        }
        transaction.commit().map_err(|_| {
            fixture_error(
                "fixture_state_commit_failed",
                "fixture state provider transaction could not commit",
                Some(database.path.clone()),
            )
        })?;
        changed_paths.insert(database.path.clone());
    }

    let after = scan_for_fixture_mutation(request)?;
    if after.config.active_provider.as_deref() != Some(target_provider)
        || after
            .rollouts
            .iter()
            .any(|rollout| rollout.provider != target_provider)
        || after
            .sqlite_databases
            .iter()
            .flat_map(|database| database.state_rows.iter())
            .any(|row| row.provider != target_provider)
    {
        return Err(fixture_error(
            "fixture_target_validation_failed",
            "fixture provider migration did not converge on the requested bucket",
            None,
        ));
    }
    let after_threads = build_fixture_thread_proofs(&after)?;
    let mut expected_after_threads = before_threads.clone();
    for thread in &mut expected_after_threads {
        if let Some(delta) = rollout_byte_deltas.get(&thread.thread_id).copied() {
            shift_fixture_proof_offsets(thread, delta)?;
        }
    }
    if expected_after_threads != after_threads {
        return Err(fixture_error(
            "fixture_roundtrip_invariant_changed",
            "thread identity, workspace, archive, visible history, lineage, ordinal, or byte offsets changed",
            None,
        ));
    }

    Ok(FixtureMigrationReport {
        target,
        changed_paths: changed_paths.into_iter().collect(),
        before,
        after,
        before_threads,
        after_threads,
    })
}

/// Appends one deterministic fixture round to the original rollout and its
/// official-schema paginated history projection.
pub fn append_fixture_round(
    request: &ScanRequest,
    thread_id: &str,
    round_label: &str,
) -> Result<FixtureAppendReport, FixtureMutationError> {
    validate_fixture_write_roots(request)?;
    if thread_id.trim().is_empty() || round_label.trim().is_empty() {
        return Err(fixture_error(
            "fixture_append_identity_invalid",
            "fixture append requires non-empty thread and round identifiers",
            None,
        ));
    }
    let before_report = scan_for_fixture_mutation(request)?;
    let before = build_fixture_thread_proofs(&before_report)?
        .into_iter()
        .find(|thread| thread.thread_id == thread_id)
        .ok_or_else(|| {
            fixture_error(
                "fixture_append_thread_missing",
                "fixture append thread was not found",
                None,
            )
        })?;
    let rollout = before_report
        .rollouts
        .iter()
        .find(|rollout| rollout.thread_id == thread_id)
        .ok_or_else(|| {
            fixture_error(
                "fixture_append_rollout_missing",
                "fixture append rollout was not found",
                None,
            )
        })?;
    let (history_path, history_row) = before_report
        .sqlite_databases
        .iter()
        .find_map(|database| {
            database
                .history_rows
                .iter()
                .find(|row| row.thread_id == thread_id)
                .map(|row| (database.path.clone(), row.clone()))
        })
        .ok_or_else(|| {
            fixture_error(
                "fixture_append_history_missing",
                "fixture append requires one verified paginated history database",
                None,
            )
        })?;
    let start_byte_offset = history_row.next_rollout_byte_offset.ok_or_else(|| {
        fixture_error(
            "fixture_append_cursor_missing",
            "fixture history lacks next_rollout_byte_offset",
            Some(history_path.clone()),
        )
    })?;
    let start_ordinal = history_row.next_rollout_ordinal.ok_or_else(|| {
        fixture_error(
            "fixture_append_cursor_missing",
            "fixture history lacks next_rollout_ordinal",
            Some(history_path.clone()),
        )
    })?;
    let mut logical = read_rollout_logical(rollout)?;
    if i64::try_from(logical.len()).ok() != Some(start_byte_offset) || !logical.ends_with(b"\n") {
        return Err(fixture_error(
            "fixture_append_projection_mismatch",
            "fixture projection byte cursor does not equal durable rollout length",
            Some(rollout.path.clone()),
        ));
    }

    let turn_id = format!("poc-turn-{round_label}");
    let user_item_id = format!("poc-user-{round_label}");
    let agent_item_id = format!("poc-agent-{round_label}");
    let records = [
        serde_json::json!({
            "type": "event_msg",
            "payload": {"type": "turn_started", "turn_id": turn_id, "unknown_started": true},
            "unknown_envelope": {"round": round_label}
        }),
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": user_item_id,
                "role": "user",
                "content": [{"type": "input_text", "text": format!("fixture round {round_label}")}],
                "attachment": {"path": format!("/fixture/{round_label}.png"), "unknown": 1}
            }
        }),
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": agent_item_id,
                "role": "assistant",
                "content": [{"type": "output_text", "text": format!("fixture reply {round_label}")}],
                "response_id": format!("resp-{round_label}"),
                "encrypted_content": format!("encrypted-{round_label}"),
                "unknown_payload": [1, 2, 3]
            },
            "unknown_envelope": "preserved"
        }),
        serde_json::json!({
            "type": "event_msg",
            "payload": {"type": "turn_completed", "turn_id": turn_id, "unknown_completed": true}
        }),
    ];
    for record in &records {
        serde_json::to_writer(&mut logical, record).map_err(|_| {
            fixture_error(
                "fixture_append_json_failed",
                "fixture append record could not be serialized",
                Some(rollout.path.clone()),
            )
        })?;
        logical.push(b'\n');
    }
    let end_byte_offset = i64::try_from(logical.len()).map_err(|_| {
        fixture_error(
            "fixture_append_offset_overflow",
            "fixture rollout length exceeds SQLite integer range",
            Some(rollout.path.clone()),
        )
    })?;
    let end_ordinal = start_ordinal.checked_add(3).ok_or_else(|| {
        fixture_error(
            "fixture_append_ordinal_overflow",
            "fixture rollout ordinal exceeds SQLite integer range",
            Some(history_path.clone()),
        )
    })?;
    write_rollout_logical(rollout, &logical)?;

    let mut connection = Connection::open_with_flags(
        &history_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| {
        fixture_error(
            "fixture_history_open_failed",
            "fixture history database could not be opened read-write",
            Some(history_path.clone()),
        )
    })?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| {
            fixture_error(
                "fixture_history_transaction_failed",
                "fixture history transaction could not start",
                Some(history_path.clone()),
            )
        })?;
    transaction
        .execute(
            "INSERT INTO thread_turns (thread_id, turn_id, rollout_ordinal, \
             rollout_byte_offset, rollout_end_ordinal, rollout_end_byte_offset, status, \
             started_at, completed_at, duration_ms, first_user_item_id, final_agent_item_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'completed', 1, 2, 1, ?7, ?8)",
            params![
                thread_id,
                turn_id,
                start_ordinal,
                start_byte_offset,
                end_ordinal,
                end_byte_offset,
                user_item_id,
                agent_item_id,
            ],
        )
        .map_err(|_| {
            fixture_error(
                "fixture_history_turn_insert_failed",
                "fixture history turn row could not be inserted",
                Some(history_path.clone()),
            )
        })?;
    let user_item = serde_json::json!({
        "type": "userMessage",
        "id": user_item_id,
        "content": [{"type": "inputText", "text": format!("fixture round {round_label}")}]
    });
    let agent_item = serde_json::json!({
        "type": "agentMessage",
        "id": agent_item_id,
        "text": format!("fixture reply {round_label}"),
        "phase": "final_answer"
    });
    for (item_id, ordinal, item_type, item_json) in [
        (
            user_item_id.as_str(),
            start_ordinal + 1,
            "userMessage",
            user_item,
        ),
        (
            agent_item_id.as_str(),
            start_ordinal + 2,
            "agentMessage",
            agent_item,
        ),
    ] {
        transaction
            .execute(
                "INSERT INTO thread_items (thread_id, turn_id, item_id, rollout_ordinal, \
                 updated_at_ordinal, created_at_ms, item_type, item_json) \
                 VALUES (?1, ?2, ?3, ?4, ?4, 1, ?5, ?6)",
                params![
                    thread_id,
                    turn_id,
                    item_id,
                    ordinal,
                    item_type,
                    item_json.to_string(),
                ],
            )
            .map_err(|_| {
                fixture_error(
                    "fixture_history_item_insert_failed",
                    "fixture history item row could not be inserted",
                    Some(history_path.clone()),
                )
            })?;
    }
    let cursor_changed = transaction
        .execute(
            "UPDATE thread_history_projection_state \
             SET next_rollout_byte_offset = ?1, next_rollout_ordinal = ?2 \
             WHERE thread_id = ?3 AND next_rollout_byte_offset = ?4 \
             AND next_rollout_ordinal = ?5",
            params![
                end_byte_offset,
                end_ordinal + 1,
                thread_id,
                start_byte_offset,
                start_ordinal,
            ],
        )
        .map_err(|_| {
            fixture_error(
                "fixture_history_cursor_update_failed",
                "fixture history projection cursor could not be updated",
                Some(history_path.clone()),
            )
        })?;
    if cursor_changed != 1 {
        return Err(fixture_error(
            "fixture_history_cursor_raced",
            "fixture history projection cursor changed after validation",
            Some(history_path.clone()),
        ));
    }
    transaction.commit().map_err(|_| {
        fixture_error(
            "fixture_history_commit_failed",
            "fixture history append transaction could not commit",
            Some(history_path.clone()),
        )
    })?;

    let after_report = scan_for_fixture_mutation(request)?;
    let after = build_fixture_thread_proofs(&after_report)?
        .into_iter()
        .find(|thread| thread.thread_id == thread_id)
        .ok_or_else(|| {
            fixture_error(
                "fixture_append_thread_lost",
                "fixture append removed the original thread",
                None,
            )
        })?;
    let appended_turn = after
        .history
        .iter()
        .flat_map(|history| history.row.turns.iter())
        .find(|turn| turn.turn_id == turn_id)
        .ok_or_else(|| {
            fixture_error(
                "fixture_append_turn_missing",
                "fixture append turn was not projected",
                Some(history_path.clone()),
            )
        })?;
    if appended_turn.rollout_ordinal != start_ordinal
        || appended_turn.rollout_byte_offset != Some(start_byte_offset)
        || appended_turn.rollout_end_ordinal != Some(end_ordinal)
        || appended_turn.rollout_end_byte_offset != Some(end_byte_offset)
    {
        return Err(fixture_error(
            "fixture_append_projection_invalid",
            "fixture append projection ordinal or byte offsets are inconsistent",
            Some(history_path),
        ));
    }

    Ok(FixtureAppendReport {
        thread_id: thread_id.to_owned(),
        turn_id,
        start_ordinal,
        end_ordinal,
        start_byte_offset,
        end_byte_offset,
        before,
        after,
    })
}

fn validate_fixture_write_roots(request: &ScanRequest) -> Result<(), FixtureMutationError> {
    validate_request(request).map_err(|error| {
        fixture_error(
            "fixture_root_invalid",
            error.to_string(),
            Some(request.codex_home.clone()),
        )
    })?;
    let mut roots = vec![request.codex_home.clone()];
    if let Some(sqlite_home) = &request.explicit_sqlite_home {
        if !sqlite_home.is_dir() {
            return Err(fixture_error(
                "fixture_sqlite_root_invalid",
                "explicit fixture SQLite home is not a directory",
                Some(sqlite_home.clone()),
            ));
        }
        if !roots.contains(sqlite_home) {
            roots.push(sqlite_home.clone());
        }
    }
    for root in roots {
        let marker = root.join(FIXTURE_ROOT_MARKER);
        let metadata = fs::symlink_metadata(&marker).map_err(|_| {
            fixture_error(
                "fixture_marker_missing",
                "fixture writes require the E10-2 marker in every explicit root",
                Some(marker.clone()),
            )
        })?;
        if !metadata.file_type().is_file() {
            return Err(fixture_error(
                "fixture_marker_invalid",
                "fixture root marker must be a regular file",
                Some(marker),
            ));
        }
        let contents = fs::read_to_string(&marker).map_err(|_| {
            fixture_error(
                "fixture_marker_missing",
                "fixture writes require the E10-2 marker in every explicit root",
                Some(marker.clone()),
            )
        })?;
        if contents != FIXTURE_ROOT_MARKER_CONTENT {
            return Err(fixture_error(
                "fixture_marker_invalid",
                "fixture root marker has unexpected contents",
                Some(marker),
            ));
        }
    }
    Ok(())
}

fn scan_for_fixture_mutation(request: &ScanRequest) -> Result<ScanReport, FixtureMutationError> {
    let report = scan_codex_sessions(request).map_err(|error| {
        fixture_error(
            "fixture_scan_failed",
            error.to_string(),
            Some(request.codex_home.clone()),
        )
    })?;
    if report.is_blocked() {
        let codes = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.level == DiagnosticLevel::Blocker)
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>()
            .join(",");
        return Err(fixture_error(
            "fixture_scan_blocked",
            format!("fixture inventory contains blockers: {codes}"),
            None,
        ));
    }
    Ok(report)
}

fn rewrite_config_provider(
    report: &ScanReport,
    target: FixtureProviderTarget,
) -> Result<Vec<u8>, FixtureMutationError> {
    let source = report
        .config
        .active_provider
        .as_deref()
        .unwrap_or(OFFICIAL_PROVIDER);
    let text = if report.config.present {
        fs::read_to_string(&report.config.path).map_err(|_| {
            fixture_error(
                "fixture_config_read_failed",
                "fixture config.toml could not be read",
                Some(report.config.path.clone()),
            )
        })?
    } else {
        String::new()
    };
    let mut config = text.parse::<toml::Table>().map_err(|_| {
        fixture_error(
            "fixture_config_parse_failed",
            "fixture config.toml is not valid TOML",
            Some(report.config.path.clone()),
        )
    })?;
    config.insert(
        "model_provider".to_owned(),
        toml::Value::String(target.provider().to_owned()),
    );
    if target == FixtureProviderTarget::Custom {
        let providers = config
            .entry("model_providers".to_owned())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| {
                fixture_error(
                    "fixture_config_provider_table_invalid",
                    "fixture model_providers is not a table",
                    Some(report.config.path.clone()),
                )
            })?;
        let mut custom = if source == OFFICIAL_PROVIDER {
            providers
                .get(CUSTOM_PROVIDER)
                .and_then(toml::Value::as_table)
                .cloned()
                .unwrap_or_default()
        } else {
            providers
                .get(source)
                .and_then(toml::Value::as_table)
                .cloned()
                .ok_or_else(|| {
                    fixture_error(
                        "fixture_source_provider_missing",
                        "fixture source provider definition is missing",
                        Some(report.config.path.clone()),
                    )
                })?
        };
        if source == OFFICIAL_PROVIDER {
            custom.insert("name".to_owned(), toml::Value::String("OpenAI".to_owned()));
            custom.insert(
                "requires_openai_auth".to_owned(),
                toml::Value::Boolean(true),
            );
            custom.insert("supports_websockets".to_owned(), toml::Value::Boolean(true));
            custom.insert(
                "wire_api".to_owned(),
                toml::Value::String("responses".to_owned()),
            );
        }
        providers.insert(CUSTOM_PROVIDER.to_owned(), toml::Value::Table(custom));
    }
    toml::to_string_pretty(&config)
        .map(String::into_bytes)
        .map_err(|_| {
            fixture_error(
                "fixture_config_serialize_failed",
                "fixture config.toml could not be serialized",
                Some(report.config.path.clone()),
            )
        })
}

fn build_fixture_thread_proofs(
    report: &ScanReport,
) -> Result<Vec<FixtureThreadProof>, FixtureMutationError> {
    let mut proofs = Vec::new();
    for rollout in &report.rollouts {
        let logical = read_rollout_logical(rollout)?;
        let records = rollout_records(&logical, &rollout.path)?;
        let header = records.first().ok_or_else(|| {
            fixture_error(
                "fixture_rollout_empty",
                "fixture rollout has no records",
                Some(rollout.path.clone()),
            )
        })?;
        let mut provider_neutral_header = header.value.clone();
        provider_neutral_header
            .get_mut("payload")
            .and_then(JsonValue::as_object_mut)
            .and_then(|payload| payload.remove("model_provider"));
        let mut visible_hasher = Sha256::new();
        let mut neutral_hasher = Sha256::new();
        neutral_hasher.update(serde_json::to_vec(&provider_neutral_header).map_err(|_| {
            fixture_error(
                "fixture_rollout_digest_failed",
                "fixture rollout header could not be normalized for digest",
                Some(rollout.path.clone()),
            )
        })?);
        for record in records.iter().skip(1) {
            let bytes = &logical[record.start..record.end];
            visible_hasher.update(bytes);
            neutral_hasher.update(bytes);
        }
        let thread = report
            .threads
            .iter()
            .find(|thread| thread.thread_id == rollout.thread_id)
            .ok_or_else(|| {
                fixture_error(
                    "fixture_thread_inventory_missing",
                    "fixture rollout has no thread inventory entry",
                    Some(rollout.path.clone()),
                )
            })?;
        let mut history = report
            .sqlite_databases
            .iter()
            .flat_map(|database| {
                database
                    .history_rows
                    .iter()
                    .filter(|row| row.thread_id == rollout.thread_id)
                    .cloned()
                    .map(|row| FixtureHistoryProof {
                        database: database.path.clone(),
                        row,
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        history.sort_by(|left, right| left.database.cmp(&right.database));
        proofs.push(FixtureThreadProof {
            thread_id: rollout.thread_id.clone(),
            rollout_path: rollout.path.clone(),
            state_databases: thread.state_databases.clone(),
            workspace: rollout.workspace.clone(),
            archived: rollout.archived,
            visible_event_count: records.len().saturating_sub(1),
            visible_history_digest: digest_hex(visible_hasher.finalize().as_slice()),
            provider_neutral_digest: digest_hex(neutral_hasher.finalize().as_slice()),
            history,
        });
    }
    proofs.sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
    Ok(proofs)
}

struct RolloutRecord {
    start: usize,
    content_end: usize,
    end: usize,
    value: JsonValue,
}

fn rollout_records(
    logical: &[u8],
    path: &Path,
) -> Result<Vec<RolloutRecord>, FixtureMutationError> {
    let text = std::str::from_utf8(logical).map_err(|_| {
        fixture_error(
            "fixture_rollout_utf8_invalid",
            "fixture rollout is not UTF-8 JSONL",
            Some(path.to_path_buf()),
        )
    })?;
    let mut records = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let end = offset + line.len();
        let without_newline = line.strip_suffix('\n').unwrap_or(line);
        let content = without_newline
            .strip_suffix('\r')
            .unwrap_or(without_newline);
        let content_end = offset + content.len();
        if !content.trim().is_empty() {
            let value = serde_json::from_str(content).map_err(|_| {
                fixture_error(
                    "fixture_rollout_record_invalid",
                    "fixture rollout contains invalid JSON",
                    Some(path.to_path_buf()),
                )
            })?;
            records.push(RolloutRecord {
                start: offset,
                content_end,
                end,
                value,
            });
        }
        offset = end;
    }
    if offset < logical.len() {
        return Err(fixture_error(
            "fixture_rollout_split_failed",
            "fixture rollout line boundaries could not be determined",
            Some(path.to_path_buf()),
        ));
    }
    Ok(records)
}

fn rewrite_rollout_header_provider(
    logical: &[u8],
    thread_id: &str,
    source_provider: &str,
    target_provider: &str,
    path: &Path,
) -> Result<(Vec<u8>, i64), FixtureMutationError> {
    let records = rollout_records(logical, path)?;
    let header = records.first().ok_or_else(|| {
        fixture_error(
            "fixture_rollout_header_missing",
            "fixture rollout has no session_meta record",
            Some(path.to_path_buf()),
        )
    })?;
    let mut value = header.value.clone();
    if value.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
        return Err(fixture_error(
            "fixture_rollout_header_unknown",
            "fixture rollout first record is not session_meta",
            Some(path.to_path_buf()),
        ));
    }
    let payload = value
        .get_mut("payload")
        .and_then(JsonValue::as_object_mut)
        .ok_or_else(|| {
            fixture_error(
                "fixture_rollout_header_invalid",
                "fixture session_meta payload is not an object",
                Some(path.to_path_buf()),
            )
        })?;
    if payload.get("id").and_then(JsonValue::as_str) != Some(thread_id)
        || payload.get("model_provider").and_then(JsonValue::as_str) != Some(source_provider)
    {
        return Err(fixture_error(
            "fixture_rollout_header_raced",
            "fixture rollout identity or provider changed after inventory",
            Some(path.to_path_buf()),
        ));
    }
    payload.insert(
        "model_provider".to_owned(),
        JsonValue::String(target_provider.to_owned()),
    );
    let encoded = serde_json::to_vec(&value).map_err(|_| {
        fixture_error(
            "fixture_rollout_header_serialize_failed",
            "fixture rollout header could not be serialized",
            Some(path.to_path_buf()),
        )
    })?;
    let original_header_len = header.content_end - header.start;
    let padded_header_len = original_header_len.max(encoded.len());
    let byte_delta = i64::try_from(padded_header_len)
        .and_then(|new_len| i64::try_from(original_header_len).map(|old_len| new_len - old_len))
        .map_err(|_| {
            fixture_error(
                "fixture_rollout_header_size_overflow",
                "fixture rollout header size exceeds SQLite integer range",
                Some(path.to_path_buf()),
            )
        })?;
    let mut rewritten = Vec::with_capacity(logical.len() + padded_header_len);
    rewritten.extend_from_slice(&logical[..header.start]);
    rewritten.extend_from_slice(&encoded);
    rewritten.resize(rewritten.len() + padded_header_len - encoded.len(), b' ');
    rewritten.extend_from_slice(&logical[header.content_end..]);
    Ok((rewritten, byte_delta))
}

fn shift_fixture_proof_offsets(
    thread: &mut FixtureThreadProof,
    delta: i64,
) -> Result<(), FixtureMutationError> {
    if delta == 0 {
        return Ok(());
    }
    for history in &mut thread.history {
        history.row.next_rollout_byte_offset = history
            .row
            .next_rollout_byte_offset
            .and_then(|offset| offset.checked_add(delta));
        if history.row.next_rollout_byte_offset.is_none() {
            return Err(fixture_error(
                "fixture_history_offset_overflow",
                "fixture projection byte offset overflowed during validation",
                Some(history.database.clone()),
            ));
        }
        for turn in &mut history.row.turns {
            turn.rollout_byte_offset = turn
                .rollout_byte_offset
                .and_then(|offset| offset.checked_add(delta));
            if turn.rollout_byte_offset.is_none() {
                return Err(fixture_error(
                    "fixture_history_offset_overflow",
                    "fixture turn byte offset overflowed during validation",
                    Some(history.database.clone()),
                ));
            }
            if let Some(end_offset) = turn.rollout_end_byte_offset {
                turn.rollout_end_byte_offset =
                    Some(end_offset.checked_add(delta).ok_or_else(|| {
                        fixture_error(
                            "fixture_history_offset_overflow",
                            "fixture turn end byte offset overflowed during validation",
                            Some(history.database.clone()),
                        )
                    })?);
            }
        }
    }
    Ok(())
}

fn read_rollout_logical(rollout: &RolloutArtifact) -> Result<Vec<u8>, FixtureMutationError> {
    let bytes = fs::read(&rollout.path).map_err(|_| {
        fixture_error(
            "fixture_rollout_read_failed",
            "fixture rollout could not be read",
            Some(rollout.path.clone()),
        )
    })?;
    match rollout.encoding {
        RolloutEncoding::Jsonl => Ok(bytes),
        RolloutEncoding::Zstd => {
            let mut decoder = zstd::stream::read::Decoder::new(bytes.as_slice()).map_err(|_| {
                fixture_error(
                    "fixture_rollout_zstd_invalid",
                    "fixture compressed rollout could not be decoded",
                    Some(rollout.path.clone()),
                )
            })?;
            let mut logical = Vec::new();
            decoder.read_to_end(&mut logical).map_err(|_| {
                fixture_error(
                    "fixture_rollout_zstd_read_failed",
                    "fixture compressed rollout could not be read",
                    Some(rollout.path.clone()),
                )
            })?;
            Ok(logical)
        }
    }
}

fn write_rollout_logical(
    rollout: &RolloutArtifact,
    logical: &[u8],
) -> Result<(), FixtureMutationError> {
    let bytes = match rollout.encoding {
        RolloutEncoding::Jsonl => logical.to_vec(),
        RolloutEncoding::Zstd => zstd::stream::encode_all(logical, 1).map_err(|_| {
            fixture_error(
                "fixture_rollout_zstd_write_failed",
                "fixture compressed rollout could not be encoded",
                Some(rollout.path.clone()),
            )
        })?,
    };
    fs::write(&rollout.path, bytes).map_err(|_| {
        fixture_error(
            "fixture_rollout_write_failed",
            "fixture rollout could not be written",
            Some(rollout.path.clone()),
        )
    })
}

fn digest_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn fixture_error(
    code: &'static str,
    message: impl Into<String>,
    path: Option<PathBuf>,
) -> FixtureMutationError {
    FixtureMutationError {
        code,
        message: message.into(),
        path,
    }
}
