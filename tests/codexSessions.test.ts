import assert from "node:assert/strict";
import test from "node:test";
import {
  acceptsResponse,
  beginRequest,
  boundSessionQuery,
  displaySessionTitle,
  initialRequestGuard,
  mountRequests,
  normalizeCodexSessionPage,
  parseSafeCommandError,
  safeFailure,
  unmountRequests,
  UNNAMED_SESSION_TITLE,
} from "../src/lib/codexSessions.ts";

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
    assert.equal(result.message, "操作没有完成，请重试。");
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
  assert.equal(acceptsResponse(guard, "action", secondAction.generation), false);
  const remountedAction = beginRequest(guard, "action"); guard = remountedAction.state;
  assert.equal(acceptsResponse(guard, "action", remountedAction.generation), true);
});

test("late detection responses are rejected after target changes", () => {
  let guard = initialRequestGuard();
  const first = beginRequest(guard, "detect");
  guard = first.state;
  const second = beginRequest(guard, "detect");
  guard = second.state;

  assert.equal(acceptsResponse(guard, "detect", first.generation), false);
  assert.equal(acceptsResponse(guard, "detect", second.generation), true);
  guard = unmountRequests(guard);
  assert.equal(acceptsResponse(guard, "detect", second.generation), false);
  guard = mountRequests(guard);
  assert.equal(acceptsResponse(guard, "detect", second.generation), false);
});

test("search input is bounded before crossing the command boundary", () => {
  assert.equal(boundSessionQuery(`  ${"a".repeat(200)}  `).length, 80);
});

test("safe mutation errors remain compact and safe to render", () => {
  const failure = safeFailure({ version: 1, code: "change_failed", message: "操作未完成，原有内容保持可用。", retryable: false });
  assert.equal(failure.message, "操作未完成，原有内容保持可用。");
  assert.doesNotMatch(JSON.stringify(failure), /config\.toml|auth\.json|token|SQLite/i);
});

test("session title display prefers the database title and never promotes an id", () => {
  const id = "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11";
  assert.equal(displaySessionTitle("数据库标题", "安全摘要", id), "数据库标题");
  assert.equal(displaySessionTitle(null, "安全摘要", id), "安全摘要");
  assert.equal(displaySessionTitle(`会话 ${id.slice(0, 8)}`, null, id), UNNAMED_SESSION_TITLE);
  assert.equal(displaySessionTitle(null, "/Users/a/.codex/config.toml", id), UNNAMED_SESSION_TITLE);
  assert.equal(displaySessionTitle(null, null, id), UNNAMED_SESSION_TITLE);
});

test("session blockers retain safe title, id, reason, and next step without summaries or internals", () => {
  const page = normalizeCodexSessionPage({
    status: "blocked",
    items: [{
      thread_id: "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11",
      title: "项目规划",
      summary: "不要把正文显示出来",
      provider: "custom",
      archived: false,
      can_continue: false,
      needs_migration: false,
      blockers: [{
        title: "项目规划",
        thread_id: "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11",
        reason: "会话记录重复。",
        next_step: "关闭 ChatGPT 后重新检查。",
      }],
    }],
    blockers: [{
      title: "项目规划",
      thread_id: "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11",
      reason: "会话记录重复。",
      next_step: "关闭 ChatGPT 后重新检查。",
    }],
    page: 1,
    page_size: 50,
    total: 1,
    total_pages: 1,
  });
  assert.ok(page);
  assert.equal(page.items[0].title, "项目规划");
  assert.equal(page.blockers[0].thread_id, "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11");
  assert.equal(page.blockers[0].reason, "会话记录重复。");
  assert.equal(page.blockers[0].next_step, "关闭 ChatGPT 后重新检查。");
  assert.equal(page.items[0].provider, "Niko 模型服务");
  assert.equal(page.items[0].summary, null);
  assert.doesNotMatch(JSON.stringify(page), /不要把正文显示出来/);
  const unsafe = normalizeCodexSessionPage({
    ...page,
    blockers: [{
      title: "/Users/a/.codex/config.toml",
      thread_id: "not-an-id",
      reason: "原始 token sk-secret",
      next_step: "查看 /tmp/details",
    }],
  });
  assert.ok(unsafe);
  assert.equal(unsafe.blockers.length, 0);
});

test("normalized session items use safe title fallback when the IPC title is missing", () => {
  const page = normalizeCodexSessionPage({
    status: "healthy",
    items: [{
      thread_id: "019fb1b4-f24c-7ec3-a736-c68cf9a0ae11",
      title: null,
      summary: "可显示的摘要",
      archived: false,
      can_continue: true,
      needs_migration: false,
      blockers: [],
    }],
    blockers: [],
    page: 1,
    page_size: 50,
    total: 1,
    total_pages: 1,
  });
  assert.ok(page);
  assert.equal(page.items[0].title, "可显示的摘要");
  assert.equal(page.items[0].summary, null);
});
