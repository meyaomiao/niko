# Third-Party Notices

## CC Switch

Niko's `codex_sessions` module adapts read-only portions of the following CC
Switch files:

- `src-tauri/src/codex_state_db.rs`: locating `state_5.sqlite` from the Codex
  home plus an explicit `sqlite_home`;
- `src-tauri/src/codex_config.rs`: identifying the shared `custom` provider
  bucket; and
- `src-tauri/src/codex_history_migration.rs`: finding
  `session_meta.payload.model_provider` in rollout headers and recognizing the
  `threads.model_provider` state-database capability and built-in `openai`
  bucket.

Source: `farion1231/cc-switch` commit
`606e7bbe75db7f8285f7a3be006fac22b5d22796`.

Copyright (c) 2025 Jason Young. Licensed under the MIT License. The full
license text is included at `third_party/licenses/CC-Switch-MIT.txt`.

The implementation in Niko is read-only and adds isolated-root validation,
compressed rollout discovery, multi-database inventory, schema capability
detection, paginated history inventory, diagnostics, and dry-run planning.
No CC Switch migration, backup, or write path is included in E10-1.

## Codex++ Compatibility Reference

Codex++ commit `2924333c6770497470090235f484519154698651` was inspected only to
confirm observable on-disk compatibility cases, including arbitrary provider
buckets and databases under `CODEX_HOME/sqlite/`. Codex++ is licensed
AGPL-3.0-only. No Codex++ source code is copied or adapted by Niko.
