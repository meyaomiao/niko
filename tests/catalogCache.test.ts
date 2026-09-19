import assert from "node:assert/strict";
import test, { beforeEach } from "node:test";
import {
  catalogCacheKey,
  clearCatalogCache,
  loadCatalogCached,
  peekCatalogCache,
  type CatalogSnapshot,
} from "../src/lib/catalogCache.ts";
import type { BootstrapData, PricingMeta } from "../src/api/client.ts";

const bootstrap: BootstrapData = {
  site: { base_url: "https://relay.example.com", system_name: "new-api", server_version: "1" },
  user: { id: 1, quota: 10, group: "default" },
  models: ["gpt-5"],
  pricing: [],
  groups: [{ name: "default", desc: "", ratio: 1, models: ["gpt-5"] }],
};

const pricingMeta: PricingMeta = {
  vendors: [{ id: 1, name: "OpenAI" }],
  usableGroup: { default: "默认" },
  groupRatio: { default: 1 },
};

beforeEach(() => {
  clearCatalogCache();
});

test("reuses a warm catalog snapshot instead of fetching again", async () => {
  let loads = 0;
  const loader = async () => {
    loads += 1;
    return { bootstrap, pricingMeta };
  };
  const key = catalogCacheKey("https://relay.example.com", "token");
  const first = await loadCatalogCached(key, loader);
  const second = await loadCatalogCached(key, loader);
  assert.equal(loads, 1);
  assert.equal(first.bootstrap.user.id, 1);
  assert.equal(second.pricingMeta.vendors[0]?.name, "OpenAI");
  assert.equal(peekCatalogCache(key)?.bootstrap.user.group, "default");
});

test("deduplicates inflight catalog loads", async () => {
  let loads = 0;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const loader = async (): Promise<Omit<CatalogSnapshot, "at">> => {
    loads += 1;
    await gate;
    return { bootstrap, pricingMeta };
  };
  const key = catalogCacheKey("https://relay.example.com", "token");
  const a = loadCatalogCached(key, loader);
  const b = loadCatalogCached(key, loader);
  release();
  await Promise.all([a, b]);
  assert.equal(loads, 1);
});

test("force refresh bypasses the warm snapshot", async () => {
  let loads = 0;
  const loader = async () => {
    loads += 1;
    return { bootstrap, pricingMeta };
  };
  const key = catalogCacheKey("https://relay.example.com", "token");
  await loadCatalogCached(key, loader);
  await loadCatalogCached(key, loader, true);
  assert.equal(loads, 2);
});
