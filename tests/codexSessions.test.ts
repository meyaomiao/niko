import assert from "node:assert/strict";
import test from "node:test";
import {
  codexNormalizationLabel,
  codexProviderLabel,
  filterCodexSessionThreads,
  type CodexSessionThread,
} from "../src/lib/codexSessions.ts";

const threads: CodexSessionThread[] = [
  {
    thread_id: "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11",
    providers: ["custom"],
    workspaces: ["/Users/alice/Niko"],
    archived: false,
    rollout_count: 2,
  },
  {
    thread_id: "019fb1b4-f24c-7ec3-a736-c68cf9a0ae12",
    providers: ["openai"],
    workspaces: ["/Users/alice/notes"],
    archived: true,
    rollout_count: 1,
  },
];

test("filters local sessions by thread id, provider, workspace, and archive state", () => {
  assert.equal(filterCodexSessionThreads(threads, "AE11").length, 1);
  assert.equal(filterCodexSessionThreads(threads, "niko")[0]?.thread_id, threads[0].thread_id);
  assert.equal(filterCodexSessionThreads(threads, "archived")[0]?.thread_id, threads[1].thread_id);
  assert.equal(filterCodexSessionThreads(threads, "").length, 2);
});

test("keeps provider and normalization labels in plain language", () => {
  assert.equal(codexProviderLabel("openai"), "官方");
  assert.equal(codexProviderLabel("momotoken"), "旧版 Niko");
  assert.equal(codexProviderLabel("codex-plus-plus"), "兼容来源");
  assert.equal(codexNormalizationLabel("no_changes"), "当前状态正常");
  assert.equal(codexNormalizationLabel("needs_check"), "发现需要整理的会话");
});
