import assert from "node:assert/strict";
import test from "node:test";
import { epayTradeNo, isTopupPaid } from "../src/lib/topup.ts";
import type { TopUpRecord } from "../src/api/client.ts";

const record = (trade_no: string, status: string): TopUpRecord => ({
  id: 1, amount: 10, money: 10, trade_no, status,
  payment_method: "alipay", create_time: 1, complete_time: 0,
});

test("epay merchant order identity must be present before checkout", () => {
  assert.equal(epayTradeNo({ url: "https://cashier.invalid", params: { out_trade_no: "order-1" } }), "order-1");
  for (const value of [undefined, null, "", "  ", 123, {}]) {
    assert.equal(epayTradeNo({ url: "https://cashier.invalid", params: { out_trade_no: value } }), null);
  }
});

test("only a successful record for the exact checkout confirms payment", () => {
  assert.equal(isTopupPaid("order-1", [record("order-1", "pending")]), false);
  assert.equal(isTopupPaid("order-1", [record("older-order", "success")]), false);
  assert.equal(isTopupPaid("order-1", [record("order-1", "failed")]), false);
  assert.equal(isTopupPaid("order-1", [record("older-order", "success"), record("order-1", "success")]), true);
  assert.equal(isTopupPaid(null, [record("order-1", "success")]), false);
  assert.equal(isTopupPaid("order-1", null), false);
});
