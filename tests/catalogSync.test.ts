import assert from "node:assert/strict";
import test from "node:test";
import type { BootstrapData, PricingItem } from "../src/api/client.ts";
import { buildModelCatalog } from "../src/lib/catalog.ts";
import { mergeOfficialCatalog, modelReleaseDate, catalogSyncLabel } from "../src/lib/catalogSync.ts";
import { pricingFromSnapshot, assembleNewApiCatalog } from "../src/lib/newapi.ts";
import { compareModelsByRelease } from "../src/lib/modelOrder.ts";

const row = (model_name: string, enable_groups: string[], extra: Partial<PricingItem> = {}): PricingItem => ({
  model_name, enable_groups, quota_type: 0, model_ratio: 1, model_price: 0, completion_ratio: 1, ...extra,
});
const account = (): BootstrapData => ({
  site: { base_url: "https://momotoken.win", system_name: "fixture", server_version: "1", quota_per_unit: 500000 },
  user: { id: 42, quota: 12345, group: "allowed" },
  groups: [{ name: "allowed", desc: "账号授权", ratio: 0.3, models: ["stable", "delisted"] }],
  models: ["stable", "delisted"],
  pricing: [row("stable", ["allowed"], { model_ratio: 8, release_date: "2024-01-01" }), row("delisted", ["allowed"])],
  model_metadata: { stable: { release_date: "2024-01-01" }, delisted: { release_date: "2023-01-01" } },
  model_order: ["delisted", "stable"],
});

test("current official catalog adds new models and removes delisted models without expanding account access", () => {
  const original = account(); const before = structuredClone(original);
  const current = [row("new-model", ["allowed", "admin"]), row("stable", ["allowed"], { model_ratio: 2, vendor_id: 99 }), row("restricted", ["admin"])];
  const merged = mergeOfficialCatalog(original, current);
  assert.deepEqual(merged.models, ["new-model", "stable"]);
  assert.deepEqual(merged.groups, [{ ...before.groups![0], models: ["new-model", "stable"] }]);
  assert.deepEqual(merged.user, before.user);
  assert.deepEqual(merged.site, before.site);
  assert.equal(merged.pricing[1].model_ratio, 2);
  assert.equal(merged.pricing[1].vendor_id, 99);
  assert.equal(merged.pricing_authoritative, true);
  assert.equal(merged.pricing.some((item) => item.model_name === "delisted"), false);
  assert.deepEqual(original, before);
});

test("valid current release metadata overrides an older bootstrap date before sorting", () => {
  const merged = mergeOfficialCatalog(account(), [row("stable", ["allowed"], { release_date: "2026-11-01", release_source: "https://brand.example/releases" }), row("other", ["allowed"], { release_date: "2026-01-01" })]);
  assert.equal(modelReleaseDate(merged.model_metadata?.stable), "2026-11-01");
  assert.equal(merged.model_metadata?.stable.release_source, "https://brand.example/releases");
  const sorted = ["other", "stable"].sort((a, b) => compareModelsByRelease(a, b, {
    dateOf: (name) => Date.parse(modelReleaseDate(merged.model_metadata?.[name])),
    orderOf: (name) => merged.model_order?.indexOf(name),
  }));
  assert.deepEqual(sorted, ["stable", "other"]);
  assert.equal(modelReleaseDate({ release_date: "", released_at: "invalid" }, "2026-09-04"), "2026-09-04");
});

test("an authoritative empty catalog never revives old group model lists", () => {
  const merged = mergeOfficialCatalog(account(), []);
  assert.deepEqual(merged.models, []);
  assert.deepEqual(merged.groups?.[0].models, []);
  const catalog = buildModelCatalog(merged.pricing, account().groups!, { authoritative: merged.pricing_authoritative });
  assert.deepEqual(catalog.models, []);
  assert.equal(catalog.source, "pricing");
});

test("explicit empty and unauthorized memberships do not fall back to legacy access", () => {
  for (const membership of [[], ["admin"]]) {
    const catalog = buildModelCatalog([row("stable", membership)], account().groups!);
    assert.deepEqual(catalog.models, []);
    assert.deepEqual(catalog.groupsOf("stable"), []);
  }
});

test("legacy third-party pricing without memberships retains its custom prices and account groups", () => {
  const parsed = pricingFromSnapshot({ data: [{ model_name: "custom-model", model_ratio: 0.123 }] });
  assert.equal(parsed.items[0].enable_groups, undefined);
  const snapshot = assembleNewApiCatalog({
    origin: "https://custom.example", status: { quota_per_unit: 1000 }, user: { id: 3, quota: 10, group: "custom" },
    groups: { custom: { ratio: 0.3 } }, models: ["custom-model"],
    pricing: { data: [{ model_name: "custom-model", model_ratio: 0.123 }], group_ratio: { custom: 0.3 } },
  });
  const catalog = buildModelCatalog(snapshot.bootstrap.pricing, snapshot.bootstrap.groups!);
  assert.equal(catalog.source, "group-models");
  assert.deepEqual(catalog.models.map((model) => model.name), ["custom-model"]);
  assert.equal(snapshot.bootstrap.pricing[0].model_ratio, 0.123);
  assert.equal(snapshot.bootstrap.groups![0].ratio, 0.3);
});

test("a failed refresh reports its older directory time, never the newer balance time as success", () => {
  const at = new Date(2026, 10, 11, 9, 12).getTime();
  const label = catalogSyncLabel({ at: at + 60_000, sync: { source: "account", at } });
  assert.match(label, /同步失败/);
  assert.match(label, /09:12/);
  assert.doesNotMatch(label, /09:13|已同步/);
});
