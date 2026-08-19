# 安装包可用性审计（niko-v0.1.4）

审计对象是 GitHub Releases 上的最新正式发布 `niko-v0.1.4`（2026-08-07 发布），
以及仓库当前 `main` 中与发布相关的配置、文案和流程文档。

结论：**两个平台的手工下载安装都可用**，安装包本体、签名和校验和都能对上；
但 **macOS 的应用内更新完全不可用**，且 `SHA256SUMS.txt` 的格式缺陷会让 macOS 用户
以为自己校验过了 dmg，实际并没有。

复现全部检查项：

```bash
npm run verify:release -- --tag niko-v0.1.4
```

## 一、通过的检查

| 检查项 | 结果 |
| --- | --- |
| 版本一致性 | `package.json`、`src-tauri/tauri.conf.json`、`Cargo.toml`、`Cargo.lock` 均为 `0.1.4`，与 tag `niko-v0.1.4` 一致 |
| 附件完整性 | dmg、`Niko.app.tar.gz`、msi、`_x64-setup.exe`、四个 `.sig`、两个 `SHA256SUMS`、`latest.json` 全部存在且 `state=uploaded` |
| 下载可达性 | 三个安装包、`latest.json` 以及 `releases/latest/download/...` 这个 updater 端点均可下载 |
| 字节一致性 | 实际下载到的字节的 SHA256 与 `SHA256SUMS*.txt` 和 GitHub 记录的摘要三方一致 |
| macOS 架构 | 主二进制是 `x86_64 + arm64` 的 universal binary，与「支持 Apple Silicon 和 Intel」的说明相符 |
| macOS 签名 | 两个分片都由 `Developer ID Application: zhibang xiao (3R5RTZSF89)` 签名，启用了 hardened runtime，含安全时间戳，证书 2031-07 到期 |
| macOS 公证 | `Niko.app/Contents/CodeResources` 与 dmg 尾部都存在公证票据，公证与装订已完成 |
| updater 签名 | 三个 `.sig` 都能用 `tauri.conf.json` 里的公钥验签通过，`latest.json` 内嵌签名与 `.sig` 文件逐字一致 |
| Windows 签名状态 | msi 与 exe 均无 Authenticode 签名，与 README「当前发布包暂未做代码签名」的说明一致 |
| 官网下载入口 | `https://niko-ai.cc/` 可访问，页面上的版本号、三个下载链接和 Release Notes 链接全部指向 `niko-v0.1.4` |

## 二、发现的问题

### 1. macOS 应用内更新不可用（高）

`latest.json` 的平台键是 `darwin-universal`：

```json
"platforms": { "darwin-universal": { … }, "windows-x86_64": { … } }
```

但 `tauri-plugin-updater` 2.10.1 按 `{os}-{arch}` 组合查找，`arch` 取自运行分片的编译期架构，
合法取值只有 `x86_64`、`aarch64`、`i686`、`armv7`，**没有 `universal`**。
Apple Silicon 上实际查找 `darwin-aarch64-app`、`darwin-aarch64`，Intel 上查找
`darwin-x86_64-app`、`darwin-x86_64`，都落空后返回 `TargetsNotFound`。

这个查找发生在 `check()` 内部、版本比较之前，所以 macOS 用户点设置页的「检查更新」
永远只会看到「检查失败」，即使当前已是最新版本也一样。Windows 侧的 `windows-x86_64`
命中正常，不受影响。

`niko-v0.1.2` 和 `niko-v0.1.3` 的 `latest.json` 用的也是 `darwin-universal`，
说明 macOS 的应用内更新从未生效过。

修复方式是把同一个 `.app.tar.gz` 同时挂在 `darwin-aarch64` 和 `darwin-x86_64` 两个键上，
签名沿用同一份。`latest.json` 是 Release 附件，重新上传即可，
已经装了 0.1.4 的 macOS 用户不需要重装就能恢复更新检查。

### 2. macOS 更新包解压即失败（高）

`niko-v0.1.4` 的 `Niko.app.tar.gz` 是用 macOS 自带的 `tar` 手工打的，
带 `LIBARCHIVE.xattr.*` 扩展头和 10 个 AppleDouble 条目，第一个条目就是 `._Niko.app`。

updater 解压时对每个条目固定剥掉第一段路径（`path.iter().skip(1)`）。
`._Niko.app` 只有一段，剥掉后是空路径，解包目标退化成临时目录本身，
于是安装在第一个条目就中止。用真实产物跑通这段提取逻辑可以复现：

```text
ERROR "._Niko.app" -> ""
      updater would abort install with: failed to unpack `._Niko.app` into `…/tauri_updated_app/`
```

`niko-v0.1.2` 和 `niko-v0.1.3` 的同名附件都不含 AppleDouble 条目，能正常解压，
所以这是 0.1.4 独有的打包回归。`tauri build` 自己生成的 tar 不会带这些条目，
手工打包时需要 `COPYFILE_DISABLE=1`。

即便问题 1 修好，这个包也装不上，两处都得改。

### 3. `SHA256SUMS.txt` 让 dmg 校验静默失效（中）

文件里 dmg 那一行用的是一个空格，其余行是两个空格：

```text
9bae4630…9b3b  Niko.app.tar.gz
6f9e775c…7884 Niko_0.1.4_universal.dmg
```

GNU `sha256sum -c` 和 macOS 自带的 `shasum -a 256 -c` 都会跳过这一行、
只打印一句 warning，并且**仍然以 0 退出**：

```text
Niko.app.tar.gz: OK
Niko.app.tar.gz.sig: OK
latest.json: OK
shasum: WARNING: 1 line is improperly formatted
shasum exit=0
```

于是按安装指引校验的用户会看到几行 OK 和 0 退出码，而唯一真正要装的那个 dmg
根本没有被校验。哈希值本身是对的，只是格式让它没有生效。

### 4. 应用内安装指引与实际产物不符（中）

`src/pages/InstallGuide.tsx` 的 macOS 部分描述的是一个未签名的旧版本：

- 让用户下载 `Niko_x.y.z_aarch64.dmg` 或 `x64.dmg`，而 Release 里只有 `Niko_<版本>_universal.dmg`，两个文件名都不存在。
- 用三步篇幅讲「无法验证开发者」弹窗、右键打开、到系统设置里放行，而 0.1.4 已签名并公证，正常安装不会出现这些提示。
- 顶部说「本应用目前没有购买代码签名证书」，与 README 中「发布包已签名并公证」矛盾。
- 顶部说「所有版本都在 GitHub Actions 公开构建」，而 `docs/release-process.md` 明确规定 macOS 在发布者本机构建，Actions 只产出 Windows 产物。
- Windows 部分让用户「对照页面公示的 SHA256」，但应用内并没有公示校验和。

### 5. macOS 最低系统版本与文档不一致（低）

`Info.plist` 里 `LSMinimumSystemVersion` 是 `10.13`（Tauri 默认值），
而 README 和官网都写「macOS 12 或更高版本」。`tauri.conf.json` 之前没有设置
`bundle.macOS.minimumSystemVersion`，结果 10.13–11 的用户能装上一个不受支持的版本，
拿不到「需要 macOS 12」这样的明确提示。

### 6. 发布产物没有自动校验（中）

`SHA256SUMS.txt` 和 `latest.json` 都在发布者本机手工拼装，仓库里没有生成脚本也没有校验脚本，
`release.yml` 只校验 Windows 侧「恰好一个 MSI 和一个 EXE、`.sig` 非空」。
上面 1、2、3 三个问题都属于手工拼装出错且无人复核，靠流程文档提醒不足以拦住。

另外 `SHA256SUMS_win.txt` 在 Release 中是小写哈希，而 `release.yml` 里 `Get-FileHash`
输出的是大写，说明发布时这个文件被重新生成过，上传的内容并非 CI 产物本身。

## 三、本次提交做的事

- 新增 `scripts/verify-release.mjs` 与 `scripts/release/audit.mjs`，可对本机产物目录或线上 tag 跑一遍上述检查，发现错误则非零退出；规则部分由 `tests/releaseAudit.test.ts` 覆盖。
- `docs/release-process.md` 补上 updater 平台键的正确写法、`.app.tar.gz` 的打包约束、校验和格式要求，并把审计脚本写进发布检查表。
- 修正 `src/pages/InstallGuide.tsx` 的 macOS 与 Windows 文案，文件名跟随当前版本号生成。
- `src-tauri/tauri.conf.json` 增加 `bundle.macOS.minimumSystemVersion: "12.0"`。

## 四、仍需在发布侧处理

以下几项要动 Release 附件或重新打包，不在仓库改动范围内：

1. 用正确的 `darwin-aarch64` / `darwin-x86_64` 键重新上传 `niko-v0.1.4` 的 `latest.json`，恢复 macOS 更新检查。
2. 重新生成不含 AppleDouble 条目的 `Niko.app.tar.gz` 及其 `.sig`，并同步 `latest.json` 里的签名。
3. 修正 `SHA256SUMS.txt` 中 dmg 那一行的分隔符，重新上传。
4. 补发 `niko-v0.1.2` 缺失的 `Niko_0.1.2_x64-setup.exe.sig`（可选，历史版本）。
5. 处理完成后跑 `npm run verify:release -- --tag niko-v0.1.4` 确认零错误。
