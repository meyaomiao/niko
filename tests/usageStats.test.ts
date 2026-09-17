import assert from "node:assert/strict";
import test from "node:test";

import { usageSummary, type UsageStats } from "../src/lib/usageStats.ts";

const sample: UsageStats = {
  online: {
    total: 4,
    macos: 2,
    windows: 1,
    linux: 1,
    by_version: { "0.2.1": 1, "0.2.2": 3 },
  },
  downloads: {
    installers: 9,
    macos: 4,
    windows: 5,
    by_release: [{ tag: "niko-v0.2.2", installers: 9, macos: 4, windows: 5 }],
  },
  generated_at: "2026-09-16T08:00:00.000Z",
  window_seconds: 180,
};

test("usage summary keeps installer downloads separate from live devices", () => {
  const summary = usageSummary(sample);
  assert.equal(summary.onlineTotal, 4);
  assert.equal(summary.downloadTotal, 9);
  assert.equal(summary.windowMinutes, 3);
  assert.equal(summary.onlineBreakdown, "macOS 2 · Windows 1 · Linux 1");
  assert.match(summary.downloadBreakdown, /不含自动更新文件/);
  assert.deepEqual(summary.versions, ["v0.2.2 3 台", "v0.2.1 1 台"]);
  assert.equal(summary.releases[0]?.tag, "niko-v0.2.2");
});
