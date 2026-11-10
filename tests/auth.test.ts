import assert from "node:assert/strict";
import test, { beforeEach } from "node:test";
import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import {
  clearAuth,
  loadAuth,
  isCurrentAuthSession,
  refreshAuthMeta,
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
  defaultGroup: "default",
  apiKey: "key",
  remember: true,
};

beforeEach(() => {
  Object.defineProperty(globalThis, "localStorage", { value: new MemoryStorage(), configurable: true });
  Object.defineProperty(globalThis, "sessionStorage", { value: new MemoryStorage(), configurable: true });
});

test("keeps remembered authentication out of persistent Web storage", () => {
  saveAuth(auth);

  assert.deepEqual(loadAuth(), auth);
  assert.ok(sessionStorage.getItem("niko_auth"));
  assert.equal(localStorage.getItem("niko_auth"), null);
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
  assert.equal(localStorage.getItem("niko_auth"), null);
});

test("migrates the old group field within the current session", () => {
  sessionStorage.setItem(
    "niko_auth",
    JSON.stringify({ ...auth, defaultGroup: undefined, group: "legacy-default" }),
  );

  const loaded = loadAuth()!;
  assert.equal(loaded.defaultGroup, "legacy-default");
  assert.equal("group" in loaded, false);
});

test("metadata refresh cannot create a target-active field", () => {
  saveAuth(auth);
  refreshAuthMeta({ quota: 42, defaultGroup: "server-recommendation" });

  assert.deepEqual(loadAuth(), {
    ...auth,
    quota: 42,
    defaultGroup: "server-recommendation",
  });
  assert.equal("activeGroup" in (loadAuth() as object), false);
});

test("clears persistent and session authentication", () => {
  localStorage.setItem("niko_auth", JSON.stringify(auth));
  sessionStorage.setItem("niko_auth", JSON.stringify({ ...auth, remember: false }));

  clearAuth();

  assert.equal(localStorage.getItem("niko_auth"), null);
  assert.equal(sessionStorage.getItem("niko_auth"), null);
});

test("late metadata cannot update another account or resurrect a logged-out session", () => {
  saveAuth(auth);
  const expected = loadAuth();
  const replacement = { ...auth, accessToken: "other-token", username: "other", quota: 99 };
  saveAuth(replacement);
  assert.equal(refreshAuthMeta({ quota: 1 }, expected), false);
  assert.deepEqual(loadAuth(), replacement);
  clearAuth();
  assert.equal(refreshAuthMeta({ quota: 1 }, expected), false);
  assert.equal(loadAuth(), null);
});

test("session matching includes provider/origin but tolerates same-session metadata updates", () => {
  saveAuth(auth);
  assert.equal(isCurrentAuthSession({ ...auth, kind: "momotoken" }), true);
  saveAuth({ ...auth, apiKey: "new-key" });
  assert.equal(refreshAuthMeta({ quota: 2 }, auth), true);
  assert.equal(loadAuth()?.apiKey, "new-key");
  const station = { ...auth, kind: "newapi" as const, origin: "https://first.invalid" };
  saveAuth(station);
  assert.equal(isCurrentAuthSession(auth), false);
  saveAuth({ ...station, origin: "https://second.invalid" });
  assert.equal(isCurrentAuthSession(station), false);
  assert.equal(isCurrentAuthSession(null), false);
  clearAuth();
  assert.equal(isCurrentAuthSession(null), true);
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((ok, fail) => { resolve = ok; reject = fail; });
  return { promise, resolve, reject };
}

// Execute real hook/effect/event-handler code with in-memory React/IPC/network
// fixtures. No WebView, keychain, installed target or real account is accessed.
function createRuntime(api: Record<string, any> = {}) {
  const slots: any[] = [];
  let cursor = 0;
  const pending: Array<() => void> = [];
  const navigations: string[] = [];
  const invocations: string[] = [];
  const runtime = {
    api: { clearCatalog() {}, ...api },
    navigate: (path: string) => { navigations.push(path); },
    invoke: async (command: string) => {
      invocations.push(command);
      if (command === "list_targets") return [];
      return null;
    },
    useState(initial: any) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = { state: typeof initial === "function" ? initial() : initial };
      return [slots[index].state, (value: any) => {
        slots[index].state = typeof value === "function" ? value(slots[index].state) : value;
      }];
    },
    useRef(initial: any) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = { current: initial };
      return slots[index];
    },
    useEffect(effect: () => void | (() => void), deps: any[] = []) {
      const index = cursor++;
      const prior = slots[index];
      if (prior && deps.length === prior.deps.length && deps.every((value, i) => value === prior.deps[i])) return;
      pending.push(() => {
        prior?.cleanup?.();
        slots[index] = { deps, cleanup: effect() };
      });
    },
  };
  (globalThis as any).__nikoAuthFixture = runtime;
  return {
    navigations,
    invocations,
    render(component: (props: any) => any, props: any = {}) {
      cursor = 0;
      const tree = component(props);
      for (const effect of pending.splice(0)) effect();
      return tree;
    },
    unmount() { for (const slot of slots) slot?.cleanup?.(); },
  };
}

async function loadSubject(entry: string) {
  const root = fileURLToPath(new URL("../", import.meta.url));
  const output = await build({
    entryPoints: [`${root}${entry}`],
    bundle: true,
    write: false,
    format: "esm",
    platform: "node",
    jsx: "automatic",
    logLevel: "silent",
    plugins: [{
      name: "isolated-auth-effects",
      setup(builder) {
        builder.onResolve({ filter: /^(react(?:\/jsx-runtime)?|react-router-dom|@tauri-apps\/.*)$|(?:api\/client|hooks\/useTheme|components\/|\.\/pages\/)/ }, (args) => {
          if (args.kind === "entry-point") return;
          return { path: args.path, namespace: "auth-fixture" };
        });
        builder.onLoad({ filter: /.*/, namespace: "auth-fixture" }, ({ path }) => {
          const env = "const env = () => globalThis.__nikoAuthFixture;";
          if (path === "react") return { contents: `${env}
            export const useState = (...args) => env().useState(...args);
            export const useRef = (...args) => env().useRef(...args);
            export const useEffect = (...args) => env().useEffect(...args);
            export const useCallback = (value) => value;` };
          if (path === "react/jsx-runtime") return { contents: "export const Fragment = 'fragment'; export const jsx = (type, props) => ({ type, props }); export const jsxs = jsx;" };
          if (path === "react-router-dom") return { contents: `${env} export const useNavigate = () => env().navigate; export const Routes = 'routes', Route = 'route', Navigate = 'navigate';` };
          if (path.includes("api/client")) return { contents: `${env} export const api = new Proxy({}, { get: (_, key) => env().api[key] }); export class DeviceLimitError extends Error {}` };
          if (path.startsWith("@tauri-apps/")) return { contents: `${env} export const invoke = (...args) => env().invoke(...args); export const open = () => { throw new Error('Real shell is forbidden'); };` };
          if (path.includes("useTheme")) return { contents: "export const initTheme = () => {};" };
          return { contents: "export default function Component() {} export const BookOpenIcon = () => null;" };
        });
      },
    }],
  });
  return import(`data:text/javascript;base64,${Buffer.from(output.outputFiles[0].text).toString("base64")}`);
}

function nodes(tree: any): any[] {
  if (!tree || typeof tree !== "object") return [];
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return [tree, ...nodes(tree.props?.children)];
}

async function flush() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

test("unmounted session refresh ignores both successful and unauthorized late responses", async () => {
  const { useSession } = await loadSubject("src/hooks/useSession.ts");
  for (const fail of [false, true]) {
    saveAuth(auth);
    const pending = deferred<any>();
    const fixture = createRuntime({ bootstrap: () => pending.promise });
    fixture.render(useSession);
    fixture.unmount();
    const replacement = { ...auth, accessToken: "other-token", quota: 99 };
    saveAuth(replacement);
    if (fail) pending.reject({ status: 401, message: "rejected" });
    else pending.resolve({ user: { quota: 1, group: "old" } });
    await flush();
    assert.deepEqual(loadAuth(), replacement);
    assert.deepEqual(fixture.navigations, []);
  }
});

test("current-session HTTP 401 and E_SESSION_EXPIRED still return to login", async () => {
  const { useSession } = await loadSubject("src/hooks/useSession.ts");
  for (const error of [{ status: 401, message: "rejected" }, { code: "E_SESSION_EXPIRED", message: "rejected" }]) {
    saveAuth(auth);
    const fixture = createRuntime({ bootstrap: async () => { throw error; } });
    fixture.render(useSession);
    await flush();
    fixture.unmount();
    assert.equal(loadAuth(), null);
    assert.deepEqual(fixture.navigations, ["/login"]);
  }
});

test("VersionGate cannot restore captured credentials after unmount/account switch", async () => {
  const { default: App } = await loadSubject("src/App.tsx");
  saveAuth(auth);
  const pending = deferred<any>();
  const fixture = createRuntime({ bootstrap: () => pending.promise });
  const routes = App();
  const route = nodes(routes).find((node) => node.props?.path === "/*");
  const gate = route.props.element.props.children.props.children;
  fixture.render(gate.type, gate.props);
  fixture.unmount();
  const replacement = { ...auth, accessToken: "other-token", apiKey: "other-key" };
  saveAuth(replacement);
  pending.resolve({ user: { quota: 1, group: "old" }, min_supported_version: "999.0.0" });
  await flush();
  assert.deepEqual(loadAuth(), replacement);
});

test("cancelled login does not save auth, write remembered credentials or navigate", async () => {
  const { default: Login } = await loadSubject("src/pages/Login.tsx");
  const pending = deferred<any>();
  const fixture = createRuntime({ login: () => pending.promise });
  let tree = fixture.render(Login);
  nodes(tree).find((node) => node.props?.autoComplete === "username").props.onChange({ target: { value: "fixture" } });
  nodes(tree).find((node) => node.props?.autoComplete === "current-password").props.onChange({ target: { value: "fixture-password" } });
  tree = fixture.render(Login);
  const submitted = nodes(tree).find((node) => node.type === "form").props.onSubmit({ preventDefault() {} });
  fixture.unmount();
  pending.resolve({ access_token: "new-fixture-token", username: "fixture" });
  await submitted;
  assert.equal(loadAuth(), null);
  assert.deepEqual(fixture.navigations, []);
  assert.equal(fixture.invocations.some((name) => /^(save|clear)_remembered/.test(name)), false);
});

test("MomoToken login replaces old provider before bootstrap and retains metadata if provision fails", async () => {
  const { default: Login } = await loadSubject("src/pages/Login.tsx");
  saveAuth({ ...auth, kind: "newapi", origin: "https://old-station.invalid" });
  const fixture = createRuntime({
    login: async () => ({ access_token: "momo-fixture", username: "fixture" }),
    bootstrap: async () => {
      assert.equal(loadAuth()?.kind, "momotoken");
      assert.equal(loadAuth()?.accessToken, "momo-fixture");
      return { user: { id: 2, quota: 42, group: "new-group" }, site: { quota_per_unit: 500000 } };
    },
    status: async () => ({ quota_per_unit: 500000 }),
    provision: async () => { throw new Error("fixture key request failure"); },
  });
  let tree = fixture.render(Login);
  nodes(tree).find((node) => node.props?.autoComplete === "username").props.onChange({ target: { value: "fixture" } });
  nodes(tree).find((node) => node.props?.autoComplete === "current-password").props.onChange({ target: { value: "fixture-password" } });
  tree = fixture.render(Login);
  await nodes(tree).find((node) => node.type === "form").props.onSubmit({ preventDefault() {} });
  fixture.unmount();
  assert.equal(loadAuth()?.userId, 2);
  assert.equal(loadAuth()?.quota, 42);
  assert.equal(loadAuth()?.defaultGroup, "new-group");
  assert.equal(loadAuth()?.apiKey, "");
  assert.deepEqual(fixture.navigations, ["/home"]);
});

test("returning from an in-flight 2FA attempt prevents late login and remembered writes", async () => {
  const { default: Login } = await loadSubject("src/pages/Login.tsx");
  const pending = deferred<any>();
  const fixture = createRuntime({
    login: async () => ({ require_2fa: true, pending_token: "pending-fixture" }),
    login2fa: () => pending.promise,
  });
  let tree = fixture.render(Login);
  nodes(tree).find((node) => node.props?.autoComplete === "username").props.onChange({ target: { value: "fixture" } });
  nodes(tree).find((node) => node.props?.autoComplete === "current-password").props.onChange({ target: { value: "fixture-password" } });
  tree = fixture.render(Login);
  await nodes(tree).find((node) => node.type === "form").props.onSubmit({ preventDefault() {} });
  tree = fixture.render(Login);
  nodes(tree).find((node) => node.props?.placeholder === "000000").props.onChange({ target: { value: "123456" } });
  tree = fixture.render(Login);
  const submitted = nodes(tree).find((node) => node.type === "form").props.onSubmit({ preventDefault() {} });
  tree = fixture.render(Login);
  nodes(tree).find((node) => node.type === "button" && node.props?.children === "← 返回登录").props.onClick();
  pending.resolve({ access_token: "cancelled-fixture", username: "fixture" });
  await submitted;
  tree = fixture.render(Login);
  fixture.unmount();
  assert.equal(loadAuth(), null);
  assert.deepEqual(fixture.navigations, []);
  assert.equal(fixture.invocations.some((name) => /^(save|clear)_remembered/.test(name)), false);
  assert.equal(nodes(tree).find((node) => node.props?.autoComplete === "username").props.disabled, false);
});

test("VersionGate metadata refresh preserves a key updated while bootstrap was pending", async () => {
  const { default: App } = await loadSubject("src/App.tsx");
  saveAuth(auth);
  const pending = deferred<any>();
  const fixture = createRuntime({ bootstrap: () => pending.promise });
  const route = nodes(App()).find((node) => node.props?.path === "/*");
  const gate = route.props.element.props.children.props.children;
  fixture.render(gate.type, gate.props);
  saveAuth({ ...auth, apiKey: "new-key", apiKeyGroup: "new-key-group" });
  pending.resolve({ user: { quota: 42, group: "fresh-group" } });
  await flush();
  fixture.unmount();
  assert.equal(loadAuth()?.quota, 42);
  assert.equal(loadAuth()?.apiKey, "new-key");
  assert.equal(loadAuth()?.apiKeyGroup, "new-key-group");
});
