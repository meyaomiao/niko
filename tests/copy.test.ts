import assert from "node:assert/strict";
import test from "node:test";
import {
  classifyDesktopError,
  displayDeviceLabel,
  friendlyLoginError,
  friendlyConnectivityDetail,
  friendlyDesktopError,
} from "../src/lib/copy.ts";

const FORBIDDEN_USER_TERMS = ["provider", "profile", "base url", "api key", "quota", "journal", "wal", "sqlite", "custom"];

test("maps common failures to one clear next step", () => {
  assert.equal(classifyDesktopError({ code: "UNAUTHORIZED", message: "401" }), "session");
  assert.equal(friendlyDesktopError({ code: "UNAUTHORIZED", message: "401" }), "登录状态已过期，请重新登录后再试。");
  assert.equal(friendlyDesktopError("余额不足"), "余额不足，请先充值后再试。");
  assert.equal(friendlyDesktopError(new Error("network timeout")), "网络连接失败，请检查网络后重试。");
  assert.equal(friendlyDesktopError("server unavailable"), "模型服务暂时不可用，请稍后重试。");
  assert.equal(friendlyDesktopError("未找到 ~/.codex/config.toml"), "还没有找到目标应用，请先安装并打开它，再试一次。");
  assert.equal(friendlyDesktopError("配置里没有默认模型，请先点击启用"), "接入还没有生效，请先接入到应用，再试一次。");
  assert.equal(friendlyConnectivityDetail("配置里没有默认模型，请先点击启用"), "接入还没有生效，请先接入到应用后再检查。");
  assert.equal(friendlyLoginError("账号或密码错误"), "账号或密码不正确，请检查后再试。");
});

test("connectivity failures do not expose status codes or raw credentials", () => {
  const messages = [
    friendlyConnectivityDetail("HTTP 401: Key invalid"),
    friendlyConnectivityDetail("HTTP 500 upstream error"),
    friendlyConnectivityDetail("request timeout"),
  ];
  for (const message of messages) {
    assert.doesNotMatch(message, /HTTP|Key|token|\/Users\//i);
    assert.ok(message.includes("请"));
  }
});

test("device labels do not fall back to internal identifiers", () => {
  assert.equal(displayDeviceLabel("", "macOS"), "macOS 设备");
  assert.equal(displayDeviceLabel("", ""), "其他设备");
  assert.equal(displayDeviceLabel("我的电脑", "Windows"), "我的电脑");
});

test("user-facing error copy keeps implementation terms out of the main message", () => {
  const messages = [
    friendlyDesktopError("unknown failure"),
    friendlyConnectivityDetail("unknown failure"),
    "连接密钥无效或已过期，请重新接入后再试。",
  ];
  for (const message of messages) {
    for (const term of FORBIDDEN_USER_TERMS) {
      assert.doesNotMatch(message, new RegExp(term, "i"));
    }
  }
});

test("desktop configuration failures never expose configuration as the next step", () => {
  const messages = [
    friendlyDesktopError("配置里没有默认模型，请先点击启用"),
    friendlyConnectivityDetail("配置里没有默认模型，请先点击启用"),
  ];
  for (const message of messages) {
    assert.doesNotMatch(message, /配置/);
    assert.match(message, /接入/);
  }
});
