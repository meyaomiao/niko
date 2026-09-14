// 渲染冒烟测试：真正执行 Home / Models 的渲染期代码（含所有 useMemo）。
// 目的：捕获「变量在初始化前被访问」这类只在运行时炸、tsc 抓不到的空白页错误。
// 只跑渲染阶段，不跑 useEffect，因此不需要真正的 Tauri / 网络环境。

import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { build } from "esbuild";
import { fileURLToPath, pathToFileURL } from "node:url";
import { join } from "node:path";
import { mkdir } from "node:fs/promises";

/** 把页面打包成 Node 可 import 的 ESM（测试运行器不认 .tsx） */
async function loadPage(entry: string) {
  // 输出到仓库内的缓存目录，保证裸导入（react 等）能被解析
  const outDir = fileURLToPath(new URL("../node_modules/.niko-smoke/", import.meta.url));
  await mkdir(outDir, { recursive: true });
  const outfile = join(outDir, `${entry.replace(/\W/g, "_")}.mjs`);
  await build({
    entryPoints: [fileURLToPath(new URL(`../src/pages/${entry}`, import.meta.url))],
    outfile,
    bundle: true,
    format: "esm",
    platform: "node",
    jsx: "automatic",
    packages: "external",
    loader: { ".svg": "text", ".png": "dataurl", ".jpg": "dataurl", ".webp": "dataurl" },
    logLevel: "silent",
  });
  return import(pathToFileURL(outfile).href);
}

function installBrowserStubs() {
  const store = new Map<string, string>();
  const storage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, String(v)),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
    key: (i: number) => [...store.keys()][i] ?? null,
    get length() {
      return store.size;
    },
  };
  const g = globalThis as unknown as Record<string, unknown>;
  g.localStorage = storage;
  g.sessionStorage = storage;
  g.window = {
    localStorage: storage,
    sessionStorage: storage,
    matchMedia: () => ({
      matches: false,
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
    }),
    location: { href: "http://localhost/", hash: "" },
  };
  g.matchMedia = (globalThis as unknown as { window: { matchMedia: unknown } }).window.matchMedia;
  const el = () => ({
    style: {},
    dataset: {},
    classList: { add() {}, remove() {}, toggle() {}, contains: () => false },
    setAttribute() {},
    removeAttribute() {},
    getAttribute: () => null,
    appendChild() {},
    removeChild() {},
    addEventListener() {},
    removeEventListener() {},
    querySelector: () => null,
    querySelectorAll: () => [],
    innerHTML: "",
    textContent: "",
  });
  g.document = {
    documentElement: el(),
    body: el(),
    head: el(),
    createElement: el,
    createTextNode: el,
    querySelector: () => null,
    querySelectorAll: () => [],
    addEventListener() {},
    removeEventListener() {},
    getElementById: () => null,
  };

  // Node 24 的 navigator 只有 getter，用 defineProperty 覆盖
  Object.defineProperty(globalThis, "navigator", {
    value: { userAgent: "node", language: "zh-CN", platform: "MacIntel" },
    configurable: true,
  });
}

installBrowserStubs();

test("Home 渲染期不抛错（TDZ / 未初始化变量会被抓出来）", async () => {
  const { default: Home } = await loadPage("Home.tsx");
  const html = renderToString(
    createElement(MemoryRouter, null, createElement(Home))
  );
  assert.ok(html.length > 0, "Home 渲染出了内容");
  // 渲染期若抛错会被 ErrorBoundary 之外的 renderToString 直接抛出，测试即失败
});

test("Models 渲染期不抛错", async () => {
  const { default: Models } = await loadPage("Models.tsx");
  const html = renderToString(
    createElement(MemoryRouter, null, createElement(Models))
  );
  assert.ok(html.includes("模型") || html.length > 0, "Models 渲染出了内容");
});

test("未登录时 Home 不炸（auth 为空的分支）", async () => {
  const { default: Home } = await loadPage("Home.tsx");
  const html = renderToString(
    createElement(MemoryRouter, null, createElement(Home))
  );
  assert.equal(typeof html, "string");
});
