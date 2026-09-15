import assert from "node:assert/strict";
import test from "node:test";
import { DEFAULT_QUOTA_PER_UNIT, fmtQuotaUSD, quotaPerUnitOf } from "../src/lib/pricing.ts";

test("额度换算必须使用服务端下发的单位", () => {
  // 站内单位 500000 额度 = $1：面板显示 $2.00，客户端也必须显示 $2.00
  assert.equal(fmtQuotaUSD(1_000_000, 500_000), "$2.00");
  // 单位缺失或非法时回退默认值，而不是按 100 万硬算
  assert.equal(quotaPerUnitOf(undefined), DEFAULT_QUOTA_PER_UNIT);
  assert.equal(quotaPerUnitOf(0), DEFAULT_QUOTA_PER_UNIT);
  assert.equal(quotaPerUnitOf("abc"), DEFAULT_QUOTA_PER_UNIT);
  // 字符串数字要能解析（服务端可能下发字符串）
  assert.equal(quotaPerUnitOf("500000"), 500_000);
  assert.equal(fmtQuotaUSD(500_000, "500000"), "$1.00");
  // 小额不丢精度
  assert.equal(fmtQuotaUSD(6, 500_000), "$0.000012");
  assert.equal(fmtQuotaUSD(0, 500_000), "免费");
});
