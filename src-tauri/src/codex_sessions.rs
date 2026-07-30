//! Read-only Codex session storage inventory and normalization planning.
//!
//! Portions of the path resolution and provider-bucket detection are adapted
//! from CC Switch commit 606e7bbe75db7f8285f7a3be006fac22b5d22796,
//! Copyright (c) 2025 Jason Young, under the MIT License. See
//! `third_party/licenses/CC-Switch-MIT.txt` and `THIRD_PARTY_NOTICES.md`.
//!
//! This module deliberately has no home-directory or environment fallback.
//! Callers must provide the Codex home, and must separately approve an
//! external SQLite home before configuration can lead the scan outside it.

use rusqlite::{Connection, OpenFlags};
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const CUSTOM_PROVIDER: &str = "custom";
pub const OFFICIAL_PROVIDER: &str = "openai";
pub const NIKO_PROVIDER: &str = "momotoken";

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
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistoryThreadRow {
    pub thread_id: String,
    pub turn_count: u64,
    pub item_count: u64,
    pub first_ordinal: Option<i64>,
    pub last_ordinal: Option<i64>,
    pub next_rollout_ordinal: Option<i64>,
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
        match read_state_rows(&connection) {
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
            &["thread_id", "turn_id", "rollout_ordinal"],
        ),
        capability_status(
            tables,
            "thread_items",
            &["thread_id", "turn_id", "item_id", "rollout_ordinal"],
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
        "external_agent_config_imports",
        "logs",
        "memories",
        "memory_usage",
        "remote_control_enrollments",
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

fn read_state_rows(connection: &Connection) -> rusqlite::Result<Vec<StateThreadRow>> {
    let mut statement = connection.prepare(
        "SELECT id, rollout_path, model_provider, cwd, archived FROM threads ORDER BY id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(StateThreadRow {
                thread_id: row.get(0)?,
                rollout_path: PathBuf::from(row.get::<_, String>(1)?),
                provider: row.get(2)?,
                workspace: PathBuf::from(row.get::<_, String>(3)?),
                archived: row.get::<_, i64>(4)? != 0,
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
    next_rollout_ordinal: Option<i64>,
}

fn read_history_rows(connection: &Connection) -> rusqlite::Result<Vec<HistoryThreadRow>> {
    let mut rows = BTreeMap::<String, HistoryBuilder>::new();
    {
        let mut statement = connection.prepare(
            "SELECT thread_id, COUNT(*), MIN(rollout_ordinal), MAX(rollout_ordinal) \
             FROM thread_turns GROUP BY thread_id ORDER BY thread_id",
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
            entry.turn_count = count;
            merge_ordinal_range(entry, first, last);
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
            "SELECT thread_id, next_rollout_ordinal \
             FROM thread_history_projection_state ORDER BY thread_id",
        )?;
        for row in statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })? {
            let (thread_id, next_ordinal) = row?;
            rows.entry(thread_id).or_default().next_rollout_ordinal = Some(next_ordinal);
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
            next_rollout_ordinal: row.next_rollout_ordinal,
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
            .push(rollout.logical_path.clone());
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
    plain_rollout_path(&path)
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
