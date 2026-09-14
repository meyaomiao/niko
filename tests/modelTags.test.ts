import assert from "node:assert/strict";
import test from "node:test";
import {
  computeTags,
  isNewRelease,
  vendorPriceLevels,
  vendorUsageRanks,
} from "../src/lib/modelTags.ts";
import type { UsageSummary } from "../src/api/client.ts";

const DAY = 86400_000;

function summary(entries: { name: string; requests: number }[]): UsageSummary {
  return {
    quota: 0,
    prompt_tokens: 0,
    completion_tokens: 0,
    requests: 0,
    stream_requests: 0,
    total_use_time: 0,
    by_model: entries.map((e) => ({
      name: e.name,
      quota: 0,
      prompt_tokens: 0,
      completion_tokens: 0,
      requests: e.requests,
    })),
    by_group: null,
    by_day: null,
  };
}

test("刚上新窗口是两周", () => {
  const now = Date.UTC(2026, 8, 14);
  assert.equal(isNewRelease("2026-09-01", now), true); // 13 天
  assert.equal(isNewRelease("2026-08-31", now), true); // 14 天边界
  assert.equal(isNewRelease("2026-08-30", now), false); // 15 天
  assert.equal(isNewRelease(undefined, now), false);
  assert.equal(isNewRelease("not-a-date", now), false);
});

test("用量名次在每个厂家内独立计算", () => {
  const ranks = vendorUsageRanks(
    summary([
      { name: "claude-opus-5", requests: 10 },
      { name: "claude-sonnet-5", requests: 50 },
      { name: "gpt-5", requests: 5 },
      { name: "gpt-6", requests: 40 },
    ])
  );
  // Anthropic 内 sonnet(50) > opus(10)
  assert.equal(ranks.get("claude-sonnet-5"), 1);
  assert.equal(ranks.get("claude-opus-5"), 2);
  // OpenAI 内 gpt-6(40) > gpt-5(5)
  assert.equal(ranks.get("gpt-6"), 1);
  assert.equal(ranks.get("gpt-5"), 2);
});

test("价格分位按厂家内比较", () => {
  const prices: Record<string, number> = {
    "claude-opus-5": 10,
    "claude-sonnet-5": 2,
    "gpt-6": 100,
    "gpt-5": 50,
    "gpt-5-mini": 25,
  };
  const levels = vendorPriceLevels(Object.keys(prices), (name) => prices[name]);
  // Anthropic 内最便宜的是 sonnet，分位 0；最贵是 opus，分位 1
  assert.equal(levels.get("claude-sonnet-5"), 0);
  assert.equal(levels.get("claude-opus-5"), 1);
  // OpenAI 内 mini(25)=0、gpt-5(50)=1/3、gpt-6(100)=1
  assert.equal(levels.get("gpt-5-mini"), 0);
  assert.equal(levels.get("gpt-5"), 1 / 3);
  assert.equal(levels.get("gpt-6"), 1);
});

test("标签门槛：常用前3，性价比需前5且价格分位低于1/5", () => {
  const now = Date.UTC(2026, 8, 14);
  // 常用：名次 3 命中
  assert.deepEqual(
    computeTags({ name: "m", rank: 3, now }).map((t) => t.id),
    ["hot"]
  );
  assert.deepEqual(computeTags({ name: "m", rank: 4, now }).map((t) => t.id), []);
  // 性价比：名次 5 且分位 0.1 命中
  assert.deepEqual(
    computeTags({ name: "m", rank: 5, level: 0.1, now }).map((t) => t.id),
    ["value"]
  );
  // 价格分位不够便宜 → 不出性价比
  assert.deepEqual(computeTags({ name: "m", rank: 5, level: 0.2, now }).map((t) => t.id), []);
  // 名次超出前 5 → 不出性价比
  assert.deepEqual(computeTags({ name: "m", rank: 6, level: 0.05, now }).map((t) => t.id), []);
  // 无用量数据 → 不出常用/性价比
  assert.deepEqual(computeTags({ name: "m", level: 0.05, now }).map((t) => t.id), []);
  // 刚上新可与其它标签叠加
  assert.deepEqual(
    computeTags({ name: "m", rank: 1, level: 0.05, release: "2026-09-10", now }).map((t) => t.id),
    ["new", "hot", "value"]
  );
});

test("按次计费或无价格不参与分位", () => {
  const levels = vendorPriceLevels(["gpt-6", "gpt-5"], (name) =>
    name === "gpt-6" ? undefined : 50
  );
  assert.equal(levels.has("gpt-6"), false);
  assert.equal(levels.get("gpt-5"), 0);
});
