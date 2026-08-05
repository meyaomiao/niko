import assert from "node:assert/strict";
import test from "node:test";
import {
  commonActiveGroup,
  commonActiveSelection,
  normalizeActiveGroupStatuses,
  normalizeEffectiveSelectionStatuses,
  summarizeActiveGroups,
  summarizeEffectiveSelections,
  type ActiveGroupStatus,
} from "../src/lib/activeGroup.ts";

function status(target_id: string, state: ActiveGroupStatus["status"], group?: string): ActiveGroupStatus {
  return { version: 1, target_id, status: state, ...(group ? { group } : {}) };
}

test("account default recommendation does not masquerade as the active group", () => {
  const statuses = { codex: status("codex", "active", "A") };
  assert.equal(commonActiveGroup(statuses, ["codex"]), "A");
  assert.equal(summarizeActiveGroups(statuses, ["codex"]).text, "当前正在使用的模型服务：A");
  assert.equal(summarizeActiveGroups({}, ["codex"]).kind, "unknown");
});

test("all targets only expose a common group when every target agrees", () => {
  const same = {
    codex: status("codex", "active", "A"),
    "claude-desktop": status("claude-desktop", "active", "A"),
  };
  assert.equal(commonActiveGroup(same, ["codex", "claude-desktop"]), "A");
  assert.equal(summarizeActiveGroups(same, ["codex", "claude-desktop"]).kind, "active");

  const different = {
    ...same,
    "claude-desktop": status("claude-desktop", "active", "B"),
  };
  assert.equal(commonActiveGroup(different, ["codex", "claude-desktop"]), null);
  assert.equal(summarizeActiveGroups(different, ["codex", "claude-desktop"]).kind, "different");
  assert.equal(
    summarizeActiveGroups(
      { codex: status("codex", "active", "A"), "claude-desktop": status("claude-desktop", "unknown") },
      ["codex", "claude-desktop"],
    ).kind,
    "unknown",
  );
});

test("changed, unavailable and delayed states use safe user-facing messages", () => {
  assert.equal(
    summarizeActiveGroups({ codex: status("codex", "changed") }, ["codex"]).text,
    "这个应用的设置后来被改过，请重新接入到应用后再试。",
  );
  assert.equal(
    summarizeActiveGroups({ codex: status("codex", "not_niko") }, ["codex"]).text,
    "当前应用还没有接入 Niko，可选择模型服务后接入。",
  );
  assert.equal(
    summarizeActiveGroups({}, ["codex"], true).text,
    "正在确认当前设置…",
  );
  assert.equal(summarizeActiveGroups({ codex: status("codex", "unreadable") }, ["codex"]).kind, "unknown");
});

test("unknown IPC versions fail closed to unknown", () => {
  const normalized = normalizeActiveGroupStatuses([
    { version: 99, target_id: "codex", status: "active", group: "A" },
    { version: 1, target_id: "claude-desktop", status: "active" },
  ]);
  assert.equal(normalized.codex.status, "unknown");
  assert.equal(normalized["claude-desktop"].status, "unknown");
});

test("effective status requires a confirmed model and exposes provider, model, and group", () => {
  const normalized = normalizeEffectiveSelectionStatuses([
    { version: 1, target_id: "codex", status: "active", group: "gpt-basic", model: "gpt-5.1" },
    { version: 1, target_id: "claude-desktop", status: "active", group: "gpt-basic" },
    { version: 1, target_id: "other", status: "changed", group: "gpt-basic", model: "gpt-5.1" },
  ]);
  assert.equal(commonActiveSelection(normalized, ["codex"])?.provider, "OpenAI");
  assert.deepEqual(commonActiveSelection(normalized, ["codex"]), {
    provider: "OpenAI",
    model: "gpt-5.1",
    group: "gpt-basic",
  });
  assert.equal(normalized["claude-desktop"].status, "unknown");
  assert.equal(commonActiveSelection(normalized, ["claude-desktop"]), null);
  assert.equal(summarizeEffectiveSelections(normalized, ["codex"]).text, "当前正在使用的模型服务：OpenAI · gpt-5.1 · gpt-basic");
  assert.equal(summarizeEffectiveSelections(normalized, ["other"]).kind, "changed");
});

test("effective status rejects unsafe display values and never upgrades unknown states", () => {
  const normalized = normalizeEffectiveSelectionStatuses([
    { version: 1, target_id: "codex", status: "active", group: "/Users/a/config", model: "gpt-5.1" },
    { version: 1, target_id: "claude-desktop", status: "unknown", group: "gpt-basic", model: "gpt-5.1" },
    { version: 1, target_id: "other", status: "unreadable", group: "gpt-basic", model: "gpt-5.1" },
  ]);
  assert.equal(normalized.codex.status, "unknown");
  assert.equal(commonActiveSelection(normalized, ["codex"]), null);
  assert.equal(commonActiveSelection(normalized, ["claude-desktop"]), null);
  assert.equal(commonActiveSelection(normalized, ["other"]), null);
  assert.equal(summarizeEffectiveSelections(normalized, ["claude-desktop"]).kind, "unknown");
  assert.equal(
    summarizeEffectiveSelections(
      { claude: { version: 1, target_id: "claude", status: "not_niko" } },
      ["claude"],
    ).kind,
    "not_niko",
  );
  assert.equal(
    summarizeEffectiveSelections(
      { claude: { version: 1, target_id: "claude", status: "unreadable" } },
      ["claude"],
    ).kind,
    "unknown",
  );
});
