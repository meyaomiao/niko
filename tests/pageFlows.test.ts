import assert from "node:assert/strict";
import test from "node:test";
import { build } from "esbuild";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

// 执行实际页面的 hooks、事件处理和异步回调；所有网络/IPC/存储均为内存 fixture。
// 不需要 DOM、Tauri、真实账号、目标应用或磁盘构建缓存。
type Node = { type: unknown; props: Record<string, any> };
type Slot = { value?: any; deps?: unknown[]; cleanup?: (() => void) | void };
const require = createRequire(import.meta.url);
const bundles = new Map<string, Promise<string>>();

function bundle(page: string): Promise<string> {
  if (!bundles.has(page)) {
    bundles.set(page, build({
      entryPoints: [fileURLToPath(new URL(`../src/pages/${page}.tsx`, import.meta.url))],
      bundle: true, write: false, format: "cjs", platform: "node", jsx: "automatic",
      packages: "external", logLevel: "silent",
      loader: { ".svg": "text", ".png": "dataurl", ".jpg": "dataurl", ".webp": "dataurl" },
      plugins: [{
        name: "page-fixtures",
        setup(builder) {
          builder.onResolve({ filter: /(?:api\/client|store\/auth|hooks\/useSession|lib\/catalogCache)(?:\.ts)?$/ }, (args) => ({
            path: `fixture:${args.path.replace(/.*\//, "").replace(/\.ts$/, "")}`,
            external: true,
          }));
        },
      }],
    }).then((result) => result.outputFiles[0].text));
  }
  return bundles.get(page)!;
}

function deferred<T = any>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

const authFixture = () => ({ accessToken: "fixture-session", username: "fixture", quota: 500000, quotaPerUnit: 500000 });
const summaryFixture = () => ({ quota: 100, requests: 1, prompt_tokens: 2, completion_tokens: 3, stream_requests: 0, by_day: [], by_group: [], by_model: [] });
const catalogFixture = () => ({
  site: { quota_per_unit: 500000 }, user: { quota: 500000, group: "fast" },
  models: ["gpt-fixture-a", "gpt-fixture-b"], groups: [{ name: "fast", ratio: 1, models: ["gpt-fixture-a", "gpt-fixture-b"] }],
  pricing: ["gpt-fixture-a", "gpt-fixture-b"].map((model_name) => ({ model_name, model_ratio: 1, completion_ratio: 1, quota_type: 0, enable_groups: ["fast"] })),
});
const logFixture = (model_name: string) => ({ id: 1, created_at: 1700000000, model_name, prompt_tokens: 1, completion_tokens: 1, quota: 1 });
const topupRecord = (trade_no: string, status: string) => ({ id: 1, trade_no, status, amount: 10, money: 10, payment_method: "alipay", create_time: 1, complete_time: 0 });
const benchResult = (median_ttft_ms: number) => ({ median_ttft_ms, samples: [{ ttft_ms: median_ttft_ms, error: null }] });

async function mount(page: string, apiOverrides: Record<string, any> = {}, invokeOverride?: (name: string, args: any) => Promise<any>) {
  const slots: Slot[] = [];
  let cursor = 0;
  let effects: Array<() => void> = [];
  let dirty = true;
  let mounted = true;
  let tree: Node;
  let updatesAfterUnmount = 0;
  let auth: any = authFixture();
  let timerId = 0;
  const timers = new Map<number, () => void>();
  const storage = new Map<string, string>();
  const calls: { name: string; args: any }[] = [];
  const authWrites: any[] = [];
  const sameDeps = (a?: unknown[], b?: unknown[]) => Boolean(a && b && a.length === b.length && a.every((value, i) => Object.is(value, b[i])));
  const hooks = {
    useState(initial: any) {
      const id = cursor++;
      if (!slots[id]) slots[id] = { value: typeof initial === "function" ? initial() : initial };
      return [slots[id].value, (value: any) => {
        if (!mounted) { updatesAfterUnmount += 1; return; }
        const next = typeof value === "function" ? value(slots[id].value) : value;
        if (!Object.is(next, slots[id].value)) { slots[id].value = next; dirty = true; }
      }];
    },
    useRef(value: any) {
      const id = cursor++;
      if (!slots[id]) slots[id] = { value: { current: value } };
      return slots[id].value;
    },
    useMemo(fn: () => any, deps?: unknown[]) {
      const id = cursor++;
      if (!slots[id] || !sameDeps(slots[id].deps, deps)) slots[id] = { value: fn(), deps };
      return slots[id].value;
    },
    useCallback(fn: any, deps?: unknown[]) { return hooks.useMemo(() => fn, deps); },
    useEffect(fn: () => (() => void) | void, deps?: unknown[]) {
      const id = cursor++;
      if (!slots[id] || !sameDeps(slots[id].deps, deps)) {
        const previous = slots[id]?.cleanup;
        slots[id] = { deps };
        effects.push(() => { previous?.(); slots[id].cleanup = fn(); });
      }
    },
  };
  const isCurrentAuthSession = (expected: any) => Boolean(expected && auth && expected.accessToken === auth.accessToken && expected.origin === auth.origin);
  const api = {
    bootstrap: async () => catalogFixture(),
    usageSummary: async () => summaryFixture(),
    usage: async () => ({ items: [], total: 0 }),
    status: async () => ({ quota_per_unit: 500000 }),
    catalog: async () => ({ bootstrap: catalogFixture(), pricingMeta: { vendors: [], usableGroup: {}, groupRatio: {} }, at: Date.now() }),
    provision: async () => ({ api_key: "fixture-key" }),
    topupInfo: async () => ({ enable_online_topup: true, min_topup: 1, amount_options: [10], pay_methods: [{ name: "支付宝", type: "alipay" }] }),
    topupHistory: async () => ({ items: [] }),
    requestEpay: async () => ({ url: "https://cashier.invalid", params: { out_trade_no: "order-current" } }),
    ...apiOverrides,
  };
  const modules: Record<string, any> = {
    react: hooks,
    "react/jsx-runtime": { jsx: (type: unknown, props: any) => ({ type, props }), jsxs: (type: unknown, props: any) => ({ type, props }), Fragment: Symbol("Fragment") },
    "react-router-dom": { useNavigate: () => () => {} },
    "@tauri-apps/api/core": { invoke: async (name: string, args: any) => { calls.push({ name, args }); return invokeOverride?.(name, args); } },
    "fixture:client": { api },
    "fixture:auth": {
      loadAuth: () => auth,
      isCurrentAuthSession,
      refreshAuthMeta: (patch: any, expected: any) => {
        if (!isCurrentAuthSession(expected)) return false;
        authWrites.push(patch); auth = { ...auth, ...patch }; return true;
      },
    },
    "fixture:catalogCache": { peekCurrentCatalog: () => null },
    "fixture:useSession": { useSession: () => ({ handleSessionExpired: () => calls.push({ name: "expired", args: undefined }) }) },
  };
  const module = { exports: {} as any };
  vm.runInNewContext(await bundle(page), {
    module, exports: module.exports,
    require: (name: string) => modules[name] ?? require(name),
    console, performance, AbortController, URLSearchParams, URL,
    fetch: () => { throw new Error("Unexpected real network call"); },
    localStorage: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) },
    window: {
      setTimeout: (fn: () => void) => { const id = ++timerId; timers.set(id, fn); return id; },
      clearTimeout: (id: number) => timers.delete(id),
    },
  });
  const render = () => {
    dirty = false; cursor = 0;
    tree = module.exports.default();
    const pending = effects; effects = [];
    pending.forEach((effect) => effect());
  };
  const flush = async () => {
    for (let tick = 0; tick < 8; tick += 1) {
      let renders = 0;
      while (mounted && dirty) {
        if (++renders > 30) throw new Error("Render loop in page fixture");
        render();
      }
      await new Promise<void>((resolve) => setImmediate(resolve));
    }
  };
  const walkNodes = (node: any): Node[] => {
    if (Array.isArray(node)) return node.flatMap(walkNodes);
    if (!node || typeof node !== "object") return [];
    return [node, ...walkNodes(node.props?.children)];
  };
  const nodes = () => walkNodes(tree);
  const textOf = (node: any): string => {
    if (Array.isArray(node)) return node.map(textOf).join("");
    if (node == null || typeof node === "boolean") return "";
    if (typeof node !== "object") return String(node);
    return textOf(node.props?.children);
  };
  const button = (label: string) => {
    const found = nodes().find((node) => node.type === "button" && textOf(node) === label);
    assert.ok(found, `Missing button: ${label}`);
    return found;
  };
  const selectModel = (name: string) => {
    const found = nodes().find((node) => node.props?.card?.name === name);
    assert.ok(found, `Missing model: ${name}`);
    found.props.onSelect();
  };
  await flush();
  return {
    api, calls, storage, authWrites, flush, button, selectModel,
    text: () => textOf(tree),
    nodes,
    setAuth(value: any) { auth = value; },
    get updatesAfterUnmount() { return updatesAfterUnmount; },
    get timerCount() { return timers.size; },
    async tick() {
      const next = timers.entries().next().value;
      assert.ok(next, "Expected a pending polling timer");
      timers.delete(next[0]); next[1](); await flush();
    },
    unmount() { mounted = false; slots.forEach((slot) => slot.cleanup?.()); },
  };
}

test("Usage restores first-page rows after going to page two", async () => {
  const pages: number[] = [];
  const view = await mount("Usage", {
    usage: async (_token: string, query: any) => { pages.push(query.page); return { items: [logFixture(query.page === 1 ? "first-page" : "second-page")], total: 100 }; },
  });
  assert.match(view.text(), /first-page/);
  view.button("下一页").props.onClick(); await view.flush();
  assert.match(view.text(), /second-page/);
  view.button("上一页").props.onClick(); await view.flush();
  assert.match(view.text(), /first-page/);
  assert.doesNotMatch(view.text(), /second-page/);
  assert.deepEqual(pages, [1, 2]);
});

test("Usage changes filters while a page request is pending without locking pagination", async () => {
  const oldPage = deferred();
  const view = await mount("Usage", {
    usage: async (_token: string, query: any) => query.page === 2 ? oldPage.promise : { items: [logFixture("current-first-page")], total: 100 },
  });
  view.button("下一页").props.onClick(); await view.flush();
  assert.equal(view.button("下一页").props.disabled, true);
  view.button("今天").props.onClick(); await view.flush();
  assert.equal(view.button("下一页").props.disabled, false);
  oldPage.resolve({ items: [logFixture("stale-second-page")], total: 100 }); await view.flush();
  assert.match(view.text(), /current-first-page/);
  assert.doesNotMatch(view.text(), /stale-second-page/);
});

test("Usage can return to its first page after a page request fails", async () => {
  const view = await mount("Usage", {
    usage: async (_token: string, query: any) => {
      if (query.page === 2) throw new Error("network timeout");
      return { items: [logFixture("first-page")], total: 100 };
    },
  });
  view.button("下一页").props.onClick(); await view.flush();
  assert.equal(view.nodes().filter((node) => node.props?.className?.includes("nk-alert-danger")).length, 1);
  view.button("上一页").props.onClick(); await view.flush();
  assert.match(view.text(), /first-page/);
  assert.equal(view.nodes().filter((node) => node.props?.className?.includes("nk-alert-danger")).length, 0);
});

test("Usage vendor filtering reads the ledger when bootstrap models is empty", async () => {
  let ledgerReads = 0;
  const view = await mount("Usage", {
    bootstrap: async () => ({ ...catalogFixture(), models: [] }),
    usageRecords: async (_token: string, query: any) => {
      ledgerReads += 1;
      assert.equal(query.models, undefined);
      return [logFixture("gpt-fixture-a"), logFixture("claude-other-vendor")];
    },
  });
  const select = view.nodes().find((node) => node.props?.["aria-label"] === "按模型厂商筛选");
  assert.ok(select);
  select.props.onChange({ target: { value: "OpenAI" } }); await view.flush();
  assert.equal(ledgerReads, 1);
  assert.match(view.text(), /gpt-fixture-a/);
  assert.doesNotMatch(view.text(), /claude-other-vendor/);
  assert.match(view.text(), /共 1 条记录/);
  const ranking = view.nodes().find((node) => node.props?.title === "模型使用排行");
  assert.equal(ranking?.props.items[0].name, "gpt-fixture-a");
  assert.equal(ranking?.props.items[0].requests, 1);
});

test("Usage vendor filtering includes retired models in both totals and details", async () => {
  const catalog = catalogFixture();
  const view = await mount("Usage", {
    bootstrap: async () => ({ ...catalog, models: ["gpt-fixture-a"], pricing: catalog.pricing.slice(0, 1) }),
    usageRecords: async (_token: string, query: any) => {
      assert.equal(query.models, undefined);
      assert.ok(query.startTimestamp > 0 && query.endTimestamp > query.startTimestamp);
      return [
        { ...logFixture("gpt-fixture-a"), quota: 500000 },
        { ...logFixture("gpt-retired-model"), quota: 1000000 },
        { ...logFixture("claude-other-vendor"), quota: 9000000 },
      ];
    },
  });
  const select = view.nodes().find((node) => node.props?.["aria-label"] === "按模型厂商筛选");
  assert.ok(select);
  select.props.onChange({ target: { value: "OpenAI" } }); await view.flush();
  assert.match(view.text(), /gpt-fixture-a/);
  assert.match(view.text(), /gpt-retired-model/);
  assert.doesNotMatch(view.text(), /claude-other-vendor/);
  assert.match(view.text(), /共 2 条记录/);
  const ranking = view.nodes().find((node) => node.props?.title === "模型使用排行");
  assert.equal(ranking?.props.items.length, 2);
  assert.equal(ranking?.props.items.reduce((total: number, item: any) => total + item.quota, 0), 1500000);
  assert.equal(ranking?.props.items.reduce((total: number, item: any) => total + item.requests, 0), 2);
});

test("Models does not start benchmark after an obsolete provision response", async () => {
  const provision = deferred();
  const view = await mount("Models", { provision: () => provision.promise });
  view.selectModel("gpt-fixture-a"); await view.flush();
  const run = view.button("⚡ 测速").props.onClick(); await view.flush();
  view.selectModel("gpt-fixture-b"); await view.flush();
  provision.resolve({ api_key: "fixture-old-key" }); await run; await view.flush();
  assert.equal(view.calls.length, 0);
  assert.equal(view.storage.size, 0);
  assert.equal(view.button("⚡ 测速").props.disabled, false);
});

test("Models ignores old benchmark completion while the newly selected model is running", async () => {
  const requests = [deferred(), deferred()];
  let index = 0;
  const view = await mount("Models", {}, (name) => name === "benchmark_group" ? requests[index++].promise : Promise.resolve());
  view.selectModel("gpt-fixture-a"); await view.flush();
  const first = view.button("⚡ 测速").props.onClick(); await view.flush();
  view.selectModel("gpt-fixture-b"); await view.flush();
  const second = view.button("⚡ 测速").props.onClick(); await view.flush();
  requests[0].resolve(benchResult(1200)); await first; await view.flush();
  assert.equal(view.button("测速中 0/1…").props.disabled, true);
  assert.doesNotMatch(view.text(), /1\.2s/);
  assert.equal(view.storage.size, 0);
  requests[1].resolve(benchResult(8700)); await second; await view.flush();
  assert.match(view.text(), /8\.7s/);
  assert.deepEqual(Object.keys(JSON.parse(view.storage.get("niko_model_bench")!)), ["gpt-fixture-b"]);
});

test("Models discards a benchmark rejection after leaving the page", async () => {
  const request = deferred();
  const view = await mount("Models", {}, () => request.promise);
  view.selectModel("gpt-fixture-a"); await view.flush();
  const run = view.button("⚡ 测速").props.onClick(); await view.flush();
  view.unmount(); request.reject(new Error("fixture network failure")); await run;
  assert.equal(view.updatesAfterUnmount, 0);
  assert.equal(view.storage.size, 0);
});

test("TopUp waits for its order, then forces balance refresh and closes exactly once", async () => {
  const balanceOptions: any[] = [];
  let records: any[] = [];
  const view = await mount("TopUp", {
    bootstrap: async (_token: string, options: any) => { balanceOptions.push(options); return catalogFixture(); },
    topupHistory: async () => ({ items: records }),
  });
  await view.button("去支付").props.onClick(); await view.flush();
  records = [topupRecord("unrelated-paid-order", "success"), topupRecord("order-current", "pending")];
  await view.tick();
  assert.equal(view.calls.filter((call) => call.name === "close_cashier").length, 0);
  assert.equal(balanceOptions.length, 1);
  records = [topupRecord("order-current", "success")];
  await view.tick();
  assert.equal(view.calls.filter((call) => call.name === "close_cashier").length, 1);
  assert.equal(view.timerCount, 0);
  assert.equal(balanceOptions.length, 2);
  assert.ok(balanceOptions.every((options) => options.force === true));
});

test("TopUp does not let the initial balance overwrite a later confirmed payment", async () => {
  const initialBalance = deferred();
  let bootstraps = 0;
  let records: any[] = [];
  const view = await mount("TopUp", {
    bootstrap: () => ++bootstraps === 1 ? initialBalance.promise : Promise.resolve({ ...catalogFixture(), user: { quota: 900000, group: "fast" } }),
    topupHistory: async () => ({ items: records }),
  });
  await view.button("去支付").props.onClick(); await view.flush();
  records = [topupRecord("order-current", "success")]; await view.tick();
  assert.equal(view.authWrites.at(-1).quota, 900000);
  initialBalance.resolve(catalogFixture()); await view.flush();
  assert.equal(view.authWrites.at(-1).quota, 900000);
  assert.equal(view.authWrites.length, 1);
});

test("TopUp manual payment refresh confirms the same order and refreshes balance", async () => {
  let records: any[] = [];
  const view = await mount("TopUp", { topupHistory: async () => ({ items: records }) });
  await view.button("去支付").props.onClick(); await view.flush();
  records = [topupRecord("order-current", "success")];
  await view.button("重新读取到账状态").props.onClick(); await view.flush();
  assert.equal(view.calls.filter((call) => call.name === "close_cashier").length, 1);
  assert.equal(view.timerCount, 0);
});

test("TopUp discards a paid order response after leaving the page", async () => {
  const history = deferred();
  let requests = 0;
  const view = await mount("TopUp", { topupHistory: () => ++requests === 1 ? Promise.resolve({ items: [] }) : history.promise });
  await view.button("去支付").props.onClick(); await view.flush();
  await view.tick();
  view.unmount(); history.resolve({ items: [topupRecord("order-current", "success")] }); await view.flush();
  assert.equal(view.calls.filter((call) => call.name === "close_cashier").length, 0);
  assert.equal(view.updatesAfterUnmount, 0);
  assert.equal(view.timerCount, 0);
});

test("TopUp does not overwrite another account or close its cashier after a late balance response", async () => {
  const balance = deferred();
  let bootstraps = 0;
  let records: any[] = [];
  const view = await mount("TopUp", {
    bootstrap: () => ++bootstraps === 1 ? Promise.resolve(catalogFixture()) : balance.promise,
    topupHistory: async () => ({ items: records }),
  });
  await view.button("去支付").props.onClick(); await view.flush();
  records = [topupRecord("order-current", "success")]; await view.tick();
  const before = view.authWrites.length;
  view.setAuth({ ...authFixture(), accessToken: "other-session" });
  balance.resolve({ ...catalogFixture(), user: { quota: 999999, group: "other" } }); await view.flush();
  assert.equal(view.authWrites.length, before);
  assert.equal(view.calls.filter((call) => call.name === "close_cashier").length, 0);
});
