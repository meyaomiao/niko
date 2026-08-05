import assert from "node:assert/strict";
import test from "node:test";
import { loadDraftSelection, saveDraftSelection } from "../src/lib/selectionState.ts";

const storage = new Map<string, string>();

Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  },
});

test("draft selections persist by target and reject malformed data", () => {
  storage.clear();
  saveDraftSelection("codex", { provider: "OpenAI", model: "gpt-5.1", group: "gpt-basic" });
  saveDraftSelection("claude-desktop", { provider: "Anthropic", model: "claude-sonnet", group: "claude-basic" });
  assert.deepEqual(loadDraftSelection("codex"), { provider: "OpenAI", model: "gpt-5.1", group: "gpt-basic" });
  assert.deepEqual(loadDraftSelection("claude-desktop"), { provider: "Anthropic", model: "claude-sonnet", group: "claude-basic" });

  storage.set("niko_draft_selections", JSON.stringify({
    codex: { provider: "OpenAI", model: "gpt-5.1", group: "gpt-basic", secret: "discard" },
    bad: { provider: "", model: "gpt-5.1", group: "gpt-basic" },
  }));
  assert.equal(loadDraftSelection("codex"), null);
  assert.equal(loadDraftSelection("bad"), null);
  saveDraftSelection("unsafe", { provider: "OpenAI", model: "sk-secret", group: "gpt-basic" });
  assert.equal(loadDraftSelection("unsafe"), null);
});
