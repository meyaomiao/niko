# Third-Party Notices

## CC Switch

Niko's `codex_sessions` module adapts portions of the following CC
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

The E10-1 implementation was read-only. E10-2 adds a fixture-only provider
header/state-row migration PoC derived from the same MIT baseline, with an
explicit marker required in every writable Codex/SQLite root. E10-3 reuses
that transformation boundary in Niko's own journaled transaction engine.
Niko adds compressed rollout handling, multi-database inventory, official
paginated history cursor validation, provider-neutral digests, byte-offset
repair, consistent SQLite backup, staged atomic replacement, and crash
recovery. No production Tauri command, user-home fallback, or additional CC
Switch source is included.

## OpenAI Codex Schema Reference

The E10-2 tests reproduce the state `threads` capability and the exact
thread-history migration DDL from `openai/codex` commit
`28f3f1f9ef4e9578a5f023f6b6eba018914a5342`, including
`thread_history_migrations/0001` through `0004`. This keeps the offset,
ordinal, and lineage fixture representative of the official fixed schema.

OpenAI Codex is licensed under the Apache License 2.0. The license text is
included at `third_party/licenses/OpenAI-Codex-Apache-2.0.txt`. The upstream
NOTICE from the same fixed commit is reproduced verbatim at
`third_party/notices/OpenAI-Codex-NOTICE.txt`; it preserves the OpenAI Codex
copyright and the Ratatui attribution carried by that source distribution.

## Codex++ Compatibility Reference

Codex++ commit `2924333c6770497470090235f484519154698651` was inspected only to
confirm observable on-disk compatibility cases, including arbitrary provider
buckets, databases under `CODEX_HOME/sqlite/`, and the
`tmp/provider-sync.lock` coordination path. Codex++ is licensed AGPL-3.0-only.
No Codex++ source code is copied or adapted by Niko.
