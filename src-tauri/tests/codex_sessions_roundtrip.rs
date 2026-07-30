use niko_lib::codex_sessions::{
    append_fixture_round, migrate_fixture_provider, scan_codex_sessions, FixtureProviderTarget,
    FixtureThreadProof, RolloutEncoding, ScanRequest, CUSTOM_PROVIDER, FIXTURE_ROOT_MARKER,
    FIXTURE_ROOT_MARKER_CONTENT, OFFICIAL_PROVIDER,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

const THREAD_ACTIVE: &str = "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11";
const THREAD_ARCHIVED: &str = "019fb1b4-f24c-7ec3-a736-c68cf9a0ae12";
const THREAD_EXTERNAL: &str = "019fb1b4-f24c-7ec3-a736-c68cf9a0ae13";

struct Fixture {
    _temp: TempDir,
    codex_home: PathBuf,
    sqlite_home: PathBuf,
    request: ScanRequest,
    rollouts: BTreeMap<String, (PathBuf, RolloutEncoding)>,
    initial_visible: BTreeMap<String, Vec<JsonValue>>,
    state_databases: Vec<PathBuf>,
    history_database: PathBuf,
    auth_bytes: Vec<u8>,
    index_bytes: Vec<u8>,
}

fn mark_fixture_root(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join(FIXTURE_ROOT_MARKER), FIXTURE_ROOT_MARKER_CONTENT).unwrap();
}

fn fixture_config(provider: &str, sqlite_home: &Path) -> String {
    let prefix = format!(
        "sqlite_home = {:?}\nunknown_root = {{ keep = true }}\n",
        sqlite_home.to_string_lossy()
    );
    if provider == OFFICIAL_PROVIDER {
        return format!("{prefix}model = \"fixture-model\"\n");
    }
    format!(
        "{prefix}model_provider = {provider:?}\n\n[model_providers.{provider:?}]\n\
         name = {provider:?}\nbase_url = \"https://{provider}.example/v1\"\n\
         wire_api = \"responses\"\nunknown_provider = \"kept\"\n"
    )
}

fn seed_records(thread_id: &str, provider: &str, workspace: &str) -> Vec<JsonValue> {
    vec![
        json!({
            "type": "session_meta",
            "payload": {
                "id": thread_id,
                "model_provider": provider,
                "cwd": workspace,
                "cli_version": "0.99.0-fixture",
                "unknown_payload": {"keep": [1, 2, 3]}
            },
            "unknown_envelope": {"keep": true}
        }),
        json!({
            "type": "event_msg",
            "payload": {
                "type": "context_compacted",
                "replacement_history": [{"role": "user", "content": "compact-kept"}],
                "unknown_compact": true
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "fixture_tool",
                "call_id": format!("call-{thread_id}"),
                "arguments": "{\"keep\":true}",
                "unknown_tool": ["kept"]
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": format!("user-{thread_id}"),
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "fixture user"},
                    {"type": "input_image", "image_url": format!("file:///fixture/{thread_id}.png"), "unknown_attachment": 7}
                ]
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "reasoning",
                "id": format!("reasoning-{thread_id}"),
                "encrypted_content": format!("encrypted-{thread_id}"),
                "summary": [{"type": "summary_text", "text": "kept"}],
                "unknown_reasoning": {"keep": true}
            },
            "unknown_envelope": "reasoning-kept"
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": format!("assistant-{thread_id}"),
                "response_id": format!("resp-{thread_id}"),
                "role": "assistant",
                "content": [{"type": "output_text", "text": "fixture assistant"}],
                "unknown_payload": {"keep": "yes"}
            }
        }),
    ]
}

fn encode_records(records: &[JsonValue]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for record in records {
        serde_json::to_writer(&mut bytes, record).unwrap();
        bytes.push(b'\n');
    }
    bytes
}

fn write_rollout(path: &Path, records: &[JsonValue], encoding: RolloutEncoding) -> Vec<u8> {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let logical = encode_records(records);
    match encoding {
        RolloutEncoding::Jsonl => fs::write(path, &logical).unwrap(),
        RolloutEncoding::Zstd => fs::write(
            path,
            zstd::stream::encode_all(logical.as_slice(), 1).unwrap(),
        )
        .unwrap(),
    }
    logical
}

fn read_rollout(path: &Path, encoding: RolloutEncoding) -> Vec<u8> {
    let bytes = fs::read(path).unwrap();
    match encoding {
        RolloutEncoding::Jsonl => bytes,
        RolloutEncoding::Zstd => zstd::stream::decode_all(bytes.as_slice()).unwrap(),
    }
}

fn read_records(path: &Path, encoding: RolloutEncoding) -> Vec<JsonValue> {
    String::from_utf8(read_rollout(path, encoding))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn record_spans(logical: &[u8]) -> Vec<(i64, i64)> {
    let text = std::str::from_utf8(logical).unwrap();
    let mut offset = 0_i64;
    let mut spans = Vec::new();
    for line in text.split_inclusive('\n') {
        let end = offset + i64::try_from(line.len()).unwrap();
        if !line.trim().is_empty() {
            spans.push((offset, end));
        }
        offset = end;
    }
    spans
}

fn create_official_state_db(
    path: &Path,
    thread_id: &str,
    rollout_path: &Path,
    provider: &str,
    workspace: &str,
    archived: bool,
) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = Connection::open(path).unwrap();
    // Final `threads` table shape obtained by applying the official state
    // migrations at openai/codex@28f3f1f9 through 0045.
    connection
        .execute_batch(
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
             CREATE INDEX idx_threads_created_at ON threads(created_at DESC, id DESC);
             CREATE INDEX idx_threads_updated_at ON threads(updated_at DESC, id DESC);
             CREATE INDEX idx_threads_archived ON threads(archived);
             CREATE INDEX idx_threads_source ON threads(source);
             CREATE INDEX idx_threads_provider ON threads(model_provider);",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO threads (
                id, rollout_path, created_at, updated_at, source, model_provider, cwd,
                title, sandbox_policy, approval_mode, archived, cli_version,
                first_user_message, preview, recency_at, recency_at_ms, history_mode,
                future_column
             ) VALUES (?1, ?2, 1, 1, 'cli', ?3, ?4, 'fixture', '{}', 'never', ?5,
                       '0.99.0-fixture', 'fixture user', 'fixture user', 1, 1000,
                       'paginated', 'state-unknown-kept')",
            params![
                thread_id,
                rollout_path.to_string_lossy(),
                provider,
                workspace,
                i64::from(archived),
            ],
        )
        .unwrap();
}

fn create_official_history_db(path: &Path, threads: &[(&str, &[u8])]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = Connection::open(path).unwrap();
    // Verbatim schema changes from the official fixed commit:
    // thread_history_migrations/0001 through 0004 at openai/codex@28f3f1f9.
    connection
        .execute_batch(
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
             UPDATE thread_items SET item_type = json_extract(item_json, '$.type') WHERE item_type = '';
             CREATE INDEX idx_thread_items_user_messages
                ON thread_items(thread_id, rollout_ordinal) WHERE item_type = 'userMessage';
             ALTER TABLE thread_turns ADD COLUMN rollout_byte_offset INTEGER;
             ALTER TABLE thread_turns ADD COLUMN rollout_end_ordinal INTEGER;
             ALTER TABLE thread_turns ADD COLUMN rollout_end_byte_offset INTEGER;
             ALTER TABLE thread_items ADD COLUMN updated_at_ordinal INTEGER NOT NULL DEFAULT 0;
             UPDATE thread_items SET updated_at_ordinal = rollout_ordinal;
             CREATE INDEX idx_thread_items_updated_page
                ON thread_items(thread_id, updated_at_ordinal);
             CREATE INDEX idx_thread_items_by_turn_updated_page
                ON thread_items(thread_id, turn_id, updated_at_ordinal);",
        )
        .unwrap();
    for (thread_id, logical) in threads {
        let spans = record_spans(logical);
        assert_eq!(spans.len(), 6);
        let first_turn = format!("seed-a-{thread_id}");
        let second_turn = format!("seed-b-{thread_id}");
        connection
            .execute(
                "INSERT INTO thread_turns (
                    thread_id, turn_id, rollout_ordinal, rollout_byte_offset,
                    rollout_end_ordinal, rollout_end_byte_offset, status,
                    started_at, completed_at, duration_ms
                 ) VALUES (?1, ?2, 1, ?3, 2, ?4, 'completed', 1, 2, 1)",
                params![thread_id, first_turn, spans[1].0, spans[2].1],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO thread_turns (
                    thread_id, turn_id, rollout_ordinal, rollout_byte_offset,
                    rollout_end_ordinal, rollout_end_byte_offset, status,
                    started_at, completed_at, duration_ms
                 ) VALUES (?1, ?2, 3, ?3, 5, ?4, 'completed', 3, 5, 2)",
                params![thread_id, second_turn, spans[3].0, spans[5].1],
            )
            .unwrap();
        for (turn_id, item_id, ordinal, item_type) in [
            (first_turn.as_str(), "seed-user", 1_i64, "userMessage"),
            (second_turn.as_str(), "seed-tool", 3_i64, "dynamicToolCall"),
            (second_turn.as_str(), "seed-agent", 5_i64, "agentMessage"),
        ] {
            let item_id = format!("{item_id}-{thread_id}");
            let item_json = json!({"type": item_type, "id": item_id, "unknown": "kept"});
            connection
                .execute(
                    "INSERT INTO thread_items (
                        thread_id, turn_id, item_id, rollout_ordinal, updated_at_ordinal,
                        created_at_ms, item_type, item_json
                     ) VALUES (?1, ?2, ?3, ?4, ?4, 1, ?5, ?6)",
                    params![
                        thread_id,
                        turn_id,
                        item_id,
                        ordinal,
                        item_type,
                        item_json.to_string(),
                    ],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO thread_history_projection_state (
                    thread_id, next_rollout_byte_offset, next_rollout_ordinal
                 ) VALUES (?1, ?2, 6)",
                params![thread_id, i64::try_from(logical.len()).unwrap()],
            )
            .unwrap();
    }
}

fn create_fixture(provider: &str) -> Fixture {
    let temp = tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    let sqlite_home = temp.path().join("sqlite-home");
    mark_fixture_root(&codex_home);
    mark_fixture_root(&sqlite_home);
    fs::write(
        codex_home.join("config.toml"),
        fixture_config(provider, &sqlite_home),
    )
    .unwrap();
    let auth_bytes = b"{\"token\":\"fixture-auth-must-stay-unread\"}\n".to_vec();
    fs::write(codex_home.join("auth.json"), &auth_bytes).unwrap();
    let index_bytes = [THREAD_ACTIVE, THREAD_ARCHIVED, THREAD_EXTERNAL]
        .into_iter()
        .map(|id| format!("{{\"id\":\"{id}\",\"thread_name\":\"index-kept\"}}\n"))
        .collect::<String>()
        .into_bytes();
    fs::write(codex_home.join("session_index.jsonl"), &index_bytes).unwrap();

    let definitions = [
        (
            THREAD_ACTIVE,
            codex_home.join("sessions/2026/07/active.jsonl"),
            RolloutEncoding::Jsonl,
            "/workspace/active",
            false,
            codex_home.join("state_5.sqlite"),
        ),
        (
            THREAD_ARCHIVED,
            codex_home.join("archived_sessions/archive.jsonl.zst"),
            RolloutEncoding::Zstd,
            "/workspace/archive",
            true,
            codex_home.join("sqlite/codex-dev.db"),
        ),
        (
            THREAD_EXTERNAL,
            codex_home.join("sessions/2026/07/external.jsonl"),
            RolloutEncoding::Jsonl,
            "/workspace/external",
            false,
            sqlite_home.join("state_5.sqlite"),
        ),
    ];
    let mut rollouts = BTreeMap::new();
    let mut initial_visible = BTreeMap::new();
    let mut logical_rollouts = BTreeMap::new();
    let mut state_databases = Vec::new();
    for (thread_id, path, encoding, workspace, archived, state_path) in definitions {
        let records = seed_records(thread_id, provider, workspace);
        let logical = write_rollout(&path, &records, encoding);
        let state_rollout_path = if encoding == RolloutEncoding::Zstd {
            path.with_extension("")
        } else {
            path.clone()
        };
        create_official_state_db(
            &state_path,
            thread_id,
            &state_rollout_path,
            provider,
            workspace,
            archived,
        );
        rollouts.insert(thread_id.to_owned(), (path, encoding));
        initial_visible.insert(thread_id.to_owned(), records[1..].to_vec());
        logical_rollouts.insert(thread_id.to_owned(), logical);
        state_databases.push(state_path);
    }
    let history_database = sqlite_home.join("thread_history_1.sqlite");
    create_official_history_db(
        &history_database,
        &[
            (THREAD_ACTIVE, logical_rollouts[THREAD_ACTIVE].as_slice()),
            (
                THREAD_ARCHIVED,
                logical_rollouts[THREAD_ARCHIVED].as_slice(),
            ),
            (
                THREAD_EXTERNAL,
                logical_rollouts[THREAD_EXTERNAL].as_slice(),
            ),
        ],
    );
    let request = ScanRequest::new(&codex_home).with_sqlite_home(&sqlite_home);
    Fixture {
        _temp: temp,
        codex_home,
        sqlite_home,
        request,
        rollouts,
        initial_visible,
        state_databases,
        history_database,
        auth_bytes,
        index_bytes,
    }
}

fn snapshot_tree(roots: &[&Path]) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let entry_path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                visit(&entry_path, files);
            } else {
                files.insert(entry_path.clone(), fs::read(entry_path).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    for root in roots {
        visit(root, &mut files);
    }
    files
}

fn proof_map(proofs: &[FixtureThreadProof]) -> BTreeMap<String, FixtureThreadProof> {
    proofs
        .iter()
        .cloned()
        .map(|proof| (proof.thread_id.clone(), proof))
        .collect()
}

fn assert_target_provider(request: &ScanRequest, provider: &str) {
    let report = scan_codex_sessions(request).unwrap();
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

#[test]
fn roundtrips_legacy_buckets_with_original_threads_digest_offsets_and_append() {
    for provider in [OFFICIAL_PROVIDER, "momotoken", "codexpp-arbitrary"] {
        let fixture = create_fixture(provider);
        let initial_ids = fixture.rollouts.keys().cloned().collect::<BTreeSet<_>>();

        let to_custom =
            migrate_fixture_provider(&fixture.request, FixtureProviderTarget::Custom).unwrap();
        assert_eq!(to_custom.before_threads, to_custom.after_threads);
        assert!(!to_custom.changed_paths.is_empty());
        assert_target_provider(&fixture.request, CUSTOM_PROVIDER);
        let config = fs::read_to_string(fixture.codex_home.join("config.toml"))
            .unwrap()
            .parse::<toml::Table>()
            .unwrap();
        assert_eq!(
            config
                .get("unknown_root")
                .and_then(toml::Value::as_table)
                .and_then(|value| value.get("keep"))
                .and_then(toml::Value::as_bool),
            Some(true)
        );
        let custom = config["model_providers"][CUSTOM_PROVIDER]
            .as_table()
            .unwrap();
        if provider == OFFICIAL_PROVIDER {
            assert_eq!(custom["name"].as_str(), Some("OpenAI"));
            assert_eq!(custom["requires_openai_auth"].as_bool(), Some(true));
            assert_eq!(custom["supports_websockets"].as_bool(), Some(true));
            assert_eq!(custom["wire_api"].as_str(), Some("responses"));
        } else {
            assert_eq!(
                custom["base_url"].as_str(),
                Some(format!("https://{provider}.example/v1").as_str())
            );
            assert_eq!(custom["unknown_provider"].as_str(), Some("kept"));
            assert!(config["model_providers"].get(provider).is_some());
        }

        let custom_proofs = proof_map(&to_custom.after_threads);
        for thread_id in &initial_ids {
            let append = append_fixture_round(&fixture.request, thread_id, "custom-1").unwrap();
            assert_eq!(append.before.thread_id, append.after.thread_id);
            assert_eq!(append.before.rollout_path, append.after.rollout_path);
            assert_eq!(append.before.workspace, append.after.workspace);
            assert_eq!(append.before.archived, append.after.archived);
            assert_eq!(append.before.state_databases, append.after.state_databases);
            assert_eq!(
                append.after.visible_event_count,
                append.before.visible_event_count + 4
            );
            assert_ne!(
                append.before.visible_history_digest,
                append.after.visible_history_digest
            );
            assert_eq!(
                append.after.history[0].row.next_rollout_byte_offset,
                Some(append.end_byte_offset)
            );
            assert_eq!(
                append.after.history[0].row.next_rollout_ordinal,
                Some(append.end_ordinal + 1)
            );
            assert_eq!(
                custom_proofs[thread_id].history[0].row.turns,
                append.before.history[0].row.turns
            );
        }

        let to_openai =
            migrate_fixture_provider(&fixture.request, FixtureProviderTarget::OpenAi).unwrap();
        assert_eq!(to_openai.before_threads, to_openai.after_threads);
        assert_target_provider(&fixture.request, OFFICIAL_PROVIDER);
        for thread_id in &initial_ids {
            let append = append_fixture_round(&fixture.request, thread_id, "openai-2").unwrap();
            assert_eq!(
                append.after.visible_event_count,
                append.before.visible_event_count + 4
            );
            assert_eq!(
                append.after.history[0].row.next_rollout_byte_offset,
                Some(append.end_byte_offset)
            );
            assert_eq!(
                append.after.history[0].row.next_rollout_ordinal,
                Some(append.end_ordinal + 1)
            );
        }

        let final_report = scan_codex_sessions(&fixture.request).unwrap();
        assert_eq!(
            final_report
                .threads
                .iter()
                .map(|thread| thread.thread_id.clone())
                .collect::<BTreeSet<_>>(),
            initial_ids
        );
        for (thread_id, (path, encoding)) in &fixture.rollouts {
            let records = read_records(path, *encoding);
            assert_eq!(
                records[0]["payload"]["id"].as_str(),
                Some(thread_id.as_str())
            );
            assert_eq!(
                records[0]["payload"]["model_provider"].as_str(),
                Some(OFFICIAL_PROVIDER)
            );
            assert_eq!(records[0]["unknown_envelope"]["keep"].as_bool(), Some(true));
            assert_eq!(
                records[1..1 + fixture.initial_visible[thread_id].len()],
                fixture.initial_visible[thread_id]
            );
            assert!(records.iter().any(|record| {
                record["payload"]["encrypted_content"].as_str()
                    == Some(format!("encrypted-{thread_id}").as_str())
            }));
            assert!(records.iter().any(|record| {
                record["payload"]["response_id"].as_str()
                    == Some(format!("resp-{thread_id}").as_str())
            }));
            assert!(
                records.iter().position(|record| {
                    record["payload"]["unknown_started"].as_bool() == Some(true)
                        && record["unknown_envelope"]["round"].as_str() == Some("custom-1")
                }) < records.iter().position(|record| {
                    record["payload"]["unknown_started"].as_bool() == Some(true)
                        && record["unknown_envelope"]["round"].as_str() == Some("openai-2")
                })
            );
        }
        assert_eq!(
            fs::read(fixture.codex_home.join("auth.json")).unwrap(),
            fixture.auth_bytes
        );
        assert_eq!(
            fs::read(fixture.codex_home.join("session_index.jsonl")).unwrap(),
            fixture.index_bytes
        );
        for database in &fixture.state_databases {
            let connection = Connection::open(database).unwrap();
            let rows = connection
                .prepare("SELECT model_provider, future_column FROM threads")
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(
                rows,
                vec![(
                    OFFICIAL_PROVIDER.to_owned(),
                    "state-unknown-kept".to_owned()
                )]
            );
        }
        let history = Connection::open(&fixture.history_database).unwrap();
        for thread_id in &initial_ids {
            let (next_offset, next_ordinal): (i64, i64) = history
                .query_row(
                    "SELECT next_rollout_byte_offset, next_rollout_ordinal
                     FROM thread_history_projection_state WHERE thread_id = ?1",
                    [thread_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let (path, encoding) = &fixture.rollouts[thread_id];
            assert_eq!(
                next_offset,
                i64::try_from(read_rollout(path, *encoding).len()).unwrap()
            );
            assert_eq!(next_ordinal, 14);
            let appended_lineage: Vec<(String, i64, i64, i64, i64)> = history
                .prepare(
                    "SELECT turn_id, rollout_ordinal, rollout_byte_offset,
                            rollout_end_ordinal, rollout_end_byte_offset
                     FROM thread_turns WHERE thread_id = ?1 AND turn_id LIKE 'poc-turn-%'
                     ORDER BY rollout_ordinal",
                )
                .unwrap()
                .query_map([thread_id], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(appended_lineage.len(), 2);
            assert_eq!(appended_lineage[0].0, "poc-turn-custom-1");
            assert_eq!(appended_lineage[1].0, "poc-turn-openai-2");
            assert!(appended_lineage[0].4 <= appended_lineage[1].2);
            assert_eq!(appended_lineage[1].4, next_offset);
        }
    }
}

#[test]
fn healthy_cc_switch_custom_is_zero_rewrite() {
    let fixture = create_fixture(CUSTOM_PROVIDER);
    let before = snapshot_tree(&[&fixture.codex_home, &fixture.sqlite_home]);
    let migration =
        migrate_fixture_provider(&fixture.request, FixtureProviderTarget::Custom).unwrap();
    let after = snapshot_tree(&[&fixture.codex_home, &fixture.sqlite_home]);
    assert!(migration.changed_paths.is_empty());
    assert_eq!(migration.before_threads, migration.after_threads);
    assert_eq!(before, after);
}

#[test]
fn longer_custom_header_shifts_only_byte_offsets_and_remains_appendable() {
    let fixture = create_fixture("x");
    let migration =
        migrate_fixture_provider(&fixture.request, FixtureProviderTarget::Custom).unwrap();
    for (before, after) in migration
        .before_threads
        .iter()
        .zip(&migration.after_threads)
    {
        assert_eq!(before.thread_id, after.thread_id);
        assert_eq!(before.visible_history_digest, after.visible_history_digest);
        assert_eq!(
            before.provider_neutral_digest,
            after.provider_neutral_digest
        );
        assert_eq!(
            before.history[0]
                .row
                .turns
                .iter()
                .map(|turn| (
                    &turn.turn_id,
                    turn.rollout_ordinal,
                    turn.rollout_end_ordinal
                ))
                .collect::<Vec<_>>(),
            after.history[0]
                .row
                .turns
                .iter()
                .map(|turn| (
                    &turn.turn_id,
                    turn.rollout_ordinal,
                    turn.rollout_end_ordinal
                ))
                .collect::<Vec<_>>()
        );
        let delta = after.history[0].row.next_rollout_byte_offset.unwrap()
            - before.history[0].row.next_rollout_byte_offset.unwrap();
        assert!(delta > 0);
        assert!(before.history[0]
            .row
            .turns
            .iter()
            .zip(&after.history[0].row.turns)
            .all(|(old, new)| {
                new.rollout_byte_offset == old.rollout_byte_offset.map(|offset| offset + delta)
                    && new.rollout_end_byte_offset
                        == old.rollout_end_byte_offset.map(|offset| offset + delta)
            }));
    }
    append_fixture_round(&fixture.request, THREAD_ACTIVE, "shifted").unwrap();
}

#[test]
fn fixture_writes_require_markers_in_every_explicit_root() {
    for remove_from_sqlite_home in [false, true] {
        let fixture = create_fixture(OFFICIAL_PROVIDER);
        let root = if remove_from_sqlite_home {
            &fixture.sqlite_home
        } else {
            &fixture.codex_home
        };
        fs::remove_file(root.join(FIXTURE_ROOT_MARKER)).unwrap();
        let before = snapshot_tree(&[&fixture.codex_home, &fixture.sqlite_home]);

        let migration_error =
            migrate_fixture_provider(&fixture.request, FixtureProviderTarget::Custom).unwrap_err();
        assert_eq!(migration_error.code, "fixture_marker_missing");
        let append_error =
            append_fixture_round(&fixture.request, THREAD_ACTIVE, "unauthorized").unwrap_err();
        assert_eq!(append_error.code, "fixture_marker_missing");
        assert_eq!(
            snapshot_tree(&[&fixture.codex_home, &fixture.sqlite_home]),
            before
        );
    }
}

#[cfg(unix)]
#[test]
fn fixture_marker_must_not_be_a_symlink() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let marker = fixture.codex_home.join(FIXTURE_ROOT_MARKER);
    fs::remove_file(&marker).unwrap();
    std::os::unix::fs::symlink(fixture.sqlite_home.join(FIXTURE_ROOT_MARKER), &marker).unwrap();
    let before = snapshot_tree(&[&fixture.codex_home, &fixture.sqlite_home]);

    let error =
        migrate_fixture_provider(&fixture.request, FixtureProviderTarget::Custom).unwrap_err();
    assert_eq!(error.code, "fixture_marker_invalid");
    assert_eq!(
        snapshot_tree(&[&fixture.codex_home, &fixture.sqlite_home]),
        before
    );
}

#[test]
fn append_blocks_on_projection_byte_offset_mismatch() {
    let fixture = create_fixture(OFFICIAL_PROVIDER);
    let history = Connection::open(&fixture.history_database).unwrap();
    history
        .execute(
            "UPDATE thread_history_projection_state
             SET next_rollout_byte_offset = next_rollout_byte_offset + 1
             WHERE thread_id = ?1",
            [THREAD_ACTIVE],
        )
        .unwrap();
    drop(history);
    let error = append_fixture_round(&fixture.request, THREAD_ACTIVE, "blocked").unwrap_err();
    assert_eq!(error.code, "fixture_append_projection_mismatch");
}

#[test]
#[ignore = "requires an explicitly marked, isolated native CODEX_HOME"]
fn native_codex_home_fixture_probe() {
    let codex_home = std::env::var_os("NIKO_NATIVE_CODEX_HOME")
        .map(PathBuf::from)
        .expect("NIKO_NATIVE_CODEX_HOME must name an isolated fixture root");
    let target = match std::env::var("NIKO_NATIVE_TARGET").as_deref() {
        Ok("custom") => FixtureProviderTarget::Custom,
        Ok("openai") => FixtureProviderTarget::OpenAi,
        _ => panic!("NIKO_NATIVE_TARGET must be custom or openai"),
    };
    let request = ScanRequest::new(&codex_home);
    let migration = migrate_fixture_provider(&request, target).unwrap();
    assert_eq!(migration.before_threads.len(), 1);
    assert_eq!(migration.before_threads, migration.after_threads);
    assert_eq!(
        migration.after.config.active_provider.as_deref(),
        Some(match target {
            FixtureProviderTarget::Custom => CUSTOM_PROVIDER,
            FixtureProviderTarget::OpenAi => OFFICIAL_PROVIDER,
        })
    );
    eprintln!(
        "native fixture thread={} visible_events={} visible_digest={} changed_paths={:?}",
        migration.after_threads[0].thread_id,
        migration.after_threads[0].visible_event_count,
        migration.after_threads[0].visible_history_digest,
        migration.changed_paths
    );
}
