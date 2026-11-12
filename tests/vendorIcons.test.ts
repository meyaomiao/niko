import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { build } from "esbuild";
import { resolveVendorIcon, VENDOR_ICON_ASSETS } from "../src/lib/vendorIcons.ts";

// Public MomoToken pricing catalog: all 19 vendors, including the unverified QiHang mark.
const liveVendors = [
  ["302.AI", null, "ai302"],
  ["QiHang", null, null],
  ["Google", "Gemini.Color", "gemini"],
  ["NanoGPT", null, "nanogpt"],
  ["Vertex", "VertexAI.Color", "vertexai"],
  ["OpenCode Zen", "OpenCode", "opencode"],
  ["DeepSeek", "DeepSeek.Color", "deepseek"],
  ["xAI", "XAI", "xai"],
  ["MiniMax", "Minimax.Color", "minimax"],
  ["Anthropic", "Claude.Color", "claude"],
  ["OpenAI", "OpenAI", "openai"],
  ["讯飞", "Spark.Color", "spark"],
  ["SAP AI Core", null, "sap"],
  ["智谱", "Zhipu.Color", "zhipu"],
  ["Moonshot", "Moonshot", "moonshot"],
  ["Z.AI", "ZAI", "zai"],
  ["Moonshot AI", "Moonshot", "moonshot"],
  ["字节跳动", "Doubao.Color", "doubao"],
  ["阿里巴巴", "Qwen.Color", "qwen"],
] as const;

test("all 19 current vendors resolve to reviewed local assets or explicit unknown", () => {
  assert.equal(liveVendors.length, 19);
  for (const [vendor, icon, expected] of liveVendors) {
    const result = resolveVendorIcon(vendor, icon);
    assert.equal(result?.id ?? null, expected, vendor);
    if (result) {
      assert.match(result.src, /^\/vendor-icons\/[a-z0-9-]+\.(?:svg|png)$/);
      assert.ok(readFileSync(new URL(`../public${result.src}`, import.meta.url)).length > 0);
    }
  }
});

test("known server icon identifiers take precedence over brand names", () => {
  assert.equal(resolveVendorIcon("Google")?.id, "google");
  assert.equal(resolveVendorIcon("Google", "Gemini.Color")?.id, "gemini");
  assert.equal(resolveVendorIcon("Anthropic")?.id, "anthropic");
  assert.equal(resolveVendorIcon("Anthropic", "Claude.Color")?.id, "claude");
  assert.equal(resolveVendorIcon("OpenAI", " Claude.Color ")?.id, "claude");
  assert.equal(resolveVendorIcon("Unknown vendor", "vertexai.color")?.id, "vertexai");
  assert.equal(resolveVendorIcon("Google", "Unavailable.Icon")?.id, "google");
});

test("vendor aliases are exact and do not impersonate unverified brands", () => {
  for (const [name, id] of [
    ["Moonshot AI", "moonshot"], ["moonshot-ai", "moonshot"], ["Kimi", "moonshot"],
    ["Google Vertex AI", "vertexai"], [" openCode Zen ", "opencode"],
    ["Z.AI", "zai"], ["智谱AI", "zhipu"], ["iFlytek Spark", "spark"],
    ["ByteDance", "doubao"], ["Alibaba", "qwen"], ["通义千问", "qwen"],
    ["Nano-GPT", "nanogpt"], ["302AI", "ai302"],
  ]) assert.equal(resolveVendorIcon(name)?.id, id, name);
  for (const name of ["QiHang", "QuanHex", "Unknown", "OpenAI reseller", "not-openai", "constructor", "__proto__", ""]) {
    assert.equal(resolveVendorIcon(name), null, name);
  }
});

test("remote URLs, paths, markup and prototype names cannot become image sources", () => {
  for (const icon of [
    "https://example.invalid/logo.svg", "//example.invalid/logo.svg", "/vendor-icons/openai.svg",
    "file:///tmp/logo.svg", "data:image/svg+xml,<svg onload='alert(1)'/>", "javascript:alert(1)",
    "<svg><script>alert(1)</script></svg>", "../openai.svg", "OpenAI/../../logo.svg", "OpenAI#fragment",
    "constructor", "__proto__", "toString",
  ]) {
    assert.equal(resolveVendorIcon("Unknown", icon), null, icon);
    assert.equal(resolveVendorIcon("OpenAI", icon)?.src, "/vendor-icons/openai.svg", icon);
  }
});

test("vendored image files are bounded, inert and have documented sources", () => {
  const directory = new URL("../public/vendor-icons/", import.meta.url);
  const credits = readFileSync(new URL("CREDITS.md", directory), "utf8");
  const files = Object.values(VENDOR_ICON_ASSETS).map((asset) => asset.file);
  const images = readdirSync(directory).filter((name) => /\.(?:svg|png)$/.test(name));
  assert.deepEqual([...images].sort(), [...files].sort());
  const allowedTags = new Set(["svg", "title", "g", "path", "circle", "ellipse", "rect", "line", "polyline", "polygon", "defs", "linearGradient", "radialGradient", "stop", "clipPath", "mask"]);
  let total = 0;
  for (const file of files) {
    const bytes = readFileSync(new URL(file, directory));
    total += bytes.length;
    assert.ok(bytes.length < 64 * 1024, file);
    assert.ok(credits.includes(file), `Missing provenance for ${file}`);
    if (file.endsWith(".png")) {
      assert.equal(bytes.subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
      assert.ok(bytes.readUInt32BE(16) <= 256 && bytes.readUInt32BE(20) <= 256, file);
      continue;
    }
    const svg = bytes.toString("utf8");
    assert.match(svg, /<svg\b[^>]*\bviewBox=/);
    assert.doesNotMatch(svg, /<!DOCTYPE|<!ENTITY|\s(?:on[a-z]+|style|href|xlink:href|src)\s*=/i);
    for (const match of svg.matchAll(/<\/?([A-Za-z][\w:-]*)\b/g)) {
      assert.ok(allowedTags.has(match[1]), `${file}: unexpected ${match[1]}`);
    }
    const ids = new Set([...svg.matchAll(/\bid="([^"]+)"/g)].map((match) => match[1]));
    for (const match of svg.matchAll(/url\(([^)]+)\)/gi)) {
      assert.match(match[1], /^#[A-Za-z0-9_.:-]+$/);
      assert.ok(ids.has(match[1].slice(1)), `${file}: unresolved local reference`);
    }
  }
  assert.ok(total < 128 * 1024, "Do not vendor an entire icon library");
  assert.match(credits, /1\.102\.0/);
  assert.match(credits, /Copyright \(c\) 2023 LobeHub/);
});

let componentPromise: Promise<(props: { vendor: string; icon?: string | null; size?: number }) => any> | undefined;
function component() {
  componentPromise ??= build({
    entryPoints: [fileURLToPath(new URL("../src/components/VendorIcon.tsx", import.meta.url))],
    bundle: true, write: false, format: "cjs", platform: "node", packages: "external", jsx: "automatic", logLevel: "silent",
  }).then((result) => {
    const module = { exports: {} as { VendorIcon?: any } };
    new Function("require", "module", "exports", result.outputFiles[0].text)(createRequire(import.meta.url), module, module.exports);
    return module.exports.VendorIcon;
  });
  return componentPromise;
}

test("VendorIcon keeps decorative accessibility and size with a local image", async () => {
  const VendorIcon = await component();
  const html = renderToStaticMarkup(createElement(VendorIcon, { vendor: "Google", icon: "Gemini.Color", size: 28 }));
  assert.match(html, /aria-hidden="true"/);
  assert.match(html, /src="\/vendor-icons\/gemini-color\.svg"/);
  assert.match(html, /alt=""/);
  assert.match(html, /width:28px;height:28px/);
  const mono = renderToStaticMarkup(createElement(VendorIcon, { vendor: "OpenAI" }));
  assert.match(mono, /dark:invert/);
});

test("VendorIcon shows a clearly marked unknown fallback without remote image loading", async () => {
  const VendorIcon = await component();
  const html = renderToStaticMarkup(createElement(VendorIcon, { vendor: "QiHang", icon: "https://example.invalid/logo.svg" }));
  assert.match(html, /data-vendor-icon="fallback"/);
  assert.match(html, /暂无可核实品牌图标/);
  assert.match(html, /aria-hidden="true"/);
  assert.doesNotMatch(html, /<img|example\.invalid/);
});
