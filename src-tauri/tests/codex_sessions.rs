use niko_lib::codex_sessions::{
    scan_codex_sessions, DiagnosticLevel, NormalizationStatus, PlanAction, ProviderLayout,
    RolloutEncoding, ScanError, ScanRequest, SqliteSchemaKind, SqliteSidecarKind,
};
use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const BODY_SENTINEL: &str = "BODY-MUST-NOT-APPEAR";
const AUTH_SENTINEL: &str = "AUTH-MUST-NOT-APPEAR";
const INDEX_SENTINEL: &str = "INDEX-MUST-NOT-APPEAR";

#[derive(Clone)]
struct StateSeed<'a> {
    id: &'a str,
    rollout_path: PathBuf,
    provider: &'a str,
    cwd: &'a str,
    archived: bool,
}

fn config_with_provider(provider: &str, sqlite_home: Option<&Path>) -> String {
    let mut config = format!(
        "model_provider = {provider:?}\n\n[model_providers.{provider}]\nname = {provider:?}\n"
    );
    if let Some(sqlite_home) = sqlite_home {
        config = format!(
            "sqlite_home = {:?}\n{config}",
            sqlite_home.to_string_lossy()
        );
    }
    config
}

fn write_rollout(path: &Path, thread_id: &str, provider: &str, cwd: &str, compressed: bool) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let header = json!({
        "type": "session_meta",
        "payload": {
            "id": thread_id,
            "model_provider": provider,
            "cwd": cwd,
            "cli_version": "0.99.0-fixture",
            "future_field": {"kept": true}
        },
        "future_envelope": [1, 2, 3]
    });
    let body = json!({
        "type": "response_item",
        "payload": {"role": "user", "content": BODY_SENTINEL}
    });
    let contents = format!("{header}\n{body}\n");
    if compressed {
        let encoded = zstd::stream::encode_all(contents.as_bytes(), 1).unwrap();
        fs::write(path, encoded).unwrap();
    } else {
        fs::write(path, contents).unwrap();
    }
}

fn create_state_db(path: &Path, rows: &[StateSeed<'_>], wal: bool) -> Connection {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = Connection::open(path).unwrap();
    if wal {
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
    }
    connection
        .execute_batch(
            // Final `threads` shape produced by the official Codex state
            // migrations at openai/codex@28f3f1f9 (0001, 0005, 0007, 0013,
            // 0020, 0022, 0025, 0030, 0032, 0039, 0040, 0041, 0043, 0045).
            "CREATE TABLE thread_sections (id TEXT PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                source TEXT NOT NULL,
                model_provider TEXT NOT NULL,
                cwd TEXT NOT NULL,
                title TEXT NOT NULL,
                sandbox_policy TEXT NOT NULL,
                approval_mode TEXT NOT NULL,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                has_user_event INTEGER NOT NULL DEFAULT 0,
                archived INTEGER NOT NULL DEFAULT 0,
                archived_at INTEGER,
                git_sha TEXT,
                git_branch TEXT,
                git_origin_url TEXT,
                cli_version TEXT NOT NULL DEFAULT '',
                first_user_message TEXT NOT NULL DEFAULT '',
                agent_nickname TEXT,
                agent_role TEXT,
                model TEXT,
                reasoning_effort TEXT,
                agent_path TEXT,
                created_at_ms INTEGER,
                updated_at_ms INTEGER,
                thread_source TEXT,
                preview TEXT NOT NULL DEFAULT '',
                recency_at INTEGER NOT NULL DEFAULT 0,
                recency_at_ms INTEGER NOT NULL DEFAULT 0,
                history_mode TEXT NOT NULL DEFAULT 'legacy',
                name TEXT,
                is_pinned INTEGER NOT NULL DEFAULT 0,
                thread_section_id TEXT REFERENCES thread_sections(id) ON DELETE SET NULL,
                future_column TEXT
            );
            CREATE INDEX idx_threads_provider_fixture ON threads(model_provider);
            CREATE TABLE future_state_metadata (key TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();
    for row in rows {
        connection
            .execute(
                "INSERT INTO threads
                 (id, rollout_path, created_at, updated_at, source, model_provider, cwd,
                  title, sandbox_policy, approval_mode, archived, future_column)
                 VALUES (?1, ?2, 1, 1, 'cli', ?3, ?4, 'fixture', '{}', 'never', ?5,
                         'unknown-column-is-tolerated')",
                params![
                    row.id,
                    row.rollout_path.to_string_lossy(),
                    row.provider,
                    row.cwd,
                    i64::from(row.archived),
                ],
            )
            .unwrap();
    }
    connection
}

fn create_history_db(path: &Path, thread_ids: &[&str]) -> Connection {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            // Exact official thread-history migrations at
            // openai/codex@28f3f1f9: 0001 through 0004.
            "CREATE TABLE thread_turns (
                thread_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                rollout_ordinal INTEGER NOT NULL,
                status TEXT NOT NULL,
                error_json TEXT,
                started_at INTEGER,
                completed_at INTEGER,
                duration_ms INTEGER,
                first_user_item_id TEXT,
                final_agent_item_id TEXT,
                PRIMARY KEY (thread_id, turn_id)
            );
            CREATE UNIQUE INDEX idx_thread_turns_page
                ON thread_turns(thread_id, rollout_ordinal);
            CREATE TABLE thread_items (
                thread_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                rollout_ordinal INTEGER NOT NULL,
                created_at_ms INTEGER NOT NULL,
                item_json TEXT NOT NULL,
                PRIMARY KEY (thread_id, turn_id, item_id)
            );
            CREATE UNIQUE INDEX idx_thread_items_page
                ON thread_items(thread_id, rollout_ordinal);
            CREATE INDEX idx_thread_items_by_turn_page
                ON thread_items(thread_id, turn_id, rollout_ordinal);
            CREATE TABLE thread_history_projection_state (
                thread_id TEXT PRIMARY KEY,
                next_rollout_byte_offset INTEGER NOT NULL,
                next_rollout_ordinal INTEGER NOT NULL
            );
            ALTER TABLE thread_items ADD COLUMN item_type TEXT NOT NULL DEFAULT '';
            CREATE INDEX idx_thread_items_user_messages
                ON thread_items(thread_id, rollout_ordinal)
                WHERE item_type = 'userMessage';
            ALTER TABLE thread_turns ADD COLUMN rollout_byte_offset INTEGER;
            ALTER TABLE thread_turns ADD COLUMN rollout_end_ordinal INTEGER;
            ALTER TABLE thread_turns ADD COLUMN rollout_end_byte_offset INTEGER;
            ALTER TABLE thread_items ADD COLUMN updated_at_ordinal INTEGER NOT NULL DEFAULT 0;
            CREATE INDEX idx_thread_items_updated_page
                ON thread_items(thread_id, updated_at_ordinal);
            CREATE INDEX idx_thread_items_by_turn_updated_page
                ON thread_items(thread_id, turn_id, updated_at_ordinal);",
        )
        .unwrap();
    for thread_id in thread_ids {
        for page in 0..3_i64 {
            let turn_id = format!("turn-{page}");
            let item_id = format!("item-{page}");
            let ordinal = page * 100;
            connection
                .execute(
                    "INSERT INTO thread_turns
                     (thread_id, turn_id, rollout_ordinal, status, rollout_byte_offset,
                      rollout_end_ordinal, rollout_end_byte_offset)
                     VALUES (?1, ?2, ?3, 'completed', ?4, ?5, ?6)",
                    params![
                        thread_id,
                        turn_id,
                        ordinal,
                        page * 1000 + 10,
                        ordinal + 2,
                        page * 1000 + 110,
                    ],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO thread_items
                     (thread_id, turn_id, item_id, rollout_ordinal, updated_at_ordinal,
                      created_at_ms, item_json, item_type)
                     VALUES (?1, ?2, ?3, ?4, ?4, 1, ?5, 'userMessage')",
                    params![thread_id, turn_id, item_id, ordinal + 1, BODY_SENTINEL],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO thread_history_projection_state
                 (thread_id, next_rollout_byte_offset, next_rollout_ordinal)
                 VALUES (?1, 4096, 303)",
                [thread_id],
            )
            .unwrap();
    }
    connection
}

fn snapshot_files(paths: &[PathBuf]) -> BTreeMap<PathBuf, Vec<u8>> {
    paths
        .iter()
        .map(|path| (path.clone(), fs::read(path).unwrap()))
        .collect()
}

fn sqlite_sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(suffix);
    path.into()
}

#[test]
fn inventories_isolated_active_archive_compressed_multi_sqlite_and_history() {
    let temp = tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    let sqlite_home = temp.path().join("independent-sqlite-home");
    fs::create_dir_all(&codex_home).unwrap();
    fs::create_dir_all(&sqlite_home).unwrap();

    let active = codex_home.join("sessions/2026/07/active-a.jsonl");
    let archived = codex_home.join("archived_sessions/archive-b.jsonl.zst");
    let external = codex_home.join("sessions/2026/07/external-c.jsonl");
    write_rollout(&active, "thread-a", "custom", "/workspace/a", false);
    write_rollout(&archived, "thread-b", "custom", "/workspace/b", true);
    write_rollout(&external, "thread-c", "custom", "/workspace/c", false);

    let config_path = codex_home.join("config.toml");
    fs::write(
        &config_path,
        config_with_provider("custom", Some(&sqlite_home)),
    )
    .unwrap();
    let auth_path = codex_home.join("auth.json");
    fs::write(&auth_path, format!(r#"{{"token":"{AUTH_SENTINEL}"}}"#)).unwrap();
    let index_path = codex_home.join("session_index.jsonl");
    fs::write(
        &index_path,
        format!(r#"{{"id":"thread-a","thread_name":"{INDEX_SENTINEL}"}}"#),
    )
    .unwrap();

    let top_state_path = codex_home.join("state_5.sqlite");
    let modern_state_path = codex_home.join("sqlite/codex-dev.db");
    let external_state_path = sqlite_home.join("state_5.sqlite");
    let history_path = sqlite_home.join("thread_history_1.sqlite");
    let top_state = create_state_db(
        &top_state_path,
        &[StateSeed {
            id: "thread-a",
            rollout_path: active.clone(),
            provider: "custom",
            cwd: "/workspace/a",
            archived: false,
        }],
        true,
    );
    top_state
        .execute(
            "UPDATE threads
             SET title = 'A real title', preview = 'A preview', first_user_message = 'First user message',
                 updated_at_ms = 1760000000123
             WHERE id = 'thread-a'",
            [],
        )
        .unwrap();
    let modern_state = create_state_db(
        &modern_state_path,
        &[StateSeed {
            id: "thread-b",
            rollout_path: archived.with_extension(""),
            provider: "custom",
            cwd: "/workspace/b",
            archived: true,
        }],
        false,
    );
    modern_state
        .execute(
            "UPDATE threads
             SET preview = '', first_user_message = 'First user fallback',
                 updated_at_ms = NULL, updated_at = 1760000000
             WHERE id = 'thread-b'",
            [],
        )
        .unwrap();
    let external_state = create_state_db(
        &external_state_path,
        &[StateSeed {
            id: "thread-c",
            rollout_path: external.clone(),
            provider: "custom",
            cwd: "/workspace/c",
            archived: false,
        }],
        false,
    );
    let history = create_history_db(&history_path, &["thread-a", "thread-b", "thread-c"]);

    let protected_paths = vec![
        active.clone(),
        archived.clone(),
        external.clone(),
        config_path.clone(),
        auth_path.clone(),
        index_path.clone(),
        top_state_path.clone(),
        // SQLite readers may update transient read marks in SHM. Durable DB and
        // WAL bytes must remain unchanged.
        sqlite_sidecar_path(&top_state_path, "-wal"),
        modern_state_path.clone(),
        external_state_path.clone(),
        history_path.clone(),
    ];
    let before = snapshot_files(&protected_paths);
    let report =
        scan_codex_sessions(&ScanRequest::new(&codex_home).with_sqlite_home(&sqlite_home)).unwrap();
    let after = snapshot_files(&protected_paths);

    let changed_paths = before
        .iter()
        .filter(|(path, contents)| after.get(*path) != Some(*contents))
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    assert!(
        changed_paths.is_empty(),
        "inventory and dry-run changed fixtures: {changed_paths:?}"
    );
    assert!(
        !report.is_blocked(),
        "diagnostics: {:#?}",
        report.diagnostics
    );
    assert_eq!(report.rollouts.len(), 3);
    assert_eq!(
        report
            .rollouts
            .iter()
            .filter(|rollout| rollout.archived)
            .count(),
        1
    );
    assert!(report
        .rollouts
        .iter()
        .any(|rollout| rollout.encoding == RolloutEncoding::Zstd));
    assert_eq!(report.sqlite_databases.len(), 4);
    assert!(report.sqlite_databases.iter().any(|database| {
        database.path == top_state_path
            && database.schema_kind == SqliteSchemaKind::State
            && database
                .sidecars
                .iter()
                .any(|sidecar| sidecar.kind == SqliteSidecarKind::Wal)
            && database
                .sidecars
                .iter()
                .any(|sidecar| sidecar.kind == SqliteSidecarKind::Shm)
            && database
                .indexes
                .iter()
                .any(|index| index.name == "idx_threads_provider_fixture")
    }));
    let history_db = report
        .sqlite_databases
        .iter()
        .find(|database| database.path == history_path)
        .unwrap();
    assert_eq!(history_db.schema_kind, SqliteSchemaKind::ThreadHistory);
    assert_eq!(history_db.history_rows.len(), 3);
    assert!(history_db.history_rows.iter().all(|row| {
        row.turn_count == 3
            && row.item_count == 3
            && row.first_ordinal == Some(0)
            && row.last_ordinal == Some(202)
            && row.next_rollout_byte_offset == Some(4096)
            && row.next_rollout_ordinal == Some(303)
            && row.turns.iter().all(|turn| {
                turn.rollout_byte_offset.is_some()
                    && turn.rollout_end_ordinal.is_some()
                    && turn.rollout_end_byte_offset.is_some()
            })
    }));
    assert_eq!(report.threads.len(), 3);
    let thread_a = report
        .threads
        .iter()
        .find(|thread| thread.thread_id == "thread-a")
        .unwrap();
    assert_eq!(thread_a.title.as_deref(), Some("A real title"));
    assert_eq!(thread_a.summary.as_deref(), Some("A preview"));
    assert_eq!(thread_a.updated_at_ms, Some(1760000000123));
    let thread_b = report
        .threads
        .iter()
        .find(|thread| thread.thread_id == "thread-b")
        .unwrap();
    assert_eq!(thread_b.summary.as_deref(), Some("First user fallback"));
    assert_eq!(thread_b.updated_at_ms, Some(1760000000000));
    assert!(report
        .threads
        .iter()
        .all(|thread| !thread.history_databases.is_empty()));
    assert_eq!(report.provider_layout, ProviderLayout::CcSwitchCustom);
    assert_eq!(report.normalization.status, NormalizationStatus::NoChanges);
    assert!(report.normalization.actions.is_empty());
    let session_index = report.session_index.as_ref().unwrap();
    assert_eq!(session_index.path, index_path);
    assert_eq!(session_index.entry_count, 1);
    assert_eq!(session_index.thread_ids, vec!["thread-a"]);

    let debug_report = format!("{report:#?}");
    for secret in [BODY_SENTINEL, AUTH_SENTINEL, INDEX_SENTINEL] {
        assert!(!debug_report.contains(secret));
    }

    drop((top_state, modern_state, external_state, history));
}

#[test]
fn reads_legacy_state_schema_without_optional_metadata_columns() {
    let temp = tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(
        codex_home.join("config.toml"),
        config_with_provider("custom", None),
    )
    .unwrap();
    let rollout = codex_home.join("sessions/legacy.jsonl");
    let thread_id = "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11";
    write_rollout(&rollout, thread_id, "custom", "/workspace/legacy", false);

    let state = Connection::open(codex_home.join("state_5.sqlite")).unwrap();
    state
        .execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                model_provider TEXT NOT NULL,
                cwd TEXT NOT NULL,
                archived INTEGER NOT NULL
            );",
        )
        .unwrap();
    state
        .execute(
            "INSERT INTO threads (id, rollout_path, model_provider, cwd, archived)
             VALUES (?1, ?2, 'custom', '/workspace/legacy', 0)",
            params![thread_id, rollout.to_string_lossy()],
        )
        .unwrap();
    drop(state);

    let report = scan_codex_sessions(&ScanRequest::new(&codex_home)).unwrap();
    assert!(
        !report.is_blocked(),
        "diagnostics: {:#?}",
        report.diagnostics
    );
    let thread = report
        .threads
        .iter()
        .find(|thread| thread.thread_id == thread_id)
        .unwrap();
    assert_eq!(thread.title, None);
    assert_eq!(thread.summary, None);
    assert_eq!(thread.updated_at_ms, None);
}

fn create_single_layout_fixture(provider: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    fs::create_dir_all(&codex_home).unwrap();
    if provider == "openai" {
        fs::write(codex_home.join("config.toml"), "model = \"fixture\"\n").unwrap();
    } else {
        fs::write(
            codex_home.join("config.toml"),
            config_with_provider(provider, None),
        )
        .unwrap();
    }
    let rollout = codex_home.join("sessions/thread.jsonl");
    write_rollout(
        &rollout,
        "thread-layout",
        provider,
        "/workspace/layout",
        false,
    );
    drop(create_state_db(
        &codex_home.join("state_5.sqlite"),
        &[StateSeed {
            id: "thread-layout",
            rollout_path: rollout,
            provider,
            cwd: "/workspace/layout",
            archived: false,
        }],
        false,
    ));
    (temp, codex_home)
}

#[test]
fn classifies_legacy_buckets_and_builds_deterministic_dry_runs() {
    for (provider, expected_layout) in [
        ("openai", ProviderLayout::Official),
        ("momotoken", ProviderLayout::NikoMomotoken),
        ("codexpp-provider", ProviderLayout::CodexPlusPlusCompatible),
    ] {
        let (_temp, codex_home) = create_single_layout_fixture(provider);
        let request = ScanRequest::new(&codex_home);
        let first = scan_codex_sessions(&request).unwrap();
        let second = scan_codex_sessions(&request).unwrap();

        assert_eq!(first.provider_layout, expected_layout);
        assert_eq!(first.normalization, second.normalization);
        assert_eq!(
            first.normalization.status,
            NormalizationStatus::WouldNormalize
        );
        assert_eq!(first.normalization.actions.len(), 3);
        assert!(matches!(
            first.normalization.actions[0],
            PlanAction::ConfigureCustomBucket { .. }
        ));
        assert!(!first.is_blocked(), "diagnostics: {:#?}", first.diagnostics);
    }
}

#[test]
fn mixed_official_and_niko_buckets_have_a_deterministic_plan() {
    let temp = tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(
        codex_home.join("config.toml"),
        config_with_provider("momotoken", None),
    )
    .unwrap();
    let official = codex_home.join("sessions/official.jsonl");
    let niko = codex_home.join("archived_sessions/niko.jsonl");
    write_rollout(
        &official,
        "thread-official",
        "openai",
        "/workspace/official",
        false,
    );
    write_rollout(&niko, "thread-niko", "momotoken", "/workspace/niko", false);
    drop(create_state_db(
        &codex_home.join("state_5.sqlite"),
        &[
            StateSeed {
                id: "thread-official",
                rollout_path: official,
                provider: "openai",
                cwd: "/workspace/official",
                archived: false,
            },
            StateSeed {
                id: "thread-niko",
                rollout_path: niko,
                provider: "momotoken",
                cwd: "/workspace/niko",
                archived: true,
            },
        ],
        false,
    ));

    let request = ScanRequest::new(&codex_home);
    let first = scan_codex_sessions(&request).unwrap();
    let second = scan_codex_sessions(&request).unwrap();
    assert_eq!(first.provider_layout, ProviderLayout::Mixed);
    assert_eq!(first.normalization, second.normalization);
    assert_eq!(first.normalization.actions.len(), 5);
    assert_eq!(
        first.normalization.status,
        NormalizationStatus::WouldNormalize
    );
    assert!(!first.is_blocked(), "diagnostics: {:#?}", first.diagnostics);
}

#[test]
fn unknown_schema_corruption_and_duplicate_thread_ids_block_the_plan() {
    let temp = tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(
        codex_home.join("config.toml"),
        config_with_provider("custom", None),
    )
    .unwrap();
    fs::write(codex_home.join("session_index.jsonl"), "{invalid-json}\n").unwrap();
    let first = codex_home.join("sessions/duplicate-a.jsonl");
    let second = codex_home.join("archived_sessions/duplicate-b.jsonl");
    write_rollout(
        &first,
        "duplicate-thread",
        "custom",
        "/workspace/duplicate",
        false,
    );
    write_rollout(
        &second,
        "duplicate-thread",
        "custom",
        "/workspace/duplicate",
        false,
    );
    let corrupt = codex_home.join("sessions/corrupt.jsonl.zst");
    fs::write(&corrupt, b"not a zstd stream").unwrap();
    drop(create_state_db(
        &codex_home.join("state_5.sqlite"),
        &[
            StateSeed {
                id: "duplicate-thread",
                rollout_path: first,
                provider: "custom",
                cwd: "/workspace/duplicate",
                archived: false,
            },
            StateSeed {
                id: "invalid-state-row",
                rollout_path: codex_home.join("sessions/missing.jsonl"),
                provider: "",
                cwd: "/workspace/invalid",
                archived: false,
            },
        ],
        false,
    ));
    drop(create_history_db(
        &codex_home.join("sqlite/history-a.db"),
        &["history-duplicate"],
    ));
    drop(create_history_db(
        &codex_home.join("sqlite/history-b.db"),
        &["history-duplicate"],
    ));
    let unknown_path = codex_home.join("sqlite/future.db");
    fs::create_dir_all(unknown_path.parent().unwrap()).unwrap();
    let unknown = Connection::open(&unknown_path).unwrap();
    unknown
        .execute("CREATE TABLE mystery (future_value TEXT)", [])
        .unwrap();
    drop(unknown);

    let report = scan_codex_sessions(&ScanRequest::new(&codex_home)).unwrap();
    let codes = report
        .diagnostics
        .iter()
        .filter(|item| item.level == DiagnosticLevel::Blocker)
        .map(|item| item.code)
        .collect::<Vec<_>>();
    assert!(codes.contains(&"duplicate_thread_id"));
    assert!(codes.contains(&"session_index_entry_invalid"));
    assert!(codes.contains(&"sqlite_state_row_invalid"));
    assert!(codes.contains(&"sqlite_schema_unknown"));
    assert!(
        codes.contains(&"rollout_header_unreadable") || codes.contains(&"rollout_zstd_invalid")
    );
    assert!(report.diagnostics.iter().any(|item| {
        item.code == "duplicate_thread_id" && item.thread_id.as_deref() == Some("history-duplicate")
    }));
    assert_eq!(report.normalization.status, NormalizationStatus::Blocked);
    assert!(report.normalization.actions.is_empty());
}

#[test]
fn refuses_implicit_or_unapproved_roots_without_scanning_them() {
    let temp = tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    let unapproved = temp.path().join("outside-sqlite-home");
    fs::create_dir_all(&codex_home).unwrap();
    fs::create_dir_all(&unapproved).unwrap();
    fs::write(unapproved.join("state_5.sqlite"), b"must not be opened").unwrap();
    fs::write(
        codex_home.join("config.toml"),
        config_with_provider("custom", Some(&unapproved)),
    )
    .unwrap();

    let report = scan_codex_sessions(&ScanRequest::new(&codex_home)).unwrap();
    assert!(report
        .diagnostics
        .iter()
        .any(|item| item.code == "sqlite_home_not_approved"));
    assert!(report
        .sqlite_databases
        .iter()
        .all(|database| !database.path.starts_with(&unapproved)));
    assert!(!report
        .diagnostics
        .iter()
        .any(|item| item.code == "sqlite_unreadable"
            && item
                .path
                .as_ref()
                .is_some_and(|path| path.starts_with(&unapproved))));

    fs::write(
        codex_home.join("config.toml"),
        "sqlite_home = \"~/.codex\"\nmodel_provider = \"custom\"\n\n[model_providers.custom]\nname = \"custom\"\n",
    )
    .unwrap();
    let tilde_report = scan_codex_sessions(&ScanRequest::new(&codex_home)).unwrap();
    assert!(tilde_report
        .diagnostics
        .iter()
        .any(|item| item.code == "config_sqlite_home_invalid"));

    let relative = scan_codex_sessions(&ScanRequest::new("relative-codex-home"));
    assert!(matches!(
        relative,
        Err(ScanError::CodexHomeMustBeAbsolute(_))
    ));
}

#[test]
fn recognizes_current_codex_auxiliary_databases() {
    let temp = tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    fs::create_dir_all(codex_home.join("sqlite")).unwrap();
    fs::write(codex_home.join("config.toml"), "model = \"fixture\"\n").unwrap();

    let catalog = Connection::open(codex_home.join("sqlite/codex-dev.db")).unwrap();
    catalog
        .execute_batch(
            "CREATE TABLE local_thread_catalog (
                 host_id TEXT NOT NULL,
                 thread_id TEXT NOT NULL,
                 display_title TEXT NOT NULL,
                 source_created_at REAL NOT NULL,
                 source_updated_at REAL NOT NULL,
                 cwd TEXT NOT NULL,
                 source_kind TEXT NOT NULL,
                 model_provider TEXT NOT NULL,
                 PRIMARY KEY (host_id, thread_id)
             );
             CREATE TABLE local_thread_catalog_metadata (
                 id INTEGER PRIMARY KEY,
                 catalog_revision INTEGER NOT NULL
             );",
        )
        .unwrap();
    drop(catalog);

    let snapshots =
        Connection::open(codex_home.join("sqlite/codex-history-snapshots-dev.db")).unwrap();
    snapshots
        .execute(
            "CREATE TABLE app_server_history_snapshots (
                 principal_key TEXT NOT NULL,
                 host_id TEXT NOT NULL,
                 thread_id TEXT NOT NULL,
                 accessed_at INTEGER NOT NULL,
                 payload_bytes INTEGER NOT NULL,
                 payload_json TEXT NOT NULL,
                 PRIMARY KEY (principal_key, host_id, thread_id)
             )",
            [],
        )
        .unwrap();
    drop(snapshots);

    let report = scan_codex_sessions(&ScanRequest::new(&codex_home)).unwrap();
    assert!(
        !report.is_blocked(),
        "diagnostics: {:#?}",
        report.diagnostics
    );
    assert_eq!(report.sqlite_databases.len(), 2);
    assert!(report
        .sqlite_databases
        .iter()
        .all(|database| database.schema_kind == SqliteSchemaKind::Auxiliary));
}
