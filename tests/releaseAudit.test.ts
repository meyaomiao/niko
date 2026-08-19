import assert from "node:assert/strict";
import { createHash, generateKeyPairSync, sign } from "node:crypto";
import test from "node:test";
import {
  REQUIRED_UPDATER_PLATFORMS,
  auditAssetList,
  auditChecksumFile,
  auditMacUpdaterArchive,
  auditUpdaterManifest,
  auditUpdaterSignature,
  expectedAssets,
  hasErrors,
  parseChecksumFile,
} from "../scripts/release/audit.mjs";

const VERSION = "0.1.4";
const TAG = `niko-v${VERSION}`;
const DMG = `Niko_${VERSION}_universal.dmg`;
const EXE = `Niko_${VERSION}_x64-setup.exe`;
const DIGEST_DMG = "a".repeat(64);
const DIGEST_EXE = "b".repeat(64);

const cleanArchive = [
  "Niko.app/",
  "Niko.app/Contents/",
  "Niko.app/Contents/CodeResources",
  "Niko.app/Contents/Info.plist",
  "Niko.app/Contents/MacOS/niko",
  "Niko.app/Contents/_CodeSignature/CodeResources",
];

function codes(issues: { code: string }[]) {
  return issues.map((issue) => issue.code);
}

/** 造一把 minisign 密钥对，并按 minisign 的文件格式签名，用来测验签逻辑 */
function makeSigner(keyId = Buffer.from("0123456789abcdef", "hex")) {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const rawPublicKey = publicKey.export({ format: "der", type: "spki" }).subarray(-32);
  const pubkeyFile = [
    "untrusted comment: minisign public key",
    Buffer.concat([Buffer.from("Ed"), keyId, rawPublicKey]).toString("base64"),
    "",
  ].join("\n");

  return {
    pubkeyBase64: Buffer.from(pubkeyFile).toString("base64"),
    signature(data: Buffer, signedName: string) {
      const digest = createHash("blake2b512").update(data).digest();
      const signature = sign(null, digest, privateKey);
      const trustedComment = `timestamp:0\tfile:${signedName}`;
      const sigFile = [
        "untrusted comment: signature from tauri secret key",
        Buffer.concat([Buffer.from("ED"), keyId, signature]).toString("base64"),
        `trusted comment: ${trustedComment}`,
        sign(null, Buffer.concat([signature, Buffer.from(trustedComment)]), privateKey).toString(
          "base64",
        ),
        "",
      ].join("\n");
      return Buffer.from(sigFile).toString("base64");
    },
  };
}

test("a release is only complete when both platforms can be downloaded and updated", () => {
  assert.equal(auditAssetList(expectedAssets(VERSION), VERSION).length, 0);

  const withoutWindowsSignature = expectedAssets(VERSION).filter((name) => name !== `${EXE}.sig`);
  const issues = auditAssetList(withoutWindowsSignature, VERSION);
  assert.deepEqual(codes(issues), ["missing-asset"]);
  assert.ok(hasErrors(issues));
});

test("a single space after the digest is rejected because sha256sum -c skips that line", () => {
  const good = parseChecksumFile(`${DIGEST_DMG}  ${DMG}\n`);
  assert.equal(good.malformed.length, 0);
  assert.equal(good.entries.get(DMG), DIGEST_DMG);

  const bad = parseChecksumFile(`${DIGEST_DMG} ${DMG}\n`);
  assert.equal(bad.entries.size, 0);
  assert.deepEqual(
    bad.malformed.map((entry) => entry.line),
    [1],
  );
});

test("binary mode markers and uppercase digests stay valid", () => {
  const parsed = parseChecksumFile(`${DIGEST_EXE.toUpperCase()} *${EXE}\r\n\n`);
  assert.equal(parsed.malformed.length, 0);
  assert.equal(parsed.entries.get(EXE), DIGEST_EXE);
});

test("checksum files must cover every installer and match the real bytes", () => {
  const actualDigests = new Map([[DMG, DIGEST_DMG]]);

  assert.equal(
    auditChecksumFile({
      filename: "SHA256SUMS.txt",
      text: `${DIGEST_DMG}  ${DMG}\n`,
      coveredAssets: [DMG],
      actualDigests,
    }).length,
    0,
  );

  assert.deepEqual(
    codes(
      auditChecksumFile({
        filename: "SHA256SUMS.txt",
        text: "",
        coveredAssets: [DMG],
        actualDigests,
      }),
    ),
    ["checksum-missing"],
  );

  assert.deepEqual(
    codes(
      auditChecksumFile({
        filename: "SHA256SUMS.txt",
        text: `${DIGEST_EXE}  ${DMG}\n`,
        coveredAssets: [DMG],
        actualDigests,
      }),
    ),
    ["checksum-mismatch"],
  );

  // 单空格既是格式错误，也让该附件实际上没有被覆盖
  assert.deepEqual(
    codes(
      auditChecksumFile({
        filename: "SHA256SUMS.txt",
        text: `${DIGEST_DMG} ${DMG}\n`,
        coveredAssets: [DMG],
        actualDigests,
      }),
    ),
    ["checksum-format", "checksum-missing"],
  );
});

test("the updater manifest must use the OS-ARCH keys the plugin actually looks up", () => {
  const assetNames = expectedAssets(VERSION);
  const signatures = new Map([
    ["Niko.app.tar.gz.sig", "mac-signature\n"],
    [`${EXE}.sig`, "windows-signature\n"],
  ]);
  const url = (name: string) =>
    `https://github.com/meyaomiao/niko/releases/download/${TAG}/${name}`;

  const valid = {
    version: VERSION,
    platforms: {
      "darwin-aarch64": { url: url("Niko.app.tar.gz"), signature: "mac-signature" },
      "darwin-x86_64": { url: url("Niko.app.tar.gz"), signature: "mac-signature" },
      "windows-x86_64": { url: url(EXE), signature: "windows-signature" },
    },
  };
  assert.equal(
    auditUpdaterManifest({ manifest: valid, version: VERSION, tag: TAG, assetNames, signatures })
      .length,
    0,
  );
  assert.deepEqual(Object.keys(valid.platforms).sort(), [...REQUIRED_UPDATER_PLATFORMS].sort());

  const universalOnly = {
    version: VERSION,
    platforms: {
      "darwin-universal": { url: url("Niko.app.tar.gz"), signature: "mac-signature" },
      "windows-x86_64": { url: url(EXE), signature: "windows-signature" },
    },
  };
  assert.deepEqual(
    codes(
      auditUpdaterManifest({
        manifest: universalOnly,
        version: VERSION,
        tag: TAG,
        assetNames,
        signatures,
      }),
    ),
    ["manifest-platform", "manifest-platform", "manifest-unknown-platform"],
  );
});

test("the manifest is rejected when version, url or signature drift from the attachments", () => {
  const assetNames = expectedAssets(VERSION);
  const signatures = new Map([["Niko.app.tar.gz.sig", "mac-signature\n"]]);
  const manifest = {
    version: "0.1.3",
    platforms: {
      "darwin-aarch64": {
        url: `https://github.com/meyaomiao/niko/releases/download/niko-v0.1.3/Niko.app.tar.gz`,
        signature: "mac-signature",
      },
      "darwin-x86_64": {
        url: `https://github.com/meyaomiao/niko/releases/download/${TAG}/Niko.app.tar.gz`,
        signature: "stale-signature",
      },
      "windows-x86_64": {
        url: `https://github.com/meyaomiao/niko/releases/download/${TAG}/Niko_0.1.3_x64-setup.exe`,
        signature: "windows-signature",
      },
    },
  };
  assert.deepEqual(
    codes(auditUpdaterManifest({ manifest, version: VERSION, tag: TAG, assetNames, signatures })),
    ["manifest-version", "manifest-url", "manifest-signature", "manifest-url"],
  );
});

test("the macOS updater archive must unpack under the plugin's skip-first-segment rule", () => {
  assert.equal(auditMacUpdaterArchive(cleanArchive).length, 0);

  // macOS 自带 tar 写出的 AppleDouble 条目里，顶层那一条剥掉第一段后是空路径
  const withAppleDouble = ["._Niko.app", ...cleanArchive, "Niko.app/Contents/._Info.plist"];
  const issues = auditMacUpdaterArchive(withAppleDouble);
  assert.deepEqual(codes(issues), ["archive-appledouble", "archive-root"]);
  assert.match(issues[0].message, /COPYFILE_DISABLE=1/);
});

test("updater signatures are checked against the public key shipped in the app", () => {
  const signer = makeSigner();
  const data = Buffer.from("installer bytes");

  assert.equal(
    auditUpdaterSignature({
      filename: EXE,
      data,
      signatureText: signer.signature(data, EXE),
      pubkeyBase64: signer.pubkeyBase64,
    }).length,
    0,
  );

  assert.deepEqual(
    codes(
      auditUpdaterSignature({
        filename: EXE,
        data: Buffer.from("tampered bytes"),
        signatureText: signer.signature(data, EXE),
        pubkeyBase64: signer.pubkeyBase64,
      }),
    ),
    ["signature-invalid"],
  );

  // 把另一个附件的 .sig 复制过来：签名本身有效，但签的是别的文件
  assert.deepEqual(
    codes(
      auditUpdaterSignature({
        filename: DMG,
        data,
        signatureText: signer.signature(data, EXE),
        pubkeyBase64: signer.pubkeyBase64,
      }),
    ),
    ["signature-filename"],
  );
});

test("a signature from a different key is reported instead of a bare invalid signature", () => {
  const data = Buffer.from("installer bytes");
  const other = makeSigner(Buffer.from("fedcba9876543210", "hex"));
  const issues = auditUpdaterSignature({
    filename: EXE,
    data,
    signatureText: other.signature(data, EXE),
    pubkeyBase64: makeSigner().pubkeyBase64,
  });
  assert.deepEqual(codes(issues), ["signature-key"]);
});

test("an unsigned or unstapled app bundle is reported separately", () => {
  const unsigned = cleanArchive.filter(
    (name) => name !== "Niko.app/Contents/_CodeSignature/CodeResources",
  );
  assert.deepEqual(codes(auditMacUpdaterArchive(unsigned)), ["archive-signature"]);

  const unstapled = cleanArchive.filter((name) => name !== "Niko.app/Contents/CodeResources");
  const issues = auditMacUpdaterArchive(unstapled);
  assert.deepEqual(codes(issues), ["archive-staple"]);
  assert.equal(hasErrors(issues), false);
});
