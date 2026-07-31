import assert from "node:assert/strict";
import test from "node:test";
import { fmtUSD, priceOf } from "../src/lib/pricing.ts";

test("calculates token prices with the selected service multiplier", () => {
  const price = priceOf(
    {
      model_name: "model",
      quota_type: 0,
      model_ratio: 1.25,
      model_price: 0,
      completion_ratio: 2,
      cache_ratio: 0.5,
      create_cache_ratio: 0.75,
    },
    1.5,
  );

  assert.deepEqual(price, {
    perRequest: false,
    input: 3.75,
    output: 7.5,
    cache: 1.875,
    createCache: 2.8125,
  });
  assert.equal(fmtUSD(price!.input), "$3.75");
  assert.equal(fmtUSD(0), "免费");
});

test("keeps per-request prices separate from token prices", () => {
  const price = priceOf(
    {
      model_name: "image",
      quota_type: 1,
      model_ratio: 0,
      model_price: 0.4,
      completion_ratio: 0,
    },
    1.5,
  );

  assert.equal(price?.perRequest, true);
  assert.equal(price?.input, 0.4 * 1.5);
  assert.equal(fmtUSD(price?.input ?? Number.NaN), "$0.60");
});

test("preserves tiny positive token and per-request prices", () => {
  const tokenPrice = priceOf(
    {
      model_name: "tiny-token",
      quota_type: 0,
      model_ratio: 1e-13,
      model_price: 0,
      completion_ratio: 0.5,
      cache_ratio: 0.25,
      create_cache_ratio: 0.75,
    },
    1,
  );
  const requestPrice = priceOf(
    {
      model_name: "tiny-request",
      quota_type: 1,
      model_ratio: 0,
      model_price: 1e-7,
      completion_ratio: 0,
    },
    1,
  );

  assert.deepEqual(tokenPrice, {
    perRequest: false,
    input: 2e-13,
    output: 1e-13,
    cache: 2e-13 * 0.25,
    createCache: 2e-13 * 0.75,
  });
  assert.equal(fmtUSD(tokenPrice!.input), "<$0.000001");
  assert.equal(fmtUSD(tokenPrice!.output), "<$0.000001");
  assert.equal(fmtUSD(tokenPrice!.cache!), "<$0.000001");
  assert.equal(fmtUSD(tokenPrice!.createCache!), "<$0.000001");
  assert.equal(requestPrice?.input, 1e-7);
  assert.equal(fmtUSD(requestPrice!.input), "<$0.000001");
});

test("keeps display rounding from changing price comparison", () => {
  const lower = priceOf(
    {
      model_name: "lower",
      quota_type: 0,
      model_ratio: 1e-13,
      model_price: 0,
      completion_ratio: 1,
    },
    1,
  )!.input;
  const higher = priceOf(
    {
      model_name: "higher",
      quota_type: 0,
      model_ratio: 5e-7,
      model_price: 0,
      completion_ratio: 1,
    },
    1,
  )!.input;

  assert.equal(typeof lower, "number");
  assert.equal(typeof higher, "number");
  assert.deepEqual([higher, lower].sort((a, b) => a - b), [lower, higher]);
  assert.equal(fmtUSD(lower), "<$0.000001");
  assert.equal(fmtUSD(higher), "$0.000001");
});

test("uses the smallest readable unit at the display boundary", () => {
  assert.equal(fmtUSD(0), "免费");
  assert.equal(fmtUSD(0.000000999999), "<$0.000001");
  assert.equal(fmtUSD(0.000001), "$0.000001");
  assert.equal(fmtUSD(0.0000015), "$0.000002");
  assert.equal(fmtUSD(0.009999999), "$0.010000");
  assert.equal(fmtUSD(0.01), "$0.01");
  assert.equal(fmtUSD(-1), "价格暂不可用");
  assert.equal(fmtUSD(Number.POSITIVE_INFINITY), "价格暂不可用");
});

test("keeps the existing default output ratio when the backend sends zero", () => {
  const price = priceOf(
    {
      model_name: "model",
      quota_type: 0,
      model_ratio: 1,
      model_price: 0,
      completion_ratio: 0,
    },
    1,
  );

  assert.deepEqual(price, { perRequest: false, input: 2, output: 2 });
});

test("does not turn missing or invalid price data into a free price", () => {
  assert.equal(priceOf(undefined, 1), null);
  assert.equal(
    priceOf(
      {
        model_name: "missing",
        quota_type: 0,
        model_ratio: Number.NaN,
        model_price: 0,
        completion_ratio: 1,
      },
      1,
    ),
    null,
  );
  assert.equal(
    priceOf(
      {
        model_name: "invalid",
        quota_type: 0,
        model_ratio: 1,
        model_price: 0,
        completion_ratio: 1,
        cache_ratio: Number.NaN,
      },
      0,
    ),
    null,
  );
  assert.equal(fmtUSD(Number.NaN), "价格暂不可用");
  assert.equal(
    priceOf(
      {
        model_name: "negative-request",
        quota_type: 1,
        model_ratio: 0,
        model_price: -1,
        completion_ratio: 1,
      },
      1,
    ),
    null,
  );
  assert.equal(
    priceOf(
      {
        model_name: "negative-token",
        quota_type: 0,
        model_ratio: -1,
        model_price: 0,
        completion_ratio: 1,
      },
      1,
    ),
    null,
  );
});
