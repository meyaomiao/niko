import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  accumulateDownloadProgress,
  describeCheckResult,
  downloadPercent,
  formatDownloadProgress,
  friendlyUpdateError,
  installAvailableUpdate,
  isUpdaterPlatformKey,
  normalizeUpdaterPlatforms,
  rewriteUpdaterManifest,
  type DownloadProgressEvent,
  type UpdaterHandle,
} from "../src/lib/appUpdate.ts";

const universal = {
  url: "https://github.com/meyaomiao/niko/releases/download/niko-v0.2.3/Niko.app.tar.gz",
  signature: "mac-sig",
};

const windows = {
  url: "https://github.com/meyaomiao/niko/releases/download/niko-v0.2.3/Niko_0.2.3_x64-setup.exe",
  signature: "win-sig",
};

test("updater endpoints prefer the website rewrite, then GitHub", () => {
  const config = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  assert.deepEqual(config.plugins.updater.endpoints, [
    "https://niko-ai.cc/latest.json",
    "https://github.com/meyaomiao/niko/releases/latest/download/latest.json",
  ]);
});

test("updater platform keys follow {os}-{arch}, not darwin-universal", () => {
  assert.equal(isUpdaterPlatformKey("darwin-aarch64"), true);
  assert.equal(isUpdaterPlatformKey("darwin-x86_64"), true);
  assert.equal(isUpdaterPlatformKey("windows-x86_64"), true);
  assert.equal(isUpdaterPlatformKey("darwin-universal"), false);
});

test("rewrites darwin-universal onto both macOS arch keys", () => {
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

test("keeps explicit arch keys when rewriting a universal fallback", () => {
  const appleSilicon = { ...universal, signature: "arm-sig" };
  const rewritten = normalizeUpdaterPlatforms({
    "darwin-universal": universal,
    "darwin-aarch64": appleSilicon,
    "windows-x86_64": windows,
  });
  assert.equal(rewritten["darwin-universal"], undefined);
  assert.equal(rewritten["darwin-aarch64"].signature, "arm-sig");
  assert.equal(rewritten["darwin-x86_64"].signature, "mac-sig");
});

test("rejects a manifest the updater cannot consume", () => {
  assert.throws(() => rewriteUpdaterManifest({ version: "0.2.3", platforms: {} }));
  assert.throws(() => rewriteUpdaterManifest({ version: "bad", platforms: { "windows-x86_64": windows } }));
});

test("download progress stays determinate when the total length is known", () => {
  let progress = accumulateDownloadProgress({ received: 0 }, {
    event: "Started",
    data: { contentLength: 1000 },
  });
  progress = accumulateDownloadProgress(progress, {
    event: "Progress",
    data: { chunkLength: 250 },
  });
  progress = accumulateDownloadProgress(progress, {
    event: "Progress",
    data: { chunkLength: 250 },
  });
  assert.equal(downloadPercent(progress), 50);
  assert.equal(formatDownloadProgress(progress.received, progress.total), "正在下载新版本 50%");
});

test("download progress falls back to size when total length is unknown", () => {
  const started = accumulateDownloadProgress({ received: 0 }, { event: "Started", data: {} });
  const progressed = accumulateDownloadProgress(started, {
    event: "Progress",
    data: { chunkLength: 2 * 1024 * 1024 },
  });
  assert.equal(downloadPercent(progressed), null);
  assert.equal(formatDownloadProgress(progressed.received, progressed.total), "正在下载新版本 2.0 MB");
});

test("check result distinguishes current from available", () => {
  assert.deepEqual(describeCheckResult(null), { kind: "current" });
  assert.deepEqual(describeCheckResult({ version: "0.2.4", body: "fix" }), {
    kind: "available",
    version: "0.2.4",
    notes: "fix",
  });
});

test("update errors stay user-facing and do not mention updater internals", () => {
  assert.equal(
    friendlyUpdateError("Could not fetch a valid release JSON from the remote"),
    "暂时无法读取更新信息，请稍后重试。",
  );
  assert.equal(
    friendlyUpdateError("platform `darwin-aarch64` not found in latest.json"),
    "当前系统还没有匹配的更新包，请稍后重试或到官网下载。",
  );
  assert.equal(
    friendlyUpdateError("signature verification failed"),
    "更新包校验失败，没有安装。请稍后重试或到官网重新下载。",
  );
  assert.equal(
    friendlyUpdateError("network timeout"),
    "检查更新需要联网，请确认网络后重试。",
  );
  assert.doesNotMatch(friendlyUpdateError("minisign UnexpectedKeyId"), /minisign|json|darwin/i);
});

test("installAvailableUpdate reports current when check returns null", async () => {
  const statuses: string[] = [];
  const outcome = await installAvailableUpdate(
    {
      check: async () => null,
      relaunch: async () => {
        throw new Error("should not relaunch");
      },
    },
    (status) => statuses.push(status),
    () => {
      throw new Error("should not report progress");
    },
  );
  assert.equal(outcome, "current");
  assert.deepEqual(statuses, ["正在检查新版本…", "已是最新版本"]);
});

test("installAvailableUpdate downloads, installs, then relaunches", async () => {
  const statuses: string[] = [];
  const percents: Array<number | null> = [];
  let relaunched = false;
  const events: DownloadProgressEvent[] = [
    { event: "Started", data: { contentLength: 100 } },
    { event: "Progress", data: { chunkLength: 100 } },
    { event: "Finished" },
  ];
  const handle: UpdaterHandle = {
    version: "0.2.4",
    downloadAndInstall: async (onEvent) => {
      for (const event of events) onEvent?.(event);
    },
  };
  const outcome = await installAvailableUpdate(
    {
      check: async () => handle,
      relaunch: async () => {
        relaunched = true;
      },
    },
    (status) => statuses.push(status),
    (progress) => percents.push(downloadPercent(progress)),
  );
  assert.equal(outcome, "installed");
  assert.equal(relaunched, true);
  assert.equal(statuses[0], "正在检查新版本…");
  assert.equal(statuses[1], "发现新版本 0.2.4，正在下载…");
  assert.ok(statuses.includes("正在安装新版本…"));
  assert.equal(statuses.at(-1), "安装完成，即将重启…");
  assert.ok(percents.includes(99) || percents.includes(100) || percents.includes(0));
});

test("installAvailableUpdate asks the user to restart if relaunch fails", async () => {
  const statuses: string[] = [];
  const handle: UpdaterHandle = {
    version: "0.2.4",
    downloadAndInstall: async (onEvent) => {
      onEvent?.({ event: "Finished" });
    },
  };
  const outcome = await installAvailableUpdate(
    {
      check: async () => handle,
      relaunch: async () => {
        throw new Error("restart denied");
      },
    },
    (status) => statuses.push(status),
    () => {},
  );
  assert.equal(outcome, "installed");
  assert.equal(statuses.at(-1), "安装完成，请手动重启应用以使用新版本。");
});
