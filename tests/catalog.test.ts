import assert from "node:assert/strict";
import test from "node:test";
import { buildModelCatalog, bucketByVendor } from "../src/lib/catalog.ts";
import type { GroupOption, PricingItem } from "../src/api/client.ts";

const groups = (names: string[], models: string[] = []): GroupOption[] =>
  names.map((name) => ({ name, desc: "", ratio: 1, models }));

const pricing = (entries: [string, string[] | undefined][]): PricingItem[] =>
  entries.map(([model_name, enable_groups]) => ({
    model_name,
    quota_type: 0,
    model_ratio: 1,
    model_price: 0,
    completion_ratio: 1,
    enable_groups,
  }));

test("用 pricing.enable_groups 反查可用模型（服务端不给 groups[].models 时也能工作）", () => {
  const catalog = buildModelCatalog(
    pricing([
      ["claude-opus-5", ["claude-pro", "claude-max"]],
      ["gpt-4o", ["gpt-plus"]],
      ["gemini-2.5-pro", ["gemini"]],
    ]),
    groups(["claude-pro", "gpt-plus"]),
  );

  assert.equal(catalog.source, "pricing");
  assert.deepEqual(
    catalog.models.map((m) => m.name).sort(),
    ["claude-opus-5", "gpt-4o"],
  );
  // 只保留账号可用分组：claude-max 不在账号分组里，必须被过滤
  assert.deepEqual(catalog.groupsOf("claude-opus-5"), ["claude-pro"]);
  assert.deepEqual(catalog.groupsOf("gemini-2.5-pro"), []);
});

test("pricing 没有 enable_groups 时退回 groups[].models（旧响应兼容）", () => {
  const catalog = buildModelCatalog(
    pricing([["claude-opus-5", undefined]]),
    groups(["claude-pro"], ["claude-opus-5", "legacy-model"]),
  );
  assert.equal(catalog.source, "group-models");
  assert.deepEqual(catalog.models.map((m) => m.name).sort(), ["claude-opus-5", "legacy-model"]);
  assert.deepEqual(catalog.groupsOf("legacy-model"), ["claude-pro"]);
});

test("两边都没有时给出 empty，不抛错", () => {
  const catalog = buildModelCatalog(undefined, []);
  assert.equal(catalog.source, "empty");
  assert.deepEqual(catalog.models, []);
  assert.deepEqual(catalog.groupsOf("x"), []);
});

test("按厂家分桶并按数量降序", () => {
  const catalog = buildModelCatalog(
    pricing([
      ["claude-a", ["g"]],
      ["claude-b", ["g"]],
      ["gpt-a", ["g"]],
    ]),
    groups(["g"]),
  );
  const vendorOf = (model: string) => (model.startsWith("claude") ? "Anthropic" : "OpenAI");
  const buckets = bucketByVendor(catalog.models, vendorOf);
  assert.deepEqual(buckets.map((b) => b.vendor), ["Anthropic", "OpenAI"]);
  assert.equal(buckets[0].models.length, 2);
  assert.equal(buckets[1].models.length, 1);
});
