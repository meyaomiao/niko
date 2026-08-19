# Niko 发布流程

## 固定发布规则

从 `v0.1.3` 起，发布职责固定如下：

| 内容 | 执行位置 | 说明 |
| --- | --- | --- |
| macOS universal 构建 | 发布者本机 Mac | 不使用 GitHub Actions Runner |
| macOS Developer ID 签名 | 发布者本机钥匙串 | 不向 GitHub 上传 p12 证书 |
| macOS Apple 公证与装订 | 发布者本机 | 使用本机保存的 Apple 凭证 |
| Windows 安装包 | GitHub Actions `windows-latest` | 只生成 MSI、EXE、签名文件和 SHA256 |
| `latest.json` 与 GitHub Release | 发布者本机 Mac | 两个平台产物齐全并验证后统一发布 |

`.github/workflows/release.yml` **只允许构建 Windows 产物**。它不创建 GitHub Release，也不得加入 `macos-*` Runner、Apple 证书导入或公证步骤。

普通 CI 中的 macOS `cargo check` 只做代码检查，不生成安装包，不属于发布流程。

## 发版前提

不要每改一点就发包。合并一批相对稳定的修改后，再执行一次完整发布。

发版前确认：

1. `main` 已同步且工作区干净。
2. `package.json` 与 `src-tauri/tauri.conf.json` 的版本号一致。
3. 前端检查、Rust 测试和必要的桌面端实机验证已通过。
4. 本机钥匙串中存在有效的 `Developer ID Application` 证书。
5. 本机已配置 Apple 公证凭证和 Tauri updater 私钥；凭证不得写入仓库或命令历史。

检查本机签名身份：

```bash
security find-identity -v -p codesigning
```

输出中必须能看到本次使用的 `Developer ID Application` 身份。

## 1. 生成 Windows 产物

从 `main` 手动触发 Windows-only workflow：

```bash
TAG=niko-v0.1.3
gh workflow run release.yml --ref main -f tag="$TAG" -f confirm=WINDOWS
```

找到并等待本次运行：

```bash
gh run list --workflow release.yml --limit 5
gh run watch <run-id>
```

运行成功后，把 `windows-unsigned` 下载到本机发布目录：

```bash
mkdir -p release-artifacts/windows
gh run download <run-id> -n windows-unsigned -D release-artifacts/windows
```

该 Actions 产物只保留 7 天。不要把它当作正式 Release。

## 2. 在本机完成 macOS 构建与认证

先加载本机保存的 Apple 与 Tauri updater 凭证，再运行 universal 构建：

```bash
npm ci
npx tauri build --target universal-apple-darwin
```

构建过程必须在本机完成以下步骤：

1. 同时编译 Apple Silicon 与 Intel。
2. 使用本机钥匙串中的 Developer ID Application 证书签名。
3. 提交 Apple 公证并等待成功结果。
4. 对最终应用或 DMG 完成 stapling。
5. 生成 `.app.tar.gz` 与对应 `.sig` updater 文件。

主要产物位于：

```text
src-tauri/target/universal-apple-darwin/release/bundle/macos/
src-tauri/target/universal-apple-darwin/release/bundle/dmg/
```

`Niko.app.tar.gz` 必须直接使用 `tauri build` 生成的那一个，不要用 Finder 压缩或 `tar` 手工重打包。
updater 解压时对每个条目固定剥掉第一段路径，而 macOS 自带的 `tar` 会额外写入 AppleDouble 条目
（`._Niko.app` 等），顶层那一条剥掉后是空路径，会让整次更新在第一个条目就失败。
确实需要手工打包时，必须屏蔽这些条目：

```bash
cd src-tauri/target/universal-apple-darwin/release/bundle/macos
COPYFILE_DISABLE=1 tar -czf Niko.app.tar.gz Niko.app
tar -tzf Niko.app.tar.gz | grep '\._' && echo "存在 AppleDouble 条目，不可发布"
```

重新打包后 `.sig` 必须跟着重新生成，`latest.json` 里的签名也要同步替换。

## 3. 本机验证 macOS 产物

发布前至少完成以下检查，路径按实际产物替换：

```bash
codesign --verify --deep --strict --verbose=2 \
  src-tauri/target/universal-apple-darwin/release/bundle/macos/Niko.app

spctl --assess --type execute --verbose=4 \
  src-tauri/target/universal-apple-darwin/release/bundle/macos/Niko.app

xcrun stapler validate \
  src-tauri/target/universal-apple-darwin/release/bundle/dmg/Niko_*.dmg
```

还要确认：

- DMG 可以安装并启动。
- `.app.tar.gz.sig` 存在且非空。
- DMG 和 updater 压缩包的 SHA256 已生成。
- Apple 公证记录显示本次提交为 `Accepted`。

## 4. 在本机组装 Release

把以下文件集中到本机 `release-artifacts/`：

- macOS：DMG、`.app.tar.gz`、`.app.tar.gz.sig`、`SHA256SUMS.txt`
- Windows：MSI、MSI `.sig`、EXE、EXE `.sig`、`SHA256SUMS_win.txt`
- updater：同时包含 `darwin-aarch64`、`darwin-x86_64` 和 `windows-x86_64` 的 `latest.json`

### updater 平台键

`tauri-plugin-updater` 按 `{os}-{arch}` 查找平台键，`arch` 取自运行分片的编译期架构，
合法取值只有 `x86_64`、`aarch64`、`i686`、`armv7`。`darwin-universal` 不是合法键：
写成它以后，Apple Silicon 上的应用会去找 `darwin-aarch64`、Intel 上会去找 `darwin-x86_64`，
两者都找不到，`check()` 直接抛错，用户在设置页只会看到「检查失败」。

universal 包的正确写法是把同一个 `.app.tar.gz` 挂在两个架构键上，签名也用同一份：

```json
{
  "version": "0.1.4",
  "platforms": {
    "darwin-aarch64": { "url": "…/Niko.app.tar.gz", "signature": "<Niko.app.tar.gz.sig 的内容>" },
    "darwin-x86_64": { "url": "…/Niko.app.tar.gz", "signature": "<Niko.app.tar.gz.sig 的内容>" },
    "windows-x86_64": { "url": "…/Niko_0.1.4_x64-setup.exe", "signature": "<EXE .sig 的内容>" }
  }
}
```

### 校验和文件格式

`SHA256SUMS.txt` 和 `SHA256SUMS_win.txt` 每一行都必须是「哈希 + 两个空格 + 文件名」。
只写一个空格时，`sha256sum -c` 和 macOS 自带的 `shasum -a 256 -c` 都会跳过该行、
只打印一句 warning 并仍然以 0 退出，用户会误以为安装包已经校验通过。
最省事的生成方式是让工具自己写：

```bash
cd release-artifacts
shasum -a 256 Niko_*.dmg Niko.app.tar.gz > SHA256SUMS.txt
shasum -a 256 -c SHA256SUMS.txt
```

`latest.json` 中的版本、文件名、下载 URL 和签名必须与最终附件逐项一致。先创建 Draft Release，完成双平台安装验证后再发布；不要边修改边反复创建正式 Release。

### 发布前后各跑一次产物审计

审计脚本会检查附件是否齐全、校验和格式与哈希是否正确、`latest.json` 的平台键能否被 updater 找到、
以及 macOS 更新包能否被 updater 解压。上传前先查本机目录，正式发布后再查线上 tag：

```bash
npm run verify:release -- --dir release-artifacts
npm run verify:release -- --tag niko-v0.1.4
```

两次都必须零错误退出。审计发现的问题在 Draft 阶段修好，不要带着已知错误正式发布。

## 发布检查表

- [ ] 本次修改已经集中集成并稳定
- [ ] `main` 工作区干净，版本号与 tag 一致
- [ ] Windows-only Actions 成功，产物已下载到本机
- [ ] macOS 在本机完成 universal 构建、签名、公证和装订
- [ ] `codesign`、`spctl`、`stapler` 检查通过
- [ ] macOS 与 Windows 安装包均完成实机安装验证
- [ ] `Niko.app.tar.gz` 不含 AppleDouble（`._*`）条目
- [ ] updater 签名文件和 SHA256 完整，校验和每行都是两个空格分隔
- [ ] `latest.json` 含 `darwin-aarch64`、`darwin-x86_64`、`windows-x86_64` 三个键且 URL 可访问
- [ ] `npm run verify:release -- --dir release-artifacts` 零错误
- [ ] Draft Release 附件检查无误后才正式发布
- [ ] 正式发布后 `npm run verify:release -- --tag <tag>` 零错误
- [ ] macOS 与 Windows 均实测过设置页的「检查更新」

## 5. 更新官网首页

官网首页的版本号、Release tag 和下载文件名由 `src-tauri/tauri.conf.json` 的版本自动生成。每次正式 Release 创建并验证完成后，在本机执行：

```bash
cd website
npm run check
npm run deploy
```

部署前应确认 `npm run check` 已将首页下载链接更新到本次 Release；部署后访问 `https://niko-ai.cc/`，确认首页版本号、macOS/Windows 下载链接和 Release Notes 链接均指向当前版本。不得只更新 GitHub Release 而跳过官网部署。
