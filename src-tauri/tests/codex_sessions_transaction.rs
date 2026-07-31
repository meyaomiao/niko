use niko_lib::codex_sessions::{
    migrate_codex_sessions_transactional, migrate_codex_sessions_transactional_with_faults,
    recover_codex_session_migrations, recover_codex_session_migrations_with_faults,
    scan_codex_sessions, CodexMigrationInput, CodexProcessPolicy, FaultPoint, InjectedFaultKind,
    MigrationErrorKind, MigrationFaultInjector, MigrationOutcome, MigrationProviderTarget,
    MigrationRequest, MigrationState, RolloutEncoding, ScanRequest, CUSTOM_PROVIDER,
    FIXTURE_ROOT_MARKER, FIXTURE_ROOT_MARKER_CONTENT, MIGRATION_ROOT_MARKER,
    MIGRATION_ROOT_MARKER_CONTENT, OFFICIAL_PROVIDER,
};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, Mutex};
use tempfile::{tempdir, TempDir};

const THREAD_A: &str = "019fb2ec-1111-7000-8000-000000000001";
const THREAD_B: &str = "019fb2ec-1111-7000-8000-000000000002";
const AUTH_SENTINEL: &str = "E10-3-AUTH-MUST-NOT-BE-READ-OR-LOGGED";
const INDEX_SENTINEL: &str = "E10-3-INDEX-MUST-STAY-BYTE-EXACT";

struct Fixture {
    _temp: TempDir,
    codex_home: PathBuf,
    sqlite_home: PathBuf,
    request: MigrationRequest,
    rollouts: Vec<(PathBuf, RolloutEncoding)>,
    databases: Vec<PathBuf>,
    config_path: PathBuf,
    index_path: PathBuf,
    auth_path: PathBuf,
}

#[derive(Debug, Eq, PartialEq)]
struct SemanticSnapshot {
    config: Vec<u8>,
    rollouts: Vec<Vec<u8>>,
    index: Vec<u8>,
    auth: Vec<u8>,
    databases: Vec<DatabaseSnapshot>,
}

#[derive(Debug, Eq, PartialEq)]
struct DatabaseSnapshot {
    providers: Vec<(String, String)>,
    cursors: Vec<(String, i64, i64)>,
    indexes: Vec<String>,
}

struct OneFault {
    point: FaultPoint,
    kind: InjectedFaultKind,
    fired: Mutex<bool>,
}

struct NthFault {
    point: FaultPoint,
    remaining: Mutex<usize>,
}

impl NthFault {
    fn new(point: FaultPoint, occurrence: usize) -> Self {
        Self {
            point,
            remaining: Mutex::new(occurrence),
        }
    }
}

impl MigrationFaultInjector for NthFault {
    fn inject(&self, point: FaultPoint, _artifact_id: Option<&str>) -> Option<InjectedFaultKind> {
        if point != self.point {
            return None;
        }
        let mut remaining = self.remaining.lock().unwrap();
        *remaining -= 1;
        (*remaining == 0).then_some(InjectedFaultKind::Crash)
    }
}

impl OneFault {
    fn new(point: FaultPoint, kind: InjectedFaultKind) -> Self {
        Self {
            point,
            kind,
            fired: Mutex::new(false),
        }
    }
}

impl MigrationFaultInjector for OneFault {
    fn inject(&self, point: FaultPoint, _artifact_id: Option<&str>) -> Option<InjectedFaultKind> {
        let mut fired = self.fired.lock().unwrap();
        if !*fired && point == self.point {
            *fired = true;
            Some(self.kind)
        } else {
            None
        }
    }
}

struct BlockingFault {
    barrier: Arc<Barrier>,
    fired: Mutex<bool>,
}

impl MigrationFaultInjector for BlockingFault {
    fn inject(&self, point: FaultPoint, _artifact_id: Option<&str>) -> Option<InjectedFaultKind> {
        let mut fired = self.fired.lock().unwrap();
        if !*fired && point == FaultPoint::PlannedPersisted {
            *fired = true;
            drop(fired);
            self.barrier.wait();
            self.barrier.wait();
        }
        None
    }
}

fn mark_root(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join(MIGRATION_ROOT_MARKER),
        MIGRATION_ROOT_MARKER_CONTENT,
    )
    .unwrap();
    fs::write(root.join(FIXTURE_ROOT_MARKER), FIXTURE_ROOT_MARKER_CONTENT).unwrap();
}

fn rollout_records(thread_id: &str, provider: &str, cwd: &str) -> Vec<JsonValue> {
    vec![
        json!({
            "type": "session_meta",
            "payload": {
                "id": thread_id,
                "model_provider": provider,
                "cwd": cwd,
                "cli_version": "0.99.0-e10-3",
                "unknown": {"preserved": true}
            },
            "unknown_envelope": [1, 2, 3]
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "fixture-visible-body"}]
            }
        }),
    ]
}

fn write_rollout(
    path: &Path,
    thread_id: &str,
    provider: &str,
    cwd: &str,
    encoding: RolloutEncoding,
) -> Vec<u8> {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut logical = Vec::new();
    for record in rollout_records(thread_id, provider, cwd) {
        serde_json::to_writer(&mut logical, &record).unwrap();
        logical.push(b'\n');
    }
    let physical = match encoding {
        RolloutEncoding::Jsonl => logical.clone(),
        RolloutEncoding::Zstd => zstd::stream::encode_all(logical.as_slice(), 1).unwrap(),
    };
    fs::write(path, physical).unwrap();
    logical
}

fn create_state_database(
    path: &Path,
    thread_id: &str,
    rollout_path: &Path,
    provider: &str,
    cwd: &str,
    archived: bool,
) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                model_provider TEXT NOT NULL,
                cwd TEXT NOT NULL,
                archived INTEGER NOT NULL
             );
             CREATE INDEX idx_threads_provider_e10_3 ON threads(model_provider);
             CREATE TABLE wal_probe (key TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO threads (id, rollout_path, model_provider, cwd, archived)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                thread_id,
                rollout_path.to_string_lossy(),
                provider,
                cwd,
                i64::from(archived)
            ],
        )
        .unwrap();
}

fn create_history_database(path: &Path, logical: &[(&str, &[u8])]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE thread_turns (
                thread_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                rollout_ordinal INTEGER NOT NULL,
                rollout_byte_offset INTEGER,
                rollout_end_ordinal INTEGER,
                rollout_end_byte_offset INTEGER,
                status TEXT NOT NULL,
                PRIMARY KEY (thread_id, turn_id)
             );
             CREATE INDEX idx_turns_e10_3 ON thread_turns(thread_id, rollout_ordinal);
             CREATE TABLE thread_items (
                thread_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                rollout_ordinal INTEGER NOT NULL,
                updated_at_ordinal INTEGER NOT NULL,
                item_type TEXT NOT NULL,
                PRIMARY KEY (thread_id, turn_id, item_id)
             );
             CREATE INDEX idx_items_e10_3 ON thread_items(thread_id, rollout_ordinal);
             CREATE TABLE thread_history_projection_state (
                thread_id TEXT PRIMARY KEY,
                next_rollout_byte_offset INTEGER NOT NULL,
                next_rollout_ordinal INTEGER NOT NULL
             );",
        )
        .unwrap();
    for (thread_id, bytes) in logical {
        connection
            .execute(
                "INSERT INTO thread_history_projection_state
                 (thread_id, next_rollout_byte_offset, next_rollout_ordinal)
                 VALUES (?1, ?2, 2)",
                params![thread_id, i64::try_from(bytes.len()).unwrap()],
            )
            .unwrap();
    }
}

fn create_fixture(provider: &str) -> Fixture {
    let temp = tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    let sqlite_home = temp.path().join("sqlite-home");
    mark_root(&codex_home);
    mark_root(&sqlite_home);

    let config_path = codex_home.join("config.toml");
    let config = if provider == OFFICIAL_PROVIDER {
        format!(
            "sqlite_home = {:?}\nmodel_provider = \"openai\"\nmodel = \"fixture\"\n",
            sqlite_home.to_string_lossy()
        )
    } else {
        format!(
            "sqlite_home = {:?}\nmodel_provider = {provider:?}\n\n[model_providers.{provider:?}]\nname = {provider:?}\nbase_url = \"https://fixture.invalid/v1\"\nwire_api = \"responses\"\n",
            sqlite_home.to_string_lossy()
        )
    };
    fs::write(&config_path, config).unwrap();

    let active = codex_home.join("sessions/2026/07/a.jsonl");
    let archived = codex_home.join("archived_sessions/b.jsonl.zst");
    let logical_a = write_rollout(
        &active,
        THREAD_A,
        provider,
        "/fixture/workspace/a",
        RolloutEncoding::Jsonl,
    );
    let logical_b = write_rollout(
        &archived,
        THREAD_B,
        provider,
        "/fixture/workspace/b",
        RolloutEncoding::Zstd,
    );

    let state_a = codex_home.join("state_5.sqlite");
    let state_b = sqlite_home.join("state_5.sqlite");
    let history = sqlite_home.join("thread_history_1.sqlite");
    create_state_database(
        &state_a,
        THREAD_A,
        &active,
        provider,
        "/fixture/workspace/a",
        false,
    );
    create_state_database(
        &state_b,
        THREAD_B,
        &archived.with_extension(""),
        provider,
        "/fixture/workspace/b",
        true,
    );
    create_history_database(
        &history,
        &[
            (THREAD_A, logical_a.as_slice()),
            (THREAD_B, logical_b.as_slice()),
        ],
    );

    let index_path = codex_home.join("session_index.jsonl");
    fs::write(
        &index_path,
        format!(
            "{{\"id\":\"{THREAD_A}\",\"thread_name\":\"{INDEX_SENTINEL}\"}}\n\
             {{\"id\":\"{THREAD_B}\",\"thread_name\":\"{INDEX_SENTINEL}\"}}\n"
        ),
    )
    .unwrap();
    let auth_path = codex_home.join("auth.json");
    fs::write(&auth_path, format!("{{\"token\":\"{AUTH_SENTINEL}\"}}\n")).unwrap();

    let scan = ScanRequest::new(&codex_home).with_sqlite_home(&sqlite_home);
    let mut request = MigrationRequest::new(scan);
    request.options.codex_process_policy = CodexProcessPolicy::IsolatedFixture;
    Fixture {
        _temp: temp,
        codex_home,
        sqlite_home,
        request,
        rollouts: vec![
            (active, RolloutEncoding::Jsonl),
            (archived, RolloutEncoding::Zstd),
        ],
        databases: vec![state_a, state_b, history],
        config_path,
        index_path,
        auth_path,
    }
}

fn semantic_snapshot(fixture: &Fixture) -> SemanticSnapshot {
    let databases = fixture
        .databases
        .iter()
        .map(|path| {
            let connection = Connection::open(path).unwrap();
            let tables = connection
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let providers = if tables.iter().any(|table| table == "threads") {
                connection
                    .prepare("SELECT id, model_provider FROM threads ORDER BY id")
                    .unwrap()
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap()
            } else {
                Vec::new()
            };
            let cursors = if tables
                .iter()
                .any(|table| table == "thread_history_projection_state")
            {
                connection
                    .prepare(
                        "SELECT thread_id, next_rollout_byte_offset, next_rollout_ordinal
                         FROM thread_history_projection_state ORDER BY thread_id",
                    )
                    .unwrap()
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap()
            } else {
                Vec::new()
            };
            let indexes = connection
                .prepare(
                    "SELECT name FROM sqlite_master
                     WHERE type = 'index' AND name NOT LIKE 'sqlite_autoindex_%'
                     ORDER BY name",
                )
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            DatabaseSnapshot {
                providers,
                cursors,
                indexes,
            }
        })
        .collect();
    SemanticSnapshot {
        config: fs::read(&fixture.config_path).unwrap(),
        rollouts: fixture
            .rollouts
            .iter()
            .map(|(path, _)| fs::read(path).unwrap())
            .collect(),
        index: fs::read(&fixture.index_path).unwrap(),
        auth: fs::read(&fixture.auth_path).unwrap(),
        databases,
    }
}

fn raw_business_snapshot(fixture: &Fixture) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut paths = vec![
        fixture.config_path.clone(),
        fixture.index_path.clone(),
        fixture.auth_path.clone(),
    ];
    paths.extend(fixture.rollouts.iter().map(|(path, _)| path.clone()));
    paths.extend(fixture.databases.iter().cloned());
    for database in &fixture.databases {
        let mut wal = database.as_os_str().to_os_string();
        wal.push("-wal");
        let wal = PathBuf::from(wal);
        if wal.exists() {
            paths.push(wal);
        }
    }
    paths
        .into_iter()
        .map(|path| (path.clone(), fs::read(path).unwrap()))
        .collect()
}

fn assert_provider(fixture: &Fixture, provider: &str) {
    let report = scan_codex_sessions(&fixture.request.scan).unwrap();
    assert!(
        !report.is_blocked(),
        "diagnostics: {:#?}",
        report.diagnostics
    );
    assert_eq!(report.config.active_provider.as_deref(), Some(provider));
    assert!(report
        .rollouts
        .iter()
        .all(|rollout| rollout.provider == provider));
    assert!(report
        .sqlite_databases
        .iter()
        .flat_map(|database| database.state_rows.iter())
        .all(|row| row.provider == provider));
}

fn transaction_directories(fixture: &Fixture) -> Vec<PathBuf> {
    let root = fixture.codex_home.join(".niko-session-migrations");
    let mut directories = match fs::read_dir(root) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    directories.sort();
    directories
}

fn latest_journal(fixture: &Fixture) -> JsonValue {
    let mut journals = transaction_directories(fixture)
        .into_iter()
        .map(|directory| directory.join("journal.json"))
        .collect::<Vec<_>>();
    journals.sort_by_key(|path| fs::metadata(path).unwrap().modified().unwrap());
    serde_json::from_slice(&fs::read(journals.last().unwrap()).unwrap()).unwrap()
}

fn assert_no_runtime_writes(fixture: &Fixture) {
    assert!(transaction_directories(fixture).is_empty());
    assert!(!fixture
        .codex_home
        .join("tmp/niko-session-migration.lock")
        .exists());
}

#[test]
fn commits_full_manifest_config_last_and_is_idempotent() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let before = semantic_snapshot(&fixture);
    let report =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap();
    assert_eq!(report.outcome, MigrationOutcome::Committed);
    assert_eq!(report.state, MigrationState::Committed);
    assert!(report.restart_allowed);
    assert_eq!(report.changed_artifacts, 5);
    assert_provider(&fixture, CUSTOM_PROVIDER);

    let after = semantic_snapshot(&fixture);
    assert_eq!(after.auth, before.auth);
    assert_eq!(after.index, before.index);
    assert_eq!(after.databases[2].indexes, before.databases[2].indexes);
    assert_eq!(after.databases[2].cursors, before.databases[2].cursors);

    let journal = latest_journal(&fixture);
    assert_eq!(journal["state"], "committed");
    assert_eq!(journal["restart_allowed"], true);
    let entries = journal["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 8);
    assert_eq!(entries.last().unwrap()["kind"], "config");
    assert!(entries.iter().all(|entry| entry["backup_hash"].is_string()));
    let serialized = serde_json::to_string(&journal).unwrap();
    assert!(!serialized.contains(AUTH_SENTINEL));
    assert!(!serialized.contains(&fixture.codex_home.to_string_lossy().to_string()));
    assert!(!serialized.contains(&fixture.sqlite_home.to_string_lossy().to_string()));

    let directory_count = transaction_directories(&fixture).len();
    let second =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap();
    assert_eq!(second.outcome, MigrationOutcome::AlreadyCurrent);
    assert_eq!(second.migration_id, None);
    assert_eq!(transaction_directories(&fixture).len(), directory_count);
    assert!(recover_codex_session_migrations(&fixture.request)
        .unwrap()
        .migrations
        .is_empty());
}

#[test]
fn provider_route_auth_and_sessions_commit_and_restore_together() {
    let mut fixture = create_fixture(OFFICIAL_PROVIDER);
    let api_key = "fixture-secret-must-not-enter-journal";
    fixture.request.codex = Some(CodexMigrationInput {
        base_url: Some("https://relay.fixture.invalid/v1".to_owned()),
        api_key: Some(api_key.to_owned()),
        model: Some("gpt-fixture".to_owned()),
        mixed: false,
    });

    let applied =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap();
    assert_eq!(applied.outcome, MigrationOutcome::Committed);
    assert_provider(&fixture, CUSTOM_PROVIDER);

    let auth: JsonValue = serde_json::from_slice(&fs::read(&fixture.auth_path).unwrap()).unwrap();
    assert_eq!(auth["OPENAI_API_KEY"], api_key);
    assert_eq!(auth["auth_mode"], "apikey");
    assert_eq!(auth["token"], AUTH_SENTINEL);
    let config = fs::read_to_string(&fixture.config_path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    let provider = config["model_providers"]["custom"].as_table().unwrap();
    assert_eq!(
        provider["base_url"].as_str(),
        Some("https://relay.fixture.invalid/v1")
    );
    assert_eq!(config["model"].as_str(), Some("gpt-fixture"));
    let journal = serde_json::to_string(&latest_journal(&fixture)).unwrap();
    assert!(!journal.contains(api_key));
    assert!(!journal.contains(AUTH_SENTINEL));

    fixture.request.codex = None;
    let restored =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::OpenAi)
            .unwrap();
    assert_eq!(restored.outcome, MigrationOutcome::Committed);
    assert_provider(&fixture, OFFICIAL_PROVIDER);
    let auth: JsonValue = serde_json::from_slice(&fs::read(&fixture.auth_path).unwrap()).unwrap();
    assert!(auth.get("OPENAI_API_KEY").is_none());
    assert!(auth.get("auth_mode").is_none());
    assert_eq!(auth["token"], AUTH_SENTINEL);
    let config = fs::read_to_string(&fixture.config_path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert!(config.get("model_provider").is_none());
    assert!(config.get("model").is_none());
    assert!(config
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .is_none_or(|providers| !providers.contains_key("custom")));
}

#[test]
fn mixed_mode_keeps_chatgpt_auth_and_stages_key_only_in_provider_route() {
    let mut fixture = create_fixture(OFFICIAL_PROVIDER);
    fixture.request.codex = Some(CodexMigrationInput {
        base_url: Some("https://relay.fixture.invalid/v1".to_owned()),
        api_key: Some("fixture-mixed-secret".to_owned()),
        model: None,
        mixed: true,
    });
    migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
        .unwrap();
    let auth: JsonValue = serde_json::from_slice(&fs::read(&fixture.auth_path).unwrap()).unwrap();
    assert!(auth.get("OPENAI_API_KEY").is_none());
    assert!(auth.get("auth_mode").is_none());
    assert_eq!(auth["token"], AUTH_SENTINEL);
    let config = fs::read_to_string(&fixture.config_path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert_eq!(
        config["model_providers"]["custom"]["experimental_bearer_token"].as_str(),
        Some("fixture-mixed-secret")
    );
}

#[test]
fn auth_and_config_commit_crashes_recover_idempotently_in_both_directions() {
    let mut fixture = create_fixture(OFFICIAL_PROVIDER);
    fixture.request.codex = Some(CodexMigrationInput {
        base_url: Some("https://relay.fixture.invalid/v1".to_owned()),
        api_key: Some("fixture-recovery-secret".to_owned()),
        model: Some("gpt-fixture".to_owned()),
        mixed: false,
    });
    let auth_commit_crash = NthFault::new(FaultPoint::CommitArtifact, 5);
    let error = migrate_codex_sessions_transactional_with_faults(
        &fixture.request,
        MigrationProviderTarget::Custom,
        &auth_commit_crash,
    )
    .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::InjectedCrash);
    let recovery = recover_codex_session_migrations(&fixture.request).unwrap();
    assert!(recovery.restart_allowed);
    assert_provider(&fixture, CUSTOM_PROVIDER);
    let auth: JsonValue = serde_json::from_slice(&fs::read(&fixture.auth_path).unwrap()).unwrap();
    assert_eq!(auth["OPENAI_API_KEY"], "fixture-recovery-secret");

    fixture.request.codex = None;
    let config_commit_crash = NthFault::new(FaultPoint::CommitArtifact, 6);
    let error = migrate_codex_sessions_transactional_with_faults(
        &fixture.request,
        MigrationProviderTarget::OpenAi,
        &config_commit_crash,
    )
    .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::InjectedCrash);
    let recovery = recover_codex_session_migrations(&fixture.request).unwrap();
    assert!(recovery.restart_allowed);
    assert_provider(&fixture, OFFICIAL_PROVIDER);
    let auth: JsonValue = serde_json::from_slice(&fs::read(&fixture.auth_path).unwrap()).unwrap();
    assert!(auth.get("OPENAI_API_KEY").is_none());
}

#[test]
fn malformed_auth_or_config_is_zero_write() {
    for broken in ["auth", "config"] {
        let mut fixture = create_fixture(OFFICIAL_PROVIDER);
        fixture.request.codex = Some(CodexMigrationInput {
            base_url: Some("https://relay.fixture.invalid/v1".to_owned()),
            api_key: Some("fixture-secret".to_owned()),
            model: None,
            mixed: false,
        });
        if broken == "auth" {
            fs::write(&fixture.auth_path, b"{not-json").unwrap();
        } else {
            fs::write(&fixture.config_path, b"model_provider = [not-toml").unwrap();
        }
        let before = raw_business_snapshot(&fixture);
        let error =
            migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
                .unwrap_err();
        assert!(matches!(
            error.kind,
            MigrationErrorKind::CorruptStorage | MigrationErrorKind::InvalidRequest
        ));
        assert_eq!(raw_business_snapshot(&fixture), before);
        assert_no_runtime_writes(&fixture);
    }
}

#[test]
fn journal_state_names_roundtrip_exactly() {
    let states = [
        (MigrationState::Planned, "planned"),
        (MigrationState::Snapshotted, "snapshotted"),
        (MigrationState::Staged, "staged"),
        (MigrationState::Committing, "committing"),
        (MigrationState::Validating, "validating"),
        (MigrationState::Committed, "committed"),
        (MigrationState::RollingBack, "rolling_back"),
        (MigrationState::RolledBack, "rolled_back"),
    ];
    for (state, serialized) in states {
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, format!("\"{serialized}\""));
        assert_eq!(
            serde_json::from_str::<MigrationState>(&json).unwrap(),
            state
        );
    }
}

#[test]
fn verified_noop_ignores_locks_for_files_that_need_no_write() {
    let fixture = create_fixture(CUSTOM_PROVIDER);
    fs::create_dir_all(fixture.codex_home.join("tmp/provider-sync.lock")).unwrap();
    fs::write(
        fixture.codex_home.join("tmp/niko-session-migration.lock"),
        b"unrelated-lock-content",
    )
    .unwrap();
    let before = raw_business_snapshot(&fixture);
    let report =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap();
    assert_eq!(report.outcome, MigrationOutcome::AlreadyCurrent);
    assert_eq!(raw_business_snapshot(&fixture), before);
    assert!(transaction_directories(&fixture).is_empty());
    assert_eq!(
        fs::read(fixture.codex_home.join("tmp/niko-session-migration.lock")).unwrap(),
        b"unrelated-lock-content"
    );
    assert!(fixture.codex_home.join("tmp/provider-sync.lock").is_dir());
}

#[test]
fn crash_matrix_recovers_only_complete_old_or_complete_new_state() {
    let cases = [
        (FaultPoint::PlannedPersisted, false),
        (FaultPoint::SnapshotArtifact, false),
        (FaultPoint::SnapshottedPersisted, false),
        (FaultPoint::StageArtifact, false),
        (FaultPoint::StagedPersisted, false),
        (FaultPoint::CommittingPersisted, false),
        (FaultPoint::CommitArtifact, false),
        (FaultPoint::ValidatingPersisted, true),
        (FaultPoint::Validation, true),
        (FaultPoint::CommittedPersisted, true),
    ];
    for (point, expect_new) in cases {
        let fixture = create_fixture(OFFICIAL_PROVIDER);
        let before = semantic_snapshot(&fixture);
        let fault = OneFault::new(point, InjectedFaultKind::Crash);
        let error = migrate_codex_sessions_transactional_with_faults(
            &fixture.request,
            MigrationProviderTarget::Custom,
            &fault,
        )
        .unwrap_err();
        assert_eq!(error.kind, MigrationErrorKind::InjectedCrash, "{point:?}");
        assert!(!error.restart_allowed, "{point:?}");

        let recovery = recover_codex_session_migrations(&fixture.request).unwrap();
        assert!(recovery.restart_allowed, "{point:?}");
        if expect_new {
            assert_provider(&fixture, CUSTOM_PROVIDER);
        } else {
            assert_provider(&fixture, OFFICIAL_PROVIDER);
            assert_eq!(semantic_snapshot(&fixture), before, "{point:?}");
        }
        assert!(recover_codex_session_migrations(&fixture.request)
            .unwrap()
            .migrations
            .is_empty());
    }
}

#[test]
fn migration_retry_recovers_pending_journal_before_replanning() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let crash = OneFault::new(FaultPoint::CommitArtifact, InjectedFaultKind::Crash);
    migrate_codex_sessions_transactional_with_faults(
        &fixture.request,
        MigrationProviderTarget::Custom,
        &crash,
    )
    .unwrap_err();

    let retried =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap();
    assert_eq!(retried.outcome, MigrationOutcome::Committed);
    assert!(retried.restart_allowed);
    assert_provider(&fixture, CUSTOM_PROVIDER);
    assert!(recover_codex_session_migrations(&fixture.request)
        .unwrap()
        .migrations
        .is_empty());
}

#[test]
fn rollback_crash_matrix_is_idempotent() {
    for rollback_point in [
        FaultPoint::RollingBackPersisted,
        FaultPoint::RollbackArtifact,
        FaultPoint::RolledBackPersisted,
    ] {
        let fixture = create_fixture(OFFICIAL_PROVIDER);
        let before = semantic_snapshot(&fixture);
        let initial = OneFault::new(FaultPoint::CommitArtifact, InjectedFaultKind::Crash);
        migrate_codex_sessions_transactional_with_faults(
            &fixture.request,
            MigrationProviderTarget::Custom,
            &initial,
        )
        .unwrap_err();

        let rollback_crash = OneFault::new(rollback_point, InjectedFaultKind::Crash);
        let error = recover_codex_session_migrations_with_faults(&fixture.request, &rollback_crash)
            .unwrap_err();
        assert_eq!(
            error.kind,
            MigrationErrorKind::InjectedCrash,
            "{rollback_point:?}"
        );
        let recovery = recover_codex_session_migrations(&fixture.request).unwrap();
        assert!(recovery.restart_allowed, "{rollback_point:?}");
        assert_provider(&fixture, OFFICIAL_PROVIDER);
        assert_eq!(semantic_snapshot(&fixture), before, "{rollback_point:?}");
        assert!(recover_codex_session_migrations(&fixture.request)
            .unwrap()
            .migrations
            .is_empty());
    }
}

#[test]
fn preflight_faults_prove_zero_business_and_runtime_writes() {
    let cases = [
        (
            FaultPoint::PreflightProviderLock,
            InjectedFaultKind::ProviderSyncLocked,
            MigrationErrorKind::ProviderSyncLocked,
        ),
        (
            FaultPoint::PreflightProcess,
            InjectedFaultKind::CodexRunning,
            MigrationErrorKind::CodexRunning,
        ),
        (
            FaultPoint::PreflightPermission,
            InjectedFaultKind::PermissionDenied,
            MigrationErrorKind::PermissionDenied,
        ),
        (
            FaultPoint::PreflightPermission,
            InjectedFaultKind::FileOccupied,
            MigrationErrorKind::FileOccupied,
        ),
        (
            FaultPoint::PreflightSpace,
            InjectedFaultKind::InsufficientSpace,
            MigrationErrorKind::InsufficientSpace,
        ),
        (
            FaultPoint::PreflightSqliteBusy,
            InjectedFaultKind::SqliteBusy,
            MigrationErrorKind::SqliteBusy,
        ),
    ];
    for (point, fault_kind, expected) in cases {
        let fixture = create_fixture(OFFICIAL_PROVIDER);
        let before = raw_business_snapshot(&fixture);
        let fault = OneFault::new(point, fault_kind);
        let error = migrate_codex_sessions_transactional_with_faults(
            &fixture.request,
            MigrationProviderTarget::Custom,
            &fault,
        )
        .unwrap_err();
        assert_eq!(error.kind, expected, "{point:?}");
        assert_eq!(raw_business_snapshot(&fixture), before, "{point:?}");
        assert_no_runtime_writes(&fixture);
        let debug = format!("{error:?}");
        assert!(!debug.contains(&fixture.codex_home.to_string_lossy().to_string()));
        assert!(!debug.contains(AUTH_SENTINEL));
    }
}

#[test]
fn provider_sync_and_unverifiable_niko_locks_are_distinct_and_zero_write() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let before = raw_business_snapshot(&fixture);
    let provider_lock = fixture.codex_home.join("tmp/provider-sync.lock");
    fs::create_dir_all(&provider_lock).unwrap();
    fs::write(provider_lock.join("owner.json"), "{\"pid\":999999}\n").unwrap();
    let error =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::ProviderSyncLocked);
    assert_eq!(raw_business_snapshot(&fixture), before);
    assert_no_runtime_writes(&fixture);
    fs::remove_dir_all(provider_lock).unwrap();

    fs::create_dir_all(fixture.codex_home.join("tmp")).unwrap();
    fs::write(
        fixture.codex_home.join("tmp/niko-session-migration.lock"),
        b"not-a-verifiable-owner",
    )
    .unwrap();
    let error =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::NikoLockUnverifiable);
    assert_eq!(raw_business_snapshot(&fixture), before);
}

#[test]
fn unstarted_transaction_directories_are_cleaned_and_do_not_block_retry() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let before = raw_business_snapshot(&fixture);
    let migration_id = "0123456789abcdef0123456789abcdef";

    for root in [&fixture.codex_home, &fixture.sqlite_home] {
        let operation = root.join(".niko-session-migrations").join(migration_id);
        fs::create_dir_all(operation.join("backup")).unwrap();
        fs::create_dir_all(operation.join("staged")).unwrap();
    }
    fs::write(
        fixture
            .codex_home
            .join(".niko-session-migrations")
            .join(migration_id)
            .join("journal.json.tmp"),
        b"{",
    )
    .unwrap();

    let recovery = recover_codex_session_migrations(&fixture.request).unwrap();
    assert!(recovery.migrations.is_empty());
    assert_eq!(raw_business_snapshot(&fixture), before);
    assert!(transaction_directories(&fixture).is_empty());
    assert!(!fixture
        .sqlite_home
        .join(".niko-session-migrations")
        .join(migration_id)
        .exists());

    let report =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap();
    assert_eq!(report.outcome, MigrationOutcome::Committed);
    assert_provider(&fixture, CUSTOM_PROVIDER);
}

#[test]
fn live_niko_owner_is_not_cleared() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let barrier = Arc::new(Barrier::new(2));
    let injector = BlockingFault {
        barrier: barrier.clone(),
        fired: Mutex::new(false),
    };
    let request = fixture.request.clone();
    let worker = std::thread::spawn(move || {
        migrate_codex_sessions_transactional_with_faults(
            &request,
            MigrationProviderTarget::Custom,
            &injector,
        )
    });
    barrier.wait();
    let provider_owner: JsonValue = serde_json::from_slice(
        &fs::read(fixture.codex_home.join("tmp/provider-sync.lock/owner.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(provider_owner["owner_kind"], "niko");
    let error = recover_codex_session_migrations(&fixture.request).unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::NikoLocked);
    barrier.wait();
    let report = worker.join().unwrap().unwrap();
    assert_eq!(report.outcome, MigrationOutcome::Committed);
    assert!(!fixture.codex_home.join("tmp/provider-sync.lock").exists());
    assert_provider(&fixture, CUSTOM_PROVIDER);
}

#[test]
fn actual_sqlite_busy_blocks_before_writes_and_retries_after_release() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let mut connection = Connection::open(&fixture.databases[0]).unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let before = raw_business_snapshot(&fixture);
    let error =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::SqliteBusy);
    assert_eq!(raw_business_snapshot(&fixture), before);
    assert_no_runtime_writes(&fixture);
    transaction.rollback().unwrap();
    drop(connection);

    let report =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap();
    assert_eq!(report.outcome, MigrationOutcome::Committed);
}

#[cfg(unix)]
#[test]
fn actual_read_only_artifact_blocks_before_writes() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let path = &fixture.rollouts[0].0;
    let original_mode = fs::metadata(path).unwrap().permissions().mode();
    fs::set_permissions(path, fs::Permissions::from_mode(0o444)).unwrap();
    let before = raw_business_snapshot(&fixture);
    let error =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::PermissionDenied);
    assert_eq!(raw_business_snapshot(&fixture), before);
    assert_no_runtime_writes(&fixture);
    fs::set_permissions(path, fs::Permissions::from_mode(original_mode)).unwrap();
}

#[test]
fn markers_and_unknown_schema_block_without_writes() {
    for external in [false, true] {
        let fixture = create_fixture(OFFICIAL_PROVIDER);
        let root = if external {
            &fixture.sqlite_home
        } else {
            &fixture.codex_home
        };
        fs::remove_file(root.join(MIGRATION_ROOT_MARKER)).unwrap();
        let before = raw_business_snapshot(&fixture);
        let error =
            migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
                .unwrap_err();
        assert_eq!(error.kind, MigrationErrorKind::RootNotAuthorized);
        assert_eq!(raw_business_snapshot(&fixture), before);
        assert_no_runtime_writes(&fixture);
    }

    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let unknown_path = fixture.sqlite_home.join("unknown.db");
    let unknown = Connection::open(&unknown_path).unwrap();
    unknown
        .execute("CREATE TABLE future_unknown (value TEXT)", [])
        .unwrap();
    drop(unknown);
    let unknown_before = fs::read(&unknown_path).unwrap();
    let before = raw_business_snapshot(&fixture);
    let error =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::UnknownSchema);
    assert_eq!(raw_business_snapshot(&fixture), before);
    assert_eq!(fs::read(unknown_path).unwrap(), unknown_before);
    assert_no_runtime_writes(&fixture);

    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let corrupt_path = fixture.sqlite_home.join("corrupt.db");
    fs::write(&corrupt_path, b"not-a-sqlite-database").unwrap();
    let corrupt_before = fs::read(&corrupt_path).unwrap();
    let before = raw_business_snapshot(&fixture);
    let error =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::CorruptStorage);
    assert_eq!(raw_business_snapshot(&fixture), before);
    assert_eq!(fs::read(corrupt_path).unwrap(), corrupt_before);
    assert_no_runtime_writes(&fixture);
}

#[cfg(unix)]
#[test]
fn migration_marker_symlink_is_never_authorization() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let marker = fixture.codex_home.join(MIGRATION_ROOT_MARKER);
    fs::remove_file(&marker).unwrap();
    std::os::unix::fs::symlink(fixture.sqlite_home.join(MIGRATION_ROOT_MARKER), &marker).unwrap();
    let before = raw_business_snapshot(&fixture);
    let error =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::RootNotAuthorized);
    assert_eq!(raw_business_snapshot(&fixture), before);
    assert_no_runtime_writes(&fixture);
}

#[cfg(unix)]
#[test]
fn transaction_runtime_symlink_cannot_escape_approved_root() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let outside = fixture.codex_home.parent().unwrap().join("outside-runtime");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(
        &outside,
        fixture.codex_home.join(".niko-session-migrations"),
    )
    .unwrap();
    let before = raw_business_snapshot(&fixture);
    let error =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::InvalidRequest);
    assert_eq!(raw_business_snapshot(&fixture), before);
    assert!(fs::read_dir(outside).unwrap().next().is_none());
}

#[test]
fn backup_hash_and_final_validation_fail_closed() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let before = semantic_snapshot(&fixture);
    let hash_fault = OneFault::new(
        FaultPoint::SnapshotArtifact,
        InjectedFaultKind::HashMismatch,
    );
    let error = migrate_codex_sessions_transactional_with_faults(
        &fixture.request,
        MigrationProviderTarget::Custom,
        &hash_fault,
    )
    .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::BackupHashMismatch);
    assert!(error.restart_allowed);
    assert_provider(&fixture, OFFICIAL_PROVIDER);
    assert_eq!(semantic_snapshot(&fixture), before);
    assert_eq!(latest_journal(&fixture)["state"], "rolled_back");

    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let before = semantic_snapshot(&fixture);
    let validation_fault =
        OneFault::new(FaultPoint::Validation, InjectedFaultKind::ValidationFailed);
    let error = migrate_codex_sessions_transactional_with_faults(
        &fixture.request,
        MigrationProviderTarget::Custom,
        &validation_fault,
    )
    .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::ValidationFailed);
    assert!(error.restart_allowed);
    assert_provider(&fixture, OFFICIAL_PROVIDER);
    assert_eq!(semantic_snapshot(&fixture), before);
}

#[test]
fn corrupted_backup_blocks_recovery_before_restore() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let crash = OneFault::new(FaultPoint::CommitArtifact, InjectedFaultKind::Crash);
    migrate_codex_sessions_transactional_with_faults(
        &fixture.request,
        MigrationProviderTarget::Custom,
        &crash,
    )
    .unwrap_err();
    let transaction = transaction_directories(&fixture).pop().unwrap();
    let backup = fs::read_dir(transaction.join("backup"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .next()
        .unwrap();
    fs::write(backup, b"corrupted-backup").unwrap();
    let error = recover_codex_session_migrations(&fixture.request).unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::BackupHashMismatch);
    assert!(!error.restart_allowed);
    assert_eq!(latest_journal(&fixture)["state"], "rolling_back");
}

#[test]
fn sqlite_online_backup_captures_committed_wal_state() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let connection = Connection::open(&fixture.databases[0]).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    connection
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    connection
        .execute(
            "INSERT INTO wal_probe (key, value) VALUES ('live', 'captured')",
            [],
        )
        .unwrap();
    let wal_path = {
        let mut path = fixture.databases[0].as_os_str().to_os_string();
        path.push("-wal");
        PathBuf::from(path)
    };
    assert!(wal_path.is_file());
    let shm_path = {
        let mut path = fixture.databases[0].as_os_str().to_os_string();
        path.push("-shm");
        PathBuf::from(path)
    };
    assert!(shm_path.is_file());

    let crash = OneFault::new(FaultPoint::SnapshottedPersisted, InjectedFaultKind::Crash);
    migrate_codex_sessions_transactional_with_faults(
        &fixture.request,
        MigrationProviderTarget::Custom,
        &crash,
    )
    .unwrap_err();
    let transaction = transaction_directories(&fixture).pop().unwrap();
    let captured = fs::read_dir(transaction.join("backup"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|path| {
            let database =
                Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .ok()?;
            database
                .query_row(
                    "SELECT value FROM wal_probe WHERE key = 'live'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .ok()
        })
        .any(|value| value == "captured");
    assert!(captured, "consistent backup omitted committed WAL content");
    drop(connection);
    let recovery = recover_codex_session_migrations(&fixture.request).unwrap();
    assert!(recovery.restart_allowed);
    assert_provider(&fixture, OFFICIAL_PROVIDER);
}

#[cfg(unix)]
#[test]
fn unreadable_auth_json_blocks_with_zero_business_writes() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let before = raw_business_snapshot(&fixture);
    fs::set_permissions(&fixture.auth_path, fs::Permissions::from_mode(0o000)).unwrap();
    let error =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap_err();
    assert_eq!(error.kind, MigrationErrorKind::PermissionDenied);
    fs::set_permissions(&fixture.auth_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(raw_business_snapshot(&fixture), before);
    assert_no_runtime_writes(&fixture);
}

#[cfg(unix)]
#[test]
fn sqlite_backup_stage_and_target_preserve_source_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = create_fixture(OFFICIAL_PROVIDER);
    fs::set_permissions(&fixture.databases[0], fs::Permissions::from_mode(0o600)).unwrap();

    let report =
        migrate_codex_sessions_transactional(&fixture.request, MigrationProviderTarget::Custom)
            .unwrap();
    assert_eq!(report.outcome, MigrationOutcome::Committed);
    assert_eq!(
        fs::metadata(&fixture.databases[0])
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let journal = latest_journal(&fixture);
    let entry = journal["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["kind"] == "sqlite" && entry["locator"]["root"] == "codex")
        .unwrap();
    let artifact_id = entry["artifact_id"].as_str().unwrap();
    let transaction = transaction_directories(&fixture).pop().unwrap();
    for path in [
        transaction
            .join("backup")
            .join(format!("{artifact_id}.backup")),
        transaction
            .join("staged")
            .join(format!("{artifact_id}.stage")),
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn retention_is_bounded_across_repeated_roundtrips() {
    let mut fixture = create_fixture(OFFICIAL_PROVIDER);
    fixture.request.options.retained_transactions = 2;
    for target in [
        MigrationProviderTarget::Custom,
        MigrationProviderTarget::OpenAi,
        MigrationProviderTarget::Custom,
        MigrationProviderTarget::OpenAi,
    ] {
        let report = migrate_codex_sessions_transactional(&fixture.request, target).unwrap();
        assert_eq!(report.outcome, MigrationOutcome::Committed);
        assert!(transaction_directories(&fixture).len() <= 2);
    }
    assert_provider(&fixture, OFFICIAL_PROVIDER);
}

#[cfg(windows)]
#[test]
fn windows_replace_and_recovery_path_is_automated() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let crash = OneFault::new(FaultPoint::CommitArtifact, InjectedFaultKind::Crash);
    migrate_codex_sessions_transactional_with_faults(
        &fixture.request,
        MigrationProviderTarget::Custom,
        &crash,
    )
    .unwrap_err();
    let recovery = recover_codex_session_migrations(&fixture.request).unwrap();
    assert!(recovery.restart_allowed);
    assert_provider(&fixture, OFFICIAL_PROVIDER);
}
