import assert from "node:assert/strict";
import test from "node:test";

import { HttpError } from "../functions/_lib/security.js";
import {
  GITHUB_LATEST_JSON,
  fetchLatestUpdaterManifest,
  normalizeUpdaterPlatforms,
  rewriteUpdaterManifest,
  updaterJsonResponse,
} from "../functions/_lib/updater.js";

const universal = {
  url: "https://github.com/meyaomiao/niko/releases/download/niko-v0.2.3/Niko.app.tar.gz",
  signature: "mac-sig",
};

const windows = {
  url: "https://github.com/meyaomiao/niko/releases/download/niko-v0.2.3/Niko_0.2.3_x64-setup.exe",
  signature: "win-sig",
};

test("website updater rewrite maps darwin-universal to both macOS arch keys", () => {
  const rewritten = rewriteUpdaterManifest({
    version: "0.2.3",
    notes: "notes",
    pub_date: "2026-09-17T11:47:28Z",
    platforms: {
      "darwin-universal": universal,
      "windows-x86_64": windows,
    },
  });
  assert.equal(rewritten.platforms["darwin-universal"], undefined);
  assert.deepEqual(rewritten.platforms["darwin-aarch64"], universal);
  assert.deepEqual(rewritten.platforms["darwin-x86_64"], universal);
  assert.deepEqual(rewritten.platforms["windows-x86_64"], windows);
});

test("website updater rewrite keeps an explicit arch key", () => {
  const rewritten = normalizeUpdaterPlatforms({
    "darwin-universal": universal,
    "darwin-aarch64": { ...universal, signature: "arm-sig" },
  });
  assert.equal(rewritten["darwin-aarch64"].signature, "arm-sig");
  assert.equal(rewritten["darwin-x86_64"].signature, "mac-sig");
});

test("website updater rewrite rejects an empty manifest", () => {
  assert.throws(
    () => rewriteUpdaterManifest({ version: "0.2.3", platforms: {} }),
    (error) => error instanceof HttpError && error.status === 502,
  );
});

test("fetchLatestUpdaterManifest follows GitHub and rewrites platforms", async () => {
  const manifest = await fetchLatestUpdaterManifest(async (url, init) => {
    assert.equal(url, GITHUB_LATEST_JSON);
    assert.equal(init.redirect, "follow");
    return {
      ok: true,
      json: async () => ({
        version: "0.2.3",
        platforms: {
          "darwin-universal": universal,
          "windows-x86_64": windows,
        },
      }),
    };
  });
  assert.equal(manifest.platforms["darwin-universal"], undefined);
  assert.ok(manifest.platforms["darwin-aarch64"]);
  assert.ok(manifest.platforms["darwin-x86_64"]);
});

test("updater JSON response does not use no-store", () => {
  const response = updaterJsonResponse({ version: "0.2.3", platforms: { "windows-x86_64": windows } });
  assert.doesNotMatch(response.headers.get("Cache-Control"), /no-store/);
});
