// 发布产物审计规则。不做 IO，供 scripts/verify-release.mjs 和 tests/releaseAudit.test.ts 复用。

import { createHash, createPublicKey, verify } from "node:crypto";

// tauri-plugin-updater 按 `{os}-{arch}` 查找平台键，arch 取自运行分片的编译期架构。
// universal 不是合法 arch，macOS universal 包必须同时挂在两个架构键上。
export const REQUIRED_UPDATER_PLATFORMS = ["darwin-aarch64", "darwin-x86_64", "windows-x86_64"];

export const MAC_UPDATER_ARCHIVE = "Niko.app.tar.gz";

/** 一次正式发布必须齐全的附件；缺任意一项都会让某个平台的下载或更新断链。 */
export function expectedAssets(version) {
  return [
    `Niko_${version}_universal.dmg`,
    MAC_UPDATER_ARCHIVE,
    `${MAC_UPDATER_ARCHIVE}.sig`,
    `Niko_${version}_x64-setup.exe`,
    `Niko_${version}_x64-setup.exe.sig`,
    `Niko_${version}_x64_en-US.msi`,
    `Niko_${version}_x64_en-US.msi.sig`,
    "SHA256SUMS.txt",
    "SHA256SUMS_win.txt",
    "latest.json",
  ];
}

function issue(level, code, message) {
  return { level, code, message };
}

export function hasErrors(issues) {
  return issues.some((item) => item.level === "error");
}

export function auditAssetList(assetNames, version) {
  const present = new Set(assetNames);
  return expectedAssets(version)
    .filter((name) => !present.has(name))
    .map((name) => issue("error", "missing-asset", `发布缺少附件 ${name}`));
}

/**
 * 解析 sha256sum 风格的校验和文件。
 * 哈希后必须是两个字符（空格加空格，或空格加星号）。只写一个空格时，
 * GNU sha256sum 和 macOS 自带的 shasum 都会跳过该行、只提示一句 warning 并仍以 0 退出，
 * 用户会误以为整个文件都校验通过，所以这里把单空格当作格式错误。
 * 大小写十六进制两种工具都接受，统一转成小写比较。
 */
export function parseChecksumFile(text) {
  const entries = new Map();
  const malformed = [];
  text.split("\n").forEach((raw, index) => {
    const line = raw.replace(/\r$/, "");
    if (line.trim() === "") {
      return;
    }
    const strict = /^([0-9a-fA-F]{64}) [ *](\S.*)$/.exec(line);
    if (strict) {
      entries.set(strict[2], strict[1].toLowerCase());
      return;
    }
    malformed.push({ line: index + 1, text: line });
  });
  return { entries, malformed };
}

/**
 * 校验一个校验和文件：格式是否被 sha256sum -c 接受、是否覆盖了该文件应覆盖的附件、
 * 哈希是否与实际字节一致。actualDigests 是「附件名 -> 小写 sha256」。
 */
export function auditChecksumFile({ filename, text, coveredAssets, actualDigests }) {
  const issues = [];
  const { entries, malformed } = parseChecksumFile(text);

  for (const bad of malformed) {
    issues.push(
      issue(
        "error",
        "checksum-format",
        `${filename} 第 ${bad.line} 行不是 sha256sum 可校验的格式，` +
          `sha256sum -c 会跳过该行并仍然以 0 退出：${bad.text}`,
      ),
    );
  }

  for (const asset of coveredAssets) {
    const declared = entries.get(asset);
    if (!declared) {
      issues.push(issue("error", "checksum-missing", `${filename} 没有覆盖 ${asset}`));
      continue;
    }
    const actual = actualDigests.get(asset);
    if (!actual) {
      continue;
    }
    if (declared !== actual.toLowerCase()) {
      issues.push(
        issue("error", "checksum-mismatch", `${filename} 中 ${asset} 的哈希与实际文件不一致`),
      );
    }
  }

  return issues;
}

/**
 * 校验 updater 静态清单：版本、平台键、下载地址、签名是否与本次附件一致。
 * signatures 是「附件名 -> .sig 文件内容」。
 */
export function auditUpdaterManifest({ manifest, version, tag, assetNames, signatures }) {
  const issues = [];

  if (manifest.version !== version) {
    issues.push(
      issue(
        "error",
        "manifest-version",
        `latest.json 的 version 是 ${manifest.version}，应为 ${version}`,
      ),
    );
  }

  const platforms = manifest.platforms ?? {};
  for (const key of REQUIRED_UPDATER_PLATFORMS) {
    if (!platforms[key]) {
      issues.push(
        issue(
          "error",
          "manifest-platform",
          `latest.json 缺少平台键 ${key}，该平台的应用内更新会直接报错`,
        ),
      );
    }
  }

  for (const key of Object.keys(platforms)) {
    if (!REQUIRED_UPDATER_PLATFORMS.includes(key)) {
      issues.push(
        issue(
          "error",
          "manifest-unknown-platform",
          `latest.json 含无效平台键 ${key}；updater 只会查找 ${REQUIRED_UPDATER_PLATFORMS.join("、")}`,
        ),
      );
    }
  }

  const present = new Set(assetNames);
  for (const [key, entry] of Object.entries(platforms)) {
    const url = entry?.url ?? "";
    const filename = url.split("/").pop() ?? "";
    if (!url.startsWith(`https://github.com/meyaomiao/niko/releases/download/${tag}/`)) {
      issues.push(
        issue("error", "manifest-url", `latest.json 中 ${key} 的下载地址不指向本次发布 ${tag}：${url}`),
      );
    } else if (!present.has(filename)) {
      issues.push(
        issue("error", "manifest-url", `latest.json 中 ${key} 指向的附件 ${filename} 不存在`),
      );
    }

    const signature = (entry?.signature ?? "").trim();
    if (signature === "") {
      issues.push(issue("error", "manifest-signature", `latest.json 中 ${key} 的签名为空`));
      continue;
    }
    const expected = signatures.get(`${filename}.sig`);
    if (expected && signature !== expected.trim()) {
      issues.push(
        issue(
          "error",
          "manifest-signature",
          `latest.json 中 ${key} 的签名与 ${filename}.sig 内容不一致`,
        ),
      );
    }
  }

  return issues;
}

// Ed25519 裸公钥转 SPKI DER 的固定前缀
const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function parseMinisignPublicKey(pubkeyBase64) {
  const raw = Buffer.from(Buffer.from(pubkeyBase64, "base64").toString("utf8").split("\n")[1], "base64");
  return {
    keyId: raw.subarray(2, 10),
    key: createPublicKey({
      key: Buffer.concat([ED25519_SPKI_PREFIX, raw.subarray(10)]),
      format: "der",
      type: "spki",
    }),
  };
}

function parseMinisignSignature(signatureText) {
  const lines = Buffer.from(signatureText, "base64").toString("utf8").split("\n");
  const raw = Buffer.from(lines[1], "base64");
  return {
    algorithm: raw.subarray(0, 2).toString("utf8"),
    keyId: raw.subarray(2, 10),
    signature: raw.subarray(10),
    trustedComment: lines[2].replace("trusted comment: ", ""),
    globalSignature: Buffer.from(lines[3], "base64"),
  };
}

/**
 * 用 tauri.conf.json 里的公钥验证一个 updater 附件的 minisign 签名。
 * 除了签名本身，还核对 keyId 和 trusted comment 里的 `file:` 字段，
 * 这样把别的附件的 .sig 复制过来也会被发现。
 */
export function auditUpdaterSignature({ filename, data, signatureText, pubkeyBase64 }) {
  const issues = [];
  let parsed;
  let pubkey;
  try {
    pubkey = parseMinisignPublicKey(pubkeyBase64);
    parsed = parseMinisignSignature(signatureText);
  } catch (error) {
    return [issue("error", "signature-parse", `${filename}.sig 无法解析：${error.message}`)];
  }

  if (!parsed.keyId.equals(pubkey.keyId)) {
    issues.push(
      issue(
        "error",
        "signature-key",
        `${filename}.sig 由 ${parsed.keyId.toString("hex")} 签发，` +
          `与 tauri.conf.json 中的公钥 ${pubkey.keyId.toString("hex")} 不是同一把钥匙`,
      ),
    );
    return issues;
  }

  const message =
    parsed.algorithm === "ED" ? createHash("blake2b512").update(data).digest() : data;
  if (!verify(null, message, pubkey.key, parsed.signature)) {
    issues.push(
      issue("error", "signature-invalid", `${filename} 的签名验不过，附件与 .sig 不是同一次构建`),
    );
  }
  if (
    !verify(
      null,
      Buffer.concat([parsed.signature, Buffer.from(parsed.trustedComment, "utf8")]),
      pubkey.key,
      parsed.globalSignature,
    )
  ) {
    issues.push(issue("error", "signature-invalid", `${filename}.sig 的 trusted comment 被改动过`));
  }

  const signedName = /file:([^\t]*)/.exec(parsed.trustedComment)?.[1];
  if (signedName && signedName !== filename) {
    issues.push(
      issue("error", "signature-filename", `${filename}.sig 实际签的是 ${signedName}`),
    );
  }

  return issues;
}

/**
 * 校验 macOS 更新包的 tar 布局。
 *
 * updater 解压时对每个条目取 `path.iter().skip(1)`，即固定剥掉第一段路径。
 * macOS 自带的 tar 会额外写入 AppleDouble 条目（`._Niko.app` 等），其中顶层那一条
 * 剥掉后是空路径，解包目标变成临时目录本身，导致整次安装在第一个条目就失败。
 * tauri-bundler 生成的 tar.gz 不含这些条目，手工打包时必须设置 COPYFILE_DISABLE=1。
 */
export function auditMacUpdaterArchive(rawEntryNames) {
  const issues = [];
  const entryNames = rawEntryNames
    .map((name) => name.replace(/^\.\//, "").replace(/\/+$/, ""))
    .filter((name) => name !== "");
  const appleDouble = entryNames.filter((name) =>
    name.split("/").some((segment) => segment.startsWith("._")),
  );
  if (appleDouble.length > 0) {
    issues.push(
      issue(
        "error",
        "archive-appledouble",
        `${MAC_UPDATER_ARCHIVE} 含 ${appleDouble.length} 个 AppleDouble 条目（如 ${appleDouble[0]}）；` +
          "updater 解压这类条目时会失败，重新打包前请设置 COPYFILE_DISABLE=1",
      ),
    );
  }

  const roots = new Set(entryNames.map((name) => name.split("/")[0]));
  if (roots.size !== 1 || !roots.has("Niko.app")) {
    issues.push(
      issue(
        "error",
        "archive-root",
        `${MAC_UPDATER_ARCHIVE} 的顶层应只有 Niko.app/，实际为 ${[...roots].join("、")}`,
      ),
    );
  }

  for (const required of ["Niko.app/Contents/Info.plist", "Niko.app/Contents/MacOS/niko"]) {
    if (!entryNames.includes(required)) {
      issues.push(issue("error", "archive-content", `${MAC_UPDATER_ARCHIVE} 缺少 ${required}`));
    }
  }

  if (!entryNames.includes("Niko.app/Contents/_CodeSignature/CodeResources")) {
    issues.push(
      issue("error", "archive-signature", `${MAC_UPDATER_ARCHIVE} 里的 Niko.app 没有代码签名`),
    );
  }

  if (!entryNames.includes("Niko.app/Contents/CodeResources")) {
    issues.push(
      issue(
        "warn",
        "archive-staple",
        `${MAC_UPDATER_ARCHIVE} 里的 Niko.app 没有装订公证票据，离线首次启动会被 Gatekeeper 拦下`,
      ),
    );
  }

  return issues;
}
