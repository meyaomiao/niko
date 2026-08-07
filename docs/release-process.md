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
- updater：同时包含 `darwin-universal` 与 `windows-x86_64` 的 `latest.json`

`latest.json` 中的版本、文件名、下载 URL 和签名必须与最终附件逐项一致。先创建 Draft Release，完成双平台安装验证后再发布；不要边修改边反复创建正式 Release。

## 发布检查表

- [ ] 本次修改已经集中集成并稳定
- [ ] `main` 工作区干净，版本号与 tag 一致
- [ ] Windows-only Actions 成功，产物已下载到本机
- [ ] macOS 在本机完成 universal 构建、签名、公证和装订
- [ ] `codesign`、`spctl`、`stapler` 检查通过
- [ ] macOS 与 Windows 安装包均完成实机安装验证
- [ ] updater 签名文件和 SHA256 完整
- [ ] `latest.json` 同时包含两个平台且 URL 可访问
- [ ] Draft Release 附件检查无误后才正式发布

## 5. 更新官网首页

官网首页的版本号、Release tag 和下载文件名由 `src-tauri/tauri.conf.json` 的版本自动生成。每次正式 Release 创建并验证完成后，在本机执行：

```bash
cd website
npm run check
npm run deploy
```

部署前应确认 `npm run check` 已将首页下载链接更新到本次 Release；部署后访问 `https://niko-ai.cc/`，确认首页版本号、macOS/Windows 下载链接和 Release Notes 链接均指向当前版本。不得只更新 GitHub Release 而跳过官网部署。
