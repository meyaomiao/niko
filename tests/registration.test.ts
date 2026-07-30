import assert from "node:assert/strict";
import test from "node:test";
import {
  createRegistrationSubmissionGate,
  registerThenLogin,
  registrationErrorMessage,
  toggleAuthMode,
  validateRegistration,
} from "../src/lib/registration.ts";

test("switches between the lightweight login and registration modes", () => {
  assert.equal(toggleAuthMode("login"), "register");
  assert.equal(toggleAuthMode("register"), "login");
});

test("validates username, password length, and matching confirmation", () => {
  assert.equal(validateRegistration({
    username: "a",
    password: "Password123",
    passwordConfirmation: "Password123",
  }), "用户名需为 2-32 个字符");
  assert.equal(validateRegistration({
    username: "alice",
    password: "short",
    passwordConfirmation: "short",
  }), "密码需为 8-128 个字符");
  assert.equal(validateRegistration({
    username: "alice",
    password: "Password123",
    passwordConfirmation: "Password456",
  }), "两次输入的密码不一致");
  assert.equal(validateRegistration({
    username: " alice ",
    password: "Password123",
    passwordConfirmation: "Password123",
  }), "");
});

test("ignores a repeated registration submission until the first one settles", async () => {
  const gate = createRegistrationSubmissionGate();
  let release!: () => void;
  let calls = 0;
  const first = gate.run(async () => {
    calls += 1;
    await new Promise<void>((resolve) => { release = resolve; });
    return "created";
  });
  const duplicate = await gate.run(async () => {
    calls += 1;
    return "duplicate";
  });

  assert.deepEqual(duplicate, { started: false });
  assert.equal(calls, 1);
  release();
  assert.deepEqual(await first, { started: true, value: "created" });
});

test("registers first and then returns the existing desktop login result", async () => {
  const calls: string[] = [];
  const result = await registerThenLogin(
    "alice",
    async () => { calls.push("register"); },
    async () => {
      calls.push("login");
      return { access_token: "desktop-token" };
    },
  );

  assert.deepEqual(calls, ["register", "login"]);
  assert.deepEqual(result, {
    kind: "authenticated",
    login: { access_token: "desktop-token" },
  });
});

test("keeps the created username when automatic login fails", async () => {
  const result = await registerThenLogin(
    "alice",
    async () => undefined,
    async () => { throw new Error("offline"); },
  );

  assert.equal(result.kind, "created");
  if (result.kind === "created") {
    assert.equal(result.username, "alice");
    assert.equal(result.notice, "账号已创建，请登录");
    assert.match(String(result.loginError), /offline/);
  }
});

test("maps registration errors without echoing an upstream response", () => {
  assert.equal(
    registrationErrorMessage({ code: "USERNAME_TAKEN", message: "internal account detail" }),
    "这个用户名已被使用，请换一个再试",
  );
  assert.equal(
    registrationErrorMessage({ code: "RATE_LIMITED", retry_after_seconds: 12 }),
    "操作过于频繁，请在 12 秒后重试",
  );
  assert.equal(
    registrationErrorMessage({ message: "NIKO_BFF_SECRET leaked by upstream" }),
    "注册失败，请稍后重试",
  );
});
