import assert from "node:assert/strict";
import test from "node:test";
import { restoreConfirmationMatches, runTargetActions, waitForDesktopAction } from "../src/lib/homeActions.ts";

test("恢复确认只适用于本次确认的目标且在五秒内有效", () => {
  const confirmation = { targetIds: ["codex"], expiresAt: 6000 };
  assert.equal(restoreConfirmationMatches(confirmation, ["codex"], 5999), true);
  assert.equal(restoreConfirmationMatches(confirmation, ["claude-desktop"], 2000), false);
  assert.equal(restoreConfirmationMatches(confirmation, ["codex"], 6000), false);
  assert.equal(restoreConfirmationMatches(null, ["codex"], 2000), false);
});

test("全部应用恢复不能沿用不同应用集合的确认", () => {
  const confirmation = { targetIds: ["codex", "claude-desktop"], expiresAt: 6000 };
  assert.equal(restoreConfirmationMatches(confirmation, ["claude-desktop", "codex"], 2000), true);
  assert.equal(restoreConfirmationMatches(confirmation, ["codex"], 2000), false);
  assert.equal(restoreConfirmationMatches(confirmation, ["codex", "dsh"], 2000), false);
  assert.equal(restoreConfirmationMatches(confirmation, [], 2000), false);
});

test("原生写入超过提示时间仍等待实际完成，不允许误判失败后重复写入", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let resolve!: (value: string) => void;
  let slow = 0;
  let settled = false;
  const operation = new Promise<string>((done) => { resolve = done; });
  const result = waitForDesktopAction(operation, () => { slow += 1; });
  void result.then(() => { settled = true; });
  t.mock.timers.tick(30_000);
  await Promise.resolve();
  assert.equal(slow, 1);
  assert.equal(settled, false);
  resolve("completed");
  assert.equal(await result, "completed");
});

test("原生写入结束或失败后移除延迟提示，保留真实失败原因", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let slow = 0;
  assert.equal(await waitForDesktopAction(Promise.resolve("done"), () => { slow += 1; }), "done");
  const failure = new Error("fixture write failed");
  await assert.rejects(waitForDesktopAction(Promise.reject(failure), () => { slow += 1; }), failure);
  t.mock.timers.tick(30_000);
  assert.equal(slow, 0);
});

test("批量恢复中途失败后继续处理并记录各应用的真实结果", async () => {
  const calls: string[] = [];
  const failure = new Error("unavailable");
  const result = await runTargetActions(["codex", "claude-desktop", "dsh"], async (id) => {
    calls.push(id);
    if (id === "claude-desktop") throw failure;
    return id === "codex" ? ["updated"] : [];
  });
  assert.deepEqual(calls, ["codex", "claude-desktop", "dsh"]);
  assert.deepEqual(result, [
    { id: "codex", ok: true, value: ["updated"] },
    { id: "claude-desktop", ok: false, error: failure },
    { id: "dsh", ok: true, value: [] },
  ]);
});
