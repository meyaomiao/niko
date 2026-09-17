import assert from "node:assert/strict";
import test from "node:test";

import {
  DOWNLOADS_CACHE_KEY,
  HEARTBEAT_TTL_SECONDS,
  LIVE_PREFIX,
  RATE_LIMIT_PREFIX,
  authorizeOps,
  isInstallerAsset,
  installerPlatform,
  parseHeartbeatBody,
  presenceKey,
  rateLimitKey,
  recordHeartbeat,
  summarizeDownloads,
  summarizeOnline,
} from "../functions/_lib/presence.js";
import { HttpError } from "../functions/_lib/security.js";

function assertHttpError(error, status, code) {
  assert.ok(error instanceof HttpError);
  assert.equal(error.status, status);
  assert.equal(error.code, code);
  return true;
}

class MemoryKv {
  constructor() {
    this.store = new Map();
  }

  async get(key) {
    return this.store.has(key) ? this.store.get(key).value : null;
  }

  async put(key, value, options = {}) {
    this.store.set(key, { value, expirationTtl: options.expirationTtl });
  }

  async list({ prefix = "", cursor, limit = 1000 } = {}) {
    const names = [...this.store.keys()].filter((name) => name.startsWith(prefix)).sort();
    const start = cursor ? Number(cursor) : 0;
    const slice = names.slice(start, start + limit);
    const next = start + slice.length;
    return {
      keys: slice.map((name) => ({ name })),
      list_complete: next >= names.length,
      cursor: next < names.length ? String(next) : "",
    };
  }
}

test("installer assets exclude updater, checksum and signature files", () => {
  assert.equal(isInstallerAsset("Niko_0.2.2_universal.dmg"), true);
  assert.equal(isInstallerAsset("Niko_0.2.2_x64-setup.exe"), true);
  assert.equal(isInstallerAsset("Niko_0.2.2_x64_en-US.msi"), true);
  assert.equal(isInstallerAsset("Niko.app.tar.gz"), false);
  assert.equal(isInstallerAsset("Niko.app.tar.gz.sig"), false);
  assert.equal(isInstallerAsset("latest.json"), false);
  assert.equal(isInstallerAsset("SHA256SUMS.txt"), false);
  assert.equal(isInstallerAsset("Niko_0.2.2_x64-setup.exe.sig"), false);
  assert.equal(installerPlatform("Niko_0.2.2_universal.dmg"), "macos");
  assert.equal(installerPlatform("Niko_0.2.2_x64-setup.exe"), "windows");
});

test("download summary only counts installer files", () => {
  const summary = summarizeDownloads([
    {
      tag_name: "niko-v0.2.2",
      assets: [
        { name: "Niko_0.2.2_universal.dmg", download_count: 4 },
        { name: "Niko_0.2.2_x64-setup.exe", download_count: 3 },
        { name: "Niko_0.2.2_x64_en-US.msi", download_count: 2 },
        { name: "latest.json", download_count: 99 },
        { name: "Niko.app.tar.gz", download_count: 50 },
        { name: "SHA256SUMS.txt", download_count: 8 },
      ],
    },
  ]);
  assert.equal(summary.installers, 9);
  assert.equal(summary.macos, 4);
  assert.equal(summary.windows, 5);
  assert.deepEqual(summary.by_release, [
    { tag: "niko-v0.2.2", installers: 9, macos: 4, windows: 5 },
  ]);
});

test("heartbeat body accepts only the anonymous presence contract", () => {
  const valid = parseHeartbeatBody(
    JSON.stringify({
      install_id: "550e8400-e29b-41d4-a716-446655440000",
      platform: "macos",
      app_version: "0.2.2",
    }),
  );
  assert.equal(valid.install_id, "550e8400-e29b-41d4-a716-446655440000");
  assert.throws(
    () => parseHeartbeatBody(JSON.stringify({ install_id: "not-a-uuid", platform: "macos", app_version: "0.2.2" })),
    (error) => assertHttpError(error, 400, "INVALID_REQUEST"),
  );
  assert.throws(
    () =>
      parseHeartbeatBody(
        JSON.stringify({
          install_id: "550e8400-e29b-41d4-a716-446655440000",
          platform: "macos",
          app_version: "0.2.2",
          username: "alice",
        }),
      ),
    (error) => assertHttpError(error, 400, "INVALID_REQUEST"),
  );
});

test("presence keys hash the install id and never store it raw", async () => {
  const key = await presenceKey("550e8400-e29b-41d4-a716-446655440000");
  assert.match(key, new RegExp(`^${LIVE_PREFIX}[0-9a-f]{32}$`));
  assert.doesNotMatch(key, /550e8400/);
});

test("heartbeat writes a hashed live record and rate-limits the same install", async () => {
  const kv = new MemoryKv();
  const body = JSON.stringify({
    install_id: "550e8400-e29b-41d4-a716-446655440000",
    platform: "windows",
    app_version: "0.2.2",
  });
  await recordHeartbeat(kv, body);
  const liveKeys = [...kv.store.keys()].filter((name) => name.startsWith(LIVE_PREFIX));
  assert.equal(liveKeys.length, 1);
  assert.doesNotMatch(liveKeys[0], /550e8400/);
  const record = JSON.parse(kv.store.get(liveKeys[0]).value);
  assert.equal(record.platform, "windows");
  assert.equal(record.app_version, "0.2.2");
  assert.equal(typeof record.seen_at, "number");
  assert.equal("install_id" in record, false);
  assert.equal(kv.store.get(liveKeys[0]).expirationTtl, HEARTBEAT_TTL_SECONDS);
  assert.equal(
    [...kv.store.keys()].filter((name) => name.startsWith(RATE_LIMIT_PREFIX)).length,
    1,
  );
  await assert.rejects(
    () => recordHeartbeat(kv, body),
    (error) => assertHttpError(error, 429, "RATE_LIMITED"),
  );

  await recordHeartbeat(
    kv,
    JSON.stringify({
      install_id: "7c9e6679-7425-40de-944b-e07fc1f90ae7",
      platform: "macos",
      app_version: "0.2.2",
    }),
  );
  assert.equal(
    [...kv.store.keys()].filter((name) => name.startsWith(LIVE_PREFIX)).length,
    2,
  );
});

test("online summary counts recent devices by platform and version", () => {
  const online = summarizeOnline([
    { platform: "macos", app_version: "0.2.2" },
    { platform: "macos", app_version: "0.2.1" },
    { platform: "windows", app_version: "0.2.2" },
    { platform: "linux", app_version: "0.2.2" },
    { platform: "unknown", app_version: "0.2.2" },
  ]);
  assert.equal(online.total, 4);
  assert.equal(online.macos, 2);
  assert.equal(online.windows, 1);
  assert.equal(online.linux, 1);
  assert.deepEqual(online.by_version, { "0.2.2": 3, "0.2.1": 1 });
});

test("ops stats require a configured secret", () => {
  const request = new Request("https://niko-ai.cc/api/presence/stats", {
    headers: { Authorization: "Bearer niko-ops-secret-16" },
  });
  assert.throws(
    () => authorizeOps(request, {}),
    (error) => assertHttpError(error, 503, "OPS_NOT_CONFIGURED"),
  );
  assert.throws(
    () => authorizeOps(request, { NIKO_OPS_SECRET: "niko-ops-secret-16x" }),
    (error) => assertHttpError(error, 401, "OPS_UNAUTHORIZED"),
  );
  assert.doesNotThrow(() =>
    authorizeOps(request, { NIKO_OPS_SECRET: "niko-ops-secret-16" }),
  );
});

test("rate-limit keys stay scoped to the hashed install id", () => {
  assert.equal(rateLimitKey("abc"), `${RATE_LIMIT_PREFIX}abc`);
  assert.equal(DOWNLOADS_CACHE_KEY, "cache:github-downloads");
});
