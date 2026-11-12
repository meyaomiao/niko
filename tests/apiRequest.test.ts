import assert from "node:assert/strict";
import test, { afterEach, beforeEach } from "node:test";
import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import { ApiRequestError, requestJson } from "../src/lib/apiRequest.ts";

const originalFetch = globalThis.fetch;
const originalTimeout = globalThis.setTimeout;

beforeEach(() => {
  const makeStorage = () => {
    const values = new Map<string, string>();
    return {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => void values.set(key, value),
      removeItem: (key: string) => void values.delete(key),
    };
  };
  Object.defineProperty(globalThis, "sessionStorage", { value: makeStorage(), configurable: true });
  Object.defineProperty(globalThis, "localStorage", { value: makeStorage(), configurable: true });
});

afterEach(() => {
  globalThis.fetch = originalFetch;
  globalThis.setTimeout = originalTimeout;
  delete (globalThis as any).__catalogTestInvoke;
});

function stubFetch(callback: (...args: Parameters<typeof fetch>) => Promise<unknown>) {
  globalThis.fetch = callback as typeof fetch;
}

const clientModule = build({
  entryPoints: [fileURLToPath(new URL("../src/api/client.ts", import.meta.url))],
  bundle: true,
  write: false,
  format: "esm",
  platform: "node",
  logLevel: "silent",
  plugins: [{
    name: "forbid-real-ipc",
    setup(builder) {
      builder.onResolve({ filter: /^@tauri-apps\// }, (args) => ({ path: args.path, namespace: "mock-ipc" }));
      builder.onLoad({ filter: /.*/, namespace: "mock-ipc" }, () => ({
        contents: "export function invoke(...args) { if (globalThis.__catalogTestInvoke) return globalThis.__catalogTestInvoke(...args); throw new Error('Real IPC is forbidden in API tests'); }",
      }));
    },
  }],
}).then((output) => import(`data:text/javascript;base64,${Buffer.from(output.outputFiles[0].text).toString("base64")}`));

const loginParams = { username: "fixture", password: "fixture", deviceId: "fixture-device", deviceName: "test", platform: "test" };

test("deadline aborts a transport that never produces headers", async () => {
  let signal: AbortSignal | undefined;
  stubFetch(async (_url, init) => {
    signal = init?.signal as AbortSignal;
    return new Promise(() => {});
  });
  await assert.rejects(requestJson("https://fixture.invalid/headers", {}, 5), (error) => {
    assert.ok(error instanceof ApiRequestError);
    assert.equal(error.code, "timeout");
    return true;
  });
  assert.equal(signal?.aborted, true);
});

test("deadline includes stalled JSON bodies even when abort is ignored", async () => {
  let signal: AbortSignal | undefined;
  stubFetch(async (_url, init) => {
    signal = init?.signal as AbortSignal;
    return { ok: true, status: 200, json: () => new Promise(() => {}) };
  });
  await assert.rejects(requestJson("https://fixture.invalid/body", {}, 5), /超时/);
  assert.equal(signal?.aborted, true);
});

test("HTTP 401 retains status and server error code", async () => {
  stubFetch(async () => Response.json({ success: false, code: "E_SESSION_EXPIRED", message: "登录状态已失效" }, { status: 401 }));
  await assert.rejects(requestJson("https://fixture.invalid/auth"), (error) => {
    assert.ok(error instanceof ApiRequestError);
    assert.equal(error.status, 401);
    assert.equal(error.code, "E_SESSION_EXPIRED");
    assert.match(error.message, /401/);
    return true;
  });
});

test("HTML HTTP failure exposes status without proxy response contents", async () => {
  stubFetch(async () => new Response("<html>private upstream detail</html>", { status: 502 }));
  await assert.rejects(requestJson("https://fixture.invalid/proxy"), (error) => {
    assert.ok(error instanceof ApiRequestError);
    assert.equal(error.status, 502);
    assert.doesNotMatch(error.message, /private|html|Unexpected/);
    return true;
  });
});

test("invalid JSON in a successful response gives a recoverable error", async () => {
  stubFetch(async () => new Response("not JSON", { status: 200 }));
  await assert.rejects(requestJson("https://fixture.invalid/json"), (error) => {
    assert.ok(error instanceof ApiRequestError);
    assert.equal(error.code, "invalid_response");
    return true;
  });
});

test("caller cancellation aborts the request and skips pre-cancelled fetches", async () => {
  const controller = new AbortController();
  let called = 0;
  let signal: AbortSignal | undefined;
  stubFetch(async (_url, init) => {
    called += 1;
    signal = init?.signal as AbortSignal;
    return new Promise(() => {});
  });
  const pending = requestJson("https://fixture.invalid/cancel", { signal: controller.signal });
  controller.abort();
  await assert.rejects(pending, { name: "AbortError" });
  assert.equal(signal?.aborted, true);
  await assert.rejects(requestJson("https://fixture.invalid/cancel", { signal: controller.signal }), { name: "AbortError" });
  assert.equal(called, 1);
});

test("successful requests clear the deadline", async () => {
  let signal: AbortSignal | undefined;
  stubFetch(async (_url, init) => {
    signal = init?.signal as AbortSignal;
    return Response.json({ value: 42 });
  });
  assert.deepEqual(await requestJson("https://fixture.invalid/ok", {}, 5), { value: 42 });
  await new Promise((resolve) => originalTimeout(resolve, 10));
  assert.equal(signal?.aborted, false);
});

test("login and 2FA preserve DeviceLimitError for HTTP 200 and 409", async () => {
  const { api, DeviceLimitError } = await clientModule;
  const devices = [{ id: 7, device_name: "fixture device" }];
  for (const status of [200, 409]) {
    stubFetch(async () => Response.json({ success: false, code: "E_DEVICE_LIMIT", data: { device_limit: 1, active_devices: devices } }, { status }));
    for (const request of [() => api.login(loginParams), () => api.login2fa("pending-fixture", "123456")]) {
      await assert.rejects(request(), (error: any) => {
        assert.ok(error instanceof DeviceLimitError);
        assert.equal(error.deviceLimit, 1);
        assert.deepEqual(error.devices, devices);
        return true;
      });
    }
  }
});

test("login validates tokens while preserving the normal and 2FA response shapes", async () => {
  const { api } = await clientModule;
  stubFetch(async () => Response.json({ success: true, data: {} }));
  await assert.rejects(api.login(loginParams), /无效数据/);
  stubFetch(async () => Response.json({ success: true, data: { require_2fa: true, pending_token: "pending-fixture" } }));
  assert.equal((await api.login(loginParams)).pending_token, "pending-fixture");
  stubFetch(async () => Response.json({ success: true, data: { session_token: "session-fixture", user: { username: "fixture" } } }));
  assert.equal((await api.login2fa("pending-fixture", "123456")).access_token, "session-fixture");
});

test("epay keeps its message/data/url contract", async () => {
  const { api } = await clientModule;
  stubFetch(async () => Response.json({ message: "success", url: "https://fixture.invalid/pay", data: { trade_no: "fixture" } }));
  assert.deepEqual(await api.requestEpay("fixture", 1, "fixture"), { url: "https://fixture.invalid/pay", params: { trade_no: "fixture" } });
  stubFetch(async () => Response.json({ message: "error", data: "下单未完成" }));
  await assert.rejects(api.requestEpay("fixture", 1, "fixture"), /下单未完成/);
});

test("all official request paths terminate with a bounded timeout", async () => {
  const { api } = await clientModule;
  // Speed up only our 10-second production deadline; no real request is sent.
  globalThis.setTimeout = ((callback: (...args: any[]) => void, delay: number, ...args: any[]) =>
    originalTimeout(callback, delay === 10_000 ? 5 : delay, ...args)) as typeof setTimeout;
  stubFetch(async () => new Promise(() => {}));
  const requests = [
    () => api.login(loginParams),
    () => api.login2fa("fixture", "123456"),
    () => api.status(),
    () => api.pricingMeta(),
    () => api.bootstrap("fixture", { force: true }),
    () => api.provision("fixture", "default"),
    () => api.requestEpay("fixture", 1, "fixture"),
    () => api.revokeDevice("fixture", 1),
    () => api.revokeOtherDevices("fixture"),
  ];
  for (const request of requests) await assert.rejects(request(), /超时/);
});

test("optional pricing timeout does not prevent bootstrap recovery", async () => {
  const { api } = await clientModule;
  globalThis.setTimeout = ((callback: (...args: any[]) => void, delay: number, ...args: any[]) =>
    originalTimeout(callback, delay === 10_000 ? 5 : delay, ...args)) as typeof setTimeout;
  const bootstrap = { user: { id: 1, quota: 20, group: "default" }, site: {}, pricing: [], models: [] };
  stubFetch(async (url) => String(url).endsWith("/pricing")
    ? new Promise(() => {})
    : Response.json({ success: true, data: bootstrap }));
  const catalog = await api.catalog("fixture-pricing", { force: true });
  assert.deepEqual(catalog.bootstrap, bootstrap);
  assert.deepEqual(catalog.pricingMeta, { vendors: [], usableGroup: {}, groupRatio: {} });
  assert.equal(catalog.sync.source, "account");
});

function directoryFixture() {
  return {
    user: { id: 3, quota: 100, group: "allowed" }, site: { quota_per_unit: 500000 },
    groups: [{ name: "allowed", ratio: 0.3, models: ["delisted"] }], models: ["delisted"],
    pricing: [{ model_name: "delisted", model_ratio: 9, enable_groups: ["allowed"] }],
  };
}
const currentPrice = (name = "current-model") => ({
  model_name: name, model_ratio: 0.2, model_price: 0, completion_ratio: 2, quota_type: 0,
  enable_groups: ["allowed"], vendor_id: 19, release_date: "2026-10-01",
});

test("official catalog consumes current pricing models, metadata and vendor icons without granting public groups", async () => {
  const { api } = await clientModule; api.clearCatalog();
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  stubFetch(async (url, init) => {
    calls.push({ url: String(url), init });
    return Response.json(String(url).endsWith("/pricing") ? {
      success: true, pricing_version: "current-version", vendors: [{ id: 19, name: "New Vendor", icon: "Qwen.Color" }],
      usable_group: { allowed: "可用", admin: "仅管理员" }, group_ratio: { allowed: 9, admin: 1 },
      data: [currentPrice(), { ...currentPrice("restricted"), enable_groups: ["admin"] }],
    } : { success: true, data: directoryFixture() });
  });
  const result = await api.catalog("fixture-current-catalog", { force: true });
  assert.deepEqual(result.bootstrap.models, ["current-model"]);
  assert.deepEqual(result.bootstrap.groups.map((g: any) => g.name), ["allowed"]);
  assert.equal(result.bootstrap.groups[0].ratio, 0.3);
  assert.equal(result.bootstrap.pricing[0].model_ratio, 0.2);
  assert.equal(result.bootstrap.model_metadata["current-model"].release_date, "2026-10-01");
  assert.equal(result.pricingMeta.vendors[0].icon, "Qwen.Color");
  assert.equal(result.sync.source, "public-pricing");
  assert.equal(result.sync.version, "current-version");
  const publicRequest = calls.find((call) => call.url.endsWith("/pricing"))!;
  assert.equal(publicRequest.init?.cache, "no-store");
  assert.equal(new Headers(publicRequest.init?.headers).has("Authorization"), false);
});

test("successful empty public catalog removes old models rather than reporting cached legacy membership", async () => {
  const { api } = await clientModule; api.clearCatalog();
  stubFetch(async (url) => Response.json(String(url).endsWith("/pricing") ? { success: true, data: [] } : { success: true, data: directoryFixture() }));
  const result = await api.catalog("fixture-empty-public", { force: true });
  assert.deepEqual(result.bootstrap.models, []);
  assert.deepEqual(result.bootstrap.groups[0].models, []);
  assert.equal(result.bootstrap.pricing_authoritative, true);
  assert.equal(result.sync.source, "public-pricing");
});

test("failed or malformed public responses fall back explicitly without clearing valid account models", async () => {
  const { api } = await clientModule;
  for (const payload of [{ success: false, message: "fixture failure" }, { success: true, vendors: [] }, { success: true, data: [null] }]) {
    api.clearCatalog();
    stubFetch(async (url) => Response.json(String(url).endsWith("/pricing") ? payload : { success: true, data: directoryFixture() }));
    const result = await api.catalog("fixture-invalid-public", { force: true });
    assert.equal(result.sync.source, "account");
    assert.deepEqual(result.bootstrap.models, ["delisted"]);
    assert.notEqual(result.bootstrap.pricing_authoritative, true);
  }
});

test("public outage keeps the last successful directory time while refreshing balance and restricting revoked groups", async () => {
  const { api } = await clientModule; api.clearCatalog();
  let outage = false;
  stubFetch(async (url) => {
    if (String(url).endsWith("/pricing")) return outage
      ? Response.json({ success: false }, { status: 503 })
      : Response.json({ success: true, pricing_version: "known-good", data: [currentPrice()], vendors: [{ id: 19, name: "New Vendor", icon: "Qwen.Color" }] });
    return Response.json({ success: true, data: outage ? { ...directoryFixture(), user: { quota: 300, group: "allowed" }, groups: [] } : directoryFixture() });
  });
  const first = await api.catalog("fixture-public-outage", { force: true });
  outage = true;
  const fallback = await api.catalog("fixture-public-outage", { force: true });
  assert.equal(fallback.bootstrap.user.quota, 300);
  assert.equal(fallback.bootstrap.pricing[0].model_name, "current-model");
  assert.deepEqual(fallback.bootstrap.models, []);
  assert.equal(fallback.pricingMeta.vendors[0].icon, "Qwen.Color");
  assert.equal(fallback.sync.source, "account");
  assert.equal(fallback.sync.at, first.sync.at);
  assert.equal(fallback.sync.version, "known-good");
});

test("third-party catalog never requests MomoToken public prices or changes custom account rates", async () => {
  const { api } = await clientModule; api.clearCatalog();
  sessionStorage.setItem("niko_auth", JSON.stringify({ kind: "newapi", origin: "https://custom.example", accessToken: "fixture-station" }));
  stubFetch(async () => { throw new Error("Official network requests are forbidden for a custom station"); });
  (globalThis as any).__catalogTestInvoke = async (command: string) => {
    assert.equal(command, "newapi_bootstrap");
    return { origin: "https://custom.example", status: { quota_per_unit: 1000 }, user: { id: 1, quota: 1, group: "custom" },
      groups: { custom: { ratio: 0.3 } }, models: ["custom-model"], pricing: { data: [{ model_name: "custom-model", model_ratio: 0.123 }] } };
  };
  const result = await api.catalog("fixture-station", { force: true });
  assert.equal(result.bootstrap.pricing[0].model_ratio, 0.123);
  assert.equal(result.bootstrap.pricing[0].enable_groups, undefined);
  assert.equal(result.bootstrap.groups[0].ratio, 0.3);
  assert.equal(result.sync.source, "station");
});
