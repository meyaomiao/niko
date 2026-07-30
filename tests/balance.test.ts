import assert from "node:assert/strict";
import test from "node:test";
import {
  balanceReducer,
  formatBalanceUSD,
  formatBalanceUpdatedAt,
  parseBalanceSnapshot,
  type BalanceState,
} from "../src/lib/balance.ts";

test("parses numeric API fields and uses the server quota unit", () => {
  const snapshot = parseBalanceSnapshot("5000000", "500000", 1_000);

  assert.deepEqual(snapshot, {
    quota: 5_000_000,
    quotaPerUnit: 500_000,
    updatedAt: 1_000,
  });
  assert.equal(formatBalanceUSD(snapshot), "$10.00");
});

test("rounds display amounts to two decimals without floating-point drift", () => {
  assert.equal(formatBalanceUSD(parseBalanceSnapshot(2_500, 500_000, 1_000)), "$0.01");
  assert.equal(formatBalanceUSD(parseBalanceSnapshot(-2_500, 500_000, 1_000)), "-$0.01");
});

test("rejects invalid quota fields instead of displaying a default zero", () => {
  assert.equal(parseBalanceSnapshot("not-a-number", 500_000), null);
  assert.equal(parseBalanceSnapshot(10, 0), null);
  assert.equal(formatBalanceUSD(null), "—");
});

test("shows a useful last-updated label", () => {
  const snapshot = parseBalanceSnapshot(500_000, 500_000, 1_000)!;

  assert.match(formatBalanceUpdatedAt(snapshot), /更新$/);
});

test("refresh failure preserves the last valid balance", () => {
  const snapshot = parseBalanceSnapshot(5_000_000, 500_000, 1_000)!;
  const initial: BalanceState = { snapshot, refreshing: false, error: "" };
  const refreshing = balanceReducer(initial, { type: "refresh-started" });
  const failed = balanceReducer(refreshing, {
    type: "refresh-failed",
    error: "余额刷新失败，请稍后重试",
  });

  assert.equal(refreshing.snapshot, snapshot);
  assert.equal(refreshing.refreshing, true);
  assert.equal(failed.snapshot, snapshot);
  assert.equal(failed.refreshing, false);
  assert.equal(failed.error, "余额刷新失败，请稍后重试");
});

test("refresh success replaces the balance and clears stale errors", () => {
  const oldSnapshot = parseBalanceSnapshot(5_000_000, 500_000, 1_000)!;
  const nextSnapshot = parseBalanceSnapshot(7_500_000, 500_000, 2_000)!;
  const state = balanceReducer(
    { snapshot: oldSnapshot, refreshing: true, error: "旧错误" },
    { type: "refresh-succeeded", snapshot: nextSnapshot },
  );

  assert.equal(formatBalanceUSD(state.snapshot), "$15.00");
  assert.equal(state.refreshing, false);
  assert.equal(state.error, "");
});
