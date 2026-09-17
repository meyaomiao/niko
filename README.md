<p align="center">
  <img src="src/assets/niko-mark.png" width="120" height="120" alt="Niko Logo">
</p>

<h1 align="center">Niko</h1>

<p align="center">
  <strong>一键接入 AI 助手</strong>
</p>

<p align="center">
  登录账号，选择应用和模型，剩下的配置交给 Niko。
</p>

<p align="center">
  <a href="https://github.com/meyaomiao/niko/releases/latest">
    <img src="https://img.shields.io/badge/%E4%B8%8B%E8%BD%BD-%E6%9C%80%E6%96%B0%E7%89%88%E6%9C%AC-2563eb?style=for-the-badge&amp;logo=github&amp;logoColor=white" alt="下载最新版本">
  </a>
  <a href="https://github.com/meyaomiao/niko">
    <img src="https://img.shields.io/badge/Star-%E6%94%B6%E8%97%8F%E9%A1%B9%E7%9B%AE-f5c542?style=for-the-badge&amp;logo=github&amp;logoColor=black" alt="Star 收藏项目">
  </a>
  <a href="https://github.com/meyaomiao/niko/issues">
    <img src="https://img.shields.io/badge/%E5%8F%8D%E8%B4%A8-%E6%8F%90%E4%BA%A4%E9%97%AE%E9%A2%98-4b5563?style=for-the-badge&amp;logo=github&amp;logoColor=white" alt="提交问题">
  </a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/macOS-12%2B-000000?style=flat-square&amp;logo=apple&amp;logoColor=white" alt="macOS 12+">
  <img src="https://img.shields.io/badge/Windows-10%2B-0078d4?style=flat-square&amp;logo=windows&amp;logoColor=white" alt="Windows 10+">
  <img src="https://img.shields.io/badge/Tauri-2-24c8db?style=flat-square&amp;logo=tauri&amp;logoColor=white" alt="Tauri 2">
  <img src="https://img.shields.io/badge/React-19-61dafb?style=flat-square&amp;logo=react&amp;logoColor=111827" alt="React 19">
</p>

## 产品截图

### 完整首页：选择、接入与验证

<p align="center">
  <img src="docs/images/niko-overview.png" width="900" alt="Niko 完整首页，包含账户、应用、模型、分组和接入操作">
</p>

<p align="center">
  <sub>完整首页截图。账户标识、余额与更新时间已隐藏。</sub>
</p>

首页从上到下对应桌面端实际分区：

| 界面区域 | 实际功能 |
| --- | --- |
| 顶栏 | 切换主题，进入「设置」「ChatGPT 会话」「模型」，以及退出。 |
| 左上账户卡 | 显示用户名和可用余额，可刷新。Niko 账号显示「充值」「使用明细」和「登录设备」；中转站连接只显示站点地址，充值跳到该站控制台。 |
| 右侧「接入应用」 | 胶囊列出 ChatGPT 桌面端、Claude 桌面端、DSH、ZCode；未安装的灰色不可点。装了多个时出现「全部」。旁边是「安装指引」。 |
| 当前应用说明 | ChatGPT 可选「未订阅 / 已订阅」。Claude、DSH、ZCode 各有一句接入范围说明。 |
| 已生效 | 接入成功后显示厂家 · 模型 · 分组。 |
| 厂家 → 模型 → 分组 | 先选厂家（原生组合会标「原生」），再选模型（默认同厂家内按发布时间新到旧，可点搜索），再选分组（倍率、参考价，可测速）。 |
| 底部四个按钮 | 「接入」写入配置。「重启」打开或重启桌面应用；选中 DSH 时变成「打开」。「检查」测连通性。「恢复」需再点一次确认，移除 Niko 写入的设置。 |

### 安装与首次使用

<p align="center">
  <img src="docs/images/niko-install-guide.png" width="860" alt="Niko Windows 安装指引界面">
</p>

<p align="center">
  <sub>针对 macOS 与 Windows 提供分步安装指引，解释系统安全提示，并给出安装包校验方式。</sub>
</p>

「安装指引」页说明 Niko 自己的安装包、Gatekeeper / SmartScreen，以及要接入的应用：ChatGPT 桌面端、Claude 桌面端、DSH、ZCode。Niko 不替代这些应用。

## 关于 Niko

Niko 是桌面端接入工具（应用内称「Niko 登录器」）。它不提供聊天界面，只把账号和模型写入本机已安装的目标应用。

登录页左侧会检查 ChatGPT 和 Claude 是否已安装，未安装时给出下载步骤。登录后的首页才会检测并接入 DSH、ZCode。

默认用 Niko / momotoken 账号。也可以在登录页切到「中转站」，用系统访问令牌连接自己的 new-api 站点。

## 安装

前往 [Releases](https://github.com/meyaomiao/niko/releases/latest) 下载最新版本。

| 平台 | 系统要求 | 安装包 | 说明 |
| --- | --- | --- | --- |
| macOS | macOS 12 或更高版本 | `.dmg` | 支持 Apple Silicon 和 Intel，发布包已签名并公证 |
| Windows | Windows 10 或更高版本 | `*_setup.exe` | 当前发布包暂未做代码签名 |

Windows 首次安装时可能出现 SmartScreen 提示。点击「更多信息」，再点击「仍要运行」即可继续安装。

## 快速开始

1. 安装并打开 Niko。
2. 登录页左侧确认 ChatGPT / Claude 的安装状态；需要时点「重新检查」。
3. 右侧三个入口任选其一：
   - **登录**：Niko 账号和密码，可勾选记住登录；开了两步验证会再要验证码。
   - **注册**：创建账号，先完成安全验证。
   - **中转站**：填写站点地址、系统访问令牌（控制台个人中心生成，不是 `sk-` 密钥）；多数站点用户 ID 可留空。
4. 进入首页，在「接入应用」里选择 ChatGPT、Claude、DSH、ZCode，或点「全部」。
5. 按厂家 → 模型 → 分组选择，点「接入」。
6. ChatGPT / Claude / ZCode 点「重启」；DSH 点「打开」。再用「检查」确认能请求模型。

退出接入时点「恢复」，五秒内再点一次「确认」。

## 登录页

左右分栏。

左侧标题是「先把要接入的应用装好」。这里只检查 **ChatGPT 桌面端** 和 **Claude 桌面端**，状态为「已安装 · 准备就绪」或「还没有安装」。未安装时按四步打开官方下载页，装好后点「重新检查」。

右侧是账户入口：

| 入口 | 要填的内容 |
| --- | --- |
| 登录 | 账号、密码，可选记住登录 |
| 注册 | 用户名、密码、确认密码，以及安全验证 |
| 中转站 | 站点地址、系统访问令牌、可选用户 ID，可选「记住这个中转站」 |

设备数达到上限时，会列出已登录设备，勾选退出后再继续。

## 首页接入

核心流程：`登录 → 检测应用 → 厂家 → 模型 → 分组 → 接入 → 打开或重启 → 检查`。

选中某个应用时，首页会写清范围：

| 应用 | 首页说明 |
| --- | --- |
| ChatGPT 桌面端 | 可切「未订阅 / 已订阅」。已订阅保留 ChatGPT 登录态，模型请求走当前账户；未订阅只使用当前账户，不需要 ChatGPT 账号。接入的是其中的 Codex。 |
| Claude 桌面端 | 仅作用于内置 Claude Code 面板，桌面端普通对话仍用 Anthropic 账号。 |
| DSH | 写入自定义网关 `momotoken`，点「打开」进入工作台即可使用，不必重启 DSH 进程。 |
| ZCode | 写入自定义提供方，打开 ZCode 后请选择 `Niko / momotoken`，不改智谱登录。 |

没有一键打开界面的命令行工具，不会出现在首页。

模型区按厂家列出，当前应用的原生厂家带「原生」标记。模型默认同厂家内按发布时间从新到旧；没有发布日期时按服务端顺序。标签包括常用、性价比、刚上新。分组只列出当前模型可用的项，显示倍率、参考价，以及可选的首字测速。

中转站登录时，账户卡显示站点地址，没有 Niko 的充值和登录设备管理；充值页会提示到该站控制台完成。

## 其他页面

| 页面 | 做什么 |
| --- | --- |
| 模型 | 与首页相同的厂家 → 模型 → 分组，用来看原价和分组折算价，不负责写入配置。 |
| 使用明细 | 查看用量。 |
| 充值 | Niko 账号走站内充值；中转站连接提示去该站控制台。 |
| ChatGPT 会话 | 检查、迁移和恢复 ChatGPT 桌面端本地会话。Claude 桌面端不支持会话管理。 |
| 设置 | 「当前账户」（中转站会显示站点地址）、「ChatGPT 会话」、「开机自启」、「可恢复的备份」、「检查是否能正常使用」、脱敏日志导出、「检查更新」。 |
| 安装指引 | Niko 安装说明，以及 ChatGPT、Claude、DSH、ZCode 的安装入口。 |

## 与类似产品的体验差异

| 对比维度 | Niko | 常见方案 |
| --- | --- | --- |
| 使用入口 | 接入完成后继续使用原有应用，不增加聊天界面。 | 聚合客户端通常要求迁到自己的会话体系。 |
| 首次配置 | 登录后选择应用、厂家、模型和分组即可，不需要复制 API Key。 | 手动配置通常要填地址、密钥和字段名。 |
| 本地运行 | 不在本地跑代理；接入完成后可以关掉 Niko。 | 本地网关通常要常驻，并处理端口和启动顺序。 |
| 切换反馈 | 同一页完成检测、写入、打开或重启、连通性检查。 | 只改文件的工具往往要用户自己判断有没有生效。 |
| 恢复 | 写入前保存备份，失败回滚；首页「恢复」和设置页备份都可以还原。 | 手工改配置通常要自己备份。 |

Niko 不提供聊天、MCP 编排或本地代理。会话检查与同步只支持 ChatGPT 桌面端。

## 设计理念

- **简单**：登录、选应用、选模型、接入，一条主路径。
- **轻量**：不在本地运行代理服务。
- **少改配置**：只改接入需要的字段，尽量保留原有设置。
- **可以恢复**：写入前保存备份，失败时回滚。
- **少暴露密钥**：界面不展示完整 API Key；记住登录存在系统钥匙串，导出日志会脱敏。

## 路线图

- [x] ChatGPT 桌面端接入
- [x] Claude 桌面端接入
- [x] 分组与模型选择
- [x] 用量、充值和设备管理
- [x] 配置快照、连通性测试和恢复
- [x] macOS 与 Windows 安装包
- [x] 登录页注册与安全验证
- [x] DSH 与 ZCode 接入
- [x] 登录页「中转站」（new-api 系统访问令牌）
- [ ] 支持更多能打开界面的客户端
- [ ] 完善不同客户端版本的兼容检测
- [ ] 改进诊断、故障提示和配置恢复
- [ ] 完善 Windows 代码签名和安装体验

## 本地开发

需要提前安装：

- Node.js 20+
- Rust stable
- 当前平台对应的 [Tauri 2 开发依赖](https://v2.tauri.app/start/prerequisites/)

```bash
git clone https://github.com/meyaomiao/niko.git
cd niko
npm install
npm run tauri dev
```

构建安装包：

```bash
npm run tauri build
```

正式发布时，macOS 必须在发布者本机完成 universal 构建、Developer ID 签名、Apple 公证和装订；GitHub Actions 只生成 Windows 安装包。完整步骤见 [发布流程](docs/release-process.md)。

主要技术栈：Tauri 2、React 19、TypeScript、Tailwind CSS 和 Rust。

## 问题反馈

遇到安装、登录、模型接入或配置恢复问题，可以在 [Issues](https://github.com/meyaomiao/niko/issues) 中提交。请说明操作系统、Niko 版本、目标应用和问题现象，不要上传密码或完整 API Key。

如果 Niko 对你有帮助，可以点击仓库右上角的 **Star** 收藏项目。

## 关联项目

- [momotoken-new-api](https://github.com/meyaomiao/momotoken-new-api)：账号、模型、计费和协议转换服务
