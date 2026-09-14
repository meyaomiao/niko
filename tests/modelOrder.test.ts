import assert from "node:assert/strict";
import test from "node:test";
import { compareModelsByRelease, sortModelNames } from "../src/lib/modelOrder.ts";

const ctx = (dates: Record<string, string>, order: string[] = []) => ({
  dateOf: (name: string) => {
    const raw = dates[name];
    return raw ? Date.parse(raw) : null;
  },
  orderOf: (name: string) => {
    const index = order.indexOf(name);
    return index >= 0 ? index : undefined;
  },
});

test("有日期时按发布时间从新到旧", () => {
  const names = ["old", "newest", "middle"];
  const order = ctx({
    old: "2025-01-01",
    middle: "2026-01-01",
    newest: "2026-09-01",
  });
  assert.deepEqual(sortModelNames(names, order), ["newest", "middle", "old"]);
});

test("没有日期的排在有日期之后（不是按名称插进去）", () => {
  // aaa 没有日期，按名称排序会排到最前；这里必须排在 dated 之后
  const ctxA = ctx({ dated: "2026-05-01" });
  assert.deepEqual(sortModelNames(["aaa", "dated"], ctxA), ["dated", "aaa"]);
});

test("同一天或都无日期时，退到服务端发布顺序", () => {
  const c = ctx({}, ["first", "second", "third"]);
  assert.deepEqual(sortModelNames(["third", "first", "second"], c), [
    "first",
    "second",
    "third",
  ]);
});

test("有日期与无日期混排：有日期的整体在前，无日期内部按发布序", () => {
  const c = ctx({ dated: "2026-01-01" }, ["a", "b"]);
  assert.deepEqual(sortModelNames(["b", "a", "dated"], c), ["dated", "a", "b"]);
});

test("既无日期也无发布序时按名称，保证稳定", () => {
  const c = ctx({});
  assert.deepEqual(sortModelNames(["c", "a", "b"], c), ["a", "b", "c"]);
});

test("比较函数可直接用于 sort，且不修改原数组", () => {
  const c = ctx({ x: "2026-02-01", y: "2026-01-01" });
  const names = ["y", "x"];
  const sorted = sortModelNames(names, c);
  assert.deepEqual(sorted, ["x", "y"]);
  assert.deepEqual(names, ["y", "x"]);
  assert.ok(compareModelsByRelease("x", "y", c) < 0);
  assert.equal(compareModelsByRelease("x", "x", c), 0);
});
