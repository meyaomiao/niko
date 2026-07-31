import assert from "node:assert/strict";
import test from "node:test";
import { acceptsResponse, beginRequest, boundSessionQuery, initialRequestGuard, mountRequests, parseSafeCommandError, safeFailure, unmountRequests } from "../src/lib/codexSessions.ts";

test("accepts only the versioned bounded safe error contract", () => {
  const safe = { version: 1, code: "busy", message: "另一个操作正在进行，请稍后再试。", retryable: true, action: "retry" };
  assert.deepEqual(parseSafeCommandError(safe), safe);
  for (const rejection of [
    new Error("/Users/alice/.codex/config.toml"),
    "auth.json journal WAL SQLite custom lock sk-key API token",
    { version: 2, code: "x", message: "x", retryable: true },
    { version: 1, code: "BAD CODE", message: "x", retryable: true },
    { version: 1, code: "busy", message: "/Users/alice/auth.json token", retryable: true },
    { version: 1, code: "busy", message: "另一个操作正在进行，请稍后再试。", retryable: false },
    { version: 1, code: "change_failed", message: "操作未完成，原有内容保持可用。", retryable: true, action: "again" },
  ]) {
    const result = safeFailure(rejection);
    assert.equal(result.retryable, false);
    assert.equal(result.message, "操作失败，请稍后再试。");
    assert.doesNotMatch(JSON.stringify(result), /Users|config\.toml|auth\.json|journal|WAL|SQLite|custom|lock|API|token/i);
  }
});

test("late scan, unmount, and repeated action responses are rejected", () => {
  let guard = initialRequestGuard();
  const firstScan = beginRequest(guard, "scan"); guard = firstScan.state;
  const secondScan = beginRequest(guard, "scan"); guard = secondScan.state;
  assert.equal(acceptsResponse(guard, "scan", firstScan.generation), false);
  assert.equal(acceptsResponse(guard, "scan", secondScan.generation), true);
  const firstAction = beginRequest(guard, "action"); guard = firstAction.state;
  const secondAction = beginRequest(guard, "action"); guard = secondAction.state;
  assert.equal(acceptsResponse(guard, "action", firstAction.generation), false);
  guard = unmountRequests(guard);
  assert.equal(acceptsResponse(guard, "action", secondAction.generation), false);
  guard = mountRequests(guard);
  assert.equal(acceptsResponse(guard, "action", secondAction.generation), true);
});

test("search input is bounded before crossing the command boundary", () => {
  assert.equal(boundSessionQuery(`  ${"a".repeat(200)}  `).length, 80);
});
