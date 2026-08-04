import assert from "node:assert/strict";
import test from "node:test";
import { buildVendorModelTabs } from "../src/lib/modelSelection.ts";
import type { GroupOption } from "../src/api/client.ts";

const groups: GroupOption[] = [
  { name: "gpt-basic", desc: "Basic", ratio: 1, models: ["gpt-4.1", "gpt-5.1", "custom-openai"] },
  { name: "gpt-premium", desc: "Premium", ratio: 2, models: ["gpt-5.1"] },
  { name: "claude-basic", desc: "Claude", ratio: 1.5, models: ["claude-sonnet-4", "claude-opus-4.1"] },
];

test("orders models by official release date, regardless of catalog input order", () => {
  const tabs = buildVendorModelTabs({
    groups,
    models: ["gpt-4.1", "gpt-5.1", "custom-openai"],
    modelMetadata: {
      "gpt-4.1": { release_date: "2025-04-14", release_source: "OpenAI release notes" },
      "gpt-5.1": { release_date: "2025-11-12", release_source: "OpenAI release notes" },
    },
  });

  const openai = tabs.find((tab) => tab.vendor === "OpenAI")!;
  assert.deepEqual(openai.models.map((m) => m.name), ["gpt-5.1", "gpt-4.1", "custom-openai"]);
  assert.deepEqual(openai.models[0].groups.map((g) => g.name), ["gpt-basic", "gpt-premium"]);
  assert.deepEqual(openai.models[1].groups.map((g) => g.name), ["gpt-basic"]);
});

test("applies release ordering to the complete provider model set", () => {
  const allModels = [
    "model-2024",
    "model-2026-latest",
    "model-2025-late",
    "model-2025-early",
    "model-2025-late-variant",
    "custom-undated",
  ];
  const tabs = buildVendorModelTabs({
    groups: [{ name: "gpt-all", desc: "All", ratio: 1, models: [...allModels].reverse() }],
    models: [...allModels].reverse(),
    modelOrder: [
      "model-2026-latest",
      "model-2025-late",
      "model-2025-late-variant",
      "model-2025-early",
      "model-2024",
      "custom-undated",
    ],
    modelMetadata: {
      "model-2024": { release_date: "2024-01-01", release_source: "official catalog" },
      "model-2026-latest": { release_date: "2026-06-01", release_source: "official catalog" },
      "model-2025-late": { release_date: "2025-11-01", release_source: "official catalog" },
      "model-2025-early": { release_date: "2025-02-01", release_source: "official catalog" },
      "model-2025-late-variant": { release_date: "2025-11-01", release_source: "official catalog" },
    },
  });

  assert.deepEqual(tabs[0].models.map((model) => model.name), [
    "model-2026-latest",
    "model-2025-late",
    "model-2025-late-variant",
    "model-2025-early",
    "model-2024",
    "custom-undated",
  ]);
});

test("uses model_order before conflicting release metadata", () => {
  const tabs = buildVendorModelTabs({
    groups: [{ name: "gpt-basic", desc: "", ratio: 1, models: ["gpt-5.4", "gpt-5.6"] }],
    models: ["gpt-5.4", "gpt-5.6"],
    modelOrder: ["gpt-5.4", "gpt-5.6"],
    modelMetadata: {
      "gpt-5.4": { release_date: "2026-03-05" },
      "gpt-5.6": { release_date: "2026-07-09" },
    },
  });

  assert.deepEqual(tabs[0].models.map((model) => model.name), ["gpt-5.4", "gpt-5.6"]);
});

test("puts GPT-5.6 ahead of GPT-5.4 by the official release dates", () => {
  const tabs = buildVendorModelTabs({
    groups: [{
      name: "gpt-basic",
      desc: "",
      ratio: 1,
      models: ["gpt-5.4", "gpt-5.6-sol", "gpt-5.6-luna"],
    }],
    models: ["gpt-5.4", "gpt-5.6-sol", "gpt-5.6-luna"],
    modelMetadata: {
      "gpt-5.4": { release_date: "2026-03-05", release_source: "https://basellm.github.io/llm-metadata/api/all.json" },
      "gpt-5.6-sol": { release_date: "2026-07-09", release_source: "https://basellm.github.io/llm-metadata/api/all.json" },
      "gpt-5.6-luna": { release_date: "2026-07-09", release_source: "https://basellm.github.io/llm-metadata/api/all.json" },
    },
  });

  assert.deepEqual(tabs[0].models.map((m) => m.name), [
    "gpt-5.6-luna",
    "gpt-5.6-sol",
    "gpt-5.4",
  ]);
});

test("does not treat legacy input order as a release-date signal", () => {
  const tabs = buildVendorModelTabs({
    groups: [{ name: "gpt-basic", desc: "", ratio: 1, models: ["custom-b", "custom-a"] }],
    models: ["custom-b", "custom-a"],
  });

  assert.deepEqual(tabs[0].models.map((m) => m.name), ["custom-a", "custom-b"]);
});

test("uses stable name fallback when legacy metadata is unavailable", () => {
  const tabs = buildVendorModelTabs({
    groups: [{
      name: "claude-basic",
      desc: "",
      ratio: 1,
      models: ["claude-opus-4-1-20250805", "claude-sonnet-4-5-20250929"],
    }],
    models: ["claude-opus-4-1-20250805", "claude-sonnet-4-5-20250929"],
  });

  assert.deepEqual(tabs[0].models.map((m) => m.name), [
    "claude-opus-4-1-20250805",
    "claude-sonnet-4-5-20250929",
  ]);
});

test("uses explicit server catalog order for legacy models", () => {
  const tabs = buildVendorModelTabs({
    groups: [{ name: "gpt-basic", desc: "", ratio: 1, models: ["custom-a", "custom-b", "custom-c"] }],
    models: ["custom-a", "custom-b", "custom-c"],
    modelOrder: ["custom-c", "custom-a", "custom-b"],
  });

  assert.deepEqual(tabs[0].models.map((m) => m.name), ["custom-c", "custom-a", "custom-b"]);
});

test("keeps undated custom models stable after dated models", () => {
  const tabs = buildVendorModelTabs({
    groups: [{ name: "gpt-basic", desc: "", ratio: 1, models: ["custom-b", "gpt-old", "custom-a", "gpt-new"] }],
    models: ["gpt-old", "custom-b", "gpt-new", "custom-a"],
    modelMetadata: {
      "gpt-old": { official_release_date: "2025-01-01", release_source: "official release notes" },
      "gpt-new": { official_release_date: "2026-01-01", release_source: "official release notes" },
    },
  });

  assert.deepEqual(tabs[0].models.map((m) => m.name), ["gpt-new", "gpt-old", "custom-a", "custom-b"]);
});

test("moves the recommended provider to the front without changing model ordering", () => {
  const tabs = buildVendorModelTabs({
    groups,
    models: [
      { name: "claude-sonnet-4", release_date: "2025-05-22" },
      { name: "claude-opus-4.1", release_date: "2025-08-05" },
    ],
    recommendVendor: "Anthropic",
  });

  assert.equal(tabs[0].vendor, "Anthropic");
  assert.deepEqual(tabs[0].models.map((m) => m.name), ["claude-opus-4.1", "claude-sonnet-4"]);
});

test("orders groups by ratio from low to high with a stable name tie-break", () => {
  const tabs = buildVendorModelTabs({
    groups: [
      { name: "gpt-z", desc: "Z", ratio: 2, models: ["gpt-5"] },
      { name: "gpt-b", desc: "B", ratio: 0.5, models: ["gpt-5"] },
      { name: "gpt-a", desc: "A", ratio: 0.5, models: ["gpt-5"] },
    ],
    models: ["gpt-5"],
  });

  assert.deepEqual(tabs[0].models[0].groups.map((group) => group.name), ["gpt-a", "gpt-b", "gpt-z"]);
});
