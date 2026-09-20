import assert from "node:assert/strict";
import test from "node:test";
import {
  UsageLoadError,
  collectUsage,
  localDate,
  usageRange,
  usageTrend,
  usageUnit,
  type UsagePage,
} from "../src/lib/usageLedger.ts";
import { mapNewApiLogs, summarizeNewApiLogs } from "../src/lib/newapi.ts";
import type { UsageLogItem, UsageQuery } from "../src/api/client.ts";

const pageOf =
  (rows: UsageLogItem[], total?: number, totalPages?: number) =>
  async (query: UsageQuery): Promise<UsagePage> => {
    const pageNumber = query.page ?? 1;
    const size = query.pageSize ?? 100;
    const pages = totalPages ?? Math.ceil((total ?? rows.length) / size);
    if (pageNumber > pages) return { items: [], total };
    return {
      items: rows.slice((pageNumber - 1) * size, pageNumber * size),
      total,
    };
  };

const row = (over: Partial<UsageLogItem> = {}): UsageLogItem => ({
  id: 1,
  created_at: 1_700_000_000,
  model_name: "gpt-5",
  prompt_tokens: 10,
  completion_tokens: 20,
  quota: 5_000,
  group: "default",
  ...over,
});

test("usageRange covers preset ranges in local days and bounds all", () => {
  const now = new Date("2026-09-20T15:30:00");
  assert.deepEqual(usageRange("today", "", "", now), [
    new Date("2026-09-20T00:00:00").getTime() / 1000,
    new Date("2026-09-20T23:59:59").getTime() / 1000,
  ]);
  const [start7, end7] = usageRange("7d", "", "", now);
  assert.equal(new Date(start7 * 1000).getDate(), 14);
  assert.equal(end7, new Date("2026-09-20T23:59:59").getTime() / 1000);
  const [startAll, endAll] = usageRange("all", "", "", now);
  assert.equal(startAll, 0);
  assert.ok(endAll <= now.getTime() / 1000);
  const [startCustom, endCustom] = usageRange("custom", "2026-01-01", "2026-01-03", now);
  assert.equal(startCustom, new Date("2026-01-01T00:00:00").getTime() / 1000);
  assert.equal(endCustom, new Date("2026-01-03T23:59:59").getTime() / 1000);
});

test("usageRange rejects empty or inverted custom ranges instead of querying everything", () => {
  const now = new Date("2026-09-20T15:30:00");
  assert.throws(() => usageRange("custom", "", "", now), UsageLoadError);
  assert.throws(() => usageRange("custom", "2026-09-10", "2026-09-01", now), UsageLoadError);
});

test("usageTrend pads missing local days and keeps long ranges whole", () => {
  const start = new Date("2026-09-18T00:00:00").getTime() / 1000;
  const end = new Date("2026-09-20T23:59:59").getTime() / 1000;
  const padded = usageTrend([{ date: "2026-09-19", quota: 1, tokens: 2, requests: 3 }], start, end);
  assert.deepEqual(
    padded.map((bucket) => [bucket.date, bucket.quota]),
    [["2026-09-18", 0], ["2026-09-19", 1], ["2026-09-20", 0]],
  );
  const long = Array.from({ length: 400 }, (_, i) => ({ date: `2026-01-${String(i + 1).padStart(2, "0")}`, quota: 1, tokens: 1, requests: 1 }));
  assert.equal(usageTrend(long, 0, Date.now() / 1000).length, 400);
});

test("collectUsage assembles every page until the reported total", async () => {
  const rows = Array.from({ length: 250 }, (_, i) => row({ id: i + 1, created_at: 1_700_000_000 + i }));
  const fetched: number[] = [];
  const records = await collectUsage(async (query) => {
    fetched.push(query.page ?? 1);
    return pageOf(rows, 250)(query);
  }, {});
  assert.equal(records.length, 250);
  assert.equal(records.at(-1)?.id, 250);
  assert.deepEqual(fetched, [1, 2, 3]);
});

test("collectUsage keeps same-second rows with identical ids and never deduplicates", async () => {
  const twin = [row({ id: 7, created_at: 1_700_000_000 }), row({ id: 7, created_at: 1_700_000_000 })];
  const records = await collectUsage(pageOf(twin, 2), {});
  assert.equal(records.length, 2);
  assert.equal(summarizeNewApiLogs(records).requests, 2);
});

test("collectUsage refuses to present a partial ledger as complete", async () => {
  let calls = 0;
  await assert.rejects(
    collectUsage(async () => {
      calls += 1;
      if (calls === 1) return { items: Array.from({ length: 100 }, (_, i) => row({ id: i + 1 })), total: 300 };
      return { items: [], total: 300 };
    }, {}),
    /尚未读全|不一致|无效/,
  );
  let flip = 0;
  await assert.rejects(
    collectUsage(async () => {
      flip += 1;
      return flip === 1
        ? { items: Array.from({ length: 100 }, (_, i) => row({ id: i + 1 })), total: 300 }
        : { items: Array.from({ length: 100 }, (_, i) => row({ id: i + 101 })), total: 250 };
    }, {}),
    /发生变化/,
  );
  await assert.rejects(
    collectUsage(async () => ({ items: [row({ quota: Number.NaN })], total: 1 }), {}),
    /数值无效/,
  );
});

test("collectUsage applies range, group and model-set filters locally for legacy servers", async () => {
  const rows = [
    row({ id: 1, created_at: 100, model_name: "gpt-5", group: "fast" }),
    row({ id: 2, created_at: 100, model_name: "claude-sonnet-4-6", group: "fast" }),
    row({ id: 3, created_at: 500, model_name: "gpt-5", group: "slow" }),
  ];
  const server = pageOf(rows, 3);
  const models = await collectUsage(server, { startTimestamp: 0, endTimestamp: 1000, models: ["gpt-5"] });
  assert.deepEqual(models.map((item) => item.id), [1, 3]);
  const grouped = await collectUsage(server, { startTimestamp: 0, endTimestamp: 1000, group: "slow" });
  assert.deepEqual(grouped.map((item) => item.id), [3]);
  const none = await collectUsage(server, { startTimestamp: 0, endTimestamp: 1000, models: [] });
  assert.equal(none.length, 0);
});

test("collectUsage stops cleanly on legacy payloads without a total", async () => {
  const rows = Array.from({ length: 120 }, (_, i) => row({ id: i + 1 }));
  const records = await collectUsage(pageOf(rows, undefined), {});
  assert.equal(records.length, 120);
});

test("mapNewApiLogs rejects malformed payloads and keeps daily buckets in local time", () => {
  assert.throws(() => mapNewApiLogs({ success: true, data: { something: 1 } }), UsageLoadError);
  // 用本地时间构造 23:30 的记录：无论测试机在哪个时区，都应归入它自己的本地日期
  const ts = new Date(2026, 8, 19, 23, 30, 0).getTime() / 1000;
  const mapped = mapNewApiLogs({ success: true, data: { items: [{ id: 1, created_at: ts, quota: 500000, model_name: "gpt-5" }], total: 1 } });
  assert.equal(mapped.total, 1);
  assert.equal(summarizeNewApiLogs(mapped.items).by_day?.[0]?.date, "2026-09-19");
  assert.equal(localDate(ts), "2026-09-19");
});

test("summarizeNewApiLogs sorts dimensions by spend and days chronologically", () => {
  const summary = summarizeNewApiLogs([
    row({ model_name: "cheap", quota: 1, created_at: 1_700_000_800 }),
    row({ model_name: "pricey", quota: 9, created_at: 1_700_000_100 }),
  ]);
  assert.deepEqual(summary.by_model?.map((item) => item.name), ["pricey", "cheap"]);
  assert.ok((summary.by_day?.[0]?.date ?? "") < (summary.by_day?.[1]?.date ?? "zzzz"));
});

test("usageUnit only accepts positive finite station units", () => {
  assert.equal(usageUnit(500000), 500000);
  assert.equal(usageUnit("500000"), 500000);
  assert.throws(() => usageUnit(undefined), UsageLoadError);
  assert.throws(() => usageUnit(0), UsageLoadError);
  assert.throws(() => usageUnit(-1), UsageLoadError);
  assert.throws(() => usageUnit("abc"), UsageLoadError);
});
