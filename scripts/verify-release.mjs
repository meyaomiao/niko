// 发布产物可用性审计。发布前对本机 release-artifacts/ 跑一次，发布后对线上 tag 再跑一次。
//
//   node scripts/verify-release.mjs --dir release-artifacts
//   node scripts/verify-release.mjs --tag niko-v0.1.4
//
// 检查项：附件是否齐全、SHA256SUMS 是否可被 sha256sum -c 校验且与实际字节一致、
// latest.json 的版本与平台键是否能被 updater 找到、macOS 更新包的 tar 布局是否可解压。

import { createHash } from "node:crypto";
import { readdir, readFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { gunzipSync } from "node:zlib";

import {
  MAC_UPDATER_ARCHIVE,
  auditAssetList,
  auditChecksumFile,
  auditMacUpdaterArchive,
  auditUpdaterManifest,
  auditUpdaterSignature,
  expectedAssets,
  hasErrors,
} from "./release/audit.mjs";

const REPO = "meyaomiao/niko";

function parseArgs(argv) {
  const options = { dir: null, tag: null };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--dir" || arg === "--tag") {
      const value = argv[i + 1];
      if (!value) {
        throw new Error(`${arg} 需要一个参数`);
      }
      options[arg.slice(2)] = value;
      i += 1;
      continue;
    }
    throw new Error(`无法识别的参数：${arg}`);
  }
  if (Boolean(options.dir) === Boolean(options.tag)) {
    throw new Error("请二选一：--dir <本机产物目录> 或 --tag <已发布的 tag>");
  }
  return options;
}

/** 从 tar 字节流里读出条目名，够用即可：只解析 500 字节头部的 name/size/typeflag。 */
function listTarEntries(buffer) {
  const names = [];
  let offset = 0;
  let longName = null;
  while (offset + 512 <= buffer.length) {
    const header = buffer.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) {
      break;
    }
    const readString = (start, length) => {
      const raw = header.subarray(start, start + length);
      const end = raw.indexOf(0);
      return raw.subarray(0, end === -1 ? raw.length : end).toString("utf8");
    };
    const size = Number.parseInt(readString(124, 12).trim() || "0", 8) || 0;
    const typeflag = String.fromCharCode(header[156]);
    const prefix = readString(345, 155);
    const name = readString(0, 100);
    const body = buffer.subarray(offset + 512, offset + 512 + size);

    if (typeflag === "L") {
      // GNU 长文件名：名字放在下一个条目的数据块里
      longName = body.toString("utf8").replace(/\0+$/, "");
    } else if (typeflag === "x" || typeflag === "g") {
      // pax 扩展头，跳过；真实条目紧随其后
    } else {
      names.push(longName ?? (prefix ? `${prefix}/${name}` : name));
      longName = null;
    }

    offset += 512 + Math.ceil(size / 512) * 512;
  }
  return names;
}

function sha256(buffer) {
  return createHash("sha256").update(buffer).digest("hex");
}

async function loadFromDir(dir) {
  const root = resolve(dir);
  const names = (await readdir(root, { withFileTypes: true }))
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name);
  const files = new Map();
  for (const name of names) {
    files.set(name, await readFile(join(root, name)));
  }
  return { source: root, names, read: (name) => files.get(name) ?? null };
}

async function loadFromTag(tag) {
  const response = await fetch(`https://api.github.com/repos/${REPO}/releases/tags/${tag}`, {
    headers: { accept: "application/vnd.github+json" },
  });
  if (!response.ok) {
    throw new Error(`读取 ${tag} 的发布信息失败：HTTP ${response.status}`);
  }
  const release = await response.json();
  const names = release.assets.map((asset) => asset.name);
  const urls = new Map(release.assets.map((asset) => [asset.name, asset.browser_download_url]));
  const cache = new Map();
  return {
    source: `${REPO}@${tag}`,
    names,
    read: async (name) => {
      if (cache.has(name)) {
        return cache.get(name);
      }
      const url = urls.get(name);
      if (!url) {
        return null;
      }
      const asset = await fetch(url);
      if (!asset.ok) {
        throw new Error(`下载 ${name} 失败：HTTP ${asset.status}`);
      }
      const buffer = Buffer.from(await asset.arrayBuffer());
      cache.set(name, buffer);
      return buffer;
    },
  };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const config = JSON.parse(await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  const version = config.version;
  const tag = options.tag ?? `niko-v${version}`;
  const source = options.dir ? await loadFromDir(options.dir) : await loadFromTag(tag);

  if (options.tag && options.tag !== `niko-v${version}`) {
    console.log(`提示：审计的是 ${options.tag}，仓库当前版本为 ${version}`);
  }
  const auditedVersion = tag.replace(/^niko-v/, "");
  console.log(`审计对象：${source.source}（版本 ${auditedVersion}）\n`);

  const issues = [...auditAssetList(source.names, auditedVersion)];

  const wanted = expectedAssets(auditedVersion).filter((name) => source.names.includes(name));
  const digests = new Map();
  const signatures = new Map();
  for (const name of wanted) {
    const buffer = await source.read(name);
    if (!buffer) {
      continue;
    }
    digests.set(name, sha256(buffer));
    if (name.endsWith(".sig")) {
      signatures.set(name, buffer.toString("utf8"));
    }
  }

  // 只要求覆盖用户会下载安装的文件本体；.sig 由 updater 自己核对，不需要人工校验。
  const macAssets = [`Niko_${auditedVersion}_universal.dmg`, MAC_UPDATER_ARCHIVE];
  const winAssets = [
    `Niko_${auditedVersion}_x64-setup.exe`,
    `Niko_${auditedVersion}_x64_en-US.msi`,
  ];
  for (const [filename, coveredAssets] of [
    ["SHA256SUMS.txt", macAssets],
    ["SHA256SUMS_win.txt", winAssets],
  ]) {
    const buffer = await source.read(filename);
    if (!buffer) {
      continue;
    }
    issues.push(
      ...auditChecksumFile({
        filename,
        text: buffer.toString("utf8").replace(/^\uFEFF/, ""),
        coveredAssets: coveredAssets.filter((name) => source.names.includes(name)),
        actualDigests: digests,
      }),
    );
  }

  for (const [name, signatureText] of signatures) {
    const signed = name.replace(/\.sig$/, "");
    const data = await source.read(signed);
    if (!data) {
      continue;
    }
    issues.push(
      ...auditUpdaterSignature({
        filename: signed,
        data,
        signatureText,
        pubkeyBase64: config.plugins.updater.pubkey,
      }),
    );
  }

  const manifestBuffer = await source.read("latest.json");
  if (manifestBuffer) {
    issues.push(
      ...auditUpdaterManifest({
        manifest: JSON.parse(manifestBuffer.toString("utf8")),
        version: auditedVersion,
        tag,
        assetNames: source.names,
        signatures,
      }),
    );
  }

  const archive = await source.read(MAC_UPDATER_ARCHIVE);
  if (archive) {
    issues.push(...auditMacUpdaterArchive(listTarEntries(gunzipSync(archive))));
  }

  for (const item of issues) {
    console.log(`${item.level === "error" ? "[错误]" : "[提醒]"} ${item.message}`);
  }

  if (hasErrors(issues)) {
    console.log(`\n审计未通过：${issues.filter((item) => item.level === "error").length} 项错误`);
    process.exitCode = 1;
    return;
  }
  console.log(issues.length === 0 ? "审计通过：未发现问题" : "\n审计通过：无阻塞问题");
}

main().catch((error) => {
  console.error(`审计中断：${error.message}`);
  process.exitCode = 1;
});
