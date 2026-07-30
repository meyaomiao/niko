import assert from "node:assert/strict";
import test, { beforeEach } from "node:test";
import {
  clearAuth,
  loadAuth,
  saveAuth,
  shouldPersistAuthSession,
  type AuthState,
} from "../src/store/auth.ts";

class MemoryStorage {
  private values = new Map<string, string>();

  getItem(key: string) {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string) {
    this.values.set(key, value);
  }

  removeItem(key: string) {
    this.values.delete(key);
  }
}

const auth: AuthState = {
  accessToken: "token",
  username: "niko",
  userId: 1,
  quota: 0,
  group: "default",
  apiKey: "key",
  remember: true,
};

beforeEach(() => {
  Object.defineProperty(globalThis, "localStorage", { value: new MemoryStorage(), configurable: true });
  Object.defineProperty(globalThis, "sessionStorage", { value: new MemoryStorage(), configurable: true });
});

test("persists remembered authentication across app sessions", () => {
  saveAuth(auth);

  assert.deepEqual(loadAuth(), auth);
  assert.equal(sessionStorage.getItem("niko_auth"), null);
  assert.ok(localStorage.getItem("niko_auth"));
});

test("keeps unremembered authentication in the current session only", () => {
  saveAuth({ ...auth, remember: false });

  assert.deepEqual(loadAuth(), { ...auth, remember: false });
  assert.equal(localStorage.getItem("niko_auth"), null);
  assert.ok(sessionStorage.getItem("niko_auth"));
});

test("persists only remembered ordinary logins, never registration auto-login", () => {
  assert.equal(shouldPersistAuthSession(true, true), true);
  assert.equal(shouldPersistAuthSession(true, false), false);
  assert.equal(shouldPersistAuthSession(false, true), false);
});

test("does not auto-login legacy authentication without an explicit remember choice", () => {
  localStorage.setItem("niko_auth", JSON.stringify({ ...auth, remember: undefined }));

  assert.equal(loadAuth(), null);
});

test("clears persistent and session authentication", () => {
  localStorage.setItem("niko_auth", JSON.stringify(auth));
  sessionStorage.setItem("niko_auth", JSON.stringify({ ...auth, remember: false }));

  clearAuth();

  assert.equal(localStorage.getItem("niko_auth"), null);
  assert.equal(sessionStorage.getItem("niko_auth"), null);
});
