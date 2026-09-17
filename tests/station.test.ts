import assert from "node:assert/strict";
import test from "node:test";
import {
  isNewApiAuth,
  MOMOTOKEN_ORIGIN,
  normalizeStationOrigin,
  parseOptionalUserId,
  relayBaseUrl,
  stationOrigin,
} from "../src/lib/station.ts";

test("normalizes station origins and strips trailing api suffixes", () => {
  assert.equal(normalizeStationOrigin("https://relay.example.com/"), "https://relay.example.com");
  assert.equal(normalizeStationOrigin("relay.example.com/v1"), "https://relay.example.com");
  assert.equal(normalizeStationOrigin("http://192.168.1.8:3000/api"), "http://192.168.1.8:3000");
  assert.equal(
    normalizeStationOrigin("https://relay.example.com/new-api/"),
    "https://relay.example.com/new-api",
  );
});

test("rejects unsafe or empty station origins", () => {
  assert.throws(() => normalizeStationOrigin(""), /请填写站点地址/);
  assert.throws(() => normalizeStationOrigin("javascript:alert(1)"), /站点地址无效/);
  assert.throws(() => normalizeStationOrigin("https://user:pass@example.com"), /站点地址无效/);
  assert.throws(() => normalizeStationOrigin("https://169.254.169.254"), /站点地址无效/);
});

test("parses optional user ids", () => {
  assert.equal(parseOptionalUserId(""), undefined);
  assert.equal(parseOptionalUserId(" 10085 "), 10085);
  assert.throws(() => parseOptionalUserId("0"), /正整数/);
  assert.throws(() => parseOptionalUserId("abc"), /正整数/);
});

test("routes relay urls from auth kind", () => {
  assert.equal(stationOrigin(null), MOMOTOKEN_ORIGIN);
  assert.equal(relayBaseUrl({ kind: "momotoken" }), `${MOMOTOKEN_ORIGIN}/v1`);
  assert.equal(
    relayBaseUrl({ kind: "newapi", origin: "https://relay.example.com" }),
    "https://relay.example.com/v1",
  );
  assert.equal(isNewApiAuth({ kind: "newapi" }), false);
  assert.equal(isNewApiAuth({ kind: "newapi", origin: "https://relay.example.com" }), true);
});
